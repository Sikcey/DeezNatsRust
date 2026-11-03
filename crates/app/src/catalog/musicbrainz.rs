//! Optional public metadata for sparse provider track responses.
use super::Entry;
use serde_json::Value;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

fn normalized(s: &str) -> String {
    s.chars().flat_map(char::to_lowercase).filter(|c| c.is_alphanumeric()).collect()
}

pub(crate) fn enrich(track: &Entry, mut meta: Value) -> Value {
    let artist = track.subtitle.split([',', '?', '&']).next().unwrap_or("").trim();
    if track.title.trim().is_empty() || artist.is_empty() { return meta; }
    let clean = |s: &str| s.chars().map(|c| if c.is_alphanumeric() || c.is_whitespace() { c } else { ' ' }).collect::<String>();
    let query = format!("recording:\"{}\" AND artist:\"{}\"", clean(&track.title), clean(artist));
    let mut url = url::Url::parse("https://musicbrainz.org/ws/2/recording/").unwrap();
    url.query_pairs_mut().append_pair("query", &query).append_pair("fmt", "json").append_pair("limit", "10");
    static LAST: OnceLock<Mutex<Option<Instant>>> = OnceLock::new();
    let Ok(mut last) = LAST.get_or_init(|| Mutex::new(None)).lock() else { return meta; };
    if let Some(previous) = *last { std::thread::sleep(Duration::from_millis(1100).saturating_sub(previous.elapsed())); }
    *last = Some(Instant::now());
    let result = crate::lyrics::http::json(url.as_str(), &[("User-Agent", "DeezNatsRust/0.1 (https://github.com/Sikcey/DeezNats)")]);
    drop(last);
    let Ok(result) = result else { return meta; };
    let Some(rows) = result["recordings"].as_array() else { return meta; };
    let title = normalized(&track.title);
    let artist = normalized(artist);
    let Some(row) = rows.iter().filter(|row| {
        normalized(row["title"].as_str().unwrap_or("")) == title &&
        row["artist-credit"].as_array().is_some_and(|credits| credits.iter().any(|c| normalized(c["artist"]["name"].as_str().or_else(|| c["name"].as_str()).unwrap_or("")) == artist)) &&
        (track.duration == 0 || row["length"].as_u64().is_some_and(|length| length.abs_diff(track.duration as u64) <= 4000))
    }).max_by_key(|row| {
        !track.album.is_empty() && row["releases"].as_array().is_some_and(|releases| releases.iter().any(|r| normalized(r["title"].as_str().unwrap_or("")) == normalized(&track.album)))
    }) else { return meta; };
    if !meta.is_object() { meta = serde_json::json!({}); }
    let object = meta.as_object_mut().unwrap();
    if object.get("isrc").is_none_or(|v| v.is_null() || v.as_str() == Some("")) {
        if let Some(isrc) = row["isrcs"].as_array().and_then(|a| a.first()).and_then(Value::as_str) { object.insert("isrc".into(), isrc.into()); }
    }
    if !track.album.is_empty() {
        if let Some(release) = row["releases"].as_array().and_then(|releases| releases.iter().find(|r| normalized(r["title"].as_str().unwrap_or("")) == normalized(&track.album))) {
            if object.get("release_date").is_none_or(|v| v.is_null() || v.as_str() == Some("")) {
                if let Some(date) = release["date"].as_str().filter(|s| !s.is_empty()) { object.insert("release_date".into(), date.into()); }
            }
            if !object.contains_key("track_number") && !object.contains_key("track_position") {
                if let Some(number) = release["media"].as_array().and_then(|media| media.iter().flat_map(|m| m["tracks"].as_array().into_iter().flatten()).find_map(|t| t["number"].as_str())) { object.insert("track_number".into(), number.into()); }
            }
        }
    }
    meta
}
