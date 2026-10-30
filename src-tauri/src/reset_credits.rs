//! 重置卡查询与兑换。接口与 Codex 桌面客户端保持一致。

use crate::model::{ResetCredit, ResetCredits, ResetRedemption};
use crate::quota::AccountClient;
use anyhow::{anyhow, Context, Result};
use serde_json::{json, Value};

const CREDITS_URL: &str = "https://chatgpt.com/backend-api/wham/rate-limit-reset-credits";
const CONSUME_URL: &str = "https://chatgpt.com/backend-api/wham/rate-limit-reset-credits/consume";

pub(crate) async fn fetch(client: &mut AccountClient) -> Result<ResetCredits> {
    let body = client.get(CREDITS_URL).await?;
    parse_credits(&body, chrono::Utc::now().timestamp())
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum ConsumeResult {
    Reset,
    AlreadyRedeemed,
    NoCredit,
    NothingToReset,
}

pub(crate) async fn consume(
    client: &mut AccountClient,
    redemption: &ResetRedemption,
) -> Result<ConsumeResult> {
    let body = client
        .post(CONSUME_URL, &redemption_body(redemption))
        .await?;
    parse_consume_result(&body)
}

fn redemption_body(redemption: &ResetRedemption) -> Value {
    json!({
        "credit_id": redemption.credit_id,
        "redeem_request_id": redemption.request_id,
    })
}

fn parse_consume_result(body: &Value) -> Result<ConsumeResult> {
    match body.get("code").and_then(Value::as_str) {
        Some("reset") => Ok(ConsumeResult::Reset),
        Some("already_redeemed") => Ok(ConsumeResult::AlreadyRedeemed),
        Some("no_credit") => Ok(ConsumeResult::NoCredit),
        Some("nothing_to_reset") => Ok(ConsumeResult::NothingToReset),
        _ => Err(anyhow!("重置卡接口返回了无法确认的兑换结果")),
    }
}

fn timestamp(value: Option<&Value>) -> Result<Option<i64>> {
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(text)) => {
            let parsed =
                chrono::DateTime::parse_from_rfc3339(text).context("重置卡时间格式无效")?;
            Ok(Some(parsed.timestamp()))
        }
        Some(value) => {
            let number = value.as_i64().context("重置卡时间格式无效")?;
            Ok(Some(if number > 100_000_000_000 {
                number / 1000
            } else {
                number
            }))
        }
    }
}

fn parse_credits(body: &Value, now: i64) -> Result<ResetCredits> {
    let available_count = body
        .get("available_count")
        .and_then(Value::as_u64)
        .context("重置卡接口缺少可用数量")?;
    let rows = body
        .get("credits")
        .and_then(Value::as_array)
        .context("重置卡接口未返回卡片明细，无法选择最先到期的卡")?;
    let mut credits = Vec::new();
    for row in rows {
        if row.get("status").and_then(Value::as_str) != Some("available")
            || row.get("is_supported_by_plan").and_then(Value::as_bool) == Some(false)
        {
            continue;
        }
        let expires_at = timestamp(row.get("expires_at"))?;
        if expires_at.is_some_and(|expires| expires <= now) {
            continue;
        }
        let id = row
            .get("id")
            .and_then(Value::as_str)
            .filter(|id| !id.trim().is_empty())
            .context("重置卡缺少卡片 ID")?;
        credits.push(ResetCredit {
            id: id.to_string(),
            expires_at,
            granted_at: timestamp(row.get("granted_at"))?.unwrap_or(0),
        });
    }
    // 无到期时间的卡排在最后；同一到期时刻优先使用较早获得的卡。
    credits.sort_by(|a, b| {
        a.expires_at
            .unwrap_or(i64::MAX)
            .cmp(&b.expires_at.unwrap_or(i64::MAX))
            .then(a.granted_at.cmp(&b.granted_at))
            .then(a.id.cmp(&b.id))
    });
    Ok(ResetCredits {
        available_count,
        credits,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selects_earliest_expiry_and_excludes_unusable_cards() {
        let result = parse_credits(&json!({
            "available_count": 6,
            "credits": [
                {"id":"never", "status":"available", "expires_at":null},
                {"id":"later", "status":"available", "expires_at":300},
                {"id":"used", "status":"redeemed", "expires_at":110},
                {"id":"expired", "status":"available", "expires_at":100},
                {"id":"unsupported", "status":"available", "expires_at":120, "is_supported_by_plan":false},
                {"id":"newer", "status":"available", "expires_at":200, "granted_at":90},
                {"id":"first", "status":"available", "expires_at":200, "granted_at":50}
            ]
        }), 100).unwrap();
        let ids: Vec<_> = result
            .credits
            .iter()
            .map(|credit| credit.id.as_str())
            .collect();
        assert_eq!(ids, ["first", "newer", "later", "never"]);
        assert_eq!(result.available_count, 6);
    }

    #[test]
    fn handles_real_api_dates_and_does_not_treat_invalid_expiry_as_permanent() {
        let result = parse_credits(&json!({
            "available_count":2,
            "credits":[
                {"id":"later", "status":"available", "expires_at":"2026-11-06T21:59:37.207444Z"},
                {"id":"earlier", "status":"available", "expires_at":"2026-11-06T08:00:00+08:00"}
            ]
        }), 0).unwrap();
        assert_eq!(result.credits[0].id, "earlier");
        assert_eq!(
            timestamp(Some(&json!(1_793_999_999_000_i64))).unwrap(),
            Some(1_793_999_999)
        );
        assert!(parse_credits(
            &json!({"available_count":1, "credits":[
                {"id":"bad", "status":"available", "expires_at":"unknown"}
            ]}),
            0
        )
        .is_err());
        assert!(parse_credits(&json!({"available_count":1, "credits":null}), 0).is_err());
    }

    #[test]
    fn distinguishes_business_results_from_unconfirmed_responses() {
        for (code, expected) in [
            ("reset", ConsumeResult::Reset),
            ("already_redeemed", ConsumeResult::AlreadyRedeemed),
            ("no_credit", ConsumeResult::NoCredit),
            ("nothing_to_reset", ConsumeResult::NothingToReset),
        ] {
            assert_eq!(
                parse_consume_result(&json!({"code":code})).unwrap(),
                expected
            );
        }
        assert!(parse_consume_result(&json!({"code":"unknown"})).is_err());
        assert!(parse_consume_result(&json!({})).is_err());
    }

    #[test]
    fn persisted_redemption_reuses_the_same_card_and_idempotency_key() {
        let pending = ResetRedemption {
            credit_id: "first-expiring-card".into(),
            request_id: uuid::Uuid::new_v4().to_string(),
        };
        let saved = serde_json::to_string(&pending).unwrap();
        let restored: ResetRedemption = serde_json::from_str(&saved).unwrap();
        assert_eq!(redemption_body(&pending), redemption_body(&restored));
        assert_eq!(
            redemption_body(&restored)["credit_id"],
            "first-expiring-card"
        );
        assert_eq!(
            redemption_body(&restored)["redeem_request_id"],
            pending.request_id
        );
    }
}
