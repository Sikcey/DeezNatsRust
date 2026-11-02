use super::*;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;

pub(super) const PREFIX: &str = "spotify-flow-page:";
#[derive(Serialize, Deserialize)]
struct Source {
    uri: String,
    offset: u64,
}
#[derive(Serialize, Deserialize)]
struct Queue {
    config: String,
    sources: VecDeque<Source>,
    pending: VecDeque<Value>,
    seen: HashSet<String>,
}

fn editorial(v: &Value) -> bool {
    v["type"] == "playlist"
        && v.pointer("/owner/uri").and_then(Value::as_str) == Some("spotify:user:spotify")
}

pub(super) fn start(config: &str) -> ResultValue {
    let selection = super::super::flow::Selection::parse(config)?;
    let mut sources = VecDeque::new();
    let mut seen = HashSet::new();
    for query in selection.queries() {
        // Spotify's editorial search results are the authority for the selected topic.
        // Do not substitute arbitrary user playlists or unrelated Home mixes.
        let value = search(&query)?;
        for item in collect(&value["playlists"])
            .into_iter()
            .filter(editorial)
            .take(3)
        {
            let uri = text(&item, &["uri"]).to_owned();
            if seen.insert(uri.clone()) {
                sources.push_back(Source { uri, offset: 0 });
            }
        }
    }
    if selection.mood == "default" {
        for genre in &selection.genres {
            if let Some(uri) = super::super::flow::genre_radio(genre) {
                if seen.insert(uri.to_owned()) {
                    sources.push_back(Source {
                        uri: uri.to_owned(),
                        offset: 0,
                    });
                }
            }
        }
    }
    if sources.is_empty() {
        return Err("Spotify returned no editorial playlists for these Flow selections. Try another mood or genre.".into());
    }
    batch(
        Queue {
            config: selection.key(),
            sources,
            pending: VecDeque::new(),
            seen: HashSet::new(),
        },
        fetch,
    )
}

pub(super) fn next(cursor: &str) -> ResultValue {
    if cursor.len() > 256_000 {
        return Err("Flow cursor is too large. Refresh Flow.".into());
    }
    let queue: Queue =
        serde_json::from_str(cursor.strip_prefix(PREFIX).ok_or("Invalid Flow cursor")?)
            .map_err(|_| "Invalid Flow cursor")?;
    super::super::flow::Selection::parse(&queue.config)?;
    if queue.sources.len() > 128 * 4
        || queue.pending.len() > 10
        || queue.seen.len() > 3000
        || queue
            .sources
            .iter()
            .any(|s| !s.uri.starts_with("spotify:playlist:") || s.offset > 100_000)
    {
        return Err("Invalid Flow cursor".into());
    }
    batch(queue, fetch)
}

fn fetch(uri: &str, offset: u64) -> ResultValue {
    page(
        "fetchPlaylist",
        json!({"uri":uri,"offset":offset,"limit":10,"enableWatchFeedEntrypoint":false}),
        "/data/playlistV2/content",
    )
}

fn batch(mut queue: Queue, mut fetch: impl FnMut(&str, u64) -> ResultValue) -> ResultValue {
    let mut items = Vec::new();
    let mut reads = 0;
    loop {
        while items.len() < 10 {
            let Some(mut track) = queue.pending.pop_front() else {
                break;
            };
            let uri = text(&track, &["uri"]).to_owned();
            if track["type"] == "track"
                && !uri.is_empty()
                && track["is_playable"] != false
                && queue.seen.insert(uri)
            {
                track["spotify_station"] =
                    json!({"name":"DZFlowForConfigJSON","args":[queue.config]});
                items.push(track);
            }
        }
        if items.len() == 10 || queue.sources.is_empty() || reads == 30 || queue.seen.len() >= 3000
        {
            break;
        }
        let mut source = queue.sources.pop_front().unwrap();
        let value = fetch(&source.uri, source.offset)?;
        reads += 1;
        queue.pending.extend(collect(&value).into_iter().take(10));
        if let Some(next) = value["next"]
            .as_str()
            .and_then(|s| s.strip_prefix(CURSOR))
            .and_then(|s| serde_json::from_str::<Value>(s).ok())
            .and_then(|v| v["vars"]["offset"].as_u64())
            .filter(|n| *n > source.offset && *n <= 100_000)
        {
            source.offset = next;
            queue.sources.push_back(source);
        }
    }
    let more = (!queue.sources.is_empty() || !queue.pending.is_empty()) && queue.seen.len() < 3000;
    let next = more.then(|| format!("{PREFIX}{}", serde_json::to_string(&queue).unwrap()));
    Ok(json!({"items":items,"next":next}))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ten_tracks_then_more_without_duplicates_or_episodes() {
        let queue = Queue {
            config: "default".into(),
            sources: VecDeque::from([Source {
                uri: "spotify:playlist:test".into(),
                offset: 0,
            }]),
            pending: VecDeque::new(),
            seen: HashSet::new(),
        };
        let fetch = |_: &str, offset: u64| -> ResultValue {
            let start = if offset == 0 { 0 } else { 8 };
            Ok(
                json!({"items":(start..start+10).map(|n| json!({"type":"track","name":format!("Track {n}"),"uri":format!("spotify:track:{n}")})).collect::<Vec<_>>(),
                "next":if offset == 0 { Some(format!("{CURSOR}{}",json!({"vars":{"offset":10}}))) } else { None }}),
            )
        };
        let first = batch(queue, fetch).unwrap();
        assert_eq!(first["items"].as_array().unwrap().len(), 10);
        let cursor = first["next"]
            .as_str()
            .unwrap()
            .strip_prefix(PREFIX)
            .unwrap();
        let second = batch(serde_json::from_str(cursor).unwrap(), fetch).unwrap();
        assert_eq!(second["items"].as_array().unwrap().len(), 8);
        assert!(second["next"].is_null());
        assert_eq!(second["items"][0]["uri"], "spotify:track:10");
    }
    #[test]
    fn editorial_owner_is_required() {
        assert!(editorial(
            &json!({"type":"playlist","owner":{"uri":"spotify:user:spotify"}})
        ));
        assert!(!editorial(
            &json!({"type":"playlist","name":"Jazz Radio","owner":{"uri":"spotify:user:someone"}})
        ));
    }
    #[test]
    fn skips_unplayable_tracks_and_episodes_and_rotates_sources() {
        let queue = Queue {
            config: "default".into(),
            sources: VecDeque::from([
                Source {
                    uri: "spotify:playlist:one".into(),
                    offset: 0,
                },
                Source {
                    uri: "spotify:playlist:two".into(),
                    offset: 0,
                },
            ]),
            pending: VecDeque::new(),
            seen: HashSet::new(),
        };
        let value = batch(queue, |uri, _| {
            Ok(json!({"items": if uri.ends_with("one") {
                vec![json!({"uri":"spotify:track:blocked","name":"Blocked","is_playable":false}),
                    json!({"uri":"spotify:episode:talk","name":"Talk"})]
            } else {
                vec![json!({"uri":"spotify:track:good","name":"Good"})]
            }}))
        })
        .unwrap();
        assert_eq!(value["items"].as_array().unwrap().len(), 1);
        assert_eq!(value["items"][0]["uri"], "spotify:track:good");
        assert!(value["next"].is_null());
    }
}
