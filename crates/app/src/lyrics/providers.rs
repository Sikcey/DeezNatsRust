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
