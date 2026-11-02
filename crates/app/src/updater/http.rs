//! Unauthenticated HTTPS release downloads. Redirects may not downgrade to HTTP.
use std::{
    ffi::c_void,
    io::Write,
    ptr::{null, null_mut},
    time::{Duration, Instant},
};
use windows_sys::Win32::Networking::WinHttp::*;
struct Handle(*mut c_void);
pub const NO_RELEASE: &str = "No published update is available yet.";
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

pub fn get(
    url: &str,
    mut output: impl Write,
    limit: u64,
    mut progress: impl FnMut(u64),
) -> Result<u64, String> {
    let rest = url
        .strip_prefix("https://")
        .ok_or("Update URLs must use HTTPS.")?;
    let (host, path) = rest.split_once('/').ok_or("Invalid update URL.")?;
    if !matches!(
        host,
        "api.github.com" | "github.com" | "soloist-builds.spotifycdn.com"
    ) || path.contains(['\r', '\n', '\0'])
    {
        return Err("Invalid update server.".into());
    }
    unsafe {
        let session = Handle(WinHttpOpen(
            wide("DeezNatsRust-Updater/1").as_ptr(),
            WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY,
            null(),
            null(),
            0,
        ));
        if session.0.is_null() {
            return Err("Could not initialize update download.".into());
        }
        WinHttpSetTimeouts(session.0, 5000, 5000, 10000, 15000);
        let connection = Handle(WinHttpConnect(session.0, wide(host).as_ptr(), 443, 0));
        if connection.0.is_null() {
            return Err("Could not connect to update server.".into());
        }
        let request = Handle(WinHttpOpenRequest(
            connection.0,
            wide("GET").as_ptr(),
            wide(&format!("/{path}")).as_ptr(),
            null(),
            null(),
            null(),
            WINHTTP_FLAG_SECURE,
        ));
        if request.0.is_null() {
            return Err("Could not create update request.".into());
        }
        for (option, value) in [
            (WINHTTP_OPTION_DISABLE_FEATURE, WINHTTP_DISABLE_COOKIES),
            (
                WINHTTP_OPTION_REDIRECT_POLICY,
                WINHTTP_OPTION_REDIRECT_POLICY_DISALLOW_HTTPS_TO_HTTP,
            ),
            (
                WINHTTP_OPTION_AUTOLOGON_POLICY,
                WINHTTP_AUTOLOGON_SECURITY_LEVEL_HIGH,
            ),
        ] {
            if WinHttpSetOption(request.0, option, (&value as *const u32).cast(), 4) == 0 {
                return Err("Could not configure secure update request.".into());
            }
        }
        if WinHttpSendRequest(request.0, null(), 0, null(), 0, 0, 0) == 0
            || WinHttpReceiveResponse(request.0, null_mut()) == 0
        {
            return Err("Update request failed or timed out.".into());
        }
        let mut status = 0u32;
        let mut length = 4;
        if WinHttpQueryHeaders(
            request.0,
            WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
            null(),
            (&mut status as *mut u32).cast(),
            &mut length,
            null_mut(),
        ) == 0
        {
            return Err("Invalid update response.".into());
        }
        if status == 404 {
            return Err(NO_RELEASE.into());
        }
        if status != 200 {
            return Err(format!("Update server returned HTTP {status}."));
        }
        let start = Instant::now();
        let mut total = 0u64;
        let mut chunk = [0u8; 65536];
        loop {
            if start.elapsed() > Duration::from_secs(300) {
                return Err("Update download timed out.".into());
            }
            let mut count = 0;
            if WinHttpReadData(
                request.0,
                chunk.as_mut_ptr().cast(),
                chunk.len() as u32,
                &mut count,
            ) == 0
            {
                return Err("Update download was interrupted.".into());
            }
            if count == 0 {
                break;
            }
            total += count as u64;
            if total > limit {
                return Err("Update download exceeded its expected size.".into());
            }
            output
                .write_all(&chunk[..count as usize])
                .map_err(|e| e.to_string())?;
            progress(total);
        }
        Ok(total)
    }
}

pub fn bytes(url: &str, limit: u64) -> Result<Vec<u8>, String> {
    let mut data = Vec::new();
    get(url, &mut data, limit, |_| {})?;
    Ok(data)
}
