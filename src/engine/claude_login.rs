//! Opt-in, read-only access to the locally signed-in Claude account.
//! Desktop cache decryption uses Windows DPAPI + CNG, like Electron's safeStorage.
//! No refresh tokens, cookies or decrypted credentials are persisted or returned.

use crate::util::fsx;
use serde_json::Value;
use std::path::Path;
use windows::Win32::Foundation::{HLOCAL, LocalFree};
use windows::Win32::Security::Cryptography::*;
use windows::core::PCWSTR;

fn base64(text: &str) -> Option<Vec<u8>> {
    let wide: Vec<u16> = text.encode_utf16().collect();
    let mut len = 0;
    unsafe {
        CryptStringToBinaryW(&wide, CRYPT_STRING_BASE64, None, &mut len, None, None).ok()?;
        let mut bytes = vec![0; len as usize];
        CryptStringToBinaryW(&wide, CRYPT_STRING_BASE64, Some(bytes.as_mut_ptr()), &mut len, None, None).ok()?;
        bytes.truncate(len as usize);
        Some(bytes)
    }
}

fn dpapi(bytes: &[u8]) -> Option<Vec<u8>> {
    let input = CRYPT_INTEGER_BLOB { cbData: bytes.len() as u32, pbData: bytes.as_ptr() as *mut _ };
    let mut output = CRYPT_INTEGER_BLOB::default();
    unsafe {
        CryptUnprotectData(&input, None, None, None, None, CRYPTPROTECT_UI_FORBIDDEN, &mut output).ok()?;
        let plain = std::slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec();
        let _ = LocalFree(Some(HLOCAL(output.pbData as *mut _)));
        Some(plain)
    }
}

struct Algorithm(BCRYPT_ALG_HANDLE);
impl Drop for Algorithm { fn drop(&mut self) { unsafe { let _ = BCryptCloseAlgorithmProvider(self.0, 0); } } }
struct Key(BCRYPT_KEY_HANDLE);
impl Drop for Key { fn drop(&mut self) { unsafe { let _ = BCryptDestroyKey(self.0); } } }

fn decrypt(sealed: &[u8], secret: &[u8]) -> Option<Vec<u8>> {
    if sealed.len() < 31 || !matches!(&sealed[..3], b"v10" | b"v11") { return None; }
    unsafe {
        let mut alg = BCRYPT_ALG_HANDLE::default();
        if BCryptOpenAlgorithmProvider(&mut alg, BCRYPT_AES_ALGORITHM, PCWSTR::null(), BCRYPT_OPEN_ALGORITHM_PROVIDER_FLAGS(0)).0 < 0 { return None; }
        let alg = Algorithm(alg);
        let mode: Vec<u8> = "ChainingModeGCM\0".encode_utf16().flat_map(u16::to_le_bytes).collect();
        if BCryptSetProperty(BCRYPT_HANDLE(alg.0.0), BCRYPT_CHAINING_MODE, &mode, 0).0 < 0 { return None; }
        let mut key = BCRYPT_KEY_HANDLE::default();
        if BCryptGenerateSymmetricKey(alg.0, &mut key, None, secret, 0).0 < 0 { return None; }
        let key = Key(key);
        let info = BCRYPT_AUTHENTICATED_CIPHER_MODE_INFO {
            cbSize: std::mem::size_of::<BCRYPT_AUTHENTICATED_CIPHER_MODE_INFO>() as u32,
            dwInfoVersion: BCRYPT_AUTHENTICATED_CIPHER_MODE_INFO_VERSION,
            pbNonce: sealed[3..15].as_ptr() as *mut _, cbNonce: 12,
            pbTag: sealed[sealed.len()-16..].as_ptr() as *mut _, cbTag: 16,
            ..Default::default()
        };
        let ciphertext = &sealed[15..sealed.len()-16];
        let mut plain = vec![0; ciphertext.len()];
        let mut written = 0;
        if BCryptDecrypt(key.0, Some(ciphertext), Some(&info as *const _ as *const _), None, Some(&mut plain), &mut written, BCRYPT_FLAGS(0)).0 < 0 { return None; }
        plain.truncate(written as usize);
        Some(plain)
    }
}

fn desktop_token(home: &Path, now: i64) -> Option<String> {
    for sample in super::claude_quota::candidates(home) {
        let Some(dir) = sample.parent() else { continue };
        let Some(config) = fsx::read_text(&dir.join("config.json")).ok().and_then(|s| serde_json::from_str::<Value>(&s).ok()) else { continue };
        let Some(account) = config.get("lastKnownAccountUuid").and_then(Value::as_str).filter(|s| !s.is_empty()) else { continue };
        let Some(sealed) = config.get("oauth:tokenCacheV2").and_then(Value::as_str).and_then(base64) else { continue };
        let Some(local) = fsx::read_text(&dir.join("Local State")).ok().and_then(|s| serde_json::from_str::<Value>(&s).ok()) else { continue };
        let Some(key) = local.pointer("/os_crypt/encrypted_key").and_then(Value::as_str).and_then(base64) else { continue };
        let Some(mut key) = key.strip_prefix(b"DPAPI").and_then(dpapi) else { continue };
        let plain = decrypt(&sealed, &key);
        key.fill(0);
        let Some(mut plain) = plain else { continue };
        let cache = serde_json::from_slice::<Value>(&plain).ok();
        plain.fill(0);
        let Some(cache) = cache.and_then(|v| v.as_object().cloned()) else { continue };
        let prefix = format!("acct:{account}|");
        if let Some(token) = cache.iter()
            .filter(|(name, value)| name.starts_with(&prefix) && name.contains("claude_code") && value.get("expiresAt").and_then(Value::as_i64).is_some_and(|t| t > now))
            .max_by_key(|(_, value)| value.get("expiresAt").and_then(Value::as_i64))
            .and_then(|(_, value)| value.get("token").and_then(Value::as_str)).filter(|s| !s.is_empty()) {
            return Some(token.to_string());
        }
    }
    None
}

pub fn access_token(home: &Path, claude_root: &Path, now: i64) -> Option<String> {
    // Match the Desktop account selected by the user before the CLI fallback.
    if let Some(token) = desktop_token(home, now) { return Some(token); }
    if let Some(token) = std::env::var("CLAUDE_CODE_OAUTH_TOKEN").ok().filter(|s| !s.is_empty()) { return Some(token); }
    let doc: Value = serde_json::from_str(&fsx::read_text(&claude_root.join(".credentials.json")).ok()?).ok()?;
    let oauth = doc.get("claudeAiOauth")?;
    if oauth.get("expiresAt").and_then(Value::as_i64).is_some_and(|t| t <= now) { return None; }
    oauth.get("accessToken").and_then(Value::as_str).filter(|s| !s.is_empty()).map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn authenticates_chromium_gcm_cache_payloads() {
        // NIST AES-GCM zero-key/zero-nonce vector; no user credentials involved.
        let mut sealed = b"v10".to_vec();
        sealed.extend([0; 12]);
        sealed.extend([0x03,0x88,0xda,0xce,0x60,0xb6,0xa3,0x92,0xf3,0x28,0xc2,0xb9,0x71,0xb2,0xfe,0x78]);
        sealed.extend([0xab,0x6e,0x47,0xd4,0x2c,0xec,0x13,0xbd,0xf5,0x3a,0x67,0xb2,0x12,0x57,0xbd,0xdf]);
        assert_eq!(decrypt(&sealed, &[0; 16]).unwrap(), vec![0; 16]);
        *sealed.last_mut().unwrap() ^= 1;
        assert!(decrypt(&sealed, &[0; 16]).is_none());
    }
}
