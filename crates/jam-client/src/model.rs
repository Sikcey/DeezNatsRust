use serde_json::{json, Value};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    path::PathBuf,
    sync::Arc,
    time::Instant,
};

pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}
pub fn text(v: &Value, key: &str) -> String {
    v[key]
        .as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| v[key].as_i64().map(|n| n.to_string()).unwrap_or_default())
}
pub fn number(v: &Value, key: &str) -> f32 {
    v[key].as_f64().unwrap_or(0.) as f32
}
pub fn duration(v: &Value) -> f32 {
    v["duration_ms"]
        .as_f64()
        .unwrap_or_else(|| v["duration"].as_f64().unwrap_or(0.) * 1000.) as f32
}
pub fn time(ms: f32) -> String {
    let s = (ms.max(0.) / 1000.) as u64;
    format!("{}:{:02}", s / 60, s % 60)
}
pub fn items<'a>(v: &'a Value, key: &str) -> &'a [Value] {
    v[key].as_array().map(Vec::as_slice).unwrap_or(&[])
}
pub fn encode(s: &str) -> String {
    s.bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) {
                (b as char).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect()
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}
impl Rect {
    pub fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        Self {
            x,
            y,
            w: w.max(0.),
            h: h.max(0.),
        }
    }
    pub fn contains(self, x: f32, y: f32) -> bool {
        x >= self.x && x <= self.x + self.w && y >= self.y && y <= self.y + self.h
    }
    pub fn inset(self, n: f32) -> Self {
        Self::new(self.x + n, self.y + n, self.w - 2. * n, self.h - 2. * n)
    }
    pub fn bottom(self) -> f32 {
        self.y + self.h
    }
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Color(pub f32, pub f32, pub f32, pub f32);
impl Color {
    pub fn hex(s: &str) -> Self {
        let n = u32::from_str_radix(s.trim_start_matches('#'), 16).unwrap_or(0);
        Self(
            ((n >> 16) & 255) as f32 / 255.,
            ((n >> 8) & 255) as f32 / 255.,
            (n & 255) as f32 / 255.,
            1.,
        )
    }
    pub fn alpha(self, a: f32) -> Self {
        Self(self.0, self.1, self.2, a.clamp(0., 1.))
    }
    pub fn mix(self, b: Self, t: f32) -> Self {
        Self(
            self.0 * (1. - t) + b.0 * t,
            self.1 * (1. - t) + b.1 * t,
            self.2 * (1. - t) + b.2 * t,
            self.3 * (1. - t) + b.3 * t,
        )
    }
}
pub const WHITE: Color = Color(1., 1., 1., 1.);
pub const BLACK: Color = Color(0., 0., 0., 1.);
#[derive(Clone)]
pub struct Theme {
    pub id: u32,
    pub name: String,
    pub bg: Color,
    pub panel: Color,
    pub raised: Color,
    pub border: Color,
    pub text: Color,
    pub muted: Color,
    pub accent: Color,
    pub accent_text: Color,
}
impl Theme {
    pub fn from(v: &Value) -> Self {
        let p = &v["palette"];
        let c = |k| Color::hex(p[k].as_str().unwrap_or("#eeeeee"));
        let a = c("Accent");
        Self {
            id: number(v, "id") as u32,
            name: text(v, "name"),
            bg: c("Background"),
            panel: c("Panel"),
            raised: c("Raised"),
            border: c("Border"),
            text: c("Text"),
            muted: c("Muted"),
            accent: a,
            accent_text: if a.0 * 0.2126 + a.1 * 0.7152 + a.2 * 0.0722 > 0.55 {
                Color::hex("#141418")
            } else {
                WHITE
            },
        }
    }
    pub fn fallback() -> Self {
        Self::from(
            &json!({"id":11,"name":"Blood Moon","palette":{"Background":"#190f19","Panel":"#241621","Raised":"#342131","Border":"#49304b","Text":"#f5ebfa","Muted":"#b2a6b4","Accent":"#d82448"}}),
        )
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum View {
    Queue,
    Discover,
    Flow,
    Search,
    Lyrics,
    Settings,
}
impl View {
    pub const TABS: [Self; 5] = [
        Self::Queue,
        Self::Discover,
        Self::Flow,
        Self::Search,
        Self::Lyrics,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::Queue => "Queue",
            Self::Discover => "Discover",
            Self::Flow => "Flow",
            Self::Search => "Search",
            Self::Lyrics => "Lyrics",
            Self::Settings => "Appearance",
        }
    }
    pub fn icon(self) -> char {
        match self {
            Self::Flow => '\u{e8d6}',
            Self::Queue => '\u{e8fd}',
            Self::Discover | Self::Search => '\u{e721}',
            Self::Lyrics => '\u{ec71}',
            Self::Settings => '\u{e713}',
        }
    }
}
#[derive(Clone)]
pub enum Action {
    Tab(View),
    Listen,
    Transport(&'static str),
    Shuffle,
    Repeat,
    Autoplay,
    Seek,
    Volume,
    Language,
    Theme(u32),
    Opacity,
    Blur,
    MoreThemes,
    Search,
    Category(String),
    Open(Value),
    CloseModal,
    Play(Value),
    Insert(Value),
    Append(Value),
    Jump(usize),
    Clear,
    FlowSelect(String),
    StartFlow,
    FlowGenre(usize),
    FlowSave,
    Visualizer(bool),
    CloseLyrics,
    PlayCollection,
    Focus(&'static str),
    ConnectHost,
    SaveHostProfile,
    SaveNickname,
    ConnectHostProfile(usize),
    RemoveHostProfile(usize),
    Scroll,
    Tuner(bool),
    AllGenres(bool),
}
#[derive(Clone)]
pub struct Hit {
    pub rect: Rect,
    pub action: Action,
    pub slider: bool,
}
pub struct ImageData {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}
pub type ImageCache = HashMap<String, Arc<ImageData>>;

pub fn validate_nickname(value: &str) -> Result<String, String> {
    let name = value.trim();
    if name.is_empty() || name.chars().count() > 64 || name.chars().any(char::is_control) {
        Err("Enter a nickname of 1–64 characters without control characters.".into())
    } else { Ok(name.to_owned()) }
}
pub struct Prefs {
    pub nickname: String,
    pub host: String,
    pub theme: u32,
    pub opacity: f32,
    pub blur: f32,
    pub volume: f32,
    pub turkish: bool,
    pub bars: bool,
    pub expanded: bool,
}
impl Default for Prefs {
    fn default() -> Self {
        Self {
            nickname: String::new(),
            host: "http://127.0.0.1:3131".into(),
            theme: 8,
            opacity: 40.,
            blur: 20.,
            volume: 80.,
            turkish: false,
            bars: false,
            expanded: false,
        }
    }
}
impl Prefs {
    pub fn path() -> Option<PathBuf> {
        if let Some(p) = std::env::var_os("DEEZNATS_JAM_PROFILE") {
            return Some(PathBuf::from(p).join("client.json"));
        }
        std::env::var_os("LOCALAPPDATA")
            .map(|p| PathBuf::from(p).join("DeezNatsJam").join("client.json"))
    }
    pub fn load() -> Self {
        let mut p = Self::default();
        if let Some(v) = Self::path()
            .and_then(|p| std::fs::read(p).ok())
            .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
        {
            p.host = v["host"].as_str().unwrap_or(&p.host).into();
            p.nickname = v["nickname"].as_str().and_then(|name| validate_nickname(name).ok()).unwrap_or_default();
            p.theme = v["theme"].as_u64().unwrap_or(p.theme as u64) as u32;
            for (k, dst) in [
                ("opacity", &mut p.opacity),
                ("blur", &mut p.blur),
                ("volume", &mut p.volume),
            ] {
                *dst = v[k].as_f64().unwrap_or(*dst as f64).clamp(0., 100.) as f32
            }
            p.turkish = v["turkish"].as_bool().unwrap_or(false);
            p.bars = v["bars"].as_bool().unwrap_or(false)
        }
        p
    }
    pub fn save(&self) {
        if let Some(p) = Self::path() {
            let _ = std::fs::create_dir_all(p.parent().unwrap());
            let _=std::fs::write(p,json!({"nickname":self.nickname,"host":self.host,"theme":self.theme,"opacity":self.opacity,"blur":self.blur,"volume":self.volume,"turkish":self.turkish,"bars":self.bars}).to_string());
        }
    }
}
pub struct State {
    pub prefs: Prefs,
    pub view: View,
    pub return_view: View,
    pub playback: Value,
    pub updated: Instant,
    pub transport_at: Instant,
    pub themes: Vec<Theme>,
    pub translations: Value,
    pub images: ImageCache,
    pub requested_images: HashSet<String>,
    pub image_retry_at: HashMap<String, (Instant, u32)>,
    pub discover: Value,
    pub flow: Value,
    pub tuner: Value,
    pub search: Value,
    pub lyrics: Value,
    pub modal: Option<Value>,
    pub modal_item: Value,
    pub search_text: String,
    pub host_text: String,
    pub host_nickname: String,
    pub host_profiles: Vec<(String, String)>,
    pub active_host_profile: Option<String>,
    pub category: String,
    pub flow_id: String,
    pub genre_filter: String,
    pub scroll: f32,
    pub target_scroll: f32,
    pub modal_scroll: f32,
    pub content_height: f32,
    pub hits: Vec<Hit>,
    pub mouse: (f32, f32),
    pub listening: bool,
    pub audio_ready: bool,
    pub audio_error: String,
    pub audio_delay: f32,
    pub levels: [f32; 32],
    pub spectrum_history: VecDeque<(Instant, [f32; 32])>,
    pub last_paint: Instant,
    pub paint_dt: f32,
    pub front: [f32; 32],
    pub rear: [f32; 32],
    pub toast: String,
    pub toast_at: Instant,
    pub loading: HashSet<String>,
    pub lyric_focus: Option<usize>,
    pub lyric_manual_until: Option<Instant>,
    pub last_track: String,
    pub online: bool,
    pub error: String,
    pub focus: Option<&'static str>,
    pub select_all: bool,
    pub nav_scroll: f32,
}
impl State {
    pub fn new(prefs: Prefs) -> Self {
        Self {
            host_text: prefs.host.clone(),
            host_nickname: String::new(),
            host_profiles: Vec::new(),
            active_host_profile: None,
            prefs,
            view: View::Queue,
            return_view: View::Queue,
            playback: json!({}),
            updated: Instant::now(),
            transport_at: Instant::now(),
            themes: serde_json::from_str::<Value>(include_str!("../resources/themes.json"))
                .ok()
                .map(|v| items(&v, "themes").iter().map(Theme::from).collect())
                .filter(|v: &Vec<Theme>| !v.is_empty())
                .unwrap_or_else(|| vec![Theme::fallback()]),
            translations: serde_json::from_str(include_str!(
                "../../app/src/connect/translations.json"
            ))
            .unwrap_or_default(),
            images: HashMap::new(),
            requested_images: HashSet::new(),
            image_retry_at: HashMap::new(),
            discover: Value::Null,
            flow: Value::Null,
            tuner: Value::Null,
            search: Value::Null,
            lyrics: Value::Null,
            modal: None,
            modal_item: Value::Null,
            search_text: String::new(),
            category: "all".into(),
            flow_id: "default".into(),
            genre_filter: String::new(),
            scroll: 0.,
            target_scroll: 0.,
            modal_scroll: 0.,
            content_height: 0.,
            hits: Vec::new(),
            mouse: (-1., -1.),
            listening: false,
            audio_ready: false,
            audio_error: String::new(),
            audio_delay: 0.,
            levels: [0.; 32],
            spectrum_history: VecDeque::new(),
            last_paint: Instant::now(),
            paint_dt: 0.033,
            front: [0.; 32],
            rear: [0.; 32],
            toast: String::new(),
            toast_at: Instant::now(),
            loading: HashSet::new(),
            lyric_focus: None,
            lyric_manual_until: None,
            last_track: String::new(),
            online: false,
            error: String::new(),
            focus: None,
            select_all: false,
            nav_scroll: 0.,
        }
    }
    pub fn theme(&self) -> Theme {
        self.themes
            .iter()
            .find(|t| t.id == self.prefs.theme)
            .unwrap_or(&self.themes[0])
            .clone()
    }
    pub fn tr(&self, s: &str) -> String {
        if self.prefs.turkish {
            self.translations[s].as_str().unwrap_or(s).to_owned()
        } else {
            self.translations
                .as_object()
                .and_then(|map| {
                    map.iter()
                        .find(|(_, value)| value.as_str() == Some(s))
                        .map(|(key, _)| key.clone())
                })
                .unwrap_or_else(|| s.to_owned())
        }
    }
    pub fn playing(&self) -> bool {
        self.playback["state"]
            .as_str()
            .is_some_and(|s| s.eq_ignore_ascii_case("playing"))
    }
    pub fn position(&self) -> f32 {
        let p = number(&self.playback, "position_ms")
            + if self.playing() {
                self.updated.elapsed().as_secs_f32() * 1000.
            } else {
                0.
            };
        if number(&self.playback, "duration_ms") > 0. {
            p.min(number(&self.playback, "duration_ms"))
        } else {
            p
        }
    }
    pub fn lyric_position(&self) -> f32 {
        (self.position()
            - if self.listening && self.audio_ready {
                if self.playing() {
                    self.audio_delay
                } else {
                    (self.audio_delay - self.transport_at.elapsed().as_secs_f32() * 1000.).max(0.)
                }
            } else {
                0.
            })
        .max(0.)
    }
    pub fn image_failed(&mut self, url: String, now: Instant) {
        self.requested_images.remove(&url);
        let attempts = (self.image_retry_at.get(&url).map(|(_, n)| *n).unwrap_or(0) + 1).min(6);
        self.image_retry_at.insert(
            url,
            (
                now + std::time::Duration::from_secs((1u64 << (attempts - 1)).min(30)),
                attempts,
            ),
        );
    }
    pub fn image_due(&self, url: &str, now: Instant) -> bool {
        !self.requested_images.contains(url)
            && self
                .image_retry_at
                .get(url)
                .is_none_or(|(at, _)| now >= *at)
    }
    pub fn sample_spectrum(&mut self, now: Instant) {
        self.spectrum_history
            .retain(|(at, _)| now.duration_since(*at).as_secs_f32() < 3.);
        let delay = if self.listening && self.audio_ready {
            self.audio_delay
        } else {
            0.
        };
        self.levels = self
            .spectrum_history
            .iter()
            .rev()
            .find(|(at, _)| now.duration_since(*at).as_secs_f32() * 1000. >= delay)
            .map(|(_, levels)| *levels)
            .unwrap_or([0.; 32]);
    }
    pub fn lyric_animation(&self) -> bool {
        self.view == View::Lyrics
            && (self.playing()
                || self.front.iter().any(|v| *v > 0.0005)
                || self.levels.iter().any(|v| *v > 0.0005))
    }
    pub fn message(&mut self, s: impl Into<String>) {
        self.toast = s.into();
        self.toast_at = Instant::now()
    }
    pub fn hit(&mut self, rect: Rect, action: Action, slider: bool) {
        self.hits.push(Hit {
            rect,
            action,
            slider,
        })
    }
    pub fn switch(&mut self, view: View) {
        if view == View::Lyrics && self.view != View::Lyrics {
            self.return_view = self.view;
        }
        self.view = view;
        self.scroll = 0.;
        self.target_scroll = 0.;
        self.lyric_focus = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn required_nickname_preserves_unicode_and_rejects_invalid_names() {
        assert_eq!(validate_nickname("  Çağrı 日本語 🎵  ").unwrap(), "Çağrı 日本語 🎵");
        assert!(validate_nickname("").is_err());
        assert!(validate_nickname("   ").is_err());
        assert!(validate_nickname("Alice\nBob").is_err());
        assert!(validate_nickname(&"W".repeat(65)).is_err());
        assert_eq!(validate_nickname(&"🎵".repeat(64)).unwrap().chars().count(), 64);
    }
    #[test]
    fn visualizer_uses_the_heard_audio_delay_and_ignores_local_volume() {
        let mut s = State::new(Prefs::default());
        let now = Instant::now();
        s.spectrum_history
            .push_back((now - std::time::Duration::from_secs(1), [0.1; 32]));
        s.spectrum_history
            .push_back((now - std::time::Duration::from_millis(100), [0.9; 32]));
        s.listening = true;
        s.audio_ready = true;
        s.audio_delay = 500.;
        s.prefs.volume = 0.;
        s.sample_spectrum(now);
        assert_eq!(s.levels, [0.1; 32]);
        s.listening = false;
        s.sample_spectrum(now);
        assert_eq!(s.levels, [0.9; 32]);
    }
    #[test]
    fn paused_lyrics_catch_up_after_the_audio_buffer_drains() {
        let mut state = State::new(Prefs::default());
        state.playback = json!({"state":"paused","position_ms":10000,"duration_ms":20000});
        state.listening = true;
        state.audio_ready = true;
        state.audio_delay = 600.;
        state.transport_at = Instant::now() - std::time::Duration::from_secs(2);
        assert_eq!(state.lyric_position(), 10000.);
        state.playback["state"] = json!("playing");
        state.updated = Instant::now();
        assert!((state.lyric_position() - 9400.).abs() < 10.);
    }
    #[test]
    fn bundled_theme_names_are_unicode_and_complete() {
        let state = State::new(Prefs::default());
        assert_eq!(state.themes.len(), 18);
        assert_eq!(state.themes[3].name, "Catppuccin Frappé");
    }
    #[test]
    fn durations_do_not_mix_seconds_and_milliseconds() {
        assert_eq!(duration(&json!({"duration":199})), 199000.);
        assert_eq!(
            duration(&json!({"duration_ms":199000,"duration":199})),
            199000.
        );
        assert_eq!(time(199000.), "3:19");
    }
    #[test]
    fn all_controls_share_the_same_geometry_for_drawing_and_hit_testing() {
        let r = Rect::new(10., 20., 100., 40.);
        assert!(r.contains(50., 30.));
        assert!(!r.contains(50., 61.));
    }
    #[test]
    fn query_encoding_is_utf8_safe() {
        assert_eq!(encode("Güneş & rock"), "G%C3%BCne%C5%9F%20%26%20rock");
    }
}
