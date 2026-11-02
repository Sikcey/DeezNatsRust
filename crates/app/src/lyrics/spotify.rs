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
        assert!(parse(&serde_json::json!({"lyrics":{"syncType":"UNKNOWN","lines":[]}})).is_empty());
        assert!(
            parse(&serde_json::json!({"lyrics":{"syncType":"LINE_SYNCED","lines":"bad"}}))
                .is_empty()
        );
    }
}
