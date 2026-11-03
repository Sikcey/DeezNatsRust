// C ABI bindings for the DeezNats playback engine.
// These signatures are the ABI, not guessed from function names.
use super::Engine;
use serde_json::Value;
use std::ffi::{c_char, CString};
pub const EXPORTS: &[&str] = &[
    "DZSetSongDiskBuffering",
    "DZSetClientInfo",
    "DZNowPlayingJSON",
    "DZDiscoverDevices",
    "DZConnectDevice",
    "DZDisconnectDevice",
    "DZConnectedDevice",
    "DZLoginErrorKind",
    "DZLastErrorJSON",
    "DZSetQuality",
    "DZQuality",
    "DZHighQuality",
    "DZFormat",
    "DZUserID",
    "DZFavoritesJSON",
    "DZFavoriteIDsJSON",
    "DZFavoriteAlbumsJSON",
    "DZAlbumLibraryJSON",
    "DZPlaylistsJSON",
    "DZPlaylistTracksJSON",
    "DZPlaylistTracksStrictJSON",
    "DZCopyPlaylistTracksJSON",
    "DZAlbumTracksJSON",
    "DZSearchJSON",
    "DZPlay",
    "DZPause",
    "DZResume",
    "DZTogglePause",
    "DZStop",
    "DZSeek",
    "DZSetRepeat",
    "DZSetShuffle",
    "DZGetRepeat",
    "DZGetShuffle",
    "DZState",
    "DZPositionMS",
    "DZDurationMS",
    "DZSetVolume",
    "DZVolume",
    "DZFinishedCount",
    "DZDownloadTrack",
    "DZDownloadTrackAtQuality",
    "DZDownloadAlbum",
    "DZDownloadPlaylist",
    "DZDownloadDir",
    "DZSetDownloadDir",
    "DZMediaCacheMB",
    "DZSetMediaCacheMB",
    "DZIsPreview",
    "DZSetAdsDisabled",
    "DZAdsDisabled",
    "DZAccountJSON",
    "DZChartsJSON",
    "DZHomeJSON",
    "DZCheckUpdateJSON",
    "DZArtistTopJSON",
    "DZArtistProfileJSON",
    "DZLyricsJSON",
    "DZSetReplayGain",
    "DZReplayGain",
    "DZAddFavorite",
    "DZRemoveFavorite",
    "DZAddToPlaylist",
    "DZRemoveFromPlaylist",
    "DZCreatePlaylist",
    "DZRenamePlaylist",
    "DZUpdatePlaylistJSON",
    "DZPlaylistCoverJSON",
    "DZArtistSectionJSON",
    "DZPlaylistAccessJSON",
    "DZPlaylistLibraryJSON",
    "DZDeletePlaylist",
    "DZFlowJSON",
    "DZTrackMixJSON",
    "DZArtistMixJSON",
    "DZHistoryRecentJSON",
    "DZHistoryStatsJSON",
    "DZSearchPodcastsJSON",
    "DZPodcastEpisodesJSON",
    "DZAudioDevicesJSON",
    "DZSetAudioDevice",
    "DZCurrentAudioDevice",
    "DZSetGapless",
    "DZGapless",
    "DZSetCrossfadeMS",
    "DZCrossfadeMS",
    "DZSetSleepTimer",
    "DZCancelSleepTimer",
    "DZSleepTimerActive",
    "DZSleepTimerEndOfTrack",
    "DZSleepTimerRemainingMS",
    "DZPreload",
    "DZPlayEpisode",
    "DZEQJSON",
    "DZSetEQJSON",
    "DZClearPreload",
    "DZQueueSet",
    "DZQueueSetIndex",
    "DZQueueIndex",
    "DZQueueVersion",
    "DZQueueJSON",
    "DZQueueRemove",
    "DZQueueMove",
    "DZQueueInsertNext",
    "DZDownloadForOffline",
    "DZDownloadForOfflineAtQuality",
    "DZRemoveOfflineTrack",
    "DZFlowConfigsJSON",
    "DZFlowForConfigJSON",
    "DZFlowAppendJSON",
    "DZFlowTunerJSON",
    "DZSaveFlowTunerJSON",
    "DZHomeModulesJSON",
    "DZSmartMixJSON",
    "DZTrackMetadataJSON",
    "DZHomeModuleJSON",
    "DZDiscoveryPageJSON",
    "DZDiscoverySectionJSON",
    "DZLogout",
    "DZDownloadEpisodeJSON",
    "DZRemoveEpisodeJSON",
    "DZPodcastShelfJSON",
    "DZPodcastHomeJSON",
    "DZControlConfigJSON",
    "DZSetControlConfig",
    "DZWebRemoteSetEnabled",
    "DZWebRemoteInfoJSON",
];
impl Engine {
    pub fn invoke(&self, name: &str, args: &[Value]) -> Result<Value, String> {
        unsafe {
            match name {
                "DZSetSongDiskBuffering" => {
                    if args.len() != 1 {
                        return Err("DZSetSongDiskBuffering: wrong argument count".into());
                    }
                    let a0 = args
                        .first()
                        .and_then(Value::as_i64)
                        .ok_or("DZSetSongDiskBuffering argument 0 must be integer")?
                        as i32;
                    let function = self
                        .library
                        .get::<unsafe extern "C" fn(i32) -> ()>(b"DZSetSongDiskBuffering\0")
                        .map_err(|e| e.to_string())?;
                    function(a0);
                    Ok(Value::Null)
                }
                "DZSetClientInfo" => {
                    if args.len() != 2 {
                        return Err("DZSetClientInfo: wrong argument count".into());
                    }
                    let a0 = CString::new(
                        args.first()
                            .and_then(Value::as_str)
                            .ok_or("DZSetClientInfo argument 0 must be text")?,
                    )
                    .map_err(|_| "Text contains NUL")?;
                    let a1 = CString::new(
                        args.get(1)
                            .and_then(Value::as_str)
                            .ok_or("DZSetClientInfo argument 1 must be text")?,
                    )
                    .map_err(|_| "Text contains NUL")?;
                    let function = self
                        .library
                        .get::<unsafe extern "C" fn(*const c_char, *const c_char) -> ()>(
                            b"DZSetClientInfo\0",
                        )
                        .map_err(|e| e.to_string())?;
                    function(a0.as_ptr(), a1.as_ptr());
                    Ok(Value::Null)
                }
                "DZNowPlayingJSON" => {
                    if !args.is_empty() {
                        return Err("DZNowPlayingJSON: wrong argument count".into());
                    }
                    let function = self
                        .library
                        .get::<unsafe extern "C" fn() -> *mut c_char>(b"DZNowPlayingJSON\0")
                        .map_err(|e| e.to_string())?;
                    self.take(function())
                }
                "DZDiscoverDevices" => {
                    if args.len() != 1 {
                        return Err("DZDiscoverDevices: wrong argument count".into());
                    }
                    let a0 = args
                        .first()
                        .and_then(Value::as_i64)
                        .ok_or("DZDiscoverDevices argument 0 must be integer")?
                        as i32;
                    let function = self
                        .library
                        .get::<unsafe extern "C" fn(i32) -> *mut c_char>(b"DZDiscoverDevices\0")
                        .map_err(|e| e.to_string())?;
                    self.take(function(a0))
                }
                "DZConnectDevice" => {
                    if args.len() != 1 {
                        return Err("DZConnectDevice: wrong argument count".into());
                    }
                    let a0 = CString::new(
                        args.first()
                            .and_then(Value::as_str)
                            .ok_or("DZConnectDevice argument 0 must be text")?,
                    )
                    .map_err(|_| "Text contains NUL")?;
                    let function = self
                        .library
                        .get::<unsafe extern "C" fn(*const c_char) -> i32>(b"DZConnectDevice\0")
                        .map_err(|e| e.to_string())?;
                    Ok(serde_json::json!(function(a0.as_ptr())))
                }
                "DZDisconnectDevice" => {
                    if !args.is_empty() {
                        return Err("DZDisconnectDevice: wrong argument count".into());
                    }
                    let function = self
                        .library
                        .get::<unsafe extern "C" fn() -> ()>(b"DZDisconnectDevice\0")
                        .map_err(|e| e.to_string())?;
                    function();
                    Ok(Value::Null)
                }
                "DZConnectedDevice" => {
                    if !args.is_empty() {
                        return Err("DZConnectedDevice: wrong argument count".into());
                    }
                    let function = self
                        .library
                        .get::<unsafe extern "C" fn() -> *mut c_char>(b"DZConnectedDevice\0")
                        .map_err(|e| e.to_string())?;
                    self.take(function())
                }
                "DZLoginErrorKind" => {
                    if !args.is_empty() {
                        return Err("DZLoginErrorKind: wrong argument count".into());
                    }
                    let function = self
                        .library
                        .get::<unsafe extern "C" fn() -> i32>(b"DZLoginErrorKind\0")
                        .map_err(|e| e.to_string())?;
                    Ok(serde_json::json!(function()))
                }
                "DZLastErrorJSON" => {
                    if !args.is_empty() {
                        return Err("DZLastErrorJSON: wrong argument count".into());
                    }
                    let function = self
                        .library
                        .get::<unsafe extern "C" fn() -> *mut c_char>(b"DZLastErrorJSON\0")
                        .map_err(|e| e.to_string())?;
                    self.take(function())
                }
                "DZSetQuality" => {
                    if args.len() != 1 {
                        return Err("DZSetQuality: wrong argument count".into());
                    }
                    let a0 = args
                        .first()
                        .and_then(Value::as_i64)
                        .ok_or("DZSetQuality argument 0 must be integer")?
                        as i32;
                    let function = self
                        .library
                        .get::<unsafe extern "C" fn(i32) -> ()>(b"DZSetQuality\0")
                        .map_err(|e| e.to_string())?;
                    function(a0);
                    Ok(Value::Null)
                }
                "DZQuality" => {
                    if !args.is_empty() {
                        return Err("DZQuality: wrong argument count".into());
                    }
                    let function = self
                        .library
                        .get::<unsafe extern "C" fn() -> i32>(b"DZQuality\0")
                        .map_err(|e| e.to_string())?;
                    Ok(serde_json::json!(function()))
                }
                "DZHighQuality" => {
                    if !args.is_empty() {
                        return Err("DZHighQuality: wrong argument count".into());
                    }
                    let function = self
                        .library
                        .get::<unsafe extern "C" fn() -> i32>(b"DZHighQuality\0")
                        .map_err(|e| e.to_string())?;
                    Ok(serde_json::json!(function()))
                }
                "DZFormat" => {
                    if !args.is_empty() {
                        return Err("DZFormat: wrong argument count".into());
                    }
                    let function = self
                        .library
                        .get::<unsafe extern "C" fn() -> *mut c_char>(b"DZFormat\0")
                        .map_err(|e| e.to_string())?;
                    self.take(function())
                }
                "DZUserID" => {
                    if !args.is_empty() {
                        return Err("DZUserID: wrong argument count".into());
                    }
                    let function = self
                        .library
                        .get::<unsafe extern "C" fn() -> *mut c_char>(b"DZUserID\0")
                        .map_err(|e| e.to_string())?;
                    self.take(function())
                }
                "DZFavoritesJSON" => {
                    if !args.is_empty() {
                        return Err("DZFavoritesJSON: wrong argument count".into());
                    }
                    let function = self
                        .library
                        .get::<unsafe extern "C" fn() -> *mut c_char>(b"DZFavoritesJSON\0")
                        .map_err(|e| e.to_string())?;
                    self.take(function())
                }
                "DZFavoriteIDsJSON" => {
                    if !args.is_empty() {
                        return Err("DZFavoriteIDsJSON: wrong argument count".into());
                    }
                    let function = self
                        .library
                        .get::<unsafe extern "C" fn() -> *mut c_char>(b"DZFavoriteIDsJSON\0")
                        .map_err(|e| e.to_string())?;
                    self.take(function())
                }
                "DZFavoriteAlbumsJSON" => {
                    if !args.is_empty() {
                        return Err("DZFavoriteAlbumsJSON: wrong argument count".into());
                    }
                    let function = self
                        .library
                        .get::<unsafe extern "C" fn() -> *mut c_char>(b"DZFavoriteAlbumsJSON\0")
                        .map_err(|e| e.to_string())?;
                    self.take(function())
                }
                "DZAlbumLibraryJSON" => {
                    if args.len() != 2 {
                        return Err("DZAlbumLibraryJSON: wrong argument count".into());
                    }
                    let a0 = CString::new(
                        args.first()
                            .and_then(Value::as_str)
                            .ok_or("DZAlbumLibraryJSON argument 0 must be text")?,
                    )
                    .map_err(|_| "Text contains NUL")?;
                    let a1 = args
                        .get(1)
                        .and_then(Value::as_i64)
                        .ok_or("DZAlbumLibraryJSON argument 1 must be integer")?
                        as i32;
                    let function = self
                        .library
                        .get::<unsafe extern "C" fn(*const c_char, i32) -> *mut c_char>(
                            b"DZAlbumLibraryJSON\0",
                        )
                        .map_err(|e| e.to_string())?;
                    self.take(function(a0.as_ptr(), a1))
                }
                "DZPlaylistsJSON" => {
                    if !args.is_empty() {
                        return Err("DZPlaylistsJSON: wrong argument count".into());
                    }
                    let function = self
                        .library
                        .get::<unsafe extern "C" fn() -> *mut c_char>(b"DZPlaylistsJSON\0")
                        .map_err(|e| e.to_string())?;
                    self.take(function())
                }
                "DZCopyPlaylistTracksJSON" => {
                    if args.len() != 2 {
                        return Err("DZCopyPlaylistTracksJSON: wrong argument count".into());
                    }
                    let playlist =
                        CString::new(args[0].as_str().ok_or("Playlist ID must be text")?)
                            .map_err(|_| "Text contains NUL")?;
                    let tracks =
                        CString::new(args[1].as_str().ok_or("Track list must be JSON text")?)
                            .map_err(|_| "Text contains NUL")?;
                    let function = self
                        .library
                        .get::<unsafe extern "C" fn(*const c_char, *const c_char) -> *mut c_char>(
                            b"DZCopyPlaylistTracksJSON\0",
                        )
                        .map_err(|e| e.to_string())?;
                    self.take(function(playlist.as_ptr(), tracks.as_ptr()))
                }
                "DZPlaylistTracksJSON" | "DZPlaylistTracksStrictJSON" => {
                    if args.len() != 1 {
                        return Err("DZPlaylistTracksJSON: wrong argument count".into());
                    }
                    let a0 = CString::new(
                        args.first()
                            .and_then(Value::as_str)
                            .ok_or("DZPlaylistTracksJSON argument 0 must be text")?,
                    )
                    .map_err(|_| "Text contains NUL")?;
                    let function = self
                        .library
                        .get::<unsafe extern "C" fn(*const c_char) -> *mut c_char>(
                            if name == "DZPlaylistTracksStrictJSON" {
                                b"DZPlaylistTracksStrictJSON\0"
                            } else {
                                b"DZPlaylistTracksJSON\0"
                            },
                        )
                        .map_err(|e| e.to_string())?;
                    self.take(function(a0.as_ptr()))
                }
                "DZAlbumTracksJSON" => {
                    if args.len() != 1 {
                        return Err("DZAlbumTracksJSON: wrong argument count".into());
                    }
                    let a0 = CString::new(
                        args.first()
                            .and_then(Value::as_str)
                            .ok_or("DZAlbumTracksJSON argument 0 must be text")?,
                    )
                    .map_err(|_| "Text contains NUL")?;
                    let function = self
                        .library
                        .get::<unsafe extern "C" fn(*const c_char) -> *mut c_char>(
                            b"DZAlbumTracksJSON\0",
                        )
                        .map_err(|e| e.to_string())?;
                    self.take(function(a0.as_ptr()))
                }
                "DZSearchJSON" => {
                    if args.len() != 1 {
                        return Err("DZSearchJSON: wrong argument count".into());
                    }
                    let a0 = CString::new(
                        args.first()
                            .and_then(Value::as_str)
                            .ok_or("DZSearchJSON argument 0 must be text")?,
                    )
                    .map_err(|_| "Text contains NUL")?;
                    let function = self
                        .library
                        .get::<unsafe extern "C" fn(*const c_char) -> *mut c_char>(
                            b"DZSearchJSON\0",
                        )
                        .map_err(|e| e.to_string())?;
                    self.take(function(a0.as_ptr()))
                }
                "DZPlay" => {
                    if args.len() != 2 {
                        return Err("DZPlay: wrong argument count".into());
                    }
                    let a0 = CString::new(
                        args.first()
                            .and_then(Value::as_str)
                            .ok_or("DZPlay argument 0 must be text")?,
                    )
                    .map_err(|_| "Text contains NUL")?;
                    let a1 = args
                        .get(1)
                        .and_then(Value::as_i64)
                        .ok_or("DZPlay argument 1 must be integer")?;
                    let function = self
                        .library
                        .get::<unsafe extern "C" fn(*const c_char, i64) -> i32>(b"DZPlay\0")
                        .map_err(|e| e.to_string())?;
                    Ok(serde_json::json!(function(a0.as_ptr(), a1)))
                }
                "DZPause" => {
                    if !args.is_empty() {
                        return Err("DZPause: wrong argument count".into());
                    }
                    let function = self
                        .library
                        .get::<unsafe extern "C" fn() -> ()>(b"DZPause\0")
                        .map_err(|e| e.to_string())?;
                    function();
                    Ok(Value::Null)
                }
                "DZResume" => {
                    if !args.is_empty() {
                        return Err("DZResume: wrong argument count".into());
                    }
                    let function = self
                        .library
                        .get::<unsafe extern "C" fn() -> ()>(b"DZResume\0")
                        .map_err(|e| e.to_string())?;
                    function();
                    Ok(Value::Null)
                }
                "DZTogglePause" => {
                    if !args.is_empty() {
                        return Err("DZTogglePause: wrong argument count".into());
                    }
                    let function = self
                        .library
                        .get::<unsafe extern "C" fn() -> ()>(b"DZTogglePause\0")
                        .map_err(|e| e.to_string())?;
                    function();
                    Ok(Value::Null)
                }
                "DZStop" => {
                    if !args.is_empty() {
                        return Err("DZStop: wrong argument count".into());
                    }
                    let function = self
                        .library
                        .get::<unsafe extern "C" fn() -> ()>(b"DZStop\0")
                        .map_err(|e| e.to_string())?;
                    function();
                    Ok(Value::Null)
                }
                "DZSeek" => {
                    if args.len() != 1 {
                        return Err("DZSeek: wrong argument count".into());
                    }
                    let a0 = args
                        .first()
                        .and_then(Value::as_i64)
                        .ok_or("DZSeek argument 0 must be integer")?;
                    let function = self
                        .library
                        .get::<unsafe extern "C" fn(i64) -> ()>(b"DZSeek\0")
                        .map_err(|e| e.to_string())?;
                    function(a0);
                    Ok(Value::Null)
                }
                "DZSetRepeat" => {
                    if args.len() != 1 {
                        return Err("DZSetRepeat: wrong argument count".into());
                    }
                    let a0 = args
                        .first()
                        .and_then(Value::as_i64)
                        .ok_or("DZSetRepeat argument 0 must be integer")?
                        as i32;
                    let function = self
                        .library
                        .get::<unsafe extern "C" fn(i32) -> ()>(b"DZSetRepeat\0")
                        .map_err(|e| e.to_string())?;
                    function(a0);
                    Ok(Value::Null)
                }
                "DZSetShuffle" => {
                    if args.len() != 1 {
                        return Err("DZSetShuffle: wrong argument count".into());
                    }
                    let a0 = args
                        .first()
                        .and_then(Value::as_i64)
                        .ok_or("DZSetShuffle argument 0 must be integer")?
                        as i32;
                    let function = self
                        .library
                        .get::<unsafe extern "C" fn(i32) -> ()>(b"DZSetShuffle\0")
                        .map_err(|e| e.to_string())?;
                    function(a0);
                    Ok(Value::Null)
                }
                "DZGetRepeat" => {
                    if !args.is_empty() {
                        return Err("DZGetRepeat: wrong argument count".into());
                    }
                    let function = self
                        .library
                        .get::<unsafe extern "C" fn() -> *mut c_char>(b"DZGetRepeat\0")
                        .map_err(|e| e.to_string())?;
                    self.take(function())
                }
                "DZGetShuffle" => {
                    if !args.is_empty() {
                        return Err("DZGetShuffle: wrong argument count".into());
                    }
                    let function = self
                        .library
                        .get::<unsafe extern "C" fn() -> i32>(b"DZGetShuffle\0")
                        .map_err(|e| e.to_string())?;
                    Ok(serde_json::json!(function()))
                }
                "DZState" => {
                    if !args.is_empty() {
                        return Err("DZState: wrong argument count".into());
                    }
                    let function = self
                        .library
                        .get::<unsafe extern "C" fn() -> i32>(b"DZState\0")
                        .map_err(|e| e.to_string())?;
                    Ok(serde_json::json!(function()))
                }
                "DZPositionMS" => {
                    if !args.is_empty() {
                        return Err("DZPositionMS: wrong argument count".into());
                    }
                    let function = self
                        .library
                        .get::<unsafe extern "C" fn() -> i64>(b"DZPositionMS\0")
                        .map_err(|e| e.to_string())?;
                    Ok(serde_json::json!(function()))
                }
                "DZDurationMS" => {
                    if !args.is_empty() {
                        return Err("DZDurationMS: wrong argument count".into());
                    }
                    let function = self
                        .library
                        .get::<unsafe extern "C" fn() -> i64>(b"DZDurationMS\0")
                        .map_err(|e| e.to_string())?;
                    Ok(serde_json::json!(function()))
                }
                "DZSetVolume" => {
                    if args.len() != 1 {
                        return Err("DZSetVolume: wrong argument count".into());
                    }
                    let a0 = args
                        .first()
                        .and_then(Value::as_f64)
                        .ok_or("DZSetVolume argument 0 must be numeric")?;
                    let function = self
                        .library
                        .get::<unsafe extern "C" fn(f64) -> ()>(b"DZSetVolume\0")
                        .map_err(|e| e.to_string())?;
                    function(a0);
                    Ok(Value::Null)
                }
                "DZVolume" => {
                    if !args.is_empty() {
                        return Err("DZVolume: wrong argument count".into());
                    }
                    let function = self
                        .library
                        .get::<unsafe extern "C" fn() -> f64>(b"DZVolume\0")
                        .map_err(|e| e.to_string())?;
                    Ok(serde_json::json!(function()))
                }
                "DZFinishedCount" => {
                    if !args.is_empty() {
                        return Err("DZFinishedCount: wrong argument count".into());
                    }
                    let function = self
                        .library
                        .get::<unsafe extern "C" fn() -> i32>(b"DZFinishedCount\0")
                        .map_err(|e| e.to_string())?;
                    Ok(serde_json::json!(function()))
                }
                "DZDownloadTrack" => {
                    if args.len() != 2 {
                        return Err("DZDownloadTrack: wrong argument count".into());
                    }
                    let a0 = CString::new(
                        args.first()
                            .and_then(Value::as_str)
                            .ok_or("DZDownloadTrack argument 0 must be text")?,
                    )
                    .map_err(|_| "Text contains NUL")?;
                    let a1 = CString::new(
                        args.get(1)
                            .and_then(Value::as_str)
                            .ok_or("DZDownloadTrack argument 1 must be text")?,
                    )
                    .map_err(|_| "Text contains NUL")?;
                    let function = self
                        .library
                        .get::<unsafe extern "C" fn(*const c_char, *const c_char) -> *mut c_char>(
                            b"DZDownloadTrack\0",
                        )
                        .map_err(|e| e.to_string())?;
                    self.take(function(a0.as_ptr(), a1.as_ptr()))
                }
                "DZDownloadTrackAtQuality" => {
                    if args.len() != 3 {
                        return Err("DZDownloadTrackAtQuality: wrong argument count".into());
                    }
                    let a0 = CString::new(
                        args.first()
                            .and_then(Value::as_str)
                            .ok_or("DZDownloadTrackAtQuality argument 0 must be text")?,
                    )
                    .map_err(|_| "Text contains NUL")?;
                    let a1 = CString::new(
                        args.get(1)
                            .and_then(Value::as_str)
                            .ok_or("DZDownloadTrackAtQuality argument 1 must be text")?,
                    )
                    .map_err(|_| "Text contains NUL")?;
                    let a2 = args
                        .get(2)
                        .and_then(Value::as_i64)
                        .ok_or("DZDownloadTrackAtQuality argument 2 must be integer")?
                        as i32;
                    let function = self.library.get::<unsafe extern "C" fn(*const c_char, *const c_char, i32) -> *mut c_char>(b"DZDownloadTrackAtQuality\0").map_err(|e| e.to_string())?;
                    self.take(function(a0.as_ptr(), a1.as_ptr(), a2))
                }
                "DZDownloadAlbum" => {
                    if args.len() != 1 {
                        return Err("DZDownloadAlbum: wrong argument count".into());
                    }
                    let a0 = CString::new(
                        args.first()
                            .and_then(Value::as_str)
                            .ok_or("DZDownloadAlbum argument 0 must be text")?,
                    )
                    .map_err(|_| "Text contains NUL")?;
                    let function = self
                        .library
                        .get::<unsafe extern "C" fn(*const c_char) -> *mut c_char>(
                            b"DZDownloadAlbum\0",
                        )
                        .map_err(|e| e.to_string())?;
                    self.take(function(a0.as_ptr()))
                }
                "DZDownloadPlaylist" => {
                    if args.len() != 1 {
                        return Err("DZDownloadPlaylist: wrong argument count".into());
                    }
                    let a0 = CString::new(
                        args.first()
                            .and_then(Value::as_str)
                            .ok_or("DZDownloadPlaylist argument 0 must be text")?,
                    )
                    .map_err(|_| "Text contains NUL")?;
                    let function = self
                        .library
                        .get::<unsafe extern "C" fn(*const c_char) -> *mut c_char>(
                            b"DZDownloadPlaylist\0",
                        )
                        .map_err(|e| e.to_string())?;
                    self.take(function(a0.as_ptr()))
                }
                "DZDownloadDir" => {
                    if !args.is_empty() {
                        return Err("DZDownloadDir: wrong argument count".into());
                    }
                    let function = self
                        .library
                        .get::<unsafe extern "C" fn() -> *mut c_char>(b"DZDownloadDir\0")
                        .map_err(|e| e.to_string())?;
                    self.take(function())
                }
                "DZSetDownloadDir" => {
                    if args.len() != 1 {
                        return Err("DZSetDownloadDir: wrong argument count".into());
                    }
                    let a0 = CString::new(
                        args.first()
                            .and_then(Value::as_str)
                            .ok_or("DZSetDownloadDir argument 0 must be text")?,
                    )
                    .map_err(|_| "Text contains NUL")?;
                    let function = self
                        .library
                        .get::<unsafe extern "C" fn(*const c_char) -> i32>(b"DZSetDownloadDir\0")
                        .map_err(|e| e.to_string())?;
                    Ok(serde_json::json!(function(a0.as_ptr())))
                }
                "DZMediaCacheMB" => {
                    if !args.is_empty() {
                        return Err("DZMediaCacheMB: wrong argument count".into());
                    }
                    let function = self
                        .library
                        .get::<unsafe extern "C" fn() -> i32>(b"DZMediaCacheMB\0")
                        .map_err(|e| e.to_string())?;
                    Ok(serde_json::json!(function()))
                }
                "DZSetMediaCacheMB" => {
                    if args.len() != 1 {
                        return Err("DZSetMediaCacheMB: wrong argument count".into());
                    }
                    let a0 = args
                        .first()
                        .and_then(Value::as_i64)
                        .ok_or("DZSetMediaCacheMB argument 0 must be integer")?
                        as i32;
                    let function = self
                        .library
                        .get::<unsafe extern "C" fn(i32) -> i32>(b"DZSetMediaCacheMB\0")
                        .map_err(|e| e.to_string())?;
                    Ok(serde_json::json!(function(a0)))
                }
                "DZIsPreview" => {
                    if !args.is_empty() {
                        return Err("DZIsPreview: wrong argument count".into());
                    }
                    let function = self
                        .library
                        .get::<unsafe extern "C" fn() -> i32>(b"DZIsPreview\0")
                        .map_err(|e| e.to_string())?;
                    Ok(serde_json::json!(function()))
                }
                "DZSetAdsDisabled" => {
                    if args.len() != 1 {
                        return Err("DZSetAdsDisabled: wrong argument count".into());
                    }
                    let a0 = args
                        .first()
                        .and_then(Value::as_i64)
                        .ok_or("DZSetAdsDisabled argument 0 must be integer")?
                        as i32;
                    let function = self
                        .library
                        .get::<unsafe extern "C" fn(i32) -> i32>(b"DZSetAdsDisabled\0")
                        .map_err(|e| e.to_string())?;
                    Ok(serde_json::json!(function(a0)))
                }
                "DZAdsDisabled" => {
                    if !args.is_empty() {
                        return Err("DZAdsDisabled: wrong argument count".into());
                    }
                    let function = self
                        .library
                        .get::<unsafe extern "C" fn() -> i32>(b"DZAdsDisabled\0")
                        .map_err(|e| e.to_string())?;
                    Ok(serde_json::json!(function()))
                }
                "DZAccountJSON" => {
                    if !args.is_empty() {
                        return Err("DZAccountJSON: wrong argument count".into());
                    }
                    let function = self
                        .library
                        .get::<unsafe extern "C" fn() -> *mut c_char>(b"DZAccountJSON\0")
                        .map_err(|e| e.to_string())?;
                    self.take(function())
                }
                "DZChartsJSON" => {
                    if !args.is_empty() {
                        return Err("DZChartsJSON: wrong argument count".into());
                    }
                    let function = self
                        .library
                        .get::<unsafe extern "C" fn() -> *mut c_char>(b"DZChartsJSON\0")
                        .map_err(|e| e.to_string())?;
                    self.take(function())
                }
                "DZHomeJSON" => {
                    if !args.is_empty() {
                        return Err("DZHomeJSON: wrong argument count".into());
                    }
                    let function = self
                        .library
                        .get::<unsafe extern "C" fn() -> *mut c_char>(b"DZHomeJSON\0")
                        .map_err(|e| e.to_string())?;
                    self.take(function())
                }
                "DZCheckUpdateJSON" => {
                    if !args.is_empty() {
                        return Err("DZCheckUpdateJSON: wrong argument count".into());
                    }
                    let function = self
                        .library
                        .get::<unsafe extern "C" fn() -> *mut c_char>(b"DZCheckUpdateJSON\0")
                        .map_err(|e| e.to_string())?;
                    self.take(function())
                }
                "DZArtistTopJSON" => {
                    if args.len() != 1 {
                        return Err("DZArtistTopJSON: wrong argument count".into());
                    }
                    let a0 = CString::new(
                        args.first()
                            .and_then(Value::as_str)
                            .ok_or("DZArtistTopJSON argument 0 must be text")?,
                    )
                    .map_err(|_| "Text contains NUL")?;
                    let function = self
                        .library
                        .get::<unsafe extern "C" fn(*const c_char) -> *mut c_char>(
                            b"DZArtistTopJSON\0",
                        )
                        .map_err(|e| e.to_string())?;
                    self.take(function(a0.as_ptr()))
                }
                "DZArtistProfileJSON" => {
                    if args.len() != 1 {
                        return Err("DZArtistProfileJSON: wrong argument count".into());
                    }
                    let a0 = CString::new(
                        args.first()
                            .and_then(Value::as_str)
                            .ok_or("DZArtistProfileJSON argument 0 must be text")?,
                    )
                    .map_err(|_| "Text contains NUL")?;
                    let function = self
                        .library
                        .get::<unsafe extern "C" fn(*const c_char) -> *mut c_char>(
                            b"DZArtistProfileJSON\0",
                        )
                        .map_err(|e| e.to_string())?;
                    self.take(function(a0.as_ptr()))
                }
                "DZLyricsJSON" => {
                    if args.len() != 1 {
                        return Err("DZLyricsJSON: wrong argument count".into());
                    }
                    let a0 = CString::new(
                        args.first()
                            .and_then(Value::as_str)
                            .ok_or("DZLyricsJSON argument 0 must be text")?,
                    )
                    .map_err(|_| "Text contains NUL")?;
                    let function = self
                        .library
                        .get::<unsafe extern "C" fn(*const c_char) -> *mut c_char>(
                            b"DZLyricsJSON\0",
                        )
                        .map_err(|e| e.to_string())?;
                    self.take(function(a0.as_ptr()))
                }
                "DZSetReplayGain" => {
                    if args.len() != 1 {
                        return Err("DZSetReplayGain: wrong argument count".into());
                    }
                    let a0 = args
                        .first()
                        .and_then(Value::as_i64)
                        .ok_or("DZSetReplayGain argument 0 must be integer")?
                        as i32;
                    let function = self
                        .library
                        .get::<unsafe extern "C" fn(i32) -> ()>(b"DZSetReplayGain\0")
                        .map_err(|e| e.to_string())?;
                    function(a0);
                    Ok(Value::Null)
                }
                "DZReplayGain" => {
                    if !args.is_empty() {
                        return Err("DZReplayGain: wrong argument count".into());
                    }
                    let function = self
                        .library
                        .get::<unsafe extern "C" fn() -> i32>(b"DZReplayGain\0")
                        .map_err(|e| e.to_string())?;
                    Ok(serde_json::json!(function()))
                }
                "DZAddFavorite" => {
                    if args.len() != 1 {
                        return Err("DZAddFavorite: wrong argument count".into());
                    }
                    let a0 = CString::new(
                        args.first()
                            .and_then(Value::as_str)
                            .ok_or("DZAddFavorite argument 0 must be text")?,
                    )
                    .map_err(|_| "Text contains NUL")?;
                    let function = self
                        .library
                        .get::<unsafe extern "C" fn(*const c_char) -> i32>(b"DZAddFavorite\0")
                        .map_err(|e| e.to_string())?;
                    Ok(serde_json::json!(function(a0.as_ptr())))
                }
                "DZRemoveFavorite" => {
                    if args.len() != 1 {
                        return Err("DZRemoveFavorite: wrong argument count".into());
                    }
                    let a0 = CString::new(
                        args.first()
                            .and_then(Value::as_str)
                            .ok_or("DZRemoveFavorite argument 0 must be text")?,
                    )
                    .map_err(|_| "Text contains NUL")?;
                    let function = self
                        .library
                        .get::<unsafe extern "C" fn(*const c_char) -> i32>(b"DZRemoveFavorite\0")
                        .map_err(|e| e.to_string())?;
                    Ok(serde_json::json!(function(a0.as_ptr())))
                }
                "DZAddToPlaylist" => {
                    if args.len() != 2 {
                        return Err("DZAddToPlaylist: wrong argument count".into());
                    }
                    let a0 = CString::new(
                        args.first()
                            .and_then(Value::as_str)
                            .ok_or("DZAddToPlaylist argument 0 must be text")?,
                    )
                    .map_err(|_| "Text contains NUL")?;
                    let a1 = CString::new(
                        args.get(1)
                            .and_then(Value::as_str)
                            .ok_or("DZAddToPlaylist argument 1 must be text")?,
                    )
                    .map_err(|_| "Text contains NUL")?;
                    let function = self
                        .library
                        .get::<unsafe extern "C" fn(*const c_char, *const c_char) -> i32>(
                            b"DZAddToPlaylist\0",
                        )
                        .map_err(|e| e.to_string())?;
                    Ok(serde_json::json!(function(a0.as_ptr(), a1.as_ptr())))
                }
                "DZRemoveFromPlaylist" => {
                    if args.len() != 2 {
                        return Err("DZRemoveFromPlaylist: wrong argument count".into());
                    }
                    let a0 = CString::new(
                        args.first()
                            .and_then(Value::as_str)
                            .ok_or("DZRemoveFromPlaylist argument 0 must be text")?,
                    )
                    .map_err(|_| "Text contains NUL")?;
                    let a1 = CString::new(
                        args.get(1)
                            .and_then(Value::as_str)
                            .ok_or("DZRemoveFromPlaylist argument 1 must be text")?,
                    )
                    .map_err(|_| "Text contains NUL")?;
                    let function = self
                        .library
                        .get::<unsafe extern "C" fn(*const c_char, *const c_char) -> i32>(
                            b"DZRemoveFromPlaylist\0",
                        )
                        .map_err(|e| e.to_string())?;
                    Ok(serde_json::json!(function(a0.as_ptr(), a1.as_ptr())))
                }
                "DZCreatePlaylist" => {
                    if args.len() != 1 {
                        return Err("DZCreatePlaylist: wrong argument count".into());
                    }
                    let a0 = CString::new(
                        args.first()
                            .and_then(Value::as_str)
                            .ok_or("DZCreatePlaylist argument 0 must be text")?,
                    )
                    .map_err(|_| "Text contains NUL")?;
                    let function = self
                        .library
                        .get::<unsafe extern "C" fn(*const c_char) -> *mut c_char>(
                            b"DZCreatePlaylist\0",
                        )
                        .map_err(|e| e.to_string())?;
                    self.take(function(a0.as_ptr()))
                }
                "DZRenamePlaylist" => {
                    if args.len() != 2 {
                        return Err("DZRenamePlaylist: wrong argument count".into());
                    }
                    let a0 = CString::new(
                        args.first()
                            .and_then(Value::as_str)
                            .ok_or("DZRenamePlaylist argument 0 must be text")?,
                    )
                    .map_err(|_| "Text contains NUL")?;
                    let a1 = CString::new(
                        args.get(1)
                            .and_then(Value::as_str)
                            .ok_or("DZRenamePlaylist argument 1 must be text")?,
                    )
                    .map_err(|_| "Text contains NUL")?;
                    let function = self
                        .library
                        .get::<unsafe extern "C" fn(*const c_char, *const c_char) -> i32>(
                            b"DZRenamePlaylist\0",
                        )
                        .map_err(|e| e.to_string())?;
                    Ok(serde_json::json!(function(a0.as_ptr(), a1.as_ptr())))
                }
                "DZUpdatePlaylistJSON" => {
                    if args.len() != 3 {
                        return Err("DZUpdatePlaylistJSON: wrong argument count".into());
                    }
                    let a0 = CString::new(
                        args.first()
                            .and_then(Value::as_str)
                            .ok_or("DZUpdatePlaylistJSON argument 0 must be text")?,
                    )
                    .map_err(|_| "Text contains NUL")?;
                    let a1 = CString::new(
                        args.get(1)
                            .and_then(Value::as_str)
                            .ok_or("DZUpdatePlaylistJSON argument 1 must be text")?,
                    )
                    .map_err(|_| "Text contains NUL")?;
                    let a2 = CString::new(
                        args.get(2)
                            .and_then(Value::as_str)
                            .ok_or("DZUpdatePlaylistJSON argument 2 must be text")?,
                    )
                    .map_err(|_| "Text contains NUL")?;
                    let function = self
                        .library
                        .get::<unsafe extern "C" fn(
                            *const c_char,
                            *const c_char,
                            *const c_char,
                        ) -> *mut c_char>(b"DZUpdatePlaylistJSON\0")
                        .map_err(|e| e.to_string())?;
                    self.take(function(a0.as_ptr(), a1.as_ptr(), a2.as_ptr()))
                }
                "DZPlaylistCoverJSON" => {
                    if args.len() != 2 {
                        return Err("DZPlaylistCoverJSON: wrong argument count".into());
                    }
                    let a0 = CString::new(
                        args.first()
                            .and_then(Value::as_str)
                            .ok_or("DZPlaylistCoverJSON argument 0 must be text")?,
                    )
                    .map_err(|_| "Text contains NUL")?;
                    let a1 = CString::new(
                        args.get(1)
                            .and_then(Value::as_str)
                            .ok_or("DZPlaylistCoverJSON argument 1 must be text")?,
                    )
                    .map_err(|_| "Text contains NUL")?;
                    let function = self
                        .library
                        .get::<unsafe extern "C" fn(*const c_char, *const c_char) -> *mut c_char>(
                            b"DZPlaylistCoverJSON\0",
                        )
                        .map_err(|e| e.to_string())?;
                    self.take(function(a0.as_ptr(), a1.as_ptr()))
                }
                "DZArtistSectionJSON" => {
                    if args.len() != 3 {
                        return Err("DZArtistSectionJSON: wrong argument count".into());
                    }
                    let a0 = CString::new(
                        args.first()
                            .and_then(Value::as_str)
                            .ok_or("DZArtistSectionJSON argument 0 must be text")?,
                    )
                    .map_err(|_| "Text contains NUL")?;
                    let a1 = args
                        .get(1)
                        .and_then(Value::as_i64)
                        .ok_or("DZArtistSectionJSON argument 1 must be integer")?
                        as i32;
                    let a2 = CString::new(
                        args.get(2)
                            .and_then(Value::as_str)
                            .ok_or("DZArtistSectionJSON argument 2 must be text")?,
                    )
                    .map_err(|_| "Text contains NUL")?;
                    let function = self.library.get::<unsafe extern "C" fn(*const c_char, i32, *const c_char) -> *mut c_char>(b"DZArtistSectionJSON\0").map_err(|e| e.to_string())?;
                    self.take(function(a0.as_ptr(), a1, a2.as_ptr()))
                }
                "DZPlaylistAccessJSON" => {
                    if args.len() != 1 {
                        return Err("DZPlaylistAccessJSON: wrong argument count".into());
                    }
                    let a0 = CString::new(
                        args.first()
                            .and_then(Value::as_str)
                            .ok_or("DZPlaylistAccessJSON argument 0 must be text")?,
                    )
                    .map_err(|_| "Text contains NUL")?;
                    let function = self
                        .library
                        .get::<unsafe extern "C" fn(*const c_char) -> *mut c_char>(
                            b"DZPlaylistAccessJSON\0",
                        )
                        .map_err(|e| e.to_string())?;
                    self.take(function(a0.as_ptr()))
                }
                "DZPlaylistLibraryJSON" => {
                    if args.len() != 2 {
                        return Err("DZPlaylistLibraryJSON: wrong argument count".into());
                    }
                    let a0 = CString::new(
                        args.first()
                            .and_then(Value::as_str)
                            .ok_or("DZPlaylistLibraryJSON argument 0 must be text")?,
                    )
                    .map_err(|_| "Text contains NUL")?;
                    let a1 = args
                        .get(1)
                        .and_then(Value::as_i64)
                        .ok_or("DZPlaylistLibraryJSON argument 1 must be integer")?
                        as i32;
                    let function = self
                        .library
                        .get::<unsafe extern "C" fn(*const c_char, i32) -> *mut c_char>(
                            b"DZPlaylistLibraryJSON\0",
                        )
                        .map_err(|e| e.to_string())?;
                    self.take(function(a0.as_ptr(), a1))
                }
                "DZDeletePlaylist" => {
                    if args.len() != 1 {
                        return Err("DZDeletePlaylist: wrong argument count".into());
                    }
                    let a0 = CString::new(
                        args.first()
                            .and_then(Value::as_str)
                            .ok_or("DZDeletePlaylist argument 0 must be text")?,
                    )
                    .map_err(|_| "Text contains NUL")?;
                    let function = self
                        .library
                        .get::<unsafe extern "C" fn(*const c_char) -> i32>(b"DZDeletePlaylist\0")
                        .map_err(|e| e.to_string())?;
                    Ok(serde_json::json!(function(a0.as_ptr())))
                }
                "DZFlowJSON" => {
                    if !args.is_empty() {
                        return Err("DZFlowJSON: wrong argument count".into());
                    }
                    let function = self
                        .library
                        .get::<unsafe extern "C" fn() -> *mut c_char>(b"DZFlowJSON\0")
                        .map_err(|e| e.to_string())?;
                    self.take(function())
                }
                "DZTrackMixJSON" => {
                    if args.len() != 1 {
                        return Err("DZTrackMixJSON: wrong argument count".into());
                    }
                    let a0 = CString::new(
                        args.first()
                            .and_then(Value::as_str)
                            .ok_or("DZTrackMixJSON argument 0 must be text")?,
                    )
                    .map_err(|_| "Text contains NUL")?;
                    let function = self
                        .library
                        .get::<unsafe extern "C" fn(*const c_char) -> *mut c_char>(
                            b"DZTrackMixJSON\0",
                        )
                        .map_err(|e| e.to_string())?;
                    self.take(function(a0.as_ptr()))
                }
                "DZArtistMixJSON" => {
                    if args.len() != 1 {
                        return Err("DZArtistMixJSON: wrong argument count".into());
                    }
                    let a0 = CString::new(
                        args.first()
                            .and_then(Value::as_str)
                            .ok_or("DZArtistMixJSON argument 0 must be text")?,
                    )
                    .map_err(|_| "Text contains NUL")?;
                    let function = self
                        .library
                        .get::<unsafe extern "C" fn(*const c_char) -> *mut c_char>(
                            b"DZArtistMixJSON\0",
                        )
                        .map_err(|e| e.to_string())?;
                    self.take(function(a0.as_ptr()))
                }
                "DZHistoryRecentJSON" => {
                    if args.len() != 1 {
                        return Err("DZHistoryRecentJSON: wrong argument count".into());
                    }
                    let a0 = args
                        .first()
                        .and_then(Value::as_i64)
                        .ok_or("DZHistoryRecentJSON argument 0 must be integer")?
                        as i32;
                    let function = self
                        .library
                        .get::<unsafe extern "C" fn(i32) -> *mut c_char>(b"DZHistoryRecentJSON\0")
                        .map_err(|e| e.to_string())?;
                    self.take(function(a0))
                }
                "DZHistoryStatsJSON" => {
                    if args.len() != 1 {
                        return Err("DZHistoryStatsJSON: wrong argument count".into());
                    }
                    let a0 = args
                        .first()
                        .and_then(Value::as_i64)
                        .ok_or("DZHistoryStatsJSON argument 0 must be integer")?
                        as i32;
                    let function = self
                        .library
                        .get::<unsafe extern "C" fn(i32) -> *mut c_char>(b"DZHistoryStatsJSON\0")
                        .map_err(|e| e.to_string())?;
                    self.take(function(a0))
                }
                "DZSearchPodcastsJSON" => {
                    if args.len() != 1 {
                        return Err("DZSearchPodcastsJSON: wrong argument count".into());
                    }
                    let a0 = CString::new(
                        args.first()
                            .and_then(Value::as_str)
                            .ok_or("DZSearchPodcastsJSON argument 0 must be text")?,
                    )
                    .map_err(|_| "Text contains NUL")?;
                    let function = self
                        .library
                        .get::<unsafe extern "C" fn(*const c_char) -> *mut c_char>(
                            b"DZSearchPodcastsJSON\0",
                        )
                        .map_err(|e| e.to_string())?;
                    self.take(function(a0.as_ptr()))
                }
                "DZPodcastEpisodesJSON" => {
                    if args.len() != 1 {
                        return Err("DZPodcastEpisodesJSON: wrong argument count".into());
                    }
                    let a0 = CString::new(
                        args.first()
                            .and_then(Value::as_str)
                            .ok_or("DZPodcastEpisodesJSON argument 0 must be text")?,
                    )
                    .map_err(|_| "Text contains NUL")?;
                    let function = self
                        .library
                        .get::<unsafe extern "C" fn(*const c_char) -> *mut c_char>(
                            b"DZPodcastEpisodesJSON\0",
                        )
                        .map_err(|e| e.to_string())?;
                    self.take(function(a0.as_ptr()))
                }
                "DZAudioDevicesJSON" => {
                    if !args.is_empty() {
                        return Err("DZAudioDevicesJSON: wrong argument count".into());
                    }
                    let function = self
                        .library
                        .get::<unsafe extern "C" fn() -> *mut c_char>(b"DZAudioDevicesJSON\0")
                        .map_err(|e| e.to_string())?;
                    self.take(function())
                }
                "DZSetAudioDevice" => {
                    if args.len() != 1 {
                        return Err("DZSetAudioDevice: wrong argument count".into());
                    }
                    let a0 = CString::new(
                        args.first()
                            .and_then(Value::as_str)
                            .ok_or("DZSetAudioDevice argument 0 must be text")?,
                    )
                    .map_err(|_| "Text contains NUL")?;
                    let function = self
                        .library
                        .get::<unsafe extern "C" fn(*const c_char) -> i32>(b"DZSetAudioDevice\0")
                        .map_err(|e| e.to_string())?;
                    Ok(serde_json::json!(function(a0.as_ptr())))
                }
                "DZCurrentAudioDevice" => {
                    if !args.is_empty() {
                        return Err("DZCurrentAudioDevice: wrong argument count".into());
                    }
                    let function = self
                        .library
                        .get::<unsafe extern "C" fn() -> *mut c_char>(b"DZCurrentAudioDevice\0")
                        .map_err(|e| e.to_string())?;
                    self.take(function())
                }
                "DZSetGapless" => {
                    if args.len() != 1 {
                        return Err("DZSetGapless: wrong argument count".into());
                    }
                    let a0 = args
                        .first()
                        .and_then(Value::as_i64)
                        .ok_or("DZSetGapless argument 0 must be integer")?
                        as i32;
                    let function = self
                        .library
                        .get::<unsafe extern "C" fn(i32) -> ()>(b"DZSetGapless\0")
                        .map_err(|e| e.to_string())?;
                    function(a0);
                    Ok(Value::Null)
                }
                "DZGapless" => {
                    if !args.is_empty() {
                        return Err("DZGapless: wrong argument count".into());
                    }
                    let function = self
                        .library
                        .get::<unsafe extern "C" fn() -> i32>(b"DZGapless\0")
                        .map_err(|e| e.to_string())?;
                    Ok(serde_json::json!(function()))
                }
                "DZSetCrossfadeMS" => {
                    if args.len() != 1 {
                        return Err("DZSetCrossfadeMS: wrong argument count".into());
                    }
                    let a0 = args
                        .first()
                        .and_then(Value::as_i64)
                        .ok_or("DZSetCrossfadeMS argument 0 must be integer")?
                        as i32;
                    let function = self
                        .library
                        .get::<unsafe extern "C" fn(i32) -> ()>(b"DZSetCrossfadeMS\0")
                        .map_err(|e| e.to_string())?;
                    function(a0);
                    Ok(Value::Null)
                }
                "DZCrossfadeMS" => {
                    if !args.is_empty() {
                        return Err("DZCrossfadeMS: wrong argument count".into());
                    }
                    let function = self
                        .library
                        .get::<unsafe extern "C" fn() -> i32>(b"DZCrossfadeMS\0")
                        .map_err(|e| e.to_string())?;
                    Ok(serde_json::json!(function()))
                }
                "DZSetSleepTimer" => {
                    if args.len() != 2 {
                        return Err("DZSetSleepTimer: wrong argument count".into());
                    }
                    let a0 = args
                        .first()
                        .and_then(Value::as_i64)
                        .ok_or("DZSetSleepTimer argument 0 must be integer")?
                        as i32;
                    let a1 = args
                        .get(1)
                        .and_then(Value::as_i64)
                        .ok_or("DZSetSleepTimer argument 1 must be integer")?
                        as i32;
                    let function = self
                        .library
                        .get::<unsafe extern "C" fn(i32, i32) -> ()>(b"DZSetSleepTimer\0")
                        .map_err(|e| e.to_string())?;
                    function(a0, a1);
                    Ok(Value::Null)
                }
                "DZCancelSleepTimer" => {
                    if !args.is_empty() {
                        return Err("DZCancelSleepTimer: wrong argument count".into());
                    }
                    let function = self
                        .library
                        .get::<unsafe extern "C" fn() -> ()>(b"DZCancelSleepTimer\0")
                        .map_err(|e| e.to_string())?;
                    function();
                    Ok(Value::Null)
                }
                "DZSleepTimerActive" => {
                    if !args.is_empty() {
                        return Err("DZSleepTimerActive: wrong argument count".into());
                    }
                    let function = self
                        .library
                        .get::<unsafe extern "C" fn() -> i32>(b"DZSleepTimerActive\0")
                        .map_err(|e| e.to_string())?;
                    Ok(serde_json::json!(function()))
                }
                "DZSleepTimerEndOfTrack" => {
                    if !args.is_empty() {
                        return Err("DZSleepTimerEndOfTrack: wrong argument count".into());
                    }
                    let function = self
                        .library
                        .get::<unsafe extern "C" fn() -> i32>(b"DZSleepTimerEndOfTrack\0")
                        .map_err(|e| e.to_string())?;
                    Ok(serde_json::json!(function()))
                }
                "DZSleepTimerRemainingMS" => {
                    if !args.is_empty() {
                        return Err("DZSleepTimerRemainingMS: wrong argument count".into());
                    }
                    let function = self
                        .library
                        .get::<unsafe extern "C" fn() -> i64>(b"DZSleepTimerRemainingMS\0")
                        .map_err(|e| e.to_string())?;
                    Ok(serde_json::json!(function()))
                }
                "DZPreload" => {
                    if args.len() != 2 {
                        return Err("DZPreload: wrong argument count".into());
                    }
                    let a0 = CString::new(
                        args.first()
                            .and_then(Value::as_str)
                            .ok_or("DZPreload argument 0 must be text")?,
                    )
                    .map_err(|_| "Text contains NUL")?;
                    let a1 = args
                        .get(1)
                        .and_then(Value::as_i64)
                        .ok_or("DZPreload argument 1 must be integer")?;
                    let function = self
                        .library
                        .get::<unsafe extern "C" fn(*const c_char, i64) -> ()>(b"DZPreload\0")
                        .map_err(|e| e.to_string())?;
                    function(a0.as_ptr(), a1);
                    Ok(Value::Null)
                }
                "DZPlayEpisode" => {
                    if args.len() != 2 {
                        return Err("DZPlayEpisode: wrong argument count".into());
                    }
                    let a0 = CString::new(
                        args.first()
                            .and_then(Value::as_str)
                            .ok_or("DZPlayEpisode argument 0 must be text")?,
                    )
                    .map_err(|_| "Text contains NUL")?;
                    let a1 = args
                        .get(1)
                        .and_then(Value::as_i64)
                        .ok_or("DZPlayEpisode argument 1 must be integer")?;
                    let function = self
                        .library
                        .get::<unsafe extern "C" fn(*const c_char, i64) -> i32>(b"DZPlayEpisode\0")
                        .map_err(|e| e.to_string())?;
                    Ok(serde_json::json!(function(a0.as_ptr(), a1)))
                }
                "DZEQJSON" => {
                    if !args.is_empty() {
                        return Err("DZEQJSON: wrong argument count".into());
                    }
                    let function = self
                        .library
                        .get::<unsafe extern "C" fn() -> *mut c_char>(b"DZEQJSON\0")
                        .map_err(|e| e.to_string())?;
                    self.take(function())
                }
                "DZSetEQJSON" => {
                    if args.len() != 1 {
                        return Err("DZSetEQJSON: wrong argument count".into());
                    }
                    let a0 = CString::new(
                        args.first()
                            .and_then(Value::as_str)
                            .ok_or("DZSetEQJSON argument 0 must be text")?,
                    )
                    .map_err(|_| "Text contains NUL")?;
                    let function = self
                        .library
                        .get::<unsafe extern "C" fn(*const c_char) -> i32>(b"DZSetEQJSON\0")
                        .map_err(|e| e.to_string())?;
                    Ok(serde_json::json!(function(a0.as_ptr())))
                }
                "DZClearPreload" => {
                    if !args.is_empty() {
                        return Err("DZClearPreload: wrong argument count".into());
                    }
                    let function = self
                        .library
                        .get::<unsafe extern "C" fn() -> ()>(b"DZClearPreload\0")
                        .map_err(|e| e.to_string())?;
                    function();
                    Ok(Value::Null)
                }
                "DZQueueSet" => {
                    if args.len() != 1 {
                        return Err("DZQueueSet: wrong argument count".into());
                    }
                    let a0 = CString::new(
                        args.first()
                            .and_then(Value::as_str)
                            .ok_or("DZQueueSet argument 0 must be text")?,
                    )
                    .map_err(|_| "Text contains NUL")?;
                    let function = self
                        .library
                        .get::<unsafe extern "C" fn(*const c_char) -> i32>(b"DZQueueSet\0")
                        .map_err(|e| e.to_string())?;
                    Ok(serde_json::json!(function(a0.as_ptr())))
                }
                "DZQueueSetIndex" => {
                    if args.len() != 1 {
                        return Err("DZQueueSetIndex: wrong argument count".into());
                    }
                    let a0 = args
                        .first()
                        .and_then(Value::as_i64)
                        .ok_or("DZQueueSetIndex argument 0 must be integer")?
                        as i32;
                    let function = self
                        .library
                        .get::<unsafe extern "C" fn(i32) -> ()>(b"DZQueueSetIndex\0")
                        .map_err(|e| e.to_string())?;
                    function(a0);
                    Ok(Value::Null)
                }
                "DZQueueIndex" => {
                    if !args.is_empty() {
                        return Err("DZQueueIndex: wrong argument count".into());
                    }
                    let function = self
                        .library
                        .get::<unsafe extern "C" fn() -> i32>(b"DZQueueIndex\0")
                        .map_err(|e| e.to_string())?;
                    Ok(serde_json::json!(function()))
                }
                "DZQueueVersion" => {
                    if !args.is_empty() {
                        return Err("DZQueueVersion: wrong argument count".into());
                    }
                    let function = self
                        .library
                        .get::<unsafe extern "C" fn() -> i64>(b"DZQueueVersion\0")
                        .map_err(|e| e.to_string())?;
                    Ok(serde_json::json!(function()))
                }
                "DZQueueJSON" => {
                    if !args.is_empty() {
                        return Err("DZQueueJSON: wrong argument count".into());
                    }
                    let function = self
                        .library
                        .get::<unsafe extern "C" fn() -> *mut c_char>(b"DZQueueJSON\0")
                        .map_err(|e| e.to_string())?;
                    self.take(function())
                }
                "DZQueueRemove" => {
                    if args.len() != 1 {
                        return Err("DZQueueRemove: wrong argument count".into());
                    }
                    let a0 = args
                        .first()
                        .and_then(Value::as_i64)
                        .ok_or("DZQueueRemove argument 0 must be integer")?
                        as i32;
                    let function = self
                        .library
                        .get::<unsafe extern "C" fn(i32) -> ()>(b"DZQueueRemove\0")
                        .map_err(|e| e.to_string())?;
                    function(a0);
                    Ok(Value::Null)
                }
                "DZQueueMove" => {
                    if args.len() != 2 {
                        return Err("DZQueueMove: wrong argument count".into());
                    }
                    let a0 = args
                        .first()
                        .and_then(Value::as_i64)
                        .ok_or("DZQueueMove argument 0 must be integer")?
                        as i32;
                    let a1 = args
                        .get(1)
                        .and_then(Value::as_i64)
                        .ok_or("DZQueueMove argument 1 must be integer")?
                        as i32;
                    let function = self
                        .library
                        .get::<unsafe extern "C" fn(i32, i32) -> ()>(b"DZQueueMove\0")
                        .map_err(|e| e.to_string())?;
                    function(a0, a1);
                    Ok(Value::Null)
                }
                "DZQueueInsertNext" => {
                    if args.len() != 1 {
                        return Err("DZQueueInsertNext: wrong argument count".into());
                    }
                    let a0 = CString::new(
                        args.first()
                            .and_then(Value::as_str)
                            .ok_or("DZQueueInsertNext argument 0 must be text")?,
                    )
                    .map_err(|_| "Text contains NUL")?;
                    let function = self
                        .library
                        .get::<unsafe extern "C" fn(*const c_char) -> ()>(b"DZQueueInsertNext\0")
                        .map_err(|e| e.to_string())?;
                    function(a0.as_ptr());
                    Ok(Value::Null)
                }
                "DZDownloadForOffline" => {
                    if args.len() != 1 {
                        return Err("DZDownloadForOffline: wrong argument count".into());
                    }
                    let a0 = CString::new(
                        args.first()
                            .and_then(Value::as_str)
                            .ok_or("DZDownloadForOffline argument 0 must be text")?,
                    )
                    .map_err(|_| "Text contains NUL")?;
                    let function = self
                        .library
                        .get::<unsafe extern "C" fn(*const c_char) -> *mut c_char>(
                            b"DZDownloadForOffline\0",
                        )
                        .map_err(|e| e.to_string())?;
                    self.take(function(a0.as_ptr()))
                }
                "DZDownloadForOfflineAtQuality" => {
                    if args.len() != 2 {
                        return Err("DZDownloadForOfflineAtQuality: wrong argument count".into());
                    }
                    let a0 = CString::new(
                        args.first()
                            .and_then(Value::as_str)
                            .ok_or("DZDownloadForOfflineAtQuality argument 0 must be text")?,
                    )
                    .map_err(|_| "Text contains NUL")?;
                    let a1 = args
                        .get(1)
                        .and_then(Value::as_i64)
                        .ok_or("DZDownloadForOfflineAtQuality argument 1 must be integer")?
                        as i32;
                    let function = self
                        .library
                        .get::<unsafe extern "C" fn(*const c_char, i32) -> *mut c_char>(
                            b"DZDownloadForOfflineAtQuality\0",
                        )
                        .map_err(|e| e.to_string())?;
                    self.take(function(a0.as_ptr(), a1))
                }
                "DZRemoveOfflineTrack" => {
                    if args.len() != 1 {
                        return Err("DZRemoveOfflineTrack: wrong argument count".into());
                    }
                    let a0 = CString::new(
                        args.first()
                            .and_then(Value::as_str)
                            .ok_or("DZRemoveOfflineTrack argument 0 must be text")?,
                    )
                    .map_err(|_| "Text contains NUL")?;
                    let function = self
                        .library
                        .get::<unsafe extern "C" fn(*const c_char) -> *mut c_char>(
                            b"DZRemoveOfflineTrack\0",
                        )
                        .map_err(|e| e.to_string())?;
                    self.take(function(a0.as_ptr()))
                }
                "DZFlowConfigsJSON" => {
                    if !args.is_empty() {
                        return Err("DZFlowConfigsJSON: wrong argument count".into());
                    }
                    let function = self
                        .library
                        .get::<unsafe extern "C" fn() -> *mut c_char>(b"DZFlowConfigsJSON\0")
                        .map_err(|e| e.to_string())?;
                    self.take(function())
                }
                "DZFlowForConfigJSON" => {
                    if args.len() != 1 {
                        return Err("DZFlowForConfigJSON: wrong argument count".into());
                    }
                    let a0 = CString::new(
                        args.first()
                            .and_then(Value::as_str)
                            .ok_or("DZFlowForConfigJSON argument 0 must be text")?,
                    )
                    .map_err(|_| "Text contains NUL")?;
                    let function = self
                        .library
                        .get::<unsafe extern "C" fn(*const c_char) -> *mut c_char>(
                            b"DZFlowForConfigJSON\0",
                        )
                        .map_err(|e| e.to_string())?;
                    self.take(function(a0.as_ptr()))
                }
                "DZFlowAppendJSON" => {
                    if args.len() != 2 {
                        return Err("DZFlowAppendJSON: wrong argument count".into());
                    }
                    let a0 = CString::new(
                        args.first()
                            .and_then(Value::as_str)
                            .ok_or("DZFlowAppendJSON argument 0 must be text")?,
                    )
                    .map_err(|_| "Text contains NUL")?;
                    let a1 = args
                        .get(1)
                        .and_then(Value::as_i64)
                        .ok_or("DZFlowAppendJSON argument 1 must be integer")?
                        as u64;
                    let function = self
                        .library
                        .get::<unsafe extern "C" fn(*const c_char, u64) -> i32>(
                            b"DZFlowAppendJSON\0",
                        )
                        .map_err(|e| e.to_string())?;
                    Ok(serde_json::json!(function(a0.as_ptr(), a1)))
                }
                "DZFlowTunerJSON" => {
                    if args.len() != 1 {
                        return Err("DZFlowTunerJSON: wrong argument count".into());
                    }
                    let a0 = CString::new(
                        args.first()
                            .and_then(Value::as_str)
                            .ok_or("DZFlowTunerJSON argument 0 must be text")?,
                    )
                    .map_err(|_| "Text contains NUL")?;
                    let function = self
                        .library
                        .get::<unsafe extern "C" fn(*const c_char) -> *mut c_char>(
                            b"DZFlowTunerJSON\0",
                        )
                        .map_err(|e| e.to_string())?;
                    self.take(function(a0.as_ptr()))
                }
                "DZSaveFlowTunerJSON" => {
                    if args.len() != 2 {
                        return Err("DZSaveFlowTunerJSON: wrong argument count".into());
                    }
                    let a0 = CString::new(
                        args.first()
                            .and_then(Value::as_str)
                            .ok_or("DZSaveFlowTunerJSON argument 0 must be text")?,
                    )
                    .map_err(|_| "Text contains NUL")?;
                    let a1 = CString::new(
                        args.get(1)
                            .and_then(Value::as_str)
                            .ok_or("DZSaveFlowTunerJSON argument 1 must be text")?,
                    )
                    .map_err(|_| "Text contains NUL")?;
                    let function = self
                        .library
                        .get::<unsafe extern "C" fn(*const c_char, *const c_char) -> *mut c_char>(
                            b"DZSaveFlowTunerJSON\0",
                        )
                        .map_err(|e| e.to_string())?;
                    self.take(function(a0.as_ptr(), a1.as_ptr()))
                }
                "DZHomeModulesJSON" => {
                    if !args.is_empty() {
                        return Err("DZHomeModulesJSON: wrong argument count".into());
                    }
                    let function = self
                        .library
                        .get::<unsafe extern "C" fn() -> *mut c_char>(b"DZHomeModulesJSON\0")
                        .map_err(|e| e.to_string())?;
                    self.take(function())
                }
                "DZSmartMixJSON" => {
                    if args.len() != 1 {
                        return Err("DZSmartMixJSON: wrong argument count".into());
                    }
                    let a0 = CString::new(
                        args.first()
                            .and_then(Value::as_str)
                            .ok_or("DZSmartMixJSON argument 0 must be text")?,
                    )
                    .map_err(|_| "Text contains NUL")?;
                    let function = self
                        .library
                        .get::<unsafe extern "C" fn(*const c_char) -> *mut c_char>(
                            b"DZSmartMixJSON\0",
                        )
                        .map_err(|e| e.to_string())?;
                    self.take(function(a0.as_ptr()))
                }
                "DZTrackMetadataJSON" => {
                    if args.len() != 1 {
                        return Err("DZTrackMetadataJSON: wrong argument count".into());
                    }
                    let a0 = CString::new(
                        args.first()
                            .and_then(Value::as_str)
                            .ok_or("DZTrackMetadataJSON argument 0 must be text")?,
                    )
                    .map_err(|_| "Text contains NUL")?;
                    let function = self
                        .library
                        .get::<unsafe extern "C" fn(*const c_char) -> *mut c_char>(
                            b"DZTrackMetadataJSON\0",
                        )
                        .map_err(|e| e.to_string())?;
                    self.take(function(a0.as_ptr()))
                }
                "DZHomeModuleJSON" => {
                    if args.len() != 1 {
                        return Err("DZHomeModuleJSON: wrong argument count".into());
                    }
                    let a0 = args
                        .first()
                        .and_then(Value::as_i64)
                        .ok_or("DZHomeModuleJSON argument 0 must be integer")?
                        as i32;
                    let function = self
                        .library
                        .get::<unsafe extern "C" fn(i32) -> *mut c_char>(b"DZHomeModuleJSON\0")
                        .map_err(|e| e.to_string())?;
                    self.take(function(a0))
                }
                "DZDiscoveryPageJSON" => {
                    if args.len() != 2 {
                        return Err("DZDiscoveryPageJSON: wrong argument count".into());
                    }
                    let a0 = CString::new(
                        args.first()
                            .and_then(Value::as_str)
                            .ok_or("DZDiscoveryPageJSON argument 0 must be text")?,
                    )
                    .map_err(|_| "Text contains NUL")?;
                    let a1 = CString::new(
                        args.get(1)
                            .and_then(Value::as_str)
                            .ok_or("DZDiscoveryPageJSON argument 1 must be text")?,
                    )
                    .map_err(|_| "Text contains NUL")?;
                    let function = self
                        .library
                        .get::<unsafe extern "C" fn(*const c_char, *const c_char) -> *mut c_char>(
                            b"DZDiscoveryPageJSON\0",
                        )
                        .map_err(|e| e.to_string())?;
                    self.take(function(a0.as_ptr(), a1.as_ptr()))
                }
                "DZDiscoverySectionJSON" => {
                    if args.len() != 3 {
                        return Err("DZDiscoverySectionJSON: wrong argument count".into());
                    }
                    let a0 = CString::new(
                        args.first()
                            .and_then(Value::as_str)
                            .ok_or("DZDiscoverySectionJSON argument 0 must be text")?,
                    )
                    .map_err(|_| "Text contains NUL")?;
                    let a1 = CString::new(
                        args.get(1)
                            .and_then(Value::as_str)
                            .ok_or("DZDiscoverySectionJSON argument 1 must be text")?,
                    )
                    .map_err(|_| "Text contains NUL")?;
                    let a2 = CString::new(
                        args.get(2)
                            .and_then(Value::as_str)
                            .ok_or("DZDiscoverySectionJSON argument 2 must be text")?,
                    )
                    .map_err(|_| "Text contains NUL")?;
                    let function = self
                        .library
                        .get::<unsafe extern "C" fn(
                            *const c_char,
                            *const c_char,
                            *const c_char,
                        ) -> *mut c_char>(b"DZDiscoverySectionJSON\0")
                        .map_err(|e| e.to_string())?;
                    self.take(function(a0.as_ptr(), a1.as_ptr(), a2.as_ptr()))
                }
                "DZLogout" => {
                    if !args.is_empty() {
                        return Err("DZLogout: wrong argument count".into());
                    }
                    let function = self
                        .library
                        .get::<unsafe extern "C" fn() -> ()>(b"DZLogout\0")
                        .map_err(|e| e.to_string())?;
                    function();
                    Ok(Value::Null)
                }
                "DZDownloadEpisodeJSON" => {
                    if args.len() != 1 {
                        return Err("DZDownloadEpisodeJSON: wrong argument count".into());
                    }
                    let a0 = CString::new(
                        args.first()
                            .and_then(Value::as_str)
                            .ok_or("DZDownloadEpisodeJSON argument 0 must be text")?,
                    )
                    .map_err(|_| "Text contains NUL")?;
                    let function = self
                        .library
                        .get::<unsafe extern "C" fn(*const c_char) -> *mut c_char>(
                            b"DZDownloadEpisodeJSON\0",
                        )
                        .map_err(|e| e.to_string())?;
                    self.take(function(a0.as_ptr()))
                }
                "DZRemoveEpisodeJSON" => {
                    if args.len() != 1 {
                        return Err("DZRemoveEpisodeJSON: wrong argument count".into());
                    }
                    let a0 = CString::new(
                        args.first()
                            .and_then(Value::as_str)
                            .ok_or("DZRemoveEpisodeJSON argument 0 must be text")?,
                    )
                    .map_err(|_| "Text contains NUL")?;
                    let function = self
                        .library
                        .get::<unsafe extern "C" fn(*const c_char) -> *mut c_char>(
                            b"DZRemoveEpisodeJSON\0",
                        )
                        .map_err(|e| e.to_string())?;
                    self.take(function(a0.as_ptr()))
                }
                "DZPodcastShelfJSON" => {
                    if args.len() != 1 {
                        return Err("DZPodcastShelfJSON: wrong argument count".into());
                    }
                    let a0 = args
                        .first()
                        .and_then(Value::as_i64)
                        .ok_or("DZPodcastShelfJSON argument 0 must be integer")?
                        as i32;
                    let function = self
                        .library
                        .get::<unsafe extern "C" fn(i32) -> *mut c_char>(b"DZPodcastShelfJSON\0")
                        .map_err(|e| e.to_string())?;
                    self.take(function(a0))
                }
                "DZPodcastHomeJSON" => {
                    if !args.is_empty() {
                        return Err("DZPodcastHomeJSON: wrong argument count".into());
                    }
                    let function = self
                        .library
                        .get::<unsafe extern "C" fn() -> *mut c_char>(b"DZPodcastHomeJSON\0")
                        .map_err(|e| e.to_string())?;
                    self.take(function())
                }
                "DZControlConfigJSON" => {
                    if !args.is_empty() {
                        return Err("DZControlConfigJSON: wrong argument count".into());
                    }
                    let function = self
                        .library
                        .get::<unsafe extern "C" fn() -> *mut c_char>(b"DZControlConfigJSON\0")
                        .map_err(|e| e.to_string())?;
                    self.take(function())
                }
                "DZSetControlConfig" => {
                    if args.len() != 3 {
                        return Err("DZSetControlConfig: wrong argument count".into());
                    }
                    let a0 = args
                        .first()
                        .and_then(Value::as_i64)
                        .ok_or("DZSetControlConfig argument 0 must be integer")?
                        as i32;
                    let a1 = CString::new(
                        args.get(1)
                            .and_then(Value::as_str)
                            .ok_or("DZSetControlConfig argument 1 must be text")?,
                    )
                    .map_err(|_| "Text contains NUL")?;
                    let a2 = CString::new(
                        args.get(2)
                            .and_then(Value::as_str)
                            .ok_or("DZSetControlConfig argument 2 must be text")?,
                    )
                    .map_err(|_| "Text contains NUL")?;
                    let function = self
                        .library
                        .get::<unsafe extern "C" fn(i32, *const c_char, *const c_char) -> ()>(
                            b"DZSetControlConfig\0",
                        )
                        .map_err(|e| e.to_string())?;
                    function(a0, a1.as_ptr(), a2.as_ptr());
                    Ok(Value::Null)
                }
                "DZWebRemoteSetEnabled" => {
                    if args.len() != 1 {
                        return Err("DZWebRemoteSetEnabled: wrong argument count".into());
                    }
                    let a0 = args
                        .first()
                        .and_then(Value::as_i64)
                        .ok_or("DZWebRemoteSetEnabled argument 0 must be integer")?
                        as i32;
                    let function = self
                        .library
                        .get::<unsafe extern "C" fn(i32) -> ()>(b"DZWebRemoteSetEnabled\0")
                        .map_err(|e| e.to_string())?;
                    function(a0);
                    Ok(Value::Null)
                }
                "DZWebRemoteInfoJSON" => {
                    if !args.is_empty() {
                        return Err("DZWebRemoteInfoJSON: wrong argument count".into());
                    }
                    let function = self
                        .library
                        .get::<unsafe extern "C" fn() -> *mut c_char>(b"DZWebRemoteInfoJSON\0")
                        .map_err(|e| e.to_string())?;
                    self.take(function())
                }
                _ => Err(format!("Unsupported engine operation: {name}")),
            }
        }
    }
}
