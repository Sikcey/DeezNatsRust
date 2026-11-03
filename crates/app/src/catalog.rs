mod artwork_queue;
mod artwork_resize;
mod cache;
mod deezer_shelves;
pub(crate) mod disk_cache;
pub(crate) mod musicbrainz;
mod spotify;
pub(crate) fn spotify_discovery_report() -> Result<serde_json::Value, String> {
    spotify::discovery_report()
}
pub(crate) fn shelf_has_more(page: &Page, title: &str) -> bool {
    if deezer_shelves::has_more(page, title) {
        return true;
    }
    page.raw
        .iter()
        .find(|(key, _)| key == &format!("spotify:shelf:{title}"))
        .is_some_and(|(_, state)| {
            if state["directory"] == true {
                state["complete"] != true
            } else {
                state["sections"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .any(|s| s["total"].as_u64().unwrap_or(0) > s["offset"].as_u64().unwrap_or(0))
            }
        })
}
pub(crate) fn catalogue_page(page: &Page, title: &str) -> Option<Page> {
    let section = page.sections.iter().find(|s| s.title == title)?.clone();
    let mut related = page.clone();
    related.title = title.into();
    related.subtitle.clear();
    related.entity = None;
    related.sections = vec![section];
    related.rows.clear();
    related.next.clear();
    related.error.clear();
    related.loading = false;
    related
        .raw
        .retain(|(key, _)| key != "discover" && key != "spotify:more-shelf");
    related.raw.push(("catalogue".into(), json!(true)));
    Some(related)
}
pub(crate) mod page_updates;
pub(crate) mod parallel;
pub(crate) mod session;
use crate::{engine::Engine, ui::View};
use serde_json::{json, Value};
use std::{
    collections::{HashMap, HashSet},
    sync::{
        atomic::{AtomicU64, Ordering},
        mpsc, Arc,
    },
};
/// Shared navigation capability; catalog IDs remain provider namespaced.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Provider {
    #[default]
    Deezer,
    Spotify,
    YouTube,
}
static ACTIVE_PROVIDER: AtomicU64 = AtomicU64::new(0);
pub(crate) fn spotify_style_mix(selection: &str) -> Result<Value, String> {
    spotify::style_mix(selection)
}
impl Provider {
    pub fn label(self) -> &'static str {
        match self {
            Self::Deezer => "Deezer",
            Self::Spotify => "Spotify",
            Self::YouTube => "YouTube Music",
        }
    }
    pub fn supports_downloads(self) -> bool {
        self == Self::Deezer
    }
    pub fn active() -> Self {
        match ACTIVE_PROVIDER.load(Ordering::Acquire) {
            1 => Self::Spotify,
            2 => Self::YouTube,
            _ => Self::Deezer,
        }
    }
    pub fn set_active(self) {
        ACTIVE_PROVIDER.store(
            match self {
                Self::Deezer => 0,
                Self::Spotify => 1,
                Self::YouTube => 2,
            },
            Ordering::Release,
        );
    }

    fn invoke(self, engine: &Engine, name: &str, args: &[Value]) -> Result<Value, String> {
        match self {
            Self::Deezer => engine.invoke(name, args),
            Self::Spotify => crate::spotify::catalog_invoke(name, args),
            Self::YouTube => crate::youtube::invoke(name, args),
        }
    }
    fn next(self, engine: &Engine, cursor: &str) -> Result<Value, String> {
        match self {
            Self::Deezer => public(engine, cursor),
            Self::Spotify => self.invoke(engine, "SPCatalogNextJSON", &[json!(cursor)]),
            Self::YouTube => crate::youtube::invoke("Next", &[json!(cursor)]),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Entry {
    pub id: String,
    pub title: String,
    pub subtitle: String,
    pub artwork: String,
    pub kind: String,
    pub duration: u64,
    pub album: String,
    pub artist_id: String,
    pub album_id: String,
    pub raw: Value,
}
impl Entry {
    pub fn library_key(&self) -> String {
        if matches!(self.kind.as_str(), "track" | "episode") {
            self.id.clone()
        } else {
            format!("{}:{}", self.kind, self.id)
        }
    }
    /// Metadata targets shared by clickable credits and native context menus.
    pub fn artist_entries(&self) -> Vec<Self> {
        let mut result = Vec::new();
        let mut seen = HashSet::new();
        for key in ["artists", "contributors", "artist"] {
            for artist in entries(&self.raw[key], "artist") {
                if !artist.id.is_empty()
                    && !artist.title.is_empty()
                    && seen.insert(artist.id.clone())
                {
                    result.push(artist);
                }
            }
        }
        if !self.artist_id.is_empty() && !seen.contains(&self.artist_id) {
            let title = if result.is_empty() {
                self.subtitle.clone()
            } else {
                string(&self.raw["artist"], &["name", "title"])
            };
            if !title.is_empty() {
                result.insert(
                    0,
                    Self {
                        id: self.artist_id.clone(),
                        title,
                        kind: "artist".into(),
                        ..Default::default()
                    },
                );
            }
        }
        result
    }
    pub fn artist_entry(&self) -> Option<Self> {
        self.artist_entries().into_iter().next()
    }
    pub fn album_entry(&self) -> Option<Self> {
        if self.album_id.is_empty() {
            return None;
        }
        let mut raw = self.raw.get("album").cloned().unwrap_or_else(|| json!({}));
        if !raw.is_object() {
            raw = json!({});
        }
        raw["id"] = json!(self.album_id);
        raw["kind"] = json!("album");
        if string(&raw, &["name", "title"]).is_empty() {
            raw["name"] = json!(self.album);
        }
        if picture(&raw).is_empty() && !self.artwork.is_empty() {
            raw["artworkUrl"] = json!(self.artwork);
        }
        raw["artistId"] = json!(self.artist_id);
        raw["artistLine"] = json!(self.subtitle);
        entry(&raw, "album")
    }
    pub fn episode_count(&self) -> Option<u64> {
        [
            "episodeCount",
            "nb_episodes",
            "numberOfEpisodes",
            "episodesCount",
        ]
        .iter()
        .find_map(|key| {
            self.raw
                .get(*key)
                .and_then(|v| v.as_u64().or_else(|| v.as_str()?.parse().ok()))
        })
        .or_else(|| {
            self.raw
                .pointer("/episodes/totalCount")
                .and_then(Value::as_u64)
        })
    }
    pub fn playable(&self) -> bool {
        matches!(self.kind.as_str(), "track" | "episode")
    }
    pub fn wire(&self) -> Value {
        let mut value = self.base_wire();
        if let Some(selection) = self.raw.get(crate::style_mix::MARKER) {
            value[crate::style_mix::MARKER] = selection.clone();
        }
        value
    }
    fn base_wire(&self) -> Value {
        json!({"id":self.id,"name":self.title,"artistLine":self.subtitle,"artistId":self.artist_id,"albumId":self.album_id,"albumName":self.album,"artworkUrl":self.artwork,"durationMs":self.duration,"kind":self.kind,"episodeCount":self.episode_count(),"description":self.raw.get("description").cloned().unwrap_or(Value::Null),"explicit":self.raw["explicit"].as_bool().or(self.raw["isExplicit"].as_bool()).unwrap_or(false),"artists":self.artist_entries().iter().map(|a|json!({"id":a.id,"name":a.title,"kind":"artist"})).collect::<Vec<_>>()})
    }
}
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Section {
    pub title: String,
    pub entries: Vec<Entry>,
    pub error: String,
}
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Page {
    pub title: String,
    pub subtitle: String,
    pub sections: Vec<Section>,
    pub rows: Vec<Entry>,
    pub entity: Option<Entry>,
    pub raw: Vec<(String, Value)>,
    pub next: String,
    pub loading: bool,
    pub error: String,
}
pub fn string(value: &Value, keys: &[&str]) -> String {
    for key in keys {
        if let Some(v) = value.get(*key) {
            if let Some(s) = v.as_str() {
                if !s.is_empty() {
                    return s.into();
                }
            } else if v.is_number() {
                return v.to_string();
            }
        }
    }
    String::new()
}
fn picture(value: &Value) -> String {
    let direct = string(
        value,
        &[
            "artworkUrl",
            "cover_xl",
            "picture_xl",
            "cover_big",
            "picture_big",
            "cover_medium",
            "picture_medium",
            "image",
            "picture",
            "cover",
        ],
    );
    if direct.starts_with("https://") {
        return direct;
    }
    for name in ["mosaicCover", "picture", "cover", "image"] {
        if let Some(urls) = value
            .get(name)
            .and_then(|v| v.get("urls"))
            .and_then(Value::as_array)
        {
            for url in urls {
                if let Some(url) = url.as_str() {
                    if url.starts_with("https://") {
                        return url.into();
                    }
                }
            }
        }
    }
    for item in value.get("image_linked_item").into_iter().chain(
        value
            .get("pictures")
            .and_then(Value::as_array)
            .into_iter()
            .flatten(),
    ) {
        let hash = string(item, &["md5"]);
        let kind = string(item, &["type"]);
        if hash.len() == 32
            && hash.bytes().all(|b| b.is_ascii_hexdigit())
            && matches!(
                kind.as_str(),
                "playlist" | "cover" | "artist" | "talk" | "misc"
            )
        {
            return format!(
                "https://cdn-images.dzcdn.net/images/{kind}/{hash}/320x320-000000-80-0-0.jpg"
            );
        }
    }
    for name in ["album", "podcast"] {
        if let Some(v) = value.get(name) {
            let image = picture(v);
            if !image.is_empty() {
                return image;
            }
        }
    }
    String::new()
}
pub fn entry(value: &Value, fallback: &str) -> Option<Entry> {
    if spotify::is_item(value) {
        return spotify::entry(value, fallback);
    }
    let mut typename = string(value, &["__typename", "type", "kind"]).to_lowercase();
    if typename.is_empty() {
        typename = string(&value["data"], &["__TYPE__"]).to_lowercase();
    }
    if value.get("__typename").is_some()
        && !matches!(
            typename.as_str(),
            "track"
                | "album"
                | "artist"
                | "playlist"
                | "podcast"
                | "episode"
                | "smarttracklist"
                | "flow"
                | "channel"
        )
    {
        return None;
    }
    let mut id = string(value, &["id", "trackId"]);
    if typename == "channel" {
        let target = string(value, &["target"]);
        if target.starts_with("/channels/") {
            id = format!("web:{target}");
        } else {
            let url = string(&value["url"], &["deepLink", "webUrl"]);
            if !url.is_empty() {
                id = url;
            } else if !string(value, &["slug"]).is_empty() {
                id = format!(
                    "deezer://www.deezer.com/channels/{}",
                    string(value, &["slug"])
                );
            }
        }
    }
    if id.is_empty() {
        return None;
    }
    let mut title = string(value, &["title", "displayTitle", "name"]);
    if title.is_empty() {
        title = string(&value["data"], &["title", "name"]);
    }
    if title.is_empty() {
        return None;
    }
    let kind = match typename.as_str() {
        "smarttracklist" | "mix" => "mix",
        "podcast" | "show" => "podcast",
        "episode" => "episode",
        "track" | "song" => "track",
        "album" => "album",
        "artist" => "artist",
        "playlist" => "playlist",
        "flow" => "flow",
        "channel" => "channel",
        _ => fallback,
    }
    .to_string();
    let mut subtitle = match kind.as_str() {
        "playlist" => {
            let mut owner = String::new();
            for key in ["owner", "creator", "user"] {
                owner = value
                    .get(key)
                    .and_then(Value::as_str)
                    .map(str::to_owned)
                    .unwrap_or_else(|| string(&value[key], &["name", "displayName", "title"]));
                if !owner.trim().is_empty() {
                    break;
                }
            }
            if owner.trim().is_empty() {
                string(value, &["artistLine", "ownerName", "creatorName"])
            } else {
                owner
            }
        }
        "podcast" => string(value, &["authorName", "author", "publisher", "artistLine"]),
        "track" | "episode" | "album" => string(value, &["artistLine", "podcastName", "artist"]),
        _ => string(
            value,
            &[
                "artistLine",
                "subTitle",
                "subtitle",
                "owner",
                "podcastName",
                "artist",
            ],
        ),
    };
    if kind == "artist" && subtitle.is_empty() {
        let fans = string(value, &["nb_fan", "fans", "followers"]);
        if !fans.is_empty() && fans != "0" {
            if let Ok(n) = fans.parse::<u64>() {
                subtitle = format!("{} followers", format_count(n));
            } else {
                subtitle = format!("{fans} followers");
            }
        }
    }
    if subtitle.is_empty() {
        for key in ["artist", "owner", "user", "creator"] {
            if let Some(artist) = value.get(key) {
                subtitle = string(artist, &["name", "title"]);
                if !subtitle.is_empty() {
                    break;
                }
            }
        }
    }
    if subtitle.is_empty() {
        let mut artists = Vec::new();
        if let Some(contributors) = value.get("contributors") {
            for artist in entries(contributors, "artist") {
                artists.push(artist.title);
            }
        }
        if let Some(items) = value.get("artists").and_then(Value::as_array) {
            artists.extend(items.iter().map(|v| string(v, &["name"])));
        }
        subtitle = artists.join(", ");
    }
    title = title.split_whitespace().collect::<Vec<_>>().join(" ");
    subtitle = subtitle.split_whitespace().collect::<Vec<_>>().join(" ");
    if kind == "episode" && subtitle.is_empty() {
        subtitle = string(&value["podcast"], &["displayTitle", "title", "name"]);
    }
    let duration = value
        .get("durationMs")
        .and_then(Value::as_u64)
        .unwrap_or_else(|| {
            let duration = value.get("duration").and_then(Value::as_u64).unwrap_or(0);
            if duration >= 10000
                && (value.get("contributors").is_some() || value.get("__typename").is_some())
            {
                duration
            } else {
                duration * 1000
            }
        });
    let album = value.get("album").cloned().unwrap_or(Value::Null);
    Some(Entry {
        id,
        title,
        subtitle,
        artwork: picture(value),
        kind,
        duration,
        album: {
            let a = string(value, &["albumName"]);
            if a.is_empty() {
                album
                    .as_str()
                    .map(str::to_owned)
                    .unwrap_or_else(|| string(&album, &["title", "displayTitle", "name"]))
            } else {
                a
            }
        },
        artist_id: {
            let a = string(value, &["artistId"]);
            if a.is_empty() {
                let id = string(&value["artist"], &["id"]);
                if id.is_empty() {
                    value
                        .get("contributors")
                        .and_then(|v| entries(v, "artist").into_iter().next())
                        .map(|e| e.id)
                        .unwrap_or_default()
                } else {
                    id
                }
            } else {
                a
            }
        },
        album_id: {
            let a = string(value, &["albumId"]);
            if a.is_empty() {
                string(&album, &["id"])
            } else {
                a
            }
        },
        raw: compact_entry_metadata(value),
    })
}
// Bulk collection rows already have their own Entry records. Retain all display
// and artist metadata, but not a second copy of those entire collections.
fn compact_entry_metadata(value: &Value) -> Value {
    let Some(object) = value.as_object() else {
        return value.clone();
    };
    let mut result = serde_json::Map::new();
    for (key, value) in object {
        if matches!(
            key.as_str(),
            "tracks" | "episodes" | "albums" | "playlists" | "sections"
        ) {
            if let Some(metadata) = value.as_object() {
                let kept: serde_json::Map<String, Value> = metadata
                    .iter()
                    .filter(|(key, _)| {
                        !matches!(key.as_str(), "data" | "items" | "edges" | "nodes")
                    })
                    .map(|(key, value)| (key.clone(), value.clone()))
                    .collect();
                if !kept.is_empty() {
                    result.insert(key.clone(), Value::Object(kept));
                }
            }
        } else {
            result.insert(key.clone(), value.clone());
        }
    }
    Value::Object(result)
}
pub fn format_count(n: u64) -> String {
    let s = n.to_string();
    let mut result = String::new();
    let len = s.len();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (len - i) % 3 == 0 {
            result.push('.');
        }
        result.push(c);
    }
    result
}

pub fn entries(value: &Value, fallback: &str) -> Vec<Entry> {
    fn visit(
        v: &Value,
        kind: &str,
        out: &mut Vec<Entry>,
        seen: &mut HashSet<String>,
        depth: usize,
    ) {
        if depth > 32 || out.len() >= 2000 {
            return;
        }
        if let Some(items) = v.as_array() {
            for item in items {
                visit(item, kind, out, seen, depth + 1);
            }
            return;
        }
        if let Some(object) = v.as_object() {
            if let Some(item) = entry(v, kind) {
                if seen.insert(format!("{}:{}", item.kind, item.id)) {
                    out.push(item);
                }
                return;
            }
            for (key, child) in object {
                let inferred = match key.as_str() {
                    "tracks" | "topTracks" | "hotTracks" => "track",
                    "albums" | "topAlbums" | "newReleases" => "album",
                    "artists" | "recommendedArtists" => "artist",
                    "playlists" | "recentlyPlayed" | "madeForMe" | "artistPlaylists" => "playlist",
                    "episodes" => "episode",
                    "podcasts" | "suggestedPodcasts" => "podcast",
                    _ => kind,
                };
                if !matches!(
                    key.as_str(),
                    "pageInfo" | "errors" | "extensions" | "viewer" | "user"
                ) {
                    visit(child, inferred, out, seen, depth + 1);
                }
            }
        }
    }
    let mut result = Vec::new();
    visit(value, fallback, &mut result, &mut HashSet::new(), 0);
    result
}
// Collection metadata also has an id/title. Resolve its track connection before
// passing anything to the generic catalog walker, which otherwise stops there.
fn table_payload<'a>(value: &'a Value) -> Option<&'a Value> {
    for key in ["tracks", "episodes"] {
        if let Some(rows) = value.get(key) {
            return Some(rows);
        }
    }
    for key in ["smartTracklist", "playlist", "album"] {
        if let Some(collection) = value.get(key) {
            return Some(collection.get("tracks").unwrap_or(&Value::Null));
        }
    }
    value
        .get("data")
        .filter(|v| v.is_object())
        .and_then(table_payload)
}
pub(crate) fn table_entries(value: &Value, kind: &str) -> Vec<Entry> {
    entries(table_payload(value).unwrap_or(value), kind)
        .into_iter()
        .filter(Entry::playable)
        .collect()
}

// Collection responses may wrap metadata separately from their track connection.
fn inherit_collection_metadata(page: &mut Page, value: &Value) {
    let Some(entity) = page.entity.as_mut() else {
        return;
    };
    if !matches!(entity.kind.as_str(), "album" | "playlist") {
        return;
    }
    let metadata = value
        .get(&entity.kind)
        .or_else(|| value.get("data").and_then(|data| data.get(&entity.kind)))
        .unwrap_or(value);
    if let Some(description) = metadata
        .get("description")
        .or_else(|| metadata.get("DESCRIPTION"))
        .and_then(Value::as_str)
    {
        if !entity.raw.is_object() {
            entity.raw = json!({});
        }
        entity.raw["description"] = json!(description);
    }
    for key in [
        "owner",
        "creator",
        "user",
        "creator_id",
        "owner_id",
        "USER_ID",
    ] {
        if let Some(value) = metadata.get(key) {
            if !entity.raw.is_object() {
                entity.raw = json!({});
            }
            entity.raw[key] = value.clone();
        }
    }
    let artwork = picture(metadata);
    if !artwork.is_empty() {
        entity.artwork = artwork;
    }
    for row in &mut page.rows {
        if row.kind != "track" {
            continue;
        }
        if row.artwork.is_empty() {
            row.artwork = entity.artwork.clone();
        }
        if row.album.is_empty() {
            row.album = entity.title.clone();
        }
        if row.album_id.is_empty() {
            row.album_id = entity.id.clone();
        }
    }
}

fn curated_entries(value: &Value, fallback: &str) -> Vec<Entry> {
    entries(value, fallback)
        .into_iter()
        .filter(|item| matches!(item.kind.as_str(), "playlist" | "album" | "mix" | "podcast"))
        .collect()
}

fn web_sections(value: &Value, directory: bool) -> Vec<Section> {
    value["sections"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|row| {
            let items = if directory {
                entries(&row["items"], "channel")
                    .into_iter()
                    .filter(|item| item.kind == "channel")
                    .collect()
            } else {
                curated_entries(&row["items"], "playlist")
            };
            (!items.is_empty()).then(|| Section {
                title: string(row, &["title"]),
                entries: items,
                ..Default::default()
            })
        })
        .collect()
}

impl Page {
    pub fn is_catalogue(&self) -> bool {
        self.is_discover()
            || self
                .entity
                .as_ref()
                .is_some_and(|item| item.kind == "channel")
            || self.raw.iter().any(|(key, _)| key == "catalogue")
    }
    pub fn is_discover(&self) -> bool {
        self.raw.iter().any(|(key, _)| key == "discover")
    }
}

pub fn discover_section_matches(section: &Section, filter: usize) -> bool {
    match filter {
        1 => section
            .entries
            .iter()
            .any(|item| matches!(item.kind.as_str(), "playlist" | "mix")),
        2 => section.title == "Genres",
        3 => section.title == "Categories",
        4 => section.title == "Podcasts by category",
        _ => true,
    }
}

fn discover(
    engine: &Engine,
    mut page: Page,
    publish: &mut dyn FnMut(&Page) -> Result<(), String>,
) -> Result<Page, String> {
    page.raw.push(("discover".into(), json!(true)));
    publish(&page)?;
    let user = engine.invoke("DZUserID", &[]).unwrap_or(Value::Null);
    let logged_in = user.as_u64().is_some_and(|id| id > 0)
        || user
            .as_str()
            .is_some_and(|id| id.parse::<u64>().is_ok_and(|id| id > 0));
    // Each source is independent. Slow personalized modules must not delay the
    // public genres and web categories that are already available.
    let mut jobs = Vec::new();
    if logged_in {
        for (module, title) in [
            (3, "Recommended playlists"),
            (1, "Made for you"),
            (6, "100% playlists from your artists"),
            (9, "Recommended for offline listening"),
        ] {
            jobs.push((0, module, "", title, "playlist"));
        }
    }
    for (path, title, kind) in [
        (
            "chart/0/playlists?limit=8",
            "Editorial playlists",
            "playlist",
        ),
        ("editorial/0/releases?limit=8", "New releases", "album"),
        (
            "editorial/0/selection?limit=8",
            "Editors' album picks",
            "album",
        ),
        ("editorial?limit=8", "Genres", "channel"),
    ] {
        jobs.push((1, 0, path, title, kind));
    }
    for (path, title) in [
        ("/channels/explore", "Categories"),
        ("/channels/podcasts", "Podcasts by category"),
    ] {
        jobs.push((2, 0, path, title, "channel"));
    }
    let personal_count = if logged_in { 4 } else { 0 };
    let mut order = Vec::new();
    for index in 0..4 {
        if index < personal_count {
            order.push(index);
        }
        order.push(personal_count + index);
        if index < 2 {
            order.push(personal_count + 4 + index);
        }
    }
    let mut results = vec![None; jobs.len()];
    let cursors = std::sync::Mutex::new(HashMap::new());
    parallel::load(
        jobs.len(),
        |slot| {
            let index = order[slot];
            let (source, module, path, title, kind) = jobs[index];
            let response = match source {
                0 => {
                    deezer_shelves::api(engine, "DZHomeModuleJSON", &[json!(module)]).map(|value| {
                        vec![Section {
                            title: title.into(),
                            entries: home_module_entries(&value, module as usize, kind)
                                .into_iter()
                                .filter(|item| matches!(item.kind.as_str(), "playlist" | "mix"))
                                .collect(),
                            ..Default::default()
                        }]
                    })
                }
                1 => deezer_shelves::public(engine, path).map(|value| {
                    cursors.lock().unwrap().insert(index, value.clone());
                    let mut rows = entries(&value, kind);
                    if kind == "channel" {
                        rows.retain(|item| item.id != "0");
                        for item in &mut rows {
                            item.id = format!("editorial:{}", item.id);
                        }
                    }
                    vec![Section {
                        title: title.into(),
                        entries: rows,
                        ..Default::default()
                    }]
                }),
                _ => deezer_shelves::web(engine, &format!("https://www.deezer.com/en{path}")).map(
                    |value| {
                        web_sections(&value, true)
                            .into_iter()
                            .map(|mut section| {
                                let genre = title == "Categories"
                                    && ["genre", "t\u{fc}r", "tarz"]
                                        .iter()
                                        .any(|word| section.title.to_lowercase().contains(word));
                                section.title = if genre { "Genres" } else { title }.into();
                                section
                            })
                            .collect()
                    },
                ),
            };
            (
                index,
                response.unwrap_or_else(|error| {
                    vec![Section {
                        title: title.into(),
                        error,
                        ..Default::default()
                    }]
                }),
            )
        },
        |_, (index, sections)| {
            if let Some(value) = cursors.lock().unwrap().remove(&index) {
                deezer_shelves::remember_public(&mut page, jobs[index].3, &value, jobs[index].4);
            }
            results[index] = Some(sections);
            page.sections.clear();
            for sections in results.iter().flatten() {
                for section in sections {
                    if let Some(existing) =
                        page.sections.iter_mut().find(|s| s.title == section.title)
                    {
                        for item in &section.entries {
                            if let Some(old) = existing.entries.iter_mut().find(|e| {
                                e.id == item.id || e.title.eq_ignore_ascii_case(&item.title)
                            }) {
                                if old.artwork.is_empty() {
                                    old.artwork = item.artwork.clone();
                                }
                            } else {
                                existing.entries.push(item.clone());
                            }
                        }
                    } else {
                        page.sections.push(section.clone());
                    }
                }
            }
            page.raw
                .push((format!("discover-source:{index}"), Value::Null));
            deezer_shelves::cover_directories(&mut page.sections, "");
            publish(&page)
        },
    )?;
    Ok(page)
}
fn artist_section_entries(value: &Value, index: usize) -> Vec<Entry> {
    let artist = value
        .pointer("/data/artist")
        .or_else(|| value.get("artist"));
    let connection = artist.and_then(|artist| {
        if index == 5 {
            artist.pointer("/playlists/items")
        } else {
            artist.get("items")
        }
    });
    let kind = match index {
        0 => "track",
        4 => "artist",
        5 => "playlist",
        _ => "album",
    };
    entries(connection.unwrap_or(&Value::Null), kind)
        .into_iter()
        .filter(|item| item.kind == kind)
        .collect()
}
fn update_artist_metadata(entity: &mut Entry, metadata: &Value) {
    let artwork = picture(metadata);
    if !artwork.is_empty() {
        entity.artwork = artwork;
    }
    let name = string(metadata, &["name", "displayTitle", "title"]);
    if !name.is_empty() {
        entity.title = name;
    }
    let fans = string(
        metadata,
        &["fansCount", "nbFans", "nb_fan", "fans", "followers"],
    );
    entity.kind = "artist".into();
    entity.raw = compact_entry_metadata(&metadata);
    if !fans.is_empty() {
        entity.raw["fans"] = json!(fans);
        entity.subtitle = if fans == "0" {
            String::new()
        } else {
            format!(
                "{} followers",
                fans.parse::<u64>().map(format_count).unwrap_or(fans)
            )
        };
    }
}
pub(crate) fn home_module_entries(value: &Value, index: usize, kind: &str) -> Vec<Entry> {
    let mut items: Vec<_> = entries(value, kind)
        .into_iter()
        .filter(|entry| !is_liked_songs(entry) && (index != 1 || entry.kind != "flow"))
        .collect();
    if kind == "album" {
        for item in &mut items {
            if item.kind == "track"
                && item.duration == 0
                && item.album_id.is_empty()
                && string(&item.raw, &["__typename", "type", "kind"]).is_empty()
            {
                item.kind = "album".into();
            }
        }
    }
    normalize_daily_mixes(&mut items);
    items
}
fn normalize_daily_mixes(items: &mut [Entry]) {
    let mut daily = 0;
    for item in items {
        if item.title.trim().eq_ignore_ascii_case("daily") {
            daily += 1;
            item.title = format!("Daily Mix {daily}");
            item.kind = "mix".into();
            item.subtitle.clear();
        } else if item.kind == "mix" {
            item.subtitle.clear();
        }
    }
}

fn is_liked_songs(entry: &Entry) -> bool {
    entry.kind == "playlist"
        && [
            "sevdi\u{011f}in \u{015f}ark\u{0131}lar",
            "sevdi\u{011f}in par\u{00e7}alar",
            "sevilen \u{015f}ark\u{0131}lar",
            "favori par\u{00e7}alar",
            "liked songs",
            "favorite tracks",
            "favourite tracks",
            "coups de c\u{0153}ur",
            "coups de coeur",
        ]
        .contains(&entry.title.trim().to_lowercase().as_str())
}
pub fn web_state(bytes: &[u8]) -> Result<Value, String> {
    let html = std::str::from_utf8(bytes).map_err(|e| e.to_string())?;
    let marker = "window.__DZR_APP_STATE__";
    let start = html
        .find(marker)
        .ok_or("Deezer web page contains no catalog state")?
        + marker.len();
    let remaining = &html[start..];
    let start = remaining.find('=').ok_or("Invalid Deezer web state")? + 1;
    let mut stream =
        serde_json::Deserializer::from_str(remaining[start..].trim_start()).into_iter::<Value>();
    stream
        .next()
        .ok_or("Empty Deezer web state")?
        .map_err(|e| e.to_string())
}
fn public_cache() -> &'static cache::Cache {
    static CACHE: std::sync::OnceLock<cache::Cache> = std::sync::OnceLock::new();
    CACHE.get_or_init(|| cache::Cache::new(32 * 1024 * 1024))
}
pub(crate) fn expire_public_cache() {
    public_cache().expire();
}
fn web_catalog(engine: &Engine, url: &str) -> Result<Arc<Value>, String> {
    public_cache().refresh(
        url,
        std::time::Duration::from_secs(120),
        || engine.fetch(url),
        web_state,
    )
}
pub(crate) fn public(engine: &Engine, path: &str) -> Result<Value, String> {
    let url = if path.starts_with("https://api.deezer.com/") {
        path.to_string()
    } else {
        format!("https://api.deezer.com/{}", path.trim_start_matches('/'))
    };
    let value = public_cache().refresh(
        &url,
        std::time::Duration::from_secs(120),
        || engine.fetch(&url),
        decode_public_json,
    )?;
    Ok((*value).clone())
}
pub(crate) fn public_json(
    url: &str,
    fetch: impl FnOnce() -> Result<Vec<u8>, String>,
) -> Result<Arc<Value>, String> {
    public_cache().load(
        url,
        std::time::Duration::from_secs(120),
        fetch,
        decode_public_json,
    )
}
fn decode_public_json(bytes: &[u8]) -> Result<Value, String> {
    let value: Value = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
    if let Some(error) = value.get("error") {
        return Err(format!("Deezer: {}", string(error, &["message", "type"])));
    }
    Ok(value)
}

pub(super) fn flow_request(entity: Option<&Entry>, query: &str) -> (&'static str, Vec<Value>) {
    if let Some(track) = entity.filter(|entry| entry.kind == "track") {
        ("DZTrackMixJSON", vec![json!(track.id)])
    } else if query.is_empty() || query == "default" {
        ("DZFlowJSON", Vec::new())
    } else {
        ("DZFlowForConfigJSON", vec![json!(query)])
    }
}

/// Load the full provider library for the inline playlist submenu, independent
/// of route/page-cache residency. Repeated cursors terminate malformed paging.
fn playlist_access_cache() -> &'static std::sync::Mutex<HashMap<String, Value>> {
    static CACHE: std::sync::OnceLock<std::sync::Mutex<HashMap<String, Value>>> =
        std::sync::OnceLock::new();
    CACHE.get_or_init(Default::default)
}

pub(crate) fn restore_playlist_owner(item: &mut Entry, account: &Value) {
    if item.kind != "playlist" {
        return;
    }
    let key = format!("{}:{}", string(account, &["userId"]), item.id);
    if let Some(access) = playlist_access_cache().lock().unwrap().get(&key) {
        item.raw["owner"] = access["owner"].clone();
    }
}

fn hydrate_deezer_playlists(engine: &Engine, value: Value) -> Result<Value, String> {
    let account = engine.invoke("DZAccountJSON", &[]).unwrap_or(Value::Null);
    let user = string(&account, &["userId"]);
    hydrate_deezer_playlists_with(value, &user, |id| {
        engine.invoke("DZPlaylistAccessJSON", &[json!(id)])
    })
}

fn hydrate_deezer_playlists_with(
    value: Value,
    user: &str,
    fetch: impl Fn(&str) -> Result<Value, String> + Sync,
) -> Result<Value, String> {
    let mut lists = entries(&value, "playlist");
    let mut updates = Vec::new();
    parallel::load(
        lists.len(),
        |index| fetch(&lists[index].id),
        |index, result| {
            let result = result?;
            let access = result
                .pointer("/data/playlist")
                .or_else(|| result.get("playlist"))
                .filter(|v| v.is_object())
                .ok_or("Deezer did not return playlist ownership and library status")?;
            updates.push((index, access.clone()));
            Ok(())
        },
    )?;
    for (index, access) in updates {
        let item = &mut lists[index];
        item.raw["owner"] = access["owner"].clone();
        item.raw["isFavorite"] = access["isFavorite"].clone();
        playlist_access_cache()
            .lock()
            .unwrap()
            .insert(format!("{user}:{}", item.id), access);
    }
    Ok(
        json!({"playlists": lists.into_iter().map(|item| item.raw).collect::<Vec<_>>(), "next": value["next"]}),
    )
}

pub fn editable_menu_playlists(
    engine: Option<&Engine>,
    provider: Provider,
    account: &Value,
) -> Result<Vec<Entry>, String> {
    if provider == Provider::YouTube {
        return crate::youtube::menu_playlists();
    }
    collect_editable_menu_playlists(account, |cursor| {
        let (name, args) = match cursor {
            None => ("DZPlaylistsJSON", Vec::new()),
            Some(cursor) => ("SPCatalogNextJSON", vec![json!(cursor)]),
        };
        match provider {
            Provider::Deezer => {
                let engine = engine.ok_or("Connect to Deezer to load playlists")?;
                match cursor {
                    None => engine
                        .invoke(name, &args)
                        .and_then(|value| hydrate_deezer_playlists(engine, value)),
                    Some(cursor) => public(engine, cursor),
                }
            }
            Provider::Spotify => crate::spotify::catalog_invoke(name, &args),
            Provider::YouTube => {
                crate::youtube::invoke(if cursor.is_some() { "Next" } else { name }, &args)
            }
        }
    })
}

fn collect_editable_menu_playlists(
    account: &Value,
    mut fetch: impl FnMut(Option<&str>) -> Result<Value, String>,
) -> Result<Vec<Entry>, String> {
    let mut value = fetch(None)?;
    let mut result = Vec::new();
    let mut ids = HashSet::new();
    let mut cursors = HashSet::new();
    loop {
        for entry in entries(&value, "playlist") {
            if crate::playlist::can_edit(&entry, account) && ids.insert(entry.id.clone()) {
                result.push(entry);
            }
        }
        let next = string(&value, &["next"]);
        if next.is_empty() || !cursors.insert(next.clone()) {
            break;
        }
        value = fetch(Some(&next))?;
    }
    Ok(result)
}

fn saved_library_entries(
    engine: Option<&Engine>,
    provider: Provider,
) -> Result<Vec<Entry>, String> {
    if provider == Provider::YouTube {
        return crate::youtube::saved_library_entries();
    }
    let mut saved = Vec::new();
    let mut last_error = None;
    for (name, kind) in [
        ("DZFavoritesJSON", "track"),
        ("DZFavoriteAlbumsJSON", "album"),
        ("DZPlaylistsJSON", "playlist"),
    ] {
        let mut cursor = None;
        let mut cursors = HashSet::new();
        loop {
            let result = match (provider, cursor.as_deref()) {
                (Provider::Deezer, None) => engine
                    .ok_or_else(|| "Connect to Deezer first".to_string())
                    .and_then(|engine| engine.invoke(name, &[])),
                (Provider::Deezer, Some(cursor)) => engine
                    .ok_or_else(|| "Connect to Deezer first".to_string())
                    .and_then(|engine| public(engine, cursor)),
                (Provider::Spotify, None) => crate::spotify::catalog_invoke(name, &[]),
                (Provider::Spotify, Some(cursor)) => {
                    crate::spotify::catalog_invoke("SPCatalogNextJSON", &[json!(cursor)])
                }
                _ => unreachable!(),
            };
            match result {
                Ok(value) => {
                    let value = if provider == Provider::Deezer && kind == "playlist" {
                        hydrate_deezer_playlists(engine.unwrap(), value)?
                    } else {
                        value
                    };
                    saved.extend(
                        entries(&value, kind)
                            .into_iter()
                            .filter(|item| item.raw["isFavorite"] != false),
                    );
                    let next = string(&value, &["next"]);
                    if next.is_empty() || !cursors.insert(next.clone()) {
                        break;
                    }
                    cursor = Some(next);
                }
                Err(error) => {
                    last_error = Some(error);
                    break;
                }
            }
        }
    }
    if saved.is_empty() && last_error.is_some() {
        Err(last_error.unwrap())
    } else {
        Ok(saved)
    }
}

#[cfg(test)]
mod menu_playlist_tests {
    use super::*;
    #[test]
    fn playlist_copy_updates_only_verified_tracks_and_rejects_incomplete_success() {
        let tracks = entries(
            &json!({"tracks":[{"id":"1","title":"One"},{"id":"2","title":"Two"}]}),
            "track",
        );
        let mut applied = Vec::new();
        let result =
            confirm_playlist_copy(&tracks, &json!({"confirmed":["1"],"failure":""}), |track| {
                applied.push(track.id.clone())
            });
        assert_eq!(applied, vec!["1"]);
        assert!(result.unwrap_err().contains("1/2"));
        applied.clear();
        let result = confirm_playlist_copy(
            &tracks,
            &json!({"confirmed":["2"],"failure":"Rate limited"}),
            |track| applied.push(track.id.clone()),
        );
        assert_eq!(applied, vec!["2"]);
        assert!(result.unwrap_err().contains("Rate limited"));
        assert!(confirm_playlist_copy(
            &tracks,
            &json!({"confirmed":["1","2"],"failure":""}),
            |_| {}
        )
        .is_ok());
    }
    #[test]
    fn collection_copy_follows_nested_track_pages_past_sixty_tracks() {
        let tracks = |start: usize, end: usize| {
            (start..end)
                .map(|id| json!({"id":id.to_string(),"title":format!("Track {id}"),"type":"track"}))
                .collect::<Vec<_>>()
        };
        let first = json!({"playlist":{"id":"source","title":"Source","tracks":{"data":tracks(0,60),"next":"page2"}}});
        let mut calls = Vec::new();
        let copied = collect_collection_tracks(first, |cursor| {
            calls.push(cursor.to_owned());
            Ok(match cursor {
                "page2" => {
                    json!({"data":{"playlist":{"tracks":{"data":tracks(60,120),"next":"page3"}}}})
                }
                "page3" => json!({"data":tracks(120,127),"next":null}),
                _ => panic!("unexpected page"),
            })
        })
        .unwrap();
        assert_eq!(calls, vec!["page2", "page3"]);
        assert_eq!(copied.len(), 127);
        assert_eq!(
            copied
                .iter()
                .map(|track| track.id.clone())
                .collect::<Vec<_>>(),
            (0..127).map(|id| id.to_string()).collect::<Vec<_>>()
        );
    }

    #[test]
    fn collection_copy_does_not_accept_failed_or_repeated_pages_as_a_complete_list() {
        let first = json!({"tracks":{"data":[{"id":"1","title":"One"}],"next":"page2"}});
        let failed = collect_collection_tracks(first.clone(), |_| Err("Page failed".into()));
        assert_eq!(failed.unwrap_err(), "Page failed");
        let repeated = collect_collection_tracks(first.clone(), |_| Ok(first.clone()));
        assert!(repeated.unwrap_err().contains("repeated a playlist page"));
    }

    #[test]
    fn collection_copy_preserves_top_level_pagination_and_track_order() {
        let first = json!({"tracks":[{"id":"1","title":"One"}],"next":"page2"});
        let copied = collect_collection_tracks(first, |cursor| {
            assert_eq!(cursor, "page2");
            Ok(json!({"tracks":[{"id":"2","title":"Two"}],"next":null}))
        })
        .unwrap();
        assert_eq!(
            copied
                .iter()
                .map(|track| track.id.as_str())
                .collect::<Vec<_>>(),
            vec!["1", "2"]
        );
    }
    #[test]
    fn native_playlist_access_restores_ownership_and_membership() {
        let value = json!({"playlists":[
            {"id":"owned-menu-fixture", "name":"Mine", "owner":""},
            {"id":"foreign-menu-fixture", "name":"Saved", "owner":""}
        ]});
        let value = hydrate_deezer_playlists_with(value, "menu-fixture-user", |id| {
            Ok(json!({"data":{"playlist":{
                "id":id, "owner":{"id":if id == "owned-menu-fixture" {"menu-fixture-user"} else {"someone-else"}},
                "isFavorite":id == "foreign-menu-fixture"
            }}}))
        }).unwrap();
        let account = json!({"userId":"menu-fixture-user"});
        let lists = collect_editable_menu_playlists(&account, |_| Ok(value.clone())).unwrap();
        assert_eq!(lists.len(), 1);
        assert_eq!(lists[0].id, "owned-menu-fixture");
        let saved: Vec<_> = entries(&value, "playlist")
            .into_iter()
            .filter(|item| item.raw["isFavorite"] != false)
            .collect();
        assert_eq!(saved.len(), 1);
        assert_eq!(saved[0].id, "foreign-menu-fixture");
        let mut card = entry(
            &json!({"id":"owned-menu-fixture", "title":"Mine"}),
            "playlist",
        )
        .unwrap();
        restore_playlist_owner(&mut card, &account);
        assert!(crate::playlist::can_edit(&card, &account));
        let mut other_account_card = entry(
            &json!({"id":"owned-menu-fixture", "title":"Mine"}),
            "playlist",
        )
        .unwrap();
        restore_playlist_owner(&mut other_account_card, &json!({"userId":"other-account"}));
        assert!(!crate::playlist::can_edit(
            &other_account_card,
            &json!({"userId":"other-account"})
        ));
    }
    #[test]
    fn playlist_access_failure_is_reported_instead_of_an_empty_editable_list() {
        let result = hydrate_deezer_playlists_with(
            json!({"playlists":[{"id":"1","name":"Mine"}]}),
            "me",
            |_| Err("Access request failed".into()),
        );
        assert_eq!(result.unwrap_err(), "Access request failed");
    }

    #[test]
    fn song_radio_uses_the_track_seed_instead_of_personal_flow() {
        for id in ["42", "spotify:track:seed", "youtube:track:seed"] {
            let track = Entry {
                id: id.into(),
                kind: "track".into(),
                ..Default::default()
            };
            let (name, args) = flow_request(Some(&track), "default");
            assert_eq!(name, "DZTrackMixJSON");
            assert_eq!(args, vec![json!(id)]);
        }
        assert_eq!(flow_request(None, "default"), ("DZFlowJSON", Vec::new()));
        assert_eq!(
            flow_request(None, "focus"),
            ("DZFlowForConfigJSON", vec![json!("focus")])
        );
    }

    #[test]
    fn saved_collection_membership_does_not_collide_with_track_ids() {
        let item = |kind: &str| Entry {
            id: "42".into(),
            kind: kind.into(),
            ..Default::default()
        };
        let mut saved = HashSet::new();
        saved.insert(item("album").library_key());
        assert!(saved.contains(&item("album").library_key()));
        assert!(!saved.contains(&item("track").library_key()));
        assert!(!saved.contains(&item("playlist").library_key()));
    }

    #[test]
    fn selected_tracks_are_loaded_without_a_network_collection_lookup() {
        let entries: Vec<Entry> = ["42", "43", "44"]
            .iter()
            .map(|id| Entry {
                id: (*id).into(),
                kind: "track".into(),
                title: (*id).into(),
                ..Default::default()
            })
            .collect();
        let selection = Entry {
            kind: "selection".into(),
            raw: json!({"entries":entries}),
            ..Default::default()
        };
        for provider in [Provider::Deezer, Provider::Spotify, Provider::YouTube] {
            assert_eq!(
                collection_tracks(None, provider, &selection)
                    .unwrap()
                    .iter()
                    .map(|entry| entry.id.as_str())
                    .collect::<Vec<_>>(),
                vec!["42", "43", "44"]
            );
        }
    }

    #[test]
    fn loads_all_pages_deduplicates_and_excludes_other_owners() {
        let mut calls = 0;
        let lists = collect_editable_menu_playlists(&json!({"id":"me"}), |cursor| {
            calls += 1;
            let (start, end, next) = match cursor {
                None => (0, 75, "page2"),
                Some("page2") => (75, 150, "page3"),
                Some("page3") => (150, 153, "page3"),
                _ => panic!("unexpected cursor"),
            };
            let mut data: Vec<Value> = (start..end)
                .map(|id| {
                    json!({
                        "id":id.to_string(), "title":format!("List {id}"), "creator":{"id":"me"}
                    })
                })
                .collect();
            data.push(json!({"id":"foreign", "title":"Someone else's", "creator":{"id":"other"}}));
            data.push(json!({"id":"0", "title":"Duplicate", "creator":{"id":"me"}}));
            Ok(json!({"data":data,"next":next}))
        })
        .unwrap();
        assert_eq!(calls, 3);
        assert_eq!(lists.len(), 153);
        assert_eq!(lists.last().unwrap().id, "152");
    }

    #[test]
    fn reports_pagination_failure_instead_of_claiming_a_complete_library() {
        let result = collect_editable_menu_playlists(&json!({"id":"me"}), |cursor| {
            if cursor.is_none() {
                Ok(json!({"data":[],"next":"page2"}))
            } else {
                Err("Library request failed".into())
            }
        });
        assert_eq!(result.unwrap_err(), "Library request failed");
    }
}

#[derive(Clone)]
pub enum CollectionAction {
    Queue,
    Play,
    Download,
    Add(String),
    CreateDetails(String, String, Option<Vec<u8>>, Option<bool>),
    Remove(String),
    Favorites,
}

fn collection_tracks(
    engine: Option<&Engine>,
    provider: Provider,
    item: &Entry,
) -> Result<Vec<Entry>, String> {
    if item.kind == "selection" {
        return serde_json::from_value(item.raw["entries"].clone())
            .map_err(|error| error.to_string());
    }
    if provider == Provider::YouTube {
        return crate::youtube::collection_tracks(item);
    }
    let name = if item.kind == "mix" && provider == Provider::Deezer {
        "DZSmartMixJSON"
    } else if item.kind == "artist" {
        "DZArtistTopJSON"
    } else if item.kind == "album" {
        "DZAlbumTracksJSON"
    } else if provider == Provider::Deezer && item.kind == "playlist" {
        "DZPlaylistTracksStrictJSON"
    } else {
        "DZPlaylistTracksJSON"
    };
    let invoke = |name: &str, args: &[Value]| match provider {
        Provider::Deezer => engine
            .ok_or_else(|| "Connect to Deezer first".to_string())?
            .invoke(name, args),
        Provider::Spotify => crate::spotify::catalog_invoke(name, args),
        Provider::YouTube => crate::youtube::invoke(name, args),
    };
    let value = invoke(name, &[json!(item.id)])?;
    collect_collection_tracks(value, |next| match provider {
        Provider::Deezer => public(engine.ok_or("Connect to Deezer first")?, next),
        Provider::Spotify => invoke("SPCatalogNextJSON", &[json!(next)]),
        Provider::YouTube => invoke("Next", &[json!(next)]),
    })
}

fn collect_collection_tracks(
    mut value: Value,
    mut fetch_next: impl FnMut(&str) -> Result<Value, String>,
) -> Result<Vec<Entry>, String> {
    let mut tracks = Vec::new();
    let mut cursors = HashSet::new();
    loop {
        tracks.extend(
            table_entries(&value, "track")
                .into_iter()
                .filter(|e| e.playable()),
        );
        let mut next = string(&value, &["next"]);
        if next.is_empty() {
            next = table_payload(&value)
                .map(|tracks| string(tracks, &["next"]))
                .unwrap_or_default();
        }
        if next.is_empty() {
            break;
        }
        if !cursors.insert(next.clone()) {
            return Err(
                "The service repeated a playlist page; the collection was not copied".into(),
            );
        }
        value = fetch_next(&next)?;
    }
    if tracks.is_empty() {
        return Err("This collection has no playable tracks".into());
    }
    Ok(tracks)
}

fn process_collection(
    generation: u64,
    provider: Provider,
    engine: Option<&Engine>,
    item: Entry,
    action: CollectionAction,
    tx: &EventSender,
) {
    let tracks = collection_tracks(engine, provider, &item);
    if matches!(action, CollectionAction::Favorites) {
        let mut completed = Vec::new();
        let mut error = None;
        match tracks {
            Ok(tracks) => {
                for track in tracks {
                    let args = [json!(track.id)];
                    let result = match provider {
                        Provider::Deezer => engine
                            .ok_or_else(|| "Connect to Deezer first".to_string())
                            .and_then(|engine| engine.invoke("DZAddFavorite", &args)),
                        Provider::Spotify => crate::spotify::catalog_invoke("DZAddFavorite", &args),
                        Provider::YouTube => crate::youtube::invoke("DZAddFavorite", &args),
                    };
                    match result {
                        Ok(value) if value != false && value != 0 && !value.is_null() => {
                            completed.push(track)
                        }
                        Ok(_) => {
                            error = Some("The service rejected the favorites update".into());
                            break;
                        }
                        Err(message) => {
                            error = Some(message);
                            break;
                        }
                    }
                }
            }
            Err(message) => error = Some(message),
        }
        let _ = tx.send(Event::BatchFavorites(provider, completed, error));
        return;
    }
    if matches!(action, CollectionAction::Play) {
        let _ = tx.send(Event::CollectionPlay(provider, tracks));
        return;
    }
    if matches!(action, CollectionAction::Queue) {
        let _ = tx.send(Event::CollectionQueued(provider, tracks));
        return;
    }
    if matches!(action, CollectionAction::Download) {
        let _ = tx.send(Event::CollectionDownload(provider, item, tracks));
        return;
    }
    let mut mutation = match action {
        CollectionAction::Remove(id) => crate::playlist::Mutation::Remove(id, String::new()),
        CollectionAction::CreateDetails(title, description, cover, public) => {
            crate::playlist::Mutation::Save {
                id: String::new(),
                title,
                description: Some(description),
                cover,
                seed: None,
                public,
            }
        }
        CollectionAction::Add(id) => crate::playlist::Mutation::Add(id, String::new()),
        CollectionAction::Queue
        | CollectionAction::Play
        | CollectionAction::Download
        | CollectionAction::Favorites => unreachable!(),
    };
    let execute = |mutation: &mut crate::playlist::Mutation| match provider {
        Provider::Deezer => {
            crate::playlist::execute(engine.ok_or("Connect to Deezer first")?, mutation)
        }
        Provider::Spotify => crate::playlist::execute_spotify(mutation),
        Provider::YouTube => crate::youtube::edit_playlist(mutation),
    };
    let result = (|| {
        let tracks = tracks?;
        let id = match &mutation {
            crate::playlist::Mutation::Add(id, _) | crate::playlist::Mutation::Remove(id, _) => {
                id.clone()
            }
            _ => execute(&mut mutation)?.id,
        };
        if provider == Provider::Deezer
            && !matches!(mutation, crate::playlist::Mutation::Remove(..))
        {
            let ids: Vec<_> = tracks.iter().map(|track| track.id.as_str()).collect();
            let result = engine.ok_or("Connect to Deezer first")?.invoke(
                "DZCopyPlaylistTracksJSON",
                &[
                    json!(id),
                    json!(serde_json::to_string(&ids).map_err(|error| error.to_string())?),
                ],
            )?;
            confirm_playlist_copy(&tracks, &result, |track| {
                let _ = tx.send(Event::PlaylistTrackChanged(
                    provider,
                    id.clone(),
                    track.clone(),
                    true,
                ));
            })?;
            return Ok(crate::playlist::Outcome { id, deleted: false });
        }
        for (index, track) in tracks.iter().enumerate() {
            let removing = matches!(mutation, crate::playlist::Mutation::Remove(..));
            let mut change = if removing {
                crate::playlist::Mutation::Remove(id.clone(), track.id.clone())
            } else {
                crate::playlist::Mutation::Add(id.clone(), track.id.clone())
            };
            execute(&mut change)
                .map_err(|error| format!("Updated {index}/{} tracks: {error}", tracks.len()))?;
            let _ = tx.send(Event::PlaylistTrackChanged(
                provider,
                id.clone(),
                track.clone(),
                !removing,
            ));
        }
        Ok(crate::playlist::Outcome { id, deleted: false })
    })();
    let _ = tx.send(Event::Playlist(generation, provider, mutation, result));
}

fn confirm_playlist_copy(
    tracks: &[Entry],
    result: &Value,
    mut confirmed: impl FnMut(&Entry),
) -> Result<(), String> {
    let ids: HashSet<_> = result["confirmed"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect();
    let mut copied = 0;
    for track in tracks {
        if ids.contains(track.id.as_str()) {
            confirmed(track);
            copied += 1;
        }
    }
    let failure = string(result, &["failure"]);
    if !failure.is_empty() {
        return Err(format!(
            "Confirmed {copied}/{} tracks in destination: {failure}",
            tracks.len()
        ));
    }
    if copied != tracks.len() {
        return Err(format!(
            "Confirmed {copied}/{} tracks in destination; copy is incomplete",
            tracks.len()
        ));
    }
    Ok(())
}

#[derive(Clone)]
pub enum Job {
    LibraryState,
    Collection(Entry, CollectionAction),
    Route(View, Option<Entry>, String),
    More(Page),
    Api(String, Vec<Value>),
    Playlist(crate::playlist::Mutation),
    Lyrics(Entry, String, u32),
    LyricsWindow(Entry, Vec<Entry>, u32),
    TrackInfo(Entry),
}
pub struct Request {
    #[cfg(feature = "diagnostics")]
    pub view_tag: usize,
    pub generation: u64,
    pub provider: Provider,
    pub engine: Option<Arc<Engine>>,
    pub job: Job,
}
pub enum Event {
    PlaylistTrackChanged(Provider, String, Entry, bool),
    LibraryState(Provider, Result<Vec<Entry>, String>),
    BatchFavorites(Provider, Vec<Entry>, Option<String>),
    CollectionPlay(Provider, Result<Vec<Entry>, String>),
    CollectionDownload(Provider, Entry, Result<Vec<Entry>, String>),
    CollectionQueued(Provider, Result<Vec<Entry>, String>),
    Playlist(
        u64,
        Provider,
        crate::playlist::Mutation,
        Result<crate::playlist::Outcome, String>,
    ),
    Page(u64, Arc<Page>),
    Api(u64, Provider, String, Result<Value, String>),
    Lyrics(
        Provider,
        String,
        u32,
        Result<(String, Vec<crate::lyrics::LyricLine>), String>,
    ),
    TrackInfo(u64, Box<Entry>, Value, Value),
}
pub struct Worker {
    pub requests: mpsc::Sender<Request>,
    pub lyrics_requests: mpsc::SyncSender<Request>,
    pub updates: mpsc::Receiver<Event>,
    route_generation: Arc<AtomicU64>,
    pub session: Arc<session::Store>,
}
#[derive(Clone)]
pub(crate) struct EventSender {
    tx: mpsc::SyncSender<Event>,
    session: Arc<session::Store>,
}
impl EventSender {
    pub(crate) fn send(&self, event: Event) -> Result<(), mpsc::SendError<Event>> {
        let event = match event {
            Event::Page(generation, page) => {
                Event::Page(generation, self.session.apply(generation, page))
            }
            other => other,
        };
        self.tx.send(event)
    }
    fn resume(&self, generation: u64) -> Option<Arc<Page>> {
        let page = self.session.resume(generation)?;
        let mut displayed = (*page).clone();
        displayed.loading = true;
        displayed.error.clear();
        let _ = self.tx.send(Event::Page(generation, Arc::new(displayed)));
        Some(page)
    }
}
impl Worker {
    /// API browsing does not depend on a native Deezer engine or Soloist playback.
    pub fn request_spotify(&self, generation: u64, job: Job) -> Result<(), String> {
        #[cfg(feature = "diagnostics")]
        let view_tag = match &job {
            Job::Route(view, ..) => *view as usize + 1,
            _ => 0,
        };
        self.requests
            .send(Request {
                #[cfg(feature = "diagnostics")]
                view_tag,
                generation,
                provider: Provider::Spotify,
                engine: None,
                job,
            })
            .map_err(|_| "Catalog worker stopped".into())
    }
    pub fn set_generation(&self, generation: u64) {
        self.route_generation.store(generation, Ordering::Relaxed);
    }
    pub fn start() -> Self {
        // Navigation must never be dropped because the catalog worker is busy.
        // Obsolete routes are discarded before doing any network work.
        let (requests, rx) = mpsc::channel::<Request>();
        let (tx, updates) = mpsc::sync_channel(4);
        let session = Arc::new(session::Store::for_session());
        let tx = EventSender {
            tx,
            session: session.clone(),
        };
        let (lyrics_tx, lyrics_rx) = mpsc::sync_channel::<Request>(16);
        let lyrics_events = tx.clone();
        std::thread::spawn(move || {
            let mut pending = None;
            loop {
                let mut request = match pending.take().or_else(|| lyrics_rx.recv().ok()) {
                    Some(r) => r,
                    None => break,
                };
                while let Ok(newer) = lyrics_rx.try_recv() {
                    request = newer;
                }
                #[cfg(feature = "diagnostics")]
                let _page_scope = crate::memory_profile::page_scope(if request.view_tag == 0 {
                    0
                } else {
                    request.view_tag + 64
                });
                let (track, id, sources, neighbors) = match request.job {
                    Job::Lyrics(track, id, sources) => (track, id, sources, Vec::new()),
                    Job::LyricsWindow(track, neighbors, sources) => {
                        let id = track.id.clone();
                        (track, id, sources, neighbors)
                    }
                    _ => continue,
                };
                let result = match request.provider {
                    Provider::YouTube => crate::lyrics::fetch_spotify_lyrics(&track, &id, sources),
                    Provider::Spotify => crate::lyrics::fetch_spotify_lyrics(&track, &id, sources),
                    Provider::Deezer => request
                        .engine
                        .as_deref()
                        .ok_or_else(|| "Deezer engine unavailable".to_string())
                        .and_then(|engine| {
                            crate::lyrics::fetch_best_lyrics(engine, &track, &id, sources)
                        }),
                };
                if lyrics_events
                    .send(Event::Lyrics(request.provider, id, sources, result))
                    .is_err()
                {
                    break;
                }
                // One adjacent lookup at a time, only while no current-song request is waiting.
                for neighbor in neighbors {
                    if let Ok(newer) = lyrics_rx.try_recv() {
                        pending = Some(newer);
                        break;
                    }
                    if neighbor.kind == "track"
                        && !neighbor.id.is_empty()
                        && neighbor.id != track.id
                    {
                        if request.provider == Provider::Spotify {
                            let _ = crate::lyrics::fetch_spotify_lyrics(
                                &neighbor,
                                &neighbor.id,
                                sources,
                            );
                        } else if let Some(engine) = request.engine.as_deref() {
                            let _ = crate::lyrics::fetch_best_lyrics(
                                engine,
                                &neighbor,
                                &neighbor.id,
                                sources,
                            );
                        }
                    }
                }
            }
        });
        let lyrics_forward = lyrics_tx.clone();
        let route_generation = Arc::new(AtomicU64::new(0));
        let history_generation = route_generation.clone();
        let worker_generation = route_generation.clone();
        let (history_tx, history_rx) = mpsc::sync_channel::<(u64, Arc<Engine>, Page, usize)>(1);
        let history_events = tx.clone();
        std::thread::spawn(move || {
            let mut recent_metadata = HashMap::<(String, String), Entry>::new();
            while let Ok(mut request) = history_rx.recv() {
                while let Ok(newer) = history_rx.try_recv() {
                    request = newer;
                }
                let (generation, engine, mut page, _view_tag) = request;
                #[cfg(feature = "diagnostics")]
                let _page_scope = crate::memory_profile::page_scope(_view_tag + 64);
                if history_generation.load(Ordering::Relaxed) != generation {
                    continue;
                }
                for row in &mut page.rows {
                    if let Some(metadata) = recent_metadata.get(&(row.kind.clone(), row.id.clone()))
                    {
                        enrich_history_entry(row, metadata);
                    }
                }
                let _ = history_events.send(Event::Page(generation, Arc::new(page.clone())));
                let mut dirty = false;
                for index in 0..page.rows.len() {
                    if history_generation.load(Ordering::Relaxed) != generation {
                        break;
                    }
                    let row = &mut page.rows[index];
                    if !row.artwork.is_empty() {
                        continue;
                    }
                    let key = (row.kind.clone(), row.id.clone());
                    let metadata = if row.kind == "episode" {
                        public(&engine, &format!("episode/{}", row.id))
                    } else {
                        engine.invoke("DZTrackMetadataJSON", &[json!(row.id)])
                    };
                    if let Ok(value) = metadata {
                        if let Some(metadata) = entry(&value, &row.kind) {
                            if metadata.id == row.id && !metadata.artwork.is_empty() {
                                enrich_history_entry(row, &metadata);
                                if recent_metadata.len() >= 256 {
                                    if let Some(old) = recent_metadata.keys().next().cloned() {
                                        recent_metadata.remove(&old);
                                    }
                                }
                                recent_metadata.insert(key, metadata);
                                dirty = true;
                            }
                        }
                    }
                    if dirty
                        && (index == 0 || index % 8 == 7)
                        && history_generation.load(Ordering::Relaxed) == generation
                    {
                        if history_events
                            .send(Event::Page(generation, Arc::new(page.clone())))
                            .is_err()
                        {
                            return;
                        }
                        dirty = false;
                    }
                }
                if dirty && history_generation.load(Ordering::Relaxed) == generation {
                    let _ = history_events.send(Event::Page(generation, Arc::new(page)));
                }
            }
        });
        let (navigation_tx, navigation_rx) = mpsc::channel();
        let (action_tx, action_rx) = mpsc::channel();
        std::thread::spawn(move || {
            while let Ok(request) = rx.recv() {
                let target = if matches!(
                    request.job,
                    Job::Route(..) | Job::More(..) | Job::TrackInfo(..)
                ) || matches!(&request.job, Job::Api(name,_) if name == "DeezerStyleMixGenres" || name == "SpotifyStyleMixGenres")
                {
                    &navigation_tx
                } else {
                    &action_tx
                };
                if target.send(request).is_err() {
                    break;
                }
            }
        });
        let navigation_rx = Arc::new(std::sync::Mutex::new(navigation_rx));
        let action_rx = Arc::new(std::sync::Mutex::new(action_rx));
        for rx in [action_rx, navigation_rx.clone(), navigation_rx] {
            let tx = tx.clone();
            let worker_generation = worker_generation.clone();
            let history_tx = history_tx.clone();
            let lyrics_forward = lyrics_forward.clone();
            std::thread::spawn(move || {
                loop {
                    // Drop the receiver lock before doing network work.
                    let request = { rx.lock().unwrap().recv() };
                    let Ok(request) = request else {
                        break;
                    };

                    let Request {
                        #[cfg(feature = "diagnostics")]
                        view_tag,
                        generation,
                        engine,
                        provider,
                        job,
                    } = request;
                    #[cfg(feature = "diagnostics")]
                    let _page_scope = crate::memory_profile::page_scope(if view_tag == 0 {
                        0
                    } else {
                        view_tag + 64
                    });
                    if matches!(job, Job::Route(..) | Job::More(..) | Job::TrackInfo(..))
                        && worker_generation.load(Ordering::Relaxed) != generation
                    {
                        continue;
                    }
                    if matches!(job, Job::Lyrics(..) | Job::LyricsWindow(..)) {
                        let _ = lyrics_forward.try_send(Request {
                            #[cfg(feature = "diagnostics")]
                            view_tag,
                            generation,
                            provider,
                            engine,
                            job,
                        });
                        continue;
                    }
                    if matches!(job, Job::LibraryState) {
                        let result = saved_library_entries(engine.as_deref(), provider);
                        let _ = tx.send(Event::LibraryState(provider, result));
                        continue;
                    }
                    if let Job::Collection(item, action) = job {
                        process_collection(
                            generation,
                            provider,
                            engine.as_deref(),
                            item,
                            action,
                            &tx,
                        );
                        continue;
                    }
                    if provider == Provider::YouTube {
                        crate::youtube::process(generation, job, &tx, &worker_generation);
                        continue;
                    }
                    if provider == Provider::Spotify {
                        spotify::process(generation, job, &tx, &worker_generation);
                        continue;
                    }
                    let Some(engine) = engine else {
                        continue;
                    };
                    let cached = if matches!(job, Job::Route(..)) {
                        tx.resume(generation)
                    } else {
                        None
                    };
                    match job {
                        Job::LibraryState | Job::Collection(..) => {
                            unreachable!("collections are handled before provider dispatch")
                        }
                        Job::Route(View::Home, _, _) => {
                            let shelves = [
                                ("Recently played", "DZHomeModuleJSON", 0, "playlist"),
                                ("Made for you", "DZHomeModuleJSON", 1, "playlist"),
                                ("Playlists you love", "DZHomeModuleJSON", 2, "playlist"),
                                ("Recommended playlists", "DZHomeModuleJSON", 3, "playlist"),
                                ("Recommended artists", "DZHomeModuleJSON", 4, "artist"),
                                ("Recommended tracks", "DZHomeModuleJSON", 5, "track"),
                                (
                                    "100% playlists from your artists",
                                    "DZHomeModuleJSON",
                                    6,
                                    "playlist",
                                ),
                                ("New releases for you", "DZHomeModuleJSON", 7, "album"),
                                (
                                    "Rediscover your favorites",
                                    "DZFavoriteAlbumsJSON",
                                    -1,
                                    "album",
                                ),
                                ("More albums for you", "DZHomeModuleJSON", 8, "album"),
                                ("More for you", "DZHomeModuleJSON", 9, "playlist"),
                            ];
                            let mut page = Page {
                                title: "For you".into(),
                                subtitle: format!(
                                    "Personalized recommendations from {}",
                                    provider.label()
                                ),
                                loading: true,
                                sections: shelves
                                    .iter()
                                    .map(|(title, _, _, _)| Section {
                                        title: (*title).into(),
                                        ..Default::default()
                                    })
                                    .collect(),
                                ..Default::default()
                            };
                            let mut completed = 0;
                            let _ = parallel::load(
                                shelves.len(),
                                |index| {
                                    let (title, name, module, kind) = shelves[index];
                                    let mut section = Section {
                                        title: title.into(),
                                        ..Default::default()
                                    };
                                    if worker_generation.load(Ordering::Relaxed) != generation {
                                        return section;
                                    }
                                    let args = if module < 0 {
                                        vec![]
                                    } else {
                                        vec![json!(module)]
                                    };
                                    match provider.invoke(&engine, name, &args) {
                                        Ok(value) => {
                                            section.entries = if module >= 0
                                                && provider == Provider::Deezer
                                            {
                                                home_module_entries(&value, module as usize, kind)
                                            } else {
                                                let mut rows = entries(&value, kind);
                                                normalize_daily_mixes(&mut rows);
                                                rows
                                            };
                                        }
                                        Err(error) => section.error = error,
                                    }
                                    section
                                },
                                |index, section| {
                                    if worker_generation.load(Ordering::Relaxed) != generation {
                                        return Err("Superseded load".into());
                                    }
                                    page.sections[index] = section;
                                    page.raw.push((format!("home:{index}"), Value::Null));
                                    completed += 1;
                                    page.loading = completed < shelves.len();
                                    tx.send(Event::Page(generation, Arc::new(page.clone())))
                                        .map_err(|_| "Window closed".into())
                                },
                            );
                        }
                        Job::Playlist(mut mutation) => {
                            let result = if provider == Provider::Spotify {
                                crate::playlist::execute_spotify(&mut mutation)
                            } else {
                                crate::playlist::execute(&engine, &mut mutation)
                            };
                            if tx
                                .send(Event::Playlist(generation, provider, mutation, result))
                                .is_err()
                            {
                                return;
                            }
                        }
                        Job::Api(name, args) => {
                            let result = if matches!(
                                name.as_str(),
                                "DeezerStyleMixGenres" | "SpotifyStyleMixGenres"
                            ) {
                                args.first()
                                    .and_then(Value::as_u64)
                                    .ok_or_else(|| "Choose a valid language".to_string())
                                    .and_then(|region| {
                                        if provider == Provider::Spotify {
                                            provider.invoke(&engine, "StyleMixGenres", &args)
                                        } else {
                                            crate::style_mix::available_genres_while(
                                                &engine,
                                                region as usize,
                                                || {
                                                    worker_generation.load(Ordering::Relaxed)
                                                        == generation
                                                },
                                            )
                                        }
                                    })
                            } else if provider == Provider::Spotify {
                                provider.invoke(&engine, &name, &args)
                            } else if name == "RustPodcastLibraryJSON" {
                                podcast_library(&engine, &args)
                            } else if name == "StyleMixGenres" {
                                args.first()
                                    .and_then(Value::as_u64)
                                    .ok_or_else(|| "Choose a valid language".to_string())
                                    .and_then(|region| {
                                        crate::style_mix::available_genres_while(
                                            &engine,
                                            region as usize,
                                            || {
                                                worker_generation.load(Ordering::Relaxed)
                                                    == generation
                                            },
                                        )
                                    })
                            } else {
                                engine.invoke(&name, &args)
                            };
                            if tx
                                .send(Event::Api(generation, provider, name, result))
                                .is_err()
                            {
                                return;
                            }
                        }
                        job @ (Job::Lyrics(..) | Job::LyricsWindow(..)) => {
                            let _ = lyrics_forward.try_send(Request {
                                #[cfg(feature = "diagnostics")]
                                view_tag,
                                generation,
                                provider,
                                engine: Some(engine),
                                job,
                            });
                        }
                        Job::TrackInfo(track) => {
                            let meta = provider
                                .invoke(&engine, "DZTrackMetadataJSON", &[json!(track.id)])
                                .or_else(|error| {
                                    if provider == Provider::Deezer {
                                        public(&engine, &format!("track/{}", track.id))
                                    } else {
                                        Err(error)
                                    }
                                })
                                .unwrap_or_else(|_| track.raw.clone());
                            let artist_id = entry(&meta, "track")
                                .map(|full| full.artist_id)
                                .filter(|id| !id.is_empty())
                                .unwrap_or_else(|| {
                                    if track.artist_id.is_empty() {
                                        string(&meta["artist"], &["id"])
                                    } else {
                                        track.artist_id.clone()
                                    }
                                });
                            let artist_meta = if !artist_id.is_empty() {
                                if provider == Provider::Spotify {
                                    provider
                                        .invoke(
                                            &engine,
                                            "SPCatalogItemJSON",
                                            &[json!("artist"), json!(artist_id)],
                                        )
                                        .unwrap_or(Value::Null)
                                } else {
                                    public(&engine, &format!("artist/{artist_id}"))
                                        .unwrap_or(Value::Null)
                                }
                            } else {
                                Value::Null
                            };
                            let meta = musicbrainz::enrich(&track, meta);
                            if tx
                                .send(Event::TrackInfo(
                                    generation,
                                    Box::new(track),
                                    meta,
                                    artist_meta,
                                ))
                                .is_err()
                            {
                                return;
                            }
                        }
                        Job::More(mut page) if page.next.starts_with("flow:") => {
                            let config = page.next.trim_start_matches("flow:").to_string();
                            let result = if config.is_empty() || config == "default" {
                                provider.invoke(&engine, "DZFlowJSON", &[])
                            } else {
                                provider.invoke(&engine, "DZFlowForConfigJSON", &[json!(config)])
                            };
                            match result {
                                Ok(value) => {
                                    append_flow_rows(&mut page, table_entries(&value, "track"));
                                    page.error.clear();
                                }
                                Err(error) => page.error = error,
                            }
                            page.loading = false;
                            let _ = tx.send(Event::Page(generation, Arc::new(page)));
                        }
                        Job::More(mut page) => {
                            if let Some(index) = page
                                .raw
                                .iter()
                                .position(|(key, _)| key == "spotify:more-shelf")
                            {
                                let (_, request) = page.raw.remove(index);
                                let title = request["title"].as_str().unwrap_or("");
                                let target = request["target"].as_u64().unwrap_or(0) as usize;
                                if let Err(error) =
                                    deezer_shelves::more(&engine, &mut page, title, target)
                                {
                                    page.error = error;
                                }
                                page.loading = false;
                                let _ = tx.send(Event::Page(generation, Arc::new(page)));
                                continue;
                            }
                            let cursor = page.next.clone();
                            let result = if cursor.is_empty() {
                                Err("No more results are available".into())
                            } else {
                                provider.next(&engine, &cursor)
                            };
                            match result {
                                Ok(value) => {
                                    append_page(&mut page, &cursor, value);
                                }
                                Err(error) => page.error = error,
                            }
                            page.loading = false;
                            let _ = tx.send(Event::Page(generation, Arc::new(page)));
                        }
                        Job::Route(view, entity, query) => {
                            if (view == View::Album || view == View::Artist || view == View::Show)
                                && entity.is_none()
                            {
                                if let Some(page) = cached.clone() {
                                    let mut page = (*page).clone();
                                    page.loading = false;
                                    let _ = tx.send(Event::Page(generation, Arc::new(page)));
                                    continue;
                                }
                            }
                            if view == View::Flow {
                                if let Some(page) =
                                    cached.clone().filter(|page| !page.rows.is_empty())
                                {
                                    let mut page = (*page).clone();
                                    page.loading = false;
                                    let _ = tx.send(Event::Page(generation, Arc::new(page)));
                                    continue;
                                }
                            }
                            let mut publish = |page: &Page| {
                                if worker_generation.load(Ordering::Relaxed) != generation {
                                    return Err("Superseded load".into());
                                }
                                let mut partial = page.clone();
                                partial.loading = true;
                                tx.send(Event::Page(generation, Arc::new(partial)))
                                    .map_err(|_| "Window closed".to_string())
                            };
                            let result = if provider == Provider::Spotify {
                                spotify::load(&engine, view, entity, query, &mut publish)
                            } else {
                                load(&engine, view, entity, query, &mut publish)
                            };
                            if worker_generation.load(Ordering::Relaxed) != generation {
                                continue;
                            }
                            let page = match result {
                                Ok(mut page) => {
                                    let depth = cached
                                        .as_deref()
                                        .map(session::pagination_depth)
                                        .unwrap_or(0);
                                    for _ in 0..depth {
                                        if page.next.is_empty()
                                            || worker_generation.load(Ordering::Relaxed)
                                                != generation
                                        {
                                            break;
                                        }
                                        let cursor = page.next.clone();
                                        match provider.next(&engine, &cursor) {
                                            Ok(value) => append_page(&mut page, &cursor, value),
                                            Err(error) => {
                                                page.error = error;
                                                break;
                                            }
                                        }
                                        let mut partial = page.clone();
                                        partial.loading = true;
                                        let _ = tx.send(Event::Page(generation, Arc::new(partial)));
                                    }
                                    page
                                }
                                Err(error) => Page {
                                    title: view.title().into(),
                                    error,
                                    ..Default::default()
                                },
                            };
                            if tx
                                .send(Event::Page(generation, Arc::new(page.clone())))
                                .is_err()
                            {
                                return;
                            }
                            if view == View::Recent && provider == Provider::Deezer {
                                // Cover metadata must never occupy the page/API worker.
                                let _ = history_tx.try_send((
                                    generation,
                                    engine,
                                    page,
                                    view as usize + 1,
                                ));
                            }
                        }
                    }
                }
            });
        }
        Self {
            requests,
            updates,
            lyrics_requests: lyrics_tx,
            route_generation,
            session,
        }
    }
}

// Retain the recorded title/kind and history fields while filling replay metadata.
fn enrich_history_entry(row: &mut Entry, metadata: &Entry) {
    if row.artwork.is_empty() {
        row.artwork = metadata.artwork.clone();
    }
    if row.subtitle.is_empty() {
        row.subtitle = metadata.subtitle.clone();
    }
    if row.album.is_empty() {
        row.album = metadata.album.clone();
    }
    if row.duration == 0 {
        row.duration = metadata.duration;
    }
    if row.artist_id.is_empty() {
        row.artist_id = metadata.artist_id.clone();
    }
    if row.album_id.is_empty() {
        row.album_id = metadata.album_id.clone();
    }
    if let (Some(recorded), Some(details)) = (row.raw.as_object_mut(), metadata.raw.as_object()) {
        for (key, value) in details {
            recorded.entry(key.clone()).or_insert_with(|| value.clone());
        }
    }
}

fn append_page(page: &mut Page, cursor: &str, value: Value) {
    let start = page.rows.len();
    let depth = session::pagination_depth(page);
    let mut seen: HashSet<_> = page
        .rows
        .iter()
        .map(|item| (item.kind.clone(), item.id.clone()))
        .collect();
    page.rows.extend(
        table_entries(&value, "track")
            .into_iter()
            .filter(|item| seen.insert((item.kind.clone(), item.id.clone()))),
    );
    inherit_collection_metadata(page, &value);
    let next = string(&value, &["next"]);
    // Stop cyclic cursors instead of fetching the same page indefinitely.
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
    if let Some((_, value)) = page
        .raw
        .iter_mut()
        .find(|(key, _)| key == "cache:pagination-depth")
    {
        *value = json!(depth + 1);
    }
}
fn dynamic_sections(
    engine: &Engine,
    uri: &str,
    page: &mut Page,
    publish: &mut dyn FnMut(&Page) -> Result<(), String>,
) -> Result<(), String> {
    publish(page)?;
    let value = deezer_shelves::api(engine, "DZDiscoveryPageJSON", &[json!(uri), json!("")])?;
    let sections = value
        .pointer("/data/dynamicPage/sections/edges")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    for batch in sections.iter().take(24).collect::<Vec<_>>().chunks(3) {
        let results = std::thread::scope(|scope| {
            let jobs: Vec<_> = batch
                .iter()
                .map(|edge| {
                    let node = &edge["node"];
                    let id = string(node, &["id"]);
                    let kind = string(node, &["__typename"]);
                    let title = string(node, &["title"]);
                    scope.spawn(move || {
                        let result = if id.is_empty() {
                            Ok(Value::Null)
                        } else {
                            deezer_shelves::api(
                                engine,
                                "DZDiscoverySectionJSON",
                                &[json!(id), json!(kind), json!("")],
                            )
                        };
                        (id, title, kind, result)
                    })
                })
                .collect();
            jobs.into_iter()
                .filter_map(|job| job.join().ok())
                .collect::<Vec<_>>()
        });
        for (id, title, kind, result) in results {
            if id.is_empty() {
                continue;
            }
            match result {
                Ok(content) => {
                    deezer_shelves::remember_dynamic(page, &title, &content, &id, &kind);
                    page.sections.push(Section {
                        title,
                        entries: entries(&content, "playlist"),
                        ..Default::default()
                    });
                    page.raw.push((format!("section:{id}"), Value::Null));
                }
                Err(error) => page.sections.push(Section {
                    title,
                    error,
                    ..Default::default()
                }),
            }
            publish(page)?;
        }
    }
    let cover = page
        .entity
        .as_ref()
        .map(|item| item.artwork.as_str())
        .unwrap_or("");
    deezer_shelves::cover_directories(&mut page.sections, cover);
    page.raw.push((uri.into(), Value::Null));
    Ok(())
}

fn podcast_library_path(engine: &Engine) -> Result<std::path::PathBuf, String> {
    let user = engine.invoke("DZUserID", &[])?;
    let user = user
        .as_str()
        .map(str::to_owned)
        .or_else(|| user.as_u64().map(|id| id.to_string()))
        .ok_or("No podcast library account")?;
    if user.is_empty() || !user.bytes().all(|b| b.is_ascii_digit()) {
        return Err("No verified podcast library account".into());
    }
    Ok(crate::storage::data_dir()?.join(format!("podcasts-{user}.json")))
}

fn read_podcast_library(path: &std::path::Path) -> Result<Vec<Entry>, String> {
    match std::fs::read(path) {
        Ok(bytes) => {
            let value: Value = serde_json::from_slice(&bytes)
                .map_err(|e| format!("Invalid saved podcast library: {e}"))?;
            if !value.is_array() {
                return Err("Invalid saved podcast library".into());
            }
            Ok(entries(&value, "podcast"))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(error) => Err(error.to_string()),
    }
}

fn edit_podcast_library(items: &mut Vec<Entry>, item: Entry, add: bool) -> Result<(), String> {
    if item.kind != "podcast" || item.id.is_empty() {
        return Err("Select a podcast to save".into());
    }
    items.retain(|old| old.id != item.id);
    if add {
        items.push(item);
    }
    Ok(())
}

// App-local subscription support. The native ABI has no Deezer podcast-follow
// mutation; do not misroute a podcast ID into the track-favorites API.
fn podcast_library(engine: &Engine, args: &[Value]) -> Result<Value, String> {
    let path = podcast_library_path(engine)?;
    let mut items = read_podcast_library(&path)?;
    if !args.is_empty() {
        if args.len() != 2 {
            return Err("Podcast library expects a podcast object and add/remove boolean".into());
        }
        let add = args[1]
            .as_bool()
            .ok_or("Podcast library add/remove must be boolean")?;
        let item = entry(&args[0], "podcast").ok_or("Invalid podcast")?;
        edit_podcast_library(&mut items, item, add)?;
        let bytes = serde_json::to_vec(&items.iter().map(Entry::wire).collect::<Vec<_>>())
            .map_err(|e| e.to_string())?;
        let temporary = path.with_extension("json.tmp");
        std::fs::create_dir_all(path.parent().ok_or("Invalid podcast library path")?)
            .map_err(|e| e.to_string())?;
        std::fs::write(&temporary, bytes).map_err(|e| e.to_string())?;
        std::fs::rename(&temporary, &path).map_err(|e| e.to_string())?;
    }
    Ok(
        json!({"local":true,"changed":!args.is_empty(),"podcasts":items.iter().map(Entry::wire).collect::<Vec<_>>()}),
    )
}

pub fn podcast_library_page(
    account: Vec<Entry>,
    local: Vec<Entry>,
    bookmarks: Vec<Entry>,
    offline: Vec<Entry>,
) -> Page {
    let mut seen = HashSet::new();
    let saved = account
        .into_iter()
        .chain(local)
        .filter(|item| item.kind == "podcast" && seen.insert(item.id.clone()))
        .collect();
    let episodes = |items: Vec<Entry>| items.into_iter().filter(|e| e.kind == "episode").collect();
    Page {
        title: "Podcasts".into(),
        subtitle: "Your podcasts".into(),
        sections: vec![
            Section {
                title: "Saved podcasts".into(),
                entries: saved,
                ..Default::default()
            },
            Section {
                title: "Favorites".into(),
                entries: episodes(bookmarks),
                ..Default::default()
            },
            Section {
                title: "Downloads".into(),
                entries: episodes(offline),
                ..Default::default()
            },
        ],
        ..Default::default()
    }
}
fn load(
    engine: &Engine,
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
        ..Default::default()
    };
    if view == View::Explore && query.is_empty() {
        return discover(engine, page, publish);
    }
    if view == View::Podcasts {
        let mut errors = Vec::new();
        let mut raw = Vec::new();
        let offline = crate::downloads::load_offline();
        let local = match podcast_library(engine, &[]) {
            Ok(value) => entries(&value["podcasts"], "podcast"),
            Err(error) => {
                errors.push(error);
                Vec::new()
            }
        };
        let mut page = podcast_library_page(Vec::new(), local.clone(), Vec::new(), offline.clone());
        publish(&page)?;
        let account = match engine.invoke("DZPodcastShelfJSON", &[json!(1)]) {
            Ok(value) => {
                let items = entries(&value, "podcast");
                raw.push(("podcasts:account".into(), Value::Null));
                items
            }
            Err(error) => {
                errors.push(error);
                Vec::new()
            }
        };
        page = podcast_library_page(account.clone(), local.clone(), Vec::new(), offline.clone());
        publish(&page)?;
        let bookmarks = match engine.invoke("DZPodcastShelfJSON", &[json!(3)]) {
            Ok(value) => {
                let items = entries(&value, "episode");
                raw.push(("podcasts:bookmarks".into(), Value::Null));
                items
            }
            Err(error) => {
                errors.push(error);
                Vec::new()
            }
        };
        page = podcast_library_page(account, local, bookmarks, offline);
        page.error = errors.join("; ");
        page.raw = raw;
        return Ok(page);
    }
    if view == View::StyleMix {
        if !query.is_empty() {
            let selection = crate::style_mix::Selection::parse(&query)?;
            page.subtitle = format!("{} · From matching Deezer playlists", selection.title());
            page.rows = crate::style_mix::tracks(engine, selection)?;
        }
        return Ok(page);
    }
    let (name, args, kind, table) = match view {
        View::Favorites => ("DZFavoritesJSON", vec![], "track", true),
        View::Albums => ("DZFavoriteAlbumsJSON", vec![], "album", false),
        View::Playlists | View::PlaylistPicker => ("DZPlaylistsJSON", vec![], "playlist", false),
        View::Recent => ("DZHistoryRecentJSON", vec![json!(200)], "track", true),
        View::Flow => {
            let (name, args) = flow_request(entity.as_ref(), &query);
            if entity.as_ref().is_some_and(|entry| entry.kind == "track") {
                page.subtitle = "Song radio".into();
            }
            (name, args, "track", true)
        }
        View::Queue => ("DZQueueJSON", vec![], "track", true),
        View::Explore => ("DZSearchJSON", vec![json!(query)], "track", false),
        View::Album | View::Artist | View::Show => {
            let item = entity
                .as_ref()
                .ok_or("Select an album, artist, playlist or podcast first")?;
            match item.kind.as_str() {
                "playlist" => ("DZPlaylistTracksJSON", vec![json!(item.id)], "track", true),
                "artist" => ("DZArtistProfileJSON", vec![json!(item.id)], "album", false),
                "podcast" => (
                    "DZPodcastEpisodesJSON",
                    vec![json!(item.id)],
                    "episode",
                    true,
                ),
                "mix" => ("DZSmartMixJSON", vec![json!(item.id)], "track", true),
                "flow" => ("DZFlowJSON", vec![], "track", true),
                "channel" => {
                    if item.id.starts_with("deezer://") {
                        dynamic_sections(engine, &item.id, &mut page, publish)?;
                        for section in &mut page.sections {
                            section.entries.retain(|item| {
                                matches!(
                                    item.kind.as_str(),
                                    "playlist" | "album" | "mix" | "podcast"
                                )
                            });
                        }
                        return Ok(page);
                    }
                    if item.id.starts_with("editorial:") {
                        publish(&page)?;
                        let id = item.id.trim_start_matches("editorial:");
                        for (part, label, kind) in [
                            ("charts", "Top playlists", "playlist"),
                            ("releases", "New releases", "album"),
                            ("selection", "Editors' album picks", "album"),
                        ] {
                            match deezer_shelves::public(
                                engine,
                                &format!("editorial/{id}/{part}?limit=8"),
                            ) {
                                Ok(value) => {
                                    let data = if part == "charts" {
                                        &value["playlists"]
                                    } else {
                                        &value
                                    };
                                    deezer_shelves::remember_public(&mut page, label, data, kind);
                                    page.sections.push(Section {
                                        title: label.into(),
                                        entries: curated_entries(data, kind),
                                        ..Default::default()
                                    });
                                    page.raw.push((part.into(), Value::Null));
                                }
                                Err(error) => page.sections.push(Section {
                                    title: label.into(),
                                    error,
                                    ..Default::default()
                                }),
                            }
                            publish(&page)?;
                        }
                        return Ok(page);
                    }
                    let value = deezer_shelves::web(
                        engine,
                        &format!(
                            "https://www.deezer.com/en{}",
                            item.id.trim_start_matches("web:")
                        ),
                    )?;
                    page.sections = web_sections(&value, false);
                    page.raw.push(("web".into(), Value::Null));
                    return Ok(page);
                }
                _ => ("DZAlbumTracksJSON", vec![json!(item.id)], "track", true),
            }
        }
        _ => return Ok(page),
    };
    let value = engine.invoke(name, &args)?;
    if table {
        page.rows = table_entries(&value, kind);
        inherit_collection_metadata(&mut page, &value);
        if let Some(playlist) = page.entity.as_ref().filter(|item| item.kind == "playlist") {
            if let Ok(metadata) = public(engine, &format!("playlist/{}", playlist.id)) {
                let preview = playlist.artwork.clone();
                inherit_collection_metadata(&mut page, &metadata);
                if preview.starts_with("playlist-cover:") {
                    if let Some(entity) = page.entity.as_mut() {
                        entity.artwork = preview;
                    }
                }
            }
        }
        // The native album API often returns only tracks, without a header.
        if let Some(album) = page
            .entity
            .as_ref()
            .filter(|item| item.kind == "album" && item.artwork.is_empty())
        {
            if let Ok(metadata) = public(engine, &format!("album/{}", album.id)) {
                inherit_collection_metadata(&mut page, &metadata);
                page.raw.push(("album-metadata".into(), Value::Null));
            }
        }
        if let Some(mix) = value.pointer("/data/smartTracklist") {
            if let Some(entity) = page.entity.as_mut() {
                let artwork = picture(mix);
                if !artwork.is_empty() {
                    entity.artwork = artwork;
                }
                entity.subtitle.clear();
                entity.raw = compact_entry_metadata(&mix);
            }
        }
    } else if view == View::Explore {
        for (key, label, typ) in [
            ("tracks", "Tracks", "track"),
            ("albums", "Albums", "album"),
            ("artists", "Artists", "artist"),
            ("playlists", "Playlists", "playlist"),
            ("podcasts", "Podcasts", "podcast"),
        ] {
            if let Some(data) = value.get(key) {
                page.sections.push(Section {
                    title: label.into(),
                    entries: entries(data, typ),
                    ..Default::default()
                });
            }
        }
        publish(&page)?;
        if !query.is_empty()
            && !page
                .sections
                .iter()
                .any(|section| section.title == "Podcasts" && !section.entries.is_empty())
        {
            if let Ok(podcasts) = engine.invoke("DZSearchPodcastsJSON", &[json!(query)]) {
                page.sections.push(Section {
                    title: "Podcasts".into(),
                    entries: entries(&podcasts, "podcast"),
                    ..Default::default()
                });
                page.raw.push(("podcast-search".into(), Value::Null));
            }
        }
    } else {
        page.sections.push(Section {
            title: page.title.clone(),
            entries: entries(&value, kind),
            ..Default::default()
        });
    }
    if view == View::Playlists {
        publish(&page)?;
        let mut saved = Vec::new();
        if let Ok(value) = engine.invoke("DZPodcastShelfJSON", &[json!(1)]) {
            saved = entries(&value, "podcast");
            page.raw.push(("podcasts:account".into(), Value::Null));
        }
        match podcast_library(engine, &[]) {
            Ok(value) => {
                let mut seen: HashSet<_> = saved.iter().map(|item| item.id.clone()).collect();
                saved.extend(
                    entries(&value["podcasts"], "podcast")
                        .into_iter()
                        .filter(|item| seen.insert(item.id.clone())),
                );
                page.raw.push(("podcasts:local".into(), Value::Null));
            }
            Err(error) => page.sections.push(Section {
                title: "Saved podcasts".into(),
                error,
                ..Default::default()
            }),
        }
        if !saved.is_empty() {
            page.sections.push(Section {
                title: "Saved podcasts".into(),
                entries: saved,
                ..Default::default()
            });
        }
    }
    if view == View::Show {
        if let Some(podcast) = page.entity.as_mut().filter(|item| item.kind == "podcast") {
            // The native episodes API returns the complete paginated collection.
            podcast.raw["episodeCount"] = json!(page.rows.len());
        }
    }
    if view == View::Artist {
        if let Some(item) = entity.as_ref() {
            // Profile is an envelope, not the artist object itself.
            if let (Some(metadata), Some(artist)) = (value.get("artist"), page.entity.as_mut()) {
                update_artist_metadata(artist, metadata);
            }
            page.sections.clear();
            let mut top = table_entries(&value["top"], "track");
            if top.is_empty() {
                if let Ok(response) = engine.invoke("DZArtistTopJSON", &[json!(item.id)]) {
                    top = table_entries(&response, "track");
                }
            }
            if !top.is_empty() {
                page.sections.push(Section {
                    title: "Popular tracks".into(),
                    entries: top,
                    ..Default::default()
                });
            }
            for (index, label) in [
                (1, "Albums"),
                (2, "Singles and EPs"),
                (3, "Appears on"),
                (4, "Similar artists"),
                (5, "Related playlists"),
            ] {
                publish(&page)?;
                let fallback = match index {
                    1 => entries(&value["albums"], "album"),
                    4 => entries(&value["related"], "artist"),
                    _ => Vec::new(),
                };
                match engine.invoke(
                    "DZArtistSectionJSON",
                    &[json!(item.id), json!(index), json!("")],
                ) {
                    Ok(section) => {
                        if let (Some(metadata), Some(artist)) =
                            (section.pointer("/data/artist"), page.entity.as_mut())
                        {
                            update_artist_metadata(artist, metadata);
                        }
                        let items = artist_section_entries(&section, index);
                        page.sections.push(Section {
                            title: label.into(),
                            entries: items,
                            ..Default::default()
                        });
                        page.raw.push((format!("artist:{index}"), Value::Null));
                    }
                    Err(error) => page.sections.push(Section {
                        title: label.into(),
                        error: if fallback.is_empty() {
                            error
                        } else {
                            String::new()
                        },
                        entries: fallback,
                        ..Default::default()
                    }),
                }
            }
            page.rows.clear();
        }
    }
    page.next = if view == View::Flow && !entity.as_ref().is_some_and(|entry| entry.kind == "track")
    {
        format!("flow:{query}")
    } else {
        string(&value, &["next"])
    };
    page.raw.push((name.into(), Value::Null));
    Ok(page)
}
#[derive(Clone)]
pub struct Bitmap {
    pub id: u64,
    pub width: u32,
    pub height: u32,
    pub pixels: Arc<Vec<u32>>,
}
impl Bitmap {
    pub fn new(width: u32, height: u32, pixels: Vec<u32>) -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        Self {
            id: NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            width,
            height,
            pixels: Arc::new(pixels),
        }
    }
}
pub fn decode(bytes: &[u8], size: u32) -> Result<Bitmap, String> {
    decode_image(bytes, size, true)
}
pub fn decode_background(bytes: &[u8], size: u32) -> Result<Bitmap, String> {
    decode_image(bytes, size, false)
}
fn decode_image(bytes: &[u8], size: u32, cover: bool) -> Result<Bitmap, String> {
    let mut reader = image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|e| e.to_string())?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(6000);
    limits.max_image_height = Some(6000);
    limits.max_alloc = Some(96 * 1024 * 1024);
    reader.limits(limits);
    let image = reader.decode().map_err(|e| e.to_string())?;
    let image = if cover {
        artwork_resize::cover(image, size)
    } else {
        image.resize(size, size, image::imageops::FilterType::Triangle)
    }
    .into_rgba8();
    let pixels = image
        .pixels()
        .map(|p| 0xff000000 | ((p[0] as u32) << 16) | ((p[1] as u32) << 8) | p[2] as u32)
        .collect();
    Ok(Bitmap::new(image.width(), image.height(), pixels))
}
pub struct ArtRequest {
    #[cfg(feature = "diagnostics")]
    pub view_tag: usize,
    pub engine: Option<Arc<Engine>>,
    pub url: String,
    pub key: String,
    pub background: bool,
    pub max_edge: u32,
}

// Cards render at at most 160 physical pixels; leave sampling headroom without
// retaining a full 256px decoded copy for every thumbnail in a large queue.
pub const CARD_ART_EDGE: u32 = 192;

pub(crate) fn is_default_cover(url: &str) -> bool {
    url.contains("/images/cover/d41d8cd98f00b204e9800998ecf8427e/")
}

pub(crate) fn cover_artwork_key(url: &str) -> String {
    format!("cover:{}", display_artwork_url(url, 1024, true))
}

/// Spotify image arrays are not guaranteed to be ordered by resolution.
pub(crate) fn largest_artwork<'a>(images: impl Iterator<Item = &'a Value>) -> Option<&'a str> {
    images
        .filter_map(|image| {
            let url = image["url"]
                .as_str()
                .filter(|url| url.starts_with("https://"))?;
            let edge = image["width"]
                .as_u64()
                .unwrap_or(0)
                .max(image["height"].as_u64().unwrap_or(0));
            Some((edge, url))
        })
        .max_by_key(|(edge, _)| *edge)
        .map(|(_, url)| url)
}

pub struct Artwork {
    cache: Arc<std::sync::Mutex<ArtworkCache>>,
    queue: Arc<artwork_queue::ArtworkQueue>,
    cancelled_tx: mpsc::Sender<String>,
    pub cancelled: mpsc::Receiver<String>,
    #[cfg(test)]
    fixture_requests: Option<mpsc::SyncSender<ArtRequest>>,
    pub updates: mpsc::Receiver<(String, Result<Arc<Bitmap>, String>)>,
}

// Deezer URLs often request 1000px covers even for 160px cards. Keep the
// original UI key, but fetch the size we actually decode. Other hosts are unchanged.
#[cfg(test)]
fn artwork_url(url: &str, background: bool) -> String {
    display_artwork_url(url, if background { 768 } else { 256 }, false)
}
pub fn display_artwork_url(url: &str, size: u32, upscale: bool) -> String {
    let host = url
        .strip_prefix("https://")
        .and_then(|rest| rest.split('/').next());
    if host.is_some_and(|host| {
        matches!(
            host,
            "lh3.googleusercontent.com" | "yt3.googleusercontent.com" | "yt3.ggpht.com"
        )
    }) {
        let (path, query) = url.split_once('?').map_or((url, ""), |(a, b)| (a, b));
        if let Some((base, options)) = path.rsplit_once('=') {
            let size = size.clamp(64, 1024);
            let mut resized = false;
            let parts: Vec<String> = options
                .split('-')
                .map(|part| {
                    if let Some((prefix, digits)) = ["w", "h", "s"].iter().find_map(|prefix| {
                        part.strip_prefix(prefix).map(|digits| (*prefix, digits))
                    }) {
                        if !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()) {
                            if let Ok(original) = digits.parse::<u32>() {
                                resized = true;
                                return format!(
                                    "{prefix}{}",
                                    if upscale { size } else { original.min(size) }
                                );
                            }
                        }
                    }
                    part.to_owned()
                })
                .collect();
            if resized {
                return format!(
                    "{base}={}{}",
                    parts.join("-"),
                    if query.is_empty() {
                        String::new()
                    } else {
                        format!("?{query}")
                    }
                );
            }
        }
        return url.into();
    }
    if !url.starts_with("https://")
        || !url
            .split('/')
            .nth(2)
            .is_some_and(|host| host == "dzcdn.net" || host.ends_with(".dzcdn.net"))
    {
        return url.into();
    }
    let Some((base, file)) = url.rsplit_once('/') else {
        return url.into();
    };
    let Some((dimensions, suffix)) = file.split_once('-') else {
        return url.into();
    };
    let Some((width, height)) = dimensions.split_once('x') else {
        return url.into();
    };
    let (Ok(width), Ok(height)) = (width.parse::<u32>(), height.parse::<u32>()) else {
        return url.into();
    };
    let size = size.clamp(64, 1024);
    if !upscale && (width <= size || height <= size) {
        return url.into();
    }
    format!("{base}/{size}x{size}-{suffix}")
}

#[derive(Default)]
struct ArtworkCache {
    // One owning cache shares allocations with resident UI images. Recent
    // offscreen covers survive navigation instead of immediately expiring.
    items: std::collections::VecDeque<(String, bool, Arc<Bitmap>, usize, std::time::Instant)>,
    bytes: usize,
    pins: std::collections::HashSet<u64>,
}
impl ArtworkCache {
    fn get(&mut self, url: &str, background: bool) -> Option<Arc<Bitmap>> {
        let index = self
            .items
            .iter()
            .position(|(key, bg, ..)| key == url && *bg == background)?;
        let mut item = self.items.remove(index)?;
        let image = item.2.clone();
        item.4 = std::time::Instant::now();
        self.items.push_back(item);
        Some(image)
    }
    fn insert(&mut self, url: String, background: bool, image: Arc<Bitmap>) {
        const LIMIT: usize = 64 * 1024 * 1024;
        let bytes = image.pixels.len() * 4;
        if bytes > LIMIT {
            return;
        }
        if let Some(index) = self
            .items
            .iter()
            .position(|(key, bg, ..)| key == &url && *bg == background)
        {
            if let Some((_, _, _, bytes, _)) = self.items.remove(index) {
                self.bytes -= bytes;
            }
        }
        self.bytes += bytes;
        self.items
            .push_back((url, background, image, bytes, std::time::Instant::now()));
        self.trim_to(LIMIT);
    }
    fn trim_to(&mut self, limit: usize) {
        while self.bytes > limit || self.items.len() > 1024 {
            let Some(index) = self
                .items
                .iter()
                .position(|entry| !self.pins.contains(&entry.2.id))
            else {
                break;
            };
            let entry = self.items.remove(index).unwrap();
            self.bytes = self.bytes.saturating_sub(entry.3);
        }
    }
}
fn artwork_identity(image: &Bitmap, key: &str, background: bool) -> u64 {
    use sha2::{Digest, Sha256};
    let mut hash = Sha256::new();
    hash.update(key.as_bytes());
    hash.update([background as u8]);
    hash.update(image.width.to_le_bytes());
    hash.update(image.height.to_le_bytes());
    // Bitmap pixels are an initialized contiguous u32 buffer, read only here.
    hash.update(unsafe {
        std::slice::from_raw_parts(image.pixels.as_ptr().cast::<u8>(), image.pixels.len() * 4)
    });
    let digest = hash.finalize();
    (1u64 << 62) | (u64::from_le_bytes(digest[..8].try_into().unwrap()) & ((1u64 << 62) - 1))
}
impl Artwork {
    pub fn recent_ids(&self) -> std::collections::HashSet<u64> {
        self.cache
            .lock()
            .unwrap()
            .items
            .iter()
            .filter(|e| e.4.elapsed() < std::time::Duration::from_secs(5))
            .map(|e| e.2.id)
            .collect()
    }
    pub fn residency(
        &self,
        pins: std::collections::HashSet<u64>,
        visible: &std::collections::HashSet<u64>,
    ) {
        let mut cache = self.cache.lock().unwrap();
        cache.pins = pins;
        for entry in &mut cache.items {
            if visible.contains(&entry.2.id) {
                entry.4 = std::time::Instant::now();
            }
        }
        cache.trim_to(64 * 1024 * 1024);
    }
    pub fn shrink(&self, pins: std::collections::HashSet<u64>) {
        let mut cache = self.cache.lock().unwrap();
        cache.pins = pins;
        cache.trim_to(2 * 1024 * 1024);
    }
    pub fn request(&self, request: ArtRequest, visible: bool) -> Result<(), ArtRequest> {
        #[cfg(test)]
        if let Some(sender) = &self.fixture_requests {
            return sender.try_send(request).map_err(|e| match e {
                mpsc::TrySendError::Full(r) | mpsc::TrySendError::Disconnected(r) => r,
            });
        }
        self.queue.push(request, visible)
    }
    #[cfg(test)]
    pub(crate) fn fixture(
        requests: mpsc::SyncSender<ArtRequest>,
        updates: mpsc::Receiver<(String, Result<Arc<Bitmap>, String>)>,
    ) -> Self {
        let (cancelled_tx, cancelled) = mpsc::channel();
        Self {
            cache: Arc::new(std::sync::Mutex::new(ArtworkCache::default())),
            queue: Arc::new(artwork_queue::ArtworkQueue::default()),
            cancelled_tx,
            cancelled,
            updates,
            fixture_requests: Some(requests),
        }
    }
    pub fn set_wanted(
        &self,
        keys: std::collections::HashSet<String>,
        visible: std::collections::HashSet<String>,
    ) {
        for key in self.queue.update(keys, visible) {
            let _ = self.cancelled_tx.send(key);
        }
    }
    pub fn start() -> Self {
        let queue = Arc::new(artwork_queue::ArtworkQueue::default());
        let (cancelled_tx, cancelled) = mpsc::channel();
        let (tx, updates) = mpsc::sync_channel(16);
        let cache = Arc::new(std::sync::Mutex::new(ArtworkCache::default()));
        let cpus = std::thread::available_parallelism().map_or(4, usize::from);
        let (decode_tx, decode_rx) = mpsc::sync_channel::<(ArtRequest, Vec<u8>, String, u32)>(4);
        let decode_rx = Arc::new(std::sync::Mutex::new(decode_rx));
        // More network overlap without letting simultaneous decoders consume
        // every core when the foreground UI is CPU constrained.
        for index in 0..(cpus / 4).clamp(1, 2) {
            let work = decode_rx.clone();
            let tx = tx.clone();
            let cache = cache.clone();
            let queue = queue.clone();
            let cancelled = cancelled_tx.clone();
            std::thread::Builder::new()
                .name(format!("artwork-decode-{index}"))
                .spawn(move || {
                    artwork_background_priority();
                    loop {
                        let job = work.lock().unwrap().recv();
                        let Ok((request, bytes, cache_key, edge)) = job else {
                            return;
                        };
                        if !queue.wanted(&request) {
                            let _ = cancelled.send(request.key);
                            continue;
                        }
                        #[cfg(feature = "diagnostics")]
                        let _page_scope =
                            crate::memory_profile::page_scope(if request.view_tag == 0 {
                                0
                            } else {
                                request.view_tag + 128
                            });
                        let result = (if request.background {
                            decode_background(&bytes, edge)
                        } else {
                            decode(&bytes, edge)
                        })
                        .map(|mut image| {
                            image.id = artwork_identity(&image, &cache_key, request.background);
                            Arc::new(image)
                        });
                        if !queue.wanted(&request) {
                            let _ = cancelled.send(request.key);
                            continue;
                        }
                        if let Ok(image) = &result {
                            cache.lock().unwrap().insert(
                                cache_key,
                                request.background,
                                image.clone(),
                            );
                        }
                        let hover = if edge > 256 && !request.background {
                            result
                                .as_ref()
                                .ok()
                                .map(|image| (format!("blur:cover:{}", request.key), image.clone()))
                        } else {
                            None
                        };
                        if tx.send((request.key, result)).is_err() {
                            return;
                        }
                        if let Some((key, image)) = hover {
                            let blurred = Arc::new(crate::appearance::prepare(&image, 24));
                            if tx.send((key, Ok(blurred))).is_err() {
                                return;
                            }
                        }
                    }
                })
                .expect("start artwork decoder");
        }
        for index in 0..cpus.clamp(4, 8) {
            let queue = queue.clone();
            let cancelled = cancelled_tx.clone();
            let tx = tx.clone();
            let cache = cache.clone();
            let decode_tx = decode_tx.clone();
            std::thread::Builder::new()
                .name(format!("artwork-fetch-{index}"))
                .spawn(move || {
                    artwork_background_priority();
                    loop {
                        let Some(request) = queue.pop() else {
                            return;
                        };
                        if !queue.wanted(&request) {
                            let _ = cancelled.send(request.key);
                            continue;
                        }
                        #[cfg(feature = "diagnostics")]
                        let _page_scope =
                            crate::memory_profile::page_scope(if request.view_tag == 0 {
                                0
                            } else {
                                request.view_tag + 128
                            });
                        let edge = request.max_edge.clamp(64, 1024);
                        let cache_key = format!("{}@{edge}", request.url);
                        let cached = cache.lock().unwrap().get(&cache_key, request.background);
                        if let Some(image) = cached {
                            if tx.send((request.key, Ok(image))).is_err() {
                                return;
                            }
                            continue;
                        }
                        let url = display_artwork_url(&request.url, edge, edge > 256);
                        let public_cdn = url
                            .strip_prefix("https://")
                            .and_then(|rest| rest.split('/').next())
                            .is_some_and(|host| {
                                host == "dzcdn.net" || host.ends_with(".dzcdn.net")
                            });
                        let bytes = if public_cdn {
                            crate::lyrics::http::artwork(&url)
                        } else if let Some(engine) = &request.engine {
                            engine.fetch(&url)
                        } else {
                            crate::lyrics::http::request("GET", &url, &[], &[])
                        };
                        match bytes {
                            Ok(bytes) => {
                                if !queue.wanted(&request) {
                                    let _ = cancelled.send(request.key);
                                    continue;
                                }
                                if decode_tx.send((request, bytes, cache_key, edge)).is_err() {
                                    return;
                                }
                            }
                            Err(error) => {
                                if tx.send((request.key, Err(error))).is_err() {
                                    return;
                                }
                            }
                        }
                    }
                })
                .expect("start artwork fetcher");
        }
        Self {
            cache,
            queue,
            cancelled_tx,
            cancelled,
            updates,
            #[cfg(test)]
            fixture_requests: None,
        }
    }
}
impl Drop for Artwork {
    fn drop(&mut self) {
        self.queue.close();
    }
}
pub(crate) fn artwork_background_priority() {
    unsafe {
        use windows_sys::Win32::System::Threading::{
            GetCurrentThread, SetThreadPriority, THREAD_MODE_BACKGROUND_BEGIN,
        };
        SetThreadPriority(GetCurrentThread(), THREAD_MODE_BACKGROUND_BEGIN);
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn collection_metadata_keeps_credits_and_counts_without_duplicate_track_arrays() {
        let input = serde_json::json!({"id":"1","title":"Collection","description":"Keep this","artists":[{"id":"2","name":"Artist"}],"tracks":{"data":vec![serde_json::json!({"id":"3","title":"Track"});5000],"total":5000},"episodes":{"data":[],"totalCount":42}});
        let compact = compact_entry_metadata(&input);
        assert_eq!(compact["description"], input["description"]);
        assert_eq!(compact["artists"], input["artists"]);
        assert_eq!(compact["tracks"]["total"], 5000);
        assert_eq!(compact["episodes"]["totalCount"], 42);
        assert!(compact["tracks"].get("data").is_none());
        assert!(compact.to_string().len() * 100 < input.to_string().len());
    }
    use super::*;
    #[test]
    fn youtube_artwork_requests_larger_images_for_playback() {
        let small = "https://lh3.googleusercontent.com/album=w60-h60-l90-rj";
        assert_eq!(
            display_artwork_url(small, 1024, true),
            "https://lh3.googleusercontent.com/album=w1024-h1024-l90-rj"
        );
        assert_eq!(display_artwork_url(small, 192, false), small);
        assert_eq!(
            display_artwork_url(
                "https://yt3.googleusercontent.com/album=w120-h120-l90-rj",
                1024,
                true
            ),
            "https://yt3.googleusercontent.com/album=w1024-h1024-l90-rj"
        );
        assert_eq!(
            display_artwork_url("https://yt3.ggpht.com/image=s500-c-k?x=1", 192, false),
            "https://yt3.ggpht.com/image=s192-c-k?x=1"
        );
        assert_eq!(
            display_artwork_url("https://example.com/image=w60-h60", 1024, true),
            "https://example.com/image=w60-h60"
        );
        assert_eq!(
            display_artwork_url("https://lh3.googleusercontent.com/image=other", 1024, true),
            "https://lh3.googleusercontent.com/image=other"
        );
    }
    #[test]
    fn spotify_artwork_selects_largest_valid_image_in_any_order() {
        let images = json!([
            {"url":"https://example.test/small","width":64},
            {"url":"https://example.test/large","width":640},
            {"url":"http://example.test/invalid","width":2000},
            {"url":"https://example.test/medium","width":300}
        ]);
        assert_eq!(
            largest_artwork(images.as_array().unwrap().iter()),
            Some("https://example.test/large")
        );
        assert_ne!(
            cover_artwork_key("https://example.test/large"),
            "https://example.test/large"
        );
    }
    #[test]
    #[ignore = "requires the playback DLL and internet; writes only public timing results"]
    fn profile_public_web_cache() {
        let engine = Engine::load().unwrap();
        let mut report = Vec::new();
        for (name, url) in [
            ("explore", "https://www.deezer.com/en/channels/explore"),
            ("podcasts", "https://www.deezer.com/en/channels/podcasts"),
        ] {
            let cold_started = std::time::Instant::now();
            let cold = web_catalog(&engine, url);
            let cold_ms = cold_started.elapsed().as_secs_f64() * 1000.;
            match cold {
                Ok(first) => {
                    let store = session::Store::for_session();
                    let page = Arc::new(Page {
                        title: name.into(),
                        sections: web_sections(&first, false),
                        ..Default::default()
                    });
                    store.begin(1, name.into(), None);
                    store.apply(1, page.clone());
                    store.begin(2, name.into(), None);
                    let warm_started = std::time::Instant::now();
                    let second = store.resume(2).unwrap();
                    let warm_ms = warm_started.elapsed().as_secs_f64() * 1000.;
                    assert_eq!(page.sections, second.sections);
                    report.push(json!({"page":name,"cold_ms":cold_ms,"disk_ms":warm_ms}));
                }
                Err(error) => report.push(json!({"page":name,"cold_ms":cold_ms,"error":error})),
            }
        }
        let path = std::env::var("DEEZNATS_FETCH_PROFILE")
            .expect("Set DEEZNATS_FETCH_PROFILE to an output JSON path");
        std::fs::write(path, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
    }

    #[test]
    fn album_tracks_inherit_cover_and_independent_album_fields() {
        let value = json!({"data":{"album":{"id":"42", "title":"Album", "cover_xl":"https://example.test/album.jpg", "tracks":[
            {"id":"1", "title":"First"},
            {"id":"2", "title":"Second", "albumName":"Known title"},
            {"id":"3", "title":"Third", "artworkUrl":"https://example.test/track.jpg", "albumId":"other"}
        ]}}});
        let mut page = Page {
            entity: entry(&json!({"id":"42","title":"Album"}), "album"),
            rows: table_entries(&value, "track"),
            ..Default::default()
        };
        inherit_collection_metadata(&mut page, &value);
        assert_eq!(page.rows.len(), 3);
        assert_eq!(
            page.entity.as_ref().unwrap().artwork,
            "https://example.test/album.jpg"
        );
        assert_eq!(page.rows[0].artwork, page.rows[1].artwork);
        assert_eq!(
            page.rows[0].wire()["artworkUrl"],
            "https://example.test/album.jpg"
        );
        assert_eq!(page.rows[0].album, "Album");
        assert_eq!(page.rows[1].album, "Known title");
        assert_eq!(page.rows[1].album_id, "42");
        assert_eq!(page.rows[2].album_id, "other");
        assert_eq!(page.rows[2].artwork, "https://example.test/track.jpg");
    }

    #[test]
    fn playlist_tracks_inherit_missing_metadata_without_changing_episodes() {
        let mut page = Page {
            entity: entry(
                &json!({"id":"42","title":"Playlist","cover":"https://example.test/list.jpg"}),
                "playlist",
            ),
            rows: entries(
                &json!([
                    {"id":"1","title":"Track","kind":"track"},
                    {"id":"2","title":"Episode","kind":"episode"}
                ]),
                "track",
            ),
            ..Default::default()
        };
        inherit_collection_metadata(&mut page, &json!({}));
        assert_eq!(page.rows[0].artwork, "https://example.test/list.jpg");
        assert_eq!(page.rows[0].album_id, "42");
        assert!(page.rows[1].artwork.is_empty());
    }

    #[test]
    fn genre_charts_and_channel_shelves_keep_curated_collections() {
        let charts = json!({"tracks":{"data":[{"id":"1","title":"Track"}]},
            "artists":{"data":[{"id":"2","name":"Artist"}]},
            "playlists":{"data":[{"id":"3","title":"Top playlist"}]}});
        let playlists = curated_entries(&charts["playlists"], "playlist");
        assert_eq!(playlists.len(), 1);
        assert_eq!(playlists[0].kind, "playlist");
        let state = json!({"sections":[{"title":"Curated", "items":[
            {"id":"1","title":"Artist","data":{"__TYPE__":"artist"}},
            {"id":"2","title":"100% artist","type":"playlist"},
            {"id":"3","title":"Timeless","type":"album"},
            {"id":"4","title":"Mix","type":"mix"},
            {"id":"6","title":"Show","type":"show","target":"/show/6"}
        ]},{"title":"Artists", "items":[{"id":"5","title":"Raw artist","type":"artist"}]}]});
        let shelves = web_sections(&state, false);
        assert_eq!(shelves.len(), 1);
        assert_eq!(
            shelves[0]
                .entries
                .iter()
                .map(|e| e.kind.as_str())
                .collect::<Vec<_>>(),
            ["playlist", "album", "mix", "podcast"]
        );
    }

    #[test]
    fn discover_filters_distinguish_channel_directories() {
        for (title, filter) in [
            ("Genres", 2),
            ("Categories", 3),
            ("Podcasts by category", 4),
        ] {
            let section = Section {
                title: title.into(),
                entries: entries(
                    &json!([
                        {"type":"channel", "title":"Tile", "target":"/channels/chill"}
                    ]),
                    "channel",
                ),
                ..Default::default()
            };
            assert_eq!(section.entries[0].id, "web:/channels/chill");
            for index in 1..=4 {
                assert_eq!(discover_section_matches(&section, index), index == filter);
            }
            assert!(discover_section_matches(&section, 0));
        }
    }

    #[test]
    fn history_metadata_fills_covers_without_replacing_recorded_fields() {
        let mut row = entry(&json!({"trackId":"123", "title":"Recorded title", "artist":"Recorded artist", "album":"Recorded album", "startedAt":"today"}), "track").unwrap();
        assert!(row.artwork.is_empty());
        assert_eq!(row.subtitle, "Recorded artist");
        assert_eq!(row.album, "Recorded album");
        let metadata = entry(&json!({"id":"123", "name":"Catalog title", "artistLine":"Catalog artist", "artworkUrl":"https://example.test/cover.jpg", "durationMs":120000, "artistId":"456", "albumId":"789"}), "track").unwrap();
        enrich_history_entry(&mut row, &metadata);
        assert_eq!(row.artwork, metadata.artwork);
        assert_eq!(row.title, "Recorded title");
        assert_eq!(row.subtitle, "Recorded artist");
        assert_eq!(row.duration, 120000);
        assert_eq!(row.artist_id, "456");
        assert_eq!(row.album_id, "789");
        assert_eq!(row.raw["startedAt"], "today");
        assert_eq!(row.raw["title"], "Recorded title");
        let mut episode = entry(
            &json!({"trackId":"123", "title":"Episode", "kind":"episode"}),
            "track",
        )
        .unwrap();
        enrich_history_entry(&mut episode, &metadata);
        assert_eq!(episode.kind, "episode");
        assert_eq!(episode.artwork, metadata.artwork);
    }

    #[test]
    fn podcast_library_merges_saved_shows_likes_and_downloads_without_recommendations() {
        let show = Entry {
            id: "show".into(),
            title: "Saved show".into(),
            kind: "podcast".into(),
            ..Default::default()
        };
        let episode = Entry {
            id: "episode".into(),
            title: "Saved episode".into(),
            kind: "episode".into(),
            ..Default::default()
        };
        let music = Entry {
            id: "song".into(),
            kind: "track".into(),
            ..Default::default()
        };
        let page = podcast_library_page(
            vec![show.clone()],
            vec![show, music.clone()],
            vec![episode.clone(), music.clone()],
            vec![episode, music],
        );
        assert_eq!(
            page.sections
                .iter()
                .map(|s| s.title.as_str())
                .collect::<Vec<_>>(),
            vec!["Saved podcasts", "Favorites", "Downloads"]
        );
        assert!(page.sections.iter().all(|s| s.entries.len() == 1));
        assert_eq!(page.sections[0].entries[0].kind, "podcast");
        assert!(page.sections[1..]
            .iter()
            .all(|s| s.entries[0].kind == "episode"));
        let bookmarks = json!({"data":{"me":{"podcastEpisodeBookmarks":{"edges":[{"node":{"episode":{"id":"ep","title":"Liked episode","duration":120,"podcast":{"id":"show","displayTitle":"Show title"}}}}]}}}});
        let parsed = entries(&bookmarks, "episode");
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].subtitle, "Show title");
    }
    #[test]
    fn artist_sections_extract_connections_without_repeating_artist_metadata() {
        for (index, kind, node) in [
            (
                0,
                "track",
                json!({"id":"song","title":"Popular song","duration":200}),
            ),
            (
                1,
                "album",
                json!({"id":"album","displayTitle":"Actual album"}),
            ),
            (
                2,
                "album",
                json!({"id":"single","displayTitle":"Actual single"}),
            ),
            (
                3,
                "album",
                json!({"id":"featured","displayTitle":"Featured album"}),
            ),
            (4, "artist", json!({"id":"related","name":"Another artist"})),
            (
                5,
                "playlist",
                json!({"id":"playlist","title":"Actual playlist","owner":{"name":"Deezer"}}),
            ),
        ] {
            let mut artist = json!({"id":"lamb","name":"Lamb Of God","fansCount":123456,"picture":{"urls":["https://example.test/artist.jpg"]}});
            let connection = json!({"edges":[{"node":node}],"pageInfo":{"hasNextPage":false}});
            if index == 5 {
                artist["playlists"] = json!({"items":connection});
            } else {
                artist["items"] = connection;
            }
            let response = json!({"data":{"artist":artist}});
            let rows = artist_section_entries(&response, index);
            assert_eq!(rows.len(), 1);
            assert_eq!(rows[0].kind, kind);
            assert_ne!(rows[0].id, "lamb");
            assert!(artist_section_entries(
                &json!({"data":{"artist":{"id":"lamb","name":"Lamb Of God","items":{"edges":[]}}}}),
                index
            )
            .is_empty());
        }
        let mut artist = Entry {
            id: "lamb".into(),
            title: "Lamb Of God".into(),
            ..Default::default()
        };
        update_artist_metadata(
            &mut artist,
            &json!({"id":"lamb","name":"Lamb Of God","nbFans":123456,"artworkUrl":"https://example.test/profile.jpg"}),
        );
        assert_eq!(artist.artwork, "https://example.test/profile.jpg");
        assert_eq!(artist.raw["fans"], "123456");
        update_artist_metadata(
            &mut artist,
            &json!({"id":"lamb","name":"Lamb Of God","fansCount":654321}),
        );
        assert_eq!(artist.raw["fans"], "654321");
        assert_eq!(artist.artwork, "https://example.test/profile.jpg");
    }
    #[test]
    fn descriptions_and_marketing_subtitles_never_replace_titles_or_credits() {
        let playlist=entry(&json!({"__typename":"Playlist","id":"p","title":"Hits Remixed","subTitle":"Dance & EDM Editor description","description":"A long playlist description","owner":{"name":"Laeti\n- Deezer"}}),"playlist").unwrap();
        assert_eq!(playlist.title, "Hits Remixed");
        assert_eq!(playlist.subtitle, "Laeti - Deezer");
        assert_eq!(playlist.raw["description"], "A long playlist description");
        let missing=entry(&json!({"id":"p","title":"Techno Tunnel","subTitle":"Description masquerading as a subtitle","description":"About this playlist"}),"playlist").unwrap();
        assert!(missing.subtitle.is_empty());
        assert_eq!(missing.title, "Techno Tunnel");
        let podcast=entry(&json!({"id":"show","name":"Podcast","subtitle":"A very long description","description":"Full details"}),"podcast").unwrap();
        assert!(podcast.subtitle.is_empty());
        assert_eq!(podcast.title, "Podcast");
        let track=entry(&json!({"id":"song","title":"Song","subTitle":"Promotional blurb","artist":{"id":"a","name":"Singer"}}),"track").unwrap();
        assert_eq!(track.subtitle, "Singer");
        let dto = entry(
            &json!({"id":"native","name":"Playlist","owner":"Deezer"}),
            "playlist",
        )
        .unwrap();
        assert_eq!(dto.subtitle, "Deezer");
    }
    #[test]
    fn personalized_mix_shelf_excludes_flow_and_names_daily_mixes() {
        let value = json!({"data":{"me":{"madeForMe":{"edges":[
            {"node":{"__typename":"Flow","id":"flow","title":"Flow"}},
            {"node":{"__typename":"SmartTracklist","id":"mix-a","title":"daily","subTitle":"Featuring Borne, Ivy Lab"}},
            {"node":{"__typename":"SmartTracklist","id":"mix-b","title":"daily","subTitle":"Featuring Eminem, 50 Cent"}},
            {"node":{"__typename":"SmartTracklist","id":"mix-c","title":"Discoveries"}}
        ]}}}});
        let items = home_module_entries(&value, 1, "playlist");
        assert_eq!(
            items.iter().map(|e| e.title.as_str()).collect::<Vec<_>>(),
            vec!["Daily Mix 1", "Daily Mix 2", "Discoveries"]
        );
        assert_eq!(items[0].id, "mix-a");
        assert!(items[1].subtitle.is_empty());
        assert!(home_module_entries(&value, 0, "playlist")
            .iter()
            .any(|e| e.kind == "flow"));
    }
    #[test]
    fn home_album_shelf_keeps_album_identity_under_tracks_wrapper() {
        let value = json!({"tracks": [
            {"id":"764165091", "displayTitle":"Metalicious", "cover":{"urls":["cover"]}},
            {"__typename":"Track", "id":"123", "title":"A track", "duration":180}
        ]});
        let items = home_module_entries(&value, 8, "album");
        assert_eq!(items[0].kind, "album");
        assert_eq!(items[0].id, "764165091");
        assert_eq!(items[1].kind, "track");
    }
    #[test]
    fn fallback_home_mix_dtos_get_numbered_without_marketing_credits() {
        let value = json!([{"id":"flow","kind":"flow","name":"Flow"},
            {"id":"m1","kind":"mix","name":"daily","subtitle":"?n planda Artist"},
            {"id":"m2","kind":"playlist","name":"daily","subtitle":"?n planda Other"}]);
        let mut items = entries(&value, "playlist");
        normalize_daily_mixes(&mut items);
        assert_eq!(
            items.iter().map(|e| e.title.as_str()).collect::<Vec<_>>(),
            vec!["Flow", "Daily Mix 1", "Daily Mix 2"]
        );
        assert!(items[1..]
            .iter()
            .all(|e| e.kind == "mix" && e.subtitle.is_empty()));
    }
    #[test]
    fn smart_mix_rows_come_from_tracks_not_the_collection_header() {
        let response = json!({"data":{"smartTracklist":{"id":"daily","title":"daily","subTitle":"Featuring artists","tracks":{"edges":[
            {"node":{"id":"1","title":"First song","duration":187000,"album":{"id":"a","displayTitle":"Album"},"contributors":{"edges":[{"node":{"id":"singer","name":"Singer"}}]}}},
            {"node":{"id":"2","title":"Second song","duration":180,"contributors":{"edges":[]}}}
        ]}}}});
        let rows = table_entries(&response, "track");
        assert_eq!(
            rows.iter().map(|e| e.id.as_str()).collect::<Vec<_>>(),
            vec!["1", "2"]
        );
        assert_eq!((rows[0].duration, rows[1].duration), (187000, 180000));
        assert_eq!(rows[0].artist_id, "singer");
        assert_eq!(rows[0].subtitle, "Singer");
        assert_eq!(rows[0].album, "Album");
        assert!(rows.iter().all(Entry::playable));
        assert!(table_entries(&json!({"data":{"smartTracklist":{"id":"daily","title":"daily","tracks":{"edges":[]}}}}),"track").is_empty());
        assert_eq!(table_entries(&json!({"id":"playlist","title":"Metadata","tracks":{"data":[{"id":"3","title":"REST track","duration":200}]}}),"track")[0].id,"3");
        assert_eq!(
            table_entries(
                &json!({"episodes":[{"id":"4","title":"Episode","durationMs":1000}]}),
                "episode"
            )[0]
            .kind,
            "episode"
        );
    }
    #[test]
    fn multiple_artist_targets_survive_playback_wire_roundtrip() {
        let track = entry(
            &json!({"id":"song","title":"Together","artistId":"1",
            "artistLine":"First, Second", "contributors":{"edges":[
                {"node":{"id":"1","name":"First"}},
                {"node":{"id":"2","name":"Second"}},
                {"node":{"id":"1","name":"First"}}
            ]}}),
            "track",
        )
        .unwrap();
        let artists = track.artist_entries();
        assert_eq!(
            artists.iter().map(|a| a.id.as_str()).collect::<Vec<_>>(),
            vec!["1", "2"]
        );
        let restored = entry(&track.wire(), "track").unwrap();
        assert_eq!(
            restored
                .artist_entries()
                .iter()
                .map(|a| a.title.as_str())
                .collect::<Vec<_>>(),
            vec!["First", "Second"]
        );
    }

    #[test]
    fn navigation_targets_use_related_ids_and_preserve_metadata() {
        let track=entry(&json!({"id":1,"title":"Song","artist":{"id":7,"name":"Singer","picture_medium":"https://example.test/artist.jpg"},"album":{"id":9,"title":"Album","cover_medium":"https://example.test/album.jpg"}}),"track").unwrap();
        let artist = track.artist_entry().unwrap();
        let album = track.album_entry().unwrap();
        assert_eq!(
            (
                artist.id.as_str(),
                artist.kind.as_str(),
                artist.title.as_str()
            ),
            ("7", "artist", "Singer")
        );
        assert_eq!(
            (album.id.as_str(), album.kind.as_str(), album.title.as_str()),
            ("9", "album", "Album")
        );
        assert!(artist.artwork.ends_with("artist.jpg"));
        assert!(album.artwork.ends_with("album.jpg"));
        assert!(Entry::default().artist_entry().is_none());
        assert!(Entry::default().album_entry().is_none());
    }

    #[test]
    fn artwork_fetch_uses_bounded_cdn_sizes_and_lru_reuses_bitmaps() {
        assert_eq!(
            artwork_url(
                "https://cdn-images.dzcdn.net/images/cover/abc/1000x1000-000000-80-0-0.jpg",
                false
            ),
            "https://cdn-images.dzcdn.net/images/cover/abc/256x256-000000-80-0-0.jpg"
        );
        for url in [
            "https://other.test/1000x1000-a.jpg",
            "https://dzcdn.net.evil.test/1000x1000-a.jpg",
            "https://cdn.dzcdn.net/100x100-a.jpg",
        ] {
            assert_eq!(artwork_url(url, false), url);
        }
        let mut cache = ArtworkCache::default();
        let image = Arc::new(Bitmap::new(1, 1, vec![0]));
        cache.insert("first".into(), false, image.clone());
        assert!(Arc::ptr_eq(&cache.get("first", false).unwrap(), &image));
        assert!(cache.get("first", true).is_none());
        for i in 0..1024 {
            cache.insert(i.to_string(), false, image.clone());
        }
        assert_eq!(cache.items.len(), 1024);
        assert_eq!(cache.bytes, 1024 * 4);
        assert!(cache.get("first", false).is_none());
    }

    #[test]
    fn artwork_cache_retains_pixels_until_explicit_eviction() {
        let mut cache = ArtworkCache::default();
        let image = Arc::new(Bitmap::new(1, 1, vec![0]));
        let allocation = Arc::downgrade(&image);
        cache.insert("cover".into(), false, image.clone());
        let consumer = cache.get("cover", false).unwrap();
        drop(image);
        assert!(allocation.upgrade().is_some());
        drop(consumer);
        assert!(allocation.upgrade().is_some());
        assert!(cache.get("cover", false).is_some());
        cache.trim_to(0);
        assert!(allocation.upgrade().is_none());
        assert_eq!(cache.bytes, 0);
        assert!(cache.items.is_empty());
    }

    #[test]
    fn artwork_resource_identity_survives_redecode_but_changes_with_pixels() {
        let a = Bitmap::new(1, 1, vec![0xff123456]);
        let b = Bitmap::new(1, 1, vec![0xff123456]);
        assert_ne!(a.id, b.id);
        assert_eq!(
            artwork_identity(&a, "cover@192", false),
            artwork_identity(&b, "cover@192", false)
        );
        assert_ne!(
            artwork_identity(&a, "cover@192", false),
            artwork_identity(&b, "other@192", false)
        );
        assert_ne!(
            artwork_identity(&a, "cover@192", false),
            artwork_identity(&Bitmap::new(1, 1, vec![0xff000000]), "cover@192", false)
        );
        let mut cache = ArtworkCache::default();
        cache.insert("pinned".into(), false, Arc::new(a));
        let id = cache.get("pinned", false).unwrap().id;
        cache.pins.insert(id);
        cache.trim_to(0);
        assert_eq!(cache.bytes, 4);
        cache.pins.clear();
        cache.trim_to(0);
        assert_eq!(cache.bytes, 0);
    }

    #[test]
    fn large_cover_requests_upgrade_cdn_resolution_without_affecting_other_hosts() {
        let small = "https://cdn-images.dzcdn.net/images/cover/id/250x250-000000-80-0-0.jpg";
        assert!(display_artwork_url(small, 1024, true).contains("/1024x1024-"));
        assert!(display_artwork_url(small, 4096, true).contains("/1024x1024-"));
        assert_eq!(display_artwork_url(small, 256, false), small);
        let other = "https://example.test/250x250-cover.jpg";
        assert_eq!(display_artwork_url(other, 1024, true), other);
    }

    #[test]
    fn podcast_saves_are_idempotent_and_preserve_counts() {
        let podcast = entry(&json!({"id":"42","kind":"podcast","name":"Show","nb_episodes":"12","description":"About"}), "podcast").unwrap();
        let mut saved = Vec::new();
        edit_podcast_library(&mut saved, podcast.clone(), true).unwrap();
        edit_podcast_library(&mut saved, podcast.clone(), true).unwrap();
        assert_eq!(saved.len(), 1);
        let restored = entry(&saved[0].wire(), "podcast").unwrap();
        assert_eq!(restored.episode_count(), Some(12));
        assert_eq!(restored.raw["description"], "About");
        edit_podcast_library(&mut saved, podcast, false).unwrap();
        assert!(saved.is_empty());
        assert!(edit_podcast_library(
            &mut saved,
            Entry {
                id: "1".into(),
                kind: "track".into(),
                ..Default::default()
            },
            true
        )
        .is_err());
    }

    #[test]
    fn pagination_preserves_order_deduplicates_boundaries_and_stops_cycles() {
        let mut page = Page {
            error: "Transient failure".into(),
            ..Default::default()
        };
        append_page(
            &mut page,
            "page1",
            json!({"data":[{"id":1,"title":"A"},{"id":2,"title":"B"}],"next":"page2"}),
        );
        append_page(
            &mut page,
            "page2",
            json!({"data":[{"id":2,"title":"B"},{"id":3,"title":"C"}],"next":"page1"}),
        );
        assert_eq!(
            page.rows
                .iter()
                .map(|item| item.id.as_str())
                .collect::<Vec<_>>(),
            ["1", "2", "3"]
        );
        assert!(page.next.is_empty());
        assert!(page.error.is_empty());
        append_page(&mut page, "same", json!({"data":[],"next":"same"}));
        assert!(page.next.is_empty());
    }

    #[test]
    fn rest_graphql_and_saved_tracks_keep_the_same_identity() {
        let rest = json!({"id":42,"type":"track","title":"Example","duration":183,
            "artist":{"id":7,"name":"Artist"},
            "album":{"id":9,"title":"Album","cover_medium":"https://example.test/cover.jpg"}});
        let track = entry(&rest, "track").unwrap();
        assert_eq!(track.duration, 183_000);
        assert_eq!(track.subtitle, "Artist");
        let saved = entry(&track.wire(), "track").unwrap();
        assert_eq!(
            (saved.id, saved.artist_id, saved.album_id),
            ("42".into(), "7".into(), "9".into())
        );
        assert_eq!(saved.artwork, track.artwork);
        assert_eq!(saved.duration, track.duration);
        let graphql = json!({"__typename":"Section","id":"section","title":"Shelf",
            "edges":[{"node":{"__typename":"Track","id":"42","title":"Example"}},
                     {"node":{"__typename":"Track","id":"42","title":"Example"}}]});
        let parsed = entries(&graphql, "track");
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].id, "42");
    }

    #[test]
    fn catalog_state_parser_rejects_invalid_input() {
        let html = br#"<script>window.__DZR_APP_STATE__ = {"tracks":[{"id":1,"title":"A; B"}]}; dangerous();</script>"#;
        let state = web_state(html).unwrap();
        assert_eq!(entries(&state, "track")[0].title, "A; B");
        assert!(web_state(b"<html>No catalog</html>").is_err());
        assert!(web_state(b"window.__DZR_APP_STATE__ = broken;").is_err());
    }
}

fn append_flow_rows(page: &mut Page, rows: Vec<Entry>) {
    let mut seen: HashSet<_> = page
        .rows
        .iter()
        .map(|e| (e.kind.clone(), e.id.clone()))
        .collect();
    page.rows.extend(
        rows.into_iter()
            .filter(|e| seen.insert((e.kind.clone(), e.id.clone()))),
    );
}

#[cfg(test)]
mod flow_batch_tests {
    use super::*;
    #[test]
    fn additional_batches_keep_order_and_skip_duplicates() {
        let track = |id: &str| Entry {
            id: id.into(),
            kind: "track".into(),
            ..Default::default()
        };
        let mut page = Page {
            rows: vec![track("1"), track("2")],
            next: "flow:default".into(),
            ..Default::default()
        };
        append_flow_rows(
            &mut page,
            vec![track("2"), track("3"), track("3"), track("4")],
        );
        assert_eq!(
            page.rows.iter().map(|e| e.id.as_str()).collect::<Vec<_>>(),
            ["1", "2", "3", "4"]
        );
        assert_eq!(page.next, "flow:default");
    }
}
