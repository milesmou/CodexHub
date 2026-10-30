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
use reqwest::Method;
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
    let mut client = match AccountClient::new(auth) {
        Ok(c) => c,
        Err(e) => {
            return FetchOutcome {
                quota: fail(&format!("{e}")),
                refreshed_auth: None,
            }
        }
    };

    let quota = fetch_with_client(&mut client).await;
    FetchOutcome {
        quota,
        refreshed_auth: client.refreshed_auth,
    }
}

pub(crate) async fn fetch_with_client(client: &mut AccountClient) -> Quota {
    let mut last_error = "查询失败".to_string();
    for url in USAGE_URLS {
        match client.get(url).await {
            Ok(body) => {
                let mut quota = parse_quota(&body);
                // 重置卡查询失败不影响订阅额度展示。
                match crate::reset_credits::fetch(client).await {
                    Ok(credits) => quota.reset_credits = Some(credits),
                    Err(error) => quota.reset_credits_error = Some(format!("{error}")),
                }
                return quota;
            }
            Err(error) => last_error = format!("{error}"),
        }
    }
    fail(&last_error)
}

fn fail(msg: &str) -> Quota {
    Quota {
        ok: false,
        error: Some(msg.to_string()),
        fetched_at: now_secs(),
        ..Default::default()
    }
}

/// 同一账号的连续请求共用刷新后的 token；调用方负责持久化 refreshed_auth。
pub(crate) struct AccountClient {
    client: reqwest::Client,
    auth: Value,
    pub refreshed_auth: Option<Value>,
    refresh_attempted: bool,
}

impl AccountClient {
    pub fn new(auth: &Value) -> Result<Self> {
        let tokens = auth
            .get("tokens")
            .context("该账号是第三方 API Key，没有订阅额度窗口")?;
        if tokens
            .get("access_token")
            .and_then(Value::as_str)
            .unwrap_or("")
            .is_empty()
        {
            return Err(anyhow!("账号缺少 access_token，请重新登录"));
        }
        Ok(Self {
            client: build_client()?,
            auth: auth.clone(),
            refreshed_auth: None,
            refresh_attempted: false,
        })
    }

    pub async fn get(&mut self, url: &str) -> Result<Value> {
        self.request(Method::GET, url, None).await
    }

    pub async fn post(&mut self, url: &str, body: &Value) -> Result<Value> {
        self.request(Method::POST, url, Some(body)).await
    }

    async fn request(&mut self, method: Method, url: &str, body: Option<&Value>) -> Result<Value> {
        loop {
            let tokens = &self.auth["tokens"];
            let mut req = self
                .client
                .request(method.clone(), url)
                .bearer_auth(tokens["access_token"].as_str().unwrap_or(""))
                .header("Accept", "application/json");
            if let Some(id) = tokens["account_id"].as_str().filter(|id| !id.is_empty()) {
                req = req.header("ChatGPT-Account-Id", id);
            }
            if let Some(body) = body {
                req = req.json(body);
            }
            let resp = req.send().await.context("请求账号接口失败")?;
            let status = resp.status();
            if status.as_u16() == 401 || status.as_u16() == 403 {
                if self.refresh_attempted {
                    return Err(anyhow!("登录态已失效或无权访问，请重新登录该账号"));
                }
                self.refresh_attempted = true;
                let auth = refresh_token(&self.client, &self.auth).await?;
                self.refreshed_auth = Some(auth.clone());
                self.auth = auth;
                // POST 的请求体（含兑换请求 ID）保持不变。
                continue;
            }
            if !status.is_success() {
                // 不把服务端原始响应写入日志或前端，避免回显账号信息。
                return Err(anyhow!("账号接口返回 {status}"));
            }
            return resp.json().await.context("账号接口返回的不是 JSON");
        }
    }
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
        credits_balance: credits.and_then(|c| c.get("balance")).map(|v| match v {
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
        reset_credits: None,
        reset_credits_error: None,
        fetched_at: now_secs(),
    }
}
