//! Compatibility names are shared with the native catalog; only Spotify endpoints are called.
use super::api::{Client, Error};
use serde_json::{json, Value};
use std::sync::Mutex;
static CLIENT: Mutex<Option<Client>> = Mutex::new(None);

fn arg(args: &[Value], index: usize) -> Result<&str, String> {
    args.get(index)
        .and_then(Value::as_str)
        .ok_or_else(|| "Missing Spotify request argument".into())
}
fn id<'a>(args: &'a [Value], index: usize, kind: &str) -> Result<&'a str, String> {
    let value = arg(args, index)?;
    let prefix = format!("spotify:{kind}:");
    let id = value
        .strip_prefix(&prefix)
        .ok_or("Select a Spotify item of the correct type")?;
    if id.len() != 22 || !id.bytes().all(|b| b.is_ascii_alphanumeric()) {
        return Err("Invalid Spotify item identifier".into());
    }
    Ok(id)
}
fn raw(client: &mut Client, path: &str) -> Result<Value, String> {
    get(client, &format!("https://api.spotify.com/v1/{path}"))
}
fn get(client: &mut Client, url: &str) -> Result<Value, String> {
    if super::web::has_session() {
        super::web::rest_get(url)
    } else {
        client.get(url).map_err(|error| error.to_string())
    }
}
fn unwrapped(mut value: Value, wrapper: &str) -> Value {
    if let Some(items) = value["items"].as_array_mut() {
        for item in items {
            if let Some(entity) = item
                .get(wrapper)
                .cloned()
                .or_else(|| item.get("track").cloned())
            {
                let mut entity = entity;
                if let Some(object) = entity.as_object_mut() {
                    for key in ["played_at", "added_at", "added_by"] {
                        if let Some(value) = item.get(key) {
                            object.insert(key.into(), value.clone());
                        }
                    }
                }
                *item = entity;
            }
        }
    }
    value
}
pub(crate) fn invoke(name: &str, args: &[Value]) -> Result<Value, String> {
    if name == "DZQueueJSON" && super::web::has_session() {
        let state = super::web_devices::snapshot()?;
        return Ok(
            json!({"items":state.as_ref().and_then(|s|s["next_tracks"].as_array()).cloned().unwrap_or_default(),"next":null}),
        );
    }
    if name == "DZAccountJSON" && super::web::has_session() {
        return super::web::account();
    }
    let web = super::web_catalog::invoke(name, args);
    if let Some(Ok(value)) = &web {
        return Ok(value.clone());
    }
    // Do not replay an indeterminate authenticated mutation through another transport.
    if super::web::has_session() && super::web_catalog::is_mutation(name) {
        if let Some(result) = web {
            return result;
        }
    }
    if super::web::has_session() {
        if let Some(result) = web {
            return result;
        }
    }
    public_invoke(name, args).or_else(|public_error| match web {
        Some(Err(web_error)) => Err(web_error),
        _ => Err(public_error),
    })
}
fn public_invoke(name: &str, args: &[Value]) -> Result<Value, String> {
    let mut slot = CLIENT
        .lock()
        .map_err(|_| "Spotify catalog worker unavailable")?;
    let client = slot.get_or_insert_with(Client::saved);
    match name {
        "DZAccountJSON" => client.profile().map_err(|e|e.to_string()),
        "DZFavoritesJSON" => raw(client,"me/tracks?limit=50").map(|v|unwrapped(v,"track")),
        "DZFavoriteAlbumsJSON" => raw(client,"me/albums?limit=50").map(|v|unwrapped(v,"album")),
        "DZPlaylistsJSON" => raw(client,"me/playlists?limit=50"),
        "DZHistoryRecentJSON" => raw(client,"me/player/recently-played?limit=50").map(|v|unwrapped(v,"track")),
        "DZPodcastShelfJSON" => if args.first().and_then(Value::as_u64)==Some(3) {raw(client,"me/episodes?limit=50").map(|v|unwrapped(v,"episode"))} else {raw(client,"me/shows?limit=50").map(|v|unwrapped(v,"show"))},
        "DZTrackMetadataJSON" => raw(client,&format!("tracks/{}",id(args,0,"track")?)),
        "DZArtistProfileJSON" => raw(client,&format!("artists/{}",id(args,0,"artist")?)),
        "DZArtistSectionJSON" if args.get(1).and_then(Value::as_u64)==Some(0) => raw(client,&format!("artists/{}/albums?limit=50",id(args,0,"artist")?)),
        "DZAlbumTracksJSON" => raw(client,&format!("albums/{}/tracks?limit=50",id(args,0,"album")?)),
        "DZPlaylistTracksJSON" => raw(client,&format!("playlists/{}/items?limit=50",id(args,0,"playlist")?)).map(|v|unwrapped(v,"item")),
        "DZPodcastEpisodesJSON" => raw(client,&format!("shows/{}/episodes?limit=50",id(args,0,"show")?)),
        "SPCatalogItemJSON" => {
            let kind=arg(args,0)?;
            if !matches!(kind,"track"|"album"|"artist"|"playlist"|"show"|"episode") {return Err(Error::InvalidRequest.to_string());}
            raw(client,&format!("{kind}s/{}",id(args,1,kind)?))
        }
        "SPCatalogNextJSON" => {
            let mut value=get(client,arg(args,0)?)?;
            for wrapper in ["track","album","show","episode","item"] { value=unwrapped(value,wrapper); }
            Ok(value)
        }
        "DZSearchJSON" | "DZSearchPodcastsJSON" => {
            let query=arg(args,0)?.trim();
            if query.is_empty() || query.len()>1000 {return Err(Error::InvalidRequest.to_string());}
            let mut url=url::Url::parse("https://api.spotify.com/v1/search").unwrap();
            url.query_pairs_mut().append_pair("q",query).append_pair("type",if name=="DZSearchPodcastsJSON" {"show,episode"} else {"track,album,artist,playlist,show,episode"}).append_pair("limit","10");
            get(client,url.as_str())
        }
        "DZHomeModuleJSON" => match args.first().and_then(Value::as_u64) {
            Some(0)=>raw(client,"me/player/recently-played?limit=50").map(|v|unwrapped(v,"track")),
            Some(1)=>raw(client,"me/tracks?limit=50").map(|v|unwrapped(v,"track")),
            Some(2)=>raw(client,"me/playlists?limit=50"),
            Some(3)|Some(8)=>raw(client,"me/albums?limit=50").map(|v|unwrapped(v,"album")),
            Some(4)=>raw(client,"me/following?type=artist&limit=50").map(|v|v["artists"].clone()),
            Some(5)=>raw(client,"me/top/tracks?limit=50"),
            Some(6)=>raw(client,"me/top/artists?limit=50"),
            Some(7)=>raw(client,"me/shows?limit=50").map(|v|unwrapped(v,"show")),
            Some(9)=>raw(client,"me/episodes?limit=50").map(|v|unwrapped(v,"episode")),
            _=>Err("No Spotify home shelf for this index".into()),
        },
        "DZQueueJSON" => {
            let value=raw(client,"me/player/queue")?;
            Ok(json!({"items":value["queue"],"next":null}))
        }
        "RustPodcastLibraryJSON" if args.is_empty() => raw(client,"me/shows?limit=50").map(|value|json!({"podcasts":unwrapped(value,"show")["items"]})),
        "DZAlbumLibraryJSON" | "DZPlaylistLibraryJSON" | "RustPodcastLibraryJSON" => {
            let kind=match name {"DZAlbumLibraryJSON"=>"album","DZPlaylistLibraryJSON"=>"playlist",_=>"show"};
            id(args,0,kind)?;
            let save=match args.get(1) {Some(Value::Bool(value))=>*value,Some(Value::Number(value))=>value.as_i64().ok_or("Invalid library action")?!=0,_=>return Err("Missing library add/remove action".into())};
            client.save_library(&[arg(args,0)?.to_owned()],save).map(|_|json!(1)).map_err(|e|e.to_string())
        }
        "DZAddFavorite" | "DZRemoveFavorite" => {
            let uri=arg(args,0)?.to_owned();
            client.save_library(&[uri],name=="DZAddFavorite").map(|_|json!(1)).map_err(|e|e.to_string())
        }
        "DZAddToPlaylist" | "DZRemoveFromPlaylist" => {
            let playlist=id(args,0,"playlist")?;
            let uri=arg(args,1)?.to_owned();
            if name=="DZAddToPlaylist" {client.add_playlist_items(playlist,&[uri])} else {client.remove_playlist_items(playlist,&[uri],None)}.map_err(|e|e.to_string())
        }
        "DZCreatePlaylist" => client.create_playlist(arg(args,0)?,false).map(|mut v| {
            let uri = v["uri"].as_str().map(str::to_owned).unwrap_or_else(|| format!("spotify:playlist:{}",v["id"].as_str().unwrap_or_default()));
            v["id"] = json!(uri); v
        }).map_err(|e|e.to_string()),
        "DZRenamePlaylist" | "DZUpdatePlaylistJSON" => client.update_playlist(id(args,0,"playlist")?,arg(args,1)?,if name=="DZUpdatePlaylistJSON" {Some(arg(args,2)?)} else {None}).map(|_|json!(1)).map_err(|e|e.to_string()),
        "DZDeletePlaylist" => client.save_library(&[arg(args,0)?.to_owned()],false).map(|_|json!(1)).map_err(|e|e.to_string()),
        "DZPlaylistCoverJSON" => client.upload_playlist_cover(id(args,0,"playlist")?,std::path::Path::new(arg(args,1)?)).map(|_|json!(1)).map_err(|e|e.to_string()),
        "SPDevicesJSON" if super::web::has_session() => super::web_devices::devices(),
        "SPDevicesJSON" => client.devices().and_then(|d|serde_json::to_value(d).map_err(|_|Error::InvalidResponse)).map_err(|e|e.to_string()),
        "SPTransferPlayback" | "SPTransferPlaybackJSON" if super::web::has_session() => {
            super::web_devices::transfer(arg(args,0)?, args.get(1).and_then(Value::as_bool).unwrap_or(false))
        }
        "SPTransferPlayback" | "SPTransferPlaybackJSON" => {
            let device = arg(args,0)?;
            client.transfer_playback(device,args.get(1).and_then(Value::as_bool).unwrap_or(false)).map_err(|e|e.to_string())?;
            super::remote::select_device(Some(device.into()));
            Ok(Value::Null)
        },
        "SPCapabilitiesJSON" => Ok(json!(super::capabilities::CAPABILITIES)),
        "DZFlowJSON" | "DZFlowForConfigJSON" | "DZTrackMixJSON" | "DZArtistMixJSON" | "SPStyleMixJSON" | "StyleMixGenres" => Err("Spotify Web API does not expose Flow, constrained personal mixes or radio generation for this integration".into()),
        "SPCatalogDiscoveryJSON" => {
            let mut sections=Vec::new();
            for (title,query,kind) in [
                ("Playlist search: moods","mood","playlist"),
                ("Playlist search: genres","genre","playlist"),
                ("Podcast search: music","music","show"),
            ] {
                let mut url=url::Url::parse("https://api.spotify.com/v1/search").unwrap();
                url.query_pairs_mut().append_pair("q",query).append_pair("type",kind).append_pair("limit","10");
                match get(client,url.as_str()) {
                    Ok(value)=>sections.push(json!({"title":title,"items":value[format!("{kind}s")],"error":""})),
                    Err(error)=>sections.push(json!({"title":title,"items":[],"error":error.to_string()})),
                }
            }
            Ok(json!({"sections":sections}))
        }
        "SPCatalogChannelJSON" => Err("Spotify browse categories are unavailable for this integration; use Search".into()),
        _ => Err(format!("Spotify does not expose the requested operation ({name})")),
    }
}
pub(super) fn reset() {
    if let Ok(mut slot) = CLIENT.lock() {
        *slot = None;
    }
}
