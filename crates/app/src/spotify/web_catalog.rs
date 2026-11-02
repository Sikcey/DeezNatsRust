//! Web-player catalog adapter. Transport owns cookies, token refresh and persisted hashes.
//! Shapes verified against the current Spotify desktop bundle and SpotAPI/Meld consumers.
mod flow_queue;
use super::web;
use serde_json::{json, Value};
use std::collections::HashSet;

type ResultValue = Result<Value, String>;
const CURSOR: &str = "spotify-web:";

fn text<'a>(v: &'a Value, keys: &[&str]) -> &'a str {
    keys.iter().find_map(|k| v.get(*k)?.as_str()).unwrap_or("")
}
fn arg(args: &[Value], n: usize) -> Result<&str, String> {
    args.get(n)
        .and_then(Value::as_str)
        .ok_or_else(|| "Missing Spotify request argument".into())
}
fn uri(args: &[Value], n: usize, kinds: &[&str]) -> Result<String, String> {
    let value = arg(args, n)?;
    if value.len() == 22 && value.bytes().all(|b| b.is_ascii_alphanumeric()) && kinds.len() == 1 {
        return Ok(format!("spotify:{}:{}", kinds[0], value));
    }
    let parts: Vec<_> = value.split(':').collect();
    if parts.len() != 3
        || parts[0] != "spotify"
        || !kinds.contains(&parts[1])
        || parts[2].len() != 22
        || !parts[2].bytes().all(|b| b.is_ascii_alphanumeric())
    {
        return Err("Select a valid Spotify item of the correct type".into());
    }
    Ok(value.into())
}
fn number(v: &Value) -> Option<u64> {
    v.as_u64().or_else(|| v.as_str()?.parse().ok())
}
fn images(v: &Value) -> Value {
    fn gather(v: &Value, out: &mut Vec<Value>) {
        if let Some(url) = v["url"].as_str().filter(|s| s.starts_with("https://")) {
            out.push(json!({"url":url,"width":v["width"],"height":v["height"]}));
        } else if let Some(a) = v.as_array() {
            for v in a {
                gather(v, out);
            }
        } else {
            for key in ["sources", "items"] {
                if let Some(v) = v.get(key) {
                    gather(v, out);
                }
            }
        }
    }
    let mut out = Vec::new();
    gather(v, &mut out);
    json!(out)
}

/// Unwrap only known entity wrappers; never manufacture an entity from an error response.
pub(super) fn entity(v: &Value) -> Option<Value> {
    if !v.is_object() || text(v, &["__typename"]).contains("Error") {
        return None;
    }
    let own_uri = text(v, &["uri", "_uri"]);
    let own_name = !text(v, &["name", "title"]).is_empty() || v.pointer("/profile/name").is_some();
    if own_uri.is_empty() || !own_name {
        for key in [
            "content", "itemV2", "item", "track", "episode", "album", "data", "entity",
        ] {
            if let Some(child) = v.get(key).filter(|c| c.is_object()) {
                let mut child = child.clone();
                if text(&child, &["uri", "_uri"]).is_empty() && !own_uri.is_empty() {
                    child["uri"] = json!(own_uri);
                }
                if let Some(mut result) = entity(&child) {
                    if result["uri"].as_str().unwrap_or("").is_empty() {
                        result["uri"] = json!(text(v, &["_uri", "uri"]));
                    }
                    for key in ["uid", "addedAt", "added_at", "played_at", "timestampMs"] {
                        if let Some(value) = v.get(key) {
                            result[key] = value.clone();
                        }
                    }
                    return Some(result);
                }
            }
        }
    }
    let uri = text(v, &["uri", "_uri"]);
    let typ = uri.split(':').nth(1)?;
    if !matches!(
        typ,
        "track"
            | "album"
            | "artist"
            | "playlist"
            | "show"
            | "episode"
            | "page"
            | "genre"
            | "section"
            | "category"
    ) {
        return None;
    }
    let name = [
        v.get("name"),
        v.get("title"),
        v.pointer("/profile/name"),
        v.pointer("/data/cardRepresentation/title/transformedLabel"),
        v.pointer("/cardRepresentation/title/transformedLabel"),
    ]
    .into_iter()
    .flatten()
    .find_map(Value::as_str)?;
    let kind = if matches!(typ, "page" | "genre" | "section") {
        "category"
    } else {
        typ
    };
    let mut out = json!({"uri":uri,"id":uri,"type":kind,"name":name,"provider":"spotify"});
    if kind == "track" {
        crate::spotify_diagnostics::record(
            "catalog.track",
            &format!(
                "duration_ms={} durationMs={} duration_object={} duration_numeric={}",
                v.get("duration_ms").is_some(),
                v.get("durationMs").is_some(),
                v.pointer("/duration/totalMilliseconds").is_some(),
                v.get("duration").is_some_and(Value::is_number)
            ),
        );
    }
    for key in [
        "description",
        "publisher",
        "is_playable",
        "language",
        "is_local",
        "explicit",
        "format",
        "attributes",
        "uid",
        "added_at",
        "played_at",
        "track_number",
        "disc_number",
        "external_ids",
        "genres",
    ] {
        if let Some(value) = v.get(key) {
            out[key] = value.clone();
        }
    }
    if let Some(publisher) = v.get("publisher") {
        if let Some(name) = publisher
            .as_str()
            .or_else(|| publisher.get("name").and_then(Value::as_str))
        {
            out["publisher"] = json!(name);
        }
    }
    if out.get("description").is_none() {
        if let Some(desc) = v.get("htmlDescription") {
            out["description"] = desc.clone();
        }
    }
    let art = [
        v.get("images"),
        v.get("coverArt"),
        v.get("artwork"),
        v.pointer("/visuals/avatarImage"),
        v.pointer("/data/cardRepresentation/artwork"),
        v.pointer("/cardRepresentation/artwork"),
        v.pointer("/data/cardRepresentation/backgroundImage"),
        v.pointer("/cardRepresentation/backgroundImage"),
    ]
    .into_iter()
    .flatten()
    .map(images)
    .find(|v| v.as_array().is_some_and(|a| !a.is_empty()));
    if let Some(art) = art {
        out["images"] = art;
    }
    if let Some(ms) = v
        .get("duration_ms")
        .and_then(number)
        .or_else(|| v.get("durationMs").and_then(number))
        .or_else(|| v.pointer("/duration/totalMilliseconds").and_then(number))
    {
        out["duration_ms"] = json!(ms);
    }
    if let Some(artists) = v.get("artists") {
        out["artists"] = json!(collect(artists));
    }
    if let Some(album) = v.get("albumOfTrack").or_else(|| v.get("album")) {
        if let Some(album) = entity(album) {
            out["album"] = album;
        } else {
            // Artist top-track albums can omit their name while retaining cover art.
            out["album"] =
                json!({"uri": text(album, &["uri", "_uri"]), "images": images(&album["coverArt"])});
        }
    }
    if let Some(explicit) = v.pointer("/contentRating/label").and_then(Value::as_str) {
        if matches!(explicit, "EXPLICIT" | "NONE" | "CLEAN") {
            out["explicit"] = json!(explicit == "EXPLICIT");
        }
    }
    if let Some(track_number) = v.get("trackNumber").and_then(number) {
        out["track_number"] = json!(track_number);
    }
    if let Some(show) = v.pointer("/podcastV2/data").or_else(|| v.get("show")) {
        if let Some(show) = entity(show) {
            out["show"] = show;
        }
    }
    if let Some(owner) = v.pointer("/ownerV2/data").or_else(|| v.get("owner")) {
        out["owner"] =
            json!({"uri":owner["uri"],"display_name":text(owner,&["name","display_name"])});
    }
    for (target, paths) in [
        (
            "monthlyListeners",
            &["/stats/monthlyListeners", "/monthlyListeners"][..],
        ),
        ("followers", &["/stats/followers", "/followers/total"][..]),
        (
            "playCount",
            &["/playcount", "/playCount", "/playCount/total"][..],
        ),
        (
            "total_tracks",
            &[
                "/content/totalCount",
                "/tracks/totalCount",
                "/tracksV2/totalCount",
            ][..],
        ),
    ] {
        if let Some(n) = paths.iter().find_map(|p| v.pointer(p).and_then(number)) {
            out[target] = json!(n);
        }
    }
    if let Some(date) = v
        .get("release_date")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .or_else(|| {
            v.pointer("/releaseDate/isoString")
                .and_then(Value::as_str)
                .map(str::to_owned)
        })
        .or_else(|| {
            v.pointer("/date/isoString")
                .and_then(Value::as_str)
                .map(str::to_owned)
        })
        .or_else(|| number(&v["date"]["year"]).map(|y| y.to_string()))
    {
        out["release_date"] = json!(date);
    }
    if kind == "track" && out["release_date"].is_null() {
        if let Some(date) = out.pointer("/album/release_date").cloned() {
            out["release_date"] = date;
        }
    }
    Some(out)
}

pub(super) fn collect(v: &Value) -> Vec<Value> {
    fn visit(v: &Value, out: &mut Vec<Value>, depth: usize) {
        if depth > 12 {
            return;
        }
        if let Some(item) = entity(v) {
            out.push(item);
            return;
        }
        if let Some(a) = v.as_array() {
            for item in a {
                visit(item, out, depth + 1);
            }
        } else {
            for key in [
                "items",
                "releases",
                "tracks",
                "tracksV2",
                "albums",
                "artists",
                "playlists",
                "shows",
                "episodes",
                "episodesV2",
                "entity",
            ] {
                if let Some(child) = v.get(key) {
                    visit(child, out, depth + 1);
                }
            }
        }
    }
    let mut out = Vec::new();
    visit(v, &mut out, 0);
    // Playlist duplicates have distinct UIDs and must survive normalization.
    out
}
fn title(v: &Value) -> &str {
    [
        v.pointer("/data/title/transformedLabel"),
        v.pointer("/data/title/text"),
        v.pointer("/data/title/translatedBaseText"),
        v.pointer("/title/transformedLabel"),
        v.get("title"),
        v.get("name"),
    ]
    .into_iter()
    .flatten()
    .find_map(Value::as_str)
    .unwrap_or("")
}
pub(super) fn sections(v: &Value) -> Vec<Value> {
    v.as_array()
        .into_iter()
        .flatten()
        .map(|section| {
            let items = collect(&section["sectionItems"]);
            let failed = text(&section["data"], &["__typename"]).contains("Error");
            json!({"title":title(section),"uri":section["uri"],"items":items,
            "offset":section["sectionItems"]["items"].as_array().map_or(0,Vec::len),
            "total":section["sectionItems"]["totalCount"],
            "error":if failed {"Spotify could not load this section"} else {""}})
        })
        .filter(|s| {
            !s["title"].as_str().unwrap_or("").is_empty()
                || !s["items"].as_array().unwrap().is_empty()
        })
        .collect()
}
fn at(value: &Value, pointer: &str) -> ResultValue {
    value
        .pointer(pointer)
        .filter(|v| !v.is_null())
        .cloned()
        .ok_or_else(|| "Spotify web response is missing the requested content".into())
}
fn home() -> ResultValue {
    let value = web::query(
        "home",
        json!({"homeEndUserIntegration":"INTEGRATION_WEB_PLAYER","timeZone":"UTC","sp_t":"","facet":"","sectionItemsLimit":20,"includeEpisodeContentRatingsV2":false}),
    )?;
    let root = at(&value, "/data/home")?;
    let shelves = at(&root, "/sectionContainer/sections/items")?;
    Ok(json!({"sections":sections(&shelves),"title":root["greeting"]["transformedLabel"]}))
}
fn browse(page: Option<&str>) -> ResultValue {
    if let Some(page) = page {
        if page.len() > 300 || !page.starts_with("spotify:") || page.chars().any(char::is_control) {
            return Err("Invalid Spotify browse page".into());
        }
    }
    super::directory_cache::load(&format!("browse-v3:{}", page.unwrap_or("root")), || {
        browse_uncached(page)
    })
}
fn browse_uncached(page: Option<&str>) -> ResultValue {
    if page.is_none() {
        let value = web::browse()?;
        let root = value
            .pointer("/data/browse")
            .or_else(|| value.pointer("/data/browseStart"))
            .ok_or("Spotify browse page returned no categories")?;
        return Ok(json!({"sections":sections(&at(root,"/sections/items")?)}));
    }
    let mut vars = json!({"pagePagination":{"offset":0,"limit":20},"sectionPagination":{"offset":0,"limit":8},"browseEndUserIntegration":"INTEGRATION_WEB_PLAYER","includeEpisodeContentRatingsV2":false});
    let (op, path) = if let Some(page) = page {
        if page.len() > 300 || !page.starts_with("spotify:") || page.chars().any(char::is_control) {
            return Err("Invalid Spotify browse page".into());
        }
        vars["uri"] = json!(page);
        ("browsePage", "/data/browse/sections/items")
    } else {
        unreachable!()
    };
    let v = web::query(op, vars.clone())?;
    Ok(json!({"sections":sections(&at(&v,path)?)}))
}
fn browse_section(uri: &str, offset: u64, limit: u64) -> ResultValue {
    if !uri.starts_with("spotify:section:")
        || uri.len() > 300
        || offset > 10_000
        || !(1..=100).contains(&limit)
    {
        return Err("Invalid Spotify section pagination".into());
    }
    super::directory_cache::load(&format!("section-v1:{uri}:{offset}:{limit}"), || {
        let value = web::query(
            "browseSection",
            json!({"uri":uri,"pagination":{"offset":offset,"limit":limit},
            "browseEndUserIntegration":"INTEGRATION_WEB_PLAYER","includeEpisodeContentRatingsV2":false}),
        )?;
        let raw = at(&value, "/data/browseSection")?;
        let count = raw["sectionItems"]["items"].as_array().map_or(0, Vec::len);
        let mut normalized = sections(&json!([raw]))
            .into_iter()
            .next()
            .ok_or("Spotify returned no section")?;
        normalized["offset"] = json!(offset + count as u64);
        if count == 0 {
            normalized["total"] = normalized["offset"].clone();
        }
        Ok(normalized)
    })
}

fn recent() -> ResultValue {
    let account = web::account()?;
    let user = account["id"]
        .as_str()
        .ok_or("Spotify account identity is unavailable")?;
    let mut url =
        url::Url::parse("https://spclient.wg.spotify.com/recently-played/v3/user/").unwrap();
    url.path_segments_mut()
        .map_err(|_| "Invalid recently played address")?
        .pop_if_empty()
        .push(user)
        .push("recently-played");
    url.query_pairs_mut().extend_pairs([
        ("format", "json"),
        ("offset", "0"),
        ("limit", "50"),
        ("filter", "default,collection-new-episodes"),
    ]);
    let value = web::rest_get(url.as_str())?;
    let root = value.as_array().and_then(|a| a.first()).unwrap_or(&value);
    let contexts = root["playContexts"]
        .as_array()
        .ok_or("Spotify returned no recently played history")?;
    let uris: Vec<_> = contexts
        .iter()
        .filter_map(|c| c["uri"].as_str())
        .filter(|u| u.starts_with("spotify:") && !u.starts_with("spotify:collection:"))
        .take(50)
        .collect();
    if uris.is_empty() {
        return Ok(json!({"items":[],"next":null}));
    }
    let response = web::query("fetchEntitiesForRecentlyPlayed", json!({"uris":uris}))?;
    let mut items = collect(&at(&response, "/data/lookup")?);
    for item in &mut items {
        if let Some(context) = contexts.iter().find(|c| c["uri"] == item["uri"]) {
            item["played_at"] = context["lastPlayedTime"].clone();
        }
    }
    Ok(json!({"items":items,"next":null}))
}
fn page(op: &str, vars: Value, pointer: &str) -> ResultValue {
    let value = web::query(op, vars.clone())?;
    let root = at(&value, pointer)?;
    let items = collect(&root);
    let offset = vars["offset"].as_u64().unwrap_or(0);
    let limit = vars["limit"].as_u64().unwrap_or(50);
    let total = root["totalCount"].as_u64();
    let next_offset = root
        .pointer("/pagingInfo/nextOffset")
        .and_then(Value::as_u64)
        .or_else(|| {
            total
                .filter(|t| offset + limit < *t)
                .map(|_| offset + limit)
        });
    let next = next_offset
        .filter(|n| *n > offset && *n <= 100_000)
        .map(|n| {
            let mut vars = vars.clone();
            vars["offset"] = json!(n);
            format!("{CURSOR}{}", json!({"op":op,"vars":vars,"pointer":pointer}))
        });
    Ok(json!({"items":items,"total":total,"next":next}))
}
fn library(filter: &str) -> ResultValue {
    page(
        "libraryV3",
        json!({"filters":[filter],"order":null,"textFilter":"","features":["LIKED_SONGS","YOUR_EPISODES_V2","PRERELEASES","EVENTS"],"limit":50,"offset":0,"flatten":true,"expandedFolders":[],"folderUri":null,"includeFoldersWhenFlattening":false}),
        "/data/me/libraryV3",
    )
}
fn playlist(uri: &str, offset: u64) -> ResultValue {
    page(
        "fetchPlaylist",
        json!({"uri":uri,"offset":offset,"limit":100,"enableWatchFeedEntrypoint":false}),
        "/data/playlistV2/content",
    )
}
fn artist(uri: &str) -> ResultValue {
    at(
        &web::query(
            "queryArtistOverview",
            json!({"uri":uri,"locale":"","preReleaseV2":true}),
        )?,
        "/data/artistUnion",
    )
}
fn search(query: &str) -> ResultValue {
    if query.trim().is_empty() || query.len() > 1000 {
        return Err("Enter a Spotify search".into());
    }
    let vars = json!({"searchTerm":query,"offset":0,"limit":20,"numberOfTopResults":5,"includeAudiobooks":true,"includeArtistHasConcertsField":false,"includePreReleases":true,"includeLocalConcertsField":false,"includeAuthors":false});
    let value = web::query("searchDesktop", vars)?;
    let root = at(&value, "/data/searchV2")?;
    Ok(
        json!({"tracks":{"items":collect(root.get("tracksV2").unwrap_or(&root["tracks"]))},
        "albums":{"items":collect(&root["albums"])},"artists":{"items":collect(&root["artists"])},
        "playlists":{"items":collect(&root["playlists"])},"shows":{"items":collect(root.get("podcasts").unwrap_or(&root["shows"]))},"episodes":{"items":collect(&root["episodes"])}}),
    )
}
fn mark(mut value: Value, name: &str, args: &[Value]) -> Value {
    if let Some(items) = value["items"].as_array_mut() {
        for item in items {
            item["spotify_station"] = json!({"name":name,"args":args});
        }
    }
    value
}
fn radio(uri: &str) -> ResultValue {
    let value=web::rest_get(&format!("https://spclient.wg.spotify.com/inspiredby-mix/v2/seed_to_playlist/{uri}?response-format=json"))?;
    let playlist_uri = [
        value.get("uri"),
        value.get("playlist_uri"),
        value.pointer("/playlist/uri"),
    ]
    .into_iter()
    .flatten()
    .find_map(Value::as_str)
    .filter(|s| s.starts_with("spotify:playlist:"))
    .ok_or("Spotify radio did not return a playlist")?;
    playlist(playlist_uri, 0)
}
fn flow_options() -> ResultValue {
    Ok(json!({"options":super::flow::MOODS.iter().map(|(id,title)|
        json!({"id":id,"title":title,"kind":"mood"})).collect::<Vec<_>>()}))
}

fn words(s: &str) -> String {
    format!(
        " {} ",
        s.to_lowercase()
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
fn matches_style(v: &Value, s: crate::style_mix::Selection) -> bool {
    let name = words(text(v, &["name"]));
    let has = |aliases: &[&str]| aliases.iter().any(|a| name.contains(&words(a)));
    has(crate::style_mix::REGIONS[s.region].1) && has(crate::style_mix::GENRES[s.genre].1)
}
fn style_mix(key: &str) -> ResultValue {
    let selected = crate::style_mix::Selection::parse(key)?;
    let query = format!(
        "{} {} Mix",
        crate::style_mix::REGIONS[selected.region].1[0],
        crate::style_mix::GENRES[selected.genre].1[0]
    );
    let mut sources = vec![None, None, None];
    crate::catalog::parallel::load(
        3,
        |index| match index {
            0 => home(),
            1 => library("Playlists"),
            _ => search(&query),
        },
        |index, result| {
            sources[index] = Some(result);
            Ok(())
        },
    )?;
    let mut candidates = Vec::new();
    if let Some(Ok(h)) = sources[0].take() {
        for section in h["sections"].as_array().into_iter().flatten() {
            candidates.extend(collect(&section["items"]));
        }
    }
    if let Some(Ok(v)) = sources[1].take() {
        candidates.extend(collect(&v));
    }
    match sources[2].take().expect("search completed") {
        Ok(value) => candidates.extend(collect(&value["playlists"])),
        Err(error) if !candidates.iter().any(|v| matches_style(v, selected)) => {
            return Err(error);
        }
        Err(_) => {}
    }

    let candidates: Vec<_> = candidates
        .iter()
        .filter(|v| matches_style(v, selected))
        .take(3)
        .collect();
    let mut playlists = vec![None; candidates.len()];
    crate::catalog::parallel::load(
        candidates.len(),
        |index| playlist(text(candidates[index], &["uri"]), 0),
        |index, result| {
            playlists[index] = Some(result);
            Ok(())
        },
    )?;
    let mut items = Vec::new();
    let mut seen = HashSet::new();
    let mut failure = None;
    for result in playlists.into_iter().flatten() {
        match result {
            Ok(v) => {
                for mut track in collect(&v) {
                    if track["type"] == "track"
                        && track["is_playable"] != false
                        && seen.insert(text(&track, &["uri"]).to_owned())
                    {
                        track[crate::style_mix::MARKER] = json!(key);
                        items.push(track);
                    }
                }
            }
            Err(e) => failure = Some(e),
        }
    }
    super::style_mix::retain_language(&mut items, selected, |track| track);
    if items.is_empty() {
        return Err(failure.unwrap_or_else(|| {
            "No playable tracks found in matching playlists. Try another language or genre.".into()
        }));
    }
    Ok(json!({"items":items}))
}
fn mutate_library(uri: &str, save: bool) -> ResultValue {
    let op = if save {
        "addToLibrary"
    } else {
        "removeFromLibrary"
    };
    let result = web::mutation(op, json!({"uris":[uri]}))?;
    confirmed(&result)?;
    Ok(json!(1))
}
fn confirmed(v: &Value) -> Result<(), String> {
    // HTTP success alone does not make a GraphQL error/union failure a successful mutation.
    fn failed(v: &Value) -> bool {
        match v {
            Value::Object(o) => {
                text(v, &["__typename"]).contains("Error")
                    || o.get("success") == Some(&Value::Bool(false))
                    || o.iter().any(|(k, v)| {
                        k == "errors" && v.as_array().is_some_and(|a| !a.is_empty()) || failed(v)
                    })
            }
            Value::Array(a) => a.iter().any(failed),
            _ => false,
        }
    }
    if failed(v) || v.get("data").is_none_or(Value::is_null) {
        Err("Spotify did not confirm the requested change".into())
    } else {
        Ok(())
    }
}

pub(super) fn is_mutation(name: &str) -> bool {
    matches!(
        name,
        "DZAlbumLibraryJSON"
            | "DZPlaylistLibraryJSON"
            | "RustPodcastLibraryJSON"
            | "DZAddFavorite"
            | "DZRemoveFavorite"
            | "DZDeletePlaylist"
            | "DZAddToPlaylist"
            | "DZRemoveFromPlaylist"
            | "DZRenamePlaylist"
            | "DZUpdatePlaylistJSON"
            | "DZCreatePlaylist"
            | "DZPlaylistCoverJSON"
            | "DZSaveFlowTunerJSON"
            | "DZFlowConfigSaveJSON"
    )
}

/// None means this adapter does not handle the operation; callers may use the public API.
pub(super) fn invoke(name: &str, args: &[Value]) -> Option<ResultValue> {
    let supported = matches!(
        name,
        "SPCatalogHomeJSON"
            | "SPCatalogDiscoveryJSON"
            | "SPCatalogChannelJSON"
            | "SPCatalogDirectoryCachedJSON"
            | "SPCatalogSectionJSON"
            | "SPCatalogNextJSON"
            | "DZFavoritesJSON"
            | "DZFavoriteAlbumsJSON"
            | "DZPlaylistsJSON"
            | "DZPodcastShelfJSON"
            | "DZArtistProfileJSON"
            | "DZArtistTopJSON"
            | "DZArtistSectionJSON"
            | "DZAlbumTracksJSON"
            | "DZPlaylistTracksJSON"
            | "DZPodcastEpisodesJSON"
            | "DZTrackMetadataJSON"
            | "SPCatalogItemJSON"
            | "DZSearchJSON"
            | "DZSearchPodcastsJSON"
            | "DZTrackMixJSON"
            | "DZArtistMixJSON"
            | "DZFlowJSON"
            | "DZFlowForConfigJSON"
            | "DZFlowAppendJSON"
            | "DZFlowConfigsJSON"
            | "DZFlowTunerJSON"
            | "DZFlowConfigTunerJSON"
            | "DZSaveFlowTunerJSON"
            | "DZFlowConfigSaveJSON"
            | "SPStyleMixJSON"
            | "StyleMixGenres"
            | "DZAlbumLibraryJSON"
            | "DZPlaylistLibraryJSON"
            | "RustPodcastLibraryJSON"
            | "DZAddFavorite"
            | "DZRemoveFavorite"
            | "DZDeletePlaylist"
            | "DZAddToPlaylist"
            | "DZRemoveFromPlaylist"
            | "DZRenamePlaylist"
            | "DZUpdatePlaylistJSON"
            | "DZCreatePlaylist"
            | "DZPlaylistCoverJSON"
            | "DZHistoryRecentJSON"
            | "DZHomeModuleJSON"
    );
    if !supported
        || name == "SPCatalogNextJSON"
            && !args
                .first()
                .and_then(Value::as_str)
                .is_some_and(|s| s.starts_with(CURSOR) || s.starts_with(flow_queue::PREFIX))
    {
        return None;
    }
    Some((|| match name {
        "SPCatalogHomeJSON" => home(),
        "DZHomeModuleJSON" => match args.first().and_then(Value::as_u64) {
            Some(0) => recent(),
            Some(1) => page(
                "fetchLibraryTracks",
                json!({"offset":0,"limit":50}),
                "/data/me/library/tracks",
            ),
            Some(2) => library("Playlists"),
            Some(3) | Some(8) => library("Albums"),
            Some(4) => library("Artists"),
            Some(7) => library("Podcasts"),
            Some(9) => library("Episodes"),
            _ => return Err("This Spotify shelf is not available through the web library".into()),
        },
        "SPCatalogDiscoveryJSON" => browse(None),
        "SPCatalogDirectoryCachedJSON" => Ok(super::directory_cache::peek(&format!(
            "browse-v3:{}",
            arg(args, 0)?
        ))
        .unwrap_or(Value::Null)),
        "SPCatalogSectionJSON" => browse_section(
            arg(args, 0)?,
            args.get(1)
                .and_then(Value::as_u64)
                .ok_or("Missing offset")?,
            args.get(2).and_then(Value::as_u64).ok_or("Missing limit")?,
        ),
        "SPCatalogChannelJSON" => browse(Some(arg(args, 0)?)),
        "SPCatalogNextJSON" if arg(args, 0)?.starts_with(flow_queue::PREFIX) => {
            flow_queue::next(arg(args, 0)?)
        }
        "SPCatalogNextJSON" => {
            let cursor = arg(args, 0)?
                .strip_prefix(CURSOR)
                .ok_or("Invalid Spotify cursor")?;
            if cursor.len() > 4000 {
                return Err("Invalid Spotify cursor".into());
            }
            let v: Value = serde_json::from_str(cursor).map_err(|_| "Invalid Spotify cursor")?;
            let op = text(&v, &["op"]);
            let pointer = text(&v, &["pointer"]);
            if !matches!(
                (op, pointer),
                ("libraryV3", "/data/me/libraryV3")
                    | ("fetchLibraryTracks", "/data/me/library/tracks")
                    | ("fetchPlaylist", "/data/playlistV2/content")
                    | ("getAlbum", "/data/albumUnion/tracksV2")
                    | ("queryPodcastEpisodes", "/data/podcastUnionV2/episodesV2")
            ) || v["vars"]["offset"].as_u64().is_none_or(|n| n > 100_000)
            {
                return Err("Invalid Spotify cursor".into());
            }
            page(op, v["vars"].clone(), pointer)
        }
        "DZFavoritesJSON" => page(
            "fetchLibraryTracks",
            json!({"offset":0,"limit":50}),
            "/data/me/library/tracks",
        ),
        "DZFavoriteAlbumsJSON" => library("Albums"),
        "DZPlaylistsJSON" => library("Playlists"),
        "DZPodcastShelfJSON" => library(if args.first().and_then(Value::as_u64) == Some(3) {
            "Episodes"
        } else {
            "Podcasts & Shows"
        }),
        "DZPlaylistTracksJSON" => playlist(&uri(args, 0, &["playlist"])?, 0),
        "DZAlbumTracksJSON" => page(
            "getAlbum",
            json!({"uri":uri(args,0,&["album"])?,"offset":0,"limit":50}),
            "/data/albumUnion/tracksV2",
        ),
        "DZPodcastEpisodesJSON" => {
            let offset = args.get(1).and_then(Value::as_u64).unwrap_or(0);
            page(
                "queryPodcastEpisodes",
                json!({"uri":uri(args,0,&["show"])?,"offset":offset,"limit":50}),
                "/data/podcastUnionV2/episodesV2",
            )
        }
        "DZTrackMetadataJSON" => {
            let v = web::query(
                "getTrack",
                json!({"uri":uri(args,0,&["track"])?,"includePrerelease":true}),
            )?;
            entity(&at(&v, "/data/trackUnion")?)
                .ok_or_else(|| "Spotify track metadata unavailable".into())
        }
        "DZArtistProfileJSON" => {
            let v = artist(&uri(args, 0, &["artist"])?)?;
            Ok(json!({"artist":entity(&v),"top":{"items":collect(&v["discography"]["topTracks"])}}))
        }
        "DZArtistTopJSON" => Ok(
            json!({"items":collect(&artist(&uri(args,0,&["artist"])?)?["discography"]["topTracks"])}),
        ),
        "DZArtistSectionJSON" => {
            let v = artist(&uri(args, 0, &["artist"])?)?;
            let index = args.get(1).and_then(Value::as_u64).unwrap_or(0);
            let p = match index {
                0 => "/discography/albums",
                2 => "/discography/singles",
                3 => "/relatedContent/appearsOn",
                4 => "/relatedContent/relatedArtists",
                5 => "/relatedContent/featuring",
                _ => return Err("Unknown Spotify artist section".into()),
            };
            let items = v.pointer(p).map(collect).unwrap_or_default();
            if index == 5 && items.is_empty() {
                let name = v
                    .pointer("/profile/name")
                    .and_then(Value::as_str)
                    .unwrap_or("");
                if !name.is_empty() {
                    return Ok(json!({"items": collect(&search(name)?["playlists"])}));
                }
            }
            Ok(json!({"items":items}))
        }
        "SPCatalogItemJSON" => {
            let kind = arg(args, 0)?;
            let u = uri(
                args,
                1,
                &["track", "album", "artist", "playlist", "show", "episode"],
            )?;
            let (op, pointer) = match kind {
                "album" => ("getAlbum", "/data/albumUnion"),
                "artist" => ("queryArtistOverview", "/data/artistUnion"),
                "playlist" => ("fetchPlaylist", "/data/playlistV2"),
                "track" => ("getTrack", "/data/trackUnion"),
                "show" => ("queryShowMetadataV2", "/data/podcastUnionV2"),
                "episode" => ("getEpisodeOrChapter", "/data/episodeUnionV2"),
                _ => {
                    return web::rest_get(&format!(
                        "https://api.spotify.com/v1/{kind}s/{}",
                        u.rsplit(':').next().unwrap()
                    ))
                }
            };
            let vars = match op {
                "queryShowMetadataV2" => json!({
                    "uri": u,
                    "includeContentCapabilityTrait": false,
                    "includeEpisodeContentRatingsV2": false,
                }),
                "getEpisodeOrChapter" => json!({
                    "uri": u,
                    "includeEpisodeContentRatingsV2": false,
                }),
                _ => json!({
                    "uri": u,
                    "offset": 0,
                    "limit": 50,
                    "enableWatchFeedEntrypoint": false,
                    "locale": "",
                    "preReleaseV2": true,
                    "includePrerelease": true,
                }),
            };
            entity(&at(&web::query(op, vars)?, pointer)?)
                .ok_or_else(|| "Spotify item metadata unavailable".into())
        }
        "DZSearchJSON" | "DZSearchPodcastsJSON" => search(arg(args, 0)?),
        "DZHistoryRecentJSON" => recent(),
        "DZTrackMixJSON" | "DZArtistMixJSON" => Ok(mark(
            radio(&uri(args, 0, &["track", "artist"])?)?,
            name,
            args,
        )),
        "DZFlowConfigsJSON" => flow_options(),
        "DZFlowJSON" | "DZFlowForConfigJSON" | "DZFlowAppendJSON" => {
            flow_queue::start(args.first().and_then(Value::as_str).unwrap_or("default"))
        }
        "DZFlowTunerJSON" | "DZFlowConfigTunerJSON" => {
            let key = args.first().and_then(Value::as_str).unwrap_or("default");
            Ok(super::flow::Selection::parse(key)?.tuner(key))
        }
        "DZSaveFlowTunerJSON" | "DZFlowConfigSaveJSON" => {
            super::flow::save(arg(args, 0)?, arg(args, 1)?)
        }
        "SPStyleMixJSON" => Ok(mark(style_mix(arg(args, 0)?)?, name, args)),
        "StyleMixGenres" => {
            let region = args
                .first()
                .and_then(Value::as_u64)
                .ok_or("Choose a language")? as usize;
            crate::style_mix::discover_genres(
                region,
                |selection| {
                    let query = format!(
                        "{} {} Mix",
                        crate::style_mix::REGIONS[selection.region].1[0],
                        crate::style_mix::GENRES[selection.genre].0
                    );
                    let value = search(&query)?;
                    Ok(collect(&value["playlists"])
                        .iter()
                        .any(|candidate| matches_style(candidate, selection)))
                },
                || true,
            )
        }
        "RustPodcastLibraryJSON" if args.is_empty() => {
            library("Podcasts & Shows").map(|value| json!({"podcasts":value["items"]}))
        }
        "DZAlbumLibraryJSON" | "DZPlaylistLibraryJSON" | "RustPodcastLibraryJSON" => {
            let u = uri(args, 0, &["album", "playlist", "show"])?;
            let save = match args.get(1) {
                Some(Value::Bool(b)) => *b,
                Some(Value::Number(n)) => n.as_i64().ok_or("Invalid library action")? != 0,
                _ => return Err("Missing library action".into()),
            };
            mutate_library(&u, save)
        }
        "DZAddFavorite" | "DZRemoveFavorite" | "DZDeletePlaylist" => mutate_library(
            &uri(args, 0, &["track", "episode", "playlist"])?,
            name == "DZAddFavorite",
        ),
        "DZAddToPlaylist" => {
            let v = web::mutation(
                "addToPlaylist",
                json!({"playlistUri":uri(args,0,&["playlist"])?,"playlistItemUris":[uri(args,1,&["track","episode"])?],"newPosition":{"moveType":"BOTTOM_OF_PLAYLIST","fromUid":null}}),
            )?;
            confirmed(&v)?;
            Ok(json!(1))
        }
        "DZRemoveFromPlaylist" => {
            let u = uri(args, 0, &["playlist"])?;
            let target = uri(args, 1, &["track", "episode"])?;
            let mut offset = 0;
            let mut uids = Vec::new();
            loop {
                let v = playlist(&u, offset)?;
                for t in v["items"].as_array().into_iter().flatten() {
                    if t["uri"] == target {
                        if let Some(uid) = t["uid"].as_str() {
                            uids.push(uid.to_owned());
                        }
                    }
                }
                if v["next"].is_null() {
                    break;
                }
                offset += 100;
                if offset >= 10_000 {
                    return Err(
                        "Playlist is too large to safely resolve all removal positions".into(),
                    );
                }
            }
            if uids.is_empty() {
                return Err("Track was not found in the Spotify playlist".into());
            }
            let v = web::mutation("removeFromPlaylist", json!({"playlistUri":u,"uids":uids}))?;
            confirmed(&v)?;
            Ok(json!(1))
        }
        "DZRenamePlaylist" | "DZUpdatePlaylistJSON" => {
            let u = uri(args, 0, &["playlist"])?;
            let title = arg(args, 1)?.trim();
            if title.is_empty() || title.len() > 300 {
                return Err("Enter a playlist name".into());
            }
            let mut vars = json!({"playlistUri":u,"newName":title});
            if name == "DZUpdatePlaylistJSON" {
                vars["newDescription"] = json!(arg(args, 2)?);
            }
            let v = web::mutation("editPlaylistAttributes", vars)?;
            confirmed(&v)?;
            Ok(json!(1))
        }
        "DZCreatePlaylist" => {
            let name = arg(args, 0)?.trim();
            if name.is_empty() || name.len() > 300 {
                return Err("Enter a playlist name".into());
            }
            let v = web::rest_request(
                "POST",
                "https://api.spotify.com/v1/me/playlists",
                Some(&json!({"name":name,"public":false})),
            )?;
            let u = text(&v, &["uri"]);
            if u.starts_with("spotify:playlist:") {
                Ok(json!({"id":u,"uri":u}))
            } else {
                Err("Spotify did not return the created playlist URI".into())
            }
        }
        "DZPlaylistCoverJSON" => {
            let u = uri(args, 0, &["playlist"])?;
            let p = std::path::Path::new(arg(args, 1)?);
            if std::fs::metadata(p)
                .map_err(|_| "Playlist cover could not be read")?
                .len()
                > 256 * 1024
            {
                return Err("Spotify playlist cover must be at most 256 KB".into());
            }
            let bytes = std::fs::read(p).map_err(|_| "Playlist cover could not be read")?;
            if !bytes.starts_with(&[0xff, 0xd8, 0xff]) {
                return Err("Spotify requires a JPEG playlist cover".into());
            }
            web::rest_upload(
                &format!(
                    "https://api.spotify.com/v1/playlists/{}/images",
                    u.rsplit(':').next().unwrap()
                ),
                &bytes,
                "image/jpeg",
            )?;
            Ok(json!(1))
        }
        _ => unreachable!(),
    })())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn style_mix_playlist_matching_requires_both_country_and_genre() {
        let selected = crate::style_mix::Selection::new(0, 1).unwrap();
        assert!(matches_style(&json!({"name":"Turkish Rock Mix"}), selected));
        assert!(!matches_style(&json!({"name":"Turkish Pop Mix"}), selected));
        assert!(!matches_style(
            &json!({"name":"Japanese Rock Mix"}),
            selected
        ));
        assert!(!matches_style(&json!({"name":"Rock Mix"}), selected));
    }
    #[test]
    fn browse_category_wrappers_keep_real_card_artwork_and_all_items() {
        let items: Vec<_> = (0..67).map(|i| json!({
            "uri":format!("spotify:page:category{i}"),
            "content":{"__typename":"BrowseSectionContainerWrapper","data":{
                "__typename":"BrowseSectionContainer","data":{"cardRepresentation":{
                    "title":{"transformedLabel":format!("Category {i}")},
                    "artwork":{"sources":[{"url":format!("https://i.scdn.co/image/category{i}"),"width":300,"height":300}]}
                }}
            }}
        })).collect();
        let shelves = sections(
            &json!([{"data":{"title":{"transformedLabel":"Browse all"}},"sectionItems":{"items":items}}]),
        );
        assert_eq!(shelves[0]["items"].as_array().unwrap().len(), 67);
        for (i, item) in shelves[0]["items"].as_array().unwrap().iter().enumerate() {
            assert_eq!(
                item["images"][0]["url"],
                format!("https://i.scdn.co/image/category{i}")
            );
            assert_eq!(item["images"][0]["width"], 300);
        }
    }
    #[test]
    fn nested_playlist_wrappers_preserve_uri_uid_metrics_and_credits() {
        let v = json!({"items":[{"uid":"row1","itemV2":{"_uri":"spotify:track:1111111111111111111111","data":{"uri":"spotify:track:1111111111111111111111","name":"Song","playcount":"12345","duration":{"totalMilliseconds":192000},"artists":{"items":[{"uri":"spotify:artist:2222222222222222222222","profile":{"name":"Artist"}}]},"albumOfTrack":{"uri":"spotify:album:3333333333333333333333","name":"Album","coverArt":{"sources":[{"url":"https://i.scdn.co/image/example"}]}}}}}]});
        let rows = collect(&v);
        assert_eq!(rows.len(), 1);
        let t = &rows[0];
        assert_eq!(t["uid"], "row1");
        assert_eq!(t["playCount"], 12345);
        assert_eq!(t["duration_ms"], 192000);
        assert_eq!(t["artists"][0]["name"], "Artist");
        assert_eq!(t["album"]["type"], "album");
    }
    #[test]
    fn artist_top_track_keeps_cover_when_album_has_no_name() {
        let track = json!({"uri":"spotify:track:1111111111111111111111","name":"Song","albumOfTrack":{"uri":"spotify:album:3333333333333333333333","coverArt":{"sources":[{"url":"https://i.scdn.co/image/cover"}]}}});
        let normalized = entity(&track).unwrap();
        assert_eq!(
            normalized["album"]["images"][0]["url"],
            "https://i.scdn.co/image/cover"
        );
        assert_eq!(
            crate::catalog::entry(&normalized, "track").unwrap().artwork,
            "https://i.scdn.co/image/cover"
        );
    }
    #[test]
    fn partial_home_errors_keep_valid_shelves_and_supplied_titles() {
        let v = json!([{"data":{"title":{"transformedLabel":"Your real mix"}},"sectionItems":{"items":[{"content":{"__typename":"PlaylistResponseWrapper","data":{"uri":"spotify:playlist:1111111111111111111111","name":"Daily mix","images":{"items":[{"sources":[{"url":"https://i.scdn.co/image/mix"}]}]}}}}]}},{"data":{"__typename":"GenericError","title":{"text":"Unavailable"}},"sectionItems":{"items":[]}}]);
        let shelves = sections(&v);
        assert_eq!(shelves.len(), 2);
        assert_eq!(shelves[0]["title"], "Your real mix");
        assert_eq!(
            shelves[0]["items"][0]["images"][0]["url"],
            "https://i.scdn.co/image/mix"
        );
        assert!(!shelves[1]["error"].as_str().unwrap().is_empty());
    }
    #[test]
    fn artist_metrics_are_independent_and_errors_are_not_items() {
        let v = json!({"uri":"spotify:artist:1111111111111111111111","profile":{"name":"Artist"},"stats":{"monthlyListeners":900,"followers":100}});
        let e = entity(&v).unwrap();
        assert_eq!(e["monthlyListeners"], 900);
        assert_eq!(e["followers"], 100);
        assert!(e.get("playCount").is_none());
        assert!(entity(&json!({"__typename":"NotFound","message":"Private"})).is_none());
        assert!(
            confirmed(&json!({"data":{"addToLibrary":{"__typename":"GenericError"}}})).is_err()
        );
        assert!(
            confirmed(&json!({"data":{"addToLibrary":{"__typename":"AddToLibraryPayload"}}}))
                .is_ok()
        );
    }

    #[test]
    fn track_details_keep_release_credit_and_content_fields() {
        let v = json!({"uri":"spotify:track:1111111111111111111111","name":"Song",
            "trackNumber":3,"contentRating":{"label":"EXPLICIT"},
            "external_ids":{"isrc":"TEST123"},
            "albumOfTrack":{"uri":"spotify:album:2222222222222222222222","name":"Album",
                "date":{"isoString":"2011-03-21T00:00:00Z"}}});
        let e = entity(&v).unwrap();
        assert_eq!(e["track_number"], 3);
        assert_eq!(e["explicit"], true);
        assert_eq!(e["external_ids"]["isrc"], "TEST123");
        assert_eq!(e["release_date"], "2011-03-21T00:00:00Z");
    }

    #[test]
    fn podcast_episode_and_show_wrappers_unwrap_correctly() {
        let payload = json!({
            "items": [
                {
                    "entity": {
                        "__typename": "EpisodeResponseWrapper",
                        "data": {
                            "__typename": "Episode",
                            "uri": "spotify:episode:5x11111111111111111111",
                            "name": "Test Episode #123",
                            "description": "Episode description text",
                            "duration": { "totalMilliseconds": 3600000 },
                            "releaseDate": { "isoString": "2024-02-15T12:00:00Z" },
                            "coverArt": { "sources": [{ "url": "https://i.scdn.co/image/ep123", "width": 640, "height": 640 }] },
                            "podcastV2": {
                                "data": {
                                    "__typename": "Podcast",
                                    "uri": "spotify:show:7x22222222222222222222",
                                    "name": "The Great Show",
                                    "coverArt": { "sources": [{ "url": "https://i.scdn.co/image/show123", "width": 640, "height": 640 }] }
                                }
                            }
                        }
                    }
                }
            ]
        });
        let episodes = collect(&payload);
        assert_eq!(episodes.len(), 1);
        let ep = &episodes[0];
        assert_eq!(ep["uri"], "spotify:episode:5x11111111111111111111");
        assert_eq!(ep["name"], "Test Episode #123");
        assert_eq!(ep["type"], "episode");
        assert_eq!(ep["duration_ms"], 3600000);
        assert_eq!(ep["release_date"], "2024-02-15T12:00:00Z");
        assert_eq!(ep["images"][0]["url"], "https://i.scdn.co/image/ep123");
        assert_eq!(ep["show"]["name"], "The Great Show");
        assert_eq!(ep["show"]["uri"], "spotify:show:7x22222222222222222222");

        let show_payload = json!({
            "__typename": "Podcast",
            "uri": "spotify:show:7x22222222222222222222",
            "name": "The Great Show",
            "publisher": { "name": "Show Host" },
            "htmlDescription": "<p>A wonderful show</p>",
            "coverArt": { "sources": [{ "url": "https://i.scdn.co/image/show123", "width": 640, "height": 640 }] }
        });
        let show = entity(&show_payload).unwrap();
        assert_eq!(show["type"], "show");
        assert_eq!(show["name"], "The Great Show");
        assert_eq!(show["publisher"], "Show Host");
        assert_eq!(show["description"], "<p>A wonderful show</p>");
        assert_eq!(show["images"][0]["url"], "https://i.scdn.co/image/show123");
    }
}
