use crate::{catalog, engine::Engine};
use serde_json::json;
use std::{ffi::CString, io::Write, time::Instant};
pub fn run() -> Result<(), String> {
    if std::env::var_os("DEEZNATS_PLAYBACK_BENCHMARK").is_some() {
        return playback_benchmark();
    }
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
    if let Some(mut item) = track {
        if let Ok(id) = std::env::var("DEEZNATS_TEST_TRACK") {
            if !id.is_empty() && id.bytes().all(|b| b.is_ascii_digit()) {
                item.id = id;
                item.duration = 0;
            }
        }
        if !item.artwork.is_empty() {
            let image = catalog::decode(&engine.fetch(&item.artwork)?, 256)?;
            writeln!(report, "Artwork: {}x{} decoded", image.width, image.height).ok();
        }
        unsafe {
            (engine.volume)(0.0);
        }
        let play_started = Instant::now();
        if engine
            .invoke("DZPlay", &[json!(item.id), json!(item.duration)])?
            .as_i64()
            != Some(1)
        {
            return Err("Audio playback failed".into());
        }
        writeln!(report, "Playback resolve: {:?}", play_started.elapsed()).ok();
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
        writeln!(
            report,
            "Playback to advancing output: {:?}",
            play_started.elapsed()
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

fn playback_benchmark() -> Result<(), String> {
    use std::time::Duration;
    let mut report =
        std::fs::File::create(".local/playback-benchmark.log").map_err(|e| e.to_string())?;
    let engine = Engine::load()?;
    let secret = CString::new(crate::session::saved()?).map_err(|_| "Invalid saved session")?;
    if unsafe { (engine.init)(secret.as_ptr()) } == 0 {
        return Err("Saved-session sign-in failed".into());
    }
    unsafe {
        (engine.volume)(0.0);
    }
    let result = (|| {
        let track = std::env::var("DEEZNATS_TEST_TRACK").unwrap_or_else(|_| "4059498211".into());
        if track.is_empty() || !track.bytes().all(|b| b.is_ascii_digit()) {
            return Err("Invalid test track".into());
        }
        for trial in 1..=3 {
            let start = Instant::now();
            if engine.invoke("DZPlay", &[json!(track), json!(0)])?.as_i64() != Some(1) {
                return Err("Playback resolve failed".into());
            }
            let resolve = start.elapsed();
            while unsafe { (engine.state)() != 2 || (engine.position)() == 0 } {
                if unsafe { (engine.state)() } == 4 || start.elapsed() > Duration::from_secs(30) {
                    return Err("Playback did not start".into());
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            let first_output = start.elapsed();
            writeln!(
                report,
                "format={}",
                engine
                    .invoke("DZFormat", &[])?
                    .as_str()
                    .unwrap_or("unknown")
            )
            .ok();
            let watch = Instant::now();
            let mut last_position = unsafe { (engine.position)() };
            let mut last_advance = Instant::now();
            let mut longest_stall = Duration::ZERO;
            while watch.elapsed() < Duration::from_secs(8) {
                std::thread::sleep(Duration::from_millis(20));
                if unsafe { (engine.state)() } != 2 {
                    return Err("Playback left Playing during continuity check".into());
                }
                let position = unsafe { (engine.position)() };
                longest_stall = longest_stall.max(last_advance.elapsed());
                if position > last_position {
                    last_position = position;
                    last_advance = Instant::now();
                }
            }
            writeln!(
                report,
                "trial={trial} resolve_ms={} first_output_ms={} longest_position_stall_ms={}",
                resolve.as_millis(),
                first_output.as_millis(),
                longest_stall.as_millis()
            )
            .ok();
            if std::env::var_os("DEEZNATS_TEST_SEEK").is_some() {
                for target in [30000u64, 1000, 60000, 5000] {
                    let seeking = Instant::now();
                    engine.invoke("DZSeek", &[json!(target)])?;
                    while unsafe { (engine.position)() } < target as i64 + 250 {
                        if unsafe { (engine.state)() } == 4
                            || seeking.elapsed() > Duration::from_secs(20)
                        {
                            writeln!(report, "seek_failed target_ms={target} state={}", unsafe {
                                (engine.state)()
                            })
                            .ok();
                            return Err("Seek did not recover advancing audio".into());
                        }
                        std::thread::sleep(Duration::from_millis(10));
                    }
                    writeln!(
                        report,
                        "seek_target_ms={target} recovery_ms={}",
                        seeking.elapsed().as_millis()
                    )
                    .ok();
                }
                engine.invoke("DZPause", &[])?;
                engine.invoke("DZSeek", &[json!(2000)])?;
                std::thread::sleep(Duration::from_millis(150));
                if unsafe { (engine.state)() } != 3 {
                    return Err("Seeking while paused lost pause state".into());
                }
                engine.invoke("DZResume", &[])?;
                for target in [9000u64, 2000, 12000, 4000, 18000, 8000] {
                    engine.invoke("DZSeek", &[json!(target)])?;
                }
                let recovery = Instant::now();
                while unsafe { (engine.position)() } < 8250 {
                    if recovery.elapsed() > Duration::from_secs(20)
                        || unsafe { (engine.state)() } == 4
                    {
                        return Err("Rapid seeking did not recover".into());
                    }
                    std::thread::sleep(Duration::from_millis(10));
                }
                writeln!(report, "paused_and_rapid_seek=passed").ok();
            }
            engine.invoke("DZStop", &[])?;
        }
        Ok(())
    })();
    let _ = engine.invoke("DZStop", &[]);
    result
}
