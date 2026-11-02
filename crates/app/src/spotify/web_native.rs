//! Session system and generic HTTP plumbing. Catalog/Connect integration is separate work.
use super::*;
#[path = "web_connect.rs"]
mod connect;
use std::{
    collections::HashMap,
    sync::atomic::AtomicBool,
    time::{SystemTime, UNIX_EPOCH},
};

pub(crate) const REFRESH_REQUIRED: &str = "Spotify session expired or is unavailable. Click Refresh session in Settings > Providers > Spotify.";
static INVALID: AtomicBool = AtomicBool::new(false);
const MAX_SESSION_BYTES: usize = 6 * 1024 * 1024;
static SESSION: Mutex<Option<Value>> = Mutex::new(None);
static COOLDOWNS: Mutex<Option<HashMap<String, u64>>> = Mutex::new(None);
fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
pub(super) fn reset() {
    if let Ok(mut value) = SESSION.lock() {
        *value = None;
    }
    INVALID.store(true, Ordering::Relaxed);
    let _ = crate::session::provider_remove("spotify-web-api");
}
fn normalize_expiry(value: &mut Value) {
    if value["expiry"].as_u64().is_none() {
        if let Some(expiry) = value["expiry"]
            .as_f64()
            .filter(|v| v.is_finite() && *v >= 0.0 && *v < u64::MAX as f64 && v.fract() == 0.0)
        {
            value["expiry"] = json!(expiry as u64);
        }
    }
}
fn valid(value: &Value, timestamp: u64) -> bool {
    ["token", "client", "version"].iter().all(|key| {
        value[key]
            .as_str()
            .is_some_and(|s| !s.is_empty() && !s.contains(['\r', '\n']))
    }) && value["expiry"]
        .as_u64()
        .is_some_and(|expiry| expiry > timestamp.saturating_add(30_000))
        && value["hashes"].is_object()
}
pub(crate) fn save_session(mut value: Value) -> Result<(), String> {
    // WebView2 serializes JavaScript timestamps as floating point numbers.
    normalize_expiry(&mut value);
    if !valid(&value, now()) {
        return Err(format!("Spotify session capture incomplete (token: {}, client: {}, version: {}, expiry: {}, operations: {}). Refresh the session.",
            value["token"].as_str().is_some_and(|v| !v.is_empty()),
            value["client"].as_str().is_some_and(|v| !v.is_empty()),
            value["version"].as_str().is_some_and(|v| !v.is_empty()),
            value["expiry"].as_u64().is_some_and(|v| v > now().saturating_add(30_000)),
            value["hashes"].as_object().map_or(0, |v| v.len())));
    }
    let encoded = serde_json::to_string(&value).map_err(|_| "Could not encode Spotify session")?;
    if encoded.len() > MAX_SESSION_BYTES {
        return Err("Spotify session exceeds its size limit".into());
    }
    crate::session::provider_save("spotify-web-api", &encoded)?;
    *SESSION.lock().map_err(|_| "Spotify session unavailable")? = Some(value);
    INVALID.store(false, Ordering::Relaxed);
    Ok(())
}
fn session() -> Result<Value, String> {
    if INVALID.load(Ordering::Relaxed) {
        return Err(REFRESH_REQUIRED.into());
    }
    let mut slot = SESSION.lock().map_err(|_| "Spotify session unavailable")?;
    if slot.is_none() {
        *slot = crate::session::provider_saved("spotify-web-api")
            .ok()
            .filter(|s| s.len() <= MAX_SESSION_BYTES)
            .and_then(|s| serde_json::from_str(&s).ok());
    }
    slot.as_ref()
        .filter(|v| valid(v, now()))
        .cloned()
        .ok_or_else(|| REFRESH_REQUIRED.into())
}
fn expired() -> String {
    INVALID.store(true, Ordering::Relaxed);
    REFRESH_REQUIRED.into()
}
fn request(
    s: &Value,
    method: &str,
    address: &str,
    body: &[u8],
    content: &str,
) -> Result<(u32, Value), String> {
    let url = Url::parse(address).map_err(|_| "Invalid Spotify URL")?;
    let host = url.host_str().ok_or("Invalid Spotify host")?;
    if url.scheme() != "https"
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some()
        || !matches!(
            host,
            "api-partner.spotify.com" | "api.spotify.com" | "spclient.wg.spotify.com"
        )
    {
        return Err("Spotify credential destination rejected".into());
    }
    if COOLDOWNS
        .lock()
        .ok()
        .and_then(|c| c.as_ref().and_then(|m| m.get(host).copied()))
        .is_some_and(|until| until > now())
    {
        return Err("Spotify is rate limited; retry later".into());
    }
    let authorization = format!("Bearer {}", s["token"].as_str().ok_or(REFRESH_REQUIRED)?);
    let user_agent = s["userAgent"]
        .as_str()
        .filter(|v| !v.is_empty() && v.len() <= 1024 && !v.contains(['\r', '\n']))
        .unwrap_or("DeezNatsRust/0.1");
    let headers = [
        ("Authorization", authorization.as_str()),
        (
            "Client-Token",
            s["client"].as_str().ok_or(REFRESH_REQUIRED)?,
        ),
        (
            "Spotify-App-Version",
            s["version"].as_str().ok_or(REFRESH_REQUIRED)?,
        ),
        ("App-Platform", "WebPlayer"),
        ("Origin", "https://open.spotify.com"),
        ("Referer", "https://open.spotify.com/"),
        ("User-Agent", user_agent),
        ("Accept", "application/json"),
        ("Content-Type", content),
    ];
    let response = crate::lyrics::http::response(method, address, &headers, body)?;
    if response.status == 401 {
        return Err(expired());
    }
    if response.status == 429 {
        if let Ok(mut map) = COOLDOWNS.lock() {
            map.get_or_insert_with(HashMap::new).insert(
                host.into(),
                now().saturating_add(response.retry_after.unwrap_or(5).clamp(1, 3600) * 1000),
            );
        }
        return Err("Spotify is rate limited; retry later".into());
    }
    if !(200..300).contains(&response.status) {
        return Err(format!(
            "Spotify request failed (HTTP {}).{}",
            response.status,
            if response.status == 403 {
                " Click Refresh session; this operation may also be restricted for your account."
            } else {
                ""
            }
        ));
    }
    let value = if response.body.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice::<Value>(&response.body).map_err(|_| "Invalid Spotify response")?
    };
    if let Some(errors) = value["errors"].as_array().filter(|e| !e.is_empty()) {
        if errors.iter().any(|e| {
            e["message"].as_str().is_some_and(|m| {
                let m = m.to_ascii_lowercase();
                m.contains("unauthenticated") || m.contains("token expired")
            })
        }) {
            return Err(expired());
        }
        return Err("Spotify API operation changed or was rejected. Click Refresh session to update the API session.".into());
    }
    Ok((response.status, value))
}
pub(super) fn browse_operations() -> Result<Vec<String>, String> {
    let value = session()?;
    Ok(value["hashes"]
        .as_object()
        .into_iter()
        .flat_map(|map| map.keys())
        .filter(|name| name.to_lowercase().contains("browse"))
        .cloned()
        .collect())
}
pub(super) fn execute(command: Value) -> Result<Value, String> {
    let s = session()?;
    match command["kind"].as_str() {
        Some("devices") => connect::execute(&s, &command["command"]),
        Some("rest" | "upload") => {
            let url = command["url"].as_str().ok_or("Missing Spotify URL")?;
            super::approved_url(url)?;
            let body = if command["kind"] == "upload" {
                STANDARD
                    .decode(command["body"].as_str().ok_or("Missing upload")?)
                    .map_err(|_| "Invalid upload")?
            } else if command["body"].is_null() {
                Vec::new()
            } else {
                command["body"].to_string().into_bytes()
            };
            request(
                &s,
                command["method"].as_str().ok_or("Missing HTTP method")?,
                url,
                &body,
                command["contentType"]
                    .as_str()
                    .unwrap_or("application/json"),
            )
            .map(|(_, v)| v)
        }
        Some("query") => {
            let operation = command["operation"]
                .as_str()
                .ok_or("Missing Spotify operation")?;
            let hash = s["hashes"][operation]
                .as_str()
                .filter(|hash| hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_hexdigit()))
                .or_else(|| match operation {
                    "queryPodcastEpisodes" => {
                        Some("3539d746cf882f3909660de40b4ef472b3f5893a0761d617c282541de5d412c5")
                    }
                    "queryShowMetadataV2" => {
                        Some("b475447846f37cc426add800b995a7859c169c53c72b62de6338c0c40a5dacea")
                    }
                    "getEpisodeOrChapter" => {
                        Some("5f77db47e5a2ce6680330bb61317aa09e17ddd1d189513cb218e7cd3e43288be")
                    }
                    _ => None,
                })
                .ok_or_else(|| {
                    format!("Spotify operation {operation} is unavailable; refresh the session")
                })?;
            let body = serde_json::to_vec(&json!({
                "operationName": operation, "variables": command["variables"],
                "extensions": {"persistedQuery": {"version": 1, "sha256Hash": hash}}
            }))
            .map_err(|_| "Invalid Spotify query")?;
            request(
                &s,
                "POST",
                "https://api-partner.spotify.com/pathfinder/v2/query",
                &body,
                "application/json",
            )
            .map(|(_, value)| value)
        }
        Some("browse") => s
            .get("browse")
            .filter(|v| !v.is_null())
            .cloned()
            .ok_or_else(|| "Spotify browse content is unavailable; refresh the session".into()),
        _ => Err("Unsupported Spotify request".into()),
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn webview_timestamps_accept_integral_floats_but_not_invalid_expiries() {
        let mut value = json!({"expiry":1793151028854.0});
        normalize_expiry(&mut value);
        assert_eq!(value["expiry"].as_u64(), Some(1793151028854));
        for expiry in [
            json!(-1.0),
            json!(123.5),
            json!("1793151028854"),
            json!(1e30),
        ] {
            let mut value = json!({"expiry":expiry});
            normalize_expiry(&mut value);
            assert!(value["expiry"].as_u64().is_none());
        }
    }
    #[test]
    fn expired_and_incomplete_captures_require_explicit_refresh() {
        let mut s = json!({"token":"access","client":"client","version":"version","expiry":100000,"hashes":{}});
        assert!(valid(&s, 100));
        assert!(!valid(&s, 90000));
        s["client"] = json!("bad\r\nheader");
        assert!(!valid(&s, 100));
        assert!(!valid(&Value::Null, 0));
    }
}
