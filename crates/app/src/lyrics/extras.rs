//! Independent adapters for public lyrics endpoints. Native HTTP, bounded replies,
//! metadata matching and provider-owned timing; no playback SDK is loaded here.
use super::{http, url_encode, Entry, LyricLine, Syllable};
use base64::Engine as _;
use serde_json::{json, Value};
use std::{
    sync::{Mutex, OnceLock},
    time::{Duration, Instant},
};
fn artist(track: &Entry) -> &str {
    track
        .raw
        .pointer("/artists/0/name")
        .and_then(Value::as_str)
        .unwrap_or_else(|| track.subtitle.split(", ").next().unwrap_or(&track.subtitle))
        .trim()
}
fn normalized(text: &str) -> String {
    text.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}
fn base_title(title: &str) -> &str {
    let lower = title.to_ascii_lowercase();
    [" (feat.", " (with "]
        .iter()
        .filter_map(|p| lower.find(p))
        .min()
        .map(|n| &title[..n])
        .unwrap_or(title)
}
fn matches(track: &Entry, title: &str, credited: &str, duration_ms: u64) -> bool {
    let exact_title = normalized(base_title(&track.title)) == normalized(base_title(title));
    let feature_alias = [" (with ", " (feat."]
        .iter()
        .find_map(|marker| {
            let lower = track.title.to_ascii_lowercase();
            let start = lower.find(marker)? + marker.len();
            let guest = track.title[start..].trim_end_matches(')').trim();
            Some(
                !guest.is_empty()
                    && normalized(title).starts_with(&normalized(base_title(&track.title)))
                    && normalized(title).contains(&normalized(guest)),
            )
        })
        .unwrap_or(false);
    (exact_title || feature_alias)
        && credited
            .split([',', '&', '/', '\u{3001}'])
            .any(|name| normalized(name) == normalized(artist(track)))
        && (track.duration == 0 || duration_ms == 0 || track.duration.abs_diff(duration_ms) <= 4000)
}
fn form(fields: &[(&str, &str)]) -> String {
    fields
        .iter()
        .map(|(k, v)| format!("{}={}", url_encode(k), url_encode(v)))
        .collect::<Vec<_>>()
        .join("&")
}
fn millis(value: &Value) -> Option<u64> {
    value.as_u64().or_else(|| value.as_str()?.parse().ok())
}
fn seconds(value: &Value) -> u64 {
    value
        .as_f64()
        .filter(|n| n.is_finite() && *n >= 0.0 && *n <= 86400.0)
        .map(|n| (n * 1000.0).round() as u64)
        .unwrap_or(0)
}
fn line(start: u64, end: u64, text: &str, words: Vec<Value>) -> Value {
    json!({"start_ms":start,"end_ms":end,"text":text,"words":words})
}
fn word(start: u64, end: u64, text: &str) -> Value {
    json!({"start_ms":start,"end_ms":end,"text":text})
}
pub(super) fn parse(value: &Value) -> Vec<LyricLine> {
    value["lines"]
        .as_array()
        .into_iter()
        .flatten()
        .take(2000)
        .filter_map(|v| {
            let text = v["text"].as_str()?.trim();
            if text.is_empty() {
                return None;
            }
            let mut syllables: Vec<_> = v["words"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|w| {
                    let start_ms = millis(&w["start_ms"])?;
                    let end_ms = millis(&w["end_ms"])?;
                    (end_ms > start_ms).then(|| Syllable {
                        start_ms,
                        end_ms,
                        text: w["text"].as_str().unwrap_or_default().into(),
                    })
                })
                .collect();
            if syllables.len() > 1
                && syllables
                    .iter()
                    .all(|w| w.start_ms == syllables[0].start_ms && w.end_ms == syllables[0].end_ms)
            {
                // Some providers repeat a whole-line span on every token. That
                // remains line timing, not a genuine word-synchronized result.
                syllables.clear();
            }
            Some(LyricLine {
                start_ms: millis(&v["start_ms"]).unwrap_or(0),
                end_ms: millis(&v["end_ms"]).unwrap_or(0),
                text: text.into(),
                syllables,
                translation: String::new(),
            })
        })
        .collect()
}

// Apple-origin TTML is supplied by a public catalogue, independently of Apple playback.
pub(super) fn apple(track: &Entry) -> Result<Value, String> {
    let duration = (track.duration / 1000).to_string();
    let mut fields = vec![("track", track.title.as_str()), ("artist", artist(track))];
    if !track.album.is_empty() {
        fields.push(("album", &track.album));
    }
    if track.duration > 0 {
        fields.push(("duration", &duration));
    }
    let query = form(&fields);
    let value = http::json(&format!("https://lrc.red/api/v1?{query}"), &[])?;
    let mut candidates: Vec<_> = value["results"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|v| {
            matches(
                track,
                v["track_name"].as_str().unwrap_or_default(),
                v["artist_name"].as_str().unwrap_or_default(),
                v["duration"].as_u64().unwrap_or(0).saturating_mul(1000),
            )
        })
        .collect();
    candidates.sort_by_key(|v| {
        (
            !matches!(v["timing_type"].as_str(), Some("word" | "syllable")),
            v["duration"]
                .as_u64()
                .unwrap_or(0)
                .saturating_mul(1000)
                .abs_diff(track.duration),
        )
    });
    let url = candidates
        .iter()
        .find_map(|v| v["lyricsUrl"].as_str())
        .ok_or("No matching Apple lyrics")?;
    let text = String::from_utf8(http::request("GET", url, &[], &[])?)
        .map_err(|_| "Invalid Apple TTML")?;
    Ok(json!({"ttml":text}))
}

fn xml_text<'a, 'input>(node: roxmltree::Node<'a, 'input>, tag: &str) -> &'a str {
    node.children()
        .find(|n| n.has_tag_name(tag))
        .and_then(|n| n.text())
        .unwrap_or_default()
}
fn petit_request(track: &Entry, tier: &str) -> Result<String, String> {
    let data = form(&[
        ("clientAppId", "p1110417"),
        ("terminalType", "10"),
        ("lyricsType", tier),
        ("key_title", &track.title),
        ("key_artist", artist(track)),
        ("key_album", ""),
    ]);
    String::from_utf8(http::request(
        "POST",
        "https://p0.petitlyrics.com/api/GetPetitLyricsData.php",
        &[("Content-Type", "application/x-www-form-urlencoded")],
        data.as_bytes(),
    )?)
    .map_err(|_| "Invalid PetitLyrics response".into())
}
pub(super) fn petit(track: &Entry) -> Result<Value, String> {
    let response = petit_request(track, "3")?;
    let doc = roxmltree::Document::parse(&response).map_err(|_| "Invalid PetitLyrics XML")?;
    let candidate = doc
        .descendants()
        .filter(|node| node.children().any(|n| n.has_tag_name("lyricsData")))
        .filter(|node| {
            matches(
                track,
                xml_text(*node, "title"),
                xml_text(*node, "artist"),
                xml_text(*node, "duration").parse().unwrap_or(0),
            )
        })
        .max_by_key(|n| xml_text(*n, "lyricsType").parse::<u8>().unwrap_or(0))
        .ok_or("No matching PetitLyrics recording")?;
    let payload = base64::engine::general_purpose::STANDARD
        .decode(xml_text(candidate, "lyricsData"))
        .map_err(|_| "Invalid PetitLyrics encoding")?;
    if let Ok(text) = std::str::from_utf8(&payload) {
        if text.contains("<wsy") {
            return petit_words(text);
        }
        if !payload.contains(&0) {
            return Ok(json!({"plainLyrics":text}));
        }
    }
    // A line-synced binary needs a separate matching plain payload. Never guess
    // timings from text when the tier or line count doesn't agree.
    let times = petit_times(&payload)?;
    let plain = petit_request(track, "1")?;
    let doc =
        roxmltree::Document::parse(&plain).map_err(|_| "Invalid PetitLyrics text response")?;
    let id = xml_text(candidate, "lyricsId");
    let node = doc
        .descendants()
        .find(|n| xml_text(*n, "lyricsId") == id && !id.is_empty())
        .ok_or("PetitLyrics text identity mismatch")?;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(xml_text(node, "lyricsData"))
        .map_err(|_| "Invalid PetitLyrics text encoding")?;
    let plain = std::str::from_utf8(&bytes).map_err(|_| "Invalid PetitLyrics text")?;
    let text: Vec<_> = plain.trim_end_matches(['\r', '\n']).lines().collect();
    if text.len() != times.len() {
        return Err("PetitLyrics timing/text mismatch".into());
    }
    Ok(
        json!({"lines":text.iter().enumerate().map(|(i,text)|line(times[i],times.get(i+1).copied().unwrap_or(track.duration).max(times[i]),text,vec![])).collect::<Vec<_>>()}),
    )
}
fn petit_words(text: &str) -> Result<Value, String> {
    let doc = roxmltree::Document::parse(text).map_err(|_| "Invalid PetitLyrics word timing")?;
    let lines: Vec<_> = doc
        .descendants()
        .filter(|n| n.has_tag_name("line"))
        .take(2000)
        .filter_map(|node| {
            let words: Vec<_> = node
                .children()
                .filter(|n| n.has_tag_name("word"))
                .filter_map(|w| {
                    let start = xml_text(w, "starttime").parse::<u64>().ok()?;
                    let end = xml_text(w, "endtime").parse::<u64>().ok()?;
                    (end > start).then(|| word(start, end, xml_text(w, "wordstring")))
                })
                .collect();
            let start = words.first()?["start_ms"].as_u64()?;
            let end = words.iter().filter_map(|w| w["end_ms"].as_u64()).max()?;
            let joined = words
                .iter()
                .filter_map(|w| w["text"].as_str())
                .collect::<String>();
            let text = xml_text(node, "linestring");
            let text = if text.is_empty() {
                joined.as_str()
            } else {
                text
            };
            Some(line(start, end, text, words))
        })
        .collect();
    Ok(json!({"lines":lines}))
}
fn petit_times(raw: &[u8]) -> Result<Vec<u64>, String> {
    if raw.len() < 204 || !raw.starts_with(b"MHDROBJT") {
        return Err("Unsupported PetitLyrics timing format".into());
    }
    let count = u32::from_le_bytes(raw[56..60].try_into().unwrap()) as usize;
    let width = u16::from_le_bytes(raw[66..68].try_into().unwrap()) as usize;
    if count > 2000
        || 204 + count * 2 > raw.len()
        || (width > 0 && 204 + count * (2 + width) != raw.len())
    {
        return Err("Invalid PetitLyrics timing bounds".into());
    }
    let key = u16::from_le_bytes(raw[26..28].try_into().unwrap());
    let key = if raw[25] != 0 {
        (key & 3)
            | ((key & 12) << 2)
            | ((key & 48) >> 2)
            | ((key & 192) << 2)
            | ((key & 768) >> 2)
            | ((key & 3072) << 2)
            | ((key & 12288) >> 2)
            | (key & 49152)
    } else {
        key
    };
    let mut epoch = 0;
    let mut last = 0;
    let mut out = Vec::with_capacity(count);
    for i in 0..count {
        let t = u16::from_le_bytes(raw[204 + i * 2..206 + i * 2].try_into().unwrap()) ^ key;
        if i > 0 && t < last {
            epoch += 65536;
        }
        last = t;
        out.push((epoch + t as u64) * 10);
    }
    Ok(out)
}

#[derive(Default)]
struct MxmState {
    token: Option<String>,
    last: Option<Instant>,
    blocked: Option<Instant>,
}
fn mxm_state() -> &'static Mutex<MxmState> {
    static STATE: OnceLock<Mutex<MxmState>> = OnceLock::new();
    STATE.get_or_init(|| Mutex::new(MxmState::default()))
}
const MXM_APP: &str = "web-desktop-app-v1.0";
fn mxm_get(endpoint: &str, query: &str) -> Result<Value, String> {
    http::json(&format!("https://apic-desktop.musixmatch.com/ws/1.1/{endpoint}?{query}"),&[("Cookie","x-mxm-token-guid="),("User-Agent","Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 Chrome/124.0.0.0 Safari/537.36")])
}
pub(super) fn musixmatch(track: &Entry) -> Result<Value, String> {
    let mut gate = mxm_state()
        .lock()
        .map_err(|_| "Musixmatch state unavailable")?;
    if gate.blocked.is_some_and(|until| Instant::now() < until) {
        return Err("Musixmatch temporarily unavailable; cooling down".into());
    }
    if let Some(last) = gate.last {
        if last.elapsed() < Duration::from_millis(1500) {
            std::thread::sleep(Duration::from_millis(1500) - last.elapsed());
        }
    }
    gate.last = Some(Instant::now());
    for _ in 0..2 {
        if gate.token.is_none() {
            gate.token = crate::session::provider_saved("musixmatch")
                .ok()
                .filter(|t| !t.is_empty());
            if gate.token.is_none() {
                let value = mxm_get(
                    "token.get",
                    &form(&[("format", "json"), ("app_id", MXM_APP)]),
                );
                let value = match value {
                    Ok(v) => v,
                    Err(error) => {
                        gate.blocked = Some(Instant::now() + Duration::from_secs(600));
                        return Err(error);
                    }
                };
                let token = value
                    .pointer("/message/body/user_token")
                    .and_then(Value::as_str)
                    .filter(|s| !s.is_empty() && !s.contains("UpgradeOnly"));
                let Some(token) = token else {
                    gate.blocked = Some(Instant::now() + Duration::from_secs(600));
                    return Err("Musixmatch did not issue a usable token".into());
                };
                let _ = crate::session::provider_save("musixmatch", token);
                gate.token = Some(token.into());
            }
        }
        let query = form(&[
            ("format", "json"),
            ("namespace", "lyrics_richsynched"),
            ("subtitle_format", "mxm"),
            ("optional_calls", "track.richsync"),
            ("app_id", MXM_APP),
            ("usertoken", gate.token.as_deref().unwrap()),
            ("q_track", &track.title),
            ("q_artist", artist(track)),
            ("q_duration", &(track.duration / 1000).to_string()),
        ]);
        let value = match mxm_get("macro.subtitles.get", &query) {
            Ok(v) => v,
            Err(error) => {
                gate.blocked = Some(Instant::now() + Duration::from_secs(600));
                return Err(error);
            }
        };
        let status = value
            .pointer("/message/header/status_code")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        if status == 401 {
            gate.token = None;
            let _ = crate::session::provider_remove("musixmatch");
            continue;
        }
        if status != 200 {
            if matches!(status, 402 | 403 | 429) {
                gate.blocked = Some(Instant::now() + Duration::from_secs(600));
            }
            return Err(format!("Musixmatch status {status}"));
        }
        let calls = value
            .pointer("/message/body/macro_calls")
            .ok_or("Musixmatch returned no lyrics")?;
        let metadata = &calls["matcher.track.get"]["message"]["body"]["track"];
        if !matches(
            track,
            metadata["track_name"].as_str().unwrap_or_default(),
            metadata["artist_name"].as_str().unwrap_or_default(),
            metadata["track_length"]
                .as_u64()
                .unwrap_or(0)
                .saturating_mul(1000),
        ) {
            gate.blocked = Some(Instant::now() + Duration::from_secs(600));
            return Err(
                "Musixmatch returned a different recording; ignored and cooling down".into(),
            );
        }
        return mxm_parse(calls);
    }
    gate.blocked = Some(Instant::now() + Duration::from_secs(600));
    Err("Musixmatch authentication unavailable".into())
}
fn mxm_parse(calls: &Value) -> Result<Value, String> {
    if let Some(text) = calls
        .pointer("/track.richsync.get/message/body/richsync/richsync_body")
        .and_then(Value::as_str)
    {
        let raw: Value =
            serde_json::from_str(text).map_err(|_| "Invalid Musixmatch word timing")?;
        let lines: Vec<_> = raw
            .as_array()
            .into_iter()
            .flatten()
            .take(2000)
            .map(|v| {
                let start = seconds(&v["ts"]);
                let end = seconds(&v["te"]);
                let words: Vec<_> = v["l"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .enumerate()
                    .map(|(i, w)| {
                        let ws = start.saturating_add(seconds(&w["o"]));
                        let we = v["l"]
                            .get(i + 1)
                            .map(|n| start.saturating_add(seconds(&n["o"])))
                            .unwrap_or(end);
                        word(ws, we, w["c"].as_str().unwrap_or_default())
                    })
                    .collect();
                line(start, end, v["x"].as_str().unwrap_or_default(), words)
            })
            .collect();
        if !lines.is_empty() {
            return Ok(json!({"lines":lines}));
        }
    }
    let text = calls
        .pointer("/track.subtitles.get/message/body/subtitle_list/0/subtitle/subtitle_body")
        .and_then(Value::as_str)
        .ok_or("Musixmatch returned no timed lyrics")?;
    if let Ok(cues) = serde_json::from_str::<Value>(text) {
        let lines: Vec<_> = cues
            .as_array()
            .into_iter()
            .flatten()
            .enumerate()
            .map(|(i, cue)| {
                line(
                    seconds(&cue["time"]["total"]),
                    cues.get(i + 1)
                        .map(|v| seconds(&v["time"]["total"]))
                        .unwrap_or(0),
                    cue["text"].as_str().unwrap_or_default(),
                    vec![],
                )
            })
            .collect();
        Ok(json!({"lines":lines}))
    } else {
        Ok(json!({"syncedLyrics":text}))
    }
}

fn renderers<'a>(value: &'a Value, name: &str, out: &mut Vec<&'a Value>, depth: usize) {
    if depth > 32 || out.len() > 100 {
        return;
    }
    if let Some(v) = value.get(name) {
        out.push(v);
    }
    match value {
        Value::Object(map) => {
            for v in map.values() {
                renderers(v, name, out, depth + 1)
            }
        }
        Value::Array(values) => {
            for v in values {
                renderers(v, name, out, depth + 1)
            }
        }
        _ => {}
    }
}
fn text(value: &Value) -> String {
    value["runs"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|v| v["text"].as_str())
        .collect()
}
fn innertube(route: &str, mut body: Value) -> Result<Value, String> {
    body["context"] = json!({"client":{"clientName":"WEB_REMIX","clientVersion":"1.20261021.01.00","hl":"en","gl":"US"}});
    http::post_json(
        &format!("https://music.youtube.com/youtubei/v1/{route}?prettyPrint=false"),
        &body,
    )
}
fn lyrics_browse_id(value: &Value, depth: usize) -> Option<&str> {
    if depth > 64 {
        return None;
    }
    if let Some(endpoint) = value.get("browseEndpoint") {
        if let Some(id) = endpoint["browseId"]
            .as_str()
            .filter(|s| s.starts_with("MPLYt"))
        {
            return Some(id);
        }
    }
    match value {
        Value::Object(map) => map.values().find_map(|v| lyrics_browse_id(v, depth + 1)),
        Value::Array(values) => values.iter().find_map(|v| lyrics_browse_id(v, depth + 1)),
        _ => None,
    }
}
fn youtube_sheet(id: &str) -> Result<Option<String>, String> {
    let next = innertube(
        "next",
        json!({"videoId":id,"playlistId":format!("RDAMVM{id}"),"enablePersistentPlaylistPanel":true}),
    )?;
    let Some(id) = lyrics_browse_id(&next, 0) else {
        return Ok(None);
    };
    let page = innertube("browse", json!({"browseId":id}))?;
    let mut shelves = Vec::new();
    renderers(&page, "musicDescriptionShelfRenderer", &mut shelves, 0);
    Ok(shelves
        .iter()
        .map(|v| text(&v["description"]))
        .find(|s| !s.trim().is_empty()))
}
pub(super) fn youtube(track: &Entry) -> Result<Value, String> {
    let response = innertube(
        "search",
        json!({"query":format!("{} {}",track.title,artist(track)),"params":"EgWKAQIIAWoKEAkQBRAKEAMQBA%3D%3D"}),
    )?;
    let mut rows = Vec::new();
    renderers(&response, "musicResponsiveListItemRenderer", &mut rows, 0);
    for row in rows.into_iter().take(8) {
        let columns = &row["flexColumns"];
        let title = text(&columns[0]["musicResponsiveListItemFlexColumnRenderer"]["text"]);
        let artist_text = text(&columns[1]["musicResponsiveListItemFlexColumnRenderer"]["text"]);
        let credited = artist_text
            .split('\u{2022}')
            .next()
            .unwrap_or(&artist_text)
            .trim();
        let duration = artist_text
            .rsplit('\u{2022}')
            .next()
            .and_then(|s| {
                let parts: Vec<_> = s.trim().split(':').collect();
                if parts.len() != 2 {
                    return None;
                }
                Some(
                    parts[0]
                        .parse::<u64>()
                        .ok()?
                        .checked_mul(60)?
                        .checked_add(parts[1].parse().ok()?)?
                        .saturating_mul(1000),
                )
            })
            .unwrap_or(0);
        if !matches(track, &title, credited, duration) {
            continue;
        }
        if let Some(id) = row
            .pointer("/playlistItemData/videoId")
            .and_then(Value::as_str)
        {
            if let Some(plain) = youtube_sheet(id)? {
                return Ok(json!({"plainLyrics":plain}));
            }
        }
    }
    Err("No matching YouTube Music lyrics".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn live_track() -> Entry {
        Entry {
            id: "test".into(),
            kind: "track".into(),
            title: "Yellow".into(),
            subtitle: "Coldplay".into(),
            duration: 269000,
            ..Default::default()
        }
    }
    fn report(name: &str, value: Result<Value, String>) {
        let value = value.unwrap_or_else(|error| panic!("{name}: {error}"));
        let lines = if value["ttml"].is_string() {
            super::super::formats::ttml(value["ttml"].as_str().unwrap())
        } else if value["lines"].is_array() {
            parse(&value)
        } else {
            super::super::parse_lrclib(&value)
        };
        assert!(!lines.is_empty(), "{name}: no parsed lyrics");
        println!(
            "{name}: {} lines, {} word-timed lines",
            lines.len(),
            lines.iter().filter(|l| !l.syllables.is_empty()).count()
        );
    }
    #[test]
    fn repeated_line_spans_are_not_claimed_as_word_timing() {
        let value = json!({"lines":[{"start_ms":1000,"end_ms":2000,"text":"One two","words":[
            {"start_ms":1000,"end_ms":2000,"text":"One "},{"start_ms":1000,"end_ms":2000,"text":"two"}]}]});
        let lines = parse(&value);
        assert!(lines[0].syllables.is_empty());
        assert_eq!((lines[0].start_ms, lines[0].end_ms), (1000, 2000));
    }
    #[test]
    fn youtube_finds_lyrics_after_a_long_upcoming_queue() {
        let mut values: Vec<_> = (0..150)
            .map(|n| json!({"browseEndpoint":{"browseId":format!("artist-{n}")}}))
            .collect();
        values.push(json!({"browseEndpoint":{"browseId":"MPLYt-test"}}));
        assert_eq!(lyrics_browse_id(&json!(values), 0), Some("MPLYt-test"));
    }
    #[test]
    fn musixmatch_preserves_offsets_and_does_not_make_up_word_timings() {
        let calls = json!({"track.richsync.get":{"message":{"body":{"richsync":{"richsync_body":r#"[{"ts":1.0,"te":2.0,"x":"Hello world","l":[{"c":"Hello ","o":0.0},{"c":"world","o":0.5}]}]"#}}}}});
        let lines = parse(&mxm_parse(&calls).unwrap());
        assert_eq!(lines[0].syllables[1].start_ms, 1500);
        assert_eq!(lines[0].syllables[1].end_ms, 2000);
        let fallback = json!({"track.subtitles.get":{"message":{"body":{"subtitle_list":[{"subtitle":{"subtitle_body":r#"[{"text":"One","time":{"total":1.0}},{"text":"Two","time":{"total":2.0}}]"#}}]}}}});
        assert!(parse(&mxm_parse(&fallback).unwrap())
            .iter()
            .all(|l| l.syllables.is_empty()));
    }
    #[test]
    #[ignore = "Opt-in public network provider test"]
    fn live_petitlyrics() {
        report("PetitLyrics", petit(&live_track()));
    }
    #[test]
    #[ignore = "Opt-in public network provider test"]
    fn live_binimum() {
        report("Apple Music", apple(&live_track()));
    }
    #[test]
    #[ignore = "Opt-in public network provider test"]
    fn live_musixmatch() {
        report("Musixmatch", musixmatch(&live_track()));
    }
    #[test]
    #[ignore = "Opt-in public network provider test"]
    fn live_youtube() {
        report("YouTube Music", youtube(&live_track()));
    }
    #[test]
    fn parsers_retain_real_word_times_and_plain_stays_untimed() {
        let xml="<wsy><line><linestring>Hello world</linestring><word><starttime>1000</starttime><endtime>1200</endtime><wordstring>Hello </wordstring></word><word><starttime>1200</starttime><endtime>1800</endtime><wordstring>world</wordstring></word></line></wsy>";
        let value = petit_words(xml).unwrap();
        let lines = parse(&value);
        assert_eq!(lines[0].syllables.len(), 2);
        assert_eq!(lines[0].end_ms, 1800);
        assert!(petit_times(&[0; 10]).is_err());
    }
    #[test]
    fn recording_match_rejects_covers_and_wrong_durations() {
        let track = Entry {
            title: "Song".into(),
            subtitle: "Singer".into(),
            duration: 180000,
            ..Default::default()
        };
        assert!(matches(&track, "Song", "Singer", 181000));
        assert!(!matches(&track, "Song", "Other", 180000));
        assert!(!matches(&track, "Song (Live)", "Singer", 180000));
        assert!(!matches(&track, "Song", "Singer", 200000));
    }
}
