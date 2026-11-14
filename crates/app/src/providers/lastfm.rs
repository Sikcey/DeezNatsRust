use serde_json::{json, Value};
use std::{
    collections::{HashMap, HashSet},
    sync::{mpsc, Mutex},
    time::{Duration, Instant},
};

const VERSION: &str = "mood-artists-v2";
const BUNDLED_API_KEY: &str = "1262daa4f22ee3313a5ae74dd13a44a6";
static RETRY: Mutex<Option<HashMap<String, Instant>>> = Mutex::new(None);
static REQUEST_GATE: Mutex<Option<Instant>> = Mutex::new(None);

fn valid_key(key: &str) -> bool {
    key.len() == 32 && key.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn api_key() -> Option<String> {
    resolve_key(std::env::var("DEEZNATS_LASTFM_API_KEY").ok(), || {
        crate::session::provider_saved("lastfm").ok()
    })
}

fn resolve_key(
    environment: Option<String>,
    saved: impl FnOnce() -> Option<String>,
) -> Option<String> {
    let key = environment
        .or_else(saved)
        .unwrap_or_else(|| BUNDLED_API_KEY.to_owned());
    let key = key.trim().to_owned();
    valid_key(&key).then_some(key)
}

fn aliases(mood: &str) -> &'static [&'static str] {
    match mood {
        "chill" => &["chill", "chillout"],
        "focus" => &["focus", "study"],
        "party" => &["party", "dance party"],
        "workout" => &["workout", "running"],
        "happy" => &["happy", "feel good"],
        "sad" => &["sad", "melancholic"],
        "sleep" => &["sleep", "sleep music"],
        "romance" => &["romantic", "love songs"],
        _ => &[],
    }
}

fn normalize(name: &str) -> String {
    let key: String = name
        .to_lowercase()
        .chars()
        .filter(|ch| ch.is_alphanumeric())
        .collect();
    match key.as_str() {
        "rnb" | "rhythmandblues" => "rb".into(),
        _ => key,
    }
}

fn request(key: &str, method: &str, fields: &[(&str, &str)]) -> Result<Value, String> {
    {
        let mut gate = REQUEST_GATE
            .lock()
            .map_err(|_| "Last.fm request gate unavailable")?;
        if let Some(last) = *gate {
            std::thread::sleep(Duration::from_millis(300).saturating_sub(last.elapsed()));
        }
        *gate = Some(Instant::now());
    }
    let mut url = url::Url::parse("https://ws.audioscrobbler.com/2.0/")
        .map_err(|_| "Invalid Last.fm endpoint")?;
    url.query_pairs_mut()
        .append_pair("method", method)
        .append_pair("api_key", key)
        .append_pair("format", "json")
        .extend_pairs(fields.iter().copied());
    let response = crate::lyrics::http::response("GET", url.as_str(), &[], &[])
        .map_err(|_| "Last.fm request failed")?;
    if response.status != 200 {
        return Err("Last.fm is unavailable or rate limited".into());
    }
    let value: Value =
        serde_json::from_slice(&response.body).map_err(|_| "Invalid Last.fm response")?;
    if value.get("error").is_some() {
        return Err("Last.fm rejected the metadata request".into());
    }
    Ok(value)
}

fn rank(artists: &[Value]) -> Vec<Value> {
    let mut catalog = HashMap::new();
    for genre in crate::spotify::flow::genres() {
        let key = normalize(genre);
        if matches!(
            key.as_str(),
            "focus"
                | "study"
                | "concentration"
                | "sleep"
                | "sleepmusic"
                | "chill"
                | "party"
                | "danceparty"
                | "workout"
                | "running"
                | "happy"
                | "feelgood"
                | "sad"
                | "melancholic"
                | "romance"
                | "romantic"
                | "lovesongs"
        ) {
            continue;
        }
        catalog.entry(key).or_insert(*genre);
    }
    let mut scores = HashMap::<String, (f64, usize)>::new();
    for artist in artists {
        let Some(tags) = artist.pointer("/toptags/tag").and_then(Value::as_array) else {
            continue;
        };
        let mut seen = HashSet::new();
        for (index, tag) in tags.iter().take(20).enumerate() {
            let Some(name) = tag["name"].as_str() else {
                continue;
            };
            let Some(genre) = catalog.get(&normalize(name)) else {
                continue;
            };
            if !seen.insert(*genre) {
                continue;
            }
            let score = scores.entry((*genre).to_owned()).or_default();
            score.0 += 1.0 / (index + 1) as f64;
            score.1 += 1;
        }
    }
    let mut scores: Vec<_> = scores
        .into_iter()
        .filter(|(_, (_, support))| *support >= 2)
        .collect();
    scores.sort_by(|left, right| {
        right
            .1
             .0
            .total_cmp(&left.1 .0)
            .then_with(|| left.0.cmp(&right.0))
    });
    scores
        .into_iter()
        .take(8)
        .map(|(genre, (score, support))| json!({"genre":genre,"score":score,"artists":support}))
        .collect()
}

fn discover_with(
    mood: &str,
    mut fetch: impl FnMut(&str, &[(&str, &str)]) -> Result<Value, String>,
) -> Result<Value, String> {
    let started = Instant::now();
    let mut names = Vec::new();
    let mut seen = HashSet::new();
    for tag in aliases(mood) {
        if started.elapsed() > Duration::from_secs(12) {
            break;
        }
        let value = fetch("tag.getTopArtists", &[("tag", tag), ("limit", "4")])?;
        for artist in value
            .pointer("/topartists/artist")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .take(4)
        {
            if let Some(name) = artist["name"]
                .as_str()
                .filter(|name| !name.trim().is_empty())
            {
                if seen.insert(name.to_lowercase()) {
                    names.push(name.to_owned());
                }
            }
        }
    }
    let mut artists = Vec::new();
    for name in names.iter().take(8) {
        if started.elapsed() > Duration::from_secs(12) {
            break;
        }
        artists.push(fetch(
            "artist.getTopTags",
            &[("artist", name), ("autocorrect", "1")],
        )?);
    }
    let genres = rank(&artists);
    if genres.is_empty() {
        return Err("Insufficient Last.fm genre evidence".into());
    }
    Ok(json!({"mood":mood,"source":"lastfm-artist-tags","genres":genres}))
}

fn recommendations(mood: &str) -> Option<Value> {
    if aliases(mood).is_empty() {
        return None;
    }
    let key = api_key()?;
    let cache_key = format!("{VERSION}:{mood}");
    if let Some(cached) = crate::catalog::disk_cache::peek("lastfm", 0, &cache_key) {
        return Some(cached);
    }
    let stale = crate::catalog::disk_cache::refresh("lastfm", 0, &cache_key, || {
        Err("Last.fm refresh pending".into())
    })
    .ok();
    {
        let mut retry = RETRY.lock().ok()?;
        let retry = retry.get_or_insert_with(HashMap::new);
        if retry.get(mood).is_some_and(|until| Instant::now() < *until) {
            return stale;
        }
        retry.insert(mood.to_owned(), Instant::now() + Duration::from_secs(60));
    }
    let mood = mood.to_owned();
    let (sender, receiver) = mpsc::sync_channel(1);
    std::thread::spawn(move || {
        let result = crate::catalog::disk_cache::load("lastfm", 0, &cache_key, || {
            discover_with(&mood, |method, fields| request(&key, method, fields))
        });
        let _ = sender.send(result);
    });
    receiver
        .recv_timeout(Duration::from_secs(8))
        .ok()
        .and_then(Result::ok)
        .or(stale)
}

fn apply(tuner: &mut Value, data: &Value) {
    let saved = tuner["id"]
        .as_str()
        .is_some_and(|key| key.starts_with(crate::spotify::flow::PREFIX));
    let Some(recommendations) = data["genres"]
        .as_array()
        .filter(|genres| !genres.is_empty())
    else {
        return;
    };
    let selected: HashSet<_> = recommendations
        .iter()
        .filter_map(|genre| genre["genre"].as_str())
        .map(normalize)
        .collect();
    let Some(genres) = tuner["genres"].as_array_mut() else {
        return;
    };
    if !genres.iter().any(|genre| {
        genre["title"]
            .as_str()
            .is_some_and(|title| selected.contains(&normalize(title)))
    }) {
        return;
    }
    for genre in genres {
        let recommended = genre["title"]
            .as_str()
            .is_some_and(|title| selected.contains(&normalize(title)));
        genre["isMoodGenre"] = json!(recommended);
        if !saved {
            genre["isEnabled"] = json!(recommended);
        }
    }
    tuner["moodSource"] = json!("lastfm-artist-tags");
    tuner["moodEvidence"] = data.clone();
}

pub(crate) fn enrich(mut tuner: Value) -> Value {
    let Some(id) = tuner["id"].as_str() else {
        return tuner;
    };
    if id.starts_with(crate::spotify::flow::PREFIX) {
        return tuner;
    }
    if let Some(data) = recommendations(id) {
        apply(&mut tuner, &data);
    }
    tuner
}

pub(crate) fn tuner(
    key: &str,
    directory: &Value,
    playlists: impl FnOnce() -> Value,
) -> Result<Value, String> {
    let baseline = enrich(crate::spotify::flow::provider_tuner(
        key,
        directory,
        &json!([]),
    )?);
    if baseline["moodSource"] == "lastfm-artist-tags"
        || key == "default"
        || key.starts_with(crate::spotify::flow::PREFIX)
    {
        return Ok(baseline);
    }
    crate::spotify::flow::provider_tuner(key, directory, &playlists()).map(enrich)
}

fn configure(path: &str) -> Result<(), String> {
    if std::fs::metadata(path)
        .map_err(|_| "Could not read Last.fm key file")?
        .len()
        > 128
    {
        return Err("Invalid Last.fm key file".into());
    }
    let key = std::fs::read_to_string(path).map_err(|_| "Could not read Last.fm key file")?;
    if !valid_key(key.trim()) {
        return Err("Invalid Last.fm API key".into());
    }
    crate::session::provider_save("lastfm", key.trim())
}

pub(crate) fn run_cli() -> Result<(), String> {
    let args: Vec<_> = std::env::args()
        .skip_while(|arg| arg != "--lastfm")
        .skip(1)
        .collect();
    match args.as_slice() {
        [action, path] if action == "configure" => configure(path),
        [action, mood] if action == "probe" && !aliases(mood).is_empty() => {
            let value = recommendations(mood).ok_or(
                "Last.fm metadata unavailable; check configuration or retry after a minute",
            )?;
            println!("{value}");
            Ok(())
        }
        _ => Err("Usage: --lastfm configure <private-key-file> | --lastfm probe <mood>".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_key_works_without_user_configuration() {
        assert!(valid_key(BUNDLED_API_KEY));
        assert_eq!(resolve_key(None, || None).as_deref(), Some(BUNDLED_API_KEY));
        let saved = "b".repeat(32);
        assert_eq!(resolve_key(None, || Some(saved.clone())), Some(saved));
        let environment = "c".repeat(32);
        assert_eq!(
            resolve_key(Some(format!(" {environment}\n")), || panic!(
                "Saved key should not be read"
            )),
            Some(environment)
        );
        assert!(resolve_key(Some("invalid".into()), || None).is_none());
    }

    #[test]
    #[ignore = "requires network access; validates the bundled app key without local configuration"]
    fn live_bundled_key() {
        let key = resolve_key(None, || None).unwrap();
        let response = request(
            &key,
            "tag.getTopArtists",
            &[("tag", "focus"), ("limit", "1")],
        )
        .unwrap();
        assert!(!response
            .pointer("/topartists/artist")
            .and_then(Value::as_array)
            .unwrap()
            .is_empty());
    }

    #[test]
    fn ranking_requires_multiple_artists_and_known_genres() {
        let artists = vec![
            json!({"toptags":{"tag":[{"name":"ambient"},{"name":"seen live"},{"name":"jazz"},{"name":"hip-hop"}]}}),
            json!({"toptags":{"tag":[{"name":"Ambient"},{"name":"ambient"},{"name":"hip hop"},{"name":"focus"},{"name":"sleep"}]}}),
            json!({"toptags":{"tag":[{"name":"focus"},{"name":"sleep"}]}}),
        ];
        let ranked = rank(&artists);
        assert_eq!(ranked[0]["genre"], "Ambient");
        assert_eq!(ranked[0]["artists"], 2);
        assert!(ranked.iter().any(|genre| genre["genre"] == "Hip Hop"));
        assert_eq!(ranked.len(), 2);
    }

    #[test]
    fn saved_and_all_moods_never_request_discovery() {
        for key in [
            "default".to_owned(),
            crate::spotify::flow::Selection {
                mood: "focus".into(),
                genres: vec![],
            }
            .key(),
        ] {
            let value =
                tuner(&key, &json!([]), || panic!("Unexpected playlist discovery")).unwrap();
            assert!(value["genres"]
                .as_array()
                .unwrap()
                .iter()
                .all(|genre| genre["isEnabled"] == false));
        }
        assert!(valid_key(&"a".repeat(32)));
        assert!(!valid_key(&"a".repeat(31)));
        assert!(!valid_key(&"z".repeat(32)));
    }

    #[test]
    fn discovery_bounds_and_deduplicates_artists() {
        let mut calls = Vec::new();
        let result = discover_with("focus", |method, fields| {
            calls.push((
                method.to_owned(),
                fields
                    .to_vec()
                    .iter()
                    .map(|(key, value)| (key.to_string(), value.to_string()))
                    .collect::<Vec<_>>(),
            ));
            if method == "tag.getTopArtists" {
                Ok(json!({"topartists":{"artist":[{"name":"One"},{"name":"ONE"},{"name":"Two"}]}}))
            } else {
                Ok(json!({"toptags":{"tag":[{"name":"ambient"}]}}))
            }
        })
        .unwrap();
        assert_eq!(calls.len(), 4);
        assert_eq!(result["genres"][0]["artists"], 2);
        assert!(discover_with("default", |_, _| panic!("Unexpected request")).is_err());
        assert!(discover_with("focus", |_, _| Err("unavailable".into())).is_err());
    }

    #[test]
    fn recommendations_replace_heuristics_but_preserve_explicit_edits() {
        let mut tuner =
            crate::spotify::flow::provider_tuner("focus", &json!([]), &json!([])).unwrap();
        let data = json!({"genres":[{"genre":"Jazz","artists":3}]});
        apply(&mut tuner, &data);
        assert_eq!(tuner["moodSource"], "lastfm-artist-tags");
        let enabled: Vec<_> = tuner["genres"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|genre| genre["isEnabled"] == true)
            .collect();
        assert_eq!(enabled.len(), 1);
        assert_eq!(enabled[0]["id"], "Jazz");
        let before = tuner.clone();
        apply(&mut tuner, &json!({"genres":[]}));
        assert_eq!(tuner, before);
        apply(
            &mut tuner,
            &json!({"genres":[{"genre":"Not a known genre"}]}),
        );
        assert_eq!(tuner, before);
        for selected in [vec!["Rock".into()], vec![]] {
            let key = crate::spotify::flow::Selection {
                mood: "focus".into(),
                genres: selected,
            }
            .key();
            let mut saved =
                crate::spotify::flow::provider_tuner(&key, &json!([]), &json!([])).unwrap();
            let before = saved["genres"].clone();
            apply(&mut saved, &data);
            assert!(saved["genres"]
                .as_array()
                .unwrap()
                .iter()
                .zip(before.as_array().unwrap())
                .all(|(after, before)| after["isEnabled"] == before["isEnabled"]));
        }
    }

    #[test]
    #[ignore = "requires a Last.fm key and network; optional DEEZNATS_LASTFM_KEY_FILE configures DPAPI storage"]
    fn live_lastfm_mood_discovery() {
        if let Ok(path) = std::env::var("DEEZNATS_LASTFM_KEY_FILE") {
            configure(&path).unwrap();
        }
        assert!(api_key().is_some(), "Configure Last.fm first");
        let result = recommendations("focus").expect("Expected live Last.fm mood evidence");
        let genres = result["genres"].as_array().unwrap();
        assert!(!genres.is_empty());
        assert!(genres
            .iter()
            .all(|genre| genre["artists"].as_u64().unwrap() >= 2));
        println!("Last.fm focus recommendations: {result}");
        assert!(genres
            .iter()
            .all(|genre| !matches!(genre["genre"].as_str(), Some("focus" | "sleep"))));
        let started = Instant::now();
        assert_eq!(recommendations("focus").unwrap(), result);
        assert!(started.elapsed() < Duration::from_secs(1));
        let tuner = tuner("focus", &json!([]), || {
            panic!("Last.fm should bypass playlist discovery")
        })
        .unwrap();
        assert_eq!(tuner["moodSource"], "lastfm-artist-tags");
    }
}
