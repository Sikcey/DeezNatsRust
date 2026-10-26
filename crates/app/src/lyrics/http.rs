//! Small bounded Windows HTTP transport for provider headers and POST bodies.
use std::{
    ffi::c_void,
    ptr::{null, null_mut},
    sync::atomic::{AtomicU64, Ordering},
    sync::OnceLock,
    time::{Duration, Instant},
};
use windows_sys::Win32::Networking::WinHttp::*;
static SPOTIFY_RETRY_AT: AtomicU64 = AtomicU64::new(0);
fn epoch() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
pub(crate) fn spotify_cooldown() -> Option<u64> {
    let seconds = SPOTIFY_RETRY_AT
        .load(Ordering::Relaxed)
        .saturating_sub(epoch());
    (seconds > 0).then_some(seconds)
}
fn retry_seconds(header: &str) -> u64 {
    header
        .trim()
        .parse::<u64>()
        .ok()
        .filter(|seconds| *seconds > 0)
        .unwrap_or(60)
}
struct Handle(*mut c_void);
unsafe impl Send for Handle {}
unsafe impl Sync for Handle {}
impl Drop for Handle {
    fn drop(&mut self) {
        unsafe {
            WinHttpCloseHandle(self.0);
        }
    }
}
fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(Some(0)).collect()
}
fn session() -> Result<&'static Handle, String> {
    static SESSION: OnceLock<Result<Handle, String>> = OnceLock::new();
    SESSION
        .get_or_init(|| unsafe {
            let agent = wide("DeezNatsRust/0.1");
            let handle = WinHttpOpen(
                agent.as_ptr(),
                WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY,
                null(),
                null(),
                0,
            );
            if handle.is_null() {
                return Err("HTTP initialization failed".into());
            }
            WinHttpSetTimeouts(handle, 3000, 4000, 4000, 6000);
            Ok(Handle(handle))
        })
        .as_ref()
        .map_err(Clone::clone)
}
fn artwork_session() -> Result<&'static Handle, String> {
    static SESSION: OnceLock<Result<Handle, String>> = OnceLock::new();
    SESSION.get_or_init(|| unsafe {
        let handle=WinHttpOpen(wide("DeezNatsArtwork/1").as_ptr(),WINHTTP_ACCESS_TYPE_NO_PROXY,null(),null(),0);
        if handle.is_null(){return Err("Artwork HTTP initialization failed".into());}
        WinHttpSetTimeouts(handle,3000,3000,4000,6000);
        Ok(Handle(handle))
    }).as_ref().map_err(Clone::clone)
}
pub(crate) fn artwork(url: &str) -> Result<Vec<u8>, String> {
    request_transport("GET",url,&[],&[],16*1024*1024,true)
}
pub(crate) fn request(
    method: &str,
    url: &str,
    headers: &[(&str, &str)],
    body: &[u8],
) -> Result<Vec<u8>, String> {
    request_bounded(method, url, headers, body, 2 * 1024 * 1024)
}

fn request_bounded(
    method: &str,
    url: &str,
    headers: &[(&str, &str)],
    body: &[u8],
    limit: usize,
) -> Result<Vec<u8>, String> {
    request_transport(method,url,headers,body,limit,false)
}
fn request_transport(method:&str,url:&str,headers:&[(&str,&str)],body:&[u8],limit:usize,direct:bool)->Result<Vec<u8>,String>{
    let rest = url
        .strip_prefix("https://")
        .ok_or("Provider URL must use HTTPS")?;
    let (host, path) = rest
        .split_once('/')
        .map(|(h, p)| (h, format!("/{p}")))
        .unwrap_or((rest, "/".into()));
    if host.is_empty()
        || !host
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-'))
    {
        return Err("Invalid provider host".into());
    }
    if body.len() > 1024 * 1024 {
        return Err("Provider request too large".into());
    }
    if host.eq_ignore_ascii_case("api.spotify.com") {
        if let Some(seconds) = spotify_cooldown() {
            return Err(format!(
                "Provider HTTP 429: Spotify requests paused; try again in {seconds} seconds"
            ));
        }
    }
    let mut header = String::new();
    for (name, value) in headers {
        if name.contains(['\r', '\n']) || value.contains(['\r', '\n']) {
            return Err("Invalid provider header".into());
        }
        header.push_str(&format!("{name}: {value}\r\n"));
    }
    unsafe {
        let server = wide(host);
        let transport=if direct{artwork_session()?}else{session()?};
        let connect = Handle(WinHttpConnect(transport.0, server.as_ptr(), 443, 0));
        if connect.0.is_null() {
            return Err("Provider connection failed".into());
        }
        let method = wide(method);
        let path = wide(&path);
        let request = Handle(WinHttpOpenRequest(
            connect.0,
            method.as_ptr(),
            path.as_ptr(),
            null(),
            null(),
            null(),
            WINHTTP_FLAG_SECURE,
        ));
        if request.0.is_null() {
            return Err("Provider request failed".into());
        }
        // Never forward credentials through redirects or keep provider cookies globally.
        let disabled = WINHTTP_DISABLE_REDIRECTS | WINHTTP_DISABLE_COOKIES;
        WinHttpSetOption(
            request.0,
            WINHTTP_OPTION_DISABLE_FEATURE,
            (&disabled as *const u32).cast(),
            4,
        );
        let headers = wide(&header);
        if WinHttpSendRequest(
            request.0,
            headers.as_ptr(),
            header.encode_utf16().count() as u32,
            if body.is_empty() {
                null()
            } else {
                body.as_ptr().cast()
            },
            body.len() as u32,
            body.len() as u32,
            0,
        ) == 0
            || WinHttpReceiveResponse(request.0, null_mut()) == 0
        {
            return Err("Provider request timed out or failed".into());
        }
        let mut status = 0u32;
        let mut size = 4;
        if WinHttpQueryHeaders(
            request.0,
            WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
            null(),
            (&mut status as *mut u32).cast(),
            &mut size,
            null_mut(),
        ) == 0
        {
            return Err("Invalid provider response".into());
        }
        if status == 429 && host.eq_ignore_ascii_case("api.spotify.com") {
            let mut header = [0u16; 128];
            let mut bytes = (header.len() * 2) as u32;
            let name = wide("Retry-After");
            let seconds = if WinHttpQueryHeaders(
                request.0,
                WINHTTP_QUERY_CUSTOM,
                name.as_ptr(),
                header.as_mut_ptr().cast(),
                &mut bytes,
                null_mut(),
            ) != 0
            {
                retry_seconds(&String::from_utf16_lossy(
                    &header[..header.iter().position(|c| *c == 0).unwrap_or(header.len())],
                ))
            } else {
                60
            };
            SPOTIFY_RETRY_AT.fetch_max(epoch().saturating_add(seconds), Ordering::Relaxed);
            return Err(format!("Provider HTTP 429: Spotify rate limit or quota reached; try again in {seconds} seconds"));
        }
        if !(200..300).contains(&status) {
            return Err(format!("Provider HTTP {status}"));
        }
        let started = Instant::now();
        let mut result = Vec::new();
        let mut chunk = [0u8; 16384];
        loop {
            if started.elapsed() > Duration::from_secs(12) {
                return Err("Provider response timed out".into());
            }
            let mut count = 0;
            if WinHttpReadData(
                request.0,
                chunk.as_mut_ptr().cast(),
                chunk.len() as u32,
                &mut count,
            ) == 0
            {
                return Err("Provider read failed".into());
            }
            if count == 0 {
                break;
            }
            if result.len() + count as usize > limit {
                return Err("Provider response too large".into());
            }
            result.extend_from_slice(&chunk[..count as usize]);
        }
        Ok(result)
    }
}
pub(super) fn json(url: &str, headers: &[(&str, &str)]) -> Result<serde_json::Value, String> {
    serde_json::from_slice(&request("GET", url, headers, &[])?)
        .map_err(|_| "Invalid provider JSON".into())
}
pub(super) fn post_json(url: &str, value: &serde_json::Value) -> Result<serde_json::Value, String> {
    let body = serde_json::to_vec(value).map_err(|_| "Invalid request JSON")?;
    serde_json::from_slice(&request(
        "POST",
        url,
        &[("Content-Type", "application/json")],
        &body,
    )?)
    .map_err(|_| "Invalid provider JSON".into())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn retry_after_uses_server_delay_without_shortening_long_limits() {
        assert_eq!(retry_seconds("120"), 120);
        assert_eq!(retry_seconds("86400"), 86400);
        assert_eq!(retry_seconds("invalid"), 60);
        assert_eq!(retry_seconds("0"), 60);
    }
    #[test]
    fn rejects_unsafe_urls_and_header_injection_before_network() {
        assert!(request("GET", "http://example.com", &[], &[]).is_err());
        assert!(request("GET", "https://user@example.com", &[], &[]).is_err());
        assert!(request("GET", "https://example.com", &[("Cookie", "x\r\ny")], &[]).is_err());
    }
}
