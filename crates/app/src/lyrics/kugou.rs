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
        let Some((header, mut rest)) = raw.strip_prefix('[').and_then(|s| s.split_once(']')) else {
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
            let (Ok(relative), Ok(duration)) = (fields[0].parse::<u64>(), fields[1].parse::<u64>())
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
