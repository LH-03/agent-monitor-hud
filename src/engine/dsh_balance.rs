//! DeepSeek account balance through the official `GET /user/balance` endpoint
//! (WinHTTP, system proxy settings). Only the user-provided API key is sent,
//! only to api.deepseek.com; the response is reduced to currency and amounts.

use serde_json::Value;
use windows::Win32::Networking::WinHttp::*;
use windows::core::{PCWSTR, w};

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

struct Handle(*mut core::ffi::c_void);

impl Drop for Handle {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                let _ = WinHttpCloseHandle(self.0);
            }
        }
    }
}

pub fn fetch(api_key: &str) -> Result<Balance, Failure> {
    unsafe {
        let session = Handle(WinHttpOpen(w!("AgentMonitorHUD/4.0"), WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY, PCWSTR::null(), PCWSTR::null(), 0));
        if session.0.is_null() {
            return Err(Failure::Network);
        }
        let _ = WinHttpSetTimeouts(session.0, 8000, 8000, 8000, 10_000);
        let connect = Handle(WinHttpConnect(session.0, w!("api.deepseek.com"), 443, 0));
        if connect.0.is_null() {
            return Err(Failure::Network);
        }
        let request = Handle(WinHttpOpenRequest(connect.0, w!("GET"), w!("/user/balance"), PCWSTR::null(), PCWSTR::null(), std::ptr::null(), WINHTTP_FLAG_SECURE));
        if request.0.is_null() {
            return Err(Failure::Network);
        }
        let headers: Vec<u16> = format!("Authorization: Bearer {}\r\nAccept: application/json\r\n", api_key.trim()).encode_utf16().collect();
        WinHttpAddRequestHeaders(request.0, &headers, WINHTTP_ADDREQ_FLAG_ADD | WINHTTP_ADDREQ_FLAG_REPLACE).map_err(|_| Failure::Network)?;
        WinHttpSendRequest(request.0, None, None, 0, 0, 0).map_err(|_| Failure::Network)?;
        WinHttpReceiveResponse(request.0, std::ptr::null_mut()).map_err(|_| Failure::Network)?;
        let mut status = 0u32;
        let mut size = std::mem::size_of::<u32>() as u32;
        let mut index = 0u32;
        WinHttpQueryHeaders(request.0, WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER, PCWSTR::null(), Some(&mut status as *mut u32 as *mut _), &mut size, &mut index)
            .map_err(|_| Failure::Network)?;
        let mut body = Vec::new();
        let mut buffer = [0u8; 4096];
        loop {
            let mut read = 0u32;
            WinHttpReadData(request.0, buffer.as_mut_ptr() as *mut _, buffer.len() as u32, &mut read).map_err(|_| Failure::Network)?;
            if read == 0 || body.len() > 64 * 1024 {
                break;
            }
            body.extend_from_slice(&buffer[..read as usize]);
        }
        match status {
            200 => parse(&String::from_utf8_lossy(&body)),
            401 | 403 => Err(Failure::Unauthorized),
            _ => Err(Failure::Unexpected),
        }
    }
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
