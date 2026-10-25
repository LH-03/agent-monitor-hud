//! Locally stored secrets (currently only the user's DeepSeek API key, for the
//! balance readout). Encrypted with Windows DPAPI for the current user and kept
//! apart from `settings.json`, so exporting or sharing settings never carries
//! it. The plain key exists only in memory while a request is being made.

use crate::paths::HudPaths;
use crate::util::fsx;
use std::path::PathBuf;
use windows::Win32::Foundation::{HLOCAL, LocalFree};
use windows::Win32::Security::Cryptography::{CRYPT_INTEGER_BLOB, CRYPTPROTECT_UI_FORBIDDEN, CryptProtectData, CryptUnprotectData};
use windows::core::w;

/// Extra entropy so other programs using plain user-scope DPAPI cannot
/// decrypt the file by accident.
const ENTROPY: &[u8] = b"AgentMonitorHUD/deepseek-api-key/v1";

fn key_path(paths: &HudPaths) -> PathBuf {
    paths.state_root.join("secrets").join("deepseek-api-key.bin")
}

fn blob(data: &[u8]) -> CRYPT_INTEGER_BLOB {
    CRYPT_INTEGER_BLOB { cbData: data.len() as u32, pbData: data.as_ptr() as *mut u8 }
}

fn take(out: CRYPT_INTEGER_BLOB) -> Vec<u8> {
    let bytes = unsafe { std::slice::from_raw_parts(out.pbData, out.cbData as usize).to_vec() };
    unsafe {
        let _ = LocalFree(Some(HLOCAL(out.pbData as *mut _)));
    }
    bytes
}

fn protect(plain: &[u8]) -> Option<Vec<u8>> {
    let mut out = CRYPT_INTEGER_BLOB::default();
    let entropy = blob(ENTROPY);
    unsafe { CryptProtectData(&blob(plain), w!("Agent Monitor HUD"), Some(&entropy), None, None, CRYPTPROTECT_UI_FORBIDDEN, &mut out).ok()? };
    Some(take(out))
}

fn unprotect(sealed: &[u8]) -> Option<Vec<u8>> {
    let mut out = CRYPT_INTEGER_BLOB::default();
    let entropy = blob(ENTROPY);
    unsafe { CryptUnprotectData(&blob(sealed), None, Some(&entropy), None, None, CRYPTPROTECT_UI_FORBIDDEN, &mut out).ok()? };
    Some(take(out))
}

/// Validates the shape of a DeepSeek key (`sk-` + url-safe characters).
pub fn looks_like_deepseek_key(key: &str) -> bool {
    let key = key.trim();
    key.starts_with("sk-") && key.len() >= 20 && key.len() <= 200 && key.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

pub fn save_deepseek_key(paths: &HudPaths, key: &str) -> bool {
    let Some(sealed) = protect(key.trim().as_bytes()) else { return false };
    let path = key_path(paths);
    let _ = std::fs::create_dir_all(path.parent().unwrap());
    fsx::write_atomic(&path, &sealed).is_ok()
}

pub fn load_deepseek_key(paths: &HudPaths) -> Option<String> {
    let sealed = std::fs::read(key_path(paths)).ok()?;
    String::from_utf8(unprotect(&sealed)?).ok().filter(|k| !k.is_empty())
}

pub fn has_deepseek_key(paths: &HudPaths) -> bool {
    key_path(paths).exists()
}

/// Last four characters, for "saved · …abcd" in settings.
pub fn deepseek_key_hint(paths: &HudPaths) -> Option<String> {
    let key = load_deepseek_key(paths)?;
    let tail: String = key.chars().rev().take(4).collect::<Vec<_>>().into_iter().rev().collect();
    Some(tail)
}

pub fn clear_deepseek_key(paths: &HudPaths) {
    fsx::remove_quietly(&key_path(paths));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dpapi_round_trip() {
        let sealed = protect(b"sk-test-0123456789abcdef").unwrap();
        assert!(!sealed.windows(8).any(|w| w == b"sk-test-"));
        assert_eq!(unprotect(&sealed).unwrap(), b"sk-test-0123456789abcdef");
        assert!(looks_like_deepseek_key("sk-0123456789abcdef0123"));
        assert!(!looks_like_deepseek_key("hello world"));
    }
}
