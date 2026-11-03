//! Multi-provider lyrics resolution, syllable/karaoke parsing, and audio visualizer state.
use crate::{catalog::Entry, engine::Engine};
use serde_json::Value;
#[path = "lyrics/extras.rs"]
mod extras;
#[path = "lyrics/formats.rs"]
mod formats;
#[path = "lyrics/http.rs"]
pub(crate) mod http;
#[path = "lyrics/kugou.rs"]
mod kugou;
#[path = "lyrics/providers.rs"]
mod providers;
#[path = "lyrics/spotify.rs"]
mod spotify_native;

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
    // Resolve independent providers concurrently; this stays off the UI/audio
    // threads and is bounded by the enabled provider slots.
    let results = std::thread::scope(|scope| {
        let handles: Vec<_> = [0, 3, 8, 5, 7, 2, 4, 1, 9]
            .into_iter()
            .filter(|source| enabled_sources & (1 << source) != 0)
            .map(|source| {
                (
                    source,
                    scope.spawn(move || fetch_provider_inner(engine, track, id, source)),
                )
            })
            .collect();
        handles
            .into_iter()
            .map(|(source, handle)| {
                (
                    source,
                    handle
                        .join()
                        .unwrap_or_else(|_| Err("Lyrics provider failed".into())),
                )
            })
            .collect::<std::collections::HashMap<_, _>>()
    });
    let mut results = results;
    let result = fetch_sources(track, id, enabled_sources, engine.is_none(), |source| {
        results
            .remove(&source)
            .unwrap_or_else(|| Err("Lyrics provider unavailable".into()))
    });
    if let Ok(value) = &result {
        if !value.1.is_empty() {
            lyric_cache().lock().unwrap().insert(key, value.clone());
        }
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
        if item.1.elapsed() > std::time::Duration::from_secs(1800) {
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
    if best.1.is_empty() && !errors.is_empty() {
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
