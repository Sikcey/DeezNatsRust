//! Optional public metadata for sparse provider track responses.
use super::Entry;
use serde_json::Value;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

fn normalized(s: &str) -> String {
    s.chars()
        .flat_map(char::to_lowercase)
        .filter(|c| c.is_alphanumeric())
        .collect()
}

pub(crate) fn enrich(track: &Entry, meta: Value) -> Value {
    let known = |paths: &[&str]| {
        paths.iter().any(|path| {
            meta.pointer(path).is_some_and(|value| {
                !value.is_null()
                    && value.as_str() != Some("")
                    && value.as_array().is_none_or(|values| !values.is_empty())
            })
        })
    };
    if known(&["/genres", "/genre", "/album/genres"])
        && known(&["/label", "/album/label"])
        && known(&[
            "/release_date",
            "/releaseDate",
            "/album/release_date",
            "/album/releaseDate",
        ])
        && known(&["/track_number", "/track_position"])
    {
        return meta;
    }
    use std::hash::{Hash, Hasher};
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    (
        &track.id,
        &track.title,
        &track.subtitle,
        &track.album,
        track.duration,
        meta.to_string(),
    )
        .hash(&mut hash);
    let key = hash.finish();
    static CACHE: OnceLock<Mutex<std::collections::VecDeque<(u64, Instant, Value)>>> =
        OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(std::collections::VecDeque::new()));
    if let Ok(cache) = cache.lock() {
        if let Some((_, _, value)) = cache
            .iter()
            .find(|(id, time, _)| *id == key && time.elapsed() < Duration::from_secs(3600))
        {
            return value.clone();
        }
    }
    let result = enrich_uncached(track, meta);
    if let Ok(mut cache) = cache.lock() {
        cache.retain(|(id, _, _)| *id != key);
        cache.push_front((key, Instant::now(), result.clone()));
        cache.truncate(32);
    }
    result
}

fn fetch(url: &str) -> Option<Value> {
    static LAST: OnceLock<Mutex<Option<Instant>>> = OnceLock::new();
    let mut last = LAST.get_or_init(|| Mutex::new(None)).lock().ok()?;
    if let Some(previous) = *last {
        std::thread::sleep(Duration::from_millis(1100).saturating_sub(previous.elapsed()));
    }
    *last = Some(Instant::now());
    crate::lyrics::http::json(
        url,
        &[(
            "User-Agent",
            "DeezNatsRust/0.1 (https://github.com/Sikcey/DeezNats)",
        )],
    )
    .ok()
}

fn lookup(kind: &str, id: &Value, include: &str) -> Option<Value> {
    let id = id
        .as_str()
        .filter(|id| id.len() == 36 && id.bytes().all(|b| b.is_ascii_hexdigit() || b == b'-'))?;
    let mut url = url::Url::parse(&format!("https://musicbrainz.org/ws/2/{kind}/{id}")).ok()?;
    url.query_pairs_mut()
        .append_pair("inc", include)
        .append_pair("fmt", "json");
    fetch(url.as_str())
}

fn enrich_uncached(track: &Entry, mut meta: Value) -> Value {
    let artists = track.artist_entries();
    let artist = artists
        .first()
        .map(|artist| artist.title.as_str())
        .filter(|name| !name.is_empty())
        .unwrap_or(track.subtitle.trim());
    if track.title.trim().is_empty()
        || artist.is_empty()
        || (track.duration == 0 && track.album.is_empty())
    {
        return meta;
    }
    let clean = |s: &str| {
        s.chars()
            .map(|c| {
                if c.is_alphanumeric() || c.is_whitespace() {
                    c
                } else {
                    ' '
                }
            })
            .collect::<String>()
    };
    let query = format!(
        "recording:\"{}\" AND artist:\"{}\"",
        clean(&track.title),
        clean(artist)
    );
    let mut url = url::Url::parse("https://musicbrainz.org/ws/2/recording/").unwrap();
    url.query_pairs_mut()
        .append_pair("query", &query)
        .append_pair("fmt", "json")
        .append_pair("limit", "10");
    let Some(result) = fetch(url.as_str()) else {
        return meta;
    };
    let Some(rows) = result["recordings"].as_array() else {
        return meta;
    };
    let title = normalized(&track.title);
    let artist = normalized(artist);
    let Some(row) = rows
        .iter()
        .filter(|row| {
            normalized(row["title"].as_str().unwrap_or("")) == title
                && row["artist-credit"].as_array().is_some_and(|credits| {
                    credits.iter().any(|c| {
                        normalized(
                            c["artist"]["name"]
                                .as_str()
                                .or_else(|| c["name"].as_str())
                                .unwrap_or(""),
                        ) == artist
                    })
                })
                && (track.duration == 0
                    || row["length"]
                        .as_u64()
                        .is_some_and(|length| length.abs_diff(track.duration as u64) <= 4000))
        })
        .max_by_key(|row| {
            !track.album.is_empty()
                && row["releases"].as_array().is_some_and(|releases| {
                    releases.iter().any(|r| {
                        normalized(r["title"].as_str().unwrap_or("")) == normalized(&track.album)
                    })
                })
        })
    else {
        return meta;
    };
    let detail = lookup(
        "recording",
        &row["id"],
        "releases+artist-credits+isrcs+genres",
    )
    .unwrap_or_else(|| row.clone());
    if !meta.is_object() {
        meta = serde_json::json!({});
    }
    let release = row["releases"].as_array().and_then(|releases| {
        releases.iter().find(|release| {
            !track.album.is_empty()
                && normalized(release["title"].as_str().unwrap_or("")) == normalized(&track.album)
        })
    });
    let release = release.map(|release| {
        lookup(
            "release",
            &release["id"],
            "labels+recordings+artist-credits",
        )
        .unwrap_or_else(|| release.clone())
    });
    apply_details(&mut meta, &detail, release.as_ref());
    meta
}

fn apply_details(meta: &mut Value, recording: &Value, release: Option<&Value>) {
    let empty = |value: &Value| {
        value.is_null() || value.as_str() == Some("") || value.as_array().is_some_and(Vec::is_empty)
    };
    if empty(&meta["genres"]) && empty(&meta["genre"]) && empty(&meta["album"]["genres"]) {
        let genres: Vec<_> = recording["genres"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|genre| genre["count"].as_i64().unwrap_or(1) > 0)
            .take(5)
            .filter_map(|genre| genre["name"].as_str())
            .collect();
        if !genres.is_empty() {
            meta["genres"] = serde_json::json!(genres);
        }
    }
    if empty(&meta["isrc"]) && empty(&meta["external_ids"]["isrc"]) {
        if let Some(isrc) = recording["isrcs"]
            .as_array()
            .and_then(|values| values.first())
            .and_then(Value::as_str)
        {
            meta["isrc"] = isrc.into();
        }
    }
    if let Some(release) = release {
        if ["release_date", "releaseDate", "physical_release_date"]
            .iter()
            .all(|key| empty(&meta[*key]))
            && empty(&meta["album"]["release_date"])
            && empty(&meta["album"]["releaseDate"])
        {
            if let Some(date) = release["date"].as_str().filter(|date| !date.is_empty()) {
                meta["release_date"] = date.into();
            }
        }
        if empty(&meta["label"]) && empty(&meta["label_name"]) && empty(&meta["album"]["label"]) {
            let labels: Vec<_> = release["label-info"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|info| info["label"]["name"].as_str())
                .collect();
            if !labels.is_empty() {
                meta["label"] = labels.join(" / ").into();
            }
        }
        if empty(&meta["track_number"]) && empty(&meta["track_position"]) {
            let track = release["media"]
                .as_array()
                .into_iter()
                .flatten()
                .flat_map(|medium| medium["tracks"].as_array().into_iter().flatten())
                .find(|track| {
                    !recording["id"].is_null() && track["recording"]["id"] == recording["id"]
                });
            if let Some(number) = track
                .and_then(|track| track.get("number"))
                .filter(|number| !empty(number))
            {
                meta["track_number"] = number.clone();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn third_party_details_fill_gaps_without_replacing_provider_values_or_statistics() {
        let mut meta = json!({"label":"Provider label","popularity":0,"explicit":true});
        apply_details(
            &mut meta,
            &json!({"id":"matched","genres":[{"name":"R&B","count":5}]}),
            Some(
                &json!({"label-info":[{"label":{"name":"Other label"}}],"media":[{"tracks":[{"number":"1","recording":{"id":"other"}},{"number":"4","recording":{"id":"matched"}}]}]}),
            ),
        );
        assert_eq!(meta["label"], "Provider label");
        assert_eq!(meta["track_number"], "4");
        assert_eq!(meta["genres"][0], "R&B");
        assert_eq!(meta["popularity"], 0);
        assert!(meta.get("playCount").is_none());
        assert!(meta.get("language").is_none());
    }
}
