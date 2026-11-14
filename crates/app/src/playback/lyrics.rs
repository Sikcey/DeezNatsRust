//! Multi-provider lyrics resolution, syllable/karaoke parsing, and audio visualizer state.
use crate::{catalog::Entry, engine::Engine};
use serde_json::Value;
mod extras {
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
            && (track.duration == 0
                || duration_ms == 0
                || track.duration.abs_diff(duration_ms) <= 4000)
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
                    && syllables.iter().all(|w| {
                        w.start_ms == syllables[0].start_ms && w.end_ms == syllables[0].end_ms
                    })
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
        let doc =
            roxmltree::Document::parse(text).map_err(|_| "Invalid PetitLyrics word timing")?;
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
                return Err("Musixmatch returned a different recording; ignored".into());
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
            let artist_text =
                text(&columns[1]["musicResponsiveListItemFlexColumnRenderer"]["text"]);
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
}
mod formats {
    //! Provider formats. Timings are read from the source, never estimated from text.
    use super::{LyricLine, Syllable};
    use serde_json::Value;

    pub(super) fn lyricsfile(text: &str) -> Vec<LyricLine> {
        if text.len() > 2 * 1024 * 1024 {
            return Vec::new();
        }
        let Ok(value) = serde_yaml_ng::from_str::<Value>(text) else {
            return Vec::new();
        };
        value["lines"]
            .as_array()
            .into_iter()
            .flatten()
            .take(2000)
            .filter_map(|line| {
                let text = line["text"].as_str()?.to_owned();
                let start = line["start_ms"].as_u64();
                let words = line["words"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|word| {
                        let start_ms = word["start_ms"].as_u64()?;
                        let end_ms = word["end_ms"].as_u64()?;
                        (end_ms > start_ms).then(|| Syllable {
                            start_ms,
                            end_ms,
                            text: word["text"].as_str().unwrap_or_default().into(),
                        })
                    })
                    .collect();
                Some(LyricLine {
                    start_ms: start.unwrap_or(0),
                    end_ms: line["end_ms"]
                        .as_u64()
                        .unwrap_or_else(|| start.map(|n| n.saturating_add(1)).unwrap_or(0)),
                    text,
                    syllables: words,
                    translation: String::new(),
                })
            })
            .collect()
    }

    // NetEase YRC: [absolute line start,line duration](absolute word start,duration,flags)word...
    pub(super) fn yrc(text: &str) -> Vec<LyricLine> {
        let mut lines = Vec::new();
        for raw in text.lines().take(2000) {
            let Some((header, mut rest)) = raw.strip_prefix('[').and_then(|s| s.split_once(']'))
            else {
                continue;
            };
            let Some((start, duration)) = header
                .split_once(',')
                .and_then(|(a, b)| Some((a.parse::<u64>().ok()?, b.parse::<u64>().ok()?)))
            else {
                continue;
            };
            let mut words = Vec::new();
            while let Some(after) = rest.strip_prefix('(') {
                let Some((header, tail)) = after.split_once(')') else {
                    break;
                };
                let fields: Vec<_> = header.split(',').collect();
                if fields.len() != 3 {
                    break;
                }
                let (Ok(ws), Ok(duration)) = (fields[0].parse::<u64>(), fields[1].parse::<u64>())
                else {
                    break;
                };
                let next = tail.find('(').unwrap_or(tail.len());
                if duration > 0 {
                    words.push(Syllable {
                        start_ms: ws,
                        end_ms: ws.saturating_add(duration),
                        text: tail[..next].into(),
                    });
                }
                rest = &tail[next..];
            }
            if !words.is_empty() {
                lines.push(LyricLine {
                    start_ms: start,
                    end_ms: start.saturating_add(duration),
                    text: words.iter().map(|w| w.text.as_str()).collect(),
                    syllables: words,
                    translation: String::new(),
                });
            }
        }
        lines
    }

    fn ttml_time(text: &str) -> Option<u64> {
        if let Some(ms) = text.strip_suffix("ms") {
            return ms.parse().ok();
        }
        if let Some(seconds) = text.strip_suffix('s') {
            return seconds_to_ms(seconds);
        }
        let fields: Vec<_> = text.split(':').collect();
        match fields.as_slice() {
            [seconds] => seconds_to_ms(seconds),
            [minutes, seconds] => minutes
                .parse::<u64>()
                .ok()?
                .checked_mul(60_000)?
                .checked_add(seconds_to_ms(seconds)?),
            [hours, minutes, seconds] => hours
                .parse::<u64>()
                .ok()?
                .checked_mul(3_600_000)?
                .checked_add(minutes.parse::<u64>().ok()?.checked_mul(60_000)?)?
                .checked_add(seconds_to_ms(seconds)?),
            _ => None,
        }
    }
    fn seconds_to_ms(text: &str) -> Option<u64> {
        let seconds = text.parse::<f64>().ok()?;
        (seconds.is_finite() && seconds >= 0.0 && seconds < 86400.0)
            .then(|| (seconds * 1000.0).round() as u64)
    }
    fn auxiliary(node: roxmltree::Node<'_, '_>) -> bool {
        node.ancestors().any(|parent| {
            parent.is_element()
                && parent.attributes().any(|attribute| {
                    attribute.name() == "role"
                        && matches!(attribute.value(), "x-translation" | "x-roman" | "x-bg")
                })
        })
    }

    pub(super) fn ttml(text: &str) -> Vec<LyricLine> {
        if text.len() > 2 * 1024 * 1024 {
            return Vec::new();
        }
        let Ok(doc) = roxmltree::Document::parse_with_options(
            text,
            roxmltree::ParsingOptions {
                nodes_limit: 50_000,
                ..Default::default()
            },
        ) else {
            return Vec::new();
        };
        let mut lines = Vec::new();
        for node in doc.descendants().filter(|n| n.has_tag_name("p")).take(2000) {
            if auxiliary(node) {
                continue;
            }
            let start = node.attribute("begin").and_then(ttml_time);
            let end = node.attribute("end").and_then(ttml_time).or_else(|| {
                node.attribute("dur")
                    .and_then(ttml_time)
                    .map(|dur| start.unwrap_or(0).saturating_add(dur))
            });
            let mut text = String::new();
            let mut words: Vec<Syllable> = Vec::new();
            for child in node
                .descendants()
                .filter(|child| child.is_text() && !auxiliary(*child))
            {
                let fragment = child.text().unwrap_or_default();
                text.push_str(fragment);
                // Nearest timed span owns this text, including nested emphasis spans.
                let timed = child
                    .ancestors()
                    .take_while(|parent| *parent != node)
                    .find(|parent| parent.is_element() && parent.attribute("begin").is_some());
                if let Some(timed) = timed {
                    if let (Some(ws), Some(we)) = (
                        timed.attribute("begin").and_then(ttml_time),
                        timed.attribute("end").and_then(ttml_time),
                    ) {
                        if we > ws {
                            if let Some(previous) = words
                                .last_mut()
                                .filter(|word| word.start_ms == ws && word.end_ms == we)
                            {
                                previous.text.push_str(fragment);
                            } else {
                                words.push(Syllable {
                                    start_ms: ws,
                                    end_ms: we,
                                    text: fragment.into(),
                                });
                            }
                        }
                    }
                } else if fragment.chars().all(char::is_whitespace) {
                    if let Some(previous) = words.last_mut() {
                        previous.text.push_str(fragment);
                    }
                }
            }
            if text.trim().is_empty() {
                continue;
            }
            lines.push(LyricLine {
                start_ms: start
                    .or_else(|| words.first().map(|w| w.start_ms))
                    .unwrap_or(0),
                end_ms: end
                    .or_else(|| words.last().map(|w| w.end_ms))
                    .unwrap_or_else(|| start.map(|s| s.saturating_add(1)).unwrap_or(0)),
                text: text.trim().into(),
                syllables: words,
                translation: node
                    .descendants()
                    .find(|n| {
                        n.is_element()
                            && n.attributes()
                                .any(|a| a.name() == "role" && a.value() == "x-translation")
                    })
                    .map(|n| {
                        n.descendants()
                            .filter_map(|n| n.is_text().then(|| n.text().unwrap_or_default()))
                            .collect()
                    })
                    .unwrap_or_default(),
            });
        }
        lines
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn ttml_accepts_apple_bare_seconds_before_minute_boundary() {
            let lines = ttml(
                r#"<tt><body><p begin="15.967" end="17.119"><span begin="15.967" end="17.119">First</span></p><p begin="58.045" end="59.167">Later</p><p begin="1:00.151" end="1:01.367">Next</p></body></tt>"#,
            );
            assert_eq!(
                lines.iter().map(|line| line.start_ms).collect::<Vec<_>>(),
                [15967, 58045, 60151]
            );
            assert_eq!(lines[0].syllables[0].end_ms, 17119);
            assert_eq!(lines[2].end_ms, 61367);
            assert_eq!(ttml_time("-1"), None);
            assert_eq!(ttml_time("NaN"), None);
        }
        #[test]
        fn word_formats_keep_real_word_times() {
            let yaml = "version: '1.0'\nlines:\n- text: Hello world\n  start_ms: 1000\n  end_ms: 2500\n  words:\n  - {text: 'Hello ', start_ms: 1000, end_ms: 1500}\n  - {text: world, start_ms: 1600, end_ms: 2500}\n";
            let lines = lyricsfile(yaml);
            assert_eq!(lines[0].syllables[1].start_ms, 1600);
            let lines = yrc("[1000,1500](1000,500,0)Hello (1600,900,0)world");
            assert_eq!(lines[0].text, "Hello world");
            assert_eq!(lines[0].syllables[1].end_ms, 2500);
        }
        #[test]
        fn ttml_keeps_words_spaces_and_separates_translation() {
            let xml = r#"<tt xmlns="http://www.w3.org/ns/ttml" xmlns:ttm="http://www.w3.org/ns/ttml#metadata"><body><div><p begin="00:01.000" end="3s"><span begin="1s" end="1500ms">Hello</span> <span begin="1.6s" end="3s"><span>world &amp; you</span></span><span ttm:role="x-translation">Translation</span></p></div></body></tt>"#;
            let lines = ttml(xml);
            assert_eq!(lines[0].text, "Hello world & you");
            assert_eq!(lines[0].translation, "Translation");
            assert_eq!(lines[0].syllables.len(), 2);
            assert_eq!(lines[0].syllables[1].start_ms, 1600);
            assert!(ttml("<!DOCTYPE tt [<!ENTITY x 'entity'>]><tt><p>&x;</p></tt>").is_empty());
        }
    }
}
pub(crate) mod http {
    //! Small bounded Windows HTTP transport for provider headers and POST bodies.
    use std::{
        ffi::c_void,
        ptr::{null, null_mut},
        sync::OnceLock,
        time::{Duration, Instant},
    };
    use windows_sys::Win32::Networking::WinHttp::*;
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
        SESSION
            .get_or_init(|| unsafe {
                let handle = WinHttpOpen(
                    wide("DeezNatsArtwork/1").as_ptr(),
                    WINHTTP_ACCESS_TYPE_NO_PROXY,
                    null(),
                    null(),
                    0,
                );
                if handle.is_null() {
                    return Err("Artwork HTTP initialization failed".into());
                }
                WinHttpSetTimeouts(handle, 3000, 3000, 4000, 6000);
                Ok(Handle(handle))
            })
            .as_ref()
            .map_err(Clone::clone)
    }
    pub(crate) fn artwork(url: &str) -> Result<Vec<u8>, String> {
        request_transport("GET", url, &[], &[], 16 * 1024 * 1024, true)
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
        request_transport(method, url, headers, body, limit, false)
    }
    /// Bounded response metadata for clients that need auth/rate-limit handling.
    pub(crate) struct Response {
        pub status: u32,
        pub retry_after: Option<u64>,
        pub body: Vec<u8>,
    }
    pub(crate) fn response(
        method: &str,
        url: &str,
        headers: &[(&str, &str)],
        body: &[u8],
    ) -> Result<Response, String> {
        response_transport(method, url, headers, body, 2 * 1024 * 1024, false)
    }
    /// Public Spotify bootstrap scripts can exceed the normal API response limit.
    pub(crate) fn spotify_script(url: &str) -> Result<Response, String> {
        let parsed = url::Url::parse(url).map_err(|_| "Invalid Spotify script URL")?;
        if parsed.scheme() != "https"
            || parsed.host_str() != Some("open.spotifycdn.com")
            || parsed.port().is_some()
            || !parsed.username().is_empty()
            || parsed.password().is_some()
            || !(parsed
                .path()
                .starts_with("/cdn/build/web-player/web-player.")
                || parsed
                    .path()
                    .starts_with("/cdn/build/mobile-web-player/mobile-web-player."))
            || !parsed.path().ends_with(".js")
            || parsed.query().is_some()
            || parsed.fragment().is_some()
        {
            return Err("Spotify script destination rejected".into());
        }
        let mut response = response_transport("GET", url, &[], &[], 8 * 1024 * 1024, false)?;
        if response.body.starts_with(&[0x1f, 0x8b]) {
            use std::io::Read;
            let mut decoded = Vec::new();
            flate2::read::GzDecoder::new(response.body.as_slice())
                .take(8 * 1024 * 1024 + 1)
                .read_to_end(&mut decoded)
                .map_err(|_| "Invalid compressed Spotify script")?;
            if decoded.len() > 8 * 1024 * 1024 {
                return Err("Spotify script is too large".into());
            }
            response.body = decoded;
        }
        Ok(response)
    }
    fn request_transport(
        method: &str,
        url: &str,
        headers: &[(&str, &str)],
        body: &[u8],
        limit: usize,
        direct: bool,
    ) -> Result<Vec<u8>, String> {
        let response = response_transport(method, url, headers, body, limit, direct)?;
        if !(200..300).contains(&response.status) {
            return Err(format!("Provider HTTP {}", response.status));
        }
        Ok(response.body)
    }
    fn response_transport(
        method: &str,
        url: &str,
        headers: &[(&str, &str)],
        body: &[u8],
        limit: usize,
        direct: bool,
    ) -> Result<Response, String> {
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
        let mut header = String::new();
        for (name, value) in headers {
            if name.contains(['\r', '\n']) || value.contains(['\r', '\n']) {
                return Err("Invalid provider header".into());
            }
            header.push_str(&format!("{name}: {value}\r\n"));
        }
        unsafe {
            let server = wide(host);
            let transport = if direct {
                artwork_session()?
            } else {
                session()?
            };
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
            let mut retry = [0u16; 64];
            let mut retry_size = std::mem::size_of_val(&retry) as u32;
            let retry_after = if WinHttpQueryHeaders(
                request.0,
                WINHTTP_QUERY_CUSTOM,
                wide("Retry-After").as_ptr(),
                retry.as_mut_ptr().cast(),
                &mut retry_size,
                null_mut(),
            ) != 0
            {
                String::from_utf16_lossy(
                    &retry[..retry.iter().position(|c| *c == 0).unwrap_or(retry.len())],
                )
                .trim()
                .parse::<u64>()
                .ok()
            } else {
                None
            };
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
            Ok(Response {
                status,
                retry_after,
                body: result,
            })
        }
    }
    pub(crate) fn json(url: &str, headers: &[(&str, &str)]) -> Result<serde_json::Value, String> {
        serde_json::from_slice(&request("GET", url, headers, &[])?)
            .map_err(|_| "Invalid provider JSON".into())
    }
    pub(super) fn post_json(
        url: &str,
        value: &serde_json::Value,
    ) -> Result<serde_json::Value, String> {
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
        fn rejects_unsafe_urls_and_header_injection_before_network() {
            assert!(request("GET", "http://example.com", &[], &[]).is_err());
            assert!(request("GET", "https://user@example.com", &[], &[]).is_err());
            assert!(request("GET", "https://example.com", &[("Cookie", "x\r\ny")], &[]).is_err());
        }
    }
}
mod kugou {
    //! Direct KuGou search and KRC word timings. No aggregator or account required.
    //! KRC protocol reference: f1nniboy/lrcmux (MIT); see NOTICE.md.
    use super::{url_encode, Entry, LyricLine, Syllable};
    use base64::Engine as _;
    use serde_json::{json, Value};
    use std::io::Read;

    fn normalized(value: &str) -> String {
        value
            .chars()
            .filter(|c| c.is_alphanumeric())
            .flat_map(char::to_lowercase)
            .collect()
    }
    fn title_base(value: &str) -> &str {
        let lower = value.to_ascii_lowercase();
        [" (feat.", " (feat ", " (ft.", " (with "]
            .iter()
            .filter_map(|marker| lower.find(marker))
            .min()
            .map(|i| &value[..i])
            .unwrap_or(value)
    }
    pub(super) fn fetch(
        track: &Entry,
        mut fetch: impl FnMut(&str) -> Result<Value, String>,
    ) -> Result<Value, String> {
        let artist = track
            .raw
            .pointer("/artists/0/name")
            .and_then(Value::as_str)
            .unwrap_or_else(|| track.subtitle.split(", ").next().unwrap_or(&track.subtitle));
        if artist.trim().is_empty() || track.title.trim().is_empty() {
            return Err("Missing KuGou search metadata".into());
        }
        let mut selected = None;
        for title in [track.title.as_str(), title_base(&track.title)] {
            let query = url_encode(&format!("{artist} - {title}"));
            let response = fetch(&format!(
            "https://krcs.kugou.com/search?ver=1&man=yes&client=mobi&keyword={query}&duration={}",
            track.duration
        ))?;
            selected = response["candidates"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|candidate| {
                    let singer = candidate["singer"].as_str().unwrap_or_default();
                    let matches_artist = singer
                        .split(['&', ',', '/', '\u{3001}'])
                        .any(|s| normalized(s) == normalized(artist));
                    candidate["product_from"].as_str() != Some("ugc")
                        && normalized(title_base(candidate["song"].as_str().unwrap_or_default()))
                            == normalized(title_base(&track.title))
                        && matches_artist
                        && (track.duration == 0
                            || candidate["duration"]
                                .as_u64()
                                .is_some_and(|d| d.abs_diff(track.duration) <= 2500))
                })
                .min_by_key(|c| c["duration"].as_u64().unwrap_or(0).abs_diff(track.duration))
                .cloned();
            if selected.is_some() || title_base(&track.title) == track.title {
                break;
            }
        }
        let candidate = selected.ok_or("No matching KuGou recording")?;
        let id = candidate["id"].as_str().ok_or("Missing KuGou lyric ID")?;
        let key = candidate["accesskey"]
            .as_str()
            .ok_or("Missing KuGou access key")?;
        let value = fetch(&format!(
        "https://lyrics.kugou.com/download?ver=1&client=pc&id={}&accesskey={}&fmt=krc&charset=utf8",
        url_encode(id),
        url_encode(key)
    ))?;
        let krc = decode(
            value["content"]
                .as_str()
                .ok_or("KuGou returned no lyrics")?,
        )?;
        Ok(json!({"krc":krc}))
    }
    fn decode(content: &str) -> Result<String, String> {
        const LIMIT: usize = 2 * 1024 * 1024;
        if content.len() > LIMIT {
            return Err("KuGou response too large".into());
        }
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(content)
            .map_err(|_| "Invalid KRC encoding")?;
        if !bytes.starts_with(b"krc1") {
            return Err("Invalid KRC header".into());
        }
        let key = [
            64, 71, 97, 119, 94, 50, 116, 71, 81, 54, 49, 45, 206, 210, 110, 105,
        ];
        let compressed: Vec<_> = bytes[4..]
            .iter()
            .enumerate()
            .map(|(i, b)| b ^ key[i % key.len()])
            .collect();
        let mut text = String::new();
        flate2::read::ZlibDecoder::new(compressed.as_slice())
            .take((LIMIT + 1) as u64)
            .read_to_string(&mut text)
            .map_err(|_| "Invalid KRC compression")?;
        if text.len() > LIMIT {
            return Err("Decoded KRC too large".into());
        }
        Ok(text)
    }
    pub(super) fn parse(krc: &str) -> Vec<LyricLine> {
        let mut result = Vec::new();
        let offset = krc
            .lines()
            .find_map(|line| {
                line.strip_prefix("[offset:")?
                    .strip_suffix(']')?
                    .parse::<i64>()
                    .ok()
            })
            .unwrap_or(0);
        let timestamp = |time: u64| -> u64 {
            if offset >= 0 {
                time.saturating_add(offset as u64)
            } else {
                time.saturating_sub(offset.unsigned_abs())
            }
        };
        for raw in krc.lines().take(2000) {
            let Some((header, mut rest)) = raw.strip_prefix('[').and_then(|s| s.split_once(']'))
            else {
                continue;
            };
            let Some((start, duration)) = header
                .split_once(',')
                .and_then(|(a, b)| Some((a.parse::<u64>().ok()?, b.parse::<u64>().ok()?)))
            else {
                continue;
            };
            let Some(end) = start.checked_add(duration) else {
                continue;
            };
            let mut syllables = Vec::new();
            while let Some(after) = rest.strip_prefix('<') {
                let Some((header, tail)) = after.split_once('>') else {
                    break;
                };
                let fields: Vec<_> = header.split(',').collect();
                if fields.len() != 3 {
                    break;
                }
                let (Ok(relative), Ok(duration)) =
                    (fields[0].parse::<u64>(), fields[1].parse::<u64>())
                else {
                    break;
                };
                let (text, remaining) = tail
                    .find('<')
                    .map(|n| (&tail[..n], &tail[n..]))
                    .unwrap_or((tail, ""));
                rest = remaining;
                let Some(ws) = start.checked_add(relative) else {
                    continue;
                };
                let Some(we) = ws.checked_add(duration) else {
                    continue;
                };
                if !text.is_empty() && we > ws {
                    syllables.push(Syllable {
                        start_ms: timestamp(ws),
                        end_ms: timestamp(we),
                        text: text
                            .replace("&amp;", "&")
                            .replace("&lt;", "<")
                            .replace("&gt;", ">")
                            .replace("&quot;", "\"")
                            .replace("&#39;", "'"),
                    });
                }
            }
            let text: String = syllables.iter().map(|s| s.text.as_str()).collect();
            let trimmed = text.trim();
            let credit = [
                "\u{4f5c}\u{8bcd}",
                "\u{4f5c}\u{66f2}",
                "\u{8bcd}\u{ff1a}",
                "\u{66f2}\u{ff1a}",
                "\u{7f16}\u{66f2}",
                "\u{5236}\u{4f5c}\u{4eba}",
                "lyrics:",
                "composer:",
                "written by:",
                "lyricist:",
            ]
            .iter()
            .any(|prefix| trimmed.to_lowercase().starts_with(prefix));
            if !trimmed.is_empty() && !credit && !(start == 0 && trimmed.contains(" - ")) {
                result.push(LyricLine {
                    start_ms: timestamp(start),
                    end_ms: timestamp(end),
                    text: trimmed.into(),
                    syllables,
                    translation: String::new(),
                });
            }
        }
        result
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        #[test]
        #[ignore = "Opt-in read-only KuGou network test"]
        fn live_kugou_word_timings() {
            let engine = crate::engine::Engine::load().unwrap();
            for (title, artist, duration) in [
                ("Yellow", "Coldplay", 269000),
                ("Dracula (with JENNIE)", "Tame Impala", 209000),
            ] {
                let track = Entry {
                    title: title.into(),
                    subtitle: artist.into(),
                    duration,
                    ..Default::default()
                };
                let started = std::time::Instant::now();
                let value = fetch(&track, |url| {
                    serde_json::from_slice(&engine.fetch(url)?).map_err(|e| e.to_string())
                })
                .unwrap();
                let lines = parse(value["krc"].as_str().unwrap());
                assert!(lines.len() > 10);
                assert!(lines
                    .iter()
                    .all(|l| l.syllables.iter().any(|s| s.end_ms > s.start_ms)));
                println!(
                    "KuGou {title}: {} word-timed lines in {:.2}s",
                    lines.len(),
                    started.elapsed().as_secs_f64()
                );
            }
        }
        #[test]
        fn krc_preserves_relative_word_times_and_rejects_invalid_data() {
            let lines = parse("[offset:10]\n[1000,900]<0,300,0>Hello <400,500,0>world");
            assert_eq!(lines[0].text, "Hello world");
            assert_eq!(
                (lines[0].syllables[1].start_ms, lines[0].syllables[1].end_ms),
                (1410, 1910)
            );
            assert!(parse("[99999999999999999999,1]<0,1,0>x").is_empty());
            assert!(decode("broken").is_err());
        }
        #[test]
        fn search_rejects_wrong_recording_before_downloading() {
            let track = Entry {
                title: "Song".into(),
                subtitle: "Singer".into(),
                duration: 180000,
                ..Default::default()
            };
            let mut calls = 0;
            assert!(fetch(&track, |_| { calls+=1; Ok(json!({"candidates":[{"id":"1","song":"Song","singer":"Cover artist","duration":180000}]})) }).is_err());
            assert_eq!(calls, 1);
        }
    }
}
mod providers {
    //! Public provider adapters. Cross-service IDs are resolved by metadata, never
    //! reused as though a Deezer ID were a NetEase or TTML database ID.
    use super::{url_encode, Entry};
    use serde_json::{json, Value};

    fn normalized(text: &str) -> String {
        text.chars()
            .filter(|ch| ch.is_alphanumeric())
            .flat_map(char::to_lowercase)
            .collect()
    }
    fn artist(track: &Entry) -> &str {
        track
            .raw
            .pointer("/artists/0/name")
            .and_then(Value::as_str)
            .unwrap_or_else(|| track.subtitle.split(", ").next().unwrap_or(&track.subtitle))
            .trim()
    }
    fn same_title(track: &Entry, title: &str) -> bool {
        !track.title.is_empty() && normalized(&track.title) == normalized(title)
    }
    fn same_artist(track: &Entry, names: impl Iterator<Item = impl AsRef<str>>) -> bool {
        let expected = normalized(artist(track));
        !expected.is_empty()
            && names
                .into_iter()
                .any(|name| normalized(name.as_ref()) == expected)
    }

    pub(super) fn amll(
        track: &Entry,
        mut fetch: impl FnMut(&str) -> Result<Value, String>,
    ) -> Result<Value, String> {
        let query = format!(
            "https://api.amll.dev/v1/lyrics/search?musicName={}&artistName={}&pageSize=10",
            url_encode(&track.title),
            url_encode(artist(track))
        );
        let result = fetch(&query)?;
        let items = result
            .pointer("/data/items")
            .and_then(Value::as_array)
            .ok_or("AMLL search unavailable")?;
        let item = items
            .iter()
            .find(|item| {
                item["musicNames"].as_array().is_some_and(|names| {
                    names
                        .iter()
                        .filter_map(Value::as_str)
                        .any(|name| same_title(track, name))
                }) && same_artist(
                    track,
                    item["artistNames"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(Value::as_str),
                )
            })
            .ok_or("No matching AMLL TTML lyrics")?;
        let id = item["id"].as_u64().ok_or("Invalid AMLL lyric identity")?;
        let result = fetch(&format!("https://api.amll.dev/v1/lyrics/get?id={id}"))?;
        let ttml = result
            .pointer("/data/lyrics")
            .and_then(Value::as_str)
            .ok_or("AMLL returned no TTML")?;
        Ok(json!({"ttml":ttml}))
    }

    pub(super) fn netease(
        track: &Entry,
        mut fetch: impl FnMut(&str) -> Result<Value, String>,
    ) -> Result<Value, String> {
        let query = url_encode(&format!("{} {}", track.title, artist(track)));
        let result = fetch(&format!(
            "https://music.163.com/api/search/get?s={query}&type=1&limit=10"
        ))?;
        let songs = result
            .pointer("/result/songs")
            .and_then(Value::as_array)
            .ok_or("NetEase search unavailable")?;
        let song = songs
            .iter()
            .filter(|song| {
                same_title(track, song["name"].as_str().unwrap_or_default())
                    && same_artist(
                        track,
                        song["artists"]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .filter_map(|artist| artist["name"].as_str()),
                    )
                    && (track.duration == 0
                        || song["duration"]
                            .as_u64()
                            .is_some_and(|duration| duration.abs_diff(track.duration) <= 4000))
            })
            .min_by_key(|song| {
                song["duration"]
                    .as_u64()
                    .unwrap_or(0)
                    .abs_diff(track.duration)
            })
            .ok_or("No matching NetEase recording")?;
        let id = song["id"].as_u64().ok_or("Invalid NetEase song identity")?;
        let result = fetch(&format!("https://music.163.com/api/song/lyric/v1?id={id}&lv=-1&kv=-1&tv=-1&rv=-1&yv=-1&ytv=-1&yrv=-1"));
        // Older deployments expose only line-synced LRC on the original route.
        match result {
            Ok(value)
                if value
                    .pointer("/yrc/lyric")
                    .and_then(Value::as_str)
                    .is_some_and(|s| !s.is_empty())
                    || value
                        .pointer("/lrc/lyric")
                        .and_then(Value::as_str)
                        .is_some_and(|s| !s.is_empty()) =>
            {
                Ok(value)
            }
            _ => fetch(&format!(
                "https://music.163.com/api/song/lyric?id={id}&lv=-1&kv=-1&tv=-1"
            )),
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        #[test]
        fn cross_provider_matching_rejects_wrong_artist_and_version() {
            let track = Entry {
                title: "Song".into(),
                subtitle: "Singer".into(),
                duration: 180000,
                ..Default::default()
            };
            let mut calls = 0;
            let result = netease(&track, |_| {
                calls += 1;
                Ok(json!({"result":{"songs":[
                    {"id":1,"name":"Song","duration":180000,"artists":[{"name":"Cover artist"}]},
                    {"id":2,"name":"Song","duration":220000,"artists":[{"name":"Singer"}]}
                ]}}))
            });
            assert!(result.is_err());
            assert_eq!(calls, 1);
            assert!(amll(&track, |_| Ok(
                json!({"data":{"items":[{"id":1,"musicNames":["Song"],"artistNames":["Other"]}]}})
            ))
            .is_err());
        }
        #[test]
        fn netease_resolves_its_own_id_and_retains_yrc() {
            let track = Entry {
                id: "deezer-id".into(),
                title: "Song".into(),
                subtitle: "Singer".into(),
                duration: 180000,
                ..Default::default()
            };
            let result = netease(&track, |url| {
            if url.contains("/search/") { Ok(json!({"result":{"songs":[{"id":42,"name":"Song","duration":180001,"artists":[{"name":"Singer"}]}]}})) }
            else { assert!(url.contains("id=42&")); Ok(json!({"yrc":{"lyric":"[0,1000](0,1000,0)Word"}})) }
        }).unwrap();
            assert!(result["yrc"]["lyric"].is_string());
        }
    }
}
mod spotify_native {
    use super::{LyricLine, Syllable};
    use serde_json::Value;

    pub(super) fn fetch(uri: &str) -> Result<Value, String> {
        let Some(id) = uri.strip_prefix("spotify:track:") else {
            return Err("Spotify lyrics require a Spotify track URI".into());
        };
        if id.len() != 22 || !id.bytes().all(|b| b.is_ascii_alphanumeric()) {
            return Err("Spotify lyrics require a valid 22-character track ID".into());
        }
        let url = format!("https://spclient.wg.spotify.com/color-lyrics/v2/track/{id}?format=json&vocalRemoval=false&market=from_token");
        let value = crate::spotify::web::rest_get(&url)?;
        if value.get("lyrics").is_none() {
            return Err("Spotify returned no lyrics for this track".into());
        }
        let lyrics = &value["lyrics"];
        if !matches!(
            lyrics["syncType"].as_str(),
            Some("LINE_SYNCED" | "WORD_SYNCED" | "UNSYNCED")
        ) || !lyrics["lines"]
            .as_array()
            .is_some_and(|lines| !lines.is_empty())
        {
            return Err("Spotify returned an unsupported or empty lyrics response".into());
        }
        Ok(value)
    }

    pub(super) fn parse(value: &Value) -> Vec<LyricLine> {
        let Some(lyrics) = value.get("lyrics") else {
            return Vec::new();
        };
        let sync = lyrics["syncType"].as_str().unwrap_or_default();
        if !matches!(sync, "LINE_SYNCED" | "WORD_SYNCED" | "UNSYNCED") {
            return Vec::new();
        }
        let Some(raw_lines) = lyrics["lines"].as_array() else {
            return Vec::new();
        };
        raw_lines
            .iter()
            .filter_map(|line| {
                let text = line["words"].as_str().unwrap_or_default().to_owned();
                if text.trim().is_empty() {
                    return None;
                }
                let start_ms = millis(&line["startTimeMs"]).unwrap_or(0);
                let end_ms = millis(&line["endTimeMs"]).unwrap_or(0);
                let syllables = if sync == "WORD_SYNCED" {
                    line["syllables"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(|part| {
                            let text = part["text"].as_str().unwrap_or_default().to_owned();
                            let start_ms = millis(&part["startTimeMs"]).unwrap_or(0);
                            let end_ms = millis(&part["endTimeMs"]).unwrap_or(0);
                            (!text.is_empty() && end_ms > start_ms).then_some(Syllable {
                                start_ms,
                                end_ms,
                                text,
                            })
                        })
                        .collect()
                } else {
                    Vec::new()
                };
                Some(LyricLine {
                    start_ms,
                    end_ms,
                    text,
                    syllables,
                    ..Default::default()
                })
            })
            .collect()
    }

    fn millis(value: &Value) -> Option<u64> {
        value.as_u64().or_else(|| value.as_str()?.parse().ok())
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn parses_line_synced_and_plain_without_making_up_word_timing() {
            let timed = parse(
                &serde_json::json!({"lyrics":{"syncType":"LINE_SYNCED","lines":[{"startTimeMs":"120","endTimeMs":"2000","words":"Line"}]}}),
            );
            assert_eq!(timed[0].start_ms, 120);
            assert_eq!(timed[0].end_ms, 2000);
            assert!(timed[0].syllables.is_empty());
            let plain = parse(
                &serde_json::json!({"lyrics":{"syncType":"UNSYNCED","lines":[{"words":"Plain"}]}}),
            );
            assert_eq!(plain[0].start_ms, 0);
            assert_eq!(plain[0].end_ms, 0);
            assert!(plain[0].syllables.is_empty());
        }

        #[test]
        fn parses_explicit_word_spans_and_rejects_unknown_or_malformed_responses() {
            let value = serde_json::json!({"lyrics":{"syncType":"WORD_SYNCED","lines":[{"startTimeMs":"10","endTimeMs":"40","words":"Hi","syllables":[{"text":"Hi","startTimeMs":"10","endTimeMs":"40"}]}]}});
            let lines = parse(&value);
            assert_eq!(lines[0].syllables[0].text, "Hi");
            assert_eq!(lines[0].syllables[0].end_ms, 40);
            assert!(
                parse(&serde_json::json!({"lyrics":{"syncType":"UNKNOWN","lines":[]}})).is_empty()
            );
            assert!(
                parse(&serde_json::json!({"lyrics":{"syncType":"LINE_SYNCED","lines":"bad"}}))
                    .is_empty()
            );
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Syllable {
    pub start_ms: u64,
    pub end_ms: u64,
    pub text: String,
}

#[derive(Clone, Debug, Default)]
pub struct LyricLine {
    pub start_ms: u64,
    pub end_ms: u64,
    pub text: String,
    pub syllables: Vec<Syllable>,
    pub translation: String,
}

pub const LYRIC_SOURCES: [&str; 10] = [
    "Deezer",
    "LRCLIB",
    "Musixmatch",
    "AMLL TTML",
    "NetEase",
    "PetitLyrics",
    "Genius",
    "KuGou",
    "Apple Music",
    "YouTube Music",
];

/// Fetches lyrics from enabled providers and picks the richest result:
/// Syllable-synced (3) > Line-synced (2) > Plain text (1) > Empty (0).
pub fn fetch_best_lyrics(
    engine: &Engine,
    track: &Entry,
    id: &str,
    enabled_sources: u32,
) -> Result<(String, Vec<LyricLine>), String> {
    fetch_best_inner(Some(engine), track, id, enabled_sources)
}
pub(crate) fn fetch_spotify_lyrics(
    track: &Entry,
    id: &str,
    enabled_sources: u32,
) -> Result<(String, Vec<LyricLine>), String> {
    fetch_best_inner(None, track, id, enabled_sources)
}
fn fetch_best_inner(
    engine: Option<&Engine>,
    track: &Entry,
    id: &str,
    enabled_sources: u32,
) -> Result<(String, Vec<LyricLine>), String> {
    fetch_progressive(engine, track, id, enabled_sources, |_| {})
}
pub(crate) fn fetch_progressive(
    engine: Option<&Engine>,
    track: &Entry,
    id: &str,
    enabled_sources: u32,
    mut progress: impl FnMut((String, Vec<LyricLine>)),
) -> Result<(String, Vec<LyricLine>), String> {
    let key = format!(
        "{}\0{}\0{}\0{}\0{}\0{}",
        if engine.is_none() {
            "spotify"
        } else {
            "deezer"
        },
        id,
        track.title,
        track.subtitle,
        track.duration,
        enabled_sources
    );
    if let Some(cached) = lyric_cache().lock().unwrap().get(&key) {
        return Ok(cached);
    }
    // Publish useful answers as they arrive; slow providers may improve them later.
    let result = std::thread::scope(|scope| {
        let (tx, rx) = std::sync::mpsc::channel();
        for source in [0, 3, 8, 5, 7, 2, 4, 1, 9] {
            if enabled_sources & (1 << source) == 0 {
                continue;
            }
            let tx = tx.clone();
            scope.spawn(move || {
                let value = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    fetch_provider_inner(engine, track, id, source)
                }))
                .unwrap_or_else(|_| Err("Lyrics provider failed".into()));
                let _ = tx.send((source, value));
            });
        }
        drop(tx);
        let mut results = std::collections::HashMap::new();
        let mut shown = (0, 0, 0);
        for (source, value) in rx {
            results.insert(source, value);
            if let Ok(answer) =
                fetch_sources(track, id, enabled_sources, engine.is_none(), |source| {
                    results
                        .get(&source)
                        .cloned()
                        .unwrap_or_else(|| Err("Pending".into()))
                })
            {
                let score = quality(&answer.1);
                if score > shown {
                    shown = score;
                    progress(answer);
                }
            }
        }
        fetch_sources(track, id, enabled_sources, engine.is_none(), |source| {
            results
                .remove(&source)
                .unwrap_or_else(|| Err("Lyrics provider unavailable".into()))
        })
    });
    if let Ok(value) = &result {
        lyric_cache().lock().unwrap().insert(key, value.clone());
    }
    result
}

type LyricResult = (String, Vec<LyricLine>);
#[derive(Default)]
struct LyricCache {
    entries: std::collections::VecDeque<(String, std::time::Instant, LyricResult, usize)>,
    bytes: usize,
}
impl LyricCache {
    fn get(&mut self, key: &str) -> Option<LyricResult> {
        let index = self.entries.iter().position(|(k, _, _, _)| k == key)?;
        let item = self.entries.remove(index)?;
        if item.1.elapsed()
            > std::time::Duration::from_secs(
                if item.2 .1.is_empty() && item.2 .0 != "Instrumental" {
                    300
                } else {
                    1800
                },
            )
        {
            self.bytes -= item.3;
            return None;
        }
        let result = item.2.clone();
        self.entries.push_back(item);
        Some(result)
    }
    fn insert(&mut self, key: String, value: LyricResult) {
        let bytes = value
            .1
            .iter()
            .map(|line| {
                std::mem::size_of::<LyricLine>()
                    + line.text.len()
                    + line.translation.len()
                    + line
                        .syllables
                        .iter()
                        .map(|word| std::mem::size_of::<Syllable>() + word.text.len())
                        .sum::<usize>()
            })
            .sum::<usize>();
        if bytes > 8 * 1024 * 1024 {
            return;
        }
        if let Some(index) = self.entries.iter().position(|(k, _, _, _)| k == &key) {
            if let Some(old) = self.entries.remove(index) {
                self.bytes -= old.3;
            }
        }
        while self.bytes + bytes > 8 * 1024 * 1024 || self.entries.len() >= 32 {
            if let Some(old) = self.entries.pop_front() {
                self.bytes -= old.3;
            } else {
                break;
            }
        }
        self.bytes += bytes;
        self.entries
            .push_back((key, std::time::Instant::now(), value, bytes));
    }
}
fn lyric_cache() -> &'static std::sync::Mutex<LyricCache> {
    static CACHE: std::sync::OnceLock<std::sync::Mutex<LyricCache>> = std::sync::OnceLock::new();
    CACHE.get_or_init(|| std::sync::Mutex::new(LyricCache::default()))
}

fn fetch_provider(
    engine: &Engine,
    track: &Entry,
    id: &str,
    source: usize,
) -> Result<Value, String> {
    fetch_provider_inner(Some(engine), track, id, source)
}
fn fetch_provider_inner(
    engine: Option<&Engine>,
    track: &Entry,
    id: &str,
    source: usize,
) -> Result<Value, String> {
    let mut json_fetch = |url: &str| {
        let bytes = if let Some(engine) = engine {
            engine.fetch(url)?
        } else {
            http::request("GET", url, &[], &[])?
        };
        serde_json::from_slice::<Value>(&bytes).map_err(|error| error.to_string())
    };
    match source {
        2 => extras::musixmatch(track),
        5 => extras::petit(track),
        8 => extras::apple(track),
        9 => extras::youtube(track),
        7 => kugou::fetch(track, &mut json_fetch),
        3 => providers::amll(track, &mut json_fetch),
        4 => providers::netease(track, &mut json_fetch),
        0 => {
            if engine.is_none() {
                spotify_native::fetch(id)
            } else if let Some(engine) = engine {
                if id.starts_with("spotify:") {
                    return Err("Deezer lyrics require a Deezer track".into());
                }
                engine.invoke("DZLyricsJSON", &[serde_json::json!(id)])
            } else {
                Err("Spotify does not expose lyrics through the catalog API".into())
            }
        }
        1 => {
            let title = url_encode(&track.title);
            let artist = url_encode(
                track
                    .subtitle
                    .split(',')
                    .next()
                    .unwrap_or(&track.subtitle)
                    .trim(),
            );
            let album = url_encode(&track.album);
            let mut url =
                format!("https://lrclib.net/api/get?track_name={title}&artist_name={artist}");
            if !album.is_empty() {
                url.push_str(&format!("&album_name={album}"));
            }
            if track.duration > 0 {
                url.push_str(&format!("&duration={}", track.duration / 1000));
            }
            json_fetch(&url)
        }
        _ => Err("Lyrics provider is not connected".into()),
    }
}

fn matching_instrumental(track: &Entry, value: &Value) -> bool {
    let normalize = |text: &str| {
        text.chars()
            .filter(|c| c.is_alphanumeric())
            .flat_map(char::to_lowercase)
            .collect::<String>()
    };
    value["instrumental"].as_bool() == Some(true)
        && value["trackName"]
            .as_str()
            .is_some_and(|title| normalize(title) == normalize(&track.title))
        && value["artistName"].as_str().is_some_and(|artist| {
            normalize(artist)
                == normalize(track.subtitle.split(',').next().unwrap_or(&track.subtitle))
        })
        && value["duration"].as_f64().is_none_or(|seconds| {
            track.duration == 0 || ((seconds * 1000.0) - track.duration as f64).abs() <= 10_000.0
        })
}

fn fetch_sources(
    track: &Entry,
    id: &str,
    enabled_sources: u32,
    spotify_native: bool,
    mut fetch: impl FnMut(usize) -> Result<Value, String>,
) -> Result<(String, Vec<LyricLine>), String> {
    if enabled_sources == 0 {
        return Err("No lyrics sources enabled".into());
    }
    if enabled_sources & SUPPORTED_SOURCES == 0 {
        return Err("Selected lyrics providers are not connected. Enable Deezer, LRCLIB, AMLL TTML or NetEase.".into());
    }

    let mut best = (String::new(), Vec::new());
    let mut best_score = (0, 0, 0);
    let mut instrumental = false;
    let mut answered = false;
    let mut errors = Vec::new();
    for source in [0, 3, 8, 5, 7, 2, 4, 1, 9] {
        let name = if source == 0 && spotify_native {
            "Spotify"
        } else {
            LYRIC_SOURCES[source]
        };
        if enabled_sources & (1 << source) == 0
            || (source == 0 && id.is_empty())
            || (source != 0 && track.title.is_empty())
        {
            continue;
        }
        match fetch(source) {
            Ok(value) => {
                answered = true;
                // Only explicit provider metadata establishes an instrumental recording.
                instrumental |= source == 1 && matching_instrumental(track, &value);
                let mut lines = match source {
                    0 if spotify_native => spotify_native::parse(&value),
                    0 => parse_deezer_lyrics(&value),
                    7 => kugou::parse(value["krc"].as_str().unwrap_or_default()),
                    3 | 8 => formats::ttml(value["ttml"].as_str().unwrap_or_default()),
                    2 | 5 | 9 if value["lines"].is_array() => extras::parse(&value),
                    4 => {
                        let words = formats::yrc(
                            value
                                .pointer("/yrc/lyric")
                                .and_then(Value::as_str)
                                .unwrap_or_default(),
                        );
                        if !words.is_empty() {
                            words
                        } else {
                            parse_lrclib(
                                &serde_json::json!({"syncedLyrics":value.pointer("/lrc/lyric")}),
                            )
                        }
                    }
                    _ => parse_lrclib(&value),
                };
                // Reject a lyric for a materially longer recording.
                if track.duration > 0
                    && lines
                        .iter()
                        .any(|line| line.start_ms > track.duration.saturating_add(5000))
                {
                    continue;
                }
                finalize_timings(&mut lines, track.duration);
                let score = quality(&lines);
                if score > best_score {
                    best_score = score;
                    best = (name.into(), lines);
                }
            }
            Err(error) => errors.push(format!("{name}: {error}")),
        }
    }
    if best.1.is_empty() && instrumental {
        return Ok(("Instrumental".into(), Vec::new()));
    }
    if best.1.is_empty() && !answered && !errors.is_empty() {
        return Err(errors.join("; "));
    }
    Ok(best)
}

pub const SUPPORTED_SOURCES: u32 = 0x3bb;
pub const DEFAULT_SOURCES: u32 = SUPPORTED_SOURCES;

fn quality(lines: &[LyricLine]) -> (u8, usize, usize) {
    let word_lines = lines
        .iter()
        .filter(|line| {
            line.syllables
                .iter()
                .any(|word| word.end_ms > word.start_ms && !word.text.trim().is_empty())
        })
        .count();
    (
        score_lyrics(lines),
        word_lines,
        lines
            .iter()
            .filter(|line| line.end_ms > line.start_ms)
            .count(),
    )
}

pub fn has_timing(lines: &[LyricLine]) -> bool {
    lines
        .iter()
        .any(|line| line.start_ms > 0 || line.end_ms > line.start_ms || !line.syllables.is_empty())
}

pub fn vocal_end(line: &LyricLine) -> u64 {
    line.syllables
        .iter()
        .filter(|w| w.end_ms > w.start_ms)
        .map(|w| w.end_ms)
        .max()
        .unwrap_or(line.end_ms)
        .max(line.start_ms)
}

/// Only explicit ends/word timestamps establish a break; no guessed singing duration.
pub fn instrumental_gaps(lines: &[LyricLine], duration: u64) -> Vec<(u64, u64)> {
    if !has_timing(lines) {
        return Vec::new();
    }
    let mut end = 0;
    let mut known_end = true;
    let mut gaps = Vec::new();
    for line in lines.iter().filter(|l| !l.text.trim().is_empty()) {
        if known_end && line.start_ms.saturating_sub(end) >= 3000 {
            gaps.push((end, line.start_ms));
        }
        let vocal = vocal_end(line);
        known_end = vocal > line.start_ms;
        end = end.max(vocal);
    }
    if known_end && end > 0 && duration.saturating_sub(end) >= 3000 {
        gaps.push((end, duration));
    }
    gaps
}

/// UTF-8 byte ranges keep wrapped text and provider word spans aligned.
pub fn wrapped_ranges(text: &str, width: i32, font: f32) -> Vec<std::ops::Range<usize>> {
    let mut rows = Vec::new();
    let mut start = 0;
    let mut used = 0.0;
    let mut space = None;
    let chars: Vec<_> = text.char_indices().collect();
    let mut index = 0;
    while index < chars.len() {
        let (byte, ch) = chars[index];
        if ch == '\n' {
            rows.push(start..byte);
            start = byte + ch.len_utf8();
            used = 0.0;
            space = None;
            index += 1;
            continue;
        }
        let advance = font
            * if ch.is_whitespace() {
                0.38
            } else if "ilI.,'!:;|".contains(ch) {
                0.35
            } else if "MWmw@".contains(ch) || ch as u32 >= 0x2e80 {
                1.0
            } else {
                0.64
            };
        if used + advance > width.max(40) as f32 && byte > start {
            let cut = space.filter(|p| *p > start).unwrap_or(byte);
            rows.push(start..cut);
            start = cut;
            while start < text.len()
                && text[start..]
                    .chars()
                    .next()
                    .is_some_and(char::is_whitespace)
            {
                start += text[start..].chars().next().unwrap().len_utf8();
            }
            index = chars.partition_point(|(b, _)| *b < start);
            used = 0.0;
            space = None;
            continue;
        }
        if ch.is_whitespace() {
            space = Some(byte);
        }
        used += advance;
        index += 1;
    }
    if start < text.len() {
        rows.push(start..text.len());
    }
    if rows.is_empty() {
        rows.push(0..0);
    }
    rows
}

pub fn word_ranges(line: &LyricLine) -> Option<Vec<(std::ops::Range<usize>, &Syllable)>> {
    let mut cursor = 0;
    let mut spans = Vec::new();
    for word in &line.syllables {
        let text = word.text.trim();
        if text.is_empty() || word.end_ms <= word.start_ms {
            continue;
        }
        let start = cursor + line.text[cursor..].find(text)?;
        cursor = start + text.len();
        spans.push((start..cursor, word));
    }
    (!spans.is_empty()).then_some(spans)
}

/// Map real word timestamps onto measured text, including pauses between words.
/// Returning None keeps line-only or unalignable lyrics on the plain renderer.
pub fn karaoke_progress(
    line: &LyricLine,
    position: u64,
    mut measure: impl FnMut(&str) -> f32,
) -> Option<f32> {
    let total = measure(&line.text);
    if total <= 0.0 || line.syllables.is_empty() {
        return None;
    }
    let mut cursor = 0;
    let mut frontier = 0.0f32;
    let mut valid = false;
    for word in &line.syllables {
        let text = word.text.trim();
        if text.is_empty() || word.end_ms <= word.start_ms {
            continue;
        }
        let start = cursor + line.text[cursor..].find(text)?;
        let end = start + text.len();
        cursor = end;
        valid = true;
        let left = measure(&line.text[..start]);
        let right = measure(&line.text[..end]);
        if position >= word.end_ms {
            frontier = frontier.max(right);
        } else if position >= word.start_ms {
            let fraction = (position - word.start_ms) as f32 / (word.end_ms - word.start_ms) as f32;
            frontier = frontier.max(left + (right - left) * fraction);
        }
    }
    valid.then(|| (frontier / total).clamp(0.0, 1.0))
}

/// Self-contained demonstration; these timings never enter track lyrics/cache.
pub fn karaoke_example() -> &'static LyricLine {
    static EXAMPLE: std::sync::OnceLock<LyricLine> = std::sync::OnceLock::new();
    EXAMPLE.get_or_init(|| LyricLine {
        start_ms: 500,
        end_ms: 4200,
        text: "Words light up".into(),
        syllables: [
            (500, 1400, "Words"),
            (1500, 2600, "light"),
            (2700, 4200, "up"),
        ]
        .into_iter()
        .map(|(start_ms, end_ms, text)| Syllable {
            start_ms,
            end_ms,
            text: text.into(),
        })
        .collect(),
        ..Default::default()
    })
}

fn score_lyrics(lines: &[LyricLine]) -> u8 {
    if lines.is_empty() {
        0
    } else if lines.iter().any(|line| {
        line.syllables
            .iter()
            .any(|word| word.end_ms > word.start_ms && !word.text.trim().is_empty())
    }) {
        3
    } else if has_timing(lines) {
        2
    } else {
        1
    }
}

fn finalize_timings(lines: &mut [LyricLine], track_duration: u64) {
    if !has_timing(lines) {
        return;
    }
    lines.sort_by_key(|line| line.start_ms);
    let len = lines.len();
    for i in 0..len {
        let next_start = if i + 1 < len {
            lines[i + 1].start_ms
        } else if track_duration > lines[i].start_ms {
            track_duration
        } else {
            lines[i].start_ms
        };
        if lines[i].end_ms <= lines[i].start_ms.saturating_add(1) {
            lines[i].end_ms = next_start.max(lines[i].start_ms);
        }
        for syllable in &mut lines[i].syllables {
            if syllable.end_ms <= syllable.start_ms {
                syllable.end_ms = next_start.max(syllable.start_ms);
            }
        }
    }
}

pub fn split_into_syllables(text: &str, start_ms: u64, end_ms: u64) -> Vec<Syllable> {
    let mut parts = Vec::new();
    let mut current = String::new();
    for ch in text.chars() {
        current.push(ch);
        if ch.is_whitespace() || ch == '-' {
            parts.push(std::mem::take(&mut current));
        }
    }
    if !current.is_empty() {
        parts.push(current);
    }
    if parts.is_empty() {
        return Vec::new();
    }
    let total_chars: usize = parts.iter().map(|p| p.chars().count().max(1)).sum();
    let span = end_ms.saturating_sub(start_ms).max(400);
    let mut cursor = start_ms;
    let mut out = Vec::with_capacity(parts.len());
    for (idx, part) in parts.into_iter().enumerate() {
        let weight = part.chars().count().max(1) as u64;
        let dur = (span * weight / total_chars.max(1) as u64).max(60);
        let next = if idx + 1 == total_chars {
            end_ms
        } else {
            (cursor + dur).min(end_ms)
        };
        out.push(Syllable {
            start_ms: cursor,
            end_ms: next.max(cursor + 40),
            text: part,
        });
        cursor = next;
    }
    out
}

pub fn parse_deezer_lyrics(value: &Value) -> Vec<LyricLine> {
    let mut out = Vec::new();
    if let Some(lines) = value
        .get("synced")
        .and_then(Value::as_array)
        .filter(|v| !v.is_empty())
    {
        for l in lines {
            let timestamp = l["timeMs"].as_u64().or_else(|| l["milliseconds"].as_u64());
            let start_ms = timestamp.unwrap_or(0);
            let dur_ms = l["durationMs"]
                .as_u64()
                .or_else(|| l["duration"].as_u64())
                .unwrap_or(0);
            let text = l["text"]
                .as_str()
                .or_else(|| l["line"].as_str())
                .unwrap_or_default()
                .to_string();
            let mut syllables = Vec::new();
            if let Some(words) = l
                .get("words")
                .or_else(|| l.get("syllables"))
                .and_then(Value::as_array)
            {
                for w in words {
                    let ws = w["startMs"]
                        .as_u64()
                        .or_else(|| w["timeMs"].as_u64())
                        .unwrap_or(start_ms);
                    let we = w["endMs"]
                        .as_u64()
                        .or_else(|| w["durationMs"].as_u64().map(|d| ws + d))
                        .unwrap_or(0);
                    let wt = w["text"].as_str().unwrap_or_default().to_string();
                    if !wt.is_empty() {
                        syllables.push(Syllable {
                            start_ms: ws,
                            end_ms: we,
                            text: wt,
                        });
                    }
                }
            }
            if syllables.is_empty() && text.contains('<') && text.contains('>') {
                let (clean, parsed) = parse_inline_tags(&text, start_ms);
                if !parsed.is_empty() {
                    out.push(LyricLine {
                        start_ms,
                        end_ms: start_ms
                            .saturating_add(dur_ms)
                            .max(parsed.last().map(|s| s.end_ms).unwrap_or(start_ms)),
                        text: clean,
                        syllables: parsed,
                        translation: String::new(),
                    });
                    continue;
                }
            }
            out.push(LyricLine {
                start_ms,
                end_ms: if timestamp.is_some() {
                    start_ms.saturating_add(dur_ms.max(1))
                } else {
                    0
                },
                text,
                syllables,
                translation: String::new(),
            });
        }
        return out;
    }

    if let Some(plain) = value.get("plain").and_then(Value::as_str) {
        return plain
            .lines()
            .map(|l| LyricLine {
                start_ms: 0,
                end_ms: 0,
                text: l.to_string(),
                syllables: Vec::new(),
                translation: String::new(),
            })
            .collect();
    }

    fn visit(value: &Value, result: &mut Vec<LyricLine>) {
        if let Some(items) = value.as_array() {
            for item in items {
                visit(item, result);
            }
        } else if let Some(object) = value.as_object() {
            let text = crate::catalog::string(value, &["line", "text", "lyrics"]);
            if !text.is_empty() {
                let time = value
                    .get("timestamp")
                    .or_else(|| value.get("milliseconds"))
                    .or_else(|| value.get("time"))
                    .and_then(Value::as_u64)
                    .unwrap_or(0);
                for line in text.lines() {
                    result.push(LyricLine {
                        start_ms: time,
                        end_ms: 0,
                        text: line.to_string(),
                        syllables: Vec::new(),
                        translation: String::new(),
                    });
                }
            } else {
                for child in object.values() {
                    visit(child, result);
                }
            }
        }
    }
    visit(value, &mut out);
    out
}

fn parse_lrclib(value: &Value) -> Vec<LyricLine> {
    if let Some(text) = value["lyricsfile"].as_str() {
        let lines = formats::lyricsfile(text);
        if !lines.is_empty() {
            return lines;
        }
    }
    if let Some(synced) = value.get("syncedLyrics").and_then(Value::as_str) {
        let mut out = Vec::new();
        let offset = synced
            .lines()
            .find_map(|line| {
                line.trim()
                    .strip_prefix("[offset:")?
                    .strip_suffix(']')?
                    .parse::<i64>()
                    .ok()
            })
            .unwrap_or(0);
        for raw_line in synced.lines() {
            let mut line = raw_line.trim();
            let mut stamps = Vec::new();
            while let Some((ts, rest)) = parse_lrc_prefix(line) {
                stamps.push(ts);
                line = rest;
            }
            for ts in stamps {
                let (clean, mut syllables) = parse_inline_tags(line, ts);
                let shift = |ms: u64| {
                    if offset >= 0 {
                        ms.saturating_add(offset as u64)
                    } else {
                        ms.saturating_sub(offset.unsigned_abs())
                    }
                };
                for word in &mut syllables {
                    word.start_ms = shift(word.start_ms);
                    if word.end_ms > 0 {
                        word.end_ms = shift(word.end_ms);
                    }
                }
                out.push(LyricLine {
                    start_ms: shift(ts),
                    end_ms: shift(ts).saturating_add(1),
                    text: clean,
                    syllables,
                    translation: String::new(),
                });
            }
        }
        if !out.is_empty() {
            return out;
        }
    }
    if let Some(plain) = value.get("plainLyrics").and_then(Value::as_str) {
        return plain
            .lines()
            .map(|l| LyricLine {
                start_ms: 0,
                end_ms: 0,
                text: l.to_string(),
                syllables: Vec::new(),
                translation: String::new(),
            })
            .collect();
    }
    Vec::new()
}

fn parse_lrc_prefix(line: &str) -> Option<(u64, &str)> {
    if !line.starts_with('[') {
        return None;
    }
    let close = line.find(']')?;
    let tag = &line[1..close];
    let ms = parse_time_tag(tag)?;
    Some((ms, line[close + 1..].trim()))
}

fn parse_time_tag(tag: &str) -> Option<u64> {
    let (mins, sec_part) = tag.split_once(':')?;
    let m: u64 = mins.parse().ok()?;
    let (secs, frac) = sec_part.split_once('.').unwrap_or((sec_part, ""));
    let s: u64 = secs.parse().ok()?;
    if s >= 60 || frac.len() > 3 || !frac.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let f_ms = match frac.len() {
        0 => 0,
        1 => frac.parse::<u64>().ok()? * 100,
        2 => frac.parse::<u64>().ok()? * 10,
        3 => frac.parse::<u64>().ok()?,
        _ => return None,
    };
    m.checked_mul(60_000)?
        .checked_add(s * 1000)?
        .checked_add(f_ms)
}

fn parse_inline_tags(text: &str, default_start: u64) -> (String, Vec<Syllable>) {
    if !text.contains('<') || !text.contains('>') {
        return (text.to_string(), Vec::new());
    }
    let mut clean = String::new();
    let mut syllables = Vec::new();
    let mut rest = text;
    let mut current_time = default_start;
    let mut explicit = false;
    while let Some(open) = rest.find('<') {
        let Some(close) = rest[open..].find('>').map(|i| open + i) else {
            break;
        };
        let Some(parsed_time) = parse_time_tag(&rest[open + 1..close]) else {
            clean.push_str(&rest[..=close]);
            rest = &rest[close + 1..];
            continue;
        };
        let before = &rest[..open];
        if !before.is_empty() {
            clean.push_str(before);
            syllables.push(Syllable {
                start_ms: current_time,
                end_ms: parsed_time.max(current_time),
                text: before.to_string(),
            });
        }
        if let Some(prev) = syllables.last_mut() {
            prev.end_ms = parsed_time.max(prev.start_ms);
        }
        current_time = parsed_time;
        explicit = true;
        rest = &rest[close + 1..];
    }
    if !rest.is_empty() {
        clean.push_str(rest);
        if explicit {
            syllables.push(Syllable {
                start_ms: current_time,
                end_ms: 0,
                text: rest.to_string(),
            });
        }
    }
    (clean, syllables)
}

fn url_encode(input: &str) -> String {
    let mut out = String::new();
    for b in input.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else if b == b' ' {
            out.push_str("%20");
        } else {
            out.push_str(&format!("%{:02X}", b));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn instrumental_notes_use_real_ends_and_do_not_invent_line_only_breaks() {
        let lines = vec![
            LyricLine {
                start_ms: 5000,
                end_ms: 15000,
                text: "One".into(),
                syllables: vec![Syllable {
                    start_ms: 5000,
                    end_ms: 7000,
                    text: "One".into(),
                }],
                ..Default::default()
            },
            LyricLine {
                start_ms: 15000,
                end_ms: 16000,
                text: "Two".into(),
                ..Default::default()
            },
        ];
        assert_eq!(
            instrumental_gaps(&lines, 20000),
            vec![(0, 5000), (7000, 15000), (16000, 20000)]
        );
        let unknown = vec![
            LyricLine {
                start_ms: 5000,
                text: "One".into(),
                ..Default::default()
            },
            LyricLine {
                start_ms: 15000,
                text: "Two".into(),
                ..Default::default()
            },
        ];
        assert_eq!(instrumental_gaps(&unknown, 20000), vec![(0, 5000)]);
        assert!(instrumental_gaps(
            &[LyricLine {
                text: "Plain".into(),
                ..Default::default()
            }],
            20000
        )
        .is_empty());
    }

    #[test]
    fn wrapped_lyrics_keep_utf8_and_real_word_ranges_intact() {
        let text = "A long line with \u{65e5}\u{672c}\u{8a9e} andaverylongunbrokenword and repeated repeated words";
        let ranges = wrapped_ranges(text, 160, 29.333);
        assert!(ranges.len() > 4);
        let joined: String = ranges.iter().map(|r| &text[r.clone()]).collect();
        assert_eq!(
            joined.split_whitespace().collect::<String>(),
            text.split_whitespace().collect::<String>()
        );
        let line = LyricLine {
            text: "repeated repeated".into(),
            syllables: vec![
                Syllable {
                    text: "repeated".into(),
                    start_ms: 100,
                    end_ms: 200,
                },
                Syllable {
                    text: "repeated".into(),
                    start_ms: 300,
                    end_ms: 400,
                },
            ],
            ..Default::default()
        };
        let words = word_ranges(&line).unwrap();
        assert_eq!(words[0].0, 0..8);
        assert_eq!(words[1].0, 9..17);
    }

    #[test]
    fn karaoke_uses_word_spans_and_holds_during_gaps() {
        let line = LyricLine {
            text: "Wide i".into(),
            syllables: vec![
                Syllable {
                    text: "Wide ".into(),
                    start_ms: 1000,
                    end_ms: 2000,
                },
                Syllable {
                    text: "i".into(),
                    start_ms: 3000,
                    end_ms: 4000,
                },
            ],
            ..Default::default()
        };
        let measure = |s: &str| s.chars().map(|c| if c == 'W' { 4.0 } else { 1.0 }).sum();
        assert_eq!(karaoke_progress(&line, 500, measure), Some(0.0));
        assert_eq!(karaoke_progress(&line, 1500, measure), Some(3.5 / 9.0));
        assert_eq!(karaoke_progress(&line, 2500, measure), Some(7.0 / 9.0));
        assert_eq!(karaoke_progress(&line, 3500, measure), Some(8.5 / 9.0));
        assert_eq!(karaoke_progress(&line, 4500, measure), Some(1.0));
    }

    #[test]
    fn karaoke_matches_repeated_unicode_syllables_without_inventing_alignment() {
        let mut line = LyricLine {
            text: "\u{3089}\u{3089}".into(),
            syllables: vec![
                Syllable {
                    text: "\u{3089}".into(),
                    start_ms: 100,
                    end_ms: 200,
                },
                Syllable {
                    text: "\u{3089}".into(),
                    start_ms: 300,
                    end_ms: 400,
                },
            ],
            ..Default::default()
        };
        assert_eq!(
            karaoke_progress(&line, 350, |s| s.chars().count() as f32),
            Some(0.75)
        );
        line.syllables[1].text = "unrelated".into();
        assert_eq!(karaoke_progress(&line, 350, |s| s.len() as f32), None);
        line.syllables.clear();
        assert_eq!(karaoke_progress(&line, 350, |s| s.len() as f32), None);
    }

    #[test]
    fn richer_real_timing_wins_even_when_a_previous_provider_has_words() {
        let track = Entry {
            title: "Song".into(),
            duration: 10000,
            ..Default::default()
        };
        let mut requested = Vec::new();
        let (source,lines)=fetch_sources(&track,"1",SUPPORTED_SOURCES,false,|source| {
            requested.push(source);
            Ok(match source {
                0=>json!({"plain":"Plain fallback"}),
                3=>json!({"ttml":"<tt><body><p begin='1s' end='2s'><span begin='1s' end='2s'>A</span></p></body></tt>"}),
                4=>json!({"yrc":{"lyric":"[1000,1000](1000,1000,0)A\n[3000,1000](3000,1000,0)B"}}),
                _=>json!({"syncedLyrics":"[00:01]A\n[00:03]B\n[00:04]C"}),
            })
        }).unwrap();
        assert_eq!(requested, [0, 3, 8, 5, 7, 4, 1, 9]);
        assert_eq!(source, "NetEase");
        assert_eq!(lines.len(), 2);
        assert_eq!(score_lyrics(&lines), 3);
    }

    #[test]
    fn instrumental_requires_matching_metadata_and_yields_to_real_lyrics() {
        let track = Entry {
            title: "Song".into(),
            subtitle: "Artist".into(),
            duration: 180000,
            ..Default::default()
        };
        let value =
            json!({"trackName":"Song", "artistName":"Artist", "duration":180, "instrumental":true});
        assert!(matching_instrumental(&track, &value));
        let mut wrong = value.clone();
        wrong["duration"] = json!(220);
        assert!(!matching_instrumental(&track, &wrong));
        wrong = value.clone();
        wrong["artistName"] = json!("Someone else");
        assert!(!matching_instrumental(&track, &wrong));
        let answer = fetch_sources(&track, "1", 2, false, |_| Ok(value.clone())).unwrap();
        assert_eq!(answer.0, "Instrumental");
        assert!(answer.1.is_empty());
        let answer = fetch_sources(&track, "1", 3, false, |source| {
            Ok(if source == 1 {
                value.clone()
            } else {
                json!({"plain":"Actual words"})
            })
        })
        .unwrap();
        assert!(!answer.1.is_empty());
        assert!(fetch_sources(&track, "1", 2, false, |_| Err("Offline".into())).is_err());
    }

    #[test]
    fn provider_failure_keeps_plain_fallback_and_cache_is_bounded() {
        let track = Entry {
            title: "Song".into(),
            ..Default::default()
        };
        let (source, lines) = fetch_sources(&track, "1", SUPPORTED_SOURCES, false, |source| {
            if source == 0 {
                Ok(json!({"plain":"Fallback"}))
            } else {
                Err("Offline".into())
            }
        })
        .unwrap();
        assert_eq!(source, "Deezer");
        assert!(!has_timing(&lines));
        let mut cache = LyricCache::default();
        for n in 0..40 {
            cache.insert(n.to_string(), (source.clone(), lines.clone()));
        }
        assert_eq!(cache.entries.len(), 32);
        assert!(cache.get("0").is_none());
        assert!(cache.get("39").is_some());
    }

    #[test]
    #[ignore = "Opt-in public network provider verification; no account login or playback"]
    fn live_public_word_timing_providers() {
        let engine = Engine::load().unwrap();
        for (source, title, artist, duration) in [
            (3, "ME!", "Taylor Swift", 193000),
            (4, "Hello", "Adele", 295502),
            (1, "Hello", "Adele", 295502),
        ] {
            let track = Entry {
                title: title.into(),
                subtitle: artist.into(),
                duration,
                ..Default::default()
            };
            let value = fetch_provider(&engine, &track, "", source).unwrap();
            let (name, lines) =
                fetch_sources(&track, "", 1 << source, false, |_| Ok(value.clone())).unwrap();
            assert!(!lines.is_empty(), "{name}: no parsed lyrics");
            let score = quality(&lines);
            assert!(score.0 >= 2, "{name}: no timing");
            if source != 1 {
                assert_eq!(score.0, 3, "{name}: no word timing");
            }
            println!(
                "{name}: {} lines, {} word-timed lines",
                lines.len(),
                score.1
            );
        }
    }

    #[test]
    fn disabled_sources_are_never_requested_or_used_as_fallback() {
        let track = Entry {
            title: "Song".into(),
            duration: 10000,
            ..Default::default()
        };
        let mut requested = Vec::new();
        let (source, lines) = fetch_sources(&track, "1", 2, false, |index| {
            requested.push(index);
            Ok(json!({"plainLyrics":"One\nTwo"}))
        })
        .unwrap();
        assert_eq!(requested, [1]);
        assert_eq!(source, "LRCLIB");
        assert!(!has_timing(&lines));
        for mask in [0, 64] {
            assert!(fetch_sources(&track, "1", mask, false, |_| panic!(
                "disabled provider called"
            ))
            .is_err());
        }
    }

    #[test]
    fn line_and_plain_lyrics_never_gain_invented_word_timestamps() {
        let mut plain = parse_deezer_lyrics(&json!({"plain":"One\nTwo"}));
        finalize_timings(&mut plain, 10000);
        assert!(plain
            .iter()
            .all(|line| line.start_ms == 0 && line.end_ms == 0 && line.syllables.is_empty()));
        let mut synced = parse_deezer_lyrics(
            &json!({"synced":[{"timeMs":0,"text":"One"},{"timeMs":2000,"text":"Two words"}]}),
        );
        finalize_timings(&mut synced, 10000);
        assert!(has_timing(&synced));
        assert_eq!(synced[0].end_ms, 2000);
        assert!(synced.iter().all(|line| line.syllables.is_empty()));
        assert_eq!(score_lyrics(&synced), 2);
        let mut missing = parse_deezer_lyrics(&json!({"synced":[{"text":"No timestamp"}]}));
        finalize_timings(&mut missing, 10000);
        assert!(!has_timing(&missing));
    }

    #[test]
    fn lrc_offsets_repeated_lines_and_explicit_word_timing_are_preserved() {
        let mut lines = parse_lrclib(
            &json!({"syncedLyrics":"[offset:-100]\n[00:01.00][00:03.00]Again\n[00:04.00]<00:04.00>Hello <00:04.50>world"}),
        );
        finalize_timings(&mut lines, 6000);
        assert_eq!(
            lines.iter().map(|line| line.start_ms).collect::<Vec<_>>(),
            [900, 2900, 3900]
        );
        assert_eq!(lines[2].text, "Hello world");
        assert_eq!(lines[2].syllables[0].end_ms, 4400);
        assert_eq!(lines[2].syllables[1].start_ms, 4400);
        let (text, words) = parse_inline_tags("I <love> you", 0);
        assert_eq!(text, "I <love> you");
        assert!(words.is_empty());
    }

    #[test]
    fn malformed_timestamps_are_rejected_without_overflow() {
        for tag in ["1:99", "18446744073709551615:01", "1:01.no", "1:01.1234"] {
            assert!(parse_time_tag(tag).is_none());
        }
        assert_eq!(parse_time_tag("01:02.5"), Some(62500));
    }
}

/// Computes 28 visualizer bar heights (0.0..=1.0) driven by playback state, position, and EQ profile.
pub fn visualizer_bars(playing: bool, position_ms: u64, eq: &[i32; 11]) -> [f32; 28] {
    let mut bars = [0.08f32; 28];
    if !playing {
        return bars;
    }
    let t = position_ms as f32 * 0.001;
    for (i, bar) in bars.iter_mut().enumerate() {
        let fi = i as f32;
        let eq_idx = (i * 10 / 28).min(9);
        let eq_boost = 1.0 + (eq[eq_idx] as f32 * 0.045);
        let wave1 = (t * 6.4 + fi * 0.53).sin() * 0.5 + 0.5;
        let wave2 = (t * 11.2 - fi * 0.87).cos() * 0.5 + 0.5;
        let wave3 = (t * 3.7 + fi * 0.29).sin() * 0.5 + 0.5;
        let envelope = 1.0 - ((fi - 13.5).abs() / 16.5) * 0.42;
        let v = (wave1 * 0.48 + wave2 * 0.32 + wave3 * 0.20) * envelope * eq_boost;
        *bar = v.clamp(0.10, 1.0);
    }
    bars
}

/// Sonora's verse haze and past/ahead treatment, adapted from
/// sonorahq/sonora crates/views/src/chrome/aside.rs (GPL-3.0-or-later).
/// Travel is measured in viewport heights from the focused verse.
pub(crate) fn inactive_verse_look(travel: f32, size: f32, passed: bool, blur: bool) -> (f32, f32) {
    let haze = if blur {
        travel.abs().min(1.0).powf(0.45)
    } else {
        0.0
    };
    let opacity = if passed { 0.4 } else { 0.6 } * (1.0 - 0.3 * haze).clamp(0.0, 1.0);
    let radius = (size * 0.13 * haze / 0.25).round() * 0.25;
    (opacity, radius)
}

/// Exact damped spring step, retaining velocity when a fast verse retargets it.
pub(crate) fn lyric_spring(
    position: f32,
    velocity: f32,
    seconds: f32,
    stiffness: f32,
    damping: f32,
) -> (f32, f32) {
    let decay = damping * 0.5;
    let frequency = (stiffness - decay * decay).sqrt();
    let t = seconds.max(0.0);
    let (sin, cos) = (frequency * t).sin_cos();
    let b = (velocity + decay * position) / frequency;
    let envelope = (-decay * t).exp();
    let value = position * cos + b * sin;
    (
        envelope * value,
        envelope * (-decay * value + frequency * (b * cos - position * sin)),
    )
}

#[cfg(test)]
mod verse_look_tests {
    use super::inactive_verse_look;

    #[test]
    fn lyric_spring_keeps_velocity_across_fast_retargets_and_settles() {
        let (position, velocity) = super::lyric_spring(60.0, 0.0, 0.08, 170.0, 23.0);
        let retargeted = position + 60.0;
        let (start, speed) = super::lyric_spring(retargeted, velocity, 0.0, 170.0, 23.0);
        assert!((start - retargeted).abs() < 0.001 && (speed - velocity).abs() < 0.001);
        let (next, speed) = super::lyric_spring(retargeted, velocity, 0.016, 170.0, 23.0);
        assert!(next < retargeted && speed < 0.0);
        let (rest, speed) = super::lyric_spring(next, speed, 1.2, 170.0, 23.0);
        assert!(rest.abs() < 0.05 && speed.abs() < 0.5);
    }

    #[test]
    fn sonora_haze_tracks_distance_and_preserves_directional_opacity() {
        let near = inactive_verse_look(0.1, 32.0, false, true);
        let far = inactive_verse_look(0.8, 32.0, false, true);
        let past = inactive_verse_look(-0.8, 32.0, true, true);
        assert!(far.1 > near.1 && far.0 < near.0);
        assert_eq!(past.1, far.1);
        assert!(past.0 < far.0);
        assert_eq!(far.1 * 4.0, (far.1 * 4.0).round());
        assert_eq!(inactive_verse_look(0.8, 32.0, true, false), (0.4, 0.0));
    }
}
