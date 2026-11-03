//! Spotify's API shapes adapted to the shared native page model.
use super::*;
pub(super) fn style_mix(selection: &str) -> Result<Value, String> {
    invoke("SPStyleMixJSON", &[json!(selection)])
}

fn invoke(name: &str, args: &[Value]) -> Result<Value, String> {
    if name == "SPStyleMixJSON" {
        return crate::spotify::catalog_invoke(name, args);
    }
    if name == "StyleMixGenres" {
        if let Ok(value) = crate::spotify::catalog_invoke(name, args) {
            return Ok(value);
        }
    }
    if name == "StyleMixGenres" {
        let region = args
            .first()
            .and_then(Value::as_u64)
            .ok_or("Choose a valid language")? as usize;
        crate::style_mix::Selection::new(region, 0)?;
        let playlists = library_playlists()?;
        let mut genres = Vec::new();
        for genre in 0..crate::style_mix::GENRES.len() {
            let selection = crate::style_mix::Selection::new(region, genre)?;
            if playlists
                .iter()
                .any(|playlist| matches_selection(&playlist.title, selection))
            {
                genres.push(json!({"id":genre,"title":crate::style_mix::GENRES[genre].0}));
            }
        }
        return Ok(json!({"region":region,"genres":genres,"partial":false}));
    }
    crate::spotify::catalog_invoke(name, args)
}
fn library_playlists() -> Result<Vec<Entry>, String> {
    let mut value = crate::spotify::catalog_invoke("DZPlaylistsJSON", &[])?;
    let mut result = rows(&value, "playlist");
    let mut seen = HashSet::new();
    for _ in 0..3 {
        let cursor = string(&value, &["next"]);
        if cursor.is_empty() || !seen.insert(cursor.clone()) {
            break;
        }
        value = crate::spotify::catalog_invoke("SPCatalogNextJSON", &[json!(cursor)])?;
        result.extend(rows(&value, "playlist"));
    }
    Ok(result)
}
fn words(text: &str) -> String {
    format!(
        " {} ",
        text.to_lowercase()
            .chars()
            .map(|c| match c {
                'ü' | 'ú' | 'ù' => 'u',
                'ö' | 'ó' | 'ô' => 'o',
                'ç' => 'c',
                'ñ' => 'n',
                'ä' | 'å' | 'ã' | 'á' | 'à' | 'â' => 'a',
                'ş' => 's',
                'ğ' => 'g',
                'ı' | 'ï' => 'i',
                'é' | 'è' | 'ê' => 'e',
                c if c.is_alphanumeric() => c,
                _ => ' ',
            })
            .collect::<String>()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
    )
}
fn matches_selection(title: &str, selection: crate::style_mix::Selection) -> bool {
    let title = words(title);
    let contains = |aliases: &[&str]| aliases.iter().any(|alias| title.contains(&words(alias)));
    contains(crate::style_mix::REGIONS[selection.region].1)
        && contains(crate::style_mix::GENRES[selection.genre].1)
}
fn matching_mix(
    playlists: &[Entry],
    selection: crate::style_mix::Selection,
) -> Result<Vec<Entry>, String> {
    let mut result = Vec::new();
    let mut seen = HashSet::new();
    let mut error = None;
    for playlist in playlists
        .iter()
        .filter(|playlist| matches_selection(&playlist.title, selection))
        .take(3)
    {
        match crate::spotify::catalog_invoke("DZPlaylistTracksJSON", &[json!(playlist.id)]) {
            Ok(value) => {
                for mut track in tracks(&value) {
                    if track.raw["is_playable"] == false || !seen.insert(track.id.clone()) {
                        continue;
                    }
                    track.raw[crate::style_mix::MARKER] = json!(selection.key());
                    result.push(track);
                }
            }
            Err(failure) => error = Some(failure),
        }
    }
    crate::spotify::style_mix::retain_language(&mut result, selection, |track| &track.raw);
    if result.is_empty() {
        if let Some(error) = error {
            return Err(error);
        }
    }
    Ok(result)
}
pub(super) fn is_item(value: &Value) -> bool {
    ["uri", "id"].iter().any(|key| {
        value[*key]
            .as_str()
            .is_some_and(|s| s.starts_with("spotify:"))
    }) || value["provider"] == "spotify"
}
fn identity(value: &Value, kind: &str) -> String {
    let uri = string(value, &["uri", "id"]);
    if uri.starts_with("spotify:") {
        return uri;
    }
    let id = string(value, &["id"]);
    if id.is_empty() {
        String::new()
    } else {
        format!("spotify:{kind}:{id}")
    }
}
fn artwork(value: &Value) -> String {
    super::largest_artwork(
        value["images"]
            .as_array()
            .into_iter()
            .flatten()
            .chain(value["album"]["images"].as_array().into_iter().flatten()),
    )
    .map(str::to_owned)
    .unwrap_or_else(|| picture(value))
}
pub(super) fn entry(value: &Value, fallback: &str) -> Option<Entry> {
    if value.is_null() || value["is_local"] == true {
        return None;
    }
    let uri = string(value, &["uri", "id"]);
    let inferred = uri.split(':').nth(1).unwrap_or(fallback);
    let typ = string(value, &["type", "kind"]);
    let typ = if typ.is_empty() { inferred } else { &typ };
    let kind = match typ {
        "show" => "podcast",
        "category" => "channel",
        other => other,
    };
    if !matches!(
        kind,
        "track"
            | "episode"
            | "album"
            | "artist"
            | "playlist"
            | "podcast"
            | "channel"
            | "mix"
            | "flow"
    ) {
        return None;
    }
    let id = identity(value, if typ == "podcast" { "show" } else { typ });
    let title = string(value, &["name", "title"]);
    if id.is_empty() || title.is_empty() {
        return None;
    }
    let mut raw = compact_entry_metadata(value);
    raw["provider"] = json!("spotify");
    raw["id"] = json!(id);
    raw["kind"] = json!(kind);
    // Nested credit IDs must never enter a Deezer detail page or queue.
    if let Some(artists) = raw["artists"].as_array_mut() {
        for artist in artists {
            artist["id"] = json!(identity(artist, "artist"));
            artist["provider"] = json!("spotify");
        }
    }
    if raw["album"].is_object() {
        raw["album"]["id"] = json!(identity(&value["album"], "album"));
        raw["album"]["provider"] = json!("spotify");
    }
    if let Some(count) = value.pointer("/followers/total").and_then(Value::as_u64) {
        raw["followers"] = json!(count);
    }
    if let Some(count) = value["total_episodes"].as_u64() {
        raw["episodeCount"] = json!(count);
    }
    if let Some(count) = value["total_tracks"]
        .as_u64()
        .or_else(|| value.pointer("/tracks/total").and_then(Value::as_u64))
    {
        raw["nb_tracks"] = json!(count);
    }
    if let Some(date) = value["release_date"].as_str() {
        raw["releaseDate"] = json!(date);
    }
    let artists = raw["artists"].as_array();
    let artist_id = artists
        .and_then(|artists| artists.first())
        .map(|a| string(a, &["id"]))
        .unwrap_or_default();
    let artist_line = artists
        .map(|artists| {
            artists
                .iter()
                .map(|a| string(a, &["name", "title"]))
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>()
                .join(", ")
        })
        .unwrap_or_default();
    let subtitle = match kind {
        "playlist" => string(&value["owner"], &["display_name", "name", "id"]),
        "podcast" => string(value, &["publisher", "authorName"]),
        "episode" => string(&value["show"], &["name"]),
        "artist" => raw["followers"]
            .as_u64()
            .map(|n| format!("{} followers", format_count(n)))
            .unwrap_or_default(),
        _ => artist_line,
    };
    let subtitle = if subtitle.is_empty() {
        string(value, &["artistLine", "subtitle"])
    } else {
        subtitle
    };
    Some(Entry {
        id,
        title,
        subtitle,
        kind: kind.into(),
        artwork: artwork(value),
        duration: value["duration_ms"]
            .as_u64()
            .or_else(|| value["durationMs"].as_u64())
            .unwrap_or(0),
        album: string(&value["album"], &["name", "title"]),
        artist_id,
        album_id: if value["album"].is_object() {
            identity(&value["album"], "album")
        } else {
            string(value, &["albumId"])
        },
        raw,
    })
}

fn spotify_value(value: &Value) -> Value {
    let mut value = value.clone();
    // API objects omit URI in a few browse/category responses.
    if value.is_object()
        && !string(&value, &["id"]).is_empty()
        && !string(&value, &["name", "title"]).is_empty()
    {
        value["provider"] = json!("spotify");
    }
    match &mut value {
        Value::Array(items) => {
            for item in items {
                *item = spotify_value(item);
            }
        }
        Value::Object(object) => {
            for child in object.values_mut() {
                if child.is_object() || child.is_array() {
                    *child = spotify_value(child);
                }
            }
        }
        _ => {}
    }
    value
}

fn track_info(
    track: &Entry,
    invoke: &impl Fn(&str, &[Value]) -> Result<Value, String>,
) -> (Value, Value) {
    let meta = invoke("DZTrackMetadataJSON", &[json!(track.id)])
        .map(|v| spotify_value(&v))
        .unwrap_or_else(|error| {
            crate::spotify_diagnostics::record(
                "catalog.track_info",
                crate::spotify_diagnostics::error_kind(&error),
            );
            track.raw.clone()
        });
    let artist_id = entry(&meta, "track")
        .map(|full| full.artist_id)
        .filter(|id| !id.is_empty())
        .unwrap_or_else(|| track.artist_id.clone());
    let artist = if artist_id.is_empty() {
        Value::Null
    } else {
        invoke("DZArtistProfileJSON", &[json!(artist_id)])
            .map(|v| spotify_value(v.get("artist").unwrap_or(&v)))
            .unwrap_or_else(|error| {
                crate::spotify_diagnostics::record(
                    "catalog.artist_info",
                    crate::spotify_diagnostics::error_kind(&error),
                );
                Value::Null
            })
    };
    (meta, artist)
}
fn rows(value: &Value, kind: &str) -> Vec<Entry> {
    let mut result = entries(&spotify_value(value), kind);
    if let Some(items) = value.get("items").unwrap_or(value).as_array() {
        for wrapper in items {
            let item = wrapper
                .get("track")
                .or_else(|| wrapper.get("item"))
                .or_else(|| wrapper.get("episode"));
            if let Some(item) = item {
                let uri = identity(item, kind);
                if let Some(row) = result.iter_mut().find(|row| row.id == uri) {
                    for key in ["played_at", "added_at"] {
                        if let Some(date) = wrapper.get(key) {
                            row.raw[key] = date.clone();
                        }
                    }
                }
            }
        }
    }
    result
}
fn tracks(value: &Value) -> Vec<Entry> {
    rows(value, "track")
        .into_iter()
        .filter(Entry::playable)
        .collect()
}
fn sections(value: &Value) -> Vec<Section> {
    value["sections"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|section| Section {
            title: string(section, &["title", "name"]),
            entries: rows(
                section
                    .get("items")
                    .or_else(|| section.get("entries"))
                    .unwrap_or(&Value::Null),
                "playlist",
            ),
            error: string(section, &["error"]),
        })
        .collect()
}
fn home_sections(value: &Value) -> Vec<Section> {
    let mut personal = sections(value);
    let mut featured = [
        Section {
            title: "Radios for you".into(),
            ..Default::default()
        },
        Section {
            title: "This Is…".into(),
            ..Default::default()
        },
        Section {
            title: "Recommended artists".into(),
            ..Default::default()
        },
        Section {
            title: "Made for you".into(),
            ..Default::default()
        },
        Section {
            title: "Recommended playlists".into(),
            ..Default::default()
        },
    ];
    let mut seen = HashSet::new();
    for section in &mut personal {
        section.entries.retain(|item| {
            let name = item.title.to_lowercase();
            let target = if item.kind == "artist" {
                Some(2)
            } else if item.kind == "playlist" && name.starts_with("daily mix ") {
                Some(3)
            } else if item.kind == "playlist" && name.starts_with("this is ") {
                Some(1)
            } else if item.kind == "playlist"
                && (name.contains("radio")
                    || section.title.to_lowercase().contains("radio")
                    || section.title.to_lowercase().contains("station"))
            {
                Some(0)
            } else if item.kind == "playlist"
                && (name.contains("mix") || section.title.to_lowercase().contains("made for"))
            {
                Some(3)
            } else if item.kind == "playlist" {
                Some(4)
            } else {
                None
            };
            if let Some(index) = target {
                if seen.insert((item.kind.clone(), item.id.clone())) {
                    featured[index].entries.push(item.clone());
                }
                false
            } else {
                true
            }
        });
    }
    featured
        .into_iter()
        .chain(personal)
        .filter(|section| !section.entries.is_empty() || !section.error.is_empty())
        .collect()
}
fn discovery_sections(value: &Value) -> Vec<Section> {
    let mut result: Vec<Section> = Vec::new();
    for mut section in sections(value) {
        let mut channels = Vec::new();
        section.entries.retain(|item| {
            if item.kind == "channel" {
                channels.push(item.clone());
                false
            } else {
                true
            }
        });
        for item in channels {
            let title = section.title.to_lowercase();
            let id = item.id.rsplit(':').next().unwrap_or("");
            // Spotify's browse directory mixes genres, moods and podcasts in
            // one localized shelf. Stable page IDs keep our grouping language independent.
            let inherited = item.raw["directory_group"].as_str().unwrap_or("");
            let title = if matches!(inherited, "Genres" | "Categories" | "Podcasts by category") {
                inherited
            } else if title.contains("podcast")
                || matches!(
                    id,
                    "0JQ5DArNBzkmxXHCqFLx2J"
                        | "0JQ5IMCbQBLl5gxKLgufp8"
                        | "0JQ5IMCbQBLjfX9OdDrA5X"
                        | "0JQ5IMCbQBLyUJhSxhaPc6"
                )
            {
                "Podcasts by category"
            } else if item.id.starts_with("spotify:genre:")
                || matches!(
                    id,
                    "0JQ5DAqbMKFCsfq2JeFgB0"
                        | "0JQ5DAqbMKFEC4WFtoNRpw"
                        | "0JQ5DAqbMKFHOzuVTgTizF"
                        | "0JQ5DAqbMKFDXXwE9BDJAr"
                        | "0JQ5DAqbMKFQ00XGBls6ym"
                        | "0JQ5DAqbMKFCWjUTdzaG0e"
                        | "0JQ5DAqbMKFKLfwjuJMoNC"
                        | "0JQ5DAqbMKFEZPnFQSFB1T"
                        | "0JQ5DAqbMKFGvOw3O4nLAf"
                        | "0JQ5DAqbMKFDkd668ypn6O"
                        | "0JQ5DAqbMKFAJ5xb0fwo9m"
                        | "0JQ5DAqbMKFziKOShCi009"
                        | "0JQ5DAqbMKFxXaXKP7zcDp"
                        | "0JQ5DAqbMKFPrEiAOxgac3"
                        | "0JQ5DAqbMKFy78wprEpAjl"
                        | "0JQ5DAqbMKFIpEuaCnimBj"
                        | "0JQ5DAqbMKFAjfauKLOZiv"
                        | "0JQ5DAqbMKFLjmiZRss79w"
                        | "0JQ5DAqbMKFQiK2EHwyjcU"
                        | "0JQ5DAqbMKFFtlLYUHv8bT"
                        | "0JQ5DAqbMKFObNLOHydSW8"
                        | "0JQ5DAqbMKFNQ0fGp4byGU"
                        | "0JQ5DAqbMKFFsW9N8maB6z"
                        | "0JQ5DAqbMKFHCxg5H5PtqW"
                        | "0JQ5DAqbMKFQFQN0rnK48G"
                        | "0JQ5DAqbMKFJKoGyUMo2hE"
                        | "0JQ5DAqbMKFQkIFo7vWoOd"
                        | "0JQ5DAqbMKFARvdmw6EIef"
                )
            {
                "Genres"
            } else {
                "Categories"
            };
            if let Some(existing) = result.iter_mut().find(|s| s.title == title) {
                if !existing.entries.iter().any(|entry| entry.id == item.id) {
                    existing.entries.push(item);
                }
            } else {
                result.push(Section {
                    title: title.into(),
                    entries: vec![item],
                    ..Default::default()
                });
            }
        }
        if !section.entries.is_empty() || !section.error.is_empty() {
            result.push(section);
        }
    }
    result
}
fn curate_shelves(sections: &mut Vec<Section>) {
    let dedicated_mixes: HashSet<_> = sections
        .iter()
        .filter(|s| s.title == "Made for you")
        .flat_map(|s| &s.entries)
        .filter(|item| {
            item.kind == "playlist" && item.title.to_lowercase().starts_with("daily mix ")
        })
        .map(|item| (item.kind.clone(), item.id.clone()))
        .collect();
    let mut seen = HashSet::new();
    let mut artists = std::collections::HashMap::<String, usize>::new();
    for section in sections.iter_mut() {
        // History is useful even when the same item is saved or recommended.
        let history = section.title == "Recently played";
        let mut local = HashSet::new();
        let mut themes = HashSet::new();
        section.entries.retain(|item| {
            let key = (item.kind.clone(), item.id.clone());
            if !history && section.title != "Made for you" && dedicated_mixes.contains(&key) {
                return false;
            }
            if !item.id.is_empty()
                && (!local.insert(key.clone()) || (!history && seen.contains(&key)))
            {
                return false;
            }
            if !history && section.title != "Your playlists" {
                let name = item.title.to_lowercase();
                let artist = if item.kind == "playlist" {
                    name.strip_prefix("this is ")
                        .or_else(|| name.strip_suffix(" radio"))
                } else {
                    None
                };
                if let Some(artist) = artist {
                    let artist = artist.trim().to_owned();
                    let count = artists.entry(artist.clone()).or_default();
                    if !themes.insert(artist) || *count >= 2 {
                        return false;
                    }
                    *count += 1;
                }
            }
            if !history && !item.id.is_empty() {
                seen.insert(key);
            }
            true
        });
    }
    sections.retain(|section| !section.entries.is_empty() || !section.error.is_empty());
}
fn publish_curated(
    page: &Page,
    publish: &mut dyn FnMut(&Page) -> Result<(), String>,
) -> Result<(), String> {
    let mut visible = page.clone();
    curate_shelves(&mut visible.sections);
    publish(&visible)
}
fn remember_shelf(page: &mut Page, title: &str, value: &Value, kind: &str) {
    page.raw.push((
        format!("spotify:shelf:{title}"),
        json!({"kind":kind,"sections":value["sections"]}),
    ));
}
fn directory_covers(
    sections: &mut [Section],
    parent_cover: &str,
    fetch: &(impl Fn(&str, &[Value]) -> Result<Value, String> + Sync),
) {
    let fallback = sections
        .iter()
        .flat_map(|section| &section.entries)
        .find(|entry| {
            matches!(entry.kind.as_str(), "playlist" | "album") && !entry.artwork.is_empty()
        })
        .map(|entry| entry.artwork.clone())
        .unwrap_or_else(|| parent_cover.to_owned());
    for entry in sections
        .iter_mut()
        .flat_map(|section| &mut section.entries)
        .filter(|entry| {
            entry.kind == "channel"
                && (entry.artwork.is_empty() || entry.raw["directory_artwork_fallback"] == true)
        })
    {
        // Use already downloaded genre metadata; displaying a tile must not
        // trigger another catalogue download just to find its cover.
        let cached = fetch("SPCatalogDirectoryCachedJSON", &[json!(entry.id)]).ok();
        let own_cover = cached
            .as_ref()
            .into_iter()
            .flat_map(|value| sections_from_cache(value))
            .find(|entry| {
                matches!(entry.kind.as_str(), "playlist" | "album") && !entry.artwork.is_empty()
            })
            .map(|entry| entry.artwork);
        if let Some(cover) = own_cover {
            entry.artwork = cover;
            entry.raw["directory_artwork_fallback"] = json!(false);
        } else if entry.artwork.is_empty() && !fallback.is_empty() {
            entry.artwork = fallback.clone();
            entry.raw["directory_artwork_fallback"] = json!(true);
        }
    }
}
fn sections_from_cache(value: &Value) -> impl Iterator<Item = Entry> {
    sections(value)
        .into_iter()
        .flat_map(|section| section.entries)
}
fn is_genre_directory_category(title: &str) -> bool {
    let name = title.to_lowercase();
    ["mood", "workout", "new releases", "discover", "classics"]
        .iter()
        .any(|word| name.contains(word))
}

fn load_shelf_more(
    page: &mut Page,
    title: &str,
    target: usize,
    publish: &mut dyn FnMut(&Page) -> Result<(), String>,
    fetch: &(impl Fn(&str, &[Value]) -> Result<Value, String> + Sync),
) -> Result<(), String> {
    let key = format!("spotify:shelf:{title}");
    let Some(state) = page
        .raw
        .iter()
        .find(|(k, _)| k == &key)
        .map(|(_, v)| v.clone())
    else {
        return Ok(());
    };
    if state["directory"] == true {
        expand_discovery_directories(page, publish, fetch, 3, Some((title, target)))?;
        let pending_key = format!("spotify:directory-sections:{title}");
        let mut pending = page
            .raw
            .iter()
            .find(|(k, _)| k == &pending_key)
            .map(|(_, v)| v.clone())
            .unwrap_or(json!([]));
        for _ in 0..3 {
            let loaded = page
                .sections
                .iter()
                .find(|s| s.title == title)
                .map_or(0, |s| s.entries.len());
            if loaded >= target {
                break;
            }
            let Some(index) = pending.as_array().and_then(|ss| {
                ss.iter().position(|s| {
                    s["offset"].as_u64().unwrap_or(0) < s["total"].as_u64().unwrap_or(0)
                })
            }) else {
                break;
            };
            let offset = pending[index]["offset"].as_u64().unwrap_or(0);
            let response = fetch(
                "SPCatalogSectionJSON",
                &[
                    pending[index]["uri"].clone(),
                    json!(offset),
                    json!((target - loaded).clamp(1, 100)),
                ],
            )?;
            let new_offset = response["offset"].as_u64().unwrap_or(offset);
            pending[index]["offset"] = json!(new_offset);
            if new_offset <= offset {
                pending[index]["total"] = json!(offset);
            }
            let mut known: HashSet<_> = page
                .sections
                .iter()
                .flat_map(|s| &s.entries)
                .map(|e| e.id.clone())
                .collect();
            for mut child in rows(&response["items"], "channel")
                .into_iter()
                .filter(|e| e.kind == "channel")
            {
                if title == "Genres" && is_genre_directory_category(&child.title) {
                    continue;
                }
                if !known.insert(child.id.clone()) {
                    continue;
                }
                child.raw["directory_group"] = json!(title);
                if child.artwork.is_empty() {
                    child.artwork = pending[index]["cover"].as_str().unwrap_or("").into();
                    child.raw["directory_artwork_fallback"] = json!(true);
                }
                if let Some(section) = page.sections.iter_mut().find(|s| s.title == title) {
                    section.entries.push(child);
                }
            }
        }
        let remaining = pending
            .as_array()
            .into_iter()
            .flatten()
            .any(|s| s["offset"].as_u64().unwrap_or(0) < s["total"].as_u64().unwrap_or(0));
        if let Some((_, v)) = page.raw.iter_mut().find(|(k, _)| k == &pending_key) {
            *v = pending;
        }
        if remaining {
            if let Some((_, v)) = page.raw.iter_mut().find(|(k, _)| k == &key) {
                v["complete"] = json!(false);
            }
        }
        return Ok(());
    }
    let mut state = state;
    let kind = state["kind"].as_str().unwrap_or("").to_owned();
    for _ in 0..3 {
        if page
            .sections
            .iter()
            .find(|s| s.title == title)
            .map_or(0, |s| s.entries.len())
            >= target
        {
            break;
        }
        let Some(index) = state["sections"].as_array().and_then(|ss| {
            ss.iter()
                .position(|s| s["total"].as_u64().unwrap_or(0) > s["offset"].as_u64().unwrap_or(0))
        }) else {
            break;
        };
        let section = &state["sections"][index];
        let offset = section["offset"].as_u64().unwrap_or(0);
        let value = fetch(
            "SPCatalogSectionJSON",
            &[
                section["uri"].clone(),
                json!(offset),
                json!(target
                    .saturating_sub(
                        page.sections
                            .iter()
                            .find(|s| s.title == title)
                            .map_or(0, |s| s.entries.len())
                    )
                    .clamp(1, 100)),
            ],
        )?;
        let new_offset = value["offset"].as_u64().unwrap_or(offset);
        state["sections"][index]["offset"] = json!(new_offset);
        state["sections"][index]["total"] = if new_offset <= offset {
            json!(offset)
        } else {
            value["total"].clone()
        };
        let more = rows(&value["items"], "playlist")
            .into_iter()
            .filter(|item| {
                kind.is_empty() || item.kind == kind || (kind == "album" && item.kind == "playlist")
            });
        if let Some(section) = page.sections.iter_mut().find(|s| s.title == title) {
            let mut ids: HashSet<_> = section
                .entries
                .iter()
                .map(|e| (e.kind.clone(), e.id.clone()))
                .collect();
            section
                .entries
                .extend(more.filter(|e| ids.insert((e.kind.clone(), e.id.clone()))));
        }
        publish(page)?;
    }
    if let Some((_, v)) = page.raw.iter_mut().find(|(k, _)| k == &key) {
        *v = state;
    }
    Ok(())
}
fn expand_discovery_directories(
    page: &mut Page,
    publish: &mut dyn FnMut(&Page) -> Result<(), String>,
    fetch: &(impl Fn(&str, &[Value]) -> Result<Value, String> + Sync),
    budget: usize,
    target: Option<(&str, usize)>,
) -> Result<(), String> {
    let mut queue = std::collections::VecDeque::new();
    for group in ["Genres", "Categories", "Podcasts by category"] {
        if target.is_some_and(|(title, _)| title != group) {
            continue;
        }
        for item in page
            .sections
            .iter()
            .filter(|s| s.title == group)
            .flat_map(|s| &s.entries)
        {
            if item.kind == "channel" {
                queue.push_back((item.clone(), group.to_owned(), 0usize));
            }
        }
    }
    let mut visited = HashSet::new();
    let mut known: HashSet<_> = page
        .sections
        .iter()
        .flat_map(|s| &s.entries)
        .map(|item| item.id.clone())
        .collect();
    let mut downloads = 0;
    let mut skipped = false;
    // Grow the requested directory on demand, reusing cached pages and keeping
    // new downloads within the click's budget on three worker threads.
    while !queue.is_empty() && visited.len() < 256 {
        if target.is_some_and(|(title, count)| {
            page.sections
                .iter()
                .find(|s| s.title == title)
                .is_some_and(|s| s.entries.len() >= count)
        }) {
            break;
        }
        publish_curated(page, publish)?;
        let mut jobs = Vec::new();
        while jobs.len() < 3 {
            let Some((item, group, depth)) = queue.pop_front() else {
                break;
            };
            if !visited.insert(item.id.clone()) {
                continue;
            }
            let cached = fetch("SPCatalogDirectoryCachedJSON", &[json!(item.id)])
                .ok()
                .filter(|v| !v.is_null());
            if cached.is_none() {
                if downloads >= budget {
                    skipped = true;
                    continue;
                }
                downloads += 1;
            }
            jobs.push((item, group, depth, cached));
        }
        if jobs.is_empty() {
            break;
        }
        parallel::load(
            jobs.len(),
            |index| {
                let (item, _, _, cached) = &jobs[index];
                cached
                    .clone()
                    .map(Ok)
                    .unwrap_or_else(|| fetch("SPCatalogChannelJSON", &[json!(item.id)]))
            },
            |index, result| {
                let (parent, group, depth, _) = &jobs[index];
                if let Ok(value) = result {
                    for source in
                        value["sections"]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .filter(|source| {
                                source["total"].as_u64().unwrap_or(0)
                                    > source["offset"].as_u64().unwrap_or(0)
                                    && rows(&source["items"], "playlist")
                                        .iter()
                                        .any(|item| item.kind == "channel")
                            })
                    {
                        let key = format!("spotify:directory-sections:{group}");
                        if !page.raw.iter().any(|(k, _)| k == &key) {
                            page.raw.push((key.clone(), json!([])));
                        }
                        let pending = page
                            .raw
                            .iter_mut()
                            .find(|(k, _)| k == &key)
                            .unwrap()
                            .1
                            .as_array_mut()
                            .unwrap();
                        if !pending.iter().any(|s| s["uri"] == source["uri"]) {
                            pending.push(json!({"uri":source["uri"],"offset":source["offset"],"total":source["total"],"cover":parent.artwork}));
                        }
                    }
                    let shelves = sections(&value);
                    let cover = shelves
                        .iter()
                        .flat_map(|s| &s.entries)
                        .find(|item| !item.artwork.is_empty())
                        .map(|item| item.artwork.clone());
                    if let Some(cover) = cover {
                        for entry in page
                            .sections
                            .iter_mut()
                            .flat_map(|s| &mut s.entries)
                            .filter(|e| e.id == parent.id)
                        {
                            if entry.artwork.is_empty()
                                || entry.raw["directory_artwork_fallback"] == true
                            {
                                entry.artwork = cover.clone();
                                entry.raw["directory_artwork_fallback"] = json!(false);
                            }
                        }
                    }
                    if *depth < 2 {
                        for mut child in shelves
                            .into_iter()
                            .flat_map(|s| s.entries)
                            .filter(|item| item.kind == "channel")
                        {
                            let child_group =
                                if group == "Genres" && is_genre_directory_category(&child.title) {
                                    "Categories"
                                } else {
                                    group.as_str()
                                };
                            if target.is_some_and(|(title, _)| title != child_group)
                                || !known.insert(child.id.clone())
                            {
                                continue;
                            }
                            child.raw["directory_group"] = json!(child_group);
                            if child.artwork.is_empty() {
                                child.artwork = parent.artwork.clone();
                                child.raw["directory_artwork_fallback"] = json!(true);
                            }
                            queue.push_front((child.clone(), child_group.to_owned(), depth + 1));
                            if let Some(section) =
                                page.sections.iter_mut().find(|s| s.title == child_group)
                            {
                                section.entries.push(child);
                            } else {
                                page.sections.push(Section {
                                    title: child_group.into(),
                                    entries: vec![child],
                                    ..Default::default()
                                });
                            }
                        }
                    }
                }
                publish_curated(page, publish)
            },
        )?;
    }
    if let Some((title, _)) = target {
        if let Some((_, state)) = page
            .raw
            .iter_mut()
            .find(|(k, _)| k == &format!("spotify:shelf:{title}"))
        {
            state["complete"] = json!(!skipped && queue.is_empty());
        }
    }
    Ok(())
}
fn metadata(page: &mut Page, value: &Value) {
    let source = page
        .entity
        .as_ref()
        .map(|e| e.kind.as_str())
        .unwrap_or("track");
    if let Some(item) = entry(&spotify_value(value), source) {
        page.entity = Some(item);
    }
    apply_entity_metadata(page);
}

fn apply_entity_metadata(page: &mut Page) {
    if let Some(entity) = &page.entity {
        for row in &mut page.rows {
            if entity.kind == "podcast" && row.kind == "episode" {
                if row.artwork.is_empty() {
                    row.artwork = entity.artwork.clone();
                }
                if row.subtitle.is_empty() {
                    row.subtitle = entity.title.clone();
                }
                if row.raw["show"].is_null() {
                    row.raw["show"] = entity.raw.clone();
                }
            }
            if entity.kind == "album" && row.kind == "track" {
                if row.artwork.is_empty() {
                    row.artwork = entity.artwork.clone();
                }
                if row.album.is_empty() {
                    row.album = entity.title.clone();
                }
                if row.album_id.is_empty() {
                    row.album_id = entity.id.clone();
                }
                if row.artist_id.is_empty() {
                    row.artist_id = entity.artist_id.clone();
                }
                if row.subtitle.is_empty() {
                    row.subtitle = entity.subtitle.clone();
                }
            }
        }
    }
}
fn append(page: &mut Page, cursor: &str, value: Value) {
    if let Some(index) = page
        .raw
        .iter()
        .find(|(key, _)| key == "spotify:pagination-section")
        .and_then(|(_, value)| value.as_u64())
        .map(|value| value as usize)
    {
        if let Some(section) = page.sections.get_mut(index) {
            let kind = section
                .entries
                .first()
                .map(|entry| entry.kind.as_str())
                .unwrap_or("track");
            let mut seen: HashSet<_> = section
                .entries
                .iter()
                .map(|entry| (entry.kind.clone(), entry.id.clone()))
                .collect();
            section.entries.extend(
                rows(&value, kind)
                    .into_iter()
                    .filter(|entry| seen.insert((entry.kind.clone(), entry.id.clone()))),
            );
        }
        let next = string(&value, &["next"]);
        page.next = if next == cursor
            || page
                .raw
                .iter()
                .any(|(key, _)| key == &format!("next:{next}"))
        {
            String::new()
        } else {
            next
        };
        page.raw.push((format!("next:{cursor}"), Value::Null));
        page.error.clear();
        return;
    }
    let start = page.rows.len();
    let kind = page
        .rows
        .first()
        .map(|row| row.kind.as_str())
        .unwrap_or("track");
    let more = rows(&value, kind);
    let mut seen: HashSet<_> = page
        .rows
        .iter()
        .map(|row| (row.kind.clone(), row.id.clone()))
        .collect();
    page.rows.extend(
        more.into_iter()
            .filter(|row| seen.insert((row.kind.clone(), row.id.clone()))),
    );
    let next = string(&value, &["next"]);
    page.next = if next == cursor
        || page
            .raw
            .iter()
            .any(|(key, _)| key == &format!("next:{next}"))
    {
        String::new()
    } else {
        next
    };
    page.error.clear();
    page.raw.push((format!("next:{cursor}"), Value::Null));
    page.raw.push((
        "cache:chunk".into(),
        json!({"cursor":cursor,"start":start,"end":page.rows.len()}),
    ));
    apply_entity_metadata(page);
}

pub(super) fn load(
    _engine: &Engine,
    view: View,
    entity: Option<Entry>,
    query: String,
    publish: &mut dyn FnMut(&Page) -> Result<(), String>,
) -> Result<Page, String> {
    load_page(view, entity, query, publish)
}
fn recent_history(page: &mut Page, value: &Value) {
    let entries = rows(value, "track");
    // Web history contains listening contexts; OAuth history contains tracks.
    if entries
        .iter()
        .any(|entry| !matches!(entry.kind.as_str(), "track" | "episode"))
    {
        page.sections.push(Section {
            title: page.title.clone(),
            entries,
            ..Default::default()
        });
    } else {
        page.rows = entries;
    }
}

fn load_page(
    view: View,
    entity: Option<Entry>,
    query: String,
    publish: &mut dyn FnMut(&Page) -> Result<(), String>,
) -> Result<Page, String> {
    let mut page = Page {
        title: entity
            .as_ref()
            .map(|e| e.title.clone())
            .unwrap_or_else(|| view.title().into()),
        entity: entity.clone(),
        loading: true,
        ..Default::default()
    };
    publish(&page)?;
    if view == View::StyleMix && query.is_empty() {
        page.loading = false;
        return Ok(page);
    }
    if view == View::Home {
        page.title = "For you".into();
        page.raw.push(("provider".into(), json!("spotify")));
        let shelves = [
            ("Your playlists", 2, "playlist"),
            ("Recently played", 0, "track"),
            ("Liked songs", 1, "track"),
            ("Saved albums", 3, "album"),
            ("Followed artists", 4, "artist"),
            ("Saved podcasts", 7, "podcast"),
            ("Saved episodes", 9, "episode"),
        ];
        page.title = "For you".into();
        page.subtitle = "Your Spotify library and listening history".into();
        page.sections = shelves
            .iter()
            .map(|(title, _, _)| Section {
                title: (*title).into(),
                ..Default::default()
            })
            .collect();
        let mut personal_count = 0;
        publish(&page)?;
        parallel::load(
            shelves.len() + 1,
            |index| {
                if index == 0 {
                    return Err(invoke("SPCatalogHomeJSON", &[])
                        .map(|value| home_sections(&value))
                        .unwrap_or_default());
                }
                let (title, module, kind) = shelves[index - 1];
                Ok(match invoke("DZHomeModuleJSON", &[json!(module)]) {
                    Ok(value) => Section {
                        title: title.into(),
                        entries: rows(&value, kind),
                        ..Default::default()
                    },
                    Err(error) => Section {
                        title: title.into(),
                        error,
                        ..Default::default()
                    },
                })
            },
            |index, result| {
                let section = match result {
                    Ok(section) => section,
                    Err(personal) => {
                        personal_count = personal.len();
                        page.sections.splice(2..2, personal);
                        return publish_curated(&page, publish);
                    }
                };
                let index = index - 1;
                let section_index = if index < 2 {
                    index
                } else {
                    index + personal_count
                };
                page.sections[section_index] = section;
                page.raw.push((format!("home:{index}"), Value::Null));
                publish_curated(&page, publish)
            },
        )?;
        curate_shelves(&mut page.sections);
    } else if view == View::Explore && query.is_empty() {
        page.raw.push(("discover".into(), json!(true)));
        publish(&page)?;
        let value = invoke("SPCatalogDiscoveryJSON", &[])?;
        page.sections = discovery_sections(&value);
        for title in ["Genres", "Categories", "Podcasts by category"] {
            page.raw.push((
                format!("spotify:shelf:{title}"),
                json!({"directory":true,"complete":false}),
            ));
        }
        publish(&page)?;
        let sources = [
            (
                "spotify:page:0JQ5DAqbMKFSi39LMRT0Cy",
                "Editorial playlists",
                "playlist",
            ),
            (
                "spotify:page:0JQ5DAqbMKFz6FAsUtgAab",
                "New releases",
                "album",
            ),
            ("spotify:page:0JQ5DAudkNjCgYMM0TZXDw", "Charts", "playlist"),
        ];
        let available: HashSet<_> = page
            .sections
            .iter()
            .flat_map(|s| s.entries.iter())
            .map(|entry| entry.id.clone())
            .collect();
        let sources: Vec<_> = sources
            .into_iter()
            .filter(|(uri, _, _)| available.contains(*uri))
            .collect();
        let directories = std::mem::take(&mut page.sections);
        page.sections = sources
            .iter()
            .map(|(_, title, _)| Section {
                title: (*title).into(),
                ..Default::default()
            })
            .collect();
        page.sections.extend(directories);
        parallel::load(
            sources.len(),
            |index| {
                let (uri, title, kind) = sources[index];
                match invoke("SPCatalogChannelJSON", &[json!(uri)]) {
                    Ok(value) => (
                        Section {
                            title: title.into(),
                            entries: sections(&value)
                                .into_iter()
                                .flat_map(|s| s.entries)
                                .filter(|item| {
                                    item.kind == kind
                                        || (kind == "album" && item.kind == "playlist")
                                })
                                .collect(),
                            ..Default::default()
                        },
                        value,
                    ),
                    Err(error) => (
                        Section {
                            title: title.into(),
                            error,
                            ..Default::default()
                        },
                        Value::Null,
                    ),
                }
            },
            |index, (section, value)| {
                remember_shelf(&mut page, &section.title, &value, sources[index].2);
                page.sections[index] = section;
                publish_curated(&page, publish)
            },
        )?;
        curate_shelves(&mut page.sections);
        page.loading = false;
    } else if view == View::Explore {
        let value = invoke("DZSearchJSON", &[json!(query)])?;
        for (key, title, kind) in [
            ("tracks", "Tracks", "track"),
            ("albums", "Albums", "album"),
            ("artists", "Artists", "artist"),
            ("playlists", "Playlists", "playlist"),
            ("shows", "Podcasts", "podcast"),
            ("episodes", "Episodes", "episode"),
        ] {
            page.sections.push(Section {
                title: title.into(),
                entries: rows(&value[key], kind),
                ..Default::default()
            });
        }
        if page.sections.iter().all(|s| s.entries.is_empty()) {
            page.rows = rows(&value, "track");
        }
    } else if matches!(view, View::Album | View::Artist | View::Show) {
        let item = entity.as_ref().ok_or("Select a Spotify item first")?;
        if !item.id.starts_with("spotify:") {
            return Err("Select a Spotify item for this provider".into());
        }
        if item.kind == "channel" {
            let value = invoke("SPCatalogChannelJSON", &[json!(item.id)])?;
            page.sections = sections(&value);
            directory_covers(&mut page.sections, &item.artwork, &invoke);
            for title in page
                .sections
                .iter()
                .map(|s| s.title.clone())
                .collect::<Vec<_>>()
            {
                let matching: Vec<_> = value["sections"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter(|s| s["title"] == title)
                    .cloned()
                    .collect();
                remember_shelf(&mut page, &title, &json!({"sections":matching}), "");
            }
        } else if item.kind == "artist" {
            let value = invoke("DZArtistProfileJSON", &[json!(item.id)])?;
            metadata(&mut page, value.get("artist").unwrap_or(&value));
            let top = value
                .get("top")
                .or_else(|| value.get("topTracks"))
                .cloned()
                .unwrap_or(Value::Null);
            page.sections.push(Section {
                title: "Popular tracks".into(),
                entries: tracks(&top),
                ..Default::default()
            });
            for (index, title, kind) in [
                (0, "Albums", "album"),
                (2, "Singles and EPs", "album"),
                (3, "Appears on", "album"),
                (4, "Similar artists", "artist"),
                (5, "Related playlists", "playlist"),
            ] {
                publish(&page)?;
                let section = match invoke(
                    "DZArtistSectionJSON",
                    &[json!(item.id), json!(index), json!("")],
                ) {
                    Ok(value) => Section {
                        title: title.into(),
                        entries: rows(&value, kind),
                        ..Default::default()
                    },
                    Err(error) => Section {
                        title: title.into(),
                        error,
                        ..Default::default()
                    },
                };
                page.sections.push(section);
            }
        } else {
            let name = match item.kind.as_str() {
                "playlist" => "DZPlaylistTracksJSON",
                "podcast" => "DZPodcastEpisodesJSON",
                "track" => "DZTrackMetadataJSON",
                "episode" => "SPCatalogItemJSON",
                _ => "DZAlbumTracksJSON",
            };
            let args = if item.kind == "episode" {
                vec![json!("episode"), json!(item.id)]
            } else {
                vec![json!(item.id)]
            };
            let value = invoke(name, &args)?;
            page.rows = tracks(&value);
            page.next = string(&value, &["next"]);
            match invoke(
                "SPCatalogItemJSON",
                &[
                    json!(if item.kind == "podcast" {
                        "show"
                    } else {
                        &item.kind
                    }),
                    json!(item.id),
                ],
            ) {
                Ok(value) => metadata(&mut page, &value),
                Err(error) => page.error = error,
            }
            // Album tracks omit their parent album; playlists must retain each track's album.
            let retained = page.entity.clone();
            if let Some(entity) = retained {
                metadata(&mut page, &entity.raw);
            }
        }
    } else {
        let (name, args, kind) = match view {
            View::Favorites => ("DZFavoritesJSON", vec![], "track"),
            View::Albums => ("DZFavoriteAlbumsJSON", vec![], "album"),
            View::Playlists | View::PlaylistPicker => ("DZPlaylistsJSON", vec![], "playlist"),
            View::Recent => ("DZHistoryRecentJSON", vec![json!(200)], "track"),
            View::Podcasts => ("DZPodcastShelfJSON", vec![json!(1)], "podcast"),
            View::Queue => ("DZQueueJSON", vec![], "track"),
            View::Flow if query.is_empty() || query == "default" => ("DZFlowJSON", vec![], "track"),
            View::Flow => ("DZFlowForConfigJSON", vec![json!(query)], "track"),
            View::StyleMix => ("SPStyleMixJSON", vec![json!(query)], "track"),
            _ => return Err("This page is unavailable for Spotify".into()),
        };
        let value = invoke(name, &args)?;
        if matches!(view, View::Albums | View::Playlists | View::PlaylistPicker) {
            page.sections.push(Section {
                title: page.title.clone(),
                entries: rows(&value, kind),
                ..Default::default()
            });
            page.raw
                .push(("spotify:pagination-section".into(), json!(0)));
            if view == View::Playlists {
                if let Ok(saved) = invoke("DZPodcastShelfJSON", &[json!(1)]) {
                    let podcasts = rows(&saved, "podcast");
                    if !podcasts.is_empty() {
                        page.sections.push(Section {
                            title: "Saved podcasts".into(),
                            entries: podcasts,
                            ..Default::default()
                        });
                    }
                }
            }
        } else if view == View::Recent {
            recent_history(&mut page, &value);
        } else if view == View::Podcasts {
            let saved = rows(&value, "podcast");
            let bookmarks = invoke("DZPodcastShelfJSON", &[json!(3)])
                .map(|value| rows(&value, "episode"))
                .unwrap_or_default();
            page = super::podcast_library_page(saved, Vec::new(), bookmarks, Vec::new());
            page.raw
                .push(("spotify:pagination-section".into(), json!(0)));
        } else {
            page.rows = rows(&value, kind);
        }
        if view == View::StyleMix {
            if let Some(selection) = args.first().and_then(Value::as_str) {
                for row in &mut page.rows {
                    row.raw[crate::style_mix::MARKER] = json!(selection);
                }
            }
        }
        page.next = string(&value, &["next"]);
    }
    page.raw.push(("provider".into(), json!("spotify")));
    page.loading = false;
    Ok(page)
}

pub(super) fn discovery_report() -> Result<Value, String> {
    let page = load_page(View::Explore, None, String::new(), &mut |_| Ok(()))?;
    Ok(json!({"sections":page.sections.iter().map(|section| json!({
        "title":section.title,"count":section.entries.len(),"error":section.error,
        "entries":section.entries.iter().filter(|e|e.kind=="channel").map(|entry|
            json!({"name":entry.title,"uri":entry.id,"cover":entry.artwork})).collect::<Vec<_>>()
    })).collect::<Vec<_>>()}))
}
pub(super) fn process(generation: u64, job: Job, tx: &EventSender, current: &AtomicU64) {
    let active = || current.load(Ordering::Relaxed) == generation;
    match job {
        Job::Route(view, entity, query) => {
            let cached = tx.resume(generation);
            if view == View::Flow {
                if let Some(cached) = cached.as_ref().filter(|p| !p.rows.is_empty()) {
                    let mut page = (**cached).clone();
                    page.loading = false;
                    let _ = tx.send(Event::Page(generation, Arc::new(page)));
                    return;
                }
            }
            let result = load_page(view, entity, query, &mut |page| {
                if !active() {
                    return Err("Superseded load".into());
                }
                tx.send(Event::Page(generation, Arc::new(page.clone())))
                    .map_err(|_| "Window closed".into())
            });
            if !active() {
                return;
            }
            let mut page = result.unwrap_or_else(|error| Page {
                title: view.title().into(),
                error,
                ..Default::default()
            });
            for _ in 0..cached
                .as_deref()
                .map(session::pagination_depth)
                .unwrap_or(0)
            {
                if !page.error.is_empty()
                    || page.next.is_empty()
                    || page.next.starts_with("flow:")
                    || !active()
                {
                    break;
                }
                let cursor = page.next.clone();
                match invoke("SPCatalogNextJSON", &[json!(cursor)]) {
                    Ok(value) => append(&mut page, &cursor, value),
                    Err(error) => {
                        page.error = error;
                        break;
                    }
                }
            }
            if active() {
                let _ = tx.send(Event::Page(generation, Arc::new(page)));
            }
        }
        Job::More(mut page) => {
            if let Some(index) = page.raw.iter().position(|(k, _)| k == "spotify:more-shelf") {
                let (_, request) = page.raw.remove(index);
                let title = request["title"].as_str().unwrap_or("");
                let target = request["target"].as_u64().unwrap_or(0) as usize;
                if let Err(error) = load_shelf_more(
                    &mut page,
                    title,
                    target,
                    &mut |partial| {
                        if !active() {
                            return Err("Superseded load".into());
                        }
                        tx.send(Event::Page(generation, Arc::new(partial.clone())))
                            .map_err(|_| "Window closed".into())
                    },
                    &invoke,
                ) {
                    page.error = error;
                }
                let cover = page
                    .entity
                    .as_ref()
                    .map(|item| item.artwork.as_str())
                    .unwrap_or("");
                directory_covers(&mut page.sections, cover, &invoke);
                page.loading = false;
                if active() {
                    let _ = tx.send(Event::Page(generation, Arc::new(page)));
                }
                return;
            }
            let cursor = page.next.clone();
            let result = if let Some(config) = cursor.strip_prefix("flow:") {
                invoke(
                    if config.is_empty() || config == "default" {
                        "DZFlowJSON"
                    } else {
                        "DZFlowForConfigJSON"
                    },
                    &if config.is_empty() || config == "default" {
                        vec![]
                    } else {
                        vec![json!(config)]
                    },
                )
            } else {
                invoke("SPCatalogNextJSON", &[json!(cursor)])
            };
            match result {
                Ok(value) if cursor.starts_with("flow:") => {
                    append_flow_rows(&mut page, tracks(&value));
                    page.error.clear();
                }
                Ok(value) => append(&mut page, &cursor, value),
                Err(error) => page.error = error,
            }
            page.loading = false;
            if active() {
                let _ = tx.send(Event::Page(generation, Arc::new(page)));
            }
        }
        Job::Api(name, args) => {
            let result = if name == "SpotifyStyleMixGenres" {
                invoke("StyleMixGenres", &args)
            } else {
                invoke(&name, &args).map(|value| spotify_value(&value))
            };
            let _ = tx.send(Event::Api(generation, Provider::Spotify, name, result));
        }
        Job::TrackInfo(track) => {
            let (meta, artist) = track_info(&track, &invoke);
            let meta = super::musicbrainz::enrich(&track, meta);
            if active() {
                let _ = tx.send(Event::TrackInfo(generation, Box::new(track), meta, artist));
            }
        }
        Job::Playlist(mut mutation) => {
            let result = crate::playlist::execute_spotify(&mut mutation);
            let _ = tx.send(Event::Playlist(
                generation,
                Provider::Spotify,
                mutation,
                result,
            ));
        }
        Job::Lyrics(..) | Job::LyricsWindow(..) => {} // Forwarded to the bounded lyrics worker.
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn track_info_resolves_artist_from_full_credits_and_unwraps_profile() {
        let track = Entry {
            id: "spotify:track:song".into(),
            ..Default::default()
        };
        let (meta, artist) = track_info(&track, &|name, args| match name {
            "DZTrackMetadataJSON" => Ok(json!({"id":"spotify:track:song","name":"Song",
                "artists":[{"id":"spotify:artist:primary","name":"Artist"}]})),
            "DZArtistProfileJSON" => {
                assert_eq!(args, &[json!("spotify:artist:primary")]);
                Ok(
                    json!({"artist":{"id":"spotify:artist:primary","name":"Artist",
                    "monthlyListeners":900,"followers":100},"top":{"items":[]}}),
                )
            }
            _ => panic!("Unexpected request: {name}"),
        });
        assert_eq!(meta["name"], "Song");
        assert_eq!(artist["monthlyListeners"], 900);
        assert_eq!(artist["followers"], 100);
        assert!(entry(&artist, "artist").is_some());
    }

    #[test]
    fn recent_contexts_use_cards_and_track_history_keeps_track_rows() {
        let mut page = Page {
            title: "Recently played".into(),
            ..Default::default()
        };
        recent_history(
            &mut page,
            &json!({"items":[
                {"uri":"spotify:playlist:mix", "type":"playlist", "name":"Daily Mix 5"},
                {"uri":"spotify:album:album", "type":"album", "name":"Album"},
                {"uri":"spotify:track:song", "type":"track", "name":"Song", "duration_ms":180000}
            ]}),
        );
        assert!(page.rows.is_empty());
        assert_eq!(page.sections.len(), 1);
        assert_eq!(page.sections[0].title, "Recently played");
        let entries = &page.sections[0].entries;
        assert_eq!(
            entries.iter().map(|e| e.kind.as_str()).collect::<Vec<_>>(),
            vec!["playlist", "album", "track"]
        );
        assert_eq!(entries[0].id, "spotify:playlist:mix");

        let mut tracks = Page::default();
        recent_history(
            &mut tracks,
            &json!({"items":[
                {"track":{"uri":"spotify:track:song", "type":"track", "name":"Song", "duration_ms":180000},
                 "played_at":"2026-10-28T10:00:00Z"}
            ]}),
        );
        assert!(tracks.sections.is_empty());
        assert_eq!(tracks.rows.len(), 1);
        assert_eq!(tracks.rows[0].duration, 180000);
    }
    #[test]
    fn nested_genre_covers_use_cached_art_then_parent_without_downloading() {
        let genre = |id: &str, art: &str| Entry {
            id: id.into(),
            kind: "channel".into(),
            artwork: art.into(),
            ..Default::default()
        };
        let mut shelves = vec![Section {
            title: "Related genres".into(),
            entries: vec![
                genre("spotify:page:house", ""),
                genre("spotify:page:techno", ""),
                genre("spotify:page:rock", "https://i.scdn.co/image/official"),
            ],
            ..Default::default()
        }];
        let fetch = |name: &str, args: &[Value]| {
            assert_eq!(name, "SPCatalogDirectoryCachedJSON");
            if args[0] == "spotify:page:house" {
                Ok(json!({"sections":[{"title":"House essentials","items":[{
                    "type":"playlist","uri":"spotify:playlist:house","name":"House",
                    "images":[{"url":"https://i.scdn.co/image/house"}]
                }]}]}))
            } else {
                Ok(Value::Null)
            }
        };
        directory_covers(&mut shelves, "https://i.scdn.co/image/parent", &fetch);
        assert_eq!(
            shelves[0].entries[0].artwork,
            "https://i.scdn.co/image/house"
        );
        assert_eq!(
            shelves[0].entries[1].artwork,
            "https://i.scdn.co/image/parent"
        );
        assert_eq!(
            shelves[0].entries[2].artwork,
            "https://i.scdn.co/image/official"
        );
        assert_eq!(
            shelves[0].entries[1].raw["directory_artwork_fallback"],
            true
        );
    }
    #[test]
    fn shelf_more_fetches_only_the_next_two_rows_and_advances_the_cursor() {
        let mut page = Page {
            sections: vec![Section {
                title: "Rock Classics".into(),
                entries: (0..8)
                    .map(|i| Entry {
                        kind: "playlist".into(),
                        id: format!("spotify:playlist:{i}"),
                        ..Default::default()
                    })
                    .collect(),
                ..Default::default()
            }],
            ..Default::default()
        };
        remember_shelf(
            &mut page,
            "Rock Classics",
            &json!({"sections":[{
                "uri":"spotify:section:classics","offset":8,"total":30
            }]}),
            "",
        );
        let calls = std::sync::atomic::AtomicUsize::new(0);
        let fetch = |name: &str, args: &[Value]| {
            assert_eq!(name, "SPCatalogSectionJSON");
            assert_eq!(args[2], 8);
            calls.fetch_add(1, Ordering::Relaxed);
            let offset = args[1].as_u64().unwrap();
            Ok(
                json!({"offset":offset+8,"total":30,"items":(offset..offset+8)
                .map(|i|json!({"type":"playlist","uri":format!("spotify:playlist:{i}"),"name":format!("Playlist {i}")})).collect::<Vec<_>>()}),
            )
        };
        for target in [16, 24] {
            load_shelf_more(&mut page, "Rock Classics", target, &mut |_| Ok(()), &fetch).unwrap();
            assert_eq!(page.sections[0].entries.len(), target);
        }
        assert_eq!(calls.load(Ordering::Relaxed), 2);
        assert_eq!(page.raw[0].1["sections"][0]["offset"], 24);
        load_shelf_more(&mut page, "Rock Classics", 24, &mut |_| Ok(()), &|_, _| {
            panic!("already loaded")
        })
        .unwrap();
    }
    #[test]
    fn nested_directories_expand_progressively_with_a_bounded_download_budget() {
        let mut page = Page {
            sections: vec![Section {
                title: "Genres".into(),
                entries: (0..20)
                    .map(|i| Entry {
                        id: format!("spotify:page:root{i}"),
                        title: format!("Genre {i}"),
                        kind: "channel".into(),
                        artwork: "https://i.scdn.co/image/parent".into(),
                        ..Default::default()
                    })
                    .collect(),
                ..Default::default()
            }],
            ..Default::default()
        };
        let requests = std::sync::atomic::AtomicUsize::new(0);
        let mut updates = 0;
        expand_discovery_directories(
            &mut page,
            &mut |_| {
                updates += 1;
                Ok(())
            },
            &|name, args| {
                if name == "SPCatalogDirectoryCachedJSON" {
                    return Ok(Value::Null);
                }
                requests.fetch_add(1, Ordering::Relaxed);
                if args[0] == "spotify:page:root0" {
                    Ok(json!({"sections":[{"title":"Related content","items":[
                        {"type":"category","uri":"spotify:page:house","name":"House"},
                        {"type":"category","uri":"spotify:page:techno","name":"Techno"},
                    ]}]}))
                } else {
                    Ok(Value::Null)
                }
            },
            3,
            None,
        )
        .unwrap();
        assert!(updates > 1);
    }

    #[test]
    fn expanding_genres_does_not_add_or_fetch_categories() {
        let mut page = Page {
            sections: vec![Section {
                title: "Genres".into(),
                entries: vec![Entry {
                    id: "spotify:page:root".into(),
                    kind: "channel".into(),
                    ..Default::default()
                }],
                ..Default::default()
            }],
            ..Default::default()
        };
        expand_discovery_directories(
            &mut page,
            &mut |_| Ok(()),
            &|name, args| {
                if name == "SPCatalogDirectoryCachedJSON" {
                    return Ok(Value::Null);
                }
                assert_ne!(args[0], "spotify:page:discover");
                Ok(if args[0] == "spotify:page:root" {
                    json!({"sections":[{"title":"Related","items":[
                        {"type":"category","uri":"spotify:page:discover","name":"Discover Jazz"},
                        {"type":"category","uri":"spotify:page:house","name":"House"}
                    ]}]})
                } else {
                    Value::Null
                })
            },
            3,
            Some(("Genres", 10)),
        )
        .unwrap();
        assert_eq!(page.sections.len(), 1);
        assert_eq!(page.sections[0].title, "Genres");
        assert!(page.sections[0]
            .entries
            .iter()
            .any(|e| e.id == "spotify:page:house"));
        assert!(!page.sections[0]
            .entries
            .iter()
            .any(|e| e.id == "spotify:page:discover"));
    }

    #[test]
    fn cached_directory_expansion_does_not_request_the_network() {
        let mut page = Page {
            sections: vec![Section {
                title: "Genres".into(),
                entries: vec![Entry {
                    id: "spotify:page:root0".into(),
                    title: "Genre 0".into(),
                    kind: "channel".into(),
                    artwork: "https://i.scdn.co/image/parent".into(),
                    ..Default::default()
                }],
                ..Default::default()
            }],
            ..Default::default()
        };
        let requests = std::sync::atomic::AtomicUsize::new(0);
        let mut updates = 0;
        expand_discovery_directories(
            &mut page,
            &mut |_| {
                updates += 1;
                Ok(())
            },
            &|name, _args| {
                if name == "SPCatalogDirectoryCachedJSON" {
                    return Ok(json!({"sections":[{"title":"Related content","items":[
                        {"type":"category","uri":"spotify:page:house","name":"House"}
                    ]}]}));
                }
                requests.fetch_add(1, Ordering::Relaxed);
                Ok(Value::Null)
            },
            3,
            None,
        )
        .unwrap();
        assert_eq!(requests.load(Ordering::Relaxed), 0);
        assert!(updates > 0);
    }

    #[test]
    fn localized_browse_all_is_split_into_real_genres_moods_and_podcasts() {
        let payload = json!({"sections": [
            {"title": "Genres", "items": [{"type": "category", "id": "spotify:genre:rock", "name": "Rock"}]},
            {"title": "Browse all", "items": [{"type": "category", "uri": "spotify:page:focus", "name": "Focus"}]},
            {"title": "Podcasts", "items": [{"type": "category", "uri": "spotify:page:podcasts", "name": "Podcasts"}]},
            {"title": "Recommended playlists", "items": [{"type": "playlist", "uri": "spotify:playlist:mix", "name": "Mix"}]}
        ]});
        let shelves = discovery_sections(&payload);
        assert!(shelves[..3].iter().all(|s| s.entries[0].kind == "channel"));
        for section in &shelves {
            assert!(super::super::discover_section_matches(section, 0));
        }
    }

    #[test]
    fn genre_named_shelf_does_not_turn_mood_categories_into_genres() {
        let payload = json!({"sections": [
            {"title": "Hepsine göz at", "items": [
                {"type": "category", "uri": "spotify:page:0JQ5DAqbMKFDXXwE9BDJAr", "name": "Odaklan"},
                {"type": "category", "uri": "spotify:page:0JQ5IMCbQBLl5gxKLgufp8", "name": "Eğitim"}
            ]}
        ]});
        let shelves = discovery_sections(&payload);
        assert!(shelves
            .iter()
            .any(|s| s.title == "Categories" || s.title == "Podcasts by category"));
    }

    #[test]
    fn shelf_duplicates_keep_library_priority_and_independent_recent_history() {
        let shelves = home_sections(&json!({
            "sections": [
                {"title": "Your playlists", "items": [{"type": "playlist", "uri": "spotify:playlist:m1", "name": "Daily Mix 1"}]},
                {"title": "Daily Mix 1", "items": [{"type": "playlist", "uri": "spotify:playlist:m1", "name": "Daily Mix 1"}]}
            ]
        }));
        assert_eq!(shelves.len(), 1);
    }

    #[test]
    fn shelf_variety_limits_artist_repeats_but_keeps_all_daily_mixes() {
        let shelves = home_sections(&json!({
            "sections": [
                {"title": "Made for you", "items": [
                    {"type": "playlist", "uri": "spotify:playlist:m1", "name": "Daily Mix 1"},
                    {"type": "playlist", "uri": "spotify:playlist:m2", "name": "Daily Mix 2"}
                ]}
            ]
        }));
        assert_eq!(shelves[0].entries.len(), 2);
    }

    #[test]
    fn home_groups_real_radios_artist_playlists_and_artists_without_duplicates() {
        let shelves = home_sections(&json!({
            "sections": [
                {"title": "Radios for you", "items": [{"type": "playlist", "uri": "spotify:playlist:r1", "name": "Artist Radio"}, {"type": "playlist", "uri": "spotify:playlist:r2", "name": "Other Radio"}]},
                {"title": "This Is…", "items": [{"type": "playlist", "uri": "spotify:playlist:t1", "name": "This Is Artist"}]},
                {"title": "Recommended artists", "items": [{"type": "artist", "uri": "spotify:artist:a1", "name": "Artist"}]},
                {"title": "Made for you", "items": [{"type": "playlist", "uri": "spotify:playlist:m1", "name": "Daily Mix 1"}]}
            ]
        }));
        assert_eq!(
            shelves.iter().map(|s| s.title.as_str()).collect::<Vec<_>>(),
            [
                "Radios for you",
                "This Is…",
                "Recommended artists",
                "Made for you"
            ]
        );
        assert_eq!(
            shelves.iter().map(|s| s.entries.len()).collect::<Vec<_>>(),
            vec![2, 1, 1, 1]
        );
        assert_eq!(shelves[3].entries[0].title, "Daily Mix 1");
        assert!(home_sections(&json!({"sections":[]})).is_empty());
    }
    #[test]
    fn language_genre_matches_require_both_whole_word_aliases() {
        let selection = crate::style_mix::Selection::new(
            0,
            crate::style_mix::GENRES
                .iter()
                .position(|genre| genre.0 == "Rock")
                .unwrap(),
        )
        .unwrap();
        assert!(matches_selection("Türkçe Rock", selection));
        assert!(!matches_selection("Turkish Pop", selection));
        assert!(!matches_selection("Japanese Rock", selection));
        assert!(!matches_selection("Turkish Rocket", selection));
    }
    #[test]
    fn spotify_identity_credits_art_and_duration_survive_shared_wire() {
        let value = json!({"type":"track","id":"song","uri":"spotify:track:song","name":"Song","duration_ms":1234,
            "artists":[{"id":"artist","name":"Artist"}],"album":{"id":"album","name":"Album","images":[{"url":"https://i.scdn.co/cover"}]}});
        let track = entry(&value, "track").unwrap();
        assert_eq!(track.duration, 1234);
        assert_eq!(track.id, "spotify:track:song");
        assert_eq!(track.artist_id, "spotify:artist:artist");
        assert_eq!(track.album_id, "spotify:album:album");
        assert_eq!(track.artwork, "https://i.scdn.co/cover");
        assert_eq!(track.artist_entries()[0].id, "spotify:artist:artist");
        assert_eq!(track.album_entry().unwrap().id, "spotify:album:album");
        assert_eq!(track.wire()["id"], "spotify:track:song");
    }
    #[test]
    fn wrappers_keep_history_and_show_metadata_without_inventing_metrics() {
        let result = rows(
            &json!({"items":[{"played_at":"2026-10-27","track":{"id":"a","type":"track","name":"Song"}}, {"track":null}]}),
            "track",
        );
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].raw["played_at"], "2026-10-27");
        let show=entry(&json!({"uri":"spotify:show:a","name":"Show","total_episodes":5,"publisher":"Publisher"}),"podcast").unwrap();
        assert_eq!(show.kind, "podcast");
        assert_eq!(show.episode_count(), Some(5));
        let artist = entry(
            &json!({"uri":"spotify:artist:a","name":"Artist","followers":{"total":45}}),
            "artist",
        )
        .unwrap();
        assert_eq!(artist.raw["followers"], 45);
        assert!(artist.raw["monthlyListeners"].is_null());
    }
    #[test]
    fn library_pagination_keeps_collections_and_stops_cyclic_cursors() {
        let mut page = Page {
            rows: rows(
                &json!([{"id":"a","name":"Album A","type":"album"}]),
                "album",
            ),
            ..Default::default()
        };
        append(
            &mut page,
            "https://api.spotify.com/v1/me/albums?page=2",
            json!({"items":[{"album":{"id":"b","name":"Album B","type":"album"}}],"next":"https://api.spotify.com/v1/me/albums?page=2"}),
        );
        assert_eq!(page.rows.len(), 2);
        assert_eq!(page.rows[1].kind, "album");
        assert!(page.next.is_empty());
    }
    #[test]
    fn podcast_show_page_and_episode_pagination_preserve_show_metadata_and_playable_episodes() {
        let episodes_payload = json!({
            "items": [
                {
                    "uri": "spotify:episode:ep1",
                    "id": "spotify:episode:ep1",
                    "name": "Episode 1",
                    "type": "episode",
                    "duration_ms": 1800000,
                    "release_date": "2024-03-01",
                    "images": [{"url": "https://i.scdn.co/image/ep1", "width": 300, "height": 300}],
                    "show": {
                        "uri": "spotify:show:show1",
                        "id": "spotify:show:show1",
                        "name": "My Show",
                        "type": "show",
                        "images": [{"url": "https://i.scdn.co/image/show1", "width": 640, "height": 640}]
                    }
                }
            ],
            "next": "spotify-web:cursor2"
        });
        let initial_rows = tracks(&episodes_payload);
        assert_eq!(initial_rows.len(), 1);
        assert_eq!(initial_rows[0].kind, "episode");
        assert_eq!(initial_rows[0].title, "Episode 1");
        assert_eq!(initial_rows[0].subtitle, "My Show");

        let mut page = Page {
            rows: initial_rows,
            next: "spotify-web:cursor2".into(),
            ..Default::default()
        };

        let show_meta = json!({
            "uri": "spotify:show:show1",
            "name": "My Show",
            "type": "show",
            "publisher": "Host Name",
            "images": [{"url": "https://i.scdn.co/image/show1", "width": 640, "height": 640}]
        });
        metadata(&mut page, &show_meta);
        assert!(page.entity.is_some());
        assert_eq!(page.entity.as_ref().unwrap().kind, "podcast");
        assert_eq!(
            page.entity.as_ref().unwrap().artwork,
            "https://i.scdn.co/image/show1"
        );

        let more_payload = json!({
            "items": [
                {
                    "uri": "spotify:episode:ep2",
                    "id": "spotify:episode:ep2",
                    "name": "Episode 2",
                    "type": "episode",
                    "duration_ms": 2000000,
                    "release_date": "2024-02-20"
                }
            ],
            "next": ""
        });
        append(&mut page, "spotify-web:cursor2", more_payload);
        assert_eq!(page.rows.len(), 2);
        assert_eq!(page.rows[1].kind, "episode");
        assert_eq!(page.rows[1].title, "Episode 2");
        assert_eq!(page.rows[1].artwork, "https://i.scdn.co/image/show1");
        assert_eq!(page.rows[1].subtitle, "My Show");
        assert!(page.next.is_empty());
    }
}
