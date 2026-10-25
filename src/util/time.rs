//! Wall-clock helpers. All instants are Unix milliseconds (UTC); only
//! formatting converts to the local time zone.

use std::time::{SystemTime, UNIX_EPOCH};
use windows::Win32::Foundation::{FILETIME, SYSTEMTIME};
use windows::Win32::System::Time::{FileTimeToSystemTime, SystemTimeToTzSpecificLocalTime};

/// "Never" sentinel. Far enough from i64::MIN that `now - MIN` cannot overflow.
pub const MIN: i64 = i64::MIN / 4;

pub fn now_ms() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

/// Windows FILETIME (100 ns since 1601) to Unix milliseconds.
pub fn filetime_to_ms(low: u32, high: u32) -> i64 {
    let ticks = ((high as u64) << 32) | low as u64;
    (ticks / 10_000) as i64 - 11_644_473_600_000
}

fn ms_to_filetime(ms: i64) -> FILETIME {
    let ticks = ((ms + 11_644_473_600_000).max(0) as u64) * 10_000;
    FILETIME { dwLowDateTime: ticks as u32, dwHighDateTime: (ticks >> 32) as u32 }
}

/// Local calendar fields for a Unix millisecond instant.
pub fn local_parts(ms: i64) -> SYSTEMTIME {
    let ft = ms_to_filetime(ms);
    let mut utc = SYSTEMTIME::default();
    let mut local = SYSTEMTIME::default();
    unsafe {
        if FileTimeToSystemTime(&ft, &mut utc).is_ok()
            && SystemTimeToTzSpecificLocalTime(None, &utc, &mut local).is_ok()
        {
            return local;
        }
    }
    utc
}

pub fn local_hm(ms: i64) -> String {
    let t = local_parts(ms);
    format!("{:02}:{:02}", t.wHour, t.wMinute)
}

pub fn local_hms(ms: i64) -> String {
    let t = local_parts(ms);
    format!("{:02}:{:02}:{:02}", t.wHour, t.wMinute, t.wSecond)
}

/// ISO 8601 with the local UTC offset, e.g. `2026-10-24T19:07:48.458+08:00`.
pub fn local_iso(ms: i64) -> String {
    if ms == MIN {
        return "0001-01-01T00:00:00.0000000+00:00".into();
    }
    let t = local_parts(ms);
    let utc = ms_to_parts_utc(ms);
    // Offset = local - utc, computed from calendar fields to honour DST.
    let local_ms = days_from_civil(t.wYear as i64, t.wMonth as i64, t.wDay as i64) * 86_400_000
        + (t.wHour as i64 * 3600 + t.wMinute as i64 * 60 + t.wSecond as i64) * 1000;
    let utc_ms = days_from_civil(utc.0, utc.1, utc.2) * 86_400_000 + (utc.3 * 3600 + utc.4 * 60 + utc.5) * 1000;
    let offset_min = (local_ms - utc_ms) / 60_000;
    let sign = if offset_min < 0 { '-' } else { '+' };
    let off = offset_min.abs();
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.{:03}{}{:02}:{:02}",
        t.wYear, t.wMonth, t.wDay, t.wHour, t.wMinute, t.wSecond, ms.rem_euclid(1000), sign, off / 60, off % 60
    )
}

pub fn utc_iso(ms: i64) -> String {
    let (y, mo, d, h, mi, s) = ms_to_parts_utc(ms);
    format!("{y:04}-{mo:02}-{d:02}T{h:02}:{mi:02}:{s:02}.{:03}Z", ms.rem_euclid(1000))
}

fn ms_to_parts_utc(ms: i64) -> (i64, i64, i64, i64, i64, i64) {
    let days = ms.div_euclid(86_400_000);
    let rem = ms.rem_euclid(86_400_000) / 1000;
    let (y, m, d) = civil_from_days(days);
    (y, m, d, rem / 3600, rem % 3600 / 60, rem % 60)
}

// Howard Hinnant's civil date algorithms.
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// Parses RFC 3339 / ISO 8601 timestamps such as `2026-10-24T11:12:38.915Z`
/// or `2026-10-24T19:07:48.4584586+08:00`. A missing offset is treated as UTC
/// (matching the old `AssumeUniversal` parsing).
pub fn parse_iso(text: &str) -> Option<i64> {
    let s = text.trim().as_bytes();
    if s.len() < 19 {
        return None;
    }
    let num = |a: usize, b: usize| -> Option<i64> {
        std::str::from_utf8(&s[a..b]).ok()?.parse::<i64>().ok()
    };
    let (y, mo, d) = (num(0, 4)?, num(5, 7)?, num(8, 10)?);
    if s[4] != b'-' || s[7] != b'-' || !(s[10] == b'T' || s[10] == b' ' || s[10] == b't') {
        return None;
    }
    let (h, mi, se) = (num(11, 13)?, num(14, 16)?, num(17, 19)?);
    if !(1..=12).contains(&mo) || !(1..=31).contains(&d) || h > 23 || mi > 59 || se > 60 {
        return None;
    }
    let mut i = 19;
    let mut frac_ms = 0i64;
    if i < s.len() && (s[i] == b'.' || s[i] == b',') {
        i += 1;
        let start = i;
        while i < s.len() && s[i].is_ascii_digit() {
            i += 1;
        }
        let digits = &s[start..i];
        let mut value = 0i64;
        for (k, c) in digits.iter().take(3).enumerate() {
            value += ((c - b'0') as i64) * 10i64.pow(2 - k as u32);
        }
        frac_ms = value;
    }
    let mut offset_min = 0i64;
    if i < s.len() {
        match s[i] {
            b'Z' | b'z' => {}
            b'+' | b'-' => {
                let sign = if s[i] == b'-' { -1 } else { 1 };
                let rest = std::str::from_utf8(&s[i + 1..]).ok()?;
                let digits: String = rest.chars().filter(|c| c.is_ascii_digit()).collect();
                if digits.len() < 2 {
                    return None;
                }
                let oh: i64 = digits[0..2].parse().ok()?;
                let om: i64 = if digits.len() >= 4 { digits[2..4].parse().ok()? } else { 0 };
                offset_min = sign * (oh * 60 + om);
            }
            _ => return None,
        }
    }
    let days = days_from_civil(y, mo, d);
    Some(days * 86_400_000 + (h * 3600 + mi * 60 + se) * 1000 + frac_ms - offset_min * 60_000)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_common_forms() {
        assert_eq!(parse_iso("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(parse_iso("2026-10-24T11:12:38.915Z"), Some(1792840358915));
        assert_eq!(parse_iso("2026-10-24T19:12:38.915+08:00"), Some(1792840358915));
        assert_eq!(parse_iso("2026-10-24T11:12:38.9150000Z"), Some(1792840358915));
        assert_eq!(utc_iso(1792840358915), "2026-10-24T11:12:38.915Z");
        assert!(parse_iso("nonsense").is_none());
    }
}
