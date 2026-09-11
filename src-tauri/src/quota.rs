//! 额度查询与 OAuth token 刷新。
//!
//! 用的是 Codex 客户端自己的接口（未公开，属于逆向所得）：
//!
//! ```text
//! GET https://chatgpt.com/backend-api/wham/usage
//! Authorization: Bearer <access_token>
//! ChatGPT-Account-Id: <account_id>
//! ```
//!
//! 返回 primary_window（5 小时）与 secondary_window（每周）两个窗口的已用百分比，
//! 外加各自的 `reset_after_seconds`，所以倒计时可以直接用服务端给的数字，最准。
//!
//! access_token 是短期 JWT，过期后返回 401，这里会自动用 refresh_token 换新的，
//! 并把刷新后的 auth 回传给调用方持久化。

use crate::model::{Quota, QuotaWindow};
use anyhow::{anyhow, Context, Result};
use serde_json::Value;
use std::time::Duration;

/// 额度接口，两个路径目前返回一致，第二个作为兜底。
const USAGE_URLS: [&str; 2] = [
    "https://chatgpt.com/backend-api/wham/usage",
    "https://chatgpt.com/backend-api/codex/usage",
];

/// OAuth 刷新端点与 Codex 公开的 client_id。
const TOKEN_URL: &str = "https://auth.openai.com/oauth/token";
const CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";

/// 一次查询的结果，可能顺带带回刷新过的凭证。
pub struct FetchOutcome {
    pub quota: Quota,
    pub refreshed_auth: Option<Value>,
}

fn now_secs() -> i64 {
    chrono::Utc::now().timestamp()
}

fn build_client() -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(25))
        .user_agent("codex-hub/0.1.0")
        .build()
        .context("构造 HTTP 客户端失败")
}

/// 查询单个账号的额度。
pub async fn fetch(auth: &Value) -> FetchOutcome {
    let tokens = match auth.get("tokens") {
        Some(t) => t,
        None => {
            return FetchOutcome {
                quota: fail("该账号是第三方 API Key，没有订阅额度窗口"),
                refreshed_auth: None,
            }
        }
    };

    let access_token = tokens
        .get("access_token")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    if access_token.is_empty() {
        return FetchOutcome {
            quota: fail("账号缺少 access_token，请重新登录"),
            refreshed_auth: None,
        };
    }
    let account_id = tokens
        .get("account_id")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();

    let client = match build_client() {
        Ok(c) => c,
        Err(e) => {
            return FetchOutcome {
                quota: fail(&format!("{e}")),
                refreshed_auth: None,
            }
        }
    };

    // 第一轮：直接用现有 token
    // 注意：刷新成功会走提前 return，所以这里不需要可变绑定
    let refreshed_auth = None;
    let mut last_error: Option<String> = None;

    for url in USAGE_URLS {
        match request_usage(&client, url, &access_token, &account_id).await {
            Ok(UsageResponse::Ok(body)) => {
                let mut quota = parse_quota(&body);
                quota.fetched_at = now_secs();
                return FetchOutcome {
                    quota,
                    refreshed_auth,
                };
            }
            Ok(UsageResponse::Unauthorized) => {
                // token 过期，尝试刷新一次
                match refresh_token(&client, auth).await {
                    Ok(new_auth) => {
                        let new_token = new_auth
                            .get("tokens")
                            .and_then(|t| t.get("access_token"))
                            .and_then(|v| v.as_str())
                            .unwrap_or_default()
                            .to_string();
                        match request_usage(&client, url, &new_token, &account_id).await {
                            Ok(UsageResponse::Ok(body)) => {
                                let mut quota = parse_quota(&body);
                                quota.fetched_at = now_secs();
                                return FetchOutcome {
                                    quota,
                                    refreshed_auth: Some(new_auth),
                                };
                            }
                            Ok(UsageResponse::Unauthorized) => {
                                last_error = Some("登录态已失效，请重新登录该账号".into());
                            }
                            Ok(UsageResponse::Error(msg)) => last_error = Some(msg),
                            Err(e) => last_error = Some(format!("{e}")),
                        }
                    }
                    Err(e) => last_error = Some(format!("刷新登录态失败：{e}")),
                }
            }
            Ok(UsageResponse::Error(msg)) => last_error = Some(msg),
            Err(e) => last_error = Some(format!("{e}")),
        }
    }

    FetchOutcome {
        quota: fail(last_error.as_deref().unwrap_or("查询失败")),
        refreshed_auth,
    }
}

fn fail(msg: &str) -> Quota {
    Quota {
        ok: false,
        error: Some(msg.to_string()),
        fetched_at: now_secs(),
        ..Default::default()
    }
}

enum UsageResponse {
    Ok(Value),
    Unauthorized,
    Error(String),
}

async fn request_usage(
    client: &reqwest::Client,
    url: &str,
    access_token: &str,
    account_id: &str,
) -> Result<UsageResponse> {
    let mut req = client
        .get(url)
        .header("Authorization", format!("Bearer {access_token}"))
        .header("Accept", "application/json");
    if !account_id.is_empty() {
        req = req.header("ChatGPT-Account-Id", account_id);
    }

    let resp = req.send().await.context("请求额度接口失败")?;
    let status = resp.status();

    if status.as_u16() == 401 || status.as_u16() == 403 {
        return Ok(UsageResponse::Unauthorized);
    }
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Ok(UsageResponse::Error(format!(
            "额度接口返回 {status}：{}",
            body.chars().take(160).collect::<String>()
        )));
    }

    let body: Value = resp.json().await.context("额度接口返回的不是 JSON")?;
    Ok(UsageResponse::Ok(body))
}

/// 用 refresh_token 换新的 access_token。
async fn refresh_token(client: &reqwest::Client, auth: &Value) -> Result<Value> {
    let refresh = auth
        .get("tokens")
        .and_then(|t| t.get("refresh_token"))
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("账号没有 refresh_token"))?;

    let params = [
        ("grant_type", "refresh_token"),
        ("client_id", CLIENT_ID),
        ("refresh_token", refresh),
    ];

    let resp = client
        .post(TOKEN_URL)
        .form(&params)
        .send()
        .await
        .context("请求 token 刷新接口失败")?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        if body.contains("invalid_grant") || body.contains("refresh token has already been used") {
            return Err(anyhow!(
                "登录凭证已过期或已被其他 Codex 实例更新，请重新登录该账号"
            ));
        }
        return Err(anyhow!(
            "刷新返回 {status}：{}",
            body.chars().take(160).collect::<String>()
        ));
    }

    let payload: Value = resp.json().await.context("刷新接口返回的不是 JSON")?;

    // 基于原 auth 打补丁，保留 account_id 等其余字段
    let mut new_auth = auth.clone();
    if let Some(tokens) = new_auth.get_mut("tokens").and_then(|t| t.as_object_mut()) {
        for key in ["access_token", "refresh_token", "id_token"] {
            if let Some(v) = payload.get(key).and_then(|x| x.as_str()) {
                tokens.insert(key.to_string(), Value::String(v.to_string()));
            }
        }
    }
    new_auth["last_refresh"] = Value::String(chrono::Utc::now().to_rfc3339());
    Ok(new_auth)
}

/// 解析额度响应体。
fn parse_quota(body: &Value) -> Quota {
    let rate = body.get("rate_limit");

    let window = |node: Option<&Value>| -> Option<QuotaWindow> {
        let node = node?;
        if node.is_null() {
            return None;
        }
        Some(QuotaWindow {
            used_percent: node
                .get("used_percent")
                .and_then(|v| v.as_f64())
                .unwrap_or(0.0),
            window_seconds: node
                .get("limit_window_seconds")
                .and_then(|v| v.as_i64())
                .unwrap_or(0),
            reset_at: node.get("reset_at").and_then(|v| v.as_i64()).unwrap_or(0),
            reset_after_seconds: node
                .get("reset_after_seconds")
                .and_then(|v| v.as_i64())
                .unwrap_or(0),
        })
    };

    let credits = body.get("credits");

    Quota {
        ok: true,
        error: None,
        plan_type: body
            .get("plan_type")
            .and_then(|v| v.as_str())
            .map(str::to_string),
        email: body
            .get("email")
            .and_then(|v| v.as_str())
            .map(str::to_string),
        primary: window(rate.and_then(|r| r.get("primary_window"))),
        secondary: window(rate.and_then(|r| r.get("secondary_window"))),
        code_review: window(
            body.get("code_review_rate_limit")
                .and_then(|r| r.get("primary_window")),
        ),
        credits_balance: credits
            .and_then(|c| c.get("balance"))
            .map(|v| match v {
                Value::String(s) => s.clone(),
                other => other.to_string(),
            }),
        has_credits: credits
            .and_then(|c| c.get("has_credits"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
        unlimited: credits
            .and_then(|c| c.get("unlimited"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
        limit_reached: rate
            .and_then(|r| r.get("limit_reached"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
        fetched_at: now_secs(),
    }
}
