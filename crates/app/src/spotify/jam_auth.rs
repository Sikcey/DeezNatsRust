//! Direct cookie-to-token renewal. No browser, dealer registration or playback.
use crate::lyrics::http;
use hmac::{Hmac, Mac};
use serde_json::Value;
use sha1::Sha1;
use std::{
    sync::Mutex,
    time::{SystemTime, UNIX_EPOCH},
};

static CACHE: Mutex<Option<(String, String, u64)>> = Mutex::new(None);
static RETRY_AT: Mutex<u64> = Mutex::new(0);
pub(super) fn invalidate() {
    if let Ok(mut cache) = CACHE.lock() {
        *cache = None;
    }
}
pub(super) fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
pub(super) fn cookie() -> Result<String, String> {
    let saved = crate::session::provider_saved("spotify-web")
        .map_err(|_| "Spotify Jam needs a saved Spotify sign-in.".to_string())?;
    let parsed = serde_json::from_str::<Value>(&saved).ok();
    let cookie = parsed
        .as_ref()
        .and_then(|v| v.as_str().or_else(|| v["sp_dc"].as_str()))
        .unwrap_or(&saved);
    if cookie.len() < 16
        || cookie.len() > 4096
        || !cookie
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'))
    {
        return Err("Saved Spotify sign-in is invalid.".into());
    }
    Ok(cookie.into())
}
pub(super) fn checked(response: http::Response) -> Result<Vec<u8>, String> {
    if response.status == 429 {
        let wait = response.retry_after.unwrap_or(60).clamp(1, 3600);
        if let Ok(mut retry) = RETRY_AT.lock() {
            *retry = now().saturating_add(wait);
        }
        return Err(format!("Spotify is rate limited; retry in {wait} seconds."));
    }
    if !(200..300).contains(&response.status) {
        return Err(format!(
            "Spotify Jam request failed (HTTP {}).",
            response.status
        ));
    }
    Ok(response.body)
}
pub(super) fn check_cooldown() -> Result<(), String> {
    let retry = *RETRY_AT
        .lock()
        .map_err(|_| "Spotify Jam authorization is busy")?;
    if retry > now() {
        return Err(format!(
            "Spotify is rate limited; retry in {} seconds.",
            retry - now()
        ));
    }
    Ok(())
}
fn get(url: &str) -> Result<Vec<u8>, String> {
    check_cooldown()?;
    checked(http::response("GET", url, &[], &[])?)
}
fn json(bytes: &[u8]) -> Result<Value, String> {
    serde_json::from_slice(bytes)
        .map_err(|_| "Spotify returned an invalid authorization response.".into())
}
/// Discover the current bootstrap rather than embedding an expiring TOTP version.
fn bootstrap() -> Result<(Vec<u8>, u32), String> {
    let html =
        String::from_utf8(get("https://open.spotify.com/")?).map_err(|_| "Invalid Spotify page")?;
    let url = script_url(&html).ok_or("Spotify authorization bootstrap changed.")?;
    let script = String::from_utf8(checked(http::spotify_script(url)?)?)
        .map_err(|_| "Invalid Spotify script")?;
    secret(&script)
}
fn script_url(html: &str) -> Option<&str> {
    [
        "https://open.spotifycdn.com/cdn/build/web-player/web-player.",
        "https://open.spotifycdn.com/cdn/build/mobile-web-player/mobile-web-player.",
    ]
    .iter()
    .find_map(|prefix| {
        html.match_indices(prefix).find_map(|(start, _)| {
            let candidate = html[start..].split(['"', '\'', '<', '>', ' ']).next()?;
            (candidate.len() <= 512 && candidate.ends_with(".js")).then_some(candidate)
        })
    })
}
fn secret(script: &str) -> Result<(Vec<u8>, u32), String> {
    for (_, tail) in script
        .match_indices("secret:")
        .map(|(i, _)| (i, &script[i + 7..]))
    {
        let Some(quote @ (b'\'' | b'"')) = tail.bytes().next() else {
            continue;
        };
        let mut chars = tail[1..].char_indices();
        let mut value = String::new();
        let mut end = None;
        while let Some((i, ch)) = chars.next() {
            if ch as u32 == quote as u32 {
                end = Some(i + 2);
                break;
            }
            if ch == '\\' {
                match chars.next().map(|(_, c)| c) {
                    Some(c @ ('\\' | '\'' | '"' | '/')) => value.push(c),
                    _ => {
                        value.clear();
                        break;
                    }
                }
            } else {
                value.push(ch);
            }
            if value.len() > 256 {
                break;
            }
        }
        let Some(end) = end else { continue };
        let Some(rest) = tail[end..].strip_prefix(",version:") else {
            continue;
        };
        let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
        let Ok(version) = digits.parse::<u32>() else {
            continue;
        };
        if value.is_empty() || !value.is_ascii() || version == 0 {
            continue;
        }
        let key: String = value
            .bytes()
            .enumerate()
            .map(|(i, b)| (b ^ ((i % 33 + 9) as u8)).to_string())
            .collect();
        return Ok((key.into_bytes(), version));
    }
    Err("Spotify authorization bootstrap changed; update DeezNats.".into())
}
fn totp(key: &[u8], seconds: u64) -> String {
    let mut mac = Hmac::<Sha1>::new_from_slice(key).expect("HMAC accepts arbitrary key lengths");
    mac.update(&(seconds / 30).to_be_bytes());
    let digest = mac.finalize().into_bytes();
    let offset = (digest[19] & 15) as usize;
    let code = u32::from_be_bytes(digest[offset..offset + 4].try_into().unwrap()) & 0x7fff_ffff;
    format!("{:06}", code % 1_000_000)
}
pub(super) fn token() -> Result<String, String> {
    check_cooldown()?;
    let cookie = cookie()?;
    // Serialize renewal; credentials never leave this module or enter diagnostics.
    let mut cache = CACHE
        .lock()
        .map_err(|_| "Spotify Jam authorization is busy")?;
    if let Some((identity, token, expiry)) = cache.as_ref() {
        if identity == &cookie && *expiry > now().saturating_add(60) {
            return Ok(token.clone());
        }
    }
    let (key, version) = bootstrap()?;
    let server = json(&get("https://open.spotify.com/api/server-time")?)?["serverTime"]
        .as_u64()
        .ok_or("Invalid Spotify server time")?;
    let mut url = url::Url::parse("https://open.spotify.com/api/token").unwrap();
    url.query_pairs_mut()
        .append_pair("reason", "transport")
        .append_pair("productType", "web-player")
        .append_pair("totp", &totp(&key, now()))
        .append_pair("totpServer", &totp(&key, server))
        .append_pair("totpVer", &version.to_string());
    let header = format!("sp_dc={cookie}");
    let value = json(&checked(http::response(
        "GET",
        url.as_str(),
        &[("Cookie", &header)],
        &[],
    )?)?)?;
    if value["isAnonymous"] != false {
        return Err("Spotify sign-in expired. Sign in again before using Jam.".into());
    }
    let token = value["accessToken"]
        .as_str()
        .filter(|s| !s.is_empty() && !s.contains(['\r', '\n']))
        .ok_or("Spotify did not grant a Jam token")?;
    let expiry = value["accessTokenExpirationTimestampMs"]
        .as_u64()
        .ok_or("Spotify token has no expiry")?
        / 1000;
    if expiry <= now().saturating_add(60) {
        return Err("Spotify returned an expired Jam token.".into());
    }
    *cache = Some((cookie, token.into(), expiry));
    Ok(token.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rfc6238_sha1_vectors_use_six_digits() {
        assert_eq!(totp(b"12345678901234567890", 59), "287082");
        assert_eq!(totp(b"12345678901234567890", 1111111109), "081804");
    }
    #[test]
    fn discovers_only_a_versioned_secret_and_decodes_js_quotes() {
        let (key, version) = secret("secret:unrelated;[{secret:'a\\\'b',version:61}]").unwrap();
        assert_eq!(version, 61);
        assert_eq!(key, b"10445105");
        assert!(secret("secret:'abc',version:0").is_err());
        assert!(secret("secret:'abc',other:61").is_err());
    }
    #[test]
    fn bootstrap_ignores_stylesheets_and_finds_the_actual_entry_script() {
        let url =
            "https://open.spotifycdn.com/cdn/build/mobile-web-player/mobile-web-player.abc.js";
        let html = format!("<link href='https://open.spotifycdn.com/cdn/build/mobile-web-player/mobile-web-player.abc.css'><script src='{url}'></script>");
        assert_eq!(script_url(&html), Some(url));
        assert!(script_url("https://evil.test/player.js").is_none());
    }
}
