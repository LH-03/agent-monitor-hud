//! DeepSeek account balance through the official `GET /user/balance` endpoint
//! (WinHTTP, system proxy settings). Only the user-provided API key is sent,
//! only to api.deepseek.com; the response is reduced to currency and amounts.

use serde_json::Value;
use windows::core::w;

#[derive(Clone, Debug, PartialEq)]
pub struct Balance {
    /// "CNY" or "USD".
    pub currency: String,
    pub total: f64,
    pub granted: f64,
    pub topped_up: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Failure {
    /// 401/403: the key is wrong or revoked.
    Unauthorized,
    Network,
    Unexpected,
}

pub fn parse(body: &str) -> Result<Balance, Failure> {
    let doc: Value = serde_json::from_str(body).map_err(|_| Failure::Unexpected)?;
    let infos = doc.get("balance_infos").and_then(Value::as_array).ok_or(Failure::Unexpected)?;
    let num = |v: &Value, k: &str| v.get(k).and_then(|x| x.as_str().and_then(|s| s.parse::<f64>().ok()).or_else(|| x.as_f64())).unwrap_or(0.0);
    // Prefer CNY (the default wallet), else the first listed currency.
    let pick = infos.iter().find(|i| i.get("currency").and_then(Value::as_str) == Some("CNY")).or_else(|| infos.first()).ok_or(Failure::Unexpected)?;
    Ok(Balance {
        currency: pick.get("currency").and_then(Value::as_str).unwrap_or("CNY").to_string(),
        total: num(pick, "total_balance"),
        granted: num(pick, "granted_balance"),
        topped_up: num(pick, "topped_up_balance"),
    })
}

pub fn fetch(api_key: &str) -> Result<Balance, Failure> {
    let headers = format!("Authorization: Bearer {}\r\nAccept: application/json\r\nCache-Control: no-cache\r\n", api_key.trim());
    let (status, body) = crate::util::http::get(w!("api.deepseek.com"), w!("/user/balance"), &headers).map_err(|_| Failure::Network)?;
    match status { 200 => parse(&body), 401 | 403 => Err(Failure::Unauthorized), _ => Err(Failure::Unexpected) }
}

/// Short display string: `¥23.50` / `$4.20`.
pub fn format(balance: &Balance) -> String {
    let symbol = if balance.currency == "USD" { "$" } else { "¥" };
    format!("{symbol}{:.2}", balance.total)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_official_shape() {
        let body = r#"{"is_available":true,"balance_infos":[{"currency":"CNY","total_balance":"110.00","granted_balance":"10.00","topped_up_balance":"100.00"}]}"#;
        let b = parse(body).unwrap();
        assert_eq!(b.currency, "CNY");
        assert_eq!(b.total, 110.0);
        assert_eq!(format(&b), "¥110.00");
        assert_eq!(parse("{}"), Err(Failure::Unexpected));
    }

    /// Network check: an invalid key must come back as Unauthorized.
    #[test]
    #[ignore]
    fn live_invalid_key_is_unauthorized() {
        assert_eq!(fetch("sk-agent-monitor-hud-invalid-test-key-0000"), Err(Failure::Unauthorized));
    }
}
