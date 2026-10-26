use crate::{catalog, engine::Engine};
use serde_json::json;
use std::{ffi::CString, io::Write, time::Instant};
pub fn run() -> Result<(), String> {
    let mut report = std::fs::File::create(".local/integration.log").map_err(|e| e.to_string())?;
    let engine = Engine::load()?;
    let secret = CString::new(crate::session::saved()?).map_err(|_| "Invalid saved session")?;
    if unsafe { (engine.init)(secret.as_ptr()) } == 0 {
        return Err(format!("Sign-in failed (kind {})", unsafe {
            (engine.login_error)()
        }));
    }
    let account = engine.invoke("DZAccountJSON", &[])?;
    if account["loggedIn"].as_bool() != Some(true)
        || catalog::string(&account, &["userId"]).is_empty()
    {
        return Err("Account identity verification failed".into());
    }
    writeln!(
        report,
        "Account identity: verified (no personal values recorded)"
    )
    .ok();
    let mut available = 0;
    for name in crate::engine::api::EXPORTS {
        let name = CString::new(*name).unwrap();
        if unsafe {
            engine
                .library
                .get::<*const std::ffi::c_void>(name.as_bytes_with_nul())
        }
        .is_ok()
        {
            available += 1;
        }
    }
    writeln!(
        report,
        "Engine exports: {available}/{}",
        crate::engine::api::EXPORTS.len()
    )
    .ok();
    let options = engine.invoke("DZFlowConfigsJSON", &[])?;
    let options = options["options"]
        .as_array()
        .ok_or("Flow options missing")?;
    writeln!(report, "Flow options: {}", options.len()).ok();
    for id in std::iter::once("default".to_owned()).chain(
        options
            .iter()
            .find(|o| o["kind"].as_str() == Some("mood"))
            .map(|o| catalog::string(o, &["id"])),
    ) {
        let tuner = engine.invoke("DZFlowTunerJSON", &[json!(id)])?;
        let genres = tuner["genres"].as_array().ok_or("Flow genres missing")?;
        if genres.is_empty() {
            return Err("Flow returned an empty genre grid".into());
        }
        let tracks = catalog::entries(
            &engine.invoke("DZFlowForConfigJSON", &[json!(id)])?,
            "track",
        );
        if tracks.is_empty() {
            return Err("Flow returned no playable batch".into());
        }
        writeln!(
            report,
            "Flow {id}: {} genres / {} enabled / {} tracks (read-only)",
            genres.len(),
            genres
                .iter()
                .filter(|g| g["isEnabled"].as_bool() == Some(true))
                .count(),
            tracks.len()
        )
        .ok();
    }
    let mut track = None;
    let mut first_collection = None;
    for index in 0..8 {
        let start = Instant::now();
        match engine.invoke("DZHomeModuleJSON", &[json!(index)]) {
            Ok(value) => {
                let items = catalog::entries(
                    &value,
                    match index {
                        4 => "artist",
                        5 => "track",
                        7 => "album",
                        _ => "playlist",
                    },
                );
                writeln!(
                    report,
                    "Home module {index}: {} entries, {:.2}s, root keys: {:?}",
                    items.len(),
                    start.elapsed().as_secs_f64(),
                    value.as_object().map(|v| v.keys().collect::<Vec<_>>())
                )
                .ok();
                if first_collection.is_none() {
                    first_collection = items
                        .iter()
                        .find(|e| e.kind == "playlist" || e.kind == "album")
                        .cloned();
                }
                if track.is_none() {
                    track = items.into_iter().find(|e| e.playable());
                }
            }
            Err(error) => {
                writeln!(report, "Home module {index}: ERROR {error}").ok();
            }
        }
        report.flush().ok();
    }
    for (name, args, kind) in [
        ("DZSearchJSON", vec![json!("Daft Punk")], "track"),
        ("DZFavoritesJSON", vec![], "track"),
        ("DZFavoriteAlbumsJSON", vec![], "album"),
        ("DZPlaylistsJSON", vec![], "playlist"),
        ("DZPodcastHomeJSON", vec![], "podcast"),
        ("DZFlowJSON", vec![], "track"),
    ] {
        let start = Instant::now();
        match engine.invoke(name, &args) {
            Ok(value) => {
                let items = catalog::entries(&value, kind);
                writeln!(
                    report,
                    "{name}: {} entries, {:.2}s",
                    items.len(),
                    start.elapsed().as_secs_f64()
                )
                .ok();
                if track.is_none() {
                    track = items.into_iter().find(|e| e.playable());
                }
            }
            Err(error) => {
                writeln!(report, "{name}: ERROR {error}").ok();
            }
        }
        report.flush().ok();
    }
    if let Some(item) = first_collection {
        let name = if item.kind == "album" {
            "DZAlbumTracksJSON"
        } else {
            "DZPlaylistTracksJSON"
        };
        let value = engine.invoke(name, &[json!(item.id)])?;
        let tracks = catalog::entries(&value, "track");
        writeln!(report, "Collection open: {} tracks", tracks.len()).ok();
    }
    if let Some(item) = track {
        if !item.artwork.is_empty() {
            let image = catalog::decode(&engine.fetch(&item.artwork)?, 256)?;
            writeln!(report, "Artwork: {}x{} decoded", image.width, image.height).ok();
        }
        unsafe {
            (engine.volume)(0.0);
        }
        if engine
            .invoke("DZPlay", &[json!(item.id), json!(item.duration)])?
            .as_i64()
            != Some(1)
        {
            return Err("Audio playback failed".into());
        }
        let deadline = Instant::now() + std::time::Duration::from_secs(30);
        while unsafe { (engine.state)() != 2 || (engine.position)() < 250 } {
            if unsafe { (engine.state)() } == 4 {
                return Err("Audio output entered the error state".into());
            }
            if Instant::now() > deadline {
                return Err("Timed out waiting for advancing audio output".into());
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        writeln!(
            report,
            "Audio state: {}, position: {} ms",
            unsafe { (engine.state)() },
            unsafe { (engine.position)() }
        )
        .ok();
        engine.invoke("DZPause", &[])?;
        engine.invoke("DZSeek", &[json!(1000)])?;
        engine.invoke("DZResume", &[])?;
        engine.invoke("DZStop", &[])?;
        match engine.invoke("DZLyricsJSON", &[json!(item.id)]) {
            Ok(value) => {
                writeln!(report, "Lyrics: {} bytes", value.to_string().len()).ok();
            }
            Err(error) => {
                writeln!(report, "Lyrics: {error}").ok();
            }
        }
    } else {
        return Err("No playable tracks returned".into());
    }
    writeln!(report, "Integration completed").ok();
    Ok(())
}
