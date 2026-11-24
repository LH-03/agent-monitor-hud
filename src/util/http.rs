//! Bounded account requests to fixed official hosts or a local application.

use windows::Win32::Networking::WinHttp::*;
use windows::core::{PCWSTR, w};

struct Handle(*mut core::ffi::c_void);
impl Drop for Handle {
    fn drop(&mut self) { if !self.0.is_null() { unsafe { let _ = WinHttpCloseHandle(self.0); } } }
}

pub fn get(host: PCWSTR, route: PCWSTR, headers: &str) -> Result<(u32, String), ()> {
    request(host, route, headers, None)
}

pub fn post_loopback(port: u16, route: PCWSTR, headers: &str, body: &[u8]) -> Result<(u32, String), ()> {
    request(w!("127.0.0.1"), route, headers, Some((port, body)))
}

fn request(host: PCWSTR, route: PCWSTR, headers: &str, local_post: Option<(u16, &[u8])>) -> Result<(u32, String), ()> {
    let (access, verb, port, data, flags, timeout) = match local_post {
        Some((port, data)) => (WINHTTP_ACCESS_TYPE_NO_PROXY, w!("POST"), port, data, WINHTTP_OPEN_REQUEST_FLAGS::default(), 2_000),
        None => (WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY, w!("GET"), 443, &[][..], WINHTTP_FLAG_SECURE, 8_000),
    };
    unsafe {
        let session = Handle(WinHttpOpen(w!("AgentMonitorHUD/4.1"), access, PCWSTR::null(), PCWSTR::null(), 0));
        if session.0.is_null() { return Err(()); }
        let _ = WinHttpSetTimeouts(session.0, 4000, 4000, 4000, timeout);
        let connect = Handle(WinHttpConnect(session.0, host, port, 0));
        if connect.0.is_null() { return Err(()); }
        let request = Handle(WinHttpOpenRequest(connect.0, verb, route, PCWSTR::null(), PCWSTR::null(), std::ptr::null(), flags));
        if request.0.is_null() { return Err(()); }
        // Never forward a user's account token to a redirected destination.
        let redirect = WINHTTP_OPTION_REDIRECT_POLICY_NEVER;
        WinHttpSetOption(Some(request.0), WINHTTP_OPTION_REDIRECT_POLICY, Some(&redirect.to_le_bytes())).map_err(|_| ())?;
        let headers: Vec<u16> = headers.encode_utf16().collect();
        WinHttpAddRequestHeaders(request.0, &headers, WINHTTP_ADDREQ_FLAG_ADD | WINHTTP_ADDREQ_FLAG_REPLACE).map_err(|_| ())?;
        WinHttpSendRequest(request.0, None, (!data.is_empty()).then_some(data.as_ptr().cast()), data.len() as u32, data.len() as u32, 0).map_err(|_| ())?;
        WinHttpReceiveResponse(request.0, std::ptr::null_mut()).map_err(|_| ())?;
        let mut status = 0u32;
        let mut size = std::mem::size_of::<u32>() as u32;
        let mut index = 0u32;
        WinHttpQueryHeaders(request.0, WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER, PCWSTR::null(), Some(&mut status as *mut _ as *mut _), &mut size, &mut index).map_err(|_| ())?;
        let mut body = Vec::new();
        let mut buffer = [0u8; 4096];
        loop {
            let mut read = 0u32;
            WinHttpReadData(request.0, buffer.as_mut_ptr() as *mut _, buffer.len() as u32, &mut read).map_err(|_| ())?;
            if read == 0 { break; }
            if body.len() + read as usize > 64 * 1024 { return Err(()); }
            body.extend_from_slice(&buffer[..read as usize]);
        }
        Ok((status, String::from_utf8(body).map_err(|_| ())?))
    }
}
