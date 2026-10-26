#[path = "client_test.rs"]
mod client_test;
use crate::{
    model::*,
    network::{self, Event, Worker},
    render::Renderer,
    sound::Sound,
    ui,
};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    ptr::{null, null_mut},
    sync::{atomic::Ordering, Arc},
    time::{Duration, Instant},
};
use windows_sys::Win32::{
    Foundation::*,
    Graphics::Gdi::*,
    System::LibraryLoader::GetModuleHandleW,
    UI::{Controls::*, HiDpi::*, Input::KeyboardAndMouse::*, WindowsAndMessaging::*},
};

struct App {
    state: State,
    renderer: Option<Renderer>,
    worker: Worker,
    sound: Sound,
    generation: u64,
    requests: HashMap<String, u64>,
    pressed: Option<(Hit, f32, f32)>,
    drag_queue: Option<usize>,
    minimized: bool,
    foreground: bool,
    frame: Instant,
    timer_ms: u32,
    last_progress: Instant,
    hwnd: HWND,
}
impl App {
    fn request(&mut self, key: &str, path: &str) {
        self.generation += 1;
        self.requests.insert(key.into(), self.generation);
        self.state.loading.insert(key.into());
        if let Err(e) = self.worker.request(key, self.generation, path) {
            self.state.loading.remove(key);
            self.state.message(e);
        }
    }
    fn tab(&mut self, view: View) {
        if view == View::Lyrics && self.state.view == View::Lyrics {
            let v = self.state.return_view;
            self.state.switch(v);
        } else {
            self.state.switch(view);
        }
        self.state.focus = None;
        self.worker.lyrics.store(
            self.state.view == View::Lyrics && !self.minimized,
            Ordering::Relaxed,
        );
        match self.state.view {
            View::Discover if self.state.discover.is_null() => {
                self.request("discover", "/api/discover")
            }
            View::Flow => {
                self.request(
                    "flow",
                    &format!("/api/flow?config={}", encode(&self.state.flow_id)),
                );
                self.request(
                    "tuner",
                    &format!("/api/flow/tuner?id={}", encode(&self.state.flow_id)),
                );
            }
            View::Lyrics if self.state.lyrics.is_null() => self.request("lyrics", "/api/lyrics"),
            _ => {}
        }
    }
    fn post(&mut self, path: &str, mut body: Value) {
        if validate_nickname(&self.state.prefs.nickname).is_err() {
            self.state.message("Enter your nickname before continuing");
            return;
        }
        if body.is_null() { body = json!({}); }
        if body.is_object() {
            body["added_by"] = json!(self.state.prefs.nickname);
        }
        if let Err(e) = self.worker.action(path, body) {
            self.state.message(e);
        }
    }
    fn action(&mut self, a: Action) {
        if validate_nickname(&self.state.prefs.nickname).is_err()
            && !matches!(a, Action::SaveNickname | Action::Focus("host_nickname")) { return; }
        match a {
            Action::SaveNickname => match validate_nickname(&self.state.host_nickname) {
                Ok(name) => {
                    self.state.prefs.nickname = name;
                    self.state.prefs.save();
                    self.state.host_nickname.clear();
                    self.state.focus = None;
                    self.state.toast.clear();
                }
                Err(error) => self.state.message(error),
            },
            Action::Tab(v) => self.tab(v),
            Action::Listen => {
                self.state.listening = !self.state.listening;
                if self.state.listening {
                    self.state.audio_error.clear();
                    self.sound.start(self.worker.host().into());
                } else {
                    self.sound.stop();
                }
            }
            Action::Transport(path) => self.post(&format!("/api/{path}"), Value::Null),
            Action::Shuffle => self.post(
                "/api/shuffle",
                json!({"enabled":!self.state.playback["shuffle"].as_bool().unwrap_or(false)}),
            ),
            Action::Repeat => self.post(
                "/api/repeat",
                json!({"mode":(number(&self.state.playback,"repeat")as u32+1)%3}),
            ),
            Action::Autoplay => self.post(
                "/api/autoplay",
                json!({"enabled":!self.state.playback["autoplay"].as_bool().unwrap_or(false)}),
            ),
            Action::Theme(id) => {
                self.state.prefs.theme = id;
                self.state.prefs.save();
            }
            Action::MoreThemes => self.state.prefs.expanded = !self.state.prefs.expanded,
            Action::Language => unsafe {
                let menu = CreatePopupMenu();
                AppendMenuW(
                    menu,
                    MF_STRING
                        | if !self.state.prefs.turkish {
                            MF_CHECKED
                        } else {
                            0
                        },
                    1,
                    wide("English").as_ptr(),
                );
                AppendMenuW(
                    menu,
                    MF_STRING
                        | if self.state.prefs.turkish {
                            MF_CHECKED
                        } else {
                            0
                        },
                    2,
                    wide("Türkçe").as_ptr(),
                );
                let mut p = POINT::default();
                GetCursorPos(&mut p);
                let selected = TrackPopupMenu(
                    menu,
                    TPM_RETURNCMD | TPM_RIGHTBUTTON,
                    p.x,
                    p.y,
                    0,
                    self.hwnd,
                    null(),
                );
                if selected > 0 {
                    self.state.prefs.turkish = selected == 2;
                    self.state.prefs.save();
                }
                DestroyMenu(menu);
            },
            Action::Search => {
                if !self.state.search_text.trim().is_empty() {
                    self.request(
                        "search",
                        &format!("/api/search?q={}", encode(self.state.search_text.trim())),
                    );
                    self.state.scroll = 0.;
                    self.state.target_scroll = 0.;
                }
            }
            Action::Category(key) => {
                self.state.category = key;
                self.state.scroll = 0.;
                self.state.target_scroll = 0.;
            }
            Action::Open(item) => {
                let kind = text(&item, "kind");
                if matches!(kind.as_str(), "track" | "episode") {
                    self.post("/api/play", item);
                } else {
                    self.state.modal = Some(json!({}));
                    self.state.modal_item = item.clone();
                    self.state.modal_scroll = 0.;
                    self.request(
                        "collection",
                        &format!(
                            "/api/collection?kind={}&id={}",
                            encode(&kind),
                            encode(&text(&item, "id"))
                        ),
                    );
                }
            }
            Action::CloseModal => {
                self.state.modal = None;
                self.state.modal_scroll = 0.;
            }
            Action::Play(item) => self.post("/api/play", item),
            Action::Insert(item) => self.post("/api/queue/insert-next", item),
            Action::Append(item) => self.post("/api/queue/add-last", item),
            Action::Jump(index) => self.post("/api/queue/play-index", json!({"index":index})),
            Action::Clear => self.post("/api/queue/clear", Value::Null),
            Action::PlayCollection => {
                let tracks = self
                    .state
                    .modal
                    .as_ref()
                    .map(|d| d["tracks"].clone())
                    .unwrap_or(Value::Null);
                self.post("/api/play-collection", json!({"tracks":tracks}));
            }
            Action::FlowSelect(id) => {
                self.state.flow_id = id;
                self.post("/api/flow/start", json!({"config":self.state.flow_id}));
                self.request(
                    "flow",
                    &format!("/api/flow?config={}", encode(&self.state.flow_id)),
                );
                self.request(
                    "tuner",
                    &format!("/api/flow/tuner?id={}", encode(&self.state.flow_id)),
                );
            }
            Action::StartFlow => {
                self.post("/api/flow/start", json!({"config":self.state.flow_id}));
                self.request(
                    "flow",
                    &format!("/api/flow?config={}", encode(&self.state.flow_id)),
                );
            }
            Action::Tuner(expanded) => {
                self.state.tuner["expanded"] = json!(expanded);
                if !expanded && self.state.focus == Some("genres") {
                    self.state.focus = None;
                }
            }
            Action::FlowGenre(index) => {
                if let Some(g) = self.state.tuner["genres"].get_mut(index) {
                    g["isEnabled"] = json!(!g["isEnabled"].as_bool().unwrap_or(false));
                }
            }
            Action::AllGenres(enabled) => {
                if let Some(genres) = self.state.tuner["genres"].as_array_mut() {
                    for g in genres {
                        g["isEnabled"] = json!(enabled)
                    }
                }
            }
            Action::FlowSave => {
                let genres = items(&self.state.tuner, "genres");
                if !genres
                    .iter()
                    .any(|g| g["isEnabled"].as_bool().unwrap_or(false))
                {
                    self.state.message("Select at least one genre!");
                } else {
                    let edits: Vec<_> = genres
                        .iter()
                        .filter(|g| g["isEnabled"] != g["originalEnabled"])
                        .map(|g| json!({"clusterId":g["id"],"isEnabled":g["isEnabled"]}))
                        .collect();
                    let genres: Vec<_> = genres
                        .iter()
                        .map(|g| json!({"id":g["id"],"isEnabled":g["isEnabled"]}))
                        .collect();
                    self.post(
                        "/api/flow/tuner/save",
                        json!({"flow_id":self.state.flow_id,"genres":genres,"edits":edits}),
                    );
                }
            }
            Action::Visualizer(bars) => {
                self.state.prefs.bars = bars;
                self.state.prefs.save();
            }
            Action::CloseLyrics => {
                let v = self.state.return_view;
                self.tab(v);
            }
            Action::Focus(kind) => {
                self.state.focus = Some(kind);
                self.state.select_all = false;
            }
            Action::ConnectHost => match network::normalize_host(&self.state.host_text) {
                Ok(host) => {
                    self.state.active_host_profile = None;
                    self.change_host(host);
                }
                Err(error) => self.state.message(error),
            },
            Action::SaveHostProfile => {
                match validate_nickname(&self.state.host_nickname) {
                    Err(error) => self.state.message(error),
                    Ok(nickname) => {
                    match network::normalize_host(&self.state.host_text) {
                        Ok(host) => {
                            if let Some(profile) = self.state.host_profiles.iter_mut()
                                .find(|(name, _)| name.eq_ignore_ascii_case(&nickname)) {
                                *profile = (nickname, host);
                            } else {
                                self.state.host_profiles.push((nickname, host));
                            }
                            self.state.host_nickname.clear();
                            self.state.message("Temporary profile saved");
                        }
                        Err(error) => self.state.message(error),
                    }
                }
            }
            }
            Action::ConnectHostProfile(index) => {
                if let Some((nickname, host)) = self.state.host_profiles.get(index).cloned() {
                    self.change_host(host);
                        self.state.prefs.nickname = nickname.clone();
                        self.state.prefs.save();
                    self.state.active_host_profile = Some(nickname);
                }
            }
            Action::RemoveHostProfile(index) => {
                if index < self.state.host_profiles.len() {
                    if self.state.active_host_profile.as_deref() == Some(self.state.host_profiles[index].0.as_str()) {
                        self.state.active_host_profile = None;
                    }
                    self.state.host_profiles.remove(index);
                }
            }
            Action::Seek | Action::Volume | Action::Opacity | Action::Blur | Action::Scroll => {}
        }
    }
    fn slider(&mut self, hit: &Hit, x: f32, y: f32, commit: bool) {
        let f = if matches!(hit.action, Action::Scroll) {
            ((y - hit.rect.y) / hit.rect.h).clamp(0., 1.)
        } else {
            ((x - hit.rect.x) / hit.rect.w).clamp(0., 1.)
        };
        match hit.action {
            Action::Seek => {
                let pos = number(&self.state.playback, "duration_ms") * f;
                self.state.playback["position_ms"] = json!(pos);
                self.state.updated = Instant::now();
                if commit {
                    self.post("/api/seek", json!({"position_ms":pos.round()as u64}));
                }
            }
            Action::Volume => {
                self.state.prefs.volume = f * 100.;
                self.sound.volume(f * 100.);
                if commit {
                    self.state.prefs.save();
                }
            }
            Action::Opacity => {
                self.state.prefs.opacity = f * 100.;
                if commit {
                    self.state.prefs.save();
                }
            }
            Action::Blur => {
                self.state.prefs.blur = f * 100.;
                if commit {
                    self.state.prefs.save();
                }
            }
            Action::Scroll => {
                self.state.scroll = self.state.content_height * f;
                self.state.target_scroll = self.state.scroll;
            }
            _ => {}
        }
    }
    fn events(&mut self) -> bool {
        let mut dirty = false;
        while let Ok(event) = self.worker.events.try_recv() {
            match event {
                Event::State(result) => match result {
                    Ok(v) => {
                        dirty |= self.state.playback != v || !self.state.online;
                        self.state.online = true;
                        self.state.error.clear();
                        let was_playing = self.state.playing();
                        self.state.playback = v;
                        if was_playing != self.state.playing() {
                            self.state.transport_at = Instant::now();
                        }
                        self.state.updated = Instant::now();
                        let id = text(&self.state.playback, "id");
                        if id != self.state.last_track {
                            self.state.last_track = id;
                            self.state.lyrics = Value::Null;
                            self.state.lyric_focus = None;
                            self.state.scroll = 0.;
                            self.state.target_scroll = 0.;
                            if self.state.view == View::Lyrics {
                                self.request("lyrics", "/api/lyrics");
                            }
                        }
                    }
                    Err(e) => {
                        dirty |= self.state.online;
                        self.state.online = false;
                        self.state.error = e;
                    }
                },
                Event::Data(key, generation, result) => {
                    if !key.starts_with("action:") && self.requests.get(&key) != Some(&generation) {
                        continue;
                    }
                    self.state.loading.remove(&key);
                    dirty = true;
                    match result {
                        Ok(v) => match key.as_str() {
                            "appearance" => {
                                let themes: Vec<_> =
                                    items(&v, "themes").iter().map(Theme::from).collect();
                                if !themes.is_empty() {
                                    self.state.themes = themes;
                                }
                            }
                            "discover" => self.state.discover = v,
                            "flow" => self.state.flow = v,
                            "tuner" => {
                                self.state.tuner = v;
                                if let Some(genres) = self.state.tuner["genres"].as_array_mut() {
                                    for g in genres {
                                        g["originalEnabled"] = g["isEnabled"].clone();
                                        if g["id"].is_null() {
                                            g["id"] = g["clusterId"].clone();
                                        }
                                    }
                                }
                            }
                            "search" => self.state.search = v,
                            "lyrics" => {
                                if v["id"].as_str() == Some(&self.state.last_track) {
                                    self.state.lyrics = v;
                                    self.state.lyric_focus = None;
                                }
                            }
                            "collection" => {
                                if self.state.modal.is_some() {
                                    self.state.modal = Some(v);
                                }
                            }
                            action if action.starts_with("action:") => {
                                if v["error"].is_string() {
                                    self.state.message(text(&v, "error"));
                                } else if action.ends_with("/tuner/save") {
                                    if let Some(genres) = self.state.tuner["genres"].as_array_mut()
                                    {
                                        for g in genres {
                                            g["originalEnabled"] = g["isEnabled"].clone();
                                        }
                                    }
                                    self.state.message("Flow preferences saved");
                                } else if action.contains("/queue/") {
                                    self.state.message("Queue updated");
                                }
                            }
                            _ => {}
                        },
                        Err(e) => self.state.message(e),
                    }
                }
                Event::Image(url, result) => {
                    self.state.requested_images.remove(&url);
                    if let Ok(image) = result {
                        self.state.image_retry_at.remove(&url);
                        if self.state.images.len() > 96
                            || self
                                .state
                                .images
                                .values()
                                .map(|v| v.rgba.len())
                                .sum::<usize>()
                                > 24 * 1024 * 1024
                        {
                            let art = text(&self.state.playback, "artwork");
                            self.state.images.retain(|key, _| *key == art);
                        }
                        self.state.images.insert(url, image);
                        dirty = true;
                    } else {
                        self.state.image_failed(url, Instant::now());
                        dirty = true;
                    }
                }
                Event::Spectrum(levels) => {
                    self.state
                        .spectrum_history
                        .push_back((Instant::now(), levels));
                    if self.state.spectrum_history.len() > 64 {
                        self.state.spectrum_history.pop_front();
                    }
                    self.state.sample_spectrum(Instant::now());
                    dirty |= self.state.view == View::Lyrics;
                }
            }
        }
        let ready = self.sound.ready.load(Ordering::Relaxed);
        dirty |= ready != self.state.audio_ready;
        self.state.audio_ready = ready;
        self.state.audio_delay = self.sound.delay.load(Ordering::Relaxed) as f32;
        self.worker.lyrics.store(
            self.foreground && !self.minimized && (self.state.view == View::Lyrics),
            Ordering::Relaxed,
        );
        if self.state.listening {
            self.state.audio_error = self.sound.error.lock().unwrap().clone();
        }
        dirty
    }
    fn frame(&mut self) -> bool {
        let now = Instant::now();
        self.state.sample_spectrum(now);
        let dt = now.duration_since(self.frame).as_secs_f32().clamp(0., 0.1);
        self.frame = now;
        let mut dirty = self
            .renderer
            .as_mut()
            .is_some_and(|r| r.failed.get() || r.glyph_changed());
        dirty |= self
            .state
            .image_retry_at
            .iter()
            .any(|(url, (at, _))| now >= *at && !self.state.requested_images.contains(url));
        let max = self.state.content_height;
        self.state.target_scroll = self.state.target_scroll.clamp(0., max);
        if (self.state.scroll - self.state.target_scroll).abs() > 0.25 {
            self.state.scroll +=
                (self.state.target_scroll - self.state.scroll) * (1. - (-dt / 0.075).exp());
            dirty = true;
        } else {
            self.state.scroll = self.state.target_scroll;
        }
        if !self.minimized
            && self.foreground
            && (self.state.lyric_animation()
                || (self.state.playing()
                    && now.duration_since(self.last_progress).as_millis() >= 250))
        {
            dirty = true;
            self.last_progress = now;
        }
        dirty
    }
    fn paint(&mut self) {
        if self.minimized {
            return;
        }
        if let Some(mut r) = self.renderer.take() {
            self.state.hits.clear();
            let urls = ui::draw(&mut r, &mut self.state);
            self.worker.refresh_images(&mut self.state, urls);
            if r.failed.get() && !self.hwnd.is_null() {
                unsafe {
                    let mut bounds = RECT::default();
                    GetClientRect(self.hwnd, &mut bounds);
                    self.renderer = Some(
                        Renderer::window(
                            self.hwnd,
                            bounds.right as u32,
                            bounds.bottom as u32,
                            GetDpiForWindow(self.hwnd) as f32,
                        )
                        .unwrap_or(r),
                    );
                }
            } else {
                self.renderer = Some(r);
            }
        }
    }
    fn connect_prompt(&mut self) {
        unsafe {
            if let Some(host) = connect_dialog(Some(self.hwnd), &self.state.prefs.host) {
                self.change_host(host);
            }
        }
    }
    fn change_host(&mut self, host: String) {
        self.sound.stop();
        self.state.listening = false;
        self.state.audio_ready = false;
        self.state.audio_error.clear();
        self.state.audio_delay = 0.;
        self.state.prefs.host = host.clone();
        self.state.host_text = host.clone();
        self.state.prefs.save();
        self.worker = Worker::start(host);
        self.requests.clear();
        self.state.loading.clear();
        self.state.requested_images.clear();
        self.state.image_retry_at.clear();
        self.state.online = false;
        self.state.error.clear();
        self.state.playback = json!({});
        self.state.discover = Value::Null;
        self.state.flow = Value::Null;
        self.state.tuner = Value::Null;
        self.state.lyrics = Value::Null;
        self.state.search = Value::Null;
        self.state.modal = None;
        self.state.last_track.clear();
        self.state.lyric_focus = None;
        self.state.focus = None;
        self.state.levels = [0.; 32];
        self.state.spectrum_history.clear();
        self.request("appearance", "/api/appearance");
        let view = self.state.view;
        self.tab(view);
    }
    fn edit(&mut self, ch: char) {
        let target = if self.state.focus == Some("search") {
            &mut self.state.search_text
        } else if self.state.focus == Some("genres") {
            &mut self.state.genre_filter
        } else if self.state.focus == Some("host") {
            &mut self.state.host_text
        } else if self.state.focus == Some("host_nickname") {
            &mut self.state.host_nickname
        } else {
            return;
        };
        if self.state.select_all {
            target.clear();
            self.state.select_all = false;
        }
        if ch == '\u{8}' {
            target.pop();
        } else if !ch.is_control() {
            target.push(ch);
        }
    }
}

unsafe extern "system" fn procedure(hwnd: HWND, msg: u32, wp: usize, lp: isize) -> isize {
    if msg == WM_NCCREATE {
        let cs = &*(lp as *const CREATESTRUCTW);
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, cs.lpCreateParams as isize);
    }
    let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut App;
    if ptr.is_null() {
        return DefWindowProcW(hwnd, msg, wp, lp);
    }
    let app = &mut *ptr;
    match msg {
        WM_ACTIVATE => {
            app.foreground = (wp & 0xffff) as u32 != WA_INACTIVE;
            app.worker
                .visible
                .store(app.foreground && !app.minimized, Ordering::Relaxed);
            app.worker.lyrics.store(
                app.foreground && !app.minimized && app.state.view == View::Lyrics,
                Ordering::Relaxed,
            );
            if app.foreground {
                InvalidateRect(hwnd, null(), 0);
                app.timer_ms = 10;
                SetTimer(hwnd, 1, 10, None);
            }
            0
        }
        WM_ERASEBKGND => 1,
        WM_PAINT => {
            let mut ps = PAINTSTRUCT::default();
            BeginPaint(hwnd, &mut ps);
            app.paint();
            EndPaint(hwnd, &ps);
            0
        }
        WM_SIZE => {
            app.minimized = wp as u32 == SIZE_MINIMIZED;
            app.worker
                .visible
                .store(app.foreground && !app.minimized, Ordering::Relaxed);
            app.worker.lyrics.store(
                app.foreground && !app.minimized && app.state.view == View::Lyrics,
                Ordering::Relaxed,
            );
            if !app.minimized {
                let mut r = RECT::default();
                GetClientRect(hwnd, &mut r);
                if let Some(renderer) = &mut app.renderer {
                    renderer.resize(
                        r.right as u32,
                        r.bottom as u32,
                        GetDpiForWindow(hwnd) as f32,
                    );
                }
                InvalidateRect(hwnd, null(), 0);
            }
            0
        }
        WM_DPICHANGED => {
            let r = &*(lp as *const RECT);
            SetWindowPos(
                hwnd,
                null_mut(),
                r.left,
                r.top,
                r.right - r.left,
                r.bottom - r.top,
                SWP_NOZORDER | SWP_NOACTIVATE,
            );
            0
        }
        WM_GETMINMAXINFO => {
            let m = &mut *(lp as *mut MINMAXINFO);
            m.ptMinTrackSize = POINT { x: 560, y: 420 };
            0
        }
        WM_TIMER => {
            let dirty = app.events() | if app.foreground { app.frame() } else { false };
            if dirty && !app.minimized && app.foreground {
                InvalidateRect(hwnd, null(), 0);
            }
            let interval = if app.minimized || !app.foreground {
                1000
            } else if app.state.lyric_animation()
                || (app.state.target_scroll - app.state.scroll).abs() > 0.25
            {
                10
            } else {
                100
            };
            if app.timer_ms != interval {
                app.timer_ms = interval;
                SetTimer(hwnd, 1, interval, None);
            }
            0
        }
        WM_MOUSEMOVE => {
            let scale = GetDpiForWindow(hwnd) as f32 / 96.;
            let x = (lp as u16 as i16) as f32 / scale;
            let y = ((lp >> 16) as u16 as i16) as f32 / scale;
            let old = app.state.mouse;
            app.state.mouse = (x, y);
            if let Some((hit, sx, sy)) = app.pressed.clone() {
                if hit.slider {
                    app.slider(&hit, x, y, false);
                } else if let Action::Jump(index) = hit.action {
                    if index > number(&app.state.playback, "queue_index") as usize
                        && (x - sx).abs() + (y - sy).abs() > 6.
                    {
                        app.drag_queue = Some(index);
                    }
                }
            }
            if old != (x, y) {
                InvalidateRect(hwnd, null(), 0);
            }
            0
        }
        WM_LBUTTONDOWN => {
            SetFocus(hwnd);
            let (x, y) = app.state.mouse;
            if let Some(hit) = app
                .state
                .hits
                .iter()
                .rev()
                .find(|h| h.rect.contains(x, y))
                .cloned()
            {
                app.pressed = Some((hit.clone(), x, y));
                SetCapture(hwnd);
                if hit.slider {
                    app.slider(&hit, x, y, false);
                }
            } else {
                app.state.focus = None;
            }
            InvalidateRect(hwnd, null(), 0);
            0
        }
        WM_LBUTTONUP => {
            ReleaseCapture();
            let (x, y) = app.state.mouse;
            if let Some((hit, _, _)) = app.pressed.take() {
                if hit.slider {
                    app.slider(&hit, x, y, true);
                } else if let Some(from) = app.drag_queue.take() {
                    if let Some(to) = app.state.hits.iter().rev().find_map(|hit| {
                        if hit.rect.contains(x, y) {
                            if let Action::Jump(i) = hit.action {
                                Some(i)
                            } else {
                                None
                            }
                        } else {
                            None
                        }
                    }) {
                        if to > number(&app.state.playback, "queue_index") as usize && from != to {
                            let expected: Vec<_> = items(&app.state.playback, "queue")
                                .iter()
                                .map(|t| text(t, "id"))
                                .collect();
                            app.post(
                                "/api/queue/move",
                                json!({"from":from,"to":to,"expected":expected}),
                            );
                        }
                    }
                } else if hit.rect.contains(x, y) {
                    app.action(hit.action);
                }
            }
            InvalidateRect(hwnd, null(), 0);
            0
        }
        WM_MOUSEWHEEL => {
            let delta = ((wp >> 16) as u16 as i16) as f32 / 120.;
            if app.state.mouse.1 >= 59. && app.state.mouse.1 < 110. {
                app.state.nav_scroll = (app.state.nav_scroll - delta * 80.).clamp(0., 300.);
            } else if app.state.modal.is_some() {
                app.state.modal_scroll =
                    (app.state.modal_scroll - delta * 90.).clamp(0., app.state.content_height);
            } else {
                app.state.target_scroll =
                    (app.state.target_scroll - delta * 90.).clamp(0., app.state.content_height);
                if app.state.view == View::Lyrics {
                    app.state.lyric_manual_until = Some(Instant::now() + Duration::from_secs(4));
                }
            }
            InvalidateRect(hwnd, null(), 0);
            0
        }
        WM_CHAR => {
            if let Some(ch) = char::from_u32(wp as u32) {
                if ch == '\r' && app.state.focus == Some("search") {
                    app.action(Action::Search);
                } else if ch == '\r' && app.state.focus == Some("host") {
                    app.action(Action::ConnectHost);
                } else if ch == '\r' && app.state.focus == Some("host_nickname") {
                app.action(if app.state.prefs.nickname.is_empty() { Action::SaveNickname } else { Action::SaveHostProfile });
                } else {
                    app.edit(ch);
                }
                InvalidateRect(hwnd, null(), 0);
            }
            0
        }
        WM_KEYDOWN => {
            if wp == VK_ESCAPE as usize {
                if app.state.modal.is_some() {
                    app.state.modal = None;
                } else if app.state.view == View::Lyrics {
                    app.action(Action::CloseLyrics);
                } else {
                    app.state.focus = None;
                }
            } else if wp == VK_SPACE as usize && app.state.focus.is_none() {
                app.action(Action::Transport("toggle"));
            } else if wp == 0x41 && GetKeyState(VK_CONTROL as i32) < 0 {
                app.state.select_all = true;
            } else if wp == 0x56 && GetKeyState(VK_CONTROL as i32) < 0 {
                if let Some(text) = clipboard(hwnd) {
                    for ch in text.chars() {
                        app.edit(ch);
                    }
                }
            } else if wp == VK_F5 as usize {
                app.connect_prompt();
            } else if wp == VK_NEXT as usize {
                app.state.target_scroll =
                    (app.state.target_scroll + 500.).min(app.state.content_height);
            } else if wp == VK_PRIOR as usize {
                app.state.target_scroll = (app.state.target_scroll - 500.).max(0.);
            }
            InvalidateRect(hwnd, null(), 0);
            0
        }
        WM_SYSCOMMAND if wp == 0x100 => {
            app.connect_prompt();
            0
        }
        WM_CLOSE => {
            app.state.prefs.save();
            app.sound.stop();
            DestroyWindow(hwnd);
            0
        }
        WM_DESTROY => {
            PostQuitMessage(0);
            0
        }
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}

unsafe fn clipboard(hwnd: HWND) -> Option<String> {
    use windows_sys::Win32::System::DataExchange::*;
    use windows_sys::Win32::System::Memory::*;
    if OpenClipboard(hwnd) == 0 {
        return None;
    }
    let h = GetClipboardData(13);
    let out = if !h.is_null() {
        let p = GlobalLock(h).cast::<u16>();
        if !p.is_null() {
            let mut n = 0;
            while n < 10000 && *p.add(n) != 0 {
                n += 1
            }
            let s = String::from_utf16_lossy(std::slice::from_raw_parts(p, n));
            GlobalUnlock(h);
            Some(s)
        } else {
            None
        }
    } else {
        None
    };
    CloseClipboard();
    out
}

struct Prompt {
    edit: HWND,
    result: Option<String>,
    done: bool,
}
unsafe extern "system" fn prompt_proc(hwnd: HWND, msg: u32, wp: usize, lp: isize) -> isize {
    if msg == WM_NCCREATE {
        let cs = &*(lp as *const CREATESTRUCTW);
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, cs.lpCreateParams as isize);
    }
    let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Prompt;
    if ptr.is_null() {
        return DefWindowProcW(hwnd, msg, wp, lp);
    }
    let p = &mut *ptr;
    match msg {
        WM_COMMAND if wp & 0xffff == 1 => {
            let mut s = [0u16; 1024];
            let n = GetWindowTextW(p.edit, s.as_mut_ptr(), 1024);
            let value = String::from_utf16_lossy(&s[..n as usize]);
            match network::normalize_host(&value) {
                Ok(host) => {
                    p.result = Some(host);
                    p.done = true;
                }
                Err(e) => {
                    MessageBoxW(
                        hwnd,
                        wide(&e).as_ptr(),
                        wide("Host address").as_ptr(),
                        MB_ICONINFORMATION,
                    );
                }
            }
            0
        }
        WM_COMMAND if wp & 0xffff == 2 => {
            p.done = true;
            0
        }
        WM_CLOSE => {
            p.done = true;
            0
        }
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}
unsafe fn connect_dialog(parent: Option<HWND>, default: &str) -> Option<String> {
    let instance = GetModuleHandleW(null());
    let class = wide("DeezNatsJamConnectPrompt");
    RegisterClassW(&WNDCLASSW {
        lpfnWndProc: Some(prompt_proc),
        hInstance: instance,
        lpszClassName: class.as_ptr(),
        hbrBackground: (COLOR_WINDOW + 1) as HBRUSH,
        hCursor: LoadCursorW(null_mut(), IDC_ARROW),
        ..Default::default()
    });
    let mut p = Box::new(Prompt {
        edit: null_mut(),
        result: None,
        done: false,
    });
    let hwnd = CreateWindowExW(
        WS_EX_DLGMODALFRAME,
        class.as_ptr(),
        wide("Connect to DeezNats").as_ptr(),
        WS_CAPTION | WS_SYSMENU | WS_POPUP,
        CW_USEDEFAULT,
        CW_USEDEFAULT,
        490,
        190,
        parent.unwrap_or(null_mut()),
        null_mut(),
        instance,
        p.as_mut() as *mut _ as *const _,
    );
    CreateWindowExW(
        0,
        wide("STATIC").as_ptr(),
        wide("Host address").as_ptr(),
        WS_CHILD | WS_VISIBLE,
        20,
        18,
        440,
        22,
        hwnd,
        null_mut(),
        instance,
        null(),
    );
    p.edit = CreateWindowExW(
        WS_EX_CLIENTEDGE,
        wide("EDIT").as_ptr(),
        wide(default).as_ptr(),
        WS_CHILD | WS_VISIBLE | WS_TABSTOP | ES_AUTOHSCROLL as u32,
        20,
        46,
        440,
        28,
        hwnd,
        100isize as HMENU,
        instance,
        null(),
    );
    CreateWindowExW(
        0,
        wide("BUTTON").as_ptr(),
        wide("Connect").as_ptr(),
        WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_DEFPUSHBUTTON as u32,
        270,
        92,
        90,
        30,
        hwnd,
        1isize as HMENU,
        instance,
        null(),
    );
    CreateWindowExW(
        0,
        wide("BUTTON").as_ptr(),
        wide("Cancel").as_ptr(),
        WS_CHILD | WS_VISIBLE | WS_TABSTOP,
        370,
        92,
        90,
        30,
        hwnd,
        2isize as HMENU,
        instance,
        null(),
    );
    if let Some(parent) = parent {
        EnableWindow(parent, 0);
    }
    ShowWindow(hwnd, SW_SHOWNORMAL);
    SetFocus(p.edit);
    SendMessageW(p.edit, EM_SETSEL, 0, -1);
    let mut msg = MSG::default();
    while !p.done && GetMessageW(&mut msg, null_mut(), 0, 0) > 0 {
        if IsDialogMessageW(hwnd, &msg) == 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
    DestroyWindow(hwnd);
    if let Some(parent) = parent {
        EnableWindow(parent, 1);
        SetForegroundWindow(parent);
    }
    p.result.take()
}

pub fn run() -> Result<(), String> {
    unsafe {
        let _ = windows::Win32::System::Com::CoInitializeEx(
            None,
            windows::Win32::System::Com::COINIT_APARTMENTTHREADED,
        );
        SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }
    let args: Vec<_> = std::env::args().collect();
    let arg = |name: &str| {
        args.iter()
            .position(|a| a == name)
            .and_then(|i| args.get(i + 1))
            .cloned()
    };
    if let Some(url) = arg("--probe-artwork") {
        return client_test::probe_artwork(
            &url,
            &arg("--report").unwrap_or_else(|| ".local/artwork-probe.json".into()),
        );
    }
    if let Some(host) = arg("--self-test-host") {
        return client_test::run(
            &host,
            &arg("--report").unwrap_or_else(|| ".local/native-client-test/report.json".into()),
        );
    }
    if let Some(directory) = arg("--preview-dir") {
        return preview(&directory, arg("--fixture").as_deref());
    }
    let smoke = args.iter().any(|a| a == "--smoke-test");
    let mut prefs = Prefs::load();
    if let Some(host) = arg("--host").or_else(|| {
        args.iter()
            .skip(1)
            .find(|a| a.starts_with("http://") || a.starts_with("https://"))
            .cloned()
    }) {
        prefs.host = network::normalize_host(&host)?;
    } else if !smoke && !Prefs::path().is_some_and(|p| p.exists()) {
        let Some(host) = (unsafe { connect_dialog(None, &prefs.host) }) else {
            return Ok(());
        };
        prefs.host = host;
        prefs.save();
    }
    let worker = Worker::start(prefs.host.clone());
    let sound = Sound::new(prefs.volume);
    let mut app = Box::new(App {
        state: State::new(prefs),
        renderer: None,
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
    });
    if validate_nickname(&app.state.prefs.nickname).is_err() { app.state.focus = Some("host_nickname"); }
    unsafe {
        let instance = GetModuleHandleW(null());
        let class = wide("DeezNatsJamNative");
        RegisterClassW(&WNDCLASSW {
            lpfnWndProc: Some(procedure),
            hInstance: instance,
            lpszClassName: class.as_ptr(),
            hCursor: LoadCursorW(null_mut(), IDC_ARROW),
            hIcon: LoadIconW(instance, 1usize as *const u16),
            ..Default::default()
        });
        let hwnd = CreateWindowExW(
            0,
            class.as_ptr(),
            wide("DeezNats Connect").as_ptr(),
            WS_OVERLAPPEDWINDOW,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            1060,
            920,
            null_mut(),
            null_mut(),
            instance,
            app.as_mut() as *mut _ as *const _,
        );
        if hwnd.is_null() {
            return Err("Could not create the client window".into());
        }
        app.hwnd = hwnd;
        let mut bounds = RECT::default();
        GetClientRect(hwnd, &mut bounds);
        app.renderer = Some(Renderer::window(
            hwnd,
            bounds.right as u32,
            bounds.bottom as u32,
            GetDpiForWindow(hwnd) as f32,
        )?);
        let menu = GetSystemMenu(hwnd, 0);
        AppendMenuW(menu, MF_SEPARATOR, 0, null());
        AppendMenuW(
            menu,
            MF_STRING,
            0x100,
            wide("Connect to host...\tF5").as_ptr(),
        );
        if smoke {
            app.paint();
            let result = crate::sound::smoke();
            DestroyWindow(hwnd);
            if let Some(report) = arg("--report") {
                std::fs::write(report,json!({"renderer":"Direct2D","browser_engine":false,"audio_device":result.is_ok(),"error":result.err()}).to_string()).map_err(|e|e.to_string())?;
            }
            return Ok(());
        }
        ShowWindow(hwnd, SW_SHOWNORMAL);
        SetTimer(hwnd, 1, 100, None);
        app.request("appearance", "/api/appearance");
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, null_mut(), 0, 0) > 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
    Ok(())
}

fn preview(directory: &str, fixture: Option<&str>) -> Result<(), String> {
    let dir = std::path::Path::new(directory);
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let fixture=fixture.map(std::fs::read).transpose().map_err(|e|e.to_string())?.map(|b|serde_json::from_slice::<Value>(&b)).transpose().map_err(|e|e.to_string())?.unwrap_or_else(||json!({"state":{"connected":true,"state":"paused","id":"preview","title":"Levels","artist":"Avicii","duration_ms":199000,"position_ms":75000,"queue":[],"autoplay":true},"lyrics":{"id":"preview","timed":false,"provider":"Preview","lines":[{"text":"Native lyrics follow the same layout."},{"text":"White text with frosting directly under the glyphs."}]}}));
    let mut state = State::new(Prefs { nickname: fixture["nickname"].as_str().unwrap_or("Preview").into(), ..Prefs::default() });
    state.online = true;
    state.playback = fixture["state"].clone();
    state.last_track = text(&state.playback, "id");
    for (key, dst) in [
        ("discover", &mut state.discover),
        ("flow", &mut state.flow),
        ("tuner", &mut state.tuner),
        ("lyrics", &mut state.lyrics),
        ("search", &mut state.search),
    ] {
        *dst = fixture[key].clone();
    }
    if let Some(themes) = fixture["themes"].as_array() {
        state.themes = themes.iter().map(Theme::from).collect();
    }
    let artwork = text(&state.playback, "artwork");
    if !artwork.is_empty() {
        if let Ok(bytes) = network::get(&artwork) {
            if let Ok(img) = image::load_from_memory(&bytes) {
                let img = img.thumbnail(600, 600).into_rgba8();
                let (w, h) = img.dimensions();
                state.images.insert(
                    artwork,
                    Arc::new(ImageData {
                        width: w,
                        height: h,
                        rgba: img.into_raw(),
                    }),
                );
            }
        }
    }
    let width = fixture["viewport"]["width"].as_u64().unwrap_or(1280).clamp(480, 3840) as u32;
    let height = fixture["viewport"]["height"].as_u64().unwrap_or(900).clamp(480, 2160) as u32;
    let mut r = unsafe { Renderer::offscreen(width, height)? };
    for (view, name) in [
        (View::Queue, "queue"),
        (View::Discover, "discover"),
        (View::Search, "search"),
        (View::Flow, "flow"),
        (View::Settings, "settings"),
        (View::Lyrics, "lyrics"),
    ] {
        state.switch(view);
        state.hits.clear();
        ui::draw(&mut r, &mut state);
        r.save(&dir.join(format!("{name}.png")))?;
    }
    std::fs::write(
        dir.join("report.json"),
        json!({"browser_engine":false,"renderer":"Direct2D/DirectWrite","views":6}).to_string(),
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}
