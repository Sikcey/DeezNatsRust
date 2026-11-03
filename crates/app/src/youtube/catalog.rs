//! YouTube Music catalog and radio (GPL-3.0-or-later).
use crate::catalog::{Entry, Event, EventSender, Job, Page, Provider, Section};
use crate::ui::View;
use serde_json::{json, Value};
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc,
};
use ytmusic::{parse, Client, YtMusic};
fn id(kind: &str, value: &str) -> String {
    format!("youtube:{kind}:{value}")
}
pub(super) fn bare(value: &str) -> &str {
    value
        .strip_prefix("youtube:")
        .and_then(|s| s.split_once(':').map(|(_, v)| v))
        .unwrap_or(value)
}
fn cover(images: &[ytmusic::Thumbnail]) -> String {
    ytmusic::best_thumbnail(images)
        .map(|t| t.url.clone())
        .unwrap_or_default()
}
pub(super) fn track(mut t: ytmusic::Track) -> Entry {
    // Some YouTube rows have no linked artist; the upstream fallback also
    // treats the adjacent view count as an artist name.
    t.artists.retain(|artist| {
        let name = artist.name.trim().to_lowercase();
        !(artist.id.is_none()
            && name.ends_with("views")
            && name.chars().any(|c| c.is_ascii_digit()))
    });
    let artists: Vec<Value> = t.artists.iter().map(|a| json!({"id":a.id.as_deref().map(|v| id("artist",v)).unwrap_or_default(),"name":a.name,"kind":"artist"})).collect();
    let artist_id = t
        .artists
        .first()
        .and_then(|a| a.id.as_deref())
        .map(|s| id("artist", s))
        .unwrap_or_default();
    let album = t.album.as_ref().map(|a| a.name.clone()).unwrap_or_default();
    let album_id = t
        .album
        .as_ref()
        .and_then(|a| a.id.as_deref())
        .map(|s| id("album", s))
        .unwrap_or_default();
    Entry {
        id: id("track", t.video_id.as_deref().unwrap_or_default()),
        title: t.title.clone(),
        subtitle: t
            .artists
            .iter()
            .map(|a| a.name.as_str())
            .collect::<Vec<_>>()
            .join(", "),
        artwork: cover(&t.thumbnails),
        kind: "track".into(),
        duration: t.duration.map(|d| d.as_millis() as u64).unwrap_or(0),
        album,
        artist_id,
        album_id,
        raw: json!({"provider":"youtube","artists":artists,"youtube":t}),
    }
}
fn album(a: ytmusic::Album) -> Entry {
    Entry {
        id: id("album", &a.browse_id),
        title: a.title,
        subtitle: a
            .artists
            .iter()
            .map(|a| a.name.as_str())
            .collect::<Vec<_>>()
            .join(", "),
        artwork: cover(&a.thumbnails),
        kind: "album".into(),
        ..Default::default()
    }
}
fn playlist(p: ytmusic::Playlist) -> Entry {
    // YouTube's curated mixes use the subtitle for featured artists, not an
    // owner. The upstream parser otherwise mistakes that text for an author.
    let youtube_curated = p.id.starts_with("RD")
        || p.author.as_deref().is_some_and(|author| {
            matches!(
                author.trim().to_ascii_lowercase().as_str(),
                "youtube" | "youtube music"
            )
        });
    Entry {
        id: id("playlist", &p.id),
        title: p.title,
        subtitle: if youtube_curated {
            String::new()
        } else {
            p.author.unwrap_or_default()
        },
        artwork: cover(&p.thumbnails),
        kind: "playlist".into(),
        raw: json!({"provider":"youtube","owned":p.owned,"public":p.public}),
        ..Default::default()
    }
}
fn playable(t: &ytmusic::Track) -> bool {
    t.available && t.video_id.as_ref().is_some_and(|id| !id.is_empty())
}
// Mixes should contain individual music releases, not arbitrary videos or compilations.
fn mix_song(t: &ytmusic::Track) -> bool {
    playable(t)
        && !t.is_video()
        && t.album
            .as_ref()
            .is_some_and(|album| !album.name.trim().is_empty())
        && t.duration
            .is_some_and(|duration| (30..=900).contains(&duration.as_secs()))
        && t.artists
            .iter()
            .any(|artist| !artist.name.trim().is_empty())
}
fn artist_item(v: &Value) -> Option<Entry> {
    let r = v
        .get("musicResponsiveListItemRenderer")
        .or_else(|| v.get("musicTwoRowItemRenderer"))
        .unwrap_or(v);
    let first = &r["flexColumns"][0]["musicResponsiveListItemFlexColumnRenderer"]["text"];
    let endpoint = r["navigationEndpoint"]["browseEndpoint"]["browseId"]
        .as_str()
        .or_else(|| {
            first["runs"][0]["navigationEndpoint"]["browseEndpoint"]["browseId"].as_str()
        })?;
    if !endpoint.starts_with("UC") {
        return None;
    }
    Some(Entry {
        id: id("artist", endpoint),
        title: parse::runs_text(&r["title"]).or_else(|| parse::runs_text(first))?,
        artwork: cover(&parse::thumbnails(r)),
        kind: "artist".into(),
        ..Default::default()
    })
}
fn item(v: &Value) -> Option<Entry> {
    if let Some(t) = parse::list_item_track(v).filter(playable) {
        return Some(track(t));
    }
    if let Some(a) = parse::two_row_album(v).or_else(|| parse::list_item_album(v)) {
        return Some(album(a));
    }
    if let Some(p) = parse::two_row_playlist(v).or_else(|| parse::list_item_playlist(v)) {
        return Some(playlist(p));
    }
    if let Some(artist) = artist_item(v) {
        return Some(artist);
    }
    let browse = &v["musicTwoRowItemRenderer"];
    if let Some(video) = browse["navigationEndpoint"]["watchEndpoint"]["videoId"]
        .as_str()
        .or_else(|| {
            browse["thumbnailOverlay"]["musicItemThumbnailOverlayRenderer"]["content"]
                ["musicPlayButtonRenderer"]["playNavigationEndpoint"]["watchEndpoint"]["videoId"]
                .as_str()
        })
    {
        let artists = parse::artist_runs(
            browse["subtitle"]["runs"]
                .as_array()
                .map(Vec::as_slice)
                .unwrap_or(&[]),
        );
        let mut entry = track(ytmusic::Track {
            video_id: Some(video.into()),
            title: parse::runs_text(&browse["title"])?,
            artists,
            thumbnails: parse::thumbnails(browse),
            available: true,
            kind: if browse["navigationEndpoint"]["watchEndpoint"]
                ["watchEndpointMusicSupportedConfigs"]["watchEndpointMusicConfig"]["musicVideoType"]
                .as_str()
                .is_some_and(|kind| kind != "MUSIC_VIDEO_TYPE_ATV")
            {
                ytmusic::TrackKind::Video
            } else {
                ytmusic::TrackKind::Song
            },
            ..Default::default()
        });
        let is_radio = [
            &browse["navigationEndpoint"]["watchEndpoint"]["playlistId"],
            &browse["thumbnailOverlay"]["musicItemThumbnailOverlayRenderer"]["content"]
                ["musicPlayButtonRenderer"]["playNavigationEndpoint"]["watchEndpoint"]
                ["playlistId"],
        ]
        .iter()
        .any(|id| id.as_str().is_some_and(|id| id.starts_with("RD")));
        if is_radio {
            entry.subtitle.clear();
        }
        return Some(entry);
    }
    let endpoint = &browse["navigationEndpoint"]["browseEndpoint"];
    let browse_id = endpoint["browseId"].as_str()?;
    let title = parse::runs_text(&browse["title"])?;
    let kind = if browse_id.starts_with("UC") {
        "artist"
    } else {
        "channel"
    };
    Some(Entry {
        id: id(kind, browse_id),
        title,
        kind: kind.into(),
        artwork: cover(&parse::thumbnails(
            &browse["thumbnailRenderer"]["musicThumbnailRenderer"]["thumbnail"],
        )),
        raw: json!({"provider":"youtube","params":endpoint["params"]}),
        ..Default::default()
    })
}
fn feed(value: &Value, page: &mut Page) {
    for shelf in parse::find_renderers(value, "musicCarouselShelfRenderer") {
        let title =
            parse::runs_text(&shelf["header"]["musicCarouselShelfBasicHeaderRenderer"]["title"])
                .unwrap_or_else(|| "YouTube Music".into());
        let entries = shelf["contents"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(item)
            .collect::<Vec<_>>();
        if !entries.is_empty() {
            page.sections.push(Section {
                title,
                entries,
                ..Default::default()
            });
        }
    }
    for shelf in parse::find_renderers(value, "musicShelfRenderer") {
        let entries = shelf["contents"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(item)
            .collect::<Vec<_>>();
        if !entries.is_empty() {
            page.sections.push(Section {
                title: parse::runs_text(&shelf["title"]).unwrap_or_else(|| "Songs".into()),
                entries,
                ..Default::default()
            });
        }
    }
    // Follow the browse continuation rather than silently truncating Home.
    page.next = parse::find_renderers(value, "nextContinuationData")
        .first()
        .and_then(|v| v["continuation"].as_str())
        .map(|c| json!({"browse":c}).to_string())
        .unwrap_or_default();
}
// Artist browse responses include more shelves than the typed artist endpoint exposes.
fn artist_page(value: &Value, page: &mut Page) {
    let header = [
        "musicImmersiveHeaderRenderer",
        "musicVisualHeaderRenderer",
        "musicResponsiveHeaderRenderer",
        "musicHeaderRenderer",
    ]
    .iter()
    .find_map(|name| parse::find_renderer(value, name));
    if let Some(header) = header {
        let text = |key: &str| parse::runs_text(&header[key]).filter(|s| !s.trim().is_empty());
        page.subtitle = text("description")
            .or_else(|| {
                parse::find_renderer(value, "musicDescriptionShelfRenderer")
                    .and_then(|shelf| parse::runs_text(&shelf["description"]))
            })
            .unwrap_or_default();
        if let Some(entity) = page.entity.as_mut() {
            if let Some(audience) = text("straplineTextOne").or_else(|| text("subtitle")) {
                entity.subtitle = audience;
            }
            let thumbnails = parse::thumbnails(header);
            if !thumbnails.is_empty() {
                entity.artwork = cover(&thumbnails);
            }
        }
    }
    if let Some(shelf) = parse::find_renderer(value, "musicShelfRenderer") {
        page.rows = shelf["contents"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(parse::list_item_track)
            .filter(playable)
            .map(track)
            .collect();
    }
    feed(value, page);
    if !page.rows.is_empty() {
        let ids: std::collections::HashSet<_> =
            page.rows.iter().map(|entry| entry.id.as_str()).collect();
        page.sections.retain(|section| {
            !section
                .entries
                .iter()
                .all(|entry| ids.contains(entry.id.as_str()))
        });
    }
}
pub(super) async fn station(
    api: &YtMusic,
    seed: &str,
    continuation: Option<&str>,
) -> anyhow::Result<(Vec<Entry>, String)> {
    let mut body = json!({"videoId":seed,"playlistId":format!("RDAMVM{seed}"),"enablePersistentPlaylistPanel":true,"tunerSettingValue":"AUTOMIX_SETTING_NORMAL"});
    if let Some(c) = continuation {
        body["continuation"] = json!(c);
    }
    let response = api.execute("next", Client::Music, body).await?;
    let panels = parse::find_renderers(
        &response,
        if continuation.is_some() {
            "playlistPanelContinuation"
        } else {
            "playlistPanelRenderer"
        },
    );
    let panel = panels
        .first()
        .ok_or_else(|| anyhow::anyhow!("YouTube returned no radio queue"))?;
    let rows = panel["contents"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(parse::panel_track)
        .filter(|t| t.available && t.video_id.is_some())
        .filter(playable)
        .map(track)
        .collect();
    let next = panel["continuations"]
        .as_array()
        .into_iter()
        .flatten()
        .find_map(|c| c["nextRadioContinuationData"]["continuation"].as_str())
        .map(|c| json!({"seed":seed,"radio":c}).to_string())
        .unwrap_or_default();
    Ok((rows, next))
}
async fn load(
    api: &Arc<YtMusic>,
    view: View,
    entity: Option<Entry>,
    query: String,
) -> anyhow::Result<Page> {
    let mut page = Page {
        title: entity
            .as_ref()
            .map(|e| e.title.clone())
            .unwrap_or_else(|| view.title().into()),
        entity: entity.clone(),
        ..Default::default()
    };
    if !query.is_empty() && view != View::Flow && view != View::StyleMix {
        page.rows = api
            .search_songs(&query)
            .await?
            .into_iter()
            .filter(|t| t.video_id.is_some())
            .filter(playable)
            .map(track)
            .collect();
        page.sections.push(Section {
            title: "Albums".into(),
            entries: api
                .search_albums(&query)
                .await?
                .into_iter()
                .map(album)
                .collect(),
            ..Default::default()
        });
        page.sections.push(Section {
            title: "Playlists".into(),
            entries: api
                .search_playlists(&query)
                .await?
                .into_iter()
                .map(playlist)
                .collect(),
            ..Default::default()
        });
        let artists = api
            .execute(
                "search",
                Client::Music,
                json!({"query":query,"params":ytmusic::search::ARTISTS}),
            )
            .await?;
        page.sections.push(Section {
            title: "Artists".into(),
            entries: parse::find_renderers(&artists, "musicResponsiveListItemRenderer")
                .into_iter()
                .filter_map(artist_item)
                .collect(),
            ..Default::default()
        });
        return Ok(page);
    }
    match view {
        View::Home | View::Explore => {
            let browse = if view == View::Home {
                "FEmusic_home"
            } else {
                "FEmusic_explore"
            };
            let response = api
                .execute("browse", Client::Music, json!({"browseId":browse}))
                .await?;
            feed(&response, &mut page);
            if view == View::Explore {
                page.raw.push(("discover".into(), json!(true)));
                let categories = api
                    .execute(
                        "browse",
                        Client::Music,
                        json!({"browseId":"FEmusic_moods_and_genres"}),
                    )
                    .await?;
                let mut category_shelf = 0;
                for shelf in parse::find_renderers(&categories, "gridRenderer") {
                    let entries=parse::find_renderers(shelf,"musicNavigationButtonRenderer").into_iter().filter_map(|button|{
                        let endpoint=&button["clickCommand"]["browseEndpoint"];
                        let params=endpoint["params"].as_str()?;
                        Some(Entry{id:id("channel",params),title:parse::runs_text(&button["buttonText"])? ,kind:"channel".into(),raw:json!({"provider":"youtube","browse_id":"FEmusic_moods_and_genres_category","params":params}),..Default::default()})
                    }).collect::<Vec<_>>();
                    if !entries.is_empty() {
                        page.sections.push(Section {
                            title: if category_shelf == 0 {
                                "Categories".into()
                            } else {
                                parse::runs_text(&shelf["title"]).unwrap_or_else(|| "Genres".into())
                            },
                            entries,
                            ..Default::default()
                        });
                        category_shelf += 1;
                    }
                }
                // Explore itself often contains only albums and navigation tiles. The mood
                // and genre pages contain YouTube Music's curated playlist shelves.
                let category_entries: Vec<_> = page
                    .sections
                    .iter()
                    .filter(|section| section.title == "Categories" || section.title == "Genres")
                    .flat_map(|section| section.entries.iter())
                    .take(4)
                    .cloned()
                    .collect();
                let mut editorial = Vec::new();
                let mut seen = std::collections::HashSet::new();
                for category in category_entries {
                    if let Ok(response) = api.execute("browse", Client::Music, json!({
                        "browseId": category.raw["browse_id"], "params": category.raw["params"]
                    })).await {
                        let mut category_page = Page::default();
                        feed(&response, &mut category_page);
                        editorial.extend(category_page.sections.into_iter().flat_map(|s| s.entries)
                            .filter(|entry| entry.kind == "playlist" && seen.insert(entry.id.clone())));
                    }
                }
                if !editorial.is_empty() {
                    page.sections.push(Section {
                        title: "Editorial playlists".into(),
                        entries: editorial,
                        ..Default::default()
                    });
                }
            }
        }
        View::Favorites => {
            page.rows = api
                .liked_songs_resolved()
                .await?
                .into_iter()
                .filter(playable)
                .map(track)
                .collect()
        }
        View::Albums => page.sections.push(Section {
            title: "Saved albums".into(),
            entries: api.library_albums().await?.into_iter().map(album).collect(),
            ..Default::default()
        }),
        View::Playlists | View::PlaylistPicker => page.sections.push(Section {
            title: "Your playlists".into(),
            entries: api
                .library_playlists()
                .await?
                .into_iter()
                .map(playlist)
                .collect(),
            ..Default::default()
        }),
        View::Album | View::Artist => {
            let e = entity.ok_or_else(|| anyhow::anyhow!("Choose an album, artist or playlist"))?;
            match e.kind.as_str() {
                "album" => {
                    page.rows = api
                        .album(bare(&e.id))
                        .await?
                        .tracks
                        .into_iter()
                        .filter(playable)
                        .map(track)
                        .collect()
                }
                "artist" => {
                    let response = api
                        .execute("browse", Client::Music, json!({"browseId": bare(&e.id)}))
                        .await?;
                    artist_page(&response, &mut page);
                }
                "channel" => {
                    let response=api.execute("browse",Client::Music,json!({"browseId":e.raw["browse_id"].as_str().unwrap_or_else(||bare(&e.id)),"params":e.raw["params"]})).await?;
                    feed(&response, &mut page);
                }
                _ => {
                    let p = api.playlist_page(bare(&e.id)).await?;
                    page.rows = p.tracks.into_iter().filter(playable).map(track).collect();
                    page.next = p
                        .continuation
                        .map(|c| json!({"playlist":c}).to_string())
                        .unwrap_or_default();
                }
            }
        }
        View::Flow => {
            if query.starts_with("youtube:track:") {
                let seed = if query.starts_with("youtube:track:") {
                    bare(&query).to_string()
                } else {
                    let liked = api.liked_songs().await.unwrap_or_default();
                    liked
                        .into_iter()
                        .find_map(|t| t.video_id)
                        .unwrap_or_default()
                };
                let seed = if seed.is_empty() {
                    api.search_songs("music")
                        .await?
                        .into_iter()
                        .find_map(|t| t.video_id)
                        .ok_or_else(|| anyhow::anyhow!("Choose a song to start radio"))?
                } else {
                    seed
                };
                (page.rows, page.next) = station(api, &seed, None).await?;
                for t in &mut page.rows {
                    t.raw["_youtube_radio"] = json!(true);
                }
                page.subtitle = if page
                    .entity
                    .as_ref()
                    .is_some_and(|entry| entry.kind == "track")
                {
                    "Song radio"
                } else {
                    "YouTube Music radio"
                }
                .into();
            } else {
                (page.rows, page.next) = tuned_flow(api, &query, 0).await?;
                page.subtitle = "YouTube Music playlists for your mood and genres".into();
            }
        }
        View::StyleMix => {
            if !query.is_empty() {
                let selection =
                    crate::style_mix::Selection::parse(&query).map_err(anyhow::Error::msg)?;
                page.title = selection.title();
                let playlists = api.search_playlists(&page.title).await?;
                let mut seen = std::collections::HashSet::new();
                for p in playlists.into_iter().take(3) {
                    if let Ok(detail) = api.playlist_page(&p.id).await {
                        page.rows.extend(
                            detail
                                .tracks
                                .into_iter()
                                .filter(|t| {
                                    mix_song(t)
                                        && t.video_id
                                            .as_ref()
                                            .is_some_and(|id| seen.insert(id.clone()))
                                })
                                .map(track),
                        );
                    }
                }
                if page.rows.is_empty() {
                    page.rows = api
                        .search_songs(&page.title)
                        .await?
                        .into_iter()
                        .filter(mix_song)
                        .map(track)
                        .collect();
                }
                for t in &mut page.rows {
                    t.raw[crate::style_mix::MARKER] = json!(selection.key());
                }
            }
        }
        View::Recent => {
            page.rows = super::history();
        }
        View::Account | View::Settings | View::FlowTune => {}
        _ => anyhow::bail!("{} is not available for YouTube Music", view.title()),
    }
    Ok(page)
}
// Bound each batch while rotating through every selected genre and deeper playlists.
// Playlist search is used because YouTube genre radio is not implemented.
pub(super) async fn continue_flow(
    api: &YtMusic,
    key: &str,
    offset: usize,
    mut known: std::collections::HashSet<String>,
) -> anyhow::Result<(Vec<Entry>, String)> {
    tokio::time::timeout(std::time::Duration::from_secs(85), async {
        let mut offset = offset;
        for _ in 0..3 {
            let (rows, next) = tuned_flow(api, key, offset).await?;
            let rows: Vec<_> = rows
                .into_iter()
                .filter(|row| known.insert(row.id.clone()))
                .collect();
            if !rows.is_empty() {
                return Ok((rows, next));
            }
            let cursor: Value = serde_json::from_str(&next)?;
            offset = cursor["offset"].as_u64().unwrap_or(0) as usize;
        }
        anyhow::bail!(
            "No new tracks found for this YouTube Flow selection. Try another mood or genre."
        )
    })
    .await
    .map_err(|_| anyhow::anyhow!("YouTube Flow continuation timed out"))?
}

pub(super) async fn tuned_flow(
    api: &YtMusic,
    key: &str,
    offset: usize,
) -> anyhow::Result<(Vec<Entry>, String)> {
    tokio::time::timeout(
        std::time::Duration::from_secs(75),
        tuned_flow_batch(api, key, offset),
    )
    .await
    .map_err(|_| anyhow::anyhow!("YouTube Flow timed out; try again or select fewer genres"))?
}

async fn tuned_flow_batch(
    api: &YtMusic,
    key: &str,
    offset: usize,
) -> anyhow::Result<(Vec<Entry>, String)> {
    let selection = crate::spotify::flow::Selection::parse(key).map_err(anyhow::Error::msg)?;
    let queries = selection.queries();
    let mut lanes = Vec::new();
    let mut last_error = String::new();
    for (query, rank) in flow_query_plan(&queries, offset) {
        let mut rows = Vec::new();
        match api.search_playlists(&query).await {
            Ok(playlists) => {
                for playlist in playlists.into_iter().skip(rank).take(2) {
                    match api.playlist_page(&playlist.id).await {
                        Ok(page) => {
                            rows = page
                                .tracks
                                .into_iter()
                                .filter(mix_song)
                                .map(track)
                                .take(20)
                                .collect();
                            if !rows.is_empty() {
                                break;
                            }
                        }
                        Err(error) => last_error = error.to_string(),
                    }
                }
            }
            Err(error) => last_error = error.to_string(),
        }
        if rows.is_empty() {
            match api.search_songs(&query).await {
                Ok(tracks) => {
                    rows = tracks
                        .into_iter()
                        .filter(mix_song)
                        .map(track)
                        .take(20)
                        .collect()
                }
                Err(error) => last_error = error.to_string(),
            }
        }
        lanes.push(rows);
    }
    let next_offset = offset.saturating_add(queries.len().min(4));
    let mut rows = merge_flow(lanes);
    anyhow::ensure!(!rows.is_empty(), "No playable YouTube playlists or songs for this Flow selection. Try another mood or genre. {last_error}");
    let key = selection.key();
    for row in &mut rows {
        row.raw["_youtube_radio"] = json!(true);
        row.raw["_youtube_flow"] = json!(key);
        row.raw["_youtube_flow_offset"] = json!(next_offset);
    }
    Ok((rows, json!({"flow":key,"offset":next_offset}).to_string()))
}

fn flow_query_plan(queries: &[String], offset: usize) -> Vec<(String, usize)> {
    if queries.is_empty() {
        return Vec::new();
    }
    (0..queries.len().min(4))
        .map(|step| {
            let position = offset.saturating_add(step);
            (
                queries[position % queries.len()].clone(),
                (position / queries.len()) % 8,
            )
        })
        .collect()
}

fn merge_flow(lanes: Vec<Vec<Entry>>) -> Vec<Entry> {
    let mut seen = std::collections::HashSet::new();
    let mut rows = Vec::new();
    for index in 0..lanes.iter().map(Vec::len).max().unwrap_or(0) {
        for lane in &lanes {
            if let Some(row) = lane.get(index) {
                if seen.insert(row.id.clone()) {
                    rows.push(row.clone());
                }
            }
        }
    }
    rows
}

async fn more(api: &YtMusic, page: &mut Page) -> anyhow::Result<()> {
    let cursor: Value = serde_json::from_str(&page.next)?;
    if let Some(key) = cursor["flow"].as_str() {
        let known = page.rows.iter().map(|t| t.id.clone()).collect();
        let (rows, next) = continue_flow(
            api,
            key,
            cursor["offset"].as_u64().unwrap_or(0) as usize,
            known,
        )
        .await?;
        page.rows.extend(rows);
        page.next = next;
    } else if let Some(c) = cursor["playlist"].as_str() {
        let p = api.playlist_continuation(c).await?;
        page.rows
            .extend(p.tracks.into_iter().filter(playable).map(track));
        page.next = p
            .continuation
            .map(|c| json!({"playlist":c}).to_string())
            .unwrap_or_default();
    } else if let Some(c) = cursor["radio"].as_str() {
        let (rows, next) =
            station(api, cursor["seed"].as_str().unwrap_or_default(), Some(c)).await?;
        let known = page
            .rows
            .iter()
            .map(|t| t.id.clone())
            .collect::<std::collections::HashSet<_>>();
        page.rows
            .extend(rows.into_iter().filter(|t| !known.contains(&t.id)));
        page.next = next;
    } else if let Some(c) = cursor["browse"].as_str() {
        let r = api
            .execute("browse", Client::Music, json!({"continuation":c}))
            .await?;
        feed(&r, page);
    }
    Ok(())
}
pub(crate) fn process(generation: u64, job: Job, tx: &EventSender, current: &AtomicU64) {
    let result = match job {
        Job::Route(view, entity, query) => super::api().and_then(|api| {
            super::runtime()
                .block_on(async {
                    tokio::time::timeout(
                        std::time::Duration::from_secs(90),
                        load(&api, view, entity, query),
                    )
                    .await
                    .map_err(|_| anyhow::anyhow!("YouTube catalog timed out"))?
                })
                .map_err(|e| e.to_string())
        }),
        Job::More(mut page) => super::api().and_then(|api| {
            super::runtime()
                .block_on(more(&api, &mut page))
                .map_err(|e| e.to_string())?;
            Ok(page)
        }),
        Job::Api(name, args) => {
            let _ = tx.send(Event::Api(
                generation,
                Provider::YouTube,
                name.clone(),
                invoke(&name, &args),
            ));
            return;
        }
        Job::TrackInfo(t) => {
            let _ = tx.send(Event::TrackInfo(
                generation,
                Box::new(t.clone()),
                crate::catalog::musicbrainz::enrich(&t, t.raw.clone()),
                Value::Null,
            ));
            return;
        }
        Job::Playlist(mut mutation) => {
            let result = edit(&mut mutation);
            let _ = tx.send(Event::Playlist(
                generation,
                Provider::YouTube,
                mutation,
                result,
            ));
            return;
        }
        _ => return,
    };
    if current.load(Ordering::Relaxed) == generation {
        let page = result.unwrap_or_else(|error| Page {
            error,
            ..Default::default()
        });
        let _ = tx.send(Event::Page(generation, Arc::new(page)));
    }
}
pub(crate) fn invoke(name: &str, args: &[Value]) -> Result<Value, String> {
    if name.ends_with("StyleMixGenres") || name == "StyleMixGenres" {
        return crate::style_mix::genre_options(
            args.first().and_then(Value::as_u64).unwrap_or(0) as usize
        );
    }
    match name {
        "DZFlowConfigsJSON" => {
            return Ok(
                json!({"options": crate::spotify::flow::MOODS.iter().map(|(id, title)| json!({"id":id,"title":title})).collect::<Vec<_>>()}),
            )
        }
        "DZFlowTunerJSON" | "DZFlowConfigTunerJSON" => {
            let key = args.first().and_then(Value::as_str).unwrap_or("default");
            return Ok(crate::spotify::flow::Selection::parse(key)?.tuner(key));
        }
        "DZSaveFlowTunerJSON" | "DZFlowConfigSaveJSON" => {
            return crate::spotify::flow::save(
                args.first().and_then(Value::as_str).unwrap_or("default"),
                args.get(1).and_then(Value::as_str).unwrap_or("[]"),
            )
        }
        _ => {}
    }
    let api = super::api()?;
    super::runtime().block_on(async {
        match name {
            "DZFavoriteTracksJSON" => Ok(json!({"data":api.liked_songs_resolved().await?.into_iter().filter(playable).map(track).map(|t|json!({"id":t.id,"title":t.title,"kind":t.kind,"provider":"youtube"})).collect::<Vec<_>>()})),
            "DZAddFavorite" | "DZRemoveFavorite" => {api.rate_track(bare(args.first().and_then(Value::as_str).unwrap_or_default()),name=="DZAddFavorite").await?;Ok(json!(true))},
            "DZAlbumLibraryJSON" => {
                let album=api.album(bare(args.first().and_then(Value::as_str).unwrap_or_default())).await?;
                let playlist=album.album.playlist_id.ok_or_else(||anyhow::anyhow!("YouTube did not return an album playlist ID"))?;
                api.rate_playlist(&playlist,args.get(1).and_then(Value::as_i64).unwrap_or(0)!=0).await?;
                Ok(json!(true))
            },
            "DZPlaylistLibraryJSON" => {api.rate_playlist(bare(args.first().and_then(Value::as_str).unwrap_or_default()),args.get(1).and_then(Value::as_i64).unwrap_or(0)!=0).await?;Ok(json!(true))},
            "DZLibraryJSON" => Ok(json!({"tracks":api.liked_songs().await?.into_iter().filter_map(|t|t.video_id).map(|s|json!({"id":id("track",&s),"title":"Song","kind":"track"})).collect::<Vec<_>>()})),
            _ => anyhow::bail!("YouTube Music does not support {name}"),
        }
    }).map_err(|e:anyhow::Error|e.to_string())
}
#[cfg(test)]
mod tests {
    #[test]
    fn track_view_count_is_not_an_artist() {
        let entry = super::track(ytmusic::Track {
            artists: vec![
                ytmusic::ArtistRef {
                    name: "Aykut Aks?z".into(),
                    id: None,
                },
                ytmusic::ArtistRef {
                    name: "12M views".into(),
                    id: None,
                },
            ],
            ..Default::default()
        });
        assert_eq!(entry.subtitle, "Aykut Aks?z");
        assert_eq!(entry.raw["artists"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn artist_page_keeps_bio_and_all_carousel_kinds() {
        let response = serde_json::json!({
            "header": {"musicImmersiveHeaderRenderer": {
                "description": {"runs": [{"text": "Artist biography"}]},
                "straplineTextOne": {"runs": [{"text": "608K monthly audience"}]}
            }},
            "shelves": [{"musicCarouselShelfRenderer": {
                "header": {"musicCarouselShelfBasicHeaderRenderer": {"title": {"runs": [{"text": "Featured on"}]}}},
                "contents": [{"musicTwoRowItemRenderer": {
                    "title": {"runs": [{"text": "Playlist"}]},
                    "navigationEndpoint": {"browseEndpoint": {"browseId": "VLPL123"}}
                }}]
            }}, {"musicCarouselShelfRenderer": {
                "header": {"musicCarouselShelfBasicHeaderRenderer": {"title": {"runs": [{"text": "Similar artists"}]}}},
                "contents": [{"musicTwoRowItemRenderer": {
                    "title": {"runs": [{"text": "Other artist"}]},
                    "navigationEndpoint": {"browseEndpoint": {"browseId": "UC123"}}
                }}]
            }}]
        });
        let mut page = crate::catalog::Page {
            entity: Some(crate::catalog::Entry {
                kind: "artist".into(),
                ..Default::default()
            }),
            ..Default::default()
        };
        super::artist_page(&response, &mut page);
        assert_eq!(page.subtitle, "Artist biography");
        assert_eq!(page.entity.unwrap().subtitle, "608K monthly audience");
        assert_eq!(page.sections.len(), 2);
        assert_eq!(page.sections[0].entries[0].kind, "playlist");
        assert_eq!(page.sections[1].entries[0].kind, "artist");
    }

    #[test]
    fn radio_home_cards_hide_featured_artists() {
        let card = |playlist_id: &str| {
            serde_json::json!({
                "musicTwoRowItemRenderer": {
                    "title": {"runs": [{"text": "Replay Mix"}]},
                    "subtitle": {"runs": [{"text": "Featured artist"}]},
                    "navigationEndpoint": {"watchEndpoint": {"videoId": "video", "playlistId": playlist_id}}
                }
            })
        };
        assert!(super::item(&card("RDTMAK5uy_example"))
            .unwrap()
            .subtitle
            .is_empty());
    }

    #[test]
    #[ignore = "requires a signed-in YouTube Music session and network access"]
    fn live_song_artwork_can_be_requested_large() {
        let api = super::super::api().expect("Sign in to YouTube Music first");
        super::super::runtime().block_on(async {
            let songs = api.search_songs("Daft Punk Get Lucky").await.unwrap();
            let url = songs
                .iter()
                .map(|song| super::cover(&song.thumbnails))
                .find(|url| !url.is_empty())
                .expect("no song artwork");
            let large = crate::catalog::display_artwork_url(&url, 1024, true);
            let bytes = crate::lyrics::http::artwork(&large).unwrap();
            let image = image::load_from_memory(&bytes).unwrap();
            assert!(
                image.width() >= 500 && image.height() >= 500,
                "artwork was {}x{}",
                image.width(),
                image.height()
            );
        });
    }
    #[test]
    fn mix_song_rejects_videos_compilations_and_missing_music_metadata() {
        let song = ytmusic::Track {
            video_id: Some("song".into()),
            available: true,
            duration: Some(std::time::Duration::from_secs(210)),
            album: Some(ytmusic::AlbumRef {
                name: "Album".into(),
                id: None,
            }),
            artists: vec![ytmusic::ArtistRef {
                name: "Artist".into(),
                id: None,
            }],
            ..Default::default()
        };
        assert!(super::mix_song(&song));
        let mut other = song.clone();
        other.kind = ytmusic::TrackKind::Video;
        assert!(!super::mix_song(&other));
        other = song.clone();
        other.duration = Some(std::time::Duration::from_secs(89 * 60));
        assert!(!super::mix_song(&other));
        other = song.clone();
        other.album = None;
        assert!(!super::mix_song(&other));
        other = song.clone();
        other.duration = None;
        assert!(!super::mix_song(&other));
        other = song;
        other.available = false;
        assert!(!super::mix_song(&other));
    }
    #[test]
    fn playlist_subtitle_excludes_youtube_curators_and_featured_artists() {
        let make = |id: &str, author: &str| {
            super::playlist(ytmusic::Playlist {
                id: id.into(),
                title: "Playlist".into(),
                author: Some(author.into()),
                owned: false,
                public: None,
                track_count: None,
                thumbnails: vec![],
            })
        };
        assert!(make("RDCLAK5uy_example", "Lewis Capaldi")
            .subtitle
            .is_empty());
        assert!(make("PLexample", "YouTube Music").subtitle.is_empty());
        assert_eq!(make("PLexample", "A listener").subtitle, "A listener");
    }
    use super::*;
    #[test]
    fn flow_batches_rotate_all_selected_genres_and_deeper_playlists() {
        let selection = crate::spotify::flow::Selection {
            mood: "chill".into(),
            genres: vec!["Jazz", "Rock", "Pop", "Ambient", "Soul", "Funk"]
                .into_iter()
                .map(str::to_owned)
                .collect(),
        };
        let queries = selection.queries();
        let first = flow_query_plan(&queries, 0);
        let next = flow_query_plan(&queries, 4);
        assert_eq!(first.len(), 4);
        assert_eq!(first[0], ("chill Jazz".into(), 0));
        assert_eq!(next[0], ("chill Soul".into(), 0));
        assert_eq!(next[2], ("chill Jazz".into(), 1));
        assert!(flow_query_plan(&[], 0).is_empty());
    }

    #[test]
    fn flow_picker_and_validation_work_without_network() {
        let options = invoke("DZFlowConfigsJSON", &[]).unwrap();
        assert_eq!(
            options["options"].as_array().unwrap().len(),
            crate::spotify::flow::MOODS.len()
        );
        let tuner = invoke("DZFlowTunerJSON", &[json!("focus")]).unwrap();
        assert_eq!(tuner["customizable"], true);
        assert!(tuner["genres"]
            .as_array()
            .unwrap()
            .iter()
            .any(|g| g["id"] == "Ambient" && g["isEnabled"] == true));
        let saved = invoke(
            "DZSaveFlowTunerJSON",
            &[
                json!("focus"),
                json!(r#"[{"clusterId":"Jazz","isEnabled":true}]"#),
            ],
        )
        .unwrap();
        let selection =
            crate::spotify::flow::Selection::parse(saved["id"].as_str().unwrap()).unwrap();
        assert!(selection.queries().contains(&"focus Jazz".to_string()));
        assert!(invoke("DZFlowTunerJSON", &[json!("invalid-mood")]).is_err());
        assert!(invoke("DZSaveFlowTunerJSON", &[json!("focus"), json!("invalid")]).is_err());
    }

    #[test]
    fn flow_interleaves_genres_and_deduplicates_tracks() {
        let entry = |id: &str| Entry {
            id: id.into(),
            ..Default::default()
        };
        let rows = merge_flow(vec![
            vec![entry("a"), entry("b")],
            vec![entry("c"), entry("a"), entry("d")],
        ]);
        assert_eq!(
            rows.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(),
            vec!["a", "c", "b", "d"]
        );
        assert!(merge_flow(vec![vec![]]).is_empty());
    }

    #[test]
    #[ignore = "requires access to YouTube Music"]
    fn live_tuned_flow() {
        super::super::runtime().block_on(async {
            let api = super::super::api().expect("Sign in to YouTube Music first");
            let key = crate::spotify::flow::Selection {
                mood: "focus".into(),
                genres: vec!["Jazz".into()],
            }
            .key();
            let (rows, cursor) = tuned_flow(&api, &key, 0).await.unwrap();
            assert!(!rows.is_empty());
            assert!(rows.iter().all(|r| r.raw["_youtube_flow"] == key));
            let cursor: Value = serde_json::from_str(&cursor).unwrap();
            assert_eq!(cursor["flow"], key);
            assert_eq!(cursor["offset"], 1);
            let known = rows.iter().map(|r| r.id.clone()).collect();
            let (next, _) = continue_flow(&api, &key, 1, known).await.unwrap();
            assert!(next.iter().all(|r| !rows.iter().any(|old| old.id == r.id)));
            assert!(!next.is_empty());
        });
    }

    #[test]
    #[ignore = "requires access to YouTube Music"]
    fn live_authenticated_catalog_and_audio() {
        let api = super::super::api().expect("Sign in to YouTube Music first");
        super::super::runtime().block_on(async {
            let home = load(&api, View::Home, None, String::new()).await.unwrap();
            assert!(!home.sections.is_empty());
            let explore = load(&api, View::Explore, None, String::new())
                .await
                .unwrap();
            assert!(!explore.sections.is_empty());
            let category = explore
                .sections
                .iter()
                .flat_map(|s| &s.entries)
                .find(|e| e.kind == "channel" && e.raw["browse_id"].is_string())
                .unwrap()
                .clone();
            let category_page = load(&api, View::Album, Some(category), String::new())
                .await
                .unwrap();
            assert!(!category_page.sections.is_empty());
            let albums = api.search_albums("Discovery Daft Punk").await.unwrap();
            let album_page = load(
                &api,
                View::Album,
                Some(album(albums.into_iter().next().unwrap())),
                String::new(),
            )
            .await
            .unwrap();
            assert!(!album_page.rows.is_empty());
            let playlists = api.search_playlists("Turkish rock").await.unwrap();
            let playlist_page = load(
                &api,
                View::Album,
                Some(playlist(playlists.into_iter().next().unwrap())),
                String::new(),
            )
            .await
            .unwrap();
            assert!(!playlist_page.rows.is_empty());
            let mix = load(&api, View::StyleMix, None, "0:0".into())
                .await
                .unwrap();
            assert!(!mix.rows.is_empty());
            let found = api.search_songs("Daft Punk Get Lucky").await.unwrap();
            assert!(!found.is_empty());
            let seed = found.iter().find_map(|t| t.video_id.as_deref()).unwrap();
            let (radio, _) = station(&api, seed, None).await.unwrap();
            assert!(!radio.is_empty());
            let (_, mut stream) = api.open_audio(seed).await.unwrap();
            assert!(!stream.chunk().await.unwrap().unwrap().is_empty());
            let (_, bytes) = api.load_audio(seed).await.unwrap();
            let mut decoded = rodio::Decoder::try_from(std::io::Cursor::new(bytes)).unwrap();
            assert!(decoded.next().is_some(), "audio must decode to PCM samples");
        });
    }
    #[test]
    fn responsive_artist_is_not_an_unplayable_song() {
        let v = json!({"musicResponsiveListItemRenderer":{"navigationEndpoint":{"browseEndpoint":{"browseId":"UC123"}},"flexColumns":[{"musicResponsiveListItemFlexColumnRenderer":{"text":{"runs":[{"text":"Artist"}]}}}]}});
        let e = item(&v).unwrap();
        assert_eq!(e.kind, "artist");
        assert_eq!(e.id, "youtube:artist:UC123");
    }
    #[test]
    fn ids_are_namespaced() {
        assert_eq!(bare("youtube:track:abc"), "abc");
        assert_eq!(id("album", "MPREb"), "youtube:album:MPREb");
    }
    #[test]
    fn track_metadata_keeps_credits() {
        let e = track(ytmusic::Track {
            video_id: Some("abc".into()),
            title: "Song".into(),
            artists: vec![ytmusic::ArtistRef {
                name: "Artist".into(),
                id: Some("UC1".into()),
            }],
            duration: Some(std::time::Duration::from_secs(123)),
            available: true,
            ..Default::default()
        });
        assert_eq!(e.duration, 123000);
        assert_eq!(e.artist_entries()[0].id, "youtube:artist:UC1");
    }
}

pub(crate) fn menu_playlists() -> Result<Vec<Entry>, String> {
    let api = super::api()?;
    super::runtime()
        .block_on(async {
            // library_playlists follows every library-grid continuation internally.
            api.library_playlists().await.map(|items| {
                items
                    .into_iter()
                    .filter(|p| p.owned)
                    .map(playlist)
                    .collect()
            })
        })
        .map_err(|error| error.to_string())
}

pub(crate) fn playlist_privacy(id: &str) -> Result<Option<bool>, String> {
    let api = super::api()?;
    super::runtime()
        .block_on(async {
            api.playlist_page(bare(id))
                .await
                .map(|page| page.playlist.public)
        })
        .map_err(|error| error.to_string())
}

pub(crate) fn saved_library_entries() -> Result<Vec<Entry>, String> {
    let api = super::api()?;
    super::runtime()
        .block_on(async {
            let mut entries: Vec<_> = api
                .library_playlists()
                .await?
                .into_iter()
                .map(playlist)
                .collect();
            entries.extend(api.library_albums().await?.into_iter().map(album));
            entries.extend(
                api.liked_songs()
                    .await?
                    .into_iter()
                    .filter(playable)
                    .map(track),
            );
            Ok(entries)
        })
        .map_err(|error: anyhow::Error| error.to_string())
}

pub(crate) fn collection_tracks(entry: &Entry) -> Result<Vec<Entry>, String> {
    let api = super::api()?;
    super::runtime()
        .block_on(async {
            let mut page = load(&api, View::Album, Some(entry.clone()), String::new()).await?;
            let mut cursors = std::collections::HashSet::new();
            while !page.next.is_empty() && cursors.insert(page.next.clone()) {
                more(&api, &mut page).await?;
            }
            if page.rows.is_empty() {
                anyhow::bail!("This collection has no playable tracks");
            }
            Ok(page.rows)
        })
        .map_err(|error: anyhow::Error| error.to_string())
}

pub(crate) fn edit(
    mutation: &mut crate::playlist::Mutation,
) -> Result<crate::playlist::Outcome, String> {
    use crate::playlist::{Mutation, Outcome};
    let api = super::api()?;
    super::runtime()
        .block_on(async {
            let (result, deleted) = match mutation {
                Mutation::Save {
                    id: playlist_id,
                    title,
                    description,
                    cover,
                    seed,
                public,
                } => {
                    if title.trim().is_empty() {
                        anyhow::bail!("Enter a playlist name");
                    }
                    if cover.is_some() {
                        anyhow::bail!("YouTube Music does not support cover changes here");
                    }
                    let creating = playlist_id.is_empty();
                    if creating {
                        *playlist_id = id("playlist", &api.create_playlist(title.trim()).await?);
                    } else {
                        api.rename_playlist(bare(playlist_id), title.trim()).await?;
                    }
                    if let Some(value) = description.as_ref().filter(|value| !creating || !value.is_empty()) {
                        api.execute("browse/edit_playlist", Client::Music, json!({
                            "playlistId": bare(playlist_id).trim_start_matches("VL"),
                            "actions": [{"action": "ACTION_SET_PLAYLIST_DESCRIPTION", "playlistDescription": value}]
                        })).await?;
                        *description = None;
                    }
                if let Some(target) = *public { api.set_playlist_privacy(bare(playlist_id),target).await?; *public=None; }
                if let Some(track) = seed {
                        api.add_playlist_track(bare(playlist_id), bare(track))
                            .await?;
                        *seed = None;
                    }
                    (playlist_id.clone(), false)
                }
                Mutation::Add(p, t) => {
                    api.add_playlist_track(bare(p), bare(t)).await?;
                    (p.clone(), false)
                }
                Mutation::Remove(p, t) => {
                    let detail = api.playlist(bare(p)).await?;
                    let found = detail
                        .tracks
                        .iter()
                        .find(|row| row.video_id.as_deref() == Some(bare(t)))
                        .ok_or_else(|| anyhow::anyhow!("Track is no longer in this playlist"))?;
                    let set = found.set_video_id.as_deref().ok_or_else(|| {
                        anyhow::anyhow!("YouTube did not return the playlist item ID")
                    })?;
                    api.remove_playlist_track(bare(p), bare(t), set).await?;
                    (p.clone(), false)
                }
                Mutation::Delete(p) => {
                    api.delete_playlist(bare(p)).await?;
                    (p.clone(), true)
                }
            };
            Ok(Outcome {
                id: result,
                deleted,
            })
        })
        .map_err(|e: anyhow::Error| e.to_string())
}
