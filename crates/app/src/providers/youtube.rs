//! YouTube Music adapter (GPL-3.0-or-later).
mod auth {
    //! GPL-3.0-or-later.
    use anyhow::{bail, Result};

    /// The cookies that tell a signed-in header from a guest one.
    pub(crate) const PROOF: &[&str] = &["SAPISID", "__Secure-3PAPISID"];

    /// Normalizes a `Cookie` header value to `name=value` pairs joined by `; ` and refuses one that
    /// carries no proof of a signed-in account.
    pub fn header(input: &str) -> Result<String> {
        let pairs: Vec<&str> = input
            .split(';')
            .map(str::trim)
            .filter(|pair| pair.contains('=') && !pair.contains(char::is_whitespace))
            .collect();
        let signed_in = pairs
            .iter()
            .filter_map(|pair| pair.split_once('='))
            .any(|(name, _)| PROOF.contains(&name));
        if !signed_in {
            bail!("the cookies carry no SAPISID or __Secure-3PAPISID");
        }
        Ok(pairs.join("; "))
    }

    #[cfg(test)]
    mod tests {
        use super::header;

        #[test]
        fn keeps_a_signed_in_header() {
            let value = header("VISITOR_INFO1_LIVE=abc;  SAPISID=xyz; PREF=f1").unwrap();
            assert_eq!(value, "VISITOR_INFO1_LIVE=abc; SAPISID=xyz; PREF=f1");
        }

        #[test]
        fn accepts_the_secure_variant_alone() {
            assert!(header("__Secure-3PAPISID=xyz").is_ok());
        }

        #[test]
        fn rejects_a_signed_out_header() {
            assert!(header("VISITOR_INFO1_LIVE=abc; PREF=f1").is_err());
        }

        #[test]
        fn rejects_an_empty_header() {
            assert!(header("   ").is_err());
        }
    }
}
mod catalog {
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
                    ["musicPlayButtonRenderer"]["playNavigationEndpoint"]["watchEndpoint"]
                    ["videoId"]
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
                    ["watchEndpointMusicSupportedConfigs"]["watchEndpointMusicConfig"]
                    ["musicVideoType"]
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
            let title = parse::runs_text(
                &shelf["header"]["musicCarouselShelfBasicHeaderRenderer"]["title"],
            )
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
                                    parse::runs_text(&shelf["title"])
                                        .unwrap_or_else(|| "Genres".into())
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
                        .filter(|section| {
                            section.title == "Categories" || section.title == "Genres"
                        })
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
                let e =
                    entity.ok_or_else(|| anyhow::anyhow!("Choose an album, artist or playlist"))?;
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
                    page.subtitle = "YouTube Music radio for your mood and genres".into();
                }
            }
            View::StyleMix => {
                if !query.is_empty() {
                    let selection =
                        crate::style_mix::Selection::parse(&query).map_err(anyhow::Error::msg)?;
                    page.title = selection.title();
                    page.rows = style_tracks(api, selection).await?;
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
    // Seeds are selected from matching playlists and expanded with YouTube radio.
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
            let rows = match mix_lane(api, &query, rank).await {
                Ok(rows) => rows,
                Err(error) => {
                    last_error = error.to_string();
                    Vec::new()
                }
            };
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

    fn style_genres(region: usize, current: impl Fn() -> bool + Sync) -> Result<Value, String> {
        crate::style_mix::Selection::new(region, 0)?;
        let api = super::api()?;
        let started = std::time::Instant::now();
        crate::style_mix::discover_genres(
            region,
            |selection| {
                if !current() {
                    return Err("Superseded genre check".into());
                }
                let remaining =
                    std::time::Duration::from_secs(85).saturating_sub(started.elapsed());
                if remaining.is_zero() {
                    return Err("YouTube genre check timed out".into());
                }
                super::runtime().block_on(async {
                    tokio::time::timeout(
                        remaining.min(std::time::Duration::from_secs(20)),
                        style_tracks(&api, selection),
                    )
                    .await
                    .map_err(|_| "YouTube genre check timed out".to_string())?
                    .map(|rows| !rows.is_empty())
                    .map_err(|e| e.to_string())
                })
            },
            &current,
        )
    }

    // Constrained mixes stay inside playlists matching both language and genre.
    async fn style_tracks(
        api: &YtMusic,
        selection: crate::style_mix::Selection,
    ) -> anyhow::Result<Vec<Entry>> {
        let playlists = api.search_playlists(&selection.title()).await?;
        let mut rows = Vec::new();
        let mut seen = std::collections::HashSet::new();
        let mut failure = None;
        for playlist in playlists
            .into_iter()
            .filter(|p| selection.matches(&p.title))
            .take(3)
        {
            match api.playlist_page(&playlist.id).await {
                Ok(page) => rows.extend(page.tracks.into_iter().filter(mix_song).filter_map(|t| {
                    let mut entry = track(t);
                    entry.raw[crate::style_mix::MARKER] = json!(selection.key());
                    seen.insert(entry.id.clone()).then_some(entry)
                })),
                Err(error) => failure = Some(error),
            }
        }
        if rows.is_empty() {
            if let Some(error) = failure {
                return Err(error);
            }
        }
        rows.truncate(40);
        Ok(rows)
    }

    // Search supplies a genre-relevant seed; YouTube's automix supplies the station.
    async fn mix_lane(api: &YtMusic, query: &str, rank: usize) -> anyhow::Result<Vec<Entry>> {
        let mut seeds = Vec::new();
        if let Ok(playlists) = api.search_playlists(query).await {
            for playlist in playlists.into_iter().skip(rank).take(2) {
                if let Ok(page) = api.playlist_page(&playlist.id).await {
                    seeds.extend(page.tracks.into_iter().filter(mix_song));
                    if !seeds.is_empty() {
                        break;
                    }
                }
            }
        }
        if seeds.is_empty() {
            seeds = api
                .search_songs(query)
                .await?
                .into_iter()
                .filter(mix_song)
                .collect();
        }
        if seeds.is_empty() {
            return Ok(Vec::new());
        }
        let seed = &seeds[rank % seeds.len()];
        if let Some(video) = seed.video_id.as_deref() {
            if let Ok(radio) = api.track_radio(video).await {
                let rows: Vec<_> = radio
                    .into_iter()
                    .filter(mix_song)
                    .map(track)
                    .take(20)
                    .collect();
                if !rows.is_empty() {
                    return Ok(rows);
                }
            }
        }
        Ok(seeds.into_iter().map(track).take(20).collect())
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
                    if name.ends_with("StyleMixGenres") || name == "StyleMixGenres" {
                        args.first()
                            .and_then(Value::as_u64)
                            .ok_or_else(|| "Choose a valid language".to_string())
                            .and_then(|region| {
                                style_genres(region as usize, || {
                                    current.load(Ordering::Relaxed) == generation
                                })
                            })
                    } else {
                        invoke(&name, &args)
                    },
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
    // Reuse the desktop adapter so Connect uses the host account and provider IDs.
    pub(crate) fn connect_station(style: bool, key: &str) -> Result<Vec<Entry>, String> {
        if style {
            crate::style_mix::Selection::parse(key)?;
        } else {
            crate::spotify::flow::Selection::parse(key)?;
        }
        let api = super::api()?;
        super::runtime()
            .block_on(load(
                &api,
                if style { View::StyleMix } else { View::Flow },
                None,
                key.into(),
            ))
            .map(|page| page.rows)
            .map_err(|e| e.to_string())
    }

    pub(crate) fn connect_catalog(
        action: &str,
        query: &str,
        collection_id: &str,
    ) -> Result<Value, String> {
        if action == "search" && query.trim().is_empty() {
            return Ok(
                json!({"tracks":[],"albums":[],"playlists":[],"artists":[],"podcasts":[],"provider":"youtube"}),
            );
        }
        let (view, entity, search) = match action {
            "search" => (View::Home, None, query.to_owned()),
            "discover" => (View::Explore, None, String::new()),
            "collection" => {
                if !matches!(query, "album" | "playlist" | "artist" | "channel")
                    || !collection_id.starts_with(&format!("youtube:{query}:"))
                    || bare(collection_id).is_empty()
                {
                    return Err("Refresh the host catalog before opening this YouTube item".into());
                }
                (
                    View::Album,
                    Some(Entry {
                        id: collection_id.into(),
                        kind: query.into(),
                        raw: if query == "channel" {
                            json!({"browse_id":"FEmusic_moods_and_genres_category","params":bare(collection_id)})
                        } else {
                            json!({})
                        },
                        ..Default::default()
                    }),
                    String::new(),
                )
            }
            _ => return Err("Unsupported YouTube catalog request".into()),
        };
        let api = super::api()?;
        let page = super::runtime()
            .block_on(async {
                let mut page = load(&api, view, entity, search).await?;
                if action == "collection" || action == "discover" {
                    let mut cursors = std::collections::HashSet::new();
                    while !page.next.is_empty() && cursors.insert(page.next.clone()) {
                        more(&api, &mut page).await?;
                    }
                }
                Ok::<_, anyhow::Error>(page)
            })
            .map_err(|e| e.to_string())?;
        let items = |entries: &[Entry]| {
            entries
                .iter()
                .map(|e| {
                    json!({
                        "id":e.id,"kind":e.kind,"title":e.title,"artist":e.subtitle,
                        "subtitle":e.subtitle,"album":e.album,"artwork":e.artwork,
                        "duration":e.duration / 1000,"duration_ms":e.duration,
                    })
                })
                .collect::<Vec<_>>()
        };
        let mut result = json!({"provider":"youtube"});
        result["sections"] = json!(page
            .sections
            .iter()
            .map(|section| json!({"title":section.title,"items":items(&section.entries)}))
            .collect::<Vec<_>>());
        match action {
            "search" | "collection" => {
                result["tracks"] = json!(items(&page.rows));
                for (key, kind) in [
                    ("albums", "album"),
                    ("playlists", "playlist"),
                    ("artists", "artist"),
                ] {
                    let entries: Vec<_> = page
                        .sections
                        .iter()
                        .flat_map(|s| &s.entries)
                        .filter(|e| e.kind == kind)
                        .cloned()
                        .collect();
                    result[key] = json!(items(&entries));
                }
                result["podcasts"] = json!([]);
                if action == "collection" {
                    result["id"] = json!(collection_id);
                    result["kind"] = json!(query);
                }
            }
            _ => {
                result["sections"] = json!(page
                    .sections
                    .iter()
                    .filter_map(|section| {
                        let entries: Vec<_> = section.entries.iter().cloned().collect();
                        (!entries.is_empty())
                            .then(|| json!({"title":section.title,"items":items(&entries)}))
                    })
                    .collect::<Vec<_>>());
            }
        }
        Ok(result)
    }

    fn flow_cache(
        key: &str,
        fetch: impl FnOnce() -> Result<Value, String>,
    ) -> Result<Value, String> {
        use std::hash::{Hash, Hasher};
        let saved = crate::session::provider_saved("youtube")?;
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        saved.hash(&mut hasher);
        if key.starts_with("flow-mood-") {
            crate::catalog::disk_cache::refresh("youtube", hasher.finish(), key, fetch)
        } else {
            crate::catalog::disk_cache::load("youtube", hasher.finish(), key, fetch)
        }
    }

    fn flow_directory() -> Result<Value, String> {
        flow_cache("flow-directory-v1", || {
            let api = super::api()?;
            super::runtime()
                .block_on(async {
                    let value = api
                        .execute(
                            "browse",
                            Client::Music,
                            json!({"browseId":"FEmusic_moods_and_genres"}),
                        )
                        .await?;
                    let mut entries = Vec::new();
                    for (index, shelf) in parse::find_renderers(&value, "gridRenderer")
                        .into_iter()
                        .enumerate()
                    {
                        for button in parse::find_renderers(shelf, "musicNavigationButtonRenderer")
                        {
                            let Some(title) = parse::runs_text(&button["buttonText"]) else {
                                continue;
                            };
                            let Some(params) = button
                                .pointer("/clickCommand/browseEndpoint/params")
                                .and_then(Value::as_str)
                            else {
                                continue;
                            };
                            entries.push(json!({"title":title,"params":params,
                        "kind":if index == 0 { "mood" } else { "genre" }}));
                        }
                    }
                    anyhow::ensure!(
                        !entries.is_empty(),
                        "YouTube Music returned no mood or genre directory"
                    );
                    Ok(json!(entries))
                })
                .map_err(|e: anyhow::Error| e.to_string())
        })
    }

    fn mood_playlists(mood: &str, directory: &Value) -> Result<Value, String> {
        flow_cache(&format!("flow-mood-v2:{mood}"), || {
            let api = super::api()?;
            super::runtime().block_on(async {
            if let Some(params) = directory.as_array().into_iter().flatten().find(|item| {
                item["title"].as_str().is_some_and(|title| title.eq_ignore_ascii_case(mood))
            }).and_then(|item| item["params"].as_str()) {
                let value = api.execute("browse", Client::Music,
                    json!({"browseId":"FEmusic_moods_and_genres_category","params":params})).await?;
                let mut page = Page::default();
                feed(&value, &mut page);
                let entries: Vec<_> = page.sections.iter().flat_map(|s| &s.entries)
                    .chain(page.rows.iter()).filter(|e| e.kind == "playlist")
                    .map(|e| json!({"title":e.title,"description":e.subtitle,"artwork":e.artwork,"mood":mood})).collect();
                if !entries.is_empty() { return Ok(json!(entries)); }
            }
            let entries = api.search_playlists(mood).await?;
            Ok(json!(entries.into_iter().filter(|p| p.author.as_deref().is_some_and(|author| {
                matches!(author.to_lowercase().as_str(), "youtube" | "youtube music")
            })).map(|p| json!({"title":p.title,"artwork":cover(&p.thumbnails)})).collect::<Vec<_>>()))
        }).map_err(|e: anyhow::Error| e.to_string())
        })
    }

    pub(super) fn genre_artwork(genre: &str) -> Result<String, String> {
        let value = flow_cache(&format!("flow-art-v1:{genre}"), || {
            let api = super::api()?;
            let playlists = super::runtime()
                .block_on(api.search_playlists(&format!("{genre} music")))
                .map_err(|e| e.to_string())?;
            let url = playlists
                .iter()
                .map(|p| cover(&p.thumbnails))
                .find(|url| url.starts_with("https://"))
                .ok_or("YouTube returned no genre image")?;
            Ok(json!(url))
        })?;
        value
            .as_str()
            .map(str::to_owned)
            .ok_or_else(|| "Invalid YouTube genre image".into())
    }

    pub(crate) fn invoke(name: &str, args: &[Value]) -> Result<Value, String> {
        let _request_log = crate::spotify_diagnostics::Span::new("youtube.api", name);
        if name.ends_with("StyleMixGenres") || name == "StyleMixGenres" {
            let region = args
                .first()
                .and_then(Value::as_u64)
                .ok_or("Choose a valid language")? as usize;
            return style_genres(region, || true);
        }
        match name {
            "DZFlowConfigsJSON" => {
                return Ok(
                    json!({"options": crate::spotify::flow::MOODS.iter().map(|(id, title)| json!({"id":id,"title":title})).collect::<Vec<_>>()}),
                )
            }
            "DZFlowTunerJSON" | "DZFlowConfigTunerJSON" => {
                let key = args.first().and_then(Value::as_str).unwrap_or("default");
                let selection = crate::spotify::flow::Selection::parse(key)?;
                let directory = flow_directory().unwrap_or_else(|_| json!([]));
                return crate::lastfm::tuner(key, &directory, || {
                    mood_playlists(&selection.mood, &directory).unwrap_or_else(|_| json!([]))
                });
            }
            "DZSaveFlowTunerJSON" | "DZFlowConfigSaveJSON" => {
                let key = args.first().and_then(Value::as_str).unwrap_or("default");
                if !key.starts_with(crate::spotify::flow::PREFIX) {
                    let tuner = invoke("DZFlowTunerJSON", &[json!(key)])?;
                    return crate::spotify::flow::save_provider_tuner(
                        key,
                        args.get(1).and_then(Value::as_str).unwrap_or("[]"),
                        &tuner,
                    );
                }
                return crate::spotify::flow::save(
                    args.first().and_then(Value::as_str).unwrap_or("default"),
                    args.get(1).and_then(Value::as_str).unwrap_or("[]"),
                );
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
        fn style_mix_requires_both_constraints_and_valid_language() {
            let selection = crate::style_mix::Selection::new(0, 1).unwrap();
            assert!(selection.matches("Turkish Rock Mix"));
            assert!(!selection.matches("Japanese Rock Mix"));
            assert!(!selection.matches("Turkish Pop Mix"));
            assert!(super::invoke("StyleMixGenres", &[]).is_err());
            assert!(super::invoke("StyleMixGenres", &[serde_json::json!(-1)]).is_err());
            assert!(super::invoke("StyleMixGenres", &[serde_json::json!(99999)]).is_err());
        }

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
            let tuner = crate::spotify::flow::provider_tuner(
                "focus",
                &json!([]),
                &json!([{"title":"Ambient Focus","artwork":"https://example.test/ambient.jpg"}]),
            )
            .unwrap();
            assert_eq!(tuner["customizable"], true);
            assert!(tuner["genres"]
                .as_array()
                .unwrap()
                .iter()
                .any(|g| g["id"] == "Ambient" && g["isEnabled"] == true));
            let saved = invoke(
                "DZSaveFlowTunerJSON",
                &[
                    json!(crate::spotify::flow::Selection {
                        mood: "focus".into(),
                        genres: vec!["Ambient".into()]
                    }
                    .key()),
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
}
mod playback {
    //! Native audio playback using the ytmusic resolver and ranged stream loader.
    use crate::audio::{Command, Snapshot};
    use crate::catalog::Entry;
    use rodio::{OutputStream, Sink};
    use std::{io::Cursor, sync::mpsc, time::Duration};
    type Loaded = Result<(ytmusic::AudioFormat, Vec<u8>, Option<Entry>), String>;

    fn matching_song(video: &Entry, candidates: Vec<ytmusic::Track>) -> Option<Entry> {
        fn normalized(value: &str) -> String {
            value
                .chars()
                .filter(|c| c.is_alphanumeric())
                .flat_map(char::to_lowercase)
                .collect()
        }
        let title = video
            .title
            .split(" (Official")
            .next()
            .unwrap_or(&video.title);
        let title = normalized(title);
        let artist = normalized(video.subtitle.split(", ").next().unwrap_or(&video.subtitle));
        if title.is_empty() || artist.is_empty() {
            return None;
        }
        candidates
            .into_iter()
            .filter(|t| {
                t.available && !t.is_video() && t.video_id.as_ref().is_some_and(|id| !id.is_empty())
            })
            .find(|t| {
                normalized(&t.title) == title
                    && t.artists.iter().any(|a| normalized(&a.name) == artist)
            })
            .map(super::catalog::track)
    }
    // Sample the original collection before shuffle or manual queue edits.
    fn collection_sources(queue: &[Entry]) -> Vec<Entry> {
        let mut seen=std::collections::HashSet::new();
        let tracks:Vec<_>=queue.iter().filter(|e| e.kind == "track" && !e.id.is_empty()
            && e.raw["_youtube_radio"] != true && !crate::audio::is_manual_queue_entry(e)
            && seen.insert(e.id.clone())).collect();
        let count=tracks.len().min(24);
        (0..count).map(|i| tracks[if count == 1 {0} else {i*(tracks.len()-1)/(count-1)}].clone()).collect()
    }

    fn collection_batch(sources: &[Entry], round: usize) -> Vec<Entry> {
        let count=sources.len().min(4);
        let mut selected=std::collections::HashSet::new();
        let mut artists=std::collections::HashSet::new();
        let mut batch=Vec::new();
        for i in 0..count {
            let target=(round%sources.len()+i*sources.len()/count)%sources.len();
            let artist=|e: &Entry| if e.artist_id.is_empty() {e.subtitle.clone()} else {e.artist_id.clone()};
            let candidates=|| (0..sources.len()).map(|offset|(target+offset)%sources.len()).filter(|index|!selected.contains(index));
            let index=candidates().find(|index| {let key=artist(&sources[*index]);key.is_empty() || !artists.contains(&key)})
                .or_else(||candidates().next()).unwrap();
            selected.insert(index);
            artists.insert(artist(&sources[index]));
            batch.push(sources[index].clone());
        }
        batch
    }

    fn blend_radio(batches: Vec<Vec<Entry>>, mut known: std::collections::HashSet<String>) -> Vec<Entry> {
        let mut batches:Vec<_>=batches.into_iter().map(Vec::into_iter).collect();
        let mut rows=Vec::new();
        loop {
            let mut progressed=false;
            for batch in &mut batches {
                if let Some(mut row)=batch.next() {
                    progressed=true;
                    if row.kind == "track" && !row.id.is_empty() && known.insert(row.id.clone()) {
                        if !row.raw.is_object() {row.raw=serde_json::json!({});}
                        row.raw["_youtube_radio"]=serde_json::json!(true);
                        rows.push(row);
                    }
                }
            }
            if !progressed {break;}
        }
        rows
    }

    fn advance_index(index: usize, len: usize, repeat: u8, backwards: bool, natural_end: bool) -> Option<usize> {
        if len == 0 { return None; }
        let next = if backwards { index.saturating_sub(1) }
            else if natural_end && repeat == 2 { index }
            else { index.saturating_add(1) };
        if next < len { Some(next) } else if repeat == 1 { Some(0) } else { None }
    }
    pub(crate) struct Player {
        shuffle: crate::audio::PreparedShuffle,
        eq: std::sync::Arc<std::sync::Mutex<super::routing::Eq>>,
        output: Option<OutputStream>,
        sink: Option<Sink>,
        audio_bytes: Option<std::sync::Arc<[u8]>>,
        endpoint: Option<String>,
        endpoint_checked: std::time::Instant,
        pending: Option<mpsc::Receiver<Loaded>>,
        load_cancel: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
        volume: f32,
        paused: bool,
        auth: Option<(
            std::sync::Arc<std::sync::atomic::AtomicBool>,
            mpsc::Receiver<Result<serde_json::Value, String>>,
        )>,
        radio: Option<mpsc::Receiver<Result<Vec<Entry>, String>>>,
        last_seed: String,
        collection_sources: Vec<Entry>,
        collection_round: usize,
        collection_continuations: std::sync::Arc<std::sync::Mutex<std::collections::HashMap<String,String>>>,
    }
    impl Default for Player {
        fn default() -> Self {
            Self {
                shuffle: Default::default(),
                eq: Default::default(),
                output: None,
                sink: None,
                audio_bytes: None,
                endpoint: None,
                endpoint_checked: std::time::Instant::now() - Duration::from_secs(2),
                pending: None,
                load_cancel: None,
                volume: 1.0,
                paused: false,
                auth: None,
                radio: None,
                last_seed: String::new(),
                collection_sources: Vec::new(),
                collection_round: 0,
                collection_continuations: Default::default(),
            }
        }
    }
    impl Player {
        pub(crate) fn stop(&mut self) {
            super::routing::reset();
            self.pending = None;
            if let Some(cancel) = self.load_cancel.take() {
                cancel.store(true, std::sync::atomic::Ordering::Relaxed);
            }
            if let Some(s) = self.sink.take() {
                s.stop();
            }
            self.output = None;
            self.audio_bytes = None;
        }
        pub(crate) fn suspend(&mut self) {
            self.stop();
            self.radio = None;
            self.last_seed.clear();
            self.collection_sources.clear();
            self.collection_round=0;
            self.collection_continuations=Default::default();
            if let Some((cancel, _)) = self.auth.take() {
                cancel.store(true, std::sync::atomic::Ordering::Relaxed);
            }
        }
        fn auth(&mut self, value: Option<String>, snapshot: &mut Snapshot) {
            if self.auth.is_some() {
                if value.is_none() {
                    return;
                }
                if let Some((cancel, _)) = self.auth.take() {
                    cancel.store(true, std::sync::atomic::Ordering::Relaxed);
                }
            }
            let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
            let flag = cancel.clone();
            let (tx, rx) = mpsc::sync_channel(1);
            self.auth = Some((cancel, rx));
            snapshot.message = "Connecting to YouTube Music...".into();
            std::thread::spawn(move || {
                let result = match value {
                    Some(s) if s.is_empty() => super::login(&flag),
                    Some(s) => super::login_cookies_with_cancel(&s, &flag),
                    None => super::account(),
                };
                let _ = tx.send(result);
            });
        }
        fn clear_repeat_for_selection(entry: &Entry, snapshot: &mut Snapshot) {
            if snapshot.repeat == 2 && (snapshot.now.id != entry.id || snapshot.now.kind != entry.kind) {
                snapshot.repeat = 0;
            }
        }
        fn select(&mut self, entry: Entry, snapshot: &mut Snapshot) {
            Self::clear_repeat_for_selection(&entry, snapshot);
            self.start(entry, snapshot);
        }
        fn start(&mut self, entry: Entry, snapshot: &mut Snapshot) {
            self.stop();
            self.paused = false;
            snapshot.now = entry.clone();
            snapshot.title = entry.title.clone();
            snapshot.duration = entry.duration;
            snapshot.position = 0;
            snapshot.state = "Loading".into();
            snapshot.message.clear();
            let (tx, rx) = mpsc::sync_channel(1);
            self.pending = Some(rx);
            let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
            self.load_cancel = Some(cancel.clone());
            std::thread::spawn(move || {
                let result = super::api().and_then(|api| {
                super::runtime()
                    .block_on(async {
                        let load = async {
                            let song = if entry.raw.pointer("/youtube/kind").and_then(|v| v.as_str()) == Some("Video") {
                                let title = entry.title.split(" (Official").next().unwrap_or(&entry.title);
                                let query = format!("{} {}", title, entry.subtitle);
                                match tokio::time::timeout(Duration::from_secs(5), api.search_songs(&query)).await {
                                    Ok(Ok(candidates)) => matching_song(&entry, candidates),
                                    _ => None,
                                }
                            } else { None };
                            let mut song = song;
                            let format = if let Some(selected) = song.as_ref() {
                                match api.best_audio(super::catalog::bare(&selected.id)).await {
                                    Ok(format) => format,
                                    Err(_) => { song = None; api.best_audio(super::catalog::bare(&entry.id)).await? }
                                }
                            } else { api.best_audio(super::catalog::bare(&entry.id)).await? };
                            anyhow::ensure!(format.content_length.is_some_and(|n| n <= 128 * 1024 * 1024),
                                "YouTube track has an unknown size or exceeds the 128 MiB playback buffer");
                            if cancel.load(std::sync::atomic::Ordering::Relaxed) {
                                anyhow::bail!("YouTube playback cancelled");
                            }
                            let bytes = api.download(&format).await?;
                            anyhow::ensure!(bytes.len() <= 128 * 1024 * 1024,
                                "YouTube track exceeds the 128 MiB playback buffer");
                            Ok::<_, anyhow::Error>((format, bytes, song))
                        };
                        tokio::time::timeout(Duration::from_secs(60), async {
                            tokio::select! {
                                result = load => result,
                                _ = async {
                                    while !cancel.load(std::sync::atomic::Ordering::Relaxed) {
                                        tokio::time::sleep(Duration::from_millis(100)).await;
                                    }
                                } => Err(anyhow::anyhow!("YouTube playback cancelled")),
                            }
                        }).await
                    })
                    .map_err(|_| "YouTube audio load timed out".to_string())?
                    .map_err(|e| e.to_string())
            });
                let _ = tx.send(result);
            });
        }
        fn advance(&mut self, snapshot: &mut Snapshot, backwards: bool, natural_end: bool) {
            let Some(index) = advance_index(snapshot.queue_index, snapshot.queue.len(), snapshot.repeat, backwards, natural_end) else {
                self.stop();
                snapshot.state = "Stopped".into();
                return;
            };
            snapshot.queue_index = index;
            let entry = snapshot.queue[index].clone();
            if !natural_end { Self::clear_repeat_for_selection(&entry, snapshot); }
            self.start(entry, snapshot);
        }
        pub(crate) fn handle(&mut self, command: &Command, snapshot: &mut Snapshot) {
            match command {
                Command::Login(value) => self.auth(Some(value.clone()), snapshot),
                Command::LoginSaved => self.auth(None, snapshot),
                Command::Logout => {
                    self.suspend();
                    if let Err(e) = super::logout() {
                        snapshot.message = e;
                    }
                    snapshot.connected = false;
                    snapshot.account = serde_json::Value::Null;
                    snapshot.now = Default::default();
                    snapshot.queue = Default::default();
                    snapshot.queue_index = 0;
                    snapshot.title.clear();
                    snapshot.position = 0;
                    snapshot.duration = 0;
                    snapshot.radio_active = false;
                    snapshot.state = "Stopped".into();
                }
                Command::PlayEntry(e, queue) => {
                    self.radio = None;
                    self.last_seed.clear();
                    snapshot.radio_active = e.raw["_youtube_radio"] == true;
                    let mut q = queue.clone();
                    if !q.iter().any(|t| t.id == e.id) {
                        q.insert(0, (**e).clone());
                    }
                    self.collection_sources=collection_sources(&q);
                    if self.collection_sources.is_empty() {self.collection_sources.push((**e).clone());}
                    self.collection_round=0;
                    self.collection_continuations=Default::default();
                    crate::audio::mark_queue_fallback(&mut q);
                    snapshot.queue_index = q.iter().position(|t| t.id == e.id).unwrap_or(0);
                    if snapshot.shuffle {
                        self.shuffle.prepare(&mut q, snapshot.queue_index);
                    }
                    snapshot.queue = q.into();
                    self.select((**e).clone(), snapshot);
                }
                Command::Play(id) => {
                    if let Some(i) = snapshot.queue.iter().position(|t| t.id == *id) {
                        snapshot.queue_index = i;
                        self.select(snapshot.queue[i].clone(), snapshot);
                    }
                }
                Command::PlayQueueIndex(i) if *i < snapshot.queue.len() => {
                    snapshot.queue_index = *i;
                    self.select(snapshot.queue[*i].clone(), snapshot);
                }
                Command::Next => self.advance(snapshot, false, false),
                Command::Previous if snapshot.position > 3_000 => self.seek(0, snapshot),
                Command::Previous => self.advance(snapshot, true, false),
                Command::Toggle => {
                    self.paused = !self.paused;
                    if let Some(s) = &self.sink {
                        if self.paused {
                            s.pause()
                        } else {
                            s.play()
                        }
                    }
                    snapshot.state = if self.paused { "Paused" } else { "Playing" }.into();
                }
                Command::SetPlaying(p) => {
                    self.paused = !*p;
                    if let Some(s) = &self.sink {
                        if *p {
                            s.play()
                        } else {
                            s.pause()
                        }
                    }
                    snapshot.state = if *p { "Playing" } else { "Paused" }.into();
                }
                Command::Stop => {
                    self.stop();
                    snapshot.state = "Stopped".into();
                    snapshot.position = 0;
                }
                Command::Seek(p) => self.seek(
                    snapshot.duration.saturating_mul((*p).min(100) as u64) / 100,
                    snapshot,
                ),
                Command::SeekMs(p) => self.seek(*p, snapshot),
                Command::Volume(v) => {
                    self.volume = (*v).min(100) as f32 / 100.;
                    if let Some(s) = &self.sink {
                        s.set_volume(self.volume);
                    }
                }
                Command::Repeat(v) => snapshot.repeat = *v,
                Command::Shuffle(v) if snapshot.shuffle != *v => {
                    let mut queue = snapshot.queue.as_ref().clone();
                    if *v {
                        self.shuffle.prepare(&mut queue, snapshot.queue_index);
                    } else {
                        snapshot.queue_index =
                            self.shuffle.restore(&mut queue, snapshot.queue_index);
                    }
                    snapshot.queue = queue.into();
                    snapshot.shuffle = *v;
                }
                Command::Autoplay(v) => snapshot.autoplay = *v,
                Command::QueueClear => {
                    if snapshot.queue.is_empty() {self.radio=None;}
                    snapshot.queue=crate::audio::clear_manual_upcoming(&snapshot.queue,snapshot.queue_index).into();
                    snapshot.message="Manually queued tracks cleared; source playback continues".into();
                }
                Command::QueueInsertNext(t) => {
                    let index=(snapshot.queue_index+1).min(snapshot.queue.len());
                    ArcQueue::insert(snapshot,index,crate::audio::explicit_queue_entry((**t).clone()));
                }
                Command::QueueAddLast(t) => {
                    let index=crate::audio::queue_add_position(&snapshot.queue,snapshot.queue_index);
                    ArcQueue::insert(snapshot,index,crate::audio::explicit_queue_entry((**t).clone()));
                }
                Command::QueueMove { from, to, expected } => {
                    match crate::audio::reordered_queue(
                        &snapshot.queue,
                        snapshot.queue_index,
                        *from,
                        *to,
                        expected,
                    ) {
                        Ok(queue) => {
                            snapshot.queue = std::sync::Arc::new(queue);
                            snapshot.message = "Queue reordered".into();
                        }
                        Err(error) => snapshot.message = error,
                    }
                }
                Command::Api(name, args) => {
                    if name == "DZSetEQJSON" {
                        let result = crate::engine::normalize_eq(args);
                        match result {
                            Ok(value) => {
                                if let Ok(mut eq) = self.eq.lock() {
                                    eq.update(&value);
                                }
                            }
                            Err(error) => snapshot.message = error.into(),
                        }
                        return;
                    }
                    if let Err(e) = super::invoke(name, args) {
                        snapshot.message = e;
                    }
                }
                _ => {}
            }
        }
        fn seek(&mut self, ms: u64, snapshot: &mut Snapshot) {
            if let Some(bytes) = self.audio_bytes.clone().filter(|_| self.sink.is_some()) {
                let ms = if snapshot.duration > 0 {
                    ms.min(snapshot.duration)
                } else {
                    ms
                };
                match self.reopen_output(bytes, Duration::from_millis(ms)) {
                    Ok(()) => snapshot.position = ms,
                    Err(e) => snapshot.message = format!("YouTube seek: {e}"),
                }
            }
        }
        fn reopen_output(
            &mut self,
            bytes: std::sync::Arc<[u8]>,
            position: Duration,
        ) -> Result<(), String> {
            let output =
                rodio::OutputStreamBuilder::open_default_stream().map_err(|e| e.to_string())?;
            let sink = Sink::connect_new(output.mixer());
            let source = rodio::Decoder::try_from(Cursor::new(bytes)).map_err(|e| e.to_string())?;
            sink.set_volume(self.volume);
            sink.pause();
            let source = rodio::source::UniformSourceIterator::new(source, 2, 44_100);
            sink.append(super::routing::Routed::new(source, self.eq.clone()));
            if !position.is_zero() {
                sink.try_seek(position).map_err(|e| e.to_string())?;
            }
            if !self.paused {
                sink.play();
            }
            if let Some(old) = self.sink.take() {
                old.stop();
            }
            self.output = Some(output);
            self.sink = Some(sink);
            Ok(())
        }
        pub(crate) fn poll(&mut self, snapshot: &mut Snapshot) {
            if self.endpoint_checked.elapsed() >= Duration::from_secs(1) {
                self.endpoint_checked = std::time::Instant::now();
                if let Ok(endpoint) = crate::audio::windows_default_output_id() {
                    if self.endpoint.as_ref() != Some(&endpoint) {
                        if let (Some(bytes), Some(sink)) = (&self.audio_bytes, &self.sink) {
                            let position = sink.get_pos();
                            match self.reopen_output(bytes.clone(), position) {
                                Ok(()) => self.endpoint = Some(endpoint),
                                Err(e) => snapshot.message = format!("YouTube output: {e}"),
                            }
                        } else {
                            self.output = None;
                            self.endpoint = Some(endpoint);
                        }
                    }
                }
            }
            if let Some(result) = self.auth.as_ref().and_then(|(_, rx)| rx.try_recv().ok()) {
                self.auth = None;
                match result {
                    Ok(account) => {
                        snapshot.account = account;
                        snapshot.connected = true;
                        snapshot.message.clear();
                    }
                    Err(e) => snapshot.message = e,
                }
            }
            if let Some(result) = self.radio.as_ref().and_then(|rx| rx.try_recv().ok()) {
                self.radio = None;
                match result {
                    Ok(rows) => {
                        let known = snapshot
                            .queue
                            .iter()
                            .map(|t| t.id.clone())
                            .collect::<std::collections::HashSet<_>>();
                        std::sync::Arc::make_mut(&mut snapshot.queue)
                            .extend(rows.into_iter().filter(|t| !known.contains(&t.id)));
                    }
                    Err(e) => snapshot.message = format!("YouTube radio: {e}"),
                }
            }
            if (snapshot.radio_active || snapshot.autoplay)
                && snapshot.queue_index.saturating_add(3) >= snapshot.queue.len()
                && !snapshot.queue.is_empty()
                && self.radio.is_none()
                && (self.sink.is_some() || self.pending.is_some())
            {
                let tail = snapshot.queue.last().unwrap();
                let flow = tail.raw["_youtube_flow"].as_str().map(str::to_owned);
                let offset = tail.raw["_youtube_flow_offset"].as_u64().unwrap_or(0) as usize;
                let seed = tail.id.clone();
                let known = snapshot.queue.iter().map(|t| t.id.clone()).collect();
                if seed != self.last_seed {
                    self.last_seed = seed.clone();
                    if self.collection_sources.is_empty() {
                        self.collection_sources=collection_sources(&snapshot.queue);
                    }
                    let sources=collection_batch(&self.collection_sources,self.collection_round);
                    self.collection_round=self.collection_round.wrapping_add(1);
                    let continuations=self.collection_continuations.clone();
                    let (tx, rx) = mpsc::sync_channel(1);
                    self.radio = Some(rx);
                    std::thread::spawn(move || {
                        let result = super::api().and_then(|api| {
                            super::runtime()
                                .block_on(async {
                                    if let Some(key) = flow {
                                        super::catalog::continue_flow(&api, &key, offset, known)
                                            .await
                                    } else {
                                        let mut batches=Vec::new();
                                        let mut failure=None;
                                        for source in sources {
                                            let token=continuations.lock().unwrap().get(&source.id).cloned();
                                            let request=super::catalog::station(&api,super::catalog::bare(&source.id),token.as_deref());
                                            let result=tokio::time::timeout(Duration::from_secs(10),request).await;
                                            match result {
                                                Ok(Ok((mut rows,next))) => {
                                                    if let Some(name)=source.raw.get("queue_context_name") {
                                                        for row in &mut rows {row.raw["queue_context_name"]=name.clone();}
                                                    }
                                                    let token=serde_json::from_str::<serde_json::Value>(&next).ok()
                                                        .and_then(|v|v["radio"].as_str().map(str::to_owned));
                                                    let mut tokens=continuations.lock().unwrap();
                                                    if let Some(token)=token {tokens.insert(source.id.clone(),token);} else {tokens.remove(&source.id);}
                                                    batches.push(rows);
                                                }
                                                Ok(Err(error)) => failure=Some(error),
                                                Err(_) => failure=Some(anyhow::anyhow!("YouTube collection radio timed out")),
                                            }
                                        }
                                        if batches.is_empty() {
                                            Err(failure.unwrap_or_else(||anyhow::anyhow!("No collection radio sources")))
                                        } else {
                                            Ok((blend_radio(batches,known),String::new()))
                                        }
                                    }
                                })
                                .map(|(rows, _)| rows)
                                .map_err(|e| e.to_string())
                        });
                        let _ = tx.send(result);
                    });
                }
            }
            if let Some(result) = self.pending.as_ref().and_then(|rx| rx.try_recv().ok()) {
                self.pending = None;
                let result = result.and_then(|(format, bytes, song)| {
                    let bytes: std::sync::Arc<[u8]> = bytes.into();
                    self.reopen_output(bytes.clone(), Duration::ZERO)?;
                    self.audio_bytes = Some(bytes);
                    if let Some(song) = song {
                        snapshot.title = song.title.clone();
                        snapshot.now = song;
                    }
                    snapshot.duration = format
                        .duration
                        .map(|d| d.as_millis() as u64)
                        .unwrap_or(snapshot.duration);
                    snapshot.quality = format!("{} kbps {}", format.bitrate / 1000, format.codec);
                    snapshot.state = if self.paused { "Paused" } else { "Playing" }.into();
                    snapshot.playback_connected = true;
                    super::remember(&snapshot.now);
                    Ok(())
                });
                if let Err(e) = result {
                    snapshot.message = format!("YouTube playback: {e}");
                    snapshot.state = "Stopped".into();
                    snapshot.playback_connected = false;
                }
            }
            if let Some(s) = &self.sink {
                snapshot.position = s.get_pos().as_millis() as u64;
                if s.empty()
                    && !self.paused
                    && (self.radio.is_none() || snapshot.queue_index + 1 < snapshot.queue.len())
                {
                    self.advance(snapshot, false, true);
                }
            }
        }
    }
    struct ArcQueue;
    impl ArcQueue {
        fn insert(s: &mut Snapshot, index: usize, t: Entry) {
            std::sync::Arc::make_mut(&mut s.queue).insert(index, t);
        }
    }

    impl Drop for Player {
        fn drop(&mut self) {
            self.suspend();
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        fn radio_track(id: &str) -> Entry {
            Entry {id:id.into(),kind:"track".into(),..Default::default()}
        }

        #[test]
        fn collection_radio_samples_whole_original_collection_and_excludes_queue_edits() {
            let mut tracks:Vec<_>=(0..100).map(|i|radio_track(&format!("song{i}"))).collect();
            tracks.push(crate::audio::explicit_queue_entry(radio_track("manual")));
            let mut radio=radio_track("generated");radio.raw=serde_json::json!({"_youtube_radio":true});tracks.push(radio);
            tracks.push(tracks[0].clone());
            let sources=collection_sources(&tracks);
            assert_eq!(sources.len(),24);
            assert_eq!(sources.first().unwrap().id,"song0");
            assert_eq!(sources.last().unwrap().id,"song99");
            assert!(!sources.iter().any(|e|matches!(e.id.as_str(),"manual"|"generated")));
            let first=collection_batch(&sources,0);
            assert_eq!(first.iter().map(|e|e.id.as_str()).collect::<Vec<_>>(),vec!["song0","song25","song51","song77"]);
            let mut covered=std::collections::HashSet::new();
            for round in 0..sources.len() {for e in collection_batch(&sources,round) {covered.insert(e.id);}}
            assert_eq!(covered.len(),24);
            assert!(collection_batch(&[],0).is_empty());
            assert_eq!(collection_batch(&[radio_track("only")],99)[0].id,"only");
        }

        #[test]
        fn collection_radio_avoids_sampling_only_one_artist_in_alternating_playlists() {
            let sources:Vec<_>=(0..24).map(|i| {let mut row=radio_track(&format!("song{i}"));row.artist_id=format!("artist{}",i%2);row}).collect();
            let batch=collection_batch(&sources,0);
            assert_eq!(batch.iter().map(|e|e.artist_id.as_str()).collect::<std::collections::HashSet<_>>().len(),2);
            assert_eq!(batch.iter().map(|e|e.id.as_str()).collect::<std::collections::HashSet<_>>().len(),4);
        }

        #[test]
        fn manual_next_and_natural_end_follow_distinct_repeat_one_rules() {
            assert_eq!(advance_index(0, 2, 2, false, false), Some(1));
            assert_eq!(advance_index(0, 2, 2, false, true), Some(0));
            assert_eq!(advance_index(1, 2, 2, false, false), None);
            assert_eq!(advance_index(1, 2, 1, false, false), Some(0));
        }

        #[test]
        fn explicit_selection_clears_repeat_only_for_a_different_recording() {
            let mut snapshot = Snapshot::default();
            snapshot.now = radio_track("first");
            snapshot.repeat = 2;
            Player::clear_repeat_for_selection(&radio_track("first"), &mut snapshot);
            assert_eq!(snapshot.repeat, 2);
            Player::clear_repeat_for_selection(&radio_track("second"), &mut snapshot);
            assert_eq!(snapshot.repeat, 0);
            snapshot.repeat = 1;
            Player::clear_repeat_for_selection(&radio_track("third"), &mut snapshot);
            assert_eq!(snapshot.repeat, 1);
        }

        #[test]
        fn stop_cancels_pending_load_and_releases_buffer() {
            let mut player = Player::default();
            let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
            let (_tx, rx) = std::sync::mpsc::sync_channel(1);
            player.pending = Some(rx);
            player.load_cancel = Some(cancel.clone());
            let bytes: std::sync::Arc<[u8]> = vec![1, 2, 3].into();
            let weak = std::sync::Arc::downgrade(&bytes);
            player.audio_bytes = Some(bytes);
            player.stop();
            assert!(cancel.load(std::sync::atomic::Ordering::Relaxed));
            assert!(player.pending.is_none() && player.output.is_none() && player.sink.is_none());
            assert!(weak.upgrade().is_none());
        }

        #[test]
        fn previous_after_three_seconds_keeps_current_queue_item() {
            let mut player = Player::default();
            let mut snapshot = Snapshot::default();
            snapshot.queue = vec![radio_track("first"), radio_track("second")].into();
            snapshot.queue_index = 1;
            snapshot.position = 3_001;
            player.handle(&Command::Previous, &mut snapshot);
            assert_eq!(snapshot.queue_index, 1);
            assert!(player.pending.is_none());
        }

        #[test]
        fn collection_radio_blends_styles_and_filters_known_and_duplicate_tracks() {
            let rows=blend_radio(vec![
                vec![radio_track("known"),radio_track("a1"),radio_track("shared"),radio_track("a2")],
                vec![radio_track("b1"),radio_track("shared"),radio_track("b2")],
            ],std::collections::HashSet::from(["known".into()]));
            assert_eq!(rows.iter().map(|e|e.id.as_str()).collect::<Vec<_>>(),vec!["b1","a1","shared","b2","a2"]);
            assert!(rows.iter().all(|e|e.raw["_youtube_radio"]==true));
            assert!(collection_sources(&rows).is_empty());
        }

        #[test]
        fn collection_radio_reset_discards_sources_and_continuations() {
            let mut player=Player::default();
            player.collection_sources=vec![radio_track("original")];
            player.collection_round=7;
            player.collection_continuations.lock().unwrap().insert("original".into(),"token".into());
            let old=player.collection_continuations.clone();
            player.suspend();
            assert!(player.collection_sources.is_empty());
            assert_eq!(player.collection_round,0);
            assert!(player.collection_continuations.lock().unwrap().is_empty());
            old.lock().unwrap().insert("stale".into(),"late-result".into());
            assert!(player.collection_continuations.lock().unwrap().is_empty());
        }

        #[test]
        fn prepared_shuffle_toggle_is_stable_restores_order_and_stops_at_end() {
            let mut player = Player::default();
            let mut snapshot = Snapshot::default();
            snapshot.queue = ["playing", "a1", "a2", "b1", "b2"]
                .into_iter()
                .map(|id| Entry {
                    id: id.into(),
                    artist_id: id[..1].into(),
                    ..Default::default()
                })
                .collect::<Vec<_>>()
                .into();
            player.handle(&Command::Shuffle(true), &mut snapshot);
            assert!(snapshot.shuffle);
            assert_eq!(snapshot.queue[0].id, "playing");
            let prepared = snapshot.queue.clone();
            player.handle(&Command::Shuffle(true), &mut snapshot);
            assert_eq!(snapshot.queue, prepared);
            snapshot.queue_index = snapshot.queue.len() - 1;
            let selected = snapshot.queue[snapshot.queue_index].id.clone();
            player.handle(&Command::Next, &mut snapshot);
            assert_eq!(snapshot.state, "Stopped");
            assert!(player.pending.is_none());
            player.handle(&Command::Shuffle(false), &mut snapshot);
            assert!(!snapshot.shuffle);
            assert_eq!(snapshot.queue[snapshot.queue_index].id, selected);
            assert_eq!(
                snapshot
                    .queue
                    .iter()
                    .map(|r| r.id.as_str())
                    .collect::<Vec<_>>(),
                ["playing", "a1", "a2", "b1", "b2"]
            );
        }

        #[test]
        fn video_matching_requires_same_song_and_artist() {
            let video = Entry {
                title: "KGB (Official Music Video)".into(),
                subtitle: "ADÉLA".into(),
                ..Default::default()
            };
            let candidate = |title: &str, artist: &str, kind| ytmusic::Track {
                video_id: Some("id".into()),
                title: title.into(),
                available: true,
                kind,
                artists: vec![ytmusic::ArtistRef {
                    name: artist.into(),
                    id: None,
                }],
                ..Default::default()
            };
            assert!(matching_song(
                &video,
                vec![
                    candidate("KGB", "Other", ytmusic::TrackKind::Song),
                    candidate("KGB", "ADÉLA", ytmusic::TrackKind::Video)
                ]
            )
            .is_none());
            assert!(matching_song(
                &video,
                vec![candidate("KGB", "ADÉLA", ytmusic::TrackKind::Song)]
            )
            .is_some());
        }
        #[test]
        fn reorder_preserves_current_duplicate_and_rejects_stale_queue() {
            let mut player = Player::default();
            let mut snapshot = Snapshot::default();
            snapshot.queue = ["same", "same", "a", "b"]
                .into_iter()
                .map(|id| Entry {
                    id: id.into(),
                    ..Default::default()
                })
                .collect::<Vec<_>>()
                .into();
            snapshot.queue_index = 1;
            let expected = snapshot
                .queue
                .iter()
                .map(|entry| entry.id.clone())
                .collect();
            player.handle(
                &Command::QueueMove {
                    from: 3,
                    to: 2,
                    expected,
                },
                &mut snapshot,
            );
            assert_eq!(
                snapshot
                    .queue
                    .iter()
                    .map(|e| e.id.as_str())
                    .collect::<Vec<_>>(),
                ["same", "same", "b", "a"]
            );
            assert_eq!(snapshot.queue_index, 1);
            player.handle(
                &Command::QueueMove {
                    from: 2,
                    to: 3,
                    expected: vec!["stale".into()],
                },
                &mut snapshot,
            );
            assert!(snapshot.message.contains("changed"));
            assert_eq!(snapshot.queue[2].id, "b");
        }

        #[test]
        #[ignore = "requires a signed-in YouTube Music session and network access"]
        fn live_audio_load() {
            let api = super::super::api().expect("Sign in to YouTube Music first");
            super::super::runtime().block_on(async {
                let tracks = api
                    .search_songs("Daft Punk Get Lucky")
                    .await
                    .expect("song search failed");
                let id = tracks
                    .iter()
                    .find_map(|track| track.video_id.as_deref())
                    .expect("no playable song");
                let result = tokio::time::timeout(Duration::from_secs(60), api.load_audio(id))
                    .await
                    .expect("YouTube audio load timed out")
                    .expect("YouTube audio load failed");
                let mut decoded =
                    rodio::Decoder::try_from(Cursor::new(result.1)).expect("audio decode failed");
                assert!(decoded.next().is_some());
            });
        }
        #[test]
        #[ignore = "requires a signed-in YouTube Music session, network and an audio device"]
        fn live_player_loads_paused() {
            let api = super::super::api().expect("Sign in to YouTube Music first");
            let track = super::super::runtime()
                .block_on(api.search_songs("Daft Punk Get Lucky"))
                .expect("song search failed")
                .into_iter()
                .find(|track| track.video_id.is_some())
                .expect("no playable song");
            let mut player = Player::default();
            let mut snapshot = Snapshot::default();
            player.start(super::super::catalog::track(track), &mut snapshot);
            player.paused = true;
            let deadline = std::time::Instant::now() + Duration::from_secs(65);
            while player.pending.is_some() && std::time::Instant::now() < deadline {
                player.poll(&mut snapshot);
                std::thread::sleep(Duration::from_millis(50));
            }
            assert_eq!(snapshot.state, "Paused", "{}", snapshot.message);
            assert!(player.sink.is_some());
        }
        #[test]
        fn leaving_provider_cancels_login_and_discards_pending_audio() {
            let mut player = Player::default();
            let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
            let (_tx, rx) = mpsc::channel();
            player.auth = Some((cancel.clone(), rx));
            let (_audio, rx) = mpsc::channel();
            player.pending = Some(rx);
            player.suspend();
            assert!(cancel.load(std::sync::atomic::Ordering::Relaxed));
            assert!(player.auth.is_none());
            assert!(player.pending.is_none());
        }
        #[test]
        fn manual_queue_is_fifo_before_context_and_clear_keeps_playback() {
            let mut player=Player::default();
            let (tx,rx)=mpsc::channel();player.radio=Some(rx);
            let mut snapshot=Snapshot {state:"Playing".into(),position:1234,radio_active:true,..Default::default()};
            let entries:Vec<_>=["current","context"].into_iter().map(|id|Entry {id:id.into(),..Default::default()}).collect();
            snapshot.now=entries[0].clone();snapshot.queue=entries.into();
            for id in ["manual1","manual2"] {player.handle(&Command::QueueAddLast(Box::new(Entry {id:id.into(),..Default::default()})),&mut snapshot);}
            assert_eq!(snapshot.queue.iter().map(|e|e.id.as_str()).collect::<Vec<_>>(),vec!["current","manual1","manual2","context"]);
            player.handle(&Command::QueueClear,&mut snapshot);
            assert_eq!(snapshot.queue.iter().map(|e|e.id.as_str()).collect::<Vec<_>>(),vec!["current","context"]);
            assert_eq!(snapshot.state,"Playing");assert_eq!(snapshot.position,1234);
            assert!(snapshot.radio_active);assert!(player.radio.is_some());
            assert!(tx.send(Ok(Vec::new())).is_ok());
        }
        #[test]
        fn cleared_queue_does_not_receive_old_radio_results() {
            let mut player = Player::default();
            let (tx, rx) = mpsc::channel();
            player.radio = Some(rx);
            let mut snapshot = Snapshot::default();
            player.handle(&Command::QueueClear, &mut snapshot);
            assert!(tx
                .send(Ok(vec![Entry {
                    id: "youtube:track:old".into(),
                    ..Default::default()
                }]))
                .is_err());
            player.poll(&mut snapshot);
            assert!(snapshot.queue.is_empty());
        }
    }
}
mod potoken {
    //! Browser token protocol adapted to the app's WebView2 host.
    use std::{
        collections::HashMap,
        sync::{Mutex, OnceLock},
        time::{Duration, Instant},
    };
    pub(super) struct Minter;
    #[async_trait::async_trait]
    impl ytmusic::Minter for Minter {
        async fn mint(&self, binding: &str) -> Option<String> {
            static CACHE: OnceLock<Mutex<HashMap<String, (Instant, String)>>> = OnceLock::new();
            let cache = CACHE.get_or_init(Mutex::default);
            if let Some((at, token)) = cache.lock().ok()?.get(binding) {
                if at.elapsed() < Duration::from_secs(6 * 3600) {
                    return Some(token.clone());
                }
            }
            let binding = binding.to_owned();
            let requested = binding.clone();
            let value = tokio::task::spawn_blocking(move || {
                super::potoken_webview::run(&requested, &std::sync::atomic::AtomicBool::new(false))
            })
            .await
            .ok()?
            .ok()?;
            let (returned, token) = value.split_once(':')?;
            let returned = url::form_urlencoded::parse(format!("v={returned}").as_bytes())
                .next()?
                .1
                .into_owned();
            if returned != binding || token.starts_with('!') || token.is_empty() {
                return None;
            }
            cache
                .lock()
                .ok()?
                .insert(binding, (Instant::now(), token.into()));
            Some(token.into())
        }
    }
}
mod potoken_webview {
    //! Short-lived private WebView2 login for the YouTube proof-of-origin token.
    use std::{
        cell::{Cell, RefCell},
        rc::Rc,
        sync::atomic::{AtomicBool, Ordering},
        time::{Duration, Instant},
    };
    use webview2_com::{
        AddScriptToExecuteOnDocumentCreatedCompletedHandler, CoTaskMemPWSTR,
        CreateCoreWebView2ControllerCompletedHandler,
        CreateCoreWebView2EnvironmentCompletedHandler, DownloadStartingEventHandler,
        GetCookiesCompletedHandler, Microsoft::Web::WebView2::Win32::*,
        NavigationStartingEventHandler, NewWindowRequestedEventHandler,
        PermissionRequestedEventHandler,
    };
    use windows::core::{Interface, PCWSTR, PWSTR};
    use windows_sys::Win32::{
        Foundation::*,
        Graphics::Gdi::*,
        System::{Com::*, LibraryLoader::GetModuleHandleW},
        UI::WindowsAndMessaging::*,
    };
    thread_local! { static CONTROLLER:RefCell<Option<ICoreWebView2Controller>>=const {RefCell::new(None)}; static CLOSED:Cell<bool>=const {Cell::new(false)}; }
    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(Some(0)).collect()
    }
    fn allowed_navigation(uri: &str) -> bool {
        // window.open() may create a blank document before navigating to sign-in.
        if uri == "about:blank" {
            return true;
        }
        let Some((_, rest)) = uri
            .split_once("://")
            .filter(|(scheme, _)| scheme.eq_ignore_ascii_case("https"))
        else {
            return false;
        };
        let host = rest
            .split(['/', '?', '#'])
            .next()
            .unwrap_or_default()
            .to_ascii_lowercase();
        // Identity providers may redirect through other HTTPS origins (for example,
        // an identity-provider domain). This is browser navigation, not credential export:
        // cookies retain browser origin scoping and capture still accepts only YouTube ARL.
        let host = host.strip_suffix(":443").unwrap_or(&host);
        !host.is_empty()
            && host.len() <= 253
            && host
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b".-".contains(&b))
    }
    unsafe fn text(
        get: impl FnOnce(*mut PWSTR) -> windows::core::Result<()>,
    ) -> windows::core::Result<String> {
        let mut value = PWSTR::null();
        get(&mut value)?;
        Ok(CoTaskMemPWSTR::from(value).to_string())
    }
    unsafe extern "system" fn window_proc(hwnd: HWND, msg: u32, wp: usize, lp: isize) -> isize {
        match msg {
            WM_SIZE => {
                CONTROLLER.with(|slot| {
                    if let Some(controller) = slot.borrow().as_ref() {
                        let mut r = RECT::default();
                        GetClientRect(hwnd, &mut r);
                        let _ = controller.SetBounds(windows::Win32::Foundation::RECT {
                            left: 0,
                            top: 0,
                            right: r.right,
                            bottom: r.bottom,
                        });
                    }
                });
                0
            }
            WM_CLOSE => {
                CLOSED.set(true);
                0
            }
            _ => DefWindowProcW(hwnd, msg, wp, lp),
        }
    }
    unsafe fn pump(hwnd: HWND, cancel: &AtomicBool, started: Instant) -> Result<(), String> {
        if cancel.load(Ordering::Relaxed) || CLOSED.get() {
            return Err("YouTube sign-in cancelled".into());
        }
        if started.elapsed() > Duration::from_secs(45) {
            return Err("YouTube sign-in timed out".into());
        }
        if IsWindow(hwnd) == 0 {
            return Err("YouTube sign-in window closed".into());
        }
        let mut message = MSG::default();
        while PeekMessageW(&mut message, std::ptr::null_mut(), 0, 0, PM_REMOVE) != 0 {
            if message.message == WM_QUIT {
                return Err("YouTube sign-in window closed".into());
            }
            TranslateMessage(&message);
            DispatchMessageW(&message);
        }
        std::thread::sleep(Duration::from_millis(10));
        Ok(())
    }
    unsafe fn wait<T>(
        slot: &Rc<RefCell<Option<Result<T, String>>>>,
        hwnd: HWND,
        cancel: &AtomicBool,
        started: Instant,
    ) -> Result<T, String> {
        loop {
            if let Some(value) = slot.borrow_mut().take() {
                return value;
            }
            pump(hwnd, cancel, started)?;
        }
    }
    struct Window(HWND);
    impl Drop for Window {
        fn drop(&mut self) {
            unsafe {
                CONTROLLER.with(|slot| {
                    if let Some(controller) = slot.borrow_mut().take() {
                        let _ = controller.Close();
                    }
                });
                if IsWindow(self.0) != 0 {
                    DestroyWindow(self.0);
                }
            }
        }
    }
    fn valid_youtube_cookie(name: &str, domain: &str, _secure: bool, value: &str) -> bool {
        name == "DEEZNATS_POT"
            && matches!(
                domain.trim_start_matches('.'),
                "youtube.com" | "www.youtube.com"
            )
            && !value.is_empty()
    }
    pub(crate) fn run(binding: &str, cancel: &AtomicBool) -> Result<String, String> {
        unsafe {
            if CoInitializeEx(std::ptr::null(), COINIT_APARTMENTTHREADED as u32) < 0 {
                return Err("Could not initialize sign-in window".into());
            }
            CLOSED.set(false);
            let url = format!(
                "https://www.youtube.com/embed/jNQXAC9IVRw?binding={}",
                url::form_urlencoded::byte_serialize(binding.as_bytes()).collect::<String>()
            );
            let result = run_sta(&url, cancel, false);
            CoUninitialize();
            result
        }
    }
    unsafe fn run_sta(url: &str, cancel: &AtomicBool, smoke: bool) -> Result<String, String> {
        if !url.starts_with("https://") || !allowed_navigation(url) {
            return Err("Invalid YouTube sign-in address".into());
        }
        let instance = GetModuleHandleW(std::ptr::null());
        let class_name = wide("DeezNatsYouTubePoToken");
        let class = WNDCLASSW {
            lpfnWndProc: Some(window_proc),
            hInstance: instance,
            lpszClassName: class_name.as_ptr(),
            hCursor: LoadCursorW(std::ptr::null_mut(), IDC_ARROW),
            hbrBackground: GetStockObject(BLACK_BRUSH) as HBRUSH,
            ..Default::default()
        };
        RegisterClassW(&class);
        let hwnd = CreateWindowExW(
            0,
            class_name.as_ptr(),
            wide("YouTube sign-in - DeezNats").as_ptr(),
            WS_OVERLAPPEDWINDOW,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            620,
            780,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            instance,
            std::ptr::null(),
        );
        if hwnd.is_null() {
            return Err("Could not create YouTube sign-in window".into());
        }
        let _window = Window(hwnd);
        if !smoke {
            ShowWindow(hwnd, SW_HIDE);
        }
        let started = Instant::now();
        let folder = crate::storage::data_dir()?.join("YouTubePoTokenWebView");
        std::fs::create_dir_all(&folder)
            .map_err(|_| "Could not prepare the isolated YouTube browser profile")?;
        let folder = wide(&folder.to_string_lossy());
        let environment = Rc::new(RefCell::new(None));
        let output = environment.clone();
        CreateCoreWebView2EnvironmentWithOptions(
            PCWSTR::null(),
            PCWSTR(folder.as_ptr()),
            None,
            &CreateCoreWebView2EnvironmentCompletedHandler::create(Box::new(
                move |result, environment| {
                    *output.borrow_mut() = Some(
                    result
                        .map_err(|_| {
                            "WebView2 Runtime could not start. Use normal browser sign-in instead."
                                .into()
                        })
                        .and_then(|_| {
                            environment.ok_or_else(|| "WebView2 environment is unavailable".into())
                        }),
                );
                    Ok(())
                },
            )),
        )
        .map_err(|_| "WebView2 Runtime is unavailable. Use normal browser sign-in instead.")?;
        let environment: ICoreWebView2Environment = wait(&environment, hwnd, cancel, started)?;
        let modern: ICoreWebView2Environment10 = environment
            .cast()
            .map_err(|_| "Update WebView2 Runtime to use private sign-in")?;
        let options = modern
            .CreateCoreWebView2ControllerOptions()
            .map_err(|_| "Private login options are unavailable")?;
        options
            .SetIsInPrivateModeEnabled(true)
            .map_err(|_| "Could not enable private YouTube sign-in")?;
        let controller = Rc::new(RefCell::new(None));
        let output = controller.clone();
        modern
            .CreateCoreWebView2ControllerWithOptions(
                windows::Win32::Foundation::HWND(hwnd),
                &options,
                &CreateCoreWebView2ControllerCompletedHandler::create(Box::new(
                    move |result, controller| {
                        *output.borrow_mut() = Some(
                            result
                                .map_err(|_| "YouTube WebView2 controller could not start".into())
                                .and_then(|_| {
                                    controller.ok_or_else(|| {
                                        "YouTube WebView2 controller is unavailable".into()
                                    })
                                }),
                        );
                        Ok(())
                    },
                )),
            )
            .map_err(|_| "Could not open private YouTube sign-in")?;
        let controller: ICoreWebView2Controller = wait(&controller, hwnd, cancel, started)?;
        CONTROLLER.with(|slot| *slot.borrow_mut() = Some(controller.clone()));
        let mut bounds = RECT::default();
        GetClientRect(hwnd, &mut bounds);
        controller
            .SetBounds(windows::Win32::Foundation::RECT {
                left: 0,
                top: 0,
                right: bounds.right,
                bottom: bounds.bottom,
            })
            .map_err(|_| "Could not size the YouTube window")?;
        controller
            .SetIsVisible(true)
            .map_err(|_| "Could not show YouTube sign-in")?;
        let webview = controller
            .CoreWebView2()
            .map_err(|_| "YouTube WebView2 is unavailable")?;
        let settings = webview
            .Settings()
            .map_err(|_| "YouTube WebView2 settings unavailable")?;
        settings
            .SetAreHostObjectsAllowed(false)
            .map_err(|_| "Could not isolate YouTube sign-in")?;
        settings
            .SetIsWebMessageEnabled(false)
            .map_err(|_| "Could not isolate YouTube messages")?;
        settings
            .SetAreDevToolsEnabled(false)
            .map_err(|_| "Could not configure YouTube sign-in")?;
        settings
            .SetAreDefaultContextMenusEnabled(false)
            .map_err(|_| "Could not configure YouTube context menus")?;
        let mut permission_token = 0;
        webview
            .add_PermissionRequested(
                &PermissionRequestedEventHandler::create(Box::new(|_, args| {
                    if let Some(args) = args {
                        args.SetState(COREWEBVIEW2_PERMISSION_STATE_DENY)?;
                    }
                    Ok(())
                })),
                &mut permission_token,
            )
            .map_err(|_| "Could not isolate YouTube permissions")?;
        let mut download_token = 0;
        let downloads: ICoreWebView2_4 = webview
            .cast()
            .map_err(|_| "Update WebView2 Runtime for isolated sign-in")?;
        downloads
            .add_DownloadStarting(
                &DownloadStartingEventHandler::create(Box::new(|_, args| {
                    if let Some(args) = args {
                        args.SetCancel(true)?;
                    }
                    Ok(())
                })),
                &mut download_token,
            )
            .map_err(|_| "Could not disable sign-in downloads")?;

        let captured = Rc::new(RefCell::new(None::<String>));
        let mut nav_token = 0;
        webview
            .add_NavigationStarting(
                &NavigationStartingEventHandler::create(Box::new(|_, args| {
                    if let Some(args) = args {
                        let uri = text(|out| args.Uri(out))?;
                        if !allowed_navigation(&uri) {
                            args.SetCancel(true)?;
                        }
                    }
                    Ok(())
                })),
                &mut nav_token,
            )
            .map_err(|_| "Could not configure sign-in navigation")?;
        let mut popup_token = 0;
        webview
            .add_NewWindowRequested(
                &NewWindowRequestedEventHandler::create(Box::new(|_, args| {
                    if let Some(args) = args {
                        let uri = text(|out| args.Uri(out))?;
                        args.SetHandled(!allowed_navigation(&uri))?;
                    }
                    Ok(())
                })),
                &mut popup_token,
            )
            .map_err(|_| "Could not configure sign-in popups")?;
        let modern: ICoreWebView2_2 = webview
            .cast()
            .map_err(|_| "Update WebView2 Runtime for sign-in")?;
        let cookies = modern
            .CookieManager()
            .map_err(|_| "YouTube cookie manager unavailable")?;
        if smoke {
            let cookie = cookies
                .CreateCookie(
                    PCWSTR(wide("arl").as_ptr()),
                    PCWSTR(wide(&"a".repeat(192)).as_ptr()),
                    PCWSTR(wide(".youtube.com").as_ptr()),
                    PCWSTR(wide("/").as_ptr()),
                )
                .map_err(|_| "Could not create synthetic cookie")?;
            cookie
                .SetIsSecure(true)
                .map_err(|_| "Could not secure synthetic cookie")?;
            cookies
                .AddOrUpdateCookie(&cookie)
                .map_err(|_| "Could not set synthetic cookie")?;
            let profile = webview
                .cast::<ICoreWebView2_13>()
                .map_err(|_| "Private profile unavailable")?
                .Profile()
                .map_err(|_| "Private profile unavailable")?;
            let mut private = windows::core::BOOL(0);
            profile
                .IsInPrivateModeEnabled(&mut private)
                .map_err(|_| "Could not verify private profile")?;
            if !private.as_bool() {
                return Err("Sign-in profile is not private".into());
            }
        }
        let registered = Rc::new(RefCell::new(None));
        let registration = registered.clone();
        webview
            .AddScriptToExecuteOnDocumentCreated(
                PCWSTR(wide(include_str!("youtube-potoken.js")).as_ptr()),
                &AddScriptToExecuteOnDocumentCreatedCompletedHandler::create(Box::new(
                    move |result, _| {
                        *registration.borrow_mut() =
                            Some(result.map_err(|_| {
                                "Could not register YouTube token script".to_string()
                            }));
                        Ok(())
                    },
                )),
            )
            .map_err(|_| "Could not load YouTube token script")?;
        wait(&registered, hwnd, cancel, started)?;
        webview
            .Navigate(PCWSTR(
                wide(if smoke { "about:blank" } else { url }).as_ptr(),
            ))
            .map_err(|_| "Could not open YouTube sign-in")?;
        let pending = Rc::new(Cell::new(false));
        let mut checked = Instant::now() - Duration::from_secs(1);
        let result = loop {
            if let Some(cookie) = captured.borrow_mut().take() {
                break Ok(cookie);
            }
            if let Err(error) = pump(hwnd, cancel, started) {
                break Err(error);
            }
            if checked.elapsed() >= Duration::from_millis(750) && !pending.get() {
                checked = Instant::now();
                pending.set(true);
                let output = captured.clone();
                let busy = pending.clone();
                if cookies
                    .GetCookies(
                        PCWSTR(wide("https://www.youtube.com/").as_ptr()),
                        &GetCookiesCompletedHandler::create(Box::new(move |result, list| {
                            busy.set(false);
                            if result.is_ok() {
                                if let Some(list) = list {
                                    let mut count = 0;
                                    list.Count(&mut count)?;
                                    let mut header = Vec::new();
                                    let mut signed_in = false;
                                    for i in 0..count.min(256) {
                                        let cookie = list.GetValueAtIndex(i)?;
                                        let name = text(|out| cookie.Name(out))?;
                                        let domain = text(|out| cookie.Domain(out))?;
                                        let value = text(|out| cookie.Value(out))?;
                                        let mut secure = windows::core::BOOL(0);
                                        cookie.IsSecure(&mut secure)?;
                                        if valid_youtube_cookie(
                                            &name,
                                            &domain,
                                            secure.as_bool(),
                                            &value,
                                        ) {
                                            signed_in = true;
                                            header.push(value);
                                        }
                                    }
                                    if signed_in {
                                        *output.borrow_mut() = Some(header.join(""));
                                    }
                                }
                            }
                            Ok(())
                        })),
                    )
                    .is_err()
                {
                    pending.set(false);
                }
            }
        };
        let _ = webview.remove_PermissionRequested(permission_token);
        let _ = downloads.remove_DownloadStarting(download_token);
        let _ = webview.remove_NavigationStarting(nav_token);
        let _ = webview.remove_NewWindowRequested(popup_token);
        result
    }
}
pub(crate) use catalog::{
    collection_tracks, connect_catalog, connect_station, edit as edit_playlist, invoke,
    menu_playlists, playlist_privacy, process, saved_library_entries,
};
pub(crate) use playback::Player;
pub(crate) fn genre_artwork(genre: &str) -> Result<String, String> {
    catalog::genre_artwork(genre)
}
use serde_json::{json, Value};
use std::sync::{Arc, Mutex, OnceLock};
use ytmusic::YtMusic;
pub(crate) mod routing {
    //! Decoded YouTube PCM, before Sink gain and host-only DSP. Never records an endpoint.
    use rodio::{source::SeekError, Source};
    use std::{
        collections::VecDeque,
        sync::{Arc, Mutex, OnceLock},
        time::Duration,
    };

    const SAMPLES: usize = 882; // 10 ms, 44.1 kHz stereo
    static LEVELS: Mutex<Option<(u64, crate::connect::spectrum::Snapshot)>> = Mutex::new(None);
    pub(crate) fn levels(trap: bool) -> Option<[f32; 32]> {
        let epoch = bus().lock().ok()?.epoch;
        let latest = LEVELS.lock().ok()?;
        let (generation, snapshot) = latest.as_ref()?;
        (*generation == epoch && snapshot.fresh()).then_some(if trap {
            snapshot.trap_levels
        } else {
            snapshot.levels
        })
    }
    fn analyzer() -> &'static std::sync::mpsc::SyncSender<(u64, Vec<u8>)> {
        static TX: OnceLock<std::sync::mpsc::SyncSender<(u64, Vec<u8>)>> = OnceLock::new();
        TX.get_or_init(|| {
            let (tx, rx) = std::sync::mpsc::sync_channel::<(u64, Vec<u8>)>(8);
            std::thread::spawn(move || {
                let mut analyzer = crate::connect::spectrum::Analyzer::new();
                let mut epoch = None;
                while let Ok((generation, pcm)) = rx.recv() {
                    if epoch != Some(generation) {
                        analyzer = crate::connect::spectrum::Analyzer::new();
                        epoch = Some(generation);
                    }
                    if let Some(levels) = analyzer.push(&pcm) {
                        if let Ok(mut latest) = LEVELS.lock() {
                            *latest = Some((
                                generation,
                                crate::connect::spectrum::Snapshot {
                                    levels,
                                    trap_levels: analyzer.scene_levels(),
                                    updated: Some(std::time::Instant::now()),
                                },
                            ));
                        }
                    }
                }
            });
            tx
        })
    }
    #[derive(Default)]
    struct Bus {
        epoch: u64,
        packets: VecDeque<Vec<u8>>,
    }
    fn bus() -> &'static Mutex<Bus> {
        static BUS: OnceLock<Mutex<Bus>> = OnceLock::new();
        BUS.get_or_init(Default::default)
    }
    pub(super) fn reset() {
        if let Ok(mut bus) = bus().lock() {
            bus.epoch = bus.epoch.wrapping_add(1);
            bus.packets.clear();
        }
    }
    pub(crate) fn drain() -> Vec<Vec<u8>> {
        bus()
            .lock()
            .map(|mut b| b.packets.drain(..).collect())
            .unwrap_or_default()
    }

    #[derive(Clone)]
    pub(super) struct Eq {
        pub enabled: bool,
        pub gains: [f32; 11],
    }
    impl Default for Eq {
        fn default() -> Self {
            Self {
                enabled: false,
                gains: [0.; 11],
            }
        }
    }
    impl Eq {
        pub fn update(&mut self, value: &serde_json::Value) {
            if value["reset"] == true {
                self.gains = [0.; 11];
            }
            if let Some(preset) = value["preset"].as_str() {
                if let Some(gains) = crate::ui::preset_gains(&preset.replace('-', " ")) {
                    for (out, gain) in self.gains.iter_mut().zip(gains) {
                        *out = gain as f32;
                    }
                }
            }
            if let Some(enabled) = value["enabled"].as_bool() {
                self.enabled = enabled;
            }
            if let Some(gains) = value["gainsDb"].as_array() {
                for (out, gain) in self.gains[..10].iter_mut().zip(gains) {
                    if let Some(gain) = gain.as_f64() {
                        *out = (gain as f32).clamp(-12., 12.);
                    }
                }
            }
            if let Some(gain) = value["preampDb"].as_f64() {
                self.gains[10] = (gain as f32).clamp(-12., 12.);
            }
            if let (Some(index), Some(gain)) = (
                value["band"]["index"].as_u64(),
                value["band"]["gainDb"].as_f64(),
            ) {
                if index < 10 {
                    self.gains[index as usize] = (gain as f32).clamp(-12., 12.);
                }
            }
        }
    }
    #[derive(Clone, Copy, Default)]
    struct Filter {
        c: [f32; 5],
        z: [[f32; 2]; 2],
    }
    impl Filter {
        fn configure(&mut self, hz: f32, db: f32) {
            let a = 10f32.powf(db / 40.);
            let w = 2. * std::f32::consts::PI * hz / 44100.;
            let alpha = w.sin() / 2. / 1.4;
            let a0 = 1. + alpha / a;
            self.c = [
                (1. + alpha * a) / a0,
                -2. * w.cos() / a0,
                (1. - alpha * a) / a0,
                -2. * w.cos() / a0,
                (1. - alpha / a) / a0,
            ];
        }
        fn sample(&mut self, x: f32, channel: usize) -> f32 {
            let [b0, b1, b2, a1, a2] = self.c;
            let z = &mut self.z[channel];
            let y = b0 * x + z[0];
            z[0] = b1 * x - a1 * y + z[1];
            z[1] = b2 * x - a2 * y;
            y
        }
    }
    pub(super) struct Routed<S> {
        source: S,
        epoch: u64,
        pcm: Vec<u8>,
        channel: usize,
        eq: Arc<Mutex<Eq>>,
        settings: Eq,
        filters: [Filter; 10],
    }
    impl<S: Source> Routed<S> {
        pub fn new(source: S, eq: Arc<Mutex<Eq>>) -> Self {
            debug_assert_eq!((source.channels(), source.sample_rate()), (2, 44100));
            let _ = analyzer();
            let epoch = bus().lock().map(|b| b.epoch).unwrap_or_default();
            Self {
                source,
                epoch,
                pcm: Vec::with_capacity(SAMPLES * 2),
                channel: 0,
                eq,
                settings: Eq::default(),
                filters: [Filter::default(); 10],
            }
        }
    }
    impl<S: Source> Iterator for Routed<S> {
        type Item = f32;
        fn next(&mut self) -> Option<f32> {
            let Some(x) = self.source.next() else {
                if !self.pcm.is_empty() {
                    self.publish();
                }
                return None;
            };
            if self.pcm.is_empty() {
                if let Ok(eq) = self.eq.try_lock() {
                    self.settings = eq.clone();
                }
                for ((filter, hz), gain) in self
                    .filters
                    .iter_mut()
                    .zip([
                        31., 62., 125., 250., 500., 1000., 2000., 4000., 8000., 16000.,
                    ])
                    .zip(self.settings.gains)
                {
                    filter.configure(hz, gain);
                }
            }
            self.pcm
                .extend_from_slice(&((x.clamp(-1., 1.) * 32767.).round() as i16).to_le_bytes());
            if self.pcm.len() == SAMPLES * 2 {
                self.publish();
            }
            let mut y = x;
            if self.settings.enabled {
                for filter in &mut self.filters {
                    y = filter.sample(y, self.channel);
                }
                y *= 10f32.powf(self.settings.gains[10] / 20.);
            }
            self.channel ^= 1;
            Some(y.clamp(-1., 1.))
        }
    }
    impl<S> Routed<S> {
        fn publish(&mut self) {
            let _ = analyzer().try_send((self.epoch, self.pcm.clone()));
            // Audio callback never waits for Connect or analysis. Bound backlog to 200 ms.
            if let Ok(mut bus) = bus().try_lock() {
                if bus.epoch == self.epoch {
                    if bus.packets.len() == 20 {
                        bus.packets.pop_front();
                    }
                    bus.packets.push_back(self.pcm.clone());
                }
            }
            self.pcm.clear();
        }
    }
    impl<S: Source> Source for Routed<S> {
        fn current_span_len(&self) -> Option<usize> {
            self.source.current_span_len()
        }
        fn channels(&self) -> u16 {
            2
        }
        fn sample_rate(&self) -> u32 {
            44100
        }
        fn total_duration(&self) -> Option<Duration> {
            self.source.total_duration()
        }
        fn try_seek(&mut self, pos: Duration) -> Result<(), SeekError> {
            self.source.try_seek(pos)?;
            reset();
            self.epoch = bus().lock().map(|b| b.epoch).unwrap_or_default();
            self.pcm.clear();
            self.channel = 0;
            for filter in &mut self.filters {
                filter.z = [[0.; 2]; 2];
            }
            Ok(())
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        #[test]
        fn pre_volume_pcm_survives_host_mute_eq_and_resets() {
            reset();
            let eq = Arc::new(Mutex::new(Eq::default()));
            eq.lock().unwrap().update(
                &serde_json::json!({"enabled":true,"preampDb":-12,"band":{"index":5,"gainDb":-12}}),
            );
            let samples: Vec<f32> = (0..SAMPLES * 20)
                .map(|i| ((i / 2) as f32 * std::f32::consts::TAU * 1000. / 44100.).sin() * 0.5)
                .collect();
            let source = rodio::buffer::SamplesBuffer::new(2, 44100, samples.clone());
            let routed = Routed::new(source, eq);
            // Same ordering used in production: Sink's gain is outside Routed.
            let local: Vec<_> = routed.amplify(0.).collect();
            assert!(local.iter().all(|x| *x == 0.));
            let pcm: Vec<u8> = drain().into_iter().flatten().collect();
            assert_eq!(pcm.len(), samples.len() * 2);
            for (raw, expected) in pcm.chunks_exact(2).zip(&samples) {
                assert_eq!(
                    i16::from_le_bytes([raw[0], raw[1]]),
                    (expected * 32767.).round() as i16
                );
            }
            let eq = Arc::new(Mutex::new(Eq::default()));
            eq.lock()
                .unwrap()
                .update(&serde_json::json!({"enabled": true, "preampDb": -12}));
            let source = rodio::buffer::SamplesBuffer::new(2, 44100, samples.clone());
            let local: Vec<_> = Routed::new(source, eq).collect();
            assert!(local.iter().all(|x| x.abs() < 0.13));
            assert!(drain()
                .into_iter()
                .flatten()
                .collect::<Vec<_>>()
                .chunks_exact(2)
                .any(|x| i16::from_le_bytes([x[0], x[1]]).abs() > 15000));
            let source = rodio::buffer::SamplesBuffer::new(2, 44100, samples.clone());
            let mut routed = Routed::new(source, Arc::new(Mutex::new(Eq::default())));
            // Epoch invalidation prevents a stopped/replaced decoder publishing old packets.
            reset();
            for _ in 0..SAMPLES {
                routed.next();
            }
            assert!(drain().is_empty());
            let source = rodio::buffer::SamplesBuffer::new(2, 44100, samples);
            let mut routed = Routed::new(source, Arc::new(Mutex::new(Eq::default())));
            for _ in 0..SAMPLES {
                routed.next();
            }
            assert!(!drain().is_empty());
            // Independent analysis works without constructing a ConnectHub or endpoint.
            routed.try_seek(Duration::ZERO).unwrap();
            assert!(drain().is_empty());
            for _ in 0..SAMPLES {
                routed.next();
            }
            assert!(!drain().is_empty());
            let source = rodio::buffer::SamplesBuffer::new(
                2,
                44100,
                (0..SAMPLES * 40)
                    .map(|i| ((i / 2) as f32 * std::f32::consts::TAU * 1000. / 44100.).sin() * 0.5)
                    .collect::<Vec<_>>(),
            );
            let mut routed = Routed::new(source, Arc::new(Mutex::new(Eq::default())));
            for _ in 0..40 {
                for _ in 0..SAMPLES {
                    routed.next();
                }
                std::thread::sleep(Duration::from_millis(5));
            }
            assert!(levels(false).unwrap().iter().any(|level| *level > 0.));
            assert!(bus().lock().unwrap().packets.len() <= 20);
            reset();
            assert!(levels(false).is_none());
            // Exercise the actual loopback HTTP/WAV path with a muted synthetic host.
            use std::io::{Read, Write};
            let (tx, commands) = std::sync::mpsc::sync_channel(4);
            let hub = crate::connect::ConnectHub::start(0, tx).unwrap();
            hub.update(&crate::audio::Snapshot {
                provider: crate::catalog::Provider::YouTube,
                playback_connected: true,
                state: "Playing".into(),
                ..Default::default()
            });
            let port = hub.port();
            let client = std::thread::spawn(move || {
                let mut stream = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(4)))
                    .unwrap();
                stream
                    .write_all(b"GET /api/stream?buffer_ms=250 HTTP/1.1\r\nHost: localhost\r\n\r\n")
                    .unwrap();
                let mut head = Vec::new();
                while !head.ends_with(b"\r\n\r\n") {
                    let mut byte = [0];
                    stream.read_exact(&mut byte).unwrap();
                    head.push(byte[0]);
                    assert!(head.len() < 4096);
                }
                assert!(head.starts_with(b"HTTP/1.1 200"));
                let mut wav = [0; 44];
                stream.read_exact(&mut wav).unwrap();
                assert_eq!(&wav[..4], b"RIFF");
                let mut pcm = [0; 4096];
                stream.read_exact(&mut pcm).unwrap();
                assert!(pcm
                    .chunks_exact(2)
                    .any(|x| i16::from_le_bytes([x[0], x[1]]).abs() > 15000));
            });
            let source = rodio::buffer::SamplesBuffer::new(
                2,
                44100,
                (0..SAMPLES * 100)
                    .map(|i| ((i / 2) as f32 * std::f32::consts::TAU * 1000. / 44100.).sin() * 0.5)
                    .collect::<Vec<_>>(),
            );
            let mut muted = Routed::new(source, Arc::new(Mutex::new(Eq::default()))).amplify(0.);
            for _ in 0..100 {
                for _ in 0..SAMPLES {
                    assert_eq!(muted.next(), Some(0.));
                }
                std::thread::sleep(Duration::from_millis(5));
            }
            client.join().unwrap();
            for (path, expected) in [
                ("/api/state", "\"provider\":\"youtube\""),
                ("/api/queue", "\"provider\":\"youtube\""),
                ("/api/search?q=", "\"provider\":\"youtube\""),
            ] {
                let mut stream = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(3)))
                    .unwrap();
                write!(stream, "GET {path} HTTP/1.1\r\nHost: localhost\r\n\r\n").unwrap();
                let mut response = String::new();
                stream.read_to_string(&mut response).unwrap();
                assert!(response.contains(expected), "{response}");
            }
            assert!(commands.try_recv().is_err());
        }
    }
}
// Serializes publication, cookie writes, and logout so an old request cannot
// recreate credentials after sign-out or overwrite a newer account.
static SESSION: Mutex<u64> = Mutex::new(0);
static NEXT_SESSION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
fn session_id() -> u64 {
    NEXT_SESSION.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}
fn protected_session(api: YtMusic, id: u64, authuser: usize) -> YtMusic {
    api.cookie_sink(Arc::new(move |store| {
        persist_if_current(&SESSION, id, || {
            if crate::session::provider_save(
                "youtube",
                &json!({"cookie_store":store,"authuser":authuser}).to_string(),
            )
            .is_err()
            {
                eprintln!("YouTube: could not persist protected session cookies");
            }
        });
    }))
}
fn persist_if_current(active: &Mutex<u64>, id: u64, save: impl FnOnce()) {
    let Ok(active) = active.lock() else {
        return;
    };
    if *active == id {
        save();
    }
}
fn encoded_session(api: &YtMusic, authuser: usize) -> Result<String, String> {
    let store = api
        .cookie_store()
        .map_err(|_| "Cannot encode YouTube session")?;
    Ok(json!({"cookie_store":store,"authuser":authuser}).to_string())
}
static API: Mutex<Option<Arc<YtMusic>>> = Mutex::new(None);
pub(crate) fn runtime() -> &'static tokio::runtime::Runtime {
    static RT: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RT.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("YouTube runtime")
    })
}
pub(crate) fn api() -> Result<Arc<YtMusic>, String> {
    let mut guard = API.lock().map_err(|_| "YouTube session lock unavailable")?;
    if let Some(api) = guard.as_ref() {
        return Ok(api.clone());
    }
    let saved = crate::session::provider_saved("youtube")
        .map_err(|_| "Sign in to YouTube Music to continue.")?;
    let value: Value = serde_json::from_str(&saved).map_err(|_| "Invalid YouTube session")?;
    let authuser = value["authuser"].as_u64().unwrap_or(0) as usize;
    // Legacy header-only sessions are upgraded on the next response/rotation.
    let mut api = YtMusic::with_cookies(value["cookies"].as_str().unwrap_or("")).as_user(authuser);
    if let Some(store) = value["cookie_store"].as_str() {
        api = api
            .restore_cookie_store(store)
            .map_err(|_| "Invalid YouTube cookie store")?;
    }
    let id = session_id();
    let api = protected_session(api, id, authuser);
    if !api.is_authenticated() {
        return Err("Sign in to YouTube Music to continue.".into());
    }
    let api = Arc::new(api.mint_po_tokens(Arc::new(potoken::Minter)));
    *SESSION
        .lock()
        .map_err(|_| "YouTube session lock unavailable")? = id;
    rotate(&api, true);
    *guard = Some(api.clone());
    Ok(api)
}
pub(crate) fn login(cancel: &std::sync::atomic::AtomicBool) -> Result<Value, String> {
    let cookies = crate::youtube_webview::run(cancel)?;
    if cancel.load(std::sync::atomic::Ordering::Relaxed) {
        return Err("YouTube sign-in cancelled".into());
    }
    login_cookies_with_cancel(&cookies, cancel)
}
pub(crate) fn login_cookies_with_cancel(
    cookies: &str,
    cancel: &std::sync::atomic::AtomicBool,
) -> Result<Value, String> {
    let cookies = auth::header(cookies).map_err(|e| e.to_string())?;
    let id = session_id();
    let api = Arc::new(
        protected_session(YtMusic::with_cookies(&cookies), id, 0)
            .mint_po_tokens(Arc::new(potoken::Minter)),
    );
    let profile = runtime()
        .block_on(api.profile())
        .map_err(|e| format!("YouTube sign-in: {e}"))?;
    if cancel.load(std::sync::atomic::Ordering::Relaxed) {
        return Err("YouTube sign-in cancelled".into());
    }
    let mut guard = API.lock().map_err(|_| "YouTube session lock unavailable")?;
    let mut active = SESSION
        .lock()
        .map_err(|_| "YouTube session lock unavailable")?;
    crate::session::provider_save("youtube", &encoded_session(&api, 0)?)?;
    *active = id;
    drop(active);
    rotate(&api, false);
    if let Ok(mut history) = HISTORY.lock() {
        history.clear();
    }
    *guard = Some(api);
    Ok(json!({"id":profile.email.unwrap_or_else(|| profile.name.clone()),"name":profile.name}))
}
pub(crate) fn logout() -> Result<(), String> {
    let mut guard = API.lock().map_err(|_| "YouTube session lock unavailable")?;
    let mut active = SESSION
        .lock()
        .map_err(|_| "YouTube session lock unavailable")?;
    crate::session::provider_remove("youtube")?;
    *active = 0;
    *guard = None;
    if let Ok(mut h) = HISTORY.lock() {
        h.clear();
    }
    Ok(())
}
pub(crate) fn account() -> Result<Value, String> {
    let api = api()?;
    let p = runtime()
        .block_on(api.profile())
        .map_err(|e| e.to_string())?;
    Ok(json!({"id":p.email.unwrap_or_else(|| p.name.clone()),"name":p.name}))
}

static HISTORY: Mutex<Vec<crate::catalog::Entry>> = Mutex::new(Vec::new());
pub(super) fn remember(entry: &crate::catalog::Entry) {
    if let Ok(mut history) = HISTORY.lock() {
        history.retain(|t| t.id != entry.id);
        history.insert(0, entry.clone());
        history.truncate(100);
    }
}
pub(super) fn history() -> Vec<crate::catalog::Entry> {
    HISTORY.lock().map(|h| h.clone()).unwrap_or_default()
}

static REFRESHING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub(crate) fn refreshing() -> bool {
    REFRESHING.load(std::sync::atomic::Ordering::Relaxed)
}

fn rotate(api: &Arc<YtMusic>, restored: bool) {
    if !api.is_authenticated() {
        return;
    }
    let weak = Arc::downgrade(api);
    runtime().spawn(async move {
        let mut rotated = if restored {
            None
        } else {
            Some(std::time::SystemTime::now())
        };
        loop {
            if weak.strong_count() == 0 {
                break;
            }
            // Short checks use wall time so suspend/resume cannot postpone renewal.
            if rotation_due(rotated, std::time::SystemTime::now()) {
                let Some(api) = weak.upgrade() else {
                    break;
                };
                REFRESHING.store(true, std::sync::atomic::Ordering::Relaxed);
                match tokio::time::timeout(std::time::Duration::from_secs(30), api.rotate_cookies())
                    .await
                {
                    Ok(Ok(())) => rotated = Some(std::time::SystemTime::now()),
                    _ => eprintln!("YouTube: cookie renewal failed; retrying in 30 seconds"),
                }
                REFRESHING.store(false, std::sync::atomic::Ordering::Relaxed);
            }
            tokio::time::sleep(std::time::Duration::from_secs(30)).await;
        }
    });
}

fn rotation_due(last: Option<std::time::SystemTime>, now: std::time::SystemTime) -> bool {
    last.is_none_or(|last| {
        now.duration_since(last).map_or(true, |elapsed| {
            elapsed >= std::time::Duration::from_secs(600)
        })
    })
}
#[cfg(test)]
mod session_tests {
    use super::*;
    #[test]
    #[ignore = "requires a saved YouTube session and network access"]
    fn live_saved_account_and_library() {
        let api = api().expect("saved YouTube session unavailable");
        runtime().block_on(async {
            let library =
                tokio::time::timeout(std::time::Duration::from_secs(30), api.library_playlists())
                    .await;
            let profile =
                tokio::time::timeout(std::time::Duration::from_secs(30), api.profile()).await;
            assert!(
                library.as_ref().is_ok_and(|value| value.is_ok()),
                "library request failed: {library:?}"
            );
            assert!(
                profile.as_ref().is_ok_and(|value| value.is_ok()),
                "account request failed"
            );
        });
    }
    #[test]
    fn stale_requests_cannot_write_after_logout_or_account_switch() {
        let active = Mutex::new(7);
        let writes = std::cell::Cell::new(0);
        persist_if_current(&active, 7, || writes.set(writes.get() + 1));
        *active.lock().unwrap() = 0;
        persist_if_current(&active, 7, || writes.set(writes.get() + 1));
        *active.lock().unwrap() = 8;
        persist_if_current(&active, 7, || writes.set(writes.get() + 1));
        persist_if_current(&active, 8, || writes.set(writes.get() + 1));
        assert_eq!(writes.get(), 2);
    }
    #[test]
    fn renewal_covers_restore_suspend_and_clock_rollback() {
        use std::time::{Duration, UNIX_EPOCH};
        let last = UNIX_EPOCH + Duration::from_secs(1000);
        assert!(rotation_due(None, last));
        assert!(!rotation_due(Some(last), last + Duration::from_secs(599)));
        assert!(rotation_due(Some(last), last + Duration::from_secs(600)));
        assert!(rotation_due(Some(last), last + Duration::from_secs(3600)));
        assert!(rotation_due(Some(last), UNIX_EPOCH));
    }
}

pub(crate) mod youtube_webview {
    //! Short-lived private WebView2 login for the YouTube Music cookie session.
    use std::{
        cell::{Cell, RefCell},
        rc::Rc,
        sync::atomic::{AtomicBool, Ordering},
        time::{Duration, Instant},
    };
    use webview2_com::{
        CoTaskMemPWSTR, CreateCoreWebView2ControllerCompletedHandler,
        CreateCoreWebView2EnvironmentCompletedHandler, DownloadStartingEventHandler,
        GetCookiesCompletedHandler, Microsoft::Web::WebView2::Win32::*,
        NavigationStartingEventHandler, NewWindowRequestedEventHandler,
        PermissionRequestedEventHandler,
    };
    use windows::core::{Interface, PCWSTR, PWSTR};
    use windows_sys::Win32::{
        Foundation::*,
        Graphics::Gdi::*,
        System::{Com::*, LibraryLoader::GetModuleHandleW},
        UI::WindowsAndMessaging::*,
    };
    thread_local! { static CONTROLLER:RefCell<Option<ICoreWebView2Controller>>=const {RefCell::new(None)}; static CLOSED:Cell<bool>=const {Cell::new(false)}; }
    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(Some(0)).collect()
    }
    fn allowed_navigation(uri: &str) -> bool {
        // window.open() may create a blank document before navigating to sign-in.
        if uri == "about:blank" {
            return true;
        }
        let Some((_, rest)) = uri
            .split_once("://")
            .filter(|(scheme, _)| scheme.eq_ignore_ascii_case("https"))
        else {
            return false;
        };
        let host = rest
            .split(['/', '?', '#'])
            .next()
            .unwrap_or_default()
            .to_ascii_lowercase();
        // Identity providers may redirect through other HTTPS origins (for example,
        // an identity-provider domain). This is browser navigation, not credential export:
        // cookies retain browser origin scoping and capture still accepts only YouTube ARL.
        let host = host.strip_suffix(":443").unwrap_or(&host);
        !host.is_empty()
            && host.len() <= 253
            && host
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b".-".contains(&b))
    }
    unsafe fn text(
        get: impl FnOnce(*mut PWSTR) -> windows::core::Result<()>,
    ) -> windows::core::Result<String> {
        let mut value = PWSTR::null();
        get(&mut value)?;
        Ok(CoTaskMemPWSTR::from(value).to_string())
    }
    unsafe extern "system" fn window_proc(hwnd: HWND, msg: u32, wp: usize, lp: isize) -> isize {
        match msg {
            WM_SIZE => {
                CONTROLLER.with(|slot| {
                    if let Some(controller) = slot.borrow().as_ref() {
                        let mut r = RECT::default();
                        GetClientRect(hwnd, &mut r);
                        let _ = controller.SetBounds(windows::Win32::Foundation::RECT {
                            left: 0,
                            top: 0,
                            right: r.right,
                            bottom: r.bottom,
                        });
                    }
                });
                0
            }
            WM_CLOSE => {
                CLOSED.set(true);
                0
            }
            _ => DefWindowProcW(hwnd, msg, wp, lp),
        }
    }
    unsafe fn pump(hwnd: HWND, cancel: &AtomicBool, started: Instant) -> Result<(), String> {
        if cancel.load(Ordering::Relaxed) || CLOSED.get() {
            return Err("YouTube sign-in cancelled".into());
        }
        if started.elapsed() > Duration::from_secs(300) {
            return Err("YouTube sign-in timed out".into());
        }
        if IsWindow(hwnd) == 0 {
            return Err("YouTube sign-in window closed".into());
        }
        let mut message = MSG::default();
        while PeekMessageW(&mut message, std::ptr::null_mut(), 0, 0, PM_REMOVE) != 0 {
            if message.message == WM_QUIT {
                return Err("YouTube sign-in window closed".into());
            }
            TranslateMessage(&message);
            DispatchMessageW(&message);
        }
        std::thread::sleep(Duration::from_millis(10));
        Ok(())
    }
    unsafe fn wait<T>(
        slot: &Rc<RefCell<Option<Result<T, String>>>>,
        hwnd: HWND,
        cancel: &AtomicBool,
        started: Instant,
    ) -> Result<T, String> {
        loop {
            if let Some(value) = slot.borrow_mut().take() {
                return value;
            }
            pump(hwnd, cancel, started)?;
        }
    }
    struct Window(HWND);
    impl Drop for Window {
        fn drop(&mut self) {
            unsafe {
                CONTROLLER.with(|slot| {
                    if let Some(controller) = slot.borrow_mut().take() {
                        let _ = controller.Close();
                    }
                });
                if IsWindow(self.0) != 0 {
                    DestroyWindow(self.0);
                }
            }
        }
    }
    fn valid_youtube_cookie(name: &str, domain: &str, _secure: bool, value: &str) -> bool {
        let domain = domain.trim_start_matches('.');
        (domain == "youtube.com" || domain.ends_with(".youtube.com"))
            && !name.is_empty()
            && name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
            && !value.is_empty()
            && !value.contains([';', '\r', '\n'])
    }

    // Google installs SID/SAPISID before the cross-domain sign-in handoff has
    // finished. LOGIN_INFO and a return to Music distinguish a completed handoff.
    fn capture_ready(uri: &str, names: &[&str]) -> bool {
        let music = url::Url::parse(uri).is_ok_and(|url| {
            url.scheme() == "https"
                && url.host_str() == Some("music.youtube.com")
                && !url.path().starts_with("/signin")
        });
        music
            && names.contains(&"SID")
            && names.contains(&"LOGIN_INFO")
            && names
                .iter()
                .any(|name| matches!(*name, "SAPISID" | "__Secure-3PAPISID"))
    }
    #[cfg(test)]
    mod tests {
        use super::{capture_ready, valid_youtube_cookie};
        #[test]
        fn capture_waits_for_completed_music_sign_in() {
            let partial = ["SID", "SAPISID"];
            let complete = ["SID", "SAPISID", "LOGIN_INFO"];
            assert!(!capture_ready("https://music.youtube.com/", &partial));
            assert!(!capture_ready(
                "https://accounts.google.com/ServiceLogin",
                &complete
            ));
            assert!(!capture_ready(
                "https://music.youtube.com.evil.test/",
                &complete
            ));
            assert!(!capture_ready(
                "https://music.youtube.com/signin",
                &complete
            ));
            assert!(capture_ready("https://music.youtube.com/", &complete));
            assert!(capture_ready(
                "https://music.youtube.com/",
                &["SID", "__Secure-3PAPISID", "LOGIN_INFO"]
            ));
        }

        #[test]
        fn accepts_youtube_session_cookies_without_secure_flag() {
            assert!(valid_youtube_cookie("SID", ".youtube.com", false, "value"));
            assert!(valid_youtube_cookie(
                "__Secure-3PAPISID",
                ".youtube.com",
                true,
                "value"
            ));
            assert!(!valid_youtube_cookie(
                "SID",
                "youtube.com.example.com",
                false,
                "value"
            ));
            assert!(!valid_youtube_cookie(
                "SID",
                ".youtube.com",
                false,
                "bad;value"
            ));
        }
    }
    pub(crate) fn run(cancel: &AtomicBool) -> Result<String, String> {
        unsafe {
            if CoInitializeEx(std::ptr::null(), COINIT_APARTMENTTHREADED as u32) < 0 {
                return Err("Could not initialize sign-in window".into());
            }
            CLOSED.set(false);
            let result = run_sta("https://accounts.google.com/ServiceLogin?service=youtube&continue=https%3A%2F%2Fmusic.youtube.com%2F", cancel, false);
            CoUninitialize();
            result
        }
    }
    unsafe fn run_sta(url: &str, cancel: &AtomicBool, smoke: bool) -> Result<String, String> {
        if !url.starts_with("https://") || !allowed_navigation(url) {
            return Err("Invalid YouTube sign-in address".into());
        }
        let instance = GetModuleHandleW(std::ptr::null());
        let class_name = wide("DeezNatsYouTubeAuth");
        let class = WNDCLASSW {
            lpfnWndProc: Some(window_proc),
            hInstance: instance,
            lpszClassName: class_name.as_ptr(),
            hCursor: LoadCursorW(std::ptr::null_mut(), IDC_ARROW),
            hbrBackground: GetStockObject(BLACK_BRUSH) as HBRUSH,
            ..Default::default()
        };
        RegisterClassW(&class);
        let hwnd = CreateWindowExW(
            0,
            class_name.as_ptr(),
            wide("YouTube sign-in - DeezNats").as_ptr(),
            WS_OVERLAPPEDWINDOW,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            620,
            780,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            instance,
            std::ptr::null(),
        );
        if hwnd.is_null() {
            return Err("Could not create YouTube sign-in window".into());
        }
        let _window = Window(hwnd);
        if !smoke {
            ShowWindow(hwnd, SW_SHOWNORMAL);
        }
        let started = Instant::now();
        let folder = crate::storage::data_dir()?.join("YouTubeAuthWebView");
        std::fs::create_dir_all(&folder)
            .map_err(|_| "Could not prepare the isolated YouTube browser profile")?;
        let folder = wide(&folder.to_string_lossy());
        let environment = Rc::new(RefCell::new(None));
        let output = environment.clone();
        CreateCoreWebView2EnvironmentWithOptions(
            PCWSTR::null(),
            PCWSTR(folder.as_ptr()),
            None,
            &CreateCoreWebView2EnvironmentCompletedHandler::create(Box::new(
                move |result, environment| {
                    *output.borrow_mut() = Some(
                    result
                        .map_err(|_| {
                            "WebView2 Runtime could not start. Use normal browser sign-in instead."
                                .into()
                        })
                        .and_then(|_| {
                            environment.ok_or_else(|| "WebView2 environment is unavailable".into())
                        }),
                );
                    Ok(())
                },
            )),
        )
        .map_err(|_| "WebView2 Runtime is unavailable. Use normal browser sign-in instead.")?;
        let environment: ICoreWebView2Environment = wait(&environment, hwnd, cancel, started)?;
        let modern: ICoreWebView2Environment10 = environment
            .cast()
            .map_err(|_| "Update WebView2 Runtime to use private sign-in")?;
        let options = modern
            .CreateCoreWebView2ControllerOptions()
            .map_err(|_| "Private login options are unavailable")?;
        options
            .SetIsInPrivateModeEnabled(true)
            .map_err(|_| "Could not enable private YouTube sign-in")?;
        let controller = Rc::new(RefCell::new(None));
        let output = controller.clone();
        modern
            .CreateCoreWebView2ControllerWithOptions(
                windows::Win32::Foundation::HWND(hwnd),
                &options,
                &CreateCoreWebView2ControllerCompletedHandler::create(Box::new(
                    move |result, controller| {
                        *output.borrow_mut() = Some(
                            result
                                .map_err(|_| "YouTube WebView2 controller could not start".into())
                                .and_then(|_| {
                                    controller.ok_or_else(|| {
                                        "YouTube WebView2 controller is unavailable".into()
                                    })
                                }),
                        );
                        Ok(())
                    },
                )),
            )
            .map_err(|_| "Could not open private YouTube sign-in")?;
        let controller: ICoreWebView2Controller = wait(&controller, hwnd, cancel, started)?;
        CONTROLLER.with(|slot| *slot.borrow_mut() = Some(controller.clone()));
        let mut bounds = RECT::default();
        GetClientRect(hwnd, &mut bounds);
        controller
            .SetBounds(windows::Win32::Foundation::RECT {
                left: 0,
                top: 0,
                right: bounds.right,
                bottom: bounds.bottom,
            })
            .map_err(|_| "Could not size the YouTube window")?;
        controller
            .SetIsVisible(true)
            .map_err(|_| "Could not show YouTube sign-in")?;
        let webview = controller
            .CoreWebView2()
            .map_err(|_| "YouTube WebView2 is unavailable")?;
        let settings = webview
            .Settings()
            .map_err(|_| "YouTube WebView2 settings unavailable")?;
        settings
            .SetAreHostObjectsAllowed(false)
            .map_err(|_| "Could not isolate YouTube sign-in")?;
        settings
            .SetIsWebMessageEnabled(false)
            .map_err(|_| "Could not isolate YouTube messages")?;
        settings
            .SetAreDevToolsEnabled(false)
            .map_err(|_| "Could not configure YouTube sign-in")?;
        settings
            .SetAreDefaultContextMenusEnabled(false)
            .map_err(|_| "Could not configure YouTube context menus")?;
        let mut permission_token = 0;
        webview
            .add_PermissionRequested(
                &PermissionRequestedEventHandler::create(Box::new(|_, args| {
                    if let Some(args) = args {
                        args.SetState(COREWEBVIEW2_PERMISSION_STATE_DENY)?;
                    }
                    Ok(())
                })),
                &mut permission_token,
            )
            .map_err(|_| "Could not isolate YouTube permissions")?;
        let mut download_token = 0;
        let downloads: ICoreWebView2_4 = webview
            .cast()
            .map_err(|_| "Update WebView2 Runtime for isolated sign-in")?;
        downloads
            .add_DownloadStarting(
                &DownloadStartingEventHandler::create(Box::new(|_, args| {
                    if let Some(args) = args {
                        args.SetCancel(true)?;
                    }
                    Ok(())
                })),
                &mut download_token,
            )
            .map_err(|_| "Could not disable sign-in downloads")?;

        let captured = Rc::new(RefCell::new(None::<String>));
        let mut nav_token = 0;
        webview
            .add_NavigationStarting(
                &NavigationStartingEventHandler::create(Box::new(|_, args| {
                    if let Some(args) = args {
                        let uri = text(|out| args.Uri(out))?;
                        if !allowed_navigation(&uri) {
                            args.SetCancel(true)?;
                        }
                    }
                    Ok(())
                })),
                &mut nav_token,
            )
            .map_err(|_| "Could not configure sign-in navigation")?;
        let mut popup_token = 0;
        webview
            .add_NewWindowRequested(
                &NewWindowRequestedEventHandler::create(Box::new(|_, args| {
                    if let Some(args) = args {
                        let uri = text(|out| args.Uri(out))?;
                        args.SetHandled(!allowed_navigation(&uri))?;
                    }
                    Ok(())
                })),
                &mut popup_token,
            )
            .map_err(|_| "Could not configure sign-in popups")?;
        let modern: ICoreWebView2_2 = webview
            .cast()
            .map_err(|_| "Update WebView2 Runtime for sign-in")?;
        let cookies = modern
            .CookieManager()
            .map_err(|_| "YouTube cookie manager unavailable")?;
        if smoke {
            let cookie = cookies
                .CreateCookie(
                    PCWSTR(wide("arl").as_ptr()),
                    PCWSTR(wide(&"a".repeat(192)).as_ptr()),
                    PCWSTR(wide(".youtube.com").as_ptr()),
                    PCWSTR(wide("/").as_ptr()),
                )
                .map_err(|_| "Could not create synthetic cookie")?;
            cookie
                .SetIsSecure(true)
                .map_err(|_| "Could not secure synthetic cookie")?;
            cookies
                .AddOrUpdateCookie(&cookie)
                .map_err(|_| "Could not set synthetic cookie")?;
            let profile = webview
                .cast::<ICoreWebView2_13>()
                .map_err(|_| "Private profile unavailable")?
                .Profile()
                .map_err(|_| "Private profile unavailable")?;
            let mut private = windows::core::BOOL(0);
            profile
                .IsInPrivateModeEnabled(&mut private)
                .map_err(|_| "Could not verify private profile")?;
            if !private.as_bool() {
                return Err("Sign-in profile is not private".into());
            }
        }
        webview
            .Navigate(PCWSTR(
                wide(if smoke { "about:blank" } else { url }).as_ptr(),
            ))
            .map_err(|_| "Could not open YouTube sign-in")?;
        let pending = Rc::new(Cell::new(false));
        let mut checked = Instant::now() - Duration::from_secs(1);
        let result = loop {
            if let Some(cookie) = captured.borrow_mut().take() {
                break Ok(cookie);
            }
            if let Err(error) = pump(hwnd, cancel, started) {
                break Err(error);
            }
            if checked.elapsed() >= Duration::from_millis(750) && !pending.get() {
                checked = Instant::now();
                pending.set(true);
                let uri = text(|out| webview.Source(out)).unwrap_or_default();
                let output = captured.clone();
                let busy = pending.clone();
                if cookies
                    .GetCookies(
                        PCWSTR(wide("https://music.youtube.com/").as_ptr()),
                        &GetCookiesCompletedHandler::create(Box::new(move |result, list| {
                            busy.set(false);
                            if result.is_ok() {
                                if let Some(list) = list {
                                    let mut count = 0;
                                    list.Count(&mut count)?;
                                    let mut header = Vec::new();
                                    let mut names = Vec::new();
                                    for i in 0..count.min(256) {
                                        let cookie = list.GetValueAtIndex(i)?;
                                        let name = text(|out| cookie.Name(out))?;
                                        let domain = text(|out| cookie.Domain(out))?;
                                        let value = text(|out| cookie.Value(out))?;
                                        let mut secure = windows::core::BOOL(0);
                                        cookie.IsSecure(&mut secure)?;
                                        if valid_youtube_cookie(
                                            &name,
                                            &domain,
                                            secure.as_bool(),
                                            &value,
                                        ) {
                                            names.push(name.clone());
                                            header.push(format!("{name}={value}"));
                                        }
                                    }
                                    let names: Vec<_> = names.iter().map(String::as_str).collect();
                                    if capture_ready(&uri, &names) {
                                        *output.borrow_mut() = Some(header.join("; "));
                                    }
                                }
                            }
                            Ok(())
                        })),
                    )
                    .is_err()
                {
                    pending.set(false);
                }
            }
        };
        let _ = webview.remove_PermissionRequested(permission_token);
        let _ = downloads.remove_DownloadStarting(download_token);
        let _ = webview.remove_NavigationStarting(nav_token);
        let _ = webview.remove_NewWindowRequested(popup_token);
        result
    }
}
