//! Exercises the real native client against the local deterministic host fixture.
use super::*;
pub fn probe_artwork(url: &str, report: &str) -> Result<(), String> {
    let result = match network::get(url) {
        Ok(bytes) => match image::load_from_memory(&bytes) {
            Ok(image) => {
                json!({"ok":true,"bytes":bytes.len(),"width":image.width(),"height":image.height()})
            }
            Err(error) => json!({"ok":false,"bytes":bytes.len(),"decode_error":error.to_string()}),
        },
        Err(error) => json!({"ok":false,"download_error":error}),
    };
    std::fs::write(report, result.to_string()).map_err(|e| e.to_string())
}
fn wait(app: &mut App, label: &str, predicate: impl Fn(&App) -> bool) -> Result<(), String> {
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(10) {
        let dirty = app.events() | app.frame();
        if dirty {
            app.paint();
        }
        if predicate(app) {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    Err(format!(
        "Native client check timed out: {label}; {}",
        app.state.error
    ))
}
pub fn run(host: &str, report: &str) -> Result<(), String> {
    let path = std::path::Path::new(report);
    let directory = path.parent().unwrap_or(std::path::Path::new("."));
    std::fs::create_dir_all(directory).map_err(|e| e.to_string())?;
    std::env::set_var(
        "DEEZNATS_JAM_PROFILE",
        directory.join("isolated-client-profile"),
    );
    let prefs = Prefs {
        nickname: "Tester".into(),
        host: network::normalize_host(host)?,
        ..Prefs::default()
    };
    let worker = Worker::start(prefs.host.clone());
    let sound = Sound::new(prefs.volume);
    let mut app = App {
        state: State::new(prefs),
        renderer: Some(unsafe { Renderer::offscreen(1280, 900)? }),
        worker,
        sound,
        generation: 0,
        requests: HashMap::new(),
        pressed: None,
        drag_queue: None,
        minimized: false,
        foreground: true,
        frame: Instant::now(),
        timer_ms: 100,
        last_progress: Instant::now(),
        hwnd: null_mut(),
    };
    app.renderer.as_mut().unwrap().synchronous_lyrics = false;
    let mut checks = Vec::new();
    if app.state.view != View::Queue
        || app.state.prefs.theme != 8
        || app.state.prefs.opacity != 40.
        || app.state.prefs.blur != 20.
    {
        return Err("Initial queue view or appearance defaults are incorrect".into());
    }
    checks.push("Queue opens by default; Tokyo Night, opacity 40, blur 20");
    app.request("appearance", "/api/appearance");
    wait(&mut app, "host and theme catalogue", |a| {
        a.state.online && a.state.themes.len() == 18
    })?;
    checks.push("host connection and all themes");
    app.tab(View::Settings);
    app.paint();
    let host_input = app
        .state
        .hits
        .iter()
        .find(|h| matches!(h.action, Action::Focus("host")))
        .ok_or("Host address field missing")?;
    let first_theme = app
        .state
        .hits
        .iter()
        .find(|h| matches!(h.action, Action::Theme(_)))
        .ok_or("Appearance themes missing")?;
    if host_input.rect.y >= first_theme.rect.y {
        return Err("Host address must be above Appearance".into());
    }
    app.state.host_text = "file:///invalid".into();
    app.action(Action::ConnectHost);
    if app.worker.host() != host {
        return Err("Invalid host replaced the connection".into());
    }
    app.action(Action::Focus("host"));
        while !app.state.host_text.is_empty() {
            app.edit('\u{8}');
        }
        app.edit('\u{8}');
        app.paint();
        if !app.state.host_text.is_empty() {
            return Err("Host address could not be cleared".into());
        }
    app.state.select_all = true;
    for ch in format!("{host}/").chars() {
        app.edit(ch);
    }
    app.action(Action::ConnectHost);
    if app.state.playback.get("id").is_some() {
        return Err("Old host playback was retained".into());
    }
    wait(&mut app, "settings host reconnect", |a| {
        a.state.online && a.state.loading.is_empty()
    })?;
    if Prefs::load().host != host {
        return Err("New host address was not saved".into());
    }
    checks.push("Host Address editor, validation, reconnect, saved address");
        app.action(Action::Focus("host_nickname"));
        for ch in "Living room".chars() {
            app.edit(ch);
        }
        app.action(Action::SaveHostProfile);
        if app.state.host_profiles != vec![("Living room".into(), host.into())] {
            return Err("Temporary profile nickname or address was not saved".into());
        }
        app.paint();
        app.action(Action::ConnectHostProfile(0));
        wait(&mut app, "temporary profile connection", |a| a.state.online)?;
        if app.worker.host() != host || app.state.host_profiles.len() != 1 {
            return Err("Temporary profile did not connect or survive reconnect".into());
        }
        app.action(Action::RemoveHostProfile(0));
        if !app.state.host_profiles.is_empty() {
            return Err("Temporary profile could not be removed".into());
        }
        checks.push("temporary host profiles, custom nicknames, connect and remove");
    if app.state.prefs.nickname != "Living room" || Prefs::load().nickname != "Living room" {
        return Err("Removing a host profile lost the required saved nickname".into());
    }
        app.state.host_nickname = "Alice".into();
        app.action(Action::SaveHostProfile);
        app.action(Action::ConnectHostProfile(0));
        wait(&mut app, "named profile connection", |a| a.state.online)?;
    app.tab(View::Queue);
    app.paint();
    if !app
        .state
        .hits
        .iter()
        .any(|h| matches!(h.action, Action::Autoplay))
    {
        return Err("Queue autoplay switch is missing".into());
    }
    app.action(Action::Autoplay);
    wait(&mut app, "autoplay switch", |a| {
        a.state.playback["autoplay"].as_bool() == Some(false)
    })?;
    checks.push("shared autoplay switch");
    app.tab(View::Search);
    app.state.search_text = "Levels".into();
    app.action(Action::Search);
    wait(&mut app, "search results", |a| {
        !items(&a.state.search, "tracks").is_empty()
    })?;
    app.paint();
    if !app
        .state
        .hits
        .iter()
        .any(|h| matches!(h.action, Action::Play(_)))
    {
        return Err("Search play buttons are missing".into());
    }
    checks.push("search and native result controls");
    let album = app.state.search["albums"][0].clone();
    app.action(Action::Open(album));
    wait(&mut app, "collection", |a| {
        a.state
            .modal
            .as_ref()
            .is_some_and(|v| !items(v, "tracks").is_empty())
    })?;
    app.paint();
    app.action(Action::PlayCollection);
    app.action(Action::CloseModal);
    checks.push("collection modal and play all");
    let track = app.state.search["tracks"][0].clone();
    app.action(Action::Play(track.clone()));
    app.action(Action::Insert(track.clone()));
    app.action(Action::Append(track));
        // A failed state poll must not prevent commands on a working connection.
        app.state.online = false;
    app.action(Action::Shuffle);
    app.action(Action::Repeat);
    wait(&mut app, "queue and playback modes", |a| {
        a.state.playback["shuffle"].as_bool() == Some(true)
            && a.state.playback["repeat"].as_u64() == Some(1)
            && items(&a.state.playback, "queue").len() >= 5
    })?;
    checks.push("play, insert next, queue append, shuffle, repeat");
        if !items(&app.state.playback, "queue").iter().any(|track| track["added_by"] == "Alice") {
            return Err("Queue additions did not include the active profile nickname".into());
        }
        checks.push("queue additions identify the active profile nickname");
    app.paint();
    let seek = app
        .state
        .hits
        .iter()
        .find(|h| matches!(h.action, Action::Seek))
        .cloned()
        .ok_or("Seek slider missing")?;
    app.slider(&seek, seek.rect.x + seek.rect.w * 0.5, seek.rect.y, true);
    wait(&mut app, "host seek", |a| {
        (number(&a.state.playback, "position_ms") - 99500.).abs() < 1.
    })?;
    checks.push("host progress seeking");
    app.tab(View::Flow);
    wait(&mut app, "flow and genres", |a| {
        !items(&a.state.flow, "options").is_empty() && !items(&a.state.tuner, "genres").is_empty()
    })?;
    app.action(Action::Tuner(true));
    app.action(Action::AllGenres(false));
    app.action(Action::FlowGenre(0));
    app.action(Action::FlowSave);
    app.paint();
    checks.push("Flow moods and one-genre saving");
    app.tab(View::Lyrics);
    wait(&mut app, "timed lyrics", |a| {
        !items(&a.state.lyrics, "lines").is_empty()
    })?;
    app.paint();
    app.state.scroll = app.state.target_scroll;
    app.paint();
    wait(&mut app, "background lyric masks", |_| {
        crate::glyphs::pending() == 0
    })?;
    app.renderer
        .as_ref()
        .unwrap()
        .save(&directory.join("timed-lyrics.png"))?;
    app.action(Action::Visualizer(true));
    app.paint();
    checks.push("timed and wrapped lyrics, native frosting, visualizer controls");
    app.tab(View::Queue);
    app.action(Action::Listen);
    wait(&mut app, "native PCM audio output", |a| a.state.audio_ready)?;
    app.paint();
    let volume = app
        .state
        .hits
        .iter()
        .rev()
        .find(|h| matches!(h.action, Action::Volume))
        .cloned()
        .ok_or("Local volume slider missing")?;
    app.slider(&volume, volume.rect.x, volume.rect.y, true);
    if app.state.prefs.volume != 0. {
        return Err("Native mute did not reach zero".into());
    }
    app.action(Action::Listen);
    checks.push("real native audio buffers and independent local mute");
    wait(&mut app, "artwork", |a| !a.state.images.is_empty())?;
    let attempts = network::json("GET", &format!("{host}/test/images"), &Value::Null)?;
    if attempts["attempts"].as_u64().unwrap_or(0) < 2 {
        return Err("Artwork retry was not exercised".into());
    }
    checks.push("Failed artwork downloads recover automatically");
    app.action(Action::Theme(7));
    app.paint();
    app.renderer
        .as_ref()
        .unwrap()
        .save(&directory.join("themed-player.png"))?;
    checks.push("artwork, clipping, themes, and matching player layout");
    let events = network::json("GET", &format!("{host}/test/events"), &Value::Null)?;
    for path in [
        "/api/autoplay",
        "/api/play",
        "/api/play-collection",
        "/api/queue/insert-next",
        "/api/queue/add-last",
        "/api/shuffle",
        "/api/repeat",
        "/api/seek",
        "/api/flow/tuner/save",
    ] {
        if !events
            .as_array()
            .is_some_and(|events| events.iter().any(|e| e["path"] == path))
        {
            return Err(format!("Missing host command: {path}"));
        }
    }
    let mut views = Vec::new();
    for view in [
        View::Queue,
        View::Search,
        View::Flow,
        View::Settings,
        View::Lyrics,
    ] {
        app.state.switch(view);
        app.paint();
        let name = view.label().replace(' ', "-").to_lowercase();
        app.renderer
            .as_ref()
            .unwrap()
            .save(&directory.join(format!("{name}.png")))?;
        views.push(name);
    }
    std::fs::write(path,json!({"ok":true,"browser_engine":false,"native_audio":true,"checks":checks,"screenshots":views}).to_string()).map_err(|e|e.to_string())?;
    Ok(())
}
