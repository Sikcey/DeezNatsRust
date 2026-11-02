//! Filter only explicit track language metadata; never fetch lyrics for discovery.
use serde_json::Value;

fn languages(region: usize) -> &'static [&'static str] {
    match crate::style_mix::REGIONS[region].0 {
        "Turkish" => &["tr"],
        "Japanese" => &["ja"],
        "Korean" => &["ko"],
        "French" => &["fr"],
        "Arabic" => &["ar"],
        "Brazilian" | "Portuguese" => &["pt"],
        "Spanish" | "Mexican" | "Argentinian" | "Colombian" | "Chilean" | "Peruvian" | "Cuban"
        | "Dominican" | "Puerto Rican" => &["es"],
        "German" => &["de"],
        "Italian" => &["it"],
        "Hindi" => &["hi"],
        "Punjabi" => &["pa"],
        "Persian" => &["fa"],
        "Greek" => &["el"],
        "Russian" => &["ru"],
        "Chinese" => &["zh"],
        "Indonesian" => &["id"],
        "Thai" => &["th"],
        "Polish" => &["pl"],
        "Dutch" => &["nl"],
        "Swedish" => &["sv"],
        "Norwegian" => &["no", "nb", "nn"],
        "Finnish" => &["fi"],
        "Danish" => &["da"],
        "Ukrainian" => &["uk"],
        "Romanian" => &["ro"],
        "Hungarian" => &["hu"],
        "Czech" => &["cs"],
        "Serbian" => &["sr"],
        "Croatian" => &["hr"],
        "Albanian" => &["sq"],
        "Bengali" => &["bn"],
        "Tamil" => &["ta"],
        "Telugu" => &["te"],
        "Urdu" => &["ur"],
        "Vietnamese" => &["vi"],
        "Filipino / Tagalog" => &["tl", "fil"],
        "Malay" => &["ms"],
        "Nepali" => &["ne"],
        "Hebrew" => &["he", "iw"],
        "Azerbaijani" => &["az"],
        "Georgian" => &["ka"],
        "Armenian" => &["hy"],
        "Kurdish" => &["ku", "ckb", "kmr"],
        "Swahili" => &["sw"],
        "Amharic" => &["am"],
        "Icelandic" => &["is"],
        "Slovak" => &["sk"],
        "Slovenian" => &["sl"],
        // These are country choices with multiple vocal languages.
        _ => &[],
    }
}

pub(crate) fn retain_language<T>(
    items: &mut Vec<T>,
    selection: crate::style_mix::Selection,
    metadata: impl Fn(&T) -> &Value,
) {
    let expected = languages(selection.region);
    if expected.is_empty() {
        return;
    }
    items.retain(|item| {
        let value = metadata(item);
        let language = value["language"]
            .as_str()
            .or_else(|| value.pointer("/lyrics/language").and_then(Value::as_str));
        language.is_none_or(|language| {
            let normalized = language.trim().to_ascii_lowercase();
            normalized.is_empty()
                || expected.contains(&normalized.split(['-', '_']).next().unwrap_or_default())
        })
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn missing_metadata_survives_and_explicit_language_is_respected() {
        let mut tracks = vec![
            json!({}),
            json!({"language":""}),
            json!({"language":"TR-tr"}),
            json!({"lyrics":{"language":"en"}}),
        ];
        retain_language(
            &mut tracks,
            crate::style_mix::Selection::new(0, 1).unwrap(),
            |v| v,
        );
        assert_eq!(tracks.len(), 3);
        assert_eq!(tracks[2]["language"], "TR-tr");
    }

    #[test]
    fn missing_metadata_does_not_truncate_playlists() {
        let mut tracks = vec![json!({}); 100];
        retain_language(
            &mut tracks,
            crate::style_mix::Selection::new(0, 1).unwrap(),
            |v| v,
        );
        assert_eq!(tracks.len(), 100);
    }
}
