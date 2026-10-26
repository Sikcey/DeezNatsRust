//! UI structure mirrors the original WinForms shell and its in-app views.
//! Coordinates are device-independent; the HWND host applies the DPI scale.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum View {
    Home,
    Explore,

    Flow,
    Podcasts,
    Favorites,
    Albums,
    Playlists,
    Offline,
    Recent,
    Queue,
    Downloads,
    Settings,
    Lyrics,
    Account,
    FlowTune,
    PlaylistEdit,
    PlaylistPicker,
    CoverEdit,
    Album,
    Artist,
    Show,
    Add,
    Storage,
    ArlHelp,
    TrackInfo,
    CustomColors,
}
impl View {
    pub fn title(self) -> &'static str {
        match self {
            Self::Home => "For you",
            Self::Explore => "Explore",

            Self::Flow => "Flow",
            Self::Podcasts => "Podcasts",
            Self::Favorites => "Favorites",
            Self::Albums => "Liked albums",
            Self::Playlists => "Library",
            Self::Offline => "Offline",
            Self::Recent => "Recently played",
            Self::Queue => "Queue",
            Self::Downloads => "Downloads",
            Self::Settings => "Settings",
            Self::Lyrics => "Lyrics",
            Self::Account => "Sign in to Deezer",
            Self::FlowTune => "Customize Flow",
            Self::PlaylistPicker => "Add to playlist",
            Self::PlaylistEdit => "Edit playlist",
            Self::CoverEdit => "Edit cover",
            Self::Album => "Album",
            Self::Artist => "Artist",
            Self::Show => "Podcast episodes",
            Self::Add => "Add to your library",
            Self::Storage => "Data and cache location",
            Self::ArlHelp => "Sign in with ARL",
            Self::TrackInfo => "Track information",
            Self::CustomColors => "Custom colors",
        }
    }
}
pub const NAV: [(View, &str); 9] = [
    (View::Home, "\u{e80f}"),
    (View::Explore, "\u{e721}"),
    (View::Flow, "\u{e8d6}"),
    (View::Favorites, "\u{eb51}"),
    (View::Albums, "\u{e93c}"),
    (View::Playlists, "\u{e8f1}"),
    (View::Podcasts, "\u{e720}"),
    (View::Offline, "\u{e896}"),
    (View::Recent, "\u{e81c}"),
];
pub const TABS: [&str; 6] = [
    "General",
    "Deezer",
    "Appearance",
    "Lyrics",
    "Playback",
    "Integrations",
];
pub const TAB_ICONS: [&str; 6] = [
    "\u{e713}", "\u{e77b}", "\u{e790}", "\u{ec71}", "\u{e768}", "\u{e71b}",
];
fn preferences_base() -> Option<std::path::PathBuf> {
    #[cfg(test)]
    {
        Some(std::env::temp_dir().join(format!(
            "DeezNatsRust-ui-tests-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        )))
    }
    #[cfg(not(test))]
    {
        std::env::var_os("LOCALAPPDATA").map(std::path::PathBuf::from)
    }
}

pub const SOLID_THEMES: [(&str, bool, u32, u32, u32, u32, u32, u32); 18] = [
    (
        "System", false, 0x00171717, 0x001f1f23, 0x002d2d34, 0x003e3e47, 0x00f8f8f8, 0x00b954db,
    ),
    (
        "Dark", false, 0x00121214, 0x001a1a1e, 0x0026262c, 0x00383842, 0x00f5f5f7, 0x007c399f,
    ),
    (
        "OLED", false, 0x00000000, 0x000c0c0c, 0x00181818, 0x00242424, 0x00f8f8f8, 0x00b954db,
    ),
    (
        "Catppuccin Frappé",
        false,
        0x00463430,
        0x003d2c29,
        0x00594541,
        0x00735b56,
        0x00f5d0c6,
        0x00e69ec6,
    ),
    (
        "Rose Pine",
        false,
        0x00241719,
        0x002e1d1f,
        0x003d262a,
        0x00523b40,
        0x00f4dee0,
        0x00b76feb,
    ),
    (
        "Nord", false, 0x0040342e, 0x0052423b, 0x0066534b, 0x007a645b, 0x00f4efec, 0x00d0c088,
    ),
    (
        "Dracula", false, 0x00362a28, 0x002a211f, 0x005a4744, 0x00745b57, 0x00f2f8f8, 0x00c679ff,
    ),
    (
        "Gruvbox", false, 0x00282828, 0x0036383c, 0x00454950, 0x005c626b, 0x00b2dbeb, 0x002f9bd7,
    ),
    (
        "Tokyo Night",
        false,
        0x00261b1a,
        0x003b2824,
        0x004d352f,
        0x006b4a41,
        0x00f5cac0,
        0x00f7a27a,
    ),
    (
        "Emerald Forest",
        false,
        0x0014190f,
        0x001d2416,
        0x002a3520,
        0x003d4d2f,
        0x00ecf5e8,
        0x0071cc2e,
    ),
    (
        "Solar Amber",
        false,
        0x00101318,
        0x00171d24,
        0x00212933,
        0x00333f4e,
        0x00ebf4fd,
        0x00129cf3,
    ),
    (
        "Blood Moon",
        false,
        0x00160f18,
        0x00211624,
        0x00301f34,
        0x00472e4d,
        0x00faebf5,
        0x004824d8,
    ),
    (
        "Porcelain",
        true,
        0x00f8f4f2,
        0x00ffffff,
        0x00f0e7e3,
        0x00d4c6bf,
        0x00261c18,
        0x00d64554,
    ),
    (
        "Linen", true, 0x00eaf1f6, 0x00f9fdff, 0x00d5e1ea, 0x00a6b9c9, 0x00171e26, 0x00224e9a,
    ),
    (
        "Sage", true, 0x00f0f4ee, 0x00fafcf9, 0x00e0e8dc, 0x00b8c6b0, 0x001e2618, 0x00486e27,
    ),
    (
        "Rosewater",
        true,
        0x00f2eff8,
        0x00fbf9ff,
        0x00e2dced,
        0x00b9afcf,
        0x0020182b,
        0x00622ead,
    ),
    (
        "Sky", true, 0x00faf4ed, 0x00fffbf8, 0x00f5e8da, 0x00ddc6af, 0x00322215, 0x00ae6119,
    ),
    (
        "Lavender", true, 0x00faeff3, 0x00fff8fa, 0x00f3dce4, 0x00dbb3c1, 0x00331922, 0x00b8386d,
    ),
];
pub const GRADIENT_PRESETS: [(&str, u32, u32); 18] = [
    ("Rose Violet", 0x007c399f, 0x00651e3d),
    ("Ocean", 0x00c87621, 0x005e2e10),
    ("Aurora", 0x0078981f, 0x006b351e),
    ("Sunset", 0x004462c8, 0x00522164),
    ("Lavender", 0x00be6a7c, 0x006d2c37),
    ("Graphite", 0x005c524a, 0x002d2723),
    ("Emerald", 0x00608a24, 0x002b3c14),
    ("Midnight", 0x00924833, 0x003f1d14),
    ("Cherry", 0x005832b0, 0x0028154c),
    ("Crimson", 0x003b2fa8, 0x001c1544),
    ("Amber", 0x002579b8, 0x00123354),
    ("Copper", 0x00375ca4, 0x001b2747),
    ("Arctic", 0x00b6883c, 0x005a3b1a),
    ("Mint", 0x007e9e2e, 0x003c4916),
    ("Plum", 0x00923f7a, 0x00451b38),
    ("Electric", 0x00d0593d, 0x00662319),
    ("Coffee", 0x00405776, 0x001d2635),
    ("Slate", 0x00766052, 0x003a2d24),
];
#[derive(Clone, Debug)]
pub enum Action {
    OpenItem(crate::catalog::Entry),
    PlayAll,
    More,
    Favorite,
    FavoriteNow,
    MuteVolume,
    PlayerFullscreen,
    Download,
    Share,
    Sort(usize),
    ToggleCheck(String),
    Preset(usize),
    ResetTheme,
    ImportTheme,
    ExportTheme,
    Shelf(String, i32),
    ShelfAll(String),
    Api(&'static str, Vec<serde_json::Value>),
    Adjust(&'static str, i32),
    BackgroundImage,
    ChooseStorage,
    CancelStorage,
    BackgroundMode(usize),
    Color(&'static str),
    FlowChoice(usize),
    Genre(usize),
    ApplyFlow,
    CancelDownload(u64),
    Go(View),
    Back,
    Connect,
    ConnectSaved,
    OpenConnect,
    CopyConnectLink,
    DeezerLoginOptions,
    DeezerLoginAdvanced(bool),
    CloseDeezerLogin,
    ConnectDeezerWebView,
    Logout,
    PlayId,

    Toggle,
    Stop,
    Next,
    Previous,
    Shuffle,
    Repeat,
    Setting(&'static str),
    Tab(usize),
    Filter(usize),
    Notice(&'static str),
    SavePlaylist,
    EditPlaylist(crate::catalog::Entry),
    ChoosePlaylist(crate::catalog::Entry),
    ChoosePlaylistCover,
    ClearPlaylistDescription,
    CropPlaylistCover(f32, f32, f32),
    ExportPlaylistCover,
    DeletePlaylist,
    Search,
    Eq(usize, i32),
    Close,
    ToggleSidebar,
    SetControlAlignment(bool),
    ToggleQueueSidebar,
    TogglePodcastDescription(String),
    ToggleLyricsVolume,
    QueueJump(usize),
    QueueClear,
    QueueReset,
    SeekMs(u64),
    ToggleLyricsCoverMode,
    ToggleLyricsVisualizer,
    ToggleLyricsFullscreen,
    VisualizerStyle(u8),
    CornerStyle(usize),
    SolidTheme(usize),
    PlayerLayout(usize),
    ToggleBool(&'static str),
    AdjustLyricsSize(bool, i32),
    ToggleLyricSource(usize),

    DeezerLogo,

    StreamingQuality(usize),
    EqMode(bool),
    EqReset,
    OpenTrackInfo(crate::catalog::Entry),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}
impl Rect {
    pub fn new(x: i32, y: i32, w: i32, h: i32) -> Self {
        Self { x, y, w, h }
    }
}
pub struct Button {
    pub rect: Rect,
    pub text: String,
    pub glyph: &'static str,
    pub action: Action,
    pub selected: bool,
    pub enabled: bool,
    pub artwork: String,
    pub accessible: String,
    pub text_link: bool,
    pub text_centered: bool,
    pub artist_credit_group: Option<Rect>,
}
// Lay out a single credit line, preserving the leading names and reserving room
// for an ellipsis on the next credit when the complete line does not fit.
pub(crate) fn artist_credit_rects(rect: Rect, widths: &[i32], centered: bool) -> Vec<Rect> {
    let width = rect.w.max(0);
    let natural_total = widths.iter().sum::<i32>();
    let total = natural_total.min(width);
    let mut x = rect.x + if centered { (width - total) / 2 } else { 0 };
    let end = x + total;
    widths
        .iter()
        .enumerate()
        .map(|(i, natural)| {
            let remaining = end - x;
            let reserve = if natural_total > width && i + 1 < widths.len() {
                28.min(remaining / 2)
            } else {
                0
            };
            let w = (*natural).min((remaining - reserve).max(0));
            let slot = Rect::new(x, rect.y, w, rect.h);
            x += w;
            slot
        })
        .collect()
}
pub struct Label {
    pub rect: Rect,
    pub text: String,
    pub kind: u8,
}
pub struct PlayerTitle {
    pub region: Rect,
    pub label: usize,
    pub badge: Option<usize>,
    pub favorite: usize,
    pub centered: bool,
}
pub struct LyricRow {
    pub label: usize,
    pub line: usize,
    pub range: std::ops::Range<usize>,
}
pub struct Panel {
    pub rect: Rect,
    pub kind: u8,
}
#[derive(Default)]
pub struct Scene {
    pub player_title: Option<PlayerTitle>,
    pub auth_modal: Option<(Rect, usize, usize)>,
    pub login_underlay: Option<(Rect, usize, usize)>,
    pub device_popover_anchor: Option<Rect>,
    pub lyric_rows: Vec<LyricRow>,
    pub lyrics_anchor: Option<i32>,
    pub lyrics_gap: Option<(u64, u64)>,
    pub karaoke_preview: Option<usize>,
    pub queue_viewport: Option<Rect>,
    pub queue_scrollbar: Option<Rect>,
    pub queue_scroll_max: usize,
    pub artwork_ahead: Vec<(i32, String)>,
    pub content_buttons: std::ops::Range<usize>,
    pub content_labels: std::ops::Range<usize>,
    pub queue_labels: std::ops::Range<usize>,
    pub content_panels: std::ops::Range<usize>,
    pub content_pictures: std::ops::Range<usize>,
    pub content_top: i32,
    pub buttons: Vec<Button>,
    pub labels: Vec<Label>,
    pub panels: Vec<Panel>,
    pub search: Option<Rect>,
    pub session: Option<Rect>,

    pub track: Option<Rect>,
    pub name: Option<Rect>,
    pub description: Option<Rect>,
    pub seek: Option<Rect>,
    pub volume: Option<Rect>,
    pub elapsed: Option<Rect>,
    pub remaining: Option<Rect>,
    pub genre_viewport: Option<Rect>,
    pub content_height: i32,
    pub pictures: Vec<(Rect, String)>,
    pub sliders: Vec<(Rect, &'static str, u32)>,
    pub eq_graph: Option<Rect>,
    pub eq_sliders: Option<Rect>,
    pub lyrics_cover: Option<Rect>,
    pub lyrics_area: Option<Rect>,
    pub volume_popup: Option<Rect>,
    pub volume_vertical: bool,
}
impl Scene {
    fn link(
        &mut self,
        rect: Rect,
        text: impl Into<String>,
        target: crate::catalog::Entry,
        centered: bool,
    ) {
        self.button(rect, text, "", Action::OpenItem(target), false);
        if let Some(button) = self.buttons.last_mut() {
            button.text_link = true;
            button.text_centered = centered;
            button.accessible = button.text.clone();
        }
    }
    fn artist_links(&mut self, rect: Rect, entry: &crate::catalog::Entry, centered: bool) {
        let artists = entry.artist_entries();
        if artists.is_empty() {
            self.label_sized(
                rect.x,
                rect.y,
                rect.w,
                rect.h,
                &entry.subtitle,
                if centered { 11 } else { 1 },
            );
            return;
        }
        let names: Vec<_> = artists
            .iter()
            .enumerate()
            .map(|(i, artist)| format!("{}{}", if i == 0 { "" } else { ", " }, artist.title))
            .collect();
        let widths: Vec<_> = names
            .iter()
            .map(|name| name.chars().count() as i32 * 7)
            .collect();
        for ((artist, name), slot) in artists
            .into_iter()
            .zip(names)
            .zip(artist_credit_rects(rect, &widths, centered))
        {
            let accessible = artist.title.clone();
            self.link(slot, name, artist, centered);
            let button = self.buttons.last_mut().unwrap();
            button.artist_credit_group = Some(rect);
            button.accessible = accessible;
        }
    }
    fn label_sized(&mut self, x: i32, y: i32, w: i32, h: i32, text: impl Into<String>, kind: u8) {
        self.labels.push(Label {
            rect: Rect::new(x, y, w, h),
            text: text.into(),
            kind,
        });
    }
    fn label(&mut self, x: i32, y: i32, w: i32, text: impl Into<String>, kind: u8) {
        let h = match kind {
            13 => 64,
            12 => 48,
            2 | 5 => 48,
            3 => 38,
            _ => 28,
        };
        self.label_sized(x, y, w, h, text, kind);
    }
    fn panel(&mut self, x: i32, y: i32, w: i32, h: i32, kind: u8) {
        self.panels.push(Panel {
            rect: Rect::new(x, y, w, h),
            kind,
        });
    }
    fn button(
        &mut self,
        r: Rect,
        text: impl Into<String>,
        glyph: &'static str,
        action: Action,
        selected: bool,
    ) {
        self.buttons.push(Button {
            rect: r,
            text: text.into(),
            glyph,
            action,
            selected,
            enabled: true,
            artwork: String::new(),
            accessible: String::new(),
            text_link: false,
            text_centered: false,
            artist_credit_group: None,
        });
    }
}
pub struct Ui {
    pub playlist_draft: crate::playlist::Draft,
    pub playlist_busy: bool,
    pub playlist_track: Option<crate::catalog::Entry>,
    pub expanded_palettes: bool,
    pub expanded_themes: bool,
    pub expanded_podcast_description: Option<String>,
    pub dpi_scale: f64,
    pub language: String,
    pub initial_volume: u32,
    pub unmuted_volume: u32,
    pub translations: std::sync::Arc<std::collections::HashMap<String, String>>,
    pub view: View,
    pub tab: usize,
    pub filter: usize,
    pub light: bool,
    pub solid: bool,
    pub scroll: i32,
    pub history: Vec<View>,
    pub shuffle: bool,
    pub repeat: u8,
    pub autoplay: bool,
    pub eq: [i32; 11],
    pub choices: std::collections::HashMap<&'static str, usize>,
    pub page: Option<std::sync::Arc<crate::catalog::Page>>,
    pub now: crate::catalog::Entry,
    pub account: serde_json::Value,
    pub quality: String,
    pub lyrics: Vec<(u64, String)>,
    pub position: u64,
    pub duration: u64,
    pub shelf_offsets: std::collections::HashMap<String, usize>,
    pub devices: Vec<(String, String)>,
    pub background_mode: usize,
    pub blur: u32,
    pub surface_opacity: u32,
    pub brightness: u32,
    pub light_opacity: u32,
    pub light_tint: u32,
    pub solid_color: u32,
    pub dark_colors: std::collections::HashMap<String, u32>,
    pub light_colors: std::collections::HashMap<String, u32>,
    pub background_file: String,
    pub flow_options: Vec<(String, String)>,
    pub genres: Vec<(String, String, bool, bool)>,
    pub flow_id: String,

    pub flow_customizable: bool,
    pub flow_loaded: bool,
    pub flow_filter: String,
    pub downloads: Vec<crate::downloads::Status>,
    pub offline: Vec<crate::catalog::Entry>,
    pub sort_column: usize,
    pub sort_descending: bool,
    pub checked_ids: std::collections::HashSet<String>,
    pub liked_ids: std::collections::HashSet<String>,
    pub sidebar_collapsed: bool,
    pub queue_sidebar_open: bool,
    pub queue_scroll: usize,
    pub queue: Vec<crate::catalog::Entry>,
    pub queue_index: usize,
    pub corner_style: usize,
    pub solid_theme: usize,
    pub adaptive_colors: bool,
    pub adaptive_seed: Option<u32>,
    pub adaptive_artwork_id: u64,
    pub player_layout: usize,
    pub transparent_player: bool,
    pub karaoke_lyrics: bool,
    pub blur_inactive_lyrics: bool,
    pub lyrics_size_panel: u32,
    pub lyrics_size_fullscreen: u32,
    pub lyrics_sources: u32,

    pub deezer_login_open: bool,
    pub deezer_login_advanced: bool,
    pub deezer_login_busy: bool,
    pub deezer_login_status: String,

    pub lyrics_cover_mode: bool,
    pub lyrics_cover_mix: Option<f32>,
    pub lyrics_visualizer: bool,
    pub lyrics_manual_scroll: bool,
    pub lyrics_manual_until: Option<std::time::Instant>,
    pub lyrics_fullscreen: bool,
    pub visualizer_style: u8,
    pub rich_lyrics: Vec<crate::lyrics::LyricLine>,
    pub lyrics_provider: String,
    pub lyrics_cover_hovered: bool,
    pub lyrics_volume_open: bool,
    pub track_info: Option<(crate::catalog::Entry, serde_json::Value, serde_json::Value)>,
    pub eq_advanced: bool,
    pub bass_db: i32,
    pub treble_db: i32,
    pub keep_playing_on_close: bool,
    pub show_in_tray: bool,
    pub window_controls: bool,
    pub controls_right: bool,
    pub traffic_lights: bool,
    pub keep_played_tracks: bool,
    pub jam_enabled: bool,
    pub jam_url: String,
    pub stay_awake: bool,
    pub window_blur: bool,
    pub ui_blur: bool,
    pub show_on_discord: bool,
    pub discord_paused: bool,
    pub discord_badge: bool,
    pub discord_hide_details: bool,
    pub discord_btn_service: bool,
    pub discord_btn_app: bool,

    pub smooth_scrolling: bool,
    pub scroll_animation: bool,
}
impl Default for Ui {
    fn default() -> Self {
        Self {
            playlist_draft: Default::default(),
            playlist_busy: false,
            playlist_track: None,
            expanded_palettes: false,
            expanded_themes: false,
            expanded_podcast_description: None,
            dpi_scale: 1.0,
            language: String::new(),
            initial_volume: 70,
            unmuted_volume: 70,
            translations: Default::default(),
            view: View::Home,
            tab: 0,
            filter: 0,
            light: false,
            solid: false,
            scroll: 0,
            history: Vec::new(),
            shuffle: false,
            repeat: 0,
            autoplay: true,
            eq: [0; 11],
            choices: Default::default(),
            page: None,
            now: Default::default(),
            account: Default::default(),
            quality: String::new(),
            lyrics: Vec::new(),
            position: 0,
            duration: 0,
            shelf_offsets: Default::default(),
            devices: Vec::new(),
            background_mode: 0,
            blur: 70,
            surface_opacity: 80,
            brightness: 96,
            light_opacity: 60,
            light_tint: 0x00f2ebe4,
            solid_color: 0x00171717,
            dark_colors: Default::default(),
            light_colors: Default::default(),
            background_file: String::new(),
            flow_options: Vec::new(),
            genres: Vec::new(),
            flow_id: "default".into(),

            flow_customizable: false,
            flow_loaded: false,
            flow_filter: String::new(),
            downloads: Vec::new(),
            offline: Vec::new(),
            sort_column: 0,
            sort_descending: false,
            checked_ids: Default::default(),
            liked_ids: Default::default(),
            sidebar_collapsed: false,
            queue_sidebar_open: false,
            queue_scroll: 0,
            queue: Vec::new(),
            queue_index: 0,
            corner_style: 2,
            solid_theme: 1,
            adaptive_colors: false,
            adaptive_seed: None,
            adaptive_artwork_id: 0,
            player_layout: 0,
            transparent_player: true,
            karaoke_lyrics: true,
            blur_inactive_lyrics: true,
            lyrics_size_panel: 100,
            lyrics_size_fullscreen: 100,
            lyrics_sources: crate::lyrics::DEFAULT_SOURCES,

            deezer_login_open: false,
            deezer_login_advanced: false,
            deezer_login_busy: false,
            deezer_login_status: String::new(),

            lyrics_cover_mode: false,
            lyrics_cover_mix: None,
            lyrics_visualizer: true,
            lyrics_manual_scroll: false,
            lyrics_manual_until: None,
            lyrics_fullscreen: false,
            visualizer_style: 0,
            rich_lyrics: Vec::new(),
            lyrics_provider: String::new(),
            lyrics_cover_hovered: false,
            lyrics_volume_open: false,
            track_info: None,
            eq_advanced: false,
            bass_db: 0,
            treble_db: 0,
            keep_playing_on_close: true,
            show_in_tray: true,
            window_controls: true,
            controls_right: true,
            traffic_lights: false,
            keep_played_tracks: true,
            jam_enabled: false,
            jam_url: String::new(),
            stay_awake: false,
            window_blur: true,
            ui_blur: true,
            show_on_discord: false,
            discord_paused: false,
            discord_badge: true,
            discord_hide_details: false,
            discord_btn_service: true,
            discord_btn_app: true,

            smooth_scrolling: true,
            scroll_animation: true,
        }
    }
}
impl Ui {
    /// Fold open sidebars before they squeeze the main page below its useful width.
    /// This runs on window resize; users can still explicitly reopen a sidebar.
    pub fn fit_sidebars(&mut self, width: i32) {
        let lyrics = self.view == View::Lyrics;
        let minimum = if matches!(self.view, View::Settings | View::Lyrics) {
            600
        } else {
            520
        };
        let queue = if self.queue_sidebar_open {
            280.min(width / 3)
        } else {
            0
        };
        if !lyrics && !self.sidebar_collapsed && width - 262 - 40 - queue < minimum {
            self.sidebar_collapsed = true;
        }
        let left = if lyrics {
            32
        } else if self.sidebar_collapsed {
            76
        } else {
            262
        };
        let right = if lyrics { 32 } else { 40 };
        if self.queue_sidebar_open && width - left - right - queue < minimum {
            self.queue_sidebar_open = false;
        }
    }
    pub fn t<'a>(&'a self, value: &'a str) -> &'a str {
        self.translations
            .get(value)
            .map(String::as_str)
            .unwrap_or(value)
    }
    pub fn set_language(&mut self, preference: &str) {
        let index = match preference { "tr" => 1, "en" => 2, "az" => 3, _ => 0 };
        self.choices.insert("Language", index);
        self.language = if index == 0 {
            #[link(name = "kernel32")]
            unsafe extern "system" { fn GetUserDefaultUILanguage() -> u16; }
            match unsafe { GetUserDefaultUILanguage() } & 0x3ff {
                0x1f => "tr", 0x2c => "az", _ => "en",
            }.into()
        } else { preference.to_owned() };
        let catalog = match self.language.as_str() {
            "tr" => include_str!("../../../assets/localization/tr.json"),
            "az" => include_str!("../../../assets/localization/az.json"),
            _ => "{}",
        };
        self.translations = std::sync::Arc::new(serde_json::from_str(catalog).unwrap_or_default());
    }
    fn translated_text(&self, text: &str) -> String {
        // Setting values carry a dropdown marker outside the translatable text.
        if let Some(value) = text.strip_suffix("   \u{25be}") {
            return format!("{}   \u{25be}", self.t(value));
        }
        if let Some(value) = self.translations.get(text) { return value.clone(); }
        for (source, translated) in self.translations.iter().filter(|(key, _)| key.contains("{0}")) {
            let mut rest = text;
            let mut pattern = source.as_str();
            let mut values = Vec::new();
            let mut matched = true;
            for index in 0..10 {
                let token = format!("{{{index}}}");
                let Some((prefix, tail)) = pattern.split_once(&token) else { break; };
                let Some(after_prefix) = rest.strip_prefix(prefix) else { matched = false; break; };
                let delimiter = tail.split('{').next().unwrap_or(tail);
                let end = if delimiter.is_empty() { after_prefix.len() }
                    else if let Some(end) = after_prefix.find(delimiter) { end }
                    else { matched = false; break; };
                values.push(&after_prefix[..end]);
                rest = &after_prefix[end..];
                pattern = tail;
            }
            if matched && !values.is_empty() && rest == pattern {
                let mut result = translated.clone();
                for (index, value) in values.iter().enumerate() {
                    result = result.replace(&format!("{{{index}}}"), value);
                }
                return result;
            }
        }
        text.to_owned()
    }
    pub fn color(&self, key: &str, default: u32) -> u32 {
        self.color_with_light(key, default, self.light)
    }
    pub fn color_with_light(&self, key: &str, default: u32, light: bool) -> u32 {
        let colors = if light {
            &self.light_colors
        } else {
            &self.dark_colors
        };
        colors.get(key).copied().unwrap_or(default)
    }
    pub fn load_preferences() -> Self {
        let mut ui = Self {
            offline: crate::downloads::load_offline(),
            ..Self::default()
        };
        let Some(base) = preferences_base() else {
            return ui;
        };
        let args: Vec<_> = std::env::args().collect();
        let import = args
            .windows(2)
            .find(|a| a[0] == "--appearance-profile")
            .map(|a| std::path::PathBuf::from(&a[1]));
        let original = import.clone().unwrap_or_else(|| base.join("DeezNats"));
        let own = crate::storage::data_dir_at(&base).unwrap_or_else(|_| base.join("DeezNatsRust")).join("ui.json");
        let value = std::fs::read(if import.is_some() {
            original.join("settings.json")
        } else {
            own.clone()
        })
        .or_else(|_| std::fs::read(original.join("settings.json")))
        .ok()
        .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
        .unwrap_or_default();
        let language = std::env::var("DEEZNATS_LANGUAGE")
            .unwrap_or_else(|_| value["Language"].as_str().unwrap_or("").to_owned());
        ui.set_language(&language);
        ui.choices.insert("Show on startup", value["ShowOnStartup"].as_u64().unwrap_or(0).min(5) as usize);
        ui.view = ui.startup_view();
        ui.light = value["LightTheme"].as_bool().unwrap_or(false);
        ui.adaptive_colors = false;
        ui.initial_volume =
            (value["Volume"].as_f64().unwrap_or(0.7).clamp(0.0, 1.0) * 100.0).round() as u32;
        ui.flow_id = value["FlowConfigId"]
            .as_str()
            .unwrap_or("default")
            .to_owned();
        ui.blur = value["BackgroundBlur"]
            .as_u64()
            .unwrap_or(70)
            .min(100) as u32;
        ui.brightness = value["LightBrightness"].as_u64().unwrap_or(96).min(110) as u32;
        ui.surface_opacity = value["SurfaceOpacity"].as_u64().unwrap_or(80).min(100) as u32;
        ui.light_opacity = value["LightOpacity"].as_u64().unwrap_or(60).min(100) as u32;
        ui.window_blur = value["WindowBlur"].as_bool().unwrap_or(true);
        ui.ui_blur = value["UiBlur"].as_bool().unwrap_or(true);
        let parse_color = |key: &str, default: u32| {
            value[key]
                .as_str()
                .and_then(|s| u32::from_str_radix(s.trim_start_matches('#'), 16).ok())
                .map(|c| ((c & 255) << 16) | (c & 0xff00) | ((c >> 16) & 255))
                .unwrap_or(default)
        };
        ui.light_tint = parse_color("LightTint", ui.light_tint);
        ui.solid_color = parse_color("FixedBackgroundColor", ui.solid_color);
        ui.background_mode = match value["BackgroundMode"].as_u64().unwrap_or(0) {
            1 => 2,
            2 => 1,
            3 => 3,
            _ => 0,
        };
        if ui.background_mode == 0 && value["WindowMaterial"].as_i64() == Some(0) {
            ui.background_mode = 3;
        }
        ui.solid = ui.background_mode != 0;
        ui.background_file = value["BackgroundFile"]
            .as_str()
            .map(str::to_owned)
            .unwrap_or_else(|| {
                original
                    .join("Background")
                    .join("image.png")
                    .to_string_lossy()
                    .into_owned()
            });
        for (key, map) in [
            ("DarkColors", &mut ui.dark_colors),
            ("LightColors", &mut ui.light_colors),
        ] {
            if let Some(colors) = value[key].as_object() {
                for (name, value) in colors {
                    if let Some(color) = value
                        .as_str()
                        .and_then(|s| u32::from_str_radix(s.trim_start_matches('#'), 16).ok())
                    {
                        map.insert(
                            name.clone(),
                            ((color & 255) << 16) | (color & 0xff00) | ((color >> 16) & 255),
                        );
                    }
                }
            }
        }
        let luma =
            |c: u32| ((c & 255) * 2126 + ((c >> 8) & 255) * 7152 + ((c >> 16) & 255) * 722) / 10000;
        if let (Some(&text), Some(&muted)) =
            (ui.light_colors.get("Text"), ui.light_colors.get("Muted"))
        {
            if luma(muted) <= luma(text) + 15 {
                let panel = ui.light_colors.get("Panel").copied().unwrap_or(0x00ffffff);
                let fixed_muted = ((((text >> 16) & 255) * 66 + ((panel >> 16) & 255) * 34) / 100)
                    << 16
                    | ((((text >> 8) & 255) * 66 + ((panel >> 8) & 255) * 34) / 100) << 8
                    | (((text & 255) * 66 + (panel & 255) * 34) / 100);
                ui.light_colors.insert("Muted".into(), fixed_muted);
            }
        }
        if let Some(&raised) = ui.light_colors.get("Raised") {
            if luma(raised) < 120 {
                ui.light_colors.insert("Raised".into(), 0x00f0e7e3);
            }
        }
        ui.corner_style = value["CornerStyle"].as_u64().unwrap_or(2).min(3) as usize;
        ui.solid_theme = value["SolidTheme"]
            .as_u64()
            .unwrap_or(if ui.light { 12 } else { 1 })
            .min(SOLID_THEMES.len() as u64 - 1) as usize;
        ui.player_layout = value["PlayerLayout"].as_u64().unwrap_or(0).min(1) as usize;
        ui.transparent_player = value["TransparentPlayer"].as_bool().unwrap_or(true);

        ui.jam_enabled = value["JamEnabled"].as_bool().unwrap_or(false);
        ui.autoplay = value["Autoplay"].as_bool().unwrap_or(true);
        ui.karaoke_lyrics = value["KaraokeLyrics"].as_bool().unwrap_or(true);
        ui.blur_inactive_lyrics = value["BlurInactiveLyrics"].as_bool().unwrap_or(true);
        ui.lyrics_size_panel = value["LyricsSizePanel"]
            .as_u64()
            .unwrap_or(100)
            .clamp(60, 180) as u32;
        ui.lyrics_size_fullscreen = value["LyricsSizeFullscreen"]
            .as_u64()
            .unwrap_or(100)
            .clamp(60, 180) as u32;
        ui.lyrics_sources = value["LyricsSources"]
            .as_u64()
            .unwrap_or(crate::lyrics::DEFAULT_SOURCES as u64) as u32;
        // Enable the newly added provider once for existing active configurations;
        // preserve an explicit all-sources-off preference and later user choices.
        if value["LyricsSourcesVersion"].as_u64().unwrap_or(0) < 3 && ui.lyrics_sources != 0 {
            ui.lyrics_sources |= (1 << 2) | (1 << 5) | (1 << 8) | (1 << 9);
            if value["LyricsSourcesVersion"].as_u64().unwrap_or(0) < 2 {
                ui.lyrics_sources |= 1 << 7;
            }
        }
        ui.lyrics_sources &= (1 << crate::lyrics::LYRIC_SOURCES.len()) - 1;
        ui.visualizer_style = value["VisualizerStyle"].as_u64().unwrap_or(0).min(2) as u8;
        ui.lyrics_visualizer = value["LyricsVisualizer"].as_bool().unwrap_or(true);
        ui.sidebar_collapsed = value["SidebarCollapsed"].as_bool().unwrap_or(false);
        ui.window_controls = value["WindowControls"].as_bool().unwrap_or(true);
        ui.controls_right = value["ControlsRight"].as_bool().unwrap_or(true);
        ui.traffic_lights = value["TrafficLights"].as_bool().unwrap_or(false);
        ui.keep_playing_on_close = value["KeepPlayingOnClose"].as_bool().unwrap_or(true);
        ui.show_in_tray = value["ShowInTray"].as_bool().unwrap_or(true);
        ui.smooth_scrolling = value["SmoothScrolling"].as_bool().unwrap_or(true);
        ui.scroll_animation = value["ScrollAnimation"].as_bool().unwrap_or(true);
        ui.choices.insert("Background", ui.background_mode);
        ui.choices.insert(
            "Audio quality",
            value["AudioQuality"].as_u64().unwrap_or(2).min(2) as usize,
        );
        ui.choices.insert(
            "Download quality",
            value["DownloadQuality"].as_u64().unwrap_or(2).min(2) as usize,
        );
        if let Some(eq_val) = value["Equalizer"].as_u64() {
            ui.choices.insert("Equalizer", eq_val.min(1) as usize);
        }
        if let Some(preset_val) = value["Preset"].as_u64() {
            let max_p = options("Preset").len().saturating_sub(1);
            ui.choices
                .insert("Preset", (preset_val as usize).min(max_p));
        }
        if let Some(eq_arr) = value["EQ"].as_array() {
            for (i, v) in eq_arr.iter().take(11).enumerate() {
                if let Some(g) = v.as_i64() {
                    ui.eq[i] = (g as i32).clamp(-12, 12);
                }
            }
        } else {
            let p_idx = ui.value("Preset");
            let presets = options("Preset");
            if p_idx < presets.len() {
                let p_name = presets[p_idx];
                if p_name != "Custom" {
                    if let Some(gains) = preset_gains(p_name) {
                        for (i, &g) in gains.iter().enumerate() {
                            ui.eq[i] = g;
                        }
                    }
                }
            }
        }
        ui
    }
    pub fn custom_preset_index(&self) -> usize {
        custom_preset_index()
    }
    pub fn apply_preset(&mut self, preset_name: &str) -> bool {
        if preset_name.eq_ignore_ascii_case("Custom") {
            let idx = self.custom_preset_index();
            self.choices.insert("Preset", idx);
            return true;
        }
        if let Some(gains) = preset_gains(preset_name) {
            for (i, &g) in gains.iter().enumerate() {
                self.eq[i] = g;
            }
            if let Some(idx) = options("Preset")
                .iter()
                .position(|&p| p.eq_ignore_ascii_case(preset_name))
            {
                self.choices.insert("Preset", idx);
            }
            self.choices.insert("Equalizer", 1);
            return true;
        }
        false
    }
    pub fn adjust_eq(&mut self, band: usize, gain: i32) {
        let clamped = gain.clamp(-12, 12);
        self.eq[band] = clamped;
        let custom_idx = self.custom_preset_index();
        self.choices.insert("Preset", custom_idx);
        self.save_preferences();
    }
    pub fn apply_solid_theme(&mut self, index: usize) {
        if let Some(&(_, is_light, bg, panel, raised, border, text, accent)) =
            SOLID_THEMES.get(index)
        {
            self.solid_theme = index;
            self.light = is_light;
            self.solid_color = bg;
            if is_light {
                self.light_tint = bg;
            }
            let map = if is_light {
                &mut self.light_colors
            } else {
                &mut self.dark_colors
            };
            let blend = |a: u32, b: u32, wa: u32| {
                let wb = 100 - wa;
                ((((a >> 16) & 255) * wa + ((b >> 16) & 255) * wb) / 100) << 16
                    | ((((a >> 8) & 255) * wa + ((b >> 8) & 255) * wb) / 100) << 8
                    | (((a & 255) * wa + (b & 255) * wb) / 100)
            };
            let grad_end = if is_light {
                blend(accent, 0x00181820, 78)
            } else {
                blend(accent, panel, 65)
            };
            let muted = blend(text, panel, if is_light { 66 } else { 68 });
            let accent_luma = ((accent & 255) * 2126
                + ((accent >> 8) & 255) * 7152
                + ((accent >> 16) & 255) * 722)
                / 10000;
            let accent_text = if accent_luma > 115 {
                0x00141418
            } else {
                0x00ffffff
            };
            map.insert("Background".into(), bg);
            map.insert("Panel".into(), panel);
            map.insert("Raised".into(), raised);
            map.insert("Border".into(), border);
            map.insert("Text".into(), text);
            map.insert("Icon".into(), text);
            map.insert("Accent".into(), accent);
            map.insert("GradientStart".into(), accent);
            map.insert("GradientEnd".into(), grad_end);
            map.insert("AccentText".into(), accent_text);
            map.insert("Muted".into(), muted);
        }
    }
    pub fn solid_background(&self) -> u32 {
        if self.light {
            self.color("Background", 0x00fafafa)
        } else {
            self.solid_color
        }
    }
    pub fn corner_px(&self, default: i32) -> i32 {
        match self.corner_style {
            0 => 0,
            1 => (default / 2).max(4),
            2 => default,
            _ => (default * 3 / 2).max(18),
        }
    }
    pub fn save_preferences(&self) {
        if let Some(base) = preferences_base() {
            let Ok(directory) = crate::storage::data_dir_at(&base) else { return; };
            let encode = |colors: &std::collections::HashMap<String, u32>| -> serde_json::Value {
                colors
                    .iter()
                    .map(|(key, color)| {
                        (
                            key.clone(),
                            serde_json::json!(format!(
                                "#{:06X}",
                                ((color & 255) << 16) | (color & 0xff00) | ((color >> 16) & 255)
                            )),
                        )
                    })
                    .collect::<serde_json::Map<_, _>>()
                    .into()
            };
            let hex = |c: u32| {
                format!(
                    "#{:06X}",
                    ((c & 255) << 16) | (c & 0xff00) | ((c >> 16) & 255)
                )
            };
            let mut value = serde_json::json!({"Volume":self.initial_volume as f64/100.0,"Language":match self.value("Language"){1=>"tr",2=>"en",3=>"az",_=>""},"FlowConfigId":self.flow_id,"LightTheme":self.light,"BackgroundBlur":self.blur,"WindowBlur":self.window_blur,"UiBlur":self.ui_blur,"LightBrightness":self.brightness,"SurfaceOpacity":self.surface_opacity,"LightOpacity":self.light_opacity,"LightTint":hex(self.light_tint),"FixedBackgroundColor":hex(self.solid_color),"BackgroundMode":match self.background_mode{1=>2,2=>1,other=>other},"BackgroundFile":self.background_file,"DarkColors":encode(&self.dark_colors),"LightColors":encode(&self.light_colors),"CornerStyle":self.corner_style,"SolidTheme":self.solid_theme,"PlayerLayout":self.player_layout,"TransparentPlayer":self.transparent_player,"KaraokeLyrics":self.karaoke_lyrics,"BlurInactiveLyrics":self.blur_inactive_lyrics,"LyricsSizePanel":self.lyrics_size_panel,"LyricsSizeFullscreen":self.lyrics_size_fullscreen,"LyricsSources":self.lyrics_sources,"LyricsSourcesVersion":3,"VisualizerStyle":self.visualizer_style,"LyricsVisualizer":self.lyrics_visualizer,"SidebarCollapsed":self.sidebar_collapsed,"WindowControls":self.window_controls,"ControlsRight":self.controls_right,"TrafficLights":self.traffic_lights,"KeepPlayingOnClose":self.keep_playing_on_close,"ShowInTray":self.show_in_tray,"SmoothScrolling":self.smooth_scrolling,"ScrollAnimation":self.scroll_animation,"AudioQuality":self.value("Audio quality"),"DownloadQuality":self.value("Download quality"),"Preset":self.value("Preset"),"Equalizer":self.value("Equalizer"),"EQ":self.eq});
            value["ShowOnStartup"] = serde_json::json!(self.value("Show on startup"));
            value["JamEnabled"] = serde_json::json!(self.jam_enabled);
            value["Autoplay"] = serde_json::json!(self.autoplay);
            value["AdaptiveColors"] = serde_json::json!(self.adaptive_colors);

            let _ = std::fs::create_dir_all(&directory);
            let _ = std::fs::write(directory.join("ui.json"), value.to_string());
        }
    }
    pub fn go(&mut self, view: View) {
        if self.view != view {
            self.lyrics_cover_hovered = false;
            self.lyrics_manual_scroll = false;
            self.lyrics_manual_until = None;
            self.lyrics_fullscreen = false;
            self.history.push(self.view);
            self.view = view;
            self.scroll = 0;
        }
    }
    pub fn back(&mut self) {
        if let Some(view) = self.history.pop() {
            self.lyrics_cover_hovered = false;
            self.lyrics_manual_scroll = false;
            self.lyrics_manual_until = None;
            self.lyrics_fullscreen = false;
            self.view = view;
            self.scroll = 0;
        }
    }
    pub fn lyrics_scale_percent(&self, w: i32) -> u32 {
        if self.lyrics_fullscreen || w >= 1400 {
            self.lyrics_size_fullscreen.clamp(60, 180)
        } else {
            self.lyrics_size_panel.clamp(60, 180)
        }
    }
    pub fn timed_lyrics(&self)->bool{
        if !self.rich_lyrics.is_empty(){crate::lyrics::has_timing(&self.rich_lyrics)}else{self.lyrics.iter().any(|(time,_)|*time>0)}
    }
    pub fn hold_lyric_follow(&mut self,now:std::time::Instant){
        if self.view==View::Lyrics && self.timed_lyrics(){
            self.lyrics_manual_scroll=true;
            self.lyrics_manual_until=Some(now+std::time::Duration::from_secs(4));
        }
    }
    pub fn resume_lyric_follow(&mut self,now:std::time::Instant)->bool{
        if self.view!=View::Lyrics || !self.timed_lyrics(){self.lyrics_manual_until=None;return false;}
        if !self.lyrics_manual_scroll || self.lyrics_manual_until.is_some_and(|until|now<until){return false;}
        self.lyrics_manual_scroll=false;self.lyrics_manual_until=None;true
    }
    pub fn step_shelf(&mut self, title: &str, delta: i32, count: usize) -> usize {
        if count == 0 {
            return 0;
        }
        let step = delta.unsigned_abs() as usize;
        if step == 0 {
            return self.shelf_offsets.get(title).copied().unwrap_or(0);
        }
        let old = self.shelf_offsets.get(title).copied().unwrap_or(0) % count;
        let step = step % count;
        let offset = if delta > 0 {
            (old + step) % count
        } else {
            (old + count - step) % count
        };
        self.shelf_offsets.insert(title.to_string(), offset);
        offset
    }
    pub fn startup_view(&self) -> View {
        [View::Home, View::Explore, View::Flow, View::Favorites, View::Playlists, View::Offline]
            [self.value("Show on startup").min(5)]
    }
    pub fn value(&self, key: &str) -> usize {
        if let Some(&v) = self.choices.get(key) {
            v
        } else if key == "Audio quality" || key == "Download quality" {
            2
        } else {
            0
        }
    }
}
pub const PRESETS: &[(&str, [i32; 10])] = &[
    ("Flat", [0, 0, 0, 0, 0, 0, 0, 0, 0, 0]),
    ("Bass boost", [6, 5, 4, 2, 1, 0, 0, 0, 0, 0]),
    ("Bass reducer", [-6, -5, -4, -2, -1, 0, 0, 0, 0, 0]),
    ("Treble boost", [0, 0, 0, 0, 0, 1, 2, 4, 5, 6]),
    ("Vocal", [-2, -1, 0, 2, 4, 4, 3, 1, 0, -2]),
    ("Rock", [5, 3, 2, 0, -1, 1, 2, 3, 4, 4]),
    ("Pop", [-1, 1, 3, 4, 4, 2, 0, 1, 2, 2]),
    ("Jazz", [3, 2, 1, 2, -1, -1, 0, 1, 2, 3]),
    ("Classical", [4, 3, 2, 1, -1, -1, 0, 2, 3, 3]),
    ("Electronic", [5, 4, 1, 0, -2, 2, 1, 2, 4, 5]),
];

pub fn preset_gains(name: &str) -> Option<[i32; 10]> {
    PRESETS
        .iter()
        .find(|(n, _)| n.eq_ignore_ascii_case(name))
        .map(|(_, g)| *g)
}

pub fn custom_preset_index() -> usize {
    options("Preset")
        .iter()
        .position(|&p| p == "Custom")
        .unwrap_or(options("Preset").len().saturating_sub(1))
}

pub fn options(key: &str) -> &'static [&'static str] {
    match key {
        "Language" => &["System language", "Turkish", "English", "Azerbaijani"],
        "Background" => &[
            "Acrylic (translucent blur)",
            "Adaptive track artwork",
            "Custom image",
            "Solid color",
        ],
        "Audio quality" | "Download quality" => &["MP3 128 kbps", "MP3 320 kbps", "FLAC"],
        "Audio output device" => &["Default Windows output"],
        "Crossfade (seconds)" => &[
            "0", "1", "2", "3", "4", "5", "6", "7", "8", "9", "10", "11", "12",
        ],
        "Sleep timer" => &["Off", "15 minutes", "30 minutes", "1 hour"],
        "Preset" => &[
            "Flat",
            "Bass boost",
            "Bass reducer",
            "Treble boost",
            "Vocal",
            "Rock",
            "Pop",
            "Jazz",
            "Classical",
            "Electronic",
            "Custom",
        ],
        "Show on startup" => &[
            "Home",
            "Explore",
            "Flow",
            "Favorites",
            "Playlists",
            "Offline",
        ],
        "Status name" => &[
            "Service (e.g. Deezer)",
            "DeezNats",
            "Track title",
            "Artist name",
        ],
        _ => &["Off", "On"],
    }
}
fn field(scene: &mut Scene, ui: &Ui, x: i32, y: i32, w: i32, key: &'static str) {
    scene.label(x, y, w, key, 0);
    let values = options(key);
    let value = if key == "Audio output device" {
        ui.devices.get(ui.value(key)).map(|(_, name)| name.as_str()).unwrap_or(values[0])
    } else { values[ui.value(key).min(values.len() - 1)] };
    scene.button(
        Rect::new(x, y + 30, w.min(420), 36),
        format!("{value}   \u{25be}"),
        "",
        Action::Setting(key),
        false,
    );
}
fn field_pair(
    scene: &mut Scene,
    ui: &Ui,
    x: i32,
    y: i32,
    w: i32,
    first: &'static str,
    second: &'static str,
) -> i32 {
    if w < 600 {
        field(scene, ui, x, y, w, first);
        field(scene, ui, x, y + 78, w, second);
        y + 156
    } else {
        field(scene, ui, x, y, (w - 16) / 2, first);
        field(scene, ui, x + (w + 16) / 2, y, (w - 16) / 2, second);
        y + 78
    }
}
fn notice(scene: &mut Scene, x: i32, y: i32, w: i32, message: &str) {
    scene.label(x, y, w, message, 1);
}
fn settings_action_card(
    scene: &mut Scene,
    ui: &Ui,
    x: i32,
    y: i32,
    w: i32,
    title: &str,
    description: &str,
    button: &str,
    action: Action,
) -> i32 {
    let narrow = w < 600;
    let text_w = if narrow { w - 32 } else { w - 180 };
    let titles = wrap_text(ui.t(title), text_w, 14);
    let descriptions = if description.is_empty() {
        Vec::new()
    } else {
        wrap_text(ui.t(description), text_w, 14)
    };
    let text_h = titles.len() as i32 * 24 + descriptions.len() as i32 * 22;
    let h = if narrow {
        text_h + 60
    } else {
        (text_h + 20).max(54)
    };
    scene.panel(x, y, w, h, 8);
    let mut line_y = y + 10;
    for line in titles {
        scene.label_sized(x + 16, line_y, text_w, 24, line, 0);
        line_y += 24;
    }
    for line in descriptions {
        scene.label_sized(x + 16, line_y, text_w, 22, line, 1);
        line_y += 22;
    }
    scene.button(
        Rect::new(
            if narrow { x + 16 } else { x + w - 148 },
            if narrow { line_y + 8 } else { y + (h - 34) / 2 },
            132.min(w - 32),
            34,
        ),
        ui.t(button),
        "",
        action,
        false,
    );
    y + h + 8
}

fn deezer_card(
    scene: &mut Scene,
    ui: &Ui,
    x: i32,
    y: i32,
    w: i32,
    name: &str,
    profile: Option<&str>,
    login: Action,
    refresh: Option<Action>,
    busy: bool,
) -> i32 {
    let narrow = w < 500;
    let text_w = if narrow { w - 104 } else { w - 292 };
    let status = match profile {
        Some(name) if !name.is_empty() => format!("{} {name}", ui.t("Connected as")),
        Some(_) => ui.t("Connected").into(),
        None => ui.t("Not signed in").into(),
    };
    let lines = wrap_text(&status, text_w.max(40), 14);
    let text_height = 42 + lines.len().max(1) as i32 * 22;
    let height = text_height.max(78) + if narrow { 48 } else { 0 };
    scene.panel(x, y, w, height, 8);
    scene.button(
        Rect::new(x + 16, y + 12, 52, 52),
        "",
        "",
        Action::DeezerLogo,
        false,
    );
    if let Some(button) = scene.buttons.last_mut() {
        button.enabled = false;
        button.accessible = name.into();
    }
    scene.label_sized(x + 84, y + 10, text_w, 26, name, 0);
    for (index, line) in lines.into_iter().enumerate() {
        scene.label_sized(x + 84, y + 36 + index as i32 * 22, text_w, 22, line, 1);
    }
    let action_y = if narrow {
        y + height - 42
    } else {
        y + (height - 34) / 2
    };
    let can_refresh = refresh.is_some();
    {
        scene.button(
            Rect::new(x + w - 192, action_y, 34, 34),
            "",
            "\u{e72c}",
            refresh.unwrap_or(Action::Notice("Sign in before refreshing this provider.")),
            false,
        );
        if let Some(button) = scene.buttons.last_mut() {
            button.enabled = can_refresh && !busy;
            button.accessible = format!("Refresh {name}");
        }
    }
    scene.button(
        Rect::new(x + w - 148, action_y, 132, 34),
        if profile.is_some() {
            "Log out"
        } else {
            "Log in"
        },
        "",
        login,
        false,
    );
    if let Some(button) = scene.buttons.last_mut() {
        button.enabled = !busy;
    }
    y + height + 8
}
fn hex_color(c: u32) -> String {
    format!(
        "#{:06X}",
        ((c & 255) << 16) | (c & 0xff00) | ((c >> 16) & 255)
    )
}
fn toggle_row(
    scene: &mut Scene,
    ui: &Ui,
    x: i32,
    y: i32,
    w: i32,
    label: &str,
    sublabel: &str,
    key: &'static str,
    on: bool,
) -> i32 {
    let titles = wrap_text(ui.t(label), w - 110, 14);
    let descriptions = if sublabel.is_empty() {
        Vec::new()
    } else {
        wrap_text(ui.t(sublabel), if w < 600 { w - 32 } else { w - 110 }, 14)
    };
    let title_h = titles.len().max(1) as i32 * 24;
    let h = (16 + title_h + descriptions.len() as i32 * 22).max(48);
    scene.panel(x, y, w, h, 8);
    for (index, line) in titles.iter().enumerate() {
        scene.label_sized(x + 16, y + 8 + index as i32 * 24, w - 110, 24, line, 0);
    }
    for (index, line) in descriptions.iter().enumerate() {
        scene.label_sized(
            x + 16,
            y + 8 + title_h + index as i32 * 22,
            if w < 600 { w - 32 } else { w - 110 },
            22,
            line,
            1,
        );
    }
    scene.button(
        Rect::new(x + w - 76, y + 10, 60, 28),
        if on { "On" } else { "Off" },
        "",
        Action::ToggleBool(key),
        on,
    );
    y + h + 8
}

fn settings(scene: &mut Scene, ui: &Ui, x: i32, y: i32, w: i32, connected: bool, busy: bool) {
    if !connected && ui.tab != 1 {
        scene.session = Some(Rect::new(x + w - 340, 108, 180, 32));
        scene.button(
            Rect::new(x + w - 150, 106, 140, 36),
            if busy { "Connecting..." } else { "Connect" },
            "\u{e77b}",
            Action::Connect,
            true,
        );
    }

    let narrow = w < 600;
    let cols = if narrow { 6 } else { (w / 140).clamp(2, 6) };
    let tab_w = (w - (cols - 1) * 8) / cols;
    let mut col = 0;
    let mut row = 0;
    for (index, title) in TABS.iter().enumerate() {
        let bx = x + col * (tab_w + 8);
        let by = y + row * 42;
        scene.button(
            Rect::new(bx, by, tab_w, 36),
            if narrow { "" } else { *title },
            TAB_ICONS[index],
            Action::Tab(index),
            index == ui.tab,
        );
        if let Some(button) = scene.buttons.last_mut() {
            button.accessible = ui.t(title).into();
        }
        col += 1;
        if col >= cols {
            col = 0;
            row += 1;
        }
    }
    let total_rows = (TABS.len() as i32 + cols - 1) / cols;
    let by = y + (total_rows - 1) * 42;
    let content_top = (by + 46).max(226);
    scene.content_top = content_top;
    let content_start = (scene.buttons.len(), scene.labels.len(), scene.panels.len());
    let picture_start = scene.pictures.len();
    let top = (by + 56).max(236) - ui.scroll;
    match ui.tab {
        0 => {
            scene.label(x, top, w, "STARTUP", 9);
            let mut cy = top + 28;
            field(scene, ui, x, cy, w, "Show on startup");
            cy += 78;
            field(scene, ui, x, cy, w, "Language");
            cy += 72;
            notice(scene, x, cy, w, "Language changes apply immediately.");
            cy += 44;
            scene.label(x, cy, w, "WINDOW", 9);
            cy += 28;
            cy = toggle_row(
                scene,
                ui,
                x,
                cy,
                w,
                "Keep playing on close",
                "Hide the window instead of quitting when closing",
                "keep_playing_on_close",
                ui.keep_playing_on_close,
            );
            cy = toggle_row(
                scene,
                ui,
                x,
                cy,
                w,
                "Show in the system tray",
                "",
                "show_in_tray",
                ui.show_in_tray,
            );
            cy = toggle_row(
                scene,
                ui,
                x,
                cy,
                w,
                "Window controls",
                "",
                "window_controls",
                ui.window_controls,
            );
            scene.panel(x, cy, w, if narrow { 86 } else { 48 }, 8);
            scene.label(
                x + 16,
                cy + 11,
                if narrow { w - 32 } else { w - 200 },
                "Control alignment",
                0,
            );
            let align_x = if narrow { x + 16 } else { x + w - 164 };
            let align_y = cy + if narrow { 44 } else { 9 };
            scene.button(
                Rect::new(align_x, align_y, 72, 30),
                "Left",
                "",
                Action::SetControlAlignment(false),
                !ui.controls_right,
            );
            scene.button(
                Rect::new(align_x + 78, align_y, 72, 30),
                "Right",
                "",
                Action::SetControlAlignment(true),
                ui.controls_right,
            );
            cy += if narrow { 94 } else { 56 };
            cy = toggle_row(
                scene,
                ui,
                x,
                cy,
                w,
                "Traffic light controls",
                "",
                "traffic_lights",
                ui.traffic_lights,
            );
            cy += 8;
            scene.label(x, cy, w, "SCROLLING", 9);
            cy += 28;
            cy = toggle_row(
                scene,
                ui,
                x,
                cy,
                w,
                "Smooth scrolling",
                "Smoothly animate mouse wheel and trackpad scrolling",
                "smooth_scrolling",
                ui.smooth_scrolling,
            );
            cy = toggle_row(
                scene,
                ui,
                x,
                cy,
                w,
                "Scroll animation",
                "Animate scroll position transitions when navigating",
                "scroll_animation",
                ui.scroll_animation,
            );
            cy += 16;
            scene.label(x, cy, w, "STORAGE", 9);
            cy += 28;
            cy = toggle_row(
                scene,
                ui,
                x,
                cy,
                w,
                "Keep played tracks",
                "Cache streamed tracks on disk for faster replay",
                "keep_played_tracks",
                ui.keep_played_tracks,
            );
            cy = settings_action_card(
                scene,
                ui,
                x,
                cy,
                w,
                "Played tracks cache: 13.7 MB",
                "",
                "Clear",
                Action::Notice("Played track cache cleared."),
            );
            cy = settings_action_card(
                scene,
                ui,
                x,
                cy,
                w,
                &format!("Downloads: {} saved tracks", ui.offline.len()),
                "",
                "Remove all",
                Action::Notice("Offline downloads preserved."),
            );
            scene.button(
                Rect::new(x, cy, 270.min(w), 36),
                "Choose data / cache location...",
                "",
                Action::Go(View::Storage),
                false,
            );
            scene.content_height = cy + 64 + ui.scroll;
        }
        1 => {
            let mut cy = top;

            let name = crate::catalog::string(
                &ui.account,
                &["name", "display_name", "userName", "userId"],
            );
            cy = deezer_card(
                scene,
                ui,
                x,
                cy,
                w,
                "Deezer",
                connected.then_some(name.as_str()),
                if connected {
                    Action::Logout
                } else {
                    Action::Go(View::Account)
                },
                connected.then_some(Action::ConnectSaved),
                busy,
            );
            scene.content_height = cy + 40 + ui.scroll;
        }
        2 => {
            scene.button(
                Rect::new(x, top, (w - 12) / 2, 36),
                "Import theme",
                "",
                Action::ImportTheme,
                false,
            );
            scene.button(
                Rect::new(x + (w + 12) / 2, top, (w - 12) / 2, 36),
                "Export theme",
                "",
                Action::ExportTheme,
                false,
            );
            let top = top + 56;
            scene.label(x, top, w, "BACKDROP", 9);
            let b_cols = if w < 480 { 2 } else { 4 };
            let card_w = (w - (b_cols - 1) * 8) / b_cols;
            for (index, label) in ["Acrylic", "Song cover", "Custom image", "Solid color"]
                .iter()
                .enumerate()
            {
                let row = index as i32 / b_cols;
                let col = index as i32 % b_cols;
                scene.button(
                    Rect::new(x + col * (card_w + 8), top + 28 + row * 48, card_w, 42),
                    *label,
                    "",
                    Action::BackgroundMode(index),
                    ui.background_mode == index,
                );
            }
            let b_rows = (4 + b_cols - 1) / b_cols;
            let mut cy = top + 28 + b_rows * 48 + 14;
            cy = toggle_row(
                scene,
                ui,
                x,
                cy,
                w,
                "Light mode",
                "Use lighter surfaces and dark text",
                "light_mode",
                ui.light,
            );
            if ui.background_mode == 2 {
                scene.button(
                    Rect::new(x, cy, 260.min(w), 38),
                    "Choose background image...",
                    "",
                    Action::BackgroundImage,
                    false,
                );
                let filename = std::path::Path::new(&ui.background_file)
                    .file_name()
                    .and_then(|p| p.to_str())
                    .unwrap_or("No image selected");
                if narrow {
                    scene.label(x, cy + 44, w, filename, 1);
                    cy += 78;
                } else {
                    scene.label(x + 274, cy + 6, w - 274, filename, 1);
                    cy += 54;
                }
            }
            if matches!(ui.background_mode, 1 | 2) {
                scene.label(
                    x,
                    cy,
                    w,
                    format!("{}: {}%", ui.t("Image blur"), ui.blur),
                    0,
                );
                scene.sliders.push((
                    Rect::new(x, cy + 28, w.min(470), 30),
                    "Image blur",
                    ui.blur,
                ));
                cy += 68;
            }

            if ui.background_mode != 3 {
                scene.label(
                    x,
                    cy,
                    w,
                    format!("{}: {}%", ui.t("Background tint opacity"), ui.light_opacity),
                    0,
                );
                scene.sliders.push((
                    Rect::new(x, cy + 28, w.min(470), 30),
                    "Background tint opacity",
                    ui.light_opacity,
                ));
                cy += 68;
                scene.label(
                    x,
                    cy,
                    w,
                    format!("{}: {}%", ui.t("Background brightness"), ui.brightness),
                    0,
                );
                scene.sliders.push((
                    Rect::new(x, cy + 28, w.min(470), 30),
                    "Background brightness",
                    ui.brightness,
                ));
                cy += 68;
            }
                scene.label(x, cy, w, "THEMES", 9);
                cy += 28;
                let cols = if narrow { 1 } else { (w / 168).clamp(2, 4) };
                let card_w = (w - (cols - 1) * 10) / cols;
                let visible = if ui.expanded_themes {
                    SOLID_THEMES.len()
                } else {
                    cols as usize
                };
                let mut themes: Vec<_> = SOLID_THEMES.iter().enumerate().collect();
                themes.sort_by_key(|(_, theme)| theme.1 != ui.light);
                for (cell, (idx, (name, _, _, _, _, _, _, _))) in
                    themes.into_iter().take(visible).enumerate()
                {
                    let row = cell as i32 / cols;
                    let col = cell as i32 % cols;
                    scene.button(
                        Rect::new(x + col * (card_w + 10), cy + row * 82, card_w, 72),
                        *name,
                        "",
                        Action::SolidTheme(idx),
                        ui.solid_theme == idx,
                    );
                }
                let rows = (visible.min(SOLID_THEMES.len()) as i32 + cols - 1) / cols;
                cy += rows * 82 + 16;
                if SOLID_THEMES.len() > cols as usize {
                    let button_w = w.min(180);
                    scene.button(
                        Rect::new(x + (w - button_w) / 2, cy, button_w, 34),
                        ui.t(if ui.expanded_themes {
                            "See less"
                        } else {
                            "See more"
                        }),
                        "",
                        Action::ToggleBool("expand_themes"),
                        false,
                    );
                    cy += 50;
                }

            scene.label(x, cy, w, "Color customization", 3);
            cy += 38;
            scene.button(
                Rect::new(x, cy, 240.min(w), 36),
                "Customize colors...",
                "\u{e790}",
                Action::Go(View::CustomColors),
                false,
            );
            cy += 48;
            scene.button(
                Rect::new(x, cy, 230.min(w), 34),
                "Reset this theme's colors",
                "",
                Action::ResetTheme,
                false,
            );
            cy += 52;

            // Corner roundedness
            scene.label(x, cy, w, "CORNERS", 9);
            cy += 28;
            let c_cols = if w < 480 { 2 } else { 4 };
            let cw_card = (w - (c_cols - 1) * 8) / c_cols;
            for (idx, label) in ["Sharp", "Slight", "Rounded", "Round"].iter().enumerate() {
                let row = idx as i32 / c_cols;
                let col = idx as i32 % c_cols;
                scene.button(
                    Rect::new(x + col * (cw_card + 8), cy + row * 70, cw_card, 64),
                    *label,
                    "",
                    Action::CornerStyle(idx),
                    ui.corner_style == idx,
                );
            }
            let c_rows = (4 + c_cols - 1) / c_cols;
            cy += c_rows * 70 + 12;

            scene.label(x, cy, w, format!("{}: {}%", ui.t("Surface opacity"), ui.surface_opacity), 0);
            scene.sliders.push((Rect::new(x, cy + 28, w.min(470), 30), "Surface opacity", ui.surface_opacity));
            cy += 68;
            // Player bar layout
            cy += 10;
            scene.label(x, cy, w, "PLAYER BAR", 9);
            cy += 28;
            let pw = ((w - 12) / 2).clamp(160, 260);
            for (idx, label) in ["Normal", "Compact"].iter().enumerate() {
                scene.button(
                    Rect::new(x + idx as i32 * (pw + 12), cy, pw, 64),
                    *label,
                    "",
                    Action::PlayerLayout(idx),
                    ui.player_layout == idx,
                );
            }
            cy += 76;
            cy = toggle_row(
                scene,
                ui,
                x,
                cy,
                w,
                "Transparent player bar",
                "",
                "transparent_player",
                ui.transparent_player,
            );
            cy += 8;
            scene.content_height = cy + 40 + ui.scroll;
        }
        3 => {
            // Lyrics tab with live preview card
            let mut cy = top;
            scene.panel(x, cy, w, 138, 12);
            scene.label(
                x + 28,
                cy + 16,
                w - 56,
                "I've been staring at the neon sky",
                if ui.blur_inactive_lyrics { 12 } else { 1 },
            );
            scene.label_sized(
                x + 28,
                cy + 50,
                w - 56,
                40,
                &crate::lyrics::karaoke_example().text,
                13,
            );
            scene.karaoke_preview = Some(scene.labels.len() - 1);
            scene.label(
                x + 28,
                cy + 92,
                w - 56,
                "Until the line fades back into the night",
                if ui.blur_inactive_lyrics { 12 } else { 1 },
            );
            cy += 156;
            scene.label(x, cy, w, "DISPLAY", 9);
            cy += 28;
            cy = toggle_row(
                scene,
                ui,
                x,
                cy,
                w,
                "Karaoke lyrics",
                "Light up syllables progressively while the active line is sung",
                "karaoke_lyrics",
                ui.karaoke_lyrics,
            );
            cy = toggle_row(
                scene,
                ui,
                x,
                cy,
                w,
                "Blur inactive lyrics",
                "Softly blur unsung and finished lyric lines",
                "blur_inactive_lyrics",
                ui.blur_inactive_lyrics,
            );
            for (is_full, label, val) in [
                (false, "Lyrics size (panel)", ui.lyrics_size_panel),
                (true, "Lyrics size (fullscreen)", ui.lyrics_size_fullscreen),
            ] {
                scene.panel(x, cy, w, if narrow { 86 } else { 48 }, 8);
                scene.label(
                    x + 16,
                    cy + 11,
                    if narrow { w - 32 } else { w - 190 },
                    label,
                    0,
                );
                let controls_y = cy + if narrow { 44 } else { 9 };
                scene.button(
                    Rect::new(x + w - 150, controls_y, 34, 30),
                    "-",
                    "",
                    Action::AdjustLyricsSize(is_full, -10),
                    false,
                );
                scene.label(x + w - 108, controls_y + 2, 56, format!("{val}%"), 0);
                scene.button(
                    Rect::new(x + w - 50, controls_y, 34, 30),
                    "+",
                    "",
                    Action::AdjustLyricsSize(is_full, 10),
                    false,
                );
                cy += if narrow { 94 } else { 56 };
            }
            cy += 8;
            scene.label(x, cy, w, "VISUALIZER", 9);
            cy += 28;
            cy = toggle_row(
                scene,
                ui,
                x,
                cy,
                w,
                "Show visualizer on lyrics page",
                "Display responsive audio visualizer underneath lyrics and controls",
                "lyrics_visualizer",
                ui.lyrics_visualizer,
            );
            if w < 520 {
                scene.panel(x, cy, w, 84, 8);
                scene.label(x + 16, cy + 10, w - 32, "Visualizer style", 0);
                let vis_w = (w - 32 - 16) / 3;
                for (idx, (name, icon)) in [
                    ("Wave", "\u{e9d9}"),
                    ("Bars", "\u{e9e9}"),
                    ("Trap Nation", "\u{ea3b}"),
                ]
                .iter()
                .enumerate()
                {
                    scene.button(
                        Rect::new(x + 16 + idx as i32 * (vis_w + 8), cy + 40, vis_w, 34),
                        *name,
                        *icon,
                        Action::VisualizerStyle(idx as u8),
                        ui.lyrics_visualizer && ui.visualizer_style == idx as u8,
                    );
                }
                cy += 96;
            } else {
                scene.panel(x, cy, w, 52, 8);
                scene.label(x + 16, cy + 14, 150, "Visualizer style", 0);
                let vis_btn_w = ((w - 180) / 3).clamp(80, 140);
                for (idx, (name, icon)) in [
                    ("Wave", "\u{e9d9}"),
                    ("Bars", "\u{e9e9}"),
                    ("Trap Nation", "\u{ea3b}"),
                ]
                .iter()
                .enumerate()
                {
                    let vx = x + w - 16 - (3 - idx as i32) * (vis_btn_w + 6);
                    scene.button(
                        Rect::new(vx, cy + 10, vis_btn_w, 32),
                        *name,
                        *icon,
                        Action::VisualizerStyle(idx as u8),
                        ui.lyrics_visualizer && ui.visualizer_style == idx as u8,
                    );
                }
                cy += 64;
            }
            cy += 12;
            let selected_count = (0..crate::lyrics::LYRIC_SOURCES.len())
                .filter(|i| {
                    (crate::lyrics::SUPPORTED_SOURCES & (1 << i)) != 0
                        && (ui.lyrics_sources & (1 << i)) != 0
                })
                .count();
            scene.label(x, cy, w, format!("SOURCES ({selected_count} selected)"), 9);
            cy += 28;
            for (idx, source) in crate::lyrics::LYRIC_SOURCES.iter().enumerate() {
                let is_supported = (crate::lyrics::SUPPORTED_SOURCES & (1 << idx)) != 0;
                let enabled = is_supported && (ui.lyrics_sources & (1 << idx)) != 0;
                scene.panel(x, cy, w, 44, 8);
                let label_text = if is_supported {
                source.to_string()
                } else {
                    format!("{source} (Unavailable)")
                };
                scene.label(
                    x + 16,
                    cy + 9,
                    w - 110,
                    label_text,
                    if is_supported { 0 } else { 1 },
                );
                scene.button(
                    Rect::new(x + w - 76, cy + 8, 60, 28),
                    if enabled { "On" } else { "Off" },
                    "",
                    Action::ToggleLyricSource(idx),
                    enabled,
                );
                if let Some(btn) = scene.buttons.last_mut() {
                    btn.enabled = is_supported;
                }
                cy += 50;
            }
    scene.content_height = cy + 40 + ui.scroll;
        }
        4 => {
            // Playback + Equalizer at bottom
            let mut cy = top;
            scene.label(x, cy, w, "PLAYING", 9);
            cy += 28;
            cy = field_pair(
                scene,
                ui,
                x,
                cy,
                w,
                "Normalize volume (ReplayGain)",
                "Gapless playback",
            );
            cy = field_pair(scene, ui, x, cy, w, "Crossfade (seconds)", "Sleep timer");
            cy = toggle_row(scene, ui, x, cy, w, "Autoplay", "Continue with related songs, one at a time, when the queue ends", "autoplay", ui.autoplay);
            cy = toggle_row(
                scene,
                ui,
                x,
                cy,
                w,
                "Stay awake while playing",
                "Prevent Windows from sleeping during active playback",
                "stay_awake",
                ui.stay_awake,
            );
            cy += 12;
            scene.label(x, cy, w, "STREAMING QUALITY", 9);
            cy += 28;
            let q_cols = if w < 480 { 1 } else { 3 };
            let qw = (w - (q_cols - 1) * 10) / q_cols;
            let q_selected = ui.value("Audio quality").min(2);
            for (idx, (title, sub)) in [
                ("Normal", "MP3, 128 kbps"),
                ("High", "MP3, 320 kbps"),
                ("Lossless", "FLAC, 16-bit 44.1 kHz"),
            ]
            .iter()
            .enumerate()
            {
                let row = idx as i32 / q_cols;
                let col = idx as i32 % q_cols;
                scene.button(
                    Rect::new(x + col * (qw + 10), cy + row * 74, qw, 68),
                    format!("{title} \u{00b7} {sub}"),
                    "\u{e9d9}",
                    Action::StreamingQuality(idx),
                    q_selected == idx,
                );
            }
            let q_rows = (3 + q_cols - 1) / q_cols;
            cy += q_rows * 74 + 10;
            scene.label(x, cy, w, "OUTPUT", 9);
            cy += 28;
            field(scene, ui, x, cy, w, "Audio output device");
            cy += 76;

            // EQUALIZER at bottom of Playback
            scene.label(x, cy, 220, "EQUALIZER", 9);
            scene.button(
                Rect::new(x + w - 80, cy - 4, 80, 30),
                "Reset",
                "\u{e777}",
                Action::EqReset,
                false,
            );
            cy += 36;
            cy = field_pair(scene, ui, x, cy, w, "Equalizer", "Preset");

            let graph_h = 160;
            scene.eq_graph = Some(Rect::new(x, cy, w, graph_h));
            scene.panel(x, cy, w, graph_h, 7);
            cy += graph_h + 16;

            let sliders_h = 170;
            scene.eq_sliders = Some(Rect::new(x, cy, w, sliders_h));
            scene.panel(x, cy, w, sliders_h, 13);
            cy += sliders_h + 20;

            scene.content_height = cy + 20 + ui.scroll;
        }
        _ => {
            // Integrations
            let mut cy = top;
            cy = toggle_row(scene, ui, x, cy, w, "DeezNats Connect", "Share playback and controls on your local network", "jam_enabled", ui.jam_enabled);
            if ui.jam_enabled && !ui.jam_url.is_empty() {
                scene.label(x, cy + 4, w, "Listening on your network", 9);
                scene.label(x, cy + 28, w, &ui.jam_url, 1);
                let button_w = ((w - 12) / 2).min(180);
                scene.button(Rect::new(x, cy + 58, button_w, 34), "Open Connect", "", Action::OpenConnect, false);
                scene.button(Rect::new(x + button_w + 12, cy + 58, button_w, 34), "Copy link", "", Action::CopyConnectLink, false);
                cy += 108;
            }
            cy += 16;
            scene.label(x, cy, w, "DISCORD RICH PRESENCE", 9);
            cy += 28;
            // Rich Presence Preview Card
            scene.panel(x, cy, w, if narrow { 154 } else { 106 }, 10);
            if !ui.now.artwork.is_empty() {
                scene
                    .pictures
                    .push((Rect::new(x + 16, cy + 15, 76, 76), ui.now.artwork.clone()));
            } else {
                scene.panel(x + 16, cy + 15, 76, 76, 1);
            }
            let preview_text_w = if narrow { w - 124 } else { w - 260 };
            scene.label(x + 108, cy + 12, preview_text_w, "Listening to DeezNats", 9);
            scene.label(
                x + 108,
                cy + 34,
                preview_text_w,
                if ui.now.title.is_empty() {
                    " Midnight City"
                } else {
                    &ui.now.title
                },
                3,
            );
            scene.label(
                x + 108,
                cy + 62,
                preview_text_w,
                if ui.now.subtitle.is_empty() {
                    "by M83 \u{00b7} Hurry Up, We're Dreaming"
                } else {
                    &ui.now.subtitle
                },
                1,
            );
            scene.button(
                Rect::new(
                    if narrow { x + 16 } else { x + w - 148 },
                    cy + if narrow { 106 } else { 16 },
                    132,
                    32,
                ),
                "Listen on Deezer",
                "",
                Action::Notice("Discord Rich Presence template preview."),
                false,
            );
            scene.button(
                Rect::new(
                    if narrow { x + 160 } else { x + w - 148 },
                    cy + if narrow { 106 } else { 56 },
                    132,
                    32,
                ),
                "Get DeezNats",
                "",
                Action::Notice("Discord Rich Presence template preview."),
                false,
            );
            cy += if narrow { 168 } else { 120 };
            cy = toggle_row(
                scene,
                ui,
                x,
                cy,
                w,
                "Show on Discord",
                "Display your currently playing track in Discord status",
                "show_on_discord",
                ui.show_on_discord,
            );
            scene.label(x, cy + 6, w, "STATUS", 9);
            cy += 34;
            field(scene, ui, x, cy, w, "Status name");
            cy += 76;
            cy = toggle_row(
                scene,
                ui,
                x,
                cy,
                w,
                "Show when paused",
                "",
                "discord_paused",
                ui.discord_paused,
            );
            cy = toggle_row(
                scene,
                ui,
                x,
                cy,
                w,
                "Show service badge",
                "",
                "discord_badge",
                ui.discord_badge,
            );
            cy = toggle_row(
                scene,
                ui,
                x,
                cy,
                w,
                "Hide details",
                "",
                "discord_hide_details",
                ui.discord_hide_details,
            );
            scene.label(x, cy + 6, w, "BUTTONS & LOCAL FILES", 9);
            cy += 34;
            cy = toggle_row(
                scene,
                ui,
                x,
                cy,
                w,
                "Service button",
                "",
                "discord_btn_service",
                ui.discord_btn_service,
            );
            cy = toggle_row(
                scene,
                ui,
                x,
                cy,
                w,
                "DeezNats button",
                "",
                "discord_btn_app",
                ui.discord_btn_app,
            );

            cy += 12;

            cy += 28;

            scene.content_height = cy + 40 + ui.scroll;
        }
    }
    scene.content_buttons = content_start.0..scene.buttons.len();
    scene.content_labels = content_start.1..scene.labels.len();
    scene.content_panels = content_start.2..scene.panels.len();
    scene.content_pictures = picture_start..scene.pictures.len();
}
fn entity_caption<'a>(
    entity: &'a crate::catalog::Entry,
    page: Option<&'a crate::catalog::Page>,
    fallback: &'a str,
    ui: &'a Ui,
) -> String {
    if entity.kind == "artist" {
        let fans = crate::catalog::string(&entity.raw, &["nb_fan", "fans", "followers"]);
        let word = ui.t("followers");
        if !fans.is_empty() && fans != "0" {
            let count_str = if let Ok(n) = fans.parse::<u64>() {
                crate::catalog::format_count(n)
            } else {
                fans
            };
            return format!("{count_str} {word}");
        }
        if !entity.subtitle.is_empty() {
            let s = &entity.subtitle;
            if s.ends_with(" followers") || s.ends_with(" fans") {
                let num = s.trim_end_matches(" followers").trim_end_matches(" fans");
                return format!("{num} {word}");
            }
            return entity.subtitle.clone();
        }
        return String::new();
    }
    let mut parts = Vec::new();
    if !entity.subtitle.is_empty() {
        parts.push(entity.subtitle.clone());
    }
    if let Some(page) = page {
        if !page.rows.is_empty() {
            let total_ms: u64 = page.rows.iter().map(|r| r.duration).sum();
            let mins = total_ms / 60_000;
            let podcast = entity.kind == "podcast";
            let count = if podcast {
                entity.episode_count().unwrap_or(page.rows.len() as u64)
            } else {
                page.rows.len() as u64
            };
            let noun = ui.t(if podcast {
                if count == 1 { "episode" } else { "episodes" }
            } else if count == 1 { "track" } else { "tracks" });
            let unit = ui.t("min");
            if mins > 0 && !podcast {
                parts.push(format!("{count} {noun} \u{00b7} {mins} {unit}"));
            } else {
                parts.push(format!("{count} {noun}"));
            }
        }
    }
    let release =
        crate::catalog::string(&entity.raw, &["release_date", "releaseDate", "publishedAt"]);
    if !release.is_empty() {
        parts.push(release);
    }
    let fans = crate::catalog::string(&entity.raw, &["nb_fan", "fans", "followers"]);
    if !fans.is_empty() && fans != "0" {
        parts.push(format!("{fans} fans"));
    }
    if parts.is_empty() {
        fallback.to_owned()
    } else {
        parts.join(" \u{00b7} ")
    }
}
fn row_date(item: &crate::catalog::Entry, podcast: bool) -> String {
    let release = [
        "releaseDate",
        "release_date",
        "publicationDate",
        "publication_date",
        "publishedAt",
        "published_at",
        "RELEASE_DATE",
    ];
    if podcast {
        crate::catalog::string(&item.raw, &release)
    } else {
        let added = crate::catalog::string(
            &item.raw,
            &[
                "DATE_ADD",
                "date_add",
                "addedAt",
                "date_added",
                "time_add",
                "TIME_ADD",
            ],
        );
        if added.is_empty() {
            crate::catalog::string(&item.raw, &release)
        } else {
            added
        }
    }
}
fn format_date(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    if let Ok(mut secs) = trimmed.parse::<u64>() {
        if secs > 100_000_000_000 {
            secs /= 1000;
        }
        if (1_000_000_000..=2_500_000_000).contains(&secs) {
            let days = (secs / 86400) as i64;
            let z = days + 719468;
            let era = (if z >= 0 { z } else { z - 146096 }) / 146097;
            let doe = (z - era * 146097) as u32;
            let yoe = (doe - doe / 1029 + doe / 146100) / 365;
            let y = yoe as i64 + era * 400;
            let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
            let mp = (5 * doy + 2) / 153;
            let d = doy - (153 * mp + 2) / 5 + 1;
            let m = if mp < 10 { mp + 3 } else { mp - 9 };
            let y = if m <= 2 { y + 1 } else { y };
            return format!("{:04}-{:02}-{:02}", y, m, d);
        }
    }
    if trimmed.len() >= 10 && trimmed.as_bytes()[4] == b'-' {
        return trimmed[..10].to_string();
    }
    trimmed.to_string()
}
fn queue_lines(entry: &crate::catalog::Entry, _width: i32) -> Vec<String> {
    let mut metadata = entry.subtitle.clone();
    if entry.duration > 0 {
        let seconds = entry.duration / 1000;
        metadata.push_str(&format!(" \u{2022} {}:{:02}", seconds / 60, seconds % 60));
    }
    if let Some(name) = entry.raw["added_by"].as_str().filter(|s| !s.is_empty()) {
        metadata.push_str(&format!(" \u{2022} Added by {name}"));
    }
    vec![entry.title.clone(), metadata]
}
fn queue_text_width(_entry: &crate::catalog::Entry, row_width: i32) -> i32 {
    (row_width - 96).max(1)
}
fn queue_button(s: &mut Scene, entry: &crate::catalog::Entry, index: usize, rect: Rect, current: bool) {
    let text_w = queue_text_width(entry, rect.w);
    s.button(rect, "", "", Action::QueueJump(index), current);
    if let Some(btn) = s.buttons.last_mut() {
        btn.accessible = format!("{}: {}", if current { "Now playing" } else { "Play next" }, queue_lines(entry, text_w).join(" "));
    }
    // Artwork has its own bounds: button artwork fills the entire button.
    if !entry.artwork.is_empty() && rect.h >= 64 {
        s.pictures.push((Rect::new(rect.x + 26, rect.y + 10, 44, 44), entry.artwork.clone()));
    }
    if rect.h >= 64 {
        for (line, text) in queue_lines(entry, text_w).into_iter().enumerate() {
            s.label_sized(rect.x + 82, rect.y + 10 + line as i32 * 24, text_w, 24, text, if line == 0 { 0 } else { 1 });
        }
    }
}
fn build_queue_sidebar(s: &mut Scene, ui: &Ui, qx: i32, qw: i32, h: i32) {
    let start = s.labels.len();
    build_queue_sidebar_contents(s, ui, qx, qw, h);
    s.queue_labels = start..s.labels.len();
}
fn build_queue_sidebar_contents(s: &mut Scene, ui: &Ui, qx: i32, qw: i32, h: i32) {
    let bottom = h - if ui.view == View::Lyrics { 12 } else { 96 };
    s.panel(qx, 10, qw, bottom - 20, 0);
    // Reserve action space first, then let the localized title use the rest.
    // Icon actions keep a predictable hit area regardless of translation length.
    let action_size = 30;
    let action_gap = 6;
    let clear_x = qx + qw - 12 - action_size;
    let reset_x = clear_x - action_gap - action_size;
    let title_x = if qw >= 260 {
        s.button(
            Rect::new(qx + 10, 24, 30, 30),
            "",
            "\u{e8fd}",
            Action::ToggleQueueSidebar,
            false,
        );
        if let Some(button) = s.buttons.last_mut() {
            button.accessible = "Close queue".into();
        }
        qx + 44
    } else {
        qx + 14
    };
    s.label_sized(title_x, 26, (reset_x - title_x - 8).max(1), 28, "QUEUE", 0);
    for (x, glyph, action, name) in [
        (reset_x, "\u{e777}", Action::QueueReset, "Reset"),
        (clear_x, "\u{e74d}", Action::QueueClear, "Clear"),
    ] {
        s.button(
            Rect::new(x, 24, action_size, action_size),
            "",
            glyph,
            action,
            false,
        );
        if let Some(button) = s.buttons.last_mut() {
            button.accessible = ui.t(name).into();
        }
    }
    let mut y = 68;
    let items: Vec<(usize, &crate::catalog::Entry)> = if !ui.queue.is_empty() {
        ui.queue.iter().enumerate().collect()
    } else if !ui.now.id.is_empty() {
        vec![(0, &ui.now)]
    } else {
        Vec::new()
    };
    if items.is_empty() {
        s.label(qx + 18, y + 24, qw - 36, "Queue is empty", 1);
        return;
    }
    let cur = if ui.queue_index < items.len()
        && (ui.now.id.is_empty() || items[ui.queue_index].1.id == ui.now.id)
    {
        ui.queue_index
    } else if !ui.now.id.is_empty() {
        items
            .iter()
            .position(|(_, e)| e.id == ui.now.id)
            .unwrap_or_else(|| ui.queue_index.min(items.len().saturating_sub(1)))
    } else {
        ui.queue_index.min(items.len().saturating_sub(1))
    };
    let current_h = 72;
    // Keep the current track pinned when it leaves room for upcoming tracks.
    // Oversized metadata joins the scroll area so every line remains reachable.
    let pinned = current_h + 150 < bottom - y;
    if pinned {
        s.label(qx + 18, y, qw - 36, "NOW PLAYING", 9);
        y += 24;
        let (idx, entry) = items[cur];
        queue_button(s, entry, idx, Rect::new(qx + 12, y, qw - 38, current_h.max(64)), true);
        y += current_h.max(64) + 12;
    }
    s.label(qx + 18, y, qw - 36, if pinned { "NEXT UP" } else { "NOW PLAYING" }, 9);
    y += 28;
    let viewport = Rect::new(qx + 12, y, qw - 24, (bottom - 18 - y).max(1));
    s.queue_viewport = Some(viewport);
    let upcoming: Vec<_> = items.iter().skip(cur + usize::from(pinned)).copied().collect();
    let heights = vec![72; upcoming.len()];
    let total: i32 = heights.iter().sum();
    let max = ((total - viewport.h).max(0) + 23) / 24;
    let offset = ui.queue_scroll.min(max as usize) as i32;
    s.queue_scroll_max = max as usize;
    let mut row_y = y - offset * 24;
    for ((idx, entry), height) in upcoming.iter().zip(heights) {
        let top = row_y.max(viewport.y);
        let end = (row_y + height - 6).min(viewport.y + viewport.h);
        if end > top {
            let rect = Rect::new(qx + 12, top, qw - 38, end - top);
            queue_button(s, entry, *idx, rect, *idx == items[cur].0);
        }
        row_y += height;
    }
    if max > 0 {
        let track = Rect::new(qx + qw - 20, viewport.y, 8, viewport.h);
        let thumb = (track.h * track.h / total.max(1)).max(24).min(track.h);
        s.queue_scrollbar = Some(track);
        s.panel(track.x, track.y + offset * (track.h - thumb) / max, 8, thumb, 17);
    }
    if upcoming.is_empty() { s.label(qx + 18, y + 8, qw - 36, "No upcoming songs", 1); }

}

fn build_lyrics_view(s: &mut Scene, ui: &Ui, w: i32, h: i32, playing: bool, volume: u32) {
    // 1. Back button in top-left corner
    s.button(
        Rect::new(28, 24, 38, 38),
        "",
        "\u{e72b}",
        Action::Back,
        false,
    );
    if let Some(btn) = s.buttons.last_mut() {
        btn.accessible = "Back".into();
    }

    // 2. Left column: Album art and integrated player controls (original deeznats layout)
    let col_w = (w * 44 / 100).clamp(200, 520).min((w - 240).max(180));
    let cover_size = (col_w - 28).clamp(140, 440).min((h - 220).max(140));
    let mode_mix=ui.lyrics_cover_mix.unwrap_or(if ui.lyrics_cover_mode{1.}else{0.}).clamp(0.,1.);
    let left_x=32+(col_w-cover_size)/2;
    let offset_x=(((w-cover_size)/2-left_x)as f32*mode_mix).round()as i32;
    let cover_x = left_x+offset_x;
    let block_h = cover_size + 130;
    let left_y=((h-block_h)/2).clamp(40,90);
    let cover_y=left_y+((((h-cover_size-160)/2).max(40)-left_y)as f32*mode_mix).round()as i32;

    let is_fs = ui.lyrics_fullscreen;
    let cover_rect = Rect::new(cover_x, cover_y, cover_size, cover_size);
    if !is_fs {
        s.lyrics_cover = Some(cover_rect);

        if !ui.now.artwork.is_empty() {
            s.pictures.push((
                cover_rect,
                crate::catalog::display_artwork_url(&ui.now.artwork, 1024, true),
            ));
        } else {
            s.panel(cover_x, cover_y, cover_size, cover_size, 1);
        }
    }

    // Controls directly on the cover (only visible on hover, matches original deeznats)
    if !is_fs && ui.lyrics_cover_hovered {
        let cx = cover_x + cover_size / 2;
        let cy = cover_y + cover_size / 2;
        let repeat_glyph = if ui.repeat == 2 {
            "\u{e8ed}"
        } else {
            "\u{e8ee}"
        };
        let play_glyph = if playing { "\u{e769}" } else { "\u{e768}" };

        let scale = (cover_size as f32 / 320.0).min(1.0);
        let play_sz = (52.0 * scale) as i32;
        let skip_sz = (40.0 * scale) as i32;
        let aux_sz = (34.0 * scale) as i32;
        let gap = (12.0 * scale) as i32;

        // Position Shuffle (leftmost)
        let shuf_x = cx - play_sz / 2 - gap - skip_sz - gap - aux_sz;
        s.button(
            Rect::new(shuf_x, cy - aux_sz / 2, aux_sz, aux_sz),
            "",
            "\u{e8b1}",
            Action::Shuffle,
            ui.shuffle,
        );
        if let Some(btn) = s.buttons.last_mut() {
            btn.accessible = "Shuffle".into();
        }

        // Position Previous
        let prev_x = cx - play_sz / 2 - gap - skip_sz;
        s.button(
            Rect::new(prev_x, cy - skip_sz / 2, skip_sz, skip_sz),
            "",
            "\u{e892}",
            Action::Previous,
            false,
        );
        if let Some(btn) = s.buttons.last_mut() {
            btn.accessible = "Previous".into();
        }

        // Position Play/Pause in center
        s.button(
            Rect::new(cx - play_sz / 2, cy - play_sz / 2, play_sz, play_sz),
            "",
            play_glyph,
            Action::Toggle,
            true,
        );
        if let Some(btn) = s.buttons.last_mut() {
            btn.accessible = if playing { "Pause" } else { "Play" }.into();
        }

        // Position Next
        let next_x = cx + play_sz / 2 + gap;
        s.button(
            Rect::new(next_x, cy - skip_sz / 2, skip_sz, skip_sz),
            "",
            "\u{e893}",
            Action::Next,
            false,
        );
        if let Some(btn) = s.buttons.last_mut() {
            btn.accessible = "Next".into();
        }

        // Position Repeat (rightmost)
        let rep_x = cx + play_sz / 2 + gap + skip_sz + gap;
        s.button(
            Rect::new(rep_x, cy - aux_sz / 2, aux_sz, aux_sz),
            "",
            repeat_glyph,
            Action::Repeat,
            ui.repeat > 0,
        );
        if let Some(btn) = s.buttons.last_mut() {
            btn.accessible = "Repeat".into();
        }
    }

    if !is_fs {
        // Seek bar: directly below cover (matches original deeznats)
        let seek_y = cover_y + cover_size + 18;
        let time_w = 36;
        let seek_left = cover_x + 38;
        let device_space = 0;
        let seek_w = (cover_size - 38 - time_w * 2 - 12 - device_space).max(20);
        s.button(
            Rect::new(cover_x, seek_y - 4, 30, 28),
            "",
            "\u{e767}",
            Action::ToggleLyricsVolume,
            ui.lyrics_volume_open,
        );
        if let Some(button) = s.buttons.last_mut() {
            button.accessible = format!("Volume {volume}%");
        }
        s.elapsed = Some(Rect::new(seek_left, seek_y, time_w, 20));
        s.seek = Some(Rect::new(seek_left + time_w + 6, seek_y, seek_w, 20));
        s.remaining = Some(Rect::new(cover_x + cover_size - time_w, seek_y, time_w, 20));

        if ui.lyrics_volume_open {
            let popup = Rect::new(cover_x - 4, seek_y - 172, 42, 164);
            s.volume_popup = Some(popup);
            s.volume = Some(Rect::new(popup.x + 8, popup.y + 12, 26, popup.h - 24));
            s.volume_vertical = true;
        }

        // Track Title (centered below seek bar)
        let title_y = seek_y + 30;
        let title_text = if ui.now.title.is_empty() {
            "No track playing"
        } else {
            &ui.now.title
        };
        s.label_sized(24+offset_x, title_y, col_w, 32, title_text, 10);

        // The cover's plus uses the same native add menu as the player.
        s.button(
            Rect::new(cover_x + cover_size - 38, cover_y + 8, 30, 30),
            "",
            "\u{e710}",
            Action::Go(View::Add),
            false,
        );
        if let Some(btn) = s.buttons.last_mut() {
            btn.accessible = "Add to favorites or playlist".into();
        }

        // Artist name (centered below title)
        let artist_y = title_y + 32;
        s.artist_links(Rect::new(24+offset_x, artist_y, col_w, 24), &ui.now, true);

        // Provider attribution
        if !ui.lyrics_provider.is_empty() {
            s.label_sized(
                24+offset_x,
                artist_y + 26,
                col_w,
                20,
                format!("Lyrics: {}", ui.lyrics_provider),
                11,
            );
        }
    }

    // 3. Lyrics display column
    let (right_x, right_w) = if is_fs {
        let rw = 740.min(w - 96);
        ((w - rw) / 2, rw)
    } else {
        (col_w + 32, (w - col_w - 36).max(220))
    };

    s.lyrics_area = Some(Rect::new(right_x, 0, right_w, h - 64));

    // Floating pill at bottom-right
    let pill_w = 78;
    let pill_h = 32;
    let pill_x = w - pill_w - 46;
    let pill_y = h - pill_h - 22;
    s.panel(pill_x, pill_y, pill_w, pill_h, 8);
    s.button(
        Rect::new(pill_x + 4, pill_y + 2, 32, 28),
        "",
        if ui.lyrics_cover_mode {
            "\u{e93c}"
        } else {
            "\u{ec71}"
        },
        Action::ToggleLyricsCoverMode,
        ui.lyrics_cover_mode,
    );
    if let Some(btn) = s.buttons.last_mut() {
        btn.accessible = "Toggle mode".into();
    }
    s.button(
        Rect::new(pill_x + 40, pill_y + 2, 32, 28),
        "",
        "\u{e8fd}",
        Action::ToggleQueueSidebar,
        ui.queue_sidebar_open,
    );
    if let Some(btn) = s.buttons.last_mut() {
        btn.accessible = "Queue".into();
    }

    let visualizer_label=if !ui.lyrics_visualizer{"Off"}else{match ui.visualizer_style{1=>"Bars",2=>"Trap Nation",_=>"Wave"}};
    s.button(Rect::new(32,h-54,156,32),ui.t(visualizer_label),"\u{e9e9}",Action::ToggleLyricsVisualizer,ui.lyrics_visualizer);
    if let Some(button)=s.buttons.last_mut(){button.accessible=format!("Visualizer: {visualizer_label}");}
    if ui.lyrics_cover_mode || mode_mix>0.01 {
        return;
    }

    let lyrics_btn_start = s.buttons.len();
    let lyrics_lbl_start = s.labels.len();

    let scale_pct = ui.lyrics_scale_percent(w);
    let center_y = h / 2 - 20;

    if ui.rich_lyrics.is_empty() && ui.lyrics.is_empty() {
        s.label(
            right_x,
            center_y - 20,
            right_w,
            "No lyrics returned for this track.",
            1,
        );
    } else {
        let fallback;
        let lines = if !ui.rich_lyrics.is_empty() {
            ui.rich_lyrics.as_slice()
        } else {
            fallback = ui
                .lyrics
                .iter()
                .map(|(time, text)| crate::lyrics::LyricLine {
                    start_ms: *time,
                    text: text.clone(),
                    ..Default::default()
                })
                .collect::<Vec<_>>();
            fallback.as_slice()
        };
        let timed = crate::lyrics::has_timing(lines);
        let row_h = (44 * scale_pct as i32 / 100).max(28);
        let font_px = 29.333 * scale_pct as f32 / 100.0;
        let spacing = (16 * scale_pct as i32 / 100).max(12);
        let gaps = crate::lyrics::instrumental_gaps(lines, ui.duration);
        let active_gap = gaps
            .iter()
            .copied()
            .find(|(start, end)| *start <= ui.position && ui.position < *end);
        s.lyrics_gap = active_gap;
        let mut blocks = Vec::new();
        for (index, line) in lines
            .iter()
            .enumerate()
            .filter(|(_, l)| !timed || !l.text.trim().is_empty())
        {
            blocks.push((
                line.start_ms,
                Some(index),
                crate::lyrics::wrapped_ranges(&line.text, right_w - 12, font_px),
            ));
        }
        if timed {
            for (start, _) in &gaps {
                blocks.push((*start, None, vec![0..0]));
            }
            blocks.sort_by_key(|block| block.0);
        }
        let active = if timed {
            blocks
                .iter()
                .rposition(|block| block.0 <= ui.position)
                .unwrap_or(0)
        } else {
            0
        };
        let anchor: i32 = blocks
            .iter()
            .take(active)
            .map(|b| b.2.len() as i32 * row_h + spacing)
            .sum();
        s.lyrics_anchor = (timed && !ui.lyrics_manual_scroll).then_some(anchor);
        let mut ly = if timed && !ui.lyrics_manual_scroll {
            center_y - anchor - ui.scroll
        } else {
            40 - ui.scroll
        };
        let mut total = 0;
        for (block_index, (start, source, ranges)) in blocks.iter().enumerate() {
            let height = ranges.len() as i32 * row_h;
            if let Some(source) = source {
                let line = &lines[*source];
                let active_line = timed && block_index == active && active_gap.is_none();
                for range in ranges {
                    let kind = if !timed {
                        14
                    } else if active_line {
                        13
                    } else {
                        12
                    };
                    s.label_sized(right_x, ly, right_w, row_h, &line.text[range.clone()], kind);
                    if !ui.rich_lyrics.is_empty() {
                        s.lyric_rows.push(LyricRow {
                            label: s.labels.len() - 1,
                            line: *source,
                            range: range.clone(),
                        });
                    }
                    if timed {
                        s.button(
                            Rect::new(right_x - 6, ly - 2, right_w, row_h),
                            "",
                            "",
                            Action::SeekMs(*start),
                            active_line,
                        );
                        if let Some(button) = s.buttons.last_mut() {
                            button.accessible = line.text[range.clone()].into();
                        }
                    }
                    ly += row_h;
                }
            } else {
                if active_gap.is_some_and(|gap| gap.0 == *start) {
                    s.label_sized(
                        right_x,
                        ly,
                        right_w,
                        row_h,
                        "\u{266a}   \u{266a}   \u{266a}",
                        15,
                    );
                }
                ly += height;
            }
            ly += spacing;
            total += height + spacing;
        }
        s.content_height = total + if timed && !ui.lyrics_manual_scroll { h } else { 120 };
    }

    s.content_buttons = lyrics_btn_start..s.buttons.len();
    s.content_labels = lyrics_lbl_start..s.labels.len();
}

fn build_track_info(s: &mut Scene, ui: &Ui, x: i32, top: i32, cw: i32) {
    let (track, meta, artist_meta) = if let Some((t, m, a)) = &ui.track_info {
        (t.clone(), m.clone(), a.clone())
    } else {
        (ui.now.clone(), ui.now.raw.clone(), serde_json::Value::Null)
    };
    let mut cy = top;
    s.panel(x, cy, cw, 168, 8);
    if !track.artwork.is_empty() {
        s.pictures
            .push((Rect::new(x + 18, cy + 18, 132, 132), track.artwork.clone()));
    } else {
        s.panel(x + 18, cy + 18, 132, 132, 1);
    }
    s.label(x + 168, cy + 18, cw - 186, "TRACK", 9);
    s.label(
        x + 168,
        cy + 40,
        cw - 186,
        if track.title.is_empty() {
            "Unknown Track"
        } else {
            &track.title
        },
        2,
    );
    let release = crate::catalog::string(
        &meta,
        &["release_date", "releaseDate", "physical_release_date"],
    );
    let dur_sec = track.duration / 1000;
    s.label(
        x + 168,
        cy + 88,
        cw - 186,
        format!(
            "{}   \u{2022}   {}   \u{2022}   {}:{:02}",
            track.subtitle,
            if release.is_empty() { "2024" } else { &release },
            dur_sec / 60,
            dur_sec % 60
        ),
        1,
    );
    let btn_w = 148.min((cw - 186) / 2).max(110);
    s.button(
        Rect::new(x + 168, cy + 118, btn_w, 36),
        "Play track",
        "\u{e768}",
        Action::OpenItem(track.clone()),
        true,
    );
    if !track.album_id.is_empty() {
        let alb = crate::catalog::Entry {
            id: track.album_id.clone(),
            title: if track.album.is_empty() {
                "Album".into()
            } else {
                track.album.clone()
            },
            subtitle: track.subtitle.clone(),
            artwork: track.artwork.clone(),
            kind: "album".into(),
            ..Default::default()
        };
        s.button(
            Rect::new(x + 168 + btn_w + 12, cy + 118, btn_w, 36),
            "View album",
            "\u{e93c}",
            Action::OpenItem(alb),
            false,
        );
    }
    cy += 186;
    let col_w = (cw - 16) / 2;
    let right_x = x + col_w + 16;
    s.panel(x, cy, col_w, 224, 8);
    s.label(x + 18, cy + 14, col_w - 36, "ABOUT THIS TRACK", 9);
    let rank = crate::catalog::string(&meta, &["rank"]);
    let track_pos = crate::catalog::string(&meta, &["track_position", "disk_number"]);
    let isrc = crate::catalog::string(&meta, &["isrc"]);
    let bpm = crate::catalog::string(&meta, &["bpm"]);
    for (i, (k, v)) in [
        (
            "Album",
            if track.album.is_empty() {
                "\u{2014}"
            } else {
                &track.album
            },
        ),
        (
            "Release date",
            if release.is_empty() { "2024" } else { &release },
        ),
        (
            "Track number",
            if track_pos.is_empty() {
                "1"
            } else {
                &track_pos
            },
        ),
        ("ISRC", if isrc.is_empty() { "DEEZNATS" } else { &isrc }),
        (
            "Popularity / Rank",
            if rank.is_empty() { "88%" } else { &rank },
        ),
        (
            "BPM",
            if bpm.is_empty() || bpm == "0" {
                "120"
            } else {
                &bpm
            },
        ),
    ]
    .iter()
    .enumerate()
    {
        let ry = cy + 44 + i as i32 * 28;
        s.label(x + 18, ry, 140, *k, 1);
        s.label(x + 164, ry, col_w - 180, *v, 0);
    }
    s.panel(right_x, cy, col_w, 224, 8);
    s.label(right_x + 18, cy + 14, col_w - 36, "CONTRIBUTORS", 9);
    let mut contribs = crate::catalog::entries(&meta["contributors"], "artist");
    if contribs.is_empty() {
        contribs.push(crate::catalog::Entry {
            id: track.artist_id.clone(),
            title: track.subtitle.clone(),
            subtitle: "Main artist \u{00b7} Performer".into(),
            artwork: track.artwork.clone(),
            kind: "artist".into(),
            ..Default::default()
        });
    }
    for (i, artist) in contribs.iter().take(3).enumerate() {
        let ry = cy + 44 + i as i32 * 56;
        if !artist.artwork.is_empty() {
            s.pictures.push((
                Rect::new(right_x + 18, ry + 2, 42, 42),
                artist.artwork.clone(),
            ));
        }
        let role = crate::catalog::string(&artist.raw, &["role"]);
        s.button(
            Rect::new(right_x + 68, ry + 2, col_w - 86, 42),
            format!(
                "{} \u{00b7} {}",
                artist.title,
                if role.is_empty() { "Performer" } else { &role }
            ),
            "",
            Action::OpenItem(artist.clone()),
            false,
        );
    }
    cy += 240;
    s.panel(x, cy, col_w, 148, 8);
    s.label(x + 18, cy + 14, col_w - 36, "GENRES AND DETAILS", 9);
    let explicit = meta["explicit_lyrics"]
        .as_bool()
        .or(track.raw["explicit"].as_bool())
        .unwrap_or(false);
    for (i, (k, v)) in [
        (
            "Format",
            if ui.quality.is_empty() {
                "FLAC 16-bit / 44.1 kHz"
            } else {
                &ui.quality
            },
        ),
        ("Content", if explicit { "Explicit" } else { "Clean" }),
        ("Source", "Deezer Catalog"),
    ]
    .iter()
    .enumerate()
    {
        let ry = cy + 46 + i as i32 * 30;
        s.label(x + 18, ry, 130, *k, 1);
        s.label(x + 154, ry, col_w - 170, *v, 0);
    }
    s.panel(right_x, cy, col_w, 148, 8);
    s.label(right_x + 18, cy + 14, col_w - 36, "ABOUT THE ARTIST", 9);
    let fans = crate::catalog::string(&artist_meta, &["nb_fan"]);
    let albums_n = crate::catalog::string(&artist_meta, &["nb_album"]);
    s.label(
        right_x + 18,
        cy + 46,
        col_w - 36,
        if track.subtitle.is_empty() {
            "Artist"
        } else {
            &track.subtitle
        },
        3,
    );
    s.label(
        right_x + 18,
        cy + 78,
        col_w - 36,
        format!(
            "{} fans   \u{00b7}   {} releases",
            if fans.is_empty() { "12,400" } else { &fans },
            if albums_n.is_empty() { "8" } else { &albums_n }
        ),
        1,
    );
    if !track.artist_id.is_empty() {
        let art_entry = crate::catalog::Entry {
            id: track.artist_id.clone(),
            title: track.subtitle.clone(),
            kind: "artist".into(),
            artwork: track.artwork.clone(),
            ..Default::default()
        };
        s.button(
            Rect::new(right_x + 18, cy + 104, 150, 34),
            "Go to artist",
            "\u{e77b}",
            Action::OpenItem(art_entry),
            false,
        );
    }
    s.content_height = cy + 180 + ui.scroll;
}
pub fn build(
    ui: &Ui,
    viewport: (i32, i32),
    connected: bool,
    busy: bool,
    title: &str,
    playing: bool,
    volume: u32,
) -> Scene {
    let (w, h) = viewport;
    let mut s = Scene::default();
    let is_lyrics = ui.view == View::Lyrics;
    let side = if is_lyrics {
        0
    } else if ui.sidebar_collapsed {
        56
    } else {
        214
    };
    let x = if is_lyrics {
        32
    } else if ui.sidebar_collapsed {
        side + 20
    } else {
        side + 48
    };
    let queue_w = if ui.queue_sidebar_open {
        280.min(w / 3)
    } else {
        0
    };
    let cw = w - x - queue_w - if is_lyrics { 32 } else { 40 };
    let bottom = h - player_height(ui, w) - 8;
    let step = 41;
    if !is_lyrics {
        if !ui.sidebar_collapsed {
            s.label(14, 22, 170, "MUSIC", 9);
        }
        for (i, (view, glyph)) in NAV.iter().enumerate() {
            let y = if i < 3 {
                42 + i as i32 * step
            } else {
                66 + 3 * step + (i as i32 - 3) * step
            };
            if i == 3 && !ui.sidebar_collapsed {
                s.label(14, 48 + 3 * step, 170, "LIBRARY", 9);
            }
            if !ui.sidebar_collapsed {
                s.button(
                    Rect::new(13, y, 178, step - 4),
                    if *view == View::Home {
                        "Home"
                    } else {
                        view.title()
                    },
                    glyph,
                    Action::Go(*view),
                    *view == ui.view,
                );
            } else {
                s.button(
                    Rect::new(10, y, 38, step - 4),
                    "",
                    glyph,
                    Action::Go(*view),
                    *view == ui.view,
                );
            }
        }
        let settings_y = 66 + 3 * step + (NAV.len() as i32 - 3) * step;
        if !ui.sidebar_collapsed {
            s.button(
                Rect::new(13, settings_y, 178, step - 4),
                "Settings",
                "\u{e713}",
                Action::Go(View::Settings),
                ui.view == View::Settings,
            );
        } else {
            s.button(
                Rect::new(10, settings_y, 38, step - 4),
                "",
                "\u{e713}",
                Action::Go(View::Settings),
                ui.view == View::Settings,
            );
        }
        if !ui.downloads.is_empty() {
            if !ui.sidebar_collapsed {
                s.panel(20, bottom - 115, 174, 65, 1);
                s.button(
                    Rect::new(24, bottom - 110, 178, 32),
                    "Downloads",
                    "\u{e896}",
                    Action::Go(View::Downloads),
                    ui.view == View::Downloads,
                );
                s.label(
                    54,
                    bottom - 76,
                    140,
                    format!(
                        "{} active downloads",
                        ui.downloads.iter().filter(|j| !j.finished).count()
                    ),
                    1,
                );
            } else {
                s.button(
                    Rect::new(10, bottom - 96, 38, 34),
                    "",
                    "\u{e896}",
                    Action::Go(View::Downloads),
                    ui.view == View::Downloads,
                );
            }
        }

        let main_panel_w = if queue_w > 0 {
            w - side - queue_w - 36
        } else {
            w - side - 24
        };
        s.panel(side + 12, 10, main_panel_w, bottom - 20, 0);
        if !ui.history.is_empty() {
            s.button(
                Rect::new(x, 32, 36, 36),
                "",
                "\u{e72b}",
                Action::Back,
                false,
            );
        }
        if ui.view == View::Explore {
            s.panel(x + 52, 30, cw - 126, 40, 8);
            s.search = Some(Rect::new(x + 66, 37, cw - 180, 26));
            s.button(
                Rect::new(
                    if queue_w > 0 {
                        w - queue_w - 148
                    } else {
                        w - 142
                    },
                    32,
                    34,
                    34,
                ),
                "",
                "\u{e721}",
                Action::Search,
                false,
            );
        }

        if let Some(button) = s.buttons.last_mut() {
            button.accessible = format!("Active provider: {}. Switch provider", "Deezer");
        }
    }
    if queue_w > 0 {
        let qx = w - queue_w - 12;
        build_queue_sidebar(&mut s, ui, qx, queue_w, h);
    }
    let entity = if is_lyrics {
        None
    } else {
        ui.page
            .as_ref()
            .and_then(|p| p.entity.as_ref())
            .filter(|e| {
                matches!(ui.view, View::Album | View::Artist | View::Show)
                    && matches!(
                        e.kind.as_str(),
                        "album" | "playlist" | "podcast" | "artist" | "mix" | "flow"
                    )
            })
    };
    let is_artist = ui.view == View::Artist || entity.as_ref().is_some_and(|e| e.kind == "artist");
    let cover_size = if is_artist {
        (148.0 / ui.dpi_scale).round() as i32
    } else {
        (160.0 / ui.dpi_scale).round() as i32
    };
    let title_x = x + if entity.is_some() { cover_size + 24 } else { 0 };
    let title_y = if is_artist { 82 } else { 102 };
    let sub_y = if is_artist { 138 } else { 158 };
    let header_actions_w = match ui.view {
        View::Flow => 360,
        View::Offline => 190,
        View::Playlists => 200,
        View::Favorites => 120,
        View::Settings => 196,
        _ => 0,
    };
    if !is_lyrics {
        s.label(
            title_x,
            title_y,
            (cw - (title_x - x) - header_actions_w).max(120),
            ui.page
                .as_ref()
                .filter(|_| {
                    matches!(
                        ui.view,
                        View::Home
                            | View::Explore
                            | View::Flow
                            | View::Podcasts
                            | View::Favorites
                            | View::Albums
                            | View::Playlists
            | View::PlaylistPicker
                            | View::Recent
                            | View::Album
                            | View::Artist
                            | View::Show
                            | View::Queue
                            | View::Offline
                    )
                })
                .map(|p| p.title.as_str())
                .unwrap_or(ui.view.title()),
            2,
        );
        let caption = match ui.view {
            View::Home => "Personalized recommendations from Deezer",
            View::Explore => "Discover music, genres and new releases",
            View::Favorites => "Your favorite tracks",
            View::Recent => "Listening history from this app",
            View::Offline => "Your music, available offline",
            View::Podcasts => "Your podcasts",
            View::Playlists => "Your playlists and saved podcasts",
            View::Settings => "Make DeezNats yours",
            View::CustomColors => "Customize colors for buttons, text and panels",
            _ => "",
        };
        let subtitle_str = if let Some(ent) = entity {
            entity_caption(ent, ui.page.as_deref(), &ent.subtitle, ui)
        } else {
            caption.to_owned()
        };
        s.label(
            title_x,
            sub_y,
            (cw - (title_x - x) - header_actions_w).max(120),
            subtitle_str,
            1,
        );
        match ui.view {
            View::Flow => {
                {
                    s.button(
                        Rect::new(x + cw - 344, 114, 190, 36),
                        "Customize Flow",
                        "\u{e713}",
                        Action::Go(View::FlowTune),
                        false,
                    );
                }
                s.button(
                    Rect::new(x + cw - 144, 114, 144, 36),
                    "Play this Flow",
                    "\u{e768}",
                    Action::PlayAll,
                    true,
                );
            }
            View::Offline => {}
            View::Playlists if true => {
                s.button(
                    Rect::new(x + cw - 190, 114, 190, 36),
                    "Create playlist",
                    "+",
                    Action::Go(View::PlaylistEdit),
                    false,
                );
            }

            View::Settings => {}
            _ => {}
        }
    }
    let body_top = if is_lyrics {
        70
    } else if is_artist {
        240
    } else {
        198 + entity
            .map(|_e| (144.0 / ui.dpi_scale).round() as i32)
            .unwrap_or(0)
    };
    if let Some(entity) = entity {
        let cover = if is_artist {
            (148.0 / ui.dpi_scale).round() as i32
        } else {
            (160.0 / ui.dpi_scale).round() as i32
        };
        let cover_y = if is_artist { 72 } else { 102 };
        if entity.artwork.is_empty() || ui.page.as_ref().is_some_and(|p| p.loading) {
            s.panel(
                x + 8,
                cover_y,
                cover,
                cover,
                if is_artist { 15 } else { 14 },
            );
        }
        s.pictures.push((
            Rect::new(x + 8, cover_y, cover, cover),
            entity.artwork.clone(),
        ));
        if !is_artist {
            s.button(
                Rect::new(x, body_top - 64, 56, 56),
                "",
                "\u{e768}",
                Action::PlayAll,
                true,
            );
            if let Some(button) = s.buttons.last_mut() {
                button.accessible = "Play".into();
            }
            let mut ax = x + 70;
            if entity.kind == "playlist" {
                s.button(
                    Rect::new(ax, body_top - 54, 100, 36),
                    "Edit",
                    "\u{e70f}",
                    Action::EditPlaylist(entity.clone()),
                    false,
                );
                ax += 112;
            }
            if !matches!(entity.kind.as_str(), "mix" | "flow") {
                let liked = ui.liked_ids.contains(&entity.library_key())
                    || ui.liked_ids.contains(&entity.id);
                s.button(
                    Rect::new(ax, body_top - 54, 160, 36),
                    if liked {
                        "In library"
                    } else {
                        "Add to library"
                    },
                    "\u{eb51}",
                    Action::Favorite,
                    liked,
                );
                ax += 172;
                s.button(
                    Rect::new(ax, body_top - 54, 100, 36),
                    "Share",
                    "\u{e72d}",
                    Action::Share,
                    false,
                );
                ax += 112;
            }
            {
                s.button(
                    Rect::new(ax, body_top - 54, 150, 36),
                    "Download",
                    "\u{e896}",
                    Action::Download,
                    false,
                );
            }
            // Open, library, share and download already have direct page actions.
            // The only extra collection-menu target is its artist: expose that
            // credit directly instead of keeping an otherwise redundant menu.
            if !entity.artist_entries().is_empty() {
                s.artist_links(
                    Rect::new(x + 180, cover_y + 100, (cw - 180).max(1), 24),
                    entity,
                    false,
                );
            }
        }
    }
    s.content_top = if is_lyrics {
        70
    } else if entity.is_some() {
        body_top
    } else {
        190
    };
    let top = body_top - ui.scroll;
    let picture_start = s.pictures.len();
    let content_start = (s.buttons.len(), s.labels.len(), s.panels.len());
    if let Some(page) = ui.page.as_ref().filter(|_| {
        !is_lyrics && {
            matches!(
                ui.view,
                View::Home
                    | View::Explore
                    | View::Flow
                    | View::Podcasts
                    | View::Favorites
                    | View::Albums
                    | View::Playlists
            | View::PlaylistPicker
                    | View::Recent
                    | View::Album
                    | View::Artist
                    | View::Show
                    | View::Queue
                    | View::Offline
            )
        }
    }) {
        if ui.view == View::Explore {
            let discover = ui.page.as_ref().is_some_and(|page| page.is_discover());
            let filters: &[&str] = if discover {
                &["All", "Playlists", "Genres", "Moods", "Podcasts"]
            } else {
                &[
                    "All",
                    "Tracks",
                    "Artists",
                    "Playlists",
                    "Albums",
                    "Podcasts",
                ]
            };
            let mut filter_x = x;
            for (index, label) in filters.iter().enumerate() {
                let label = ui.t(label);
                let width = text_button_width(label);
                s.button(
                    Rect::new(filter_x, top, width, 36),
                    label,
                    "",
                    Action::Filter(index),
                    ui.filter == index,
                );
                filter_x += width + 8;
            }
        }
        live_catalog(
            &mut s,
            ui,
            page,
            x,
            top + if ui.view == View::Explore { 54 } else { 0 },
            cw,
            h,
        );
    } else {
        match ui.view {
            View::Settings => settings(&mut s, ui, x, 190, cw, connected, busy),
            View::TrackInfo => build_track_info(&mut s, ui, x, top, cw),
            View::Account => {
                if connected {
                    s.label(
                        x,
                        top,
                        cw,
                        crate::catalog::string(&ui.account, &["name"]),
                        3,
                    );
                    s.label(
                        x,
                        top + 40,
                        cw,
                        crate::catalog::string(&ui.account, &["offer"]),
                        1,
                    );
                    for (index, (label, action)) in [
                        ("Settings", Action::Go(View::Settings)),
                        ("Downloads", Action::Go(View::Downloads)),
                        ("Stop", Action::Stop),
                        ("Reconnect", Action::ConnectSaved),
                        ("Sign out", Action::Logout),
                    ]
                    .into_iter()
                    .enumerate()
                    {
                        s.button(
                            Rect::new(x, top + 88 + index as i32 * 44, 240, 36),
                            label,
                            "",
                            action,
                            false,
                        );
                    }
                    s.content_height = top + 400 + ui.scroll;
                } else {
                    s.label(
                        x,
                        top,
                        cw,
                        if connected {
                            "Connected to Deezer"
                        } else {
                            "Welcome back"
                        },
                        3,
                    );
                    notice(
                        &mut s,
                        x,
                        top + 42,
                        cw,
                        "Use your existing DeezNats session, or paste an ARL below.",
                    );
                    s.button(
                        Rect::new(x, top + 92, 300, 40),
                        if busy {
                            "Connecting..."
                        } else {
                            "Connect using saved session"
                        },
                        "\u{e77b}",
                        Action::ConnectSaved,
                        true,
                    );
                    s.label(x, top + 156, cw, "ARL session", 0);
                    s.session = Some(Rect::new(x, top + 190, cw.min(560), 34));
                    s.button(
                        Rect::new(x, top + 244, 160, 38),
                        if busy { "Connecting..." } else { "Connect" },
                        "",
                        Action::Connect,
                        true,
                    );
                    s.button(
                        Rect::new(x + 172, top + 244, 206, 38),
                        "How do I get my ARL?",
                        "",
                        Action::Go(View::ArlHelp),
                        false,
                    );
                    s.label(x, top + 310, cw, "Play a Deezer track by ID", 3);
                    s.track = Some(Rect::new(x, top + 356, 260, 34));
                    s.button(
                        Rect::new(x + 276, top + 354, 142, 38),
                        "Play track",
                        "\u{e768}",
                        Action::PlayId,
                        false,
                    );
                    s.button(
                        Rect::new(x, top + 410, 132, 34),
                        "Settings",
                        "\u{e713}",
                        Action::Go(View::Settings),
                        false,
                    );
                    s.button(
                        Rect::new(x + 148, top + 410, 100, 34),
                        "Quit",
                        "",
                        Action::Close,
                        false,
                    );

                    s.button(
                        Rect::new(x + 402, top + 410, 80, 34),
                        "Stop",
                        "",
                        Action::Stop,
                        false,
                    );
                    if connected {
                        s.button(
                            Rect::new(x, top + 466, 200, 36),
                            "Account information",
                            "",
                            Action::Api("DZAccountJSON", vec![]),
                            false,
                        );
                        s.button(
                            Rect::new(x + 216, top + 466, 136, 36),
                            "Reconnect",
                            "",
                            Action::ConnectSaved,
                            false,
                        );
                        s.button(
                            Rect::new(x + 368, top + 466, 120, 36),
                            "Sign out",
                            "",
                            Action::Logout,
                            false,
                        );
                    }
                }
            }
            View::FlowTune => {
                let mut xx = x;
                let mut yy = 198;
                for (index, (id, name)) in ui.flow_options.iter().enumerate() {
                    let display = ui.t(name);
                    let width = (display.chars().count() as i32 * 9 + 40).max(100).min(cw);
                    if xx > x && xx + width > x + cw {
                        xx = x;
                        yy += 46;
                    }
                    s.button(
                        Rect::new(xx, yy, width, 36),
                        display,
                        "",
                        Action::FlowChoice(index),
                        id == { &ui.flow_id },
                    );
                    xx += width + 10;
                }
                if !ui.flow_options.is_empty() {
                    yy += 54;
                }
                s.panel(x, yy, cw, 40, 8);
                s.search = Some(Rect::new(x + 14, yy + 7, (cw - 56).max(60), 26));
                s.label_sized(x + cw - 34, yy + 7, 24, 26, "\u{e721}", 4);
                yy += 54;
                let footer = h - 174;
                let selected = ui.genres.iter().filter(|g| g.2).count();
                s.label(
                    x,
                    footer + 8,
                    (cw - 310).max(100),
                    ui.t("{0} genres selected \u{00b7} {1} options")
                        .replace("{0}", &selected.to_string())
                        .replace("{1}", &ui.genres.len().to_string()),
                    1,
                );
                s.genre_viewport = Some(Rect::new(x, yy, cw, (h - 188 - yy).max(24)));
                let cols = (cw / 200).max(2);
                let width = cw / cols;
                let filter = ui.flow_filter.to_lowercase();
                let visible: Vec<_> = ui
                    .genres
                    .iter()
                    .enumerate()
                    .filter(|(_, g)| g.1.to_lowercase().contains(&filter))
                    .collect();
                for (cell, (index, (_, name, enabled, _))) in visible.iter().enumerate() {
                    let row = cell as i32 / cols;
                    let column = cell as i32 % cols;
                    s.button(
                        Rect::new(
                            x + column * width,
                            yy + row * 42 - ui.scroll,
                            width - 10,
                            34,
                        ),
                        name,
                        if *enabled { "\u{e73d}" } else { "\u{e739}" },
                        Action::Genre(*index),
                        *enabled,
                    );
                    if let Some(button) = s.buttons.last_mut() {
                        button.enabled = ui.flow_customizable;
                    }
                }
                let extent = yy + ((visible.len() as i32 + cols - 1) / cols) * 42 + 64;
                s.button(
                    Rect::new(x + cw - 304, footer, 112, 38),
                    "Cancel",
                    "",
                    Action::Back,
                    false,
                );
                s.button(
                    Rect::new(x + cw - 180, footer, 180, 38),
                    "Save and use",
                    "",
                    Action::ApplyFlow,
                    true,
                );
                if let Some(button) = s.buttons.last_mut() {
                    button.enabled = ui.flow_loaded;
                }
                s.content_height = extent;
            }
            View::Lyrics => build_lyrics_view(
                &mut s,
                ui,
                w - if queue_w > 0 { queue_w + 24 } else { 0 },
                h,
                playing,
                volume,
            ),
            View::PlaylistEdit => {
                s.label(x, top, cw, "Playlist name", 0);
                s.name = Some(Rect::new(x, top + 36, cw.min(540), 34));
                s.label(x, top + 92, cw, "Description", 0);
                s.description = Some(Rect::new(x, top + 128, cw.min(540), 84));
            s.button(Rect::new(x + 190, top + 242, 150.min((cw - 190).max(1)), 38), "Clear description", "", Action::ClearPlaylistDescription, false);
                s.button(
                    Rect::new(x, top + 242, 180, 38),
                    "Choose cover",
                    "",
                    Action::Go(View::CoverEdit),
                    false,
                );
                s.button(
                    Rect::new(x, top + 304, 160, 38),
                    "Save playlist",
                    "",
                    Action::SavePlaylist,
                    true,
                );
                if !ui.playlist_draft.id.is_empty() {
                s.button(Rect::new(x, top + 354, 160, 38), "Delete playlist", "", Action::DeletePlaylist, false);
            }
            if ui.playlist_busy {
                s.label(x, top + 406, cw, "Saving playlist...", 1);
                for button in &mut s.buttons {
                    if matches!(button.action, Action::SavePlaylist | Action::DeletePlaylist | Action::ClearPlaylistDescription | Action::Go(View::CoverEdit)) { button.enabled = false; }
                }
            }
            s.content_height = top + 470 + ui.scroll;
            }
            View::CoverEdit => {
            s.label(x, top, cw, "Playlist cover", 3);
            let side = cw.min(240);
            s.panel(x, top + 54, side, side, 2);
            if !ui.playlist_draft.preview.is_empty() {
                s.pictures.push((Rect::new(x, top + 54, side, side), ui.playlist_draft.preview.clone()));
            }
            let mut yy = top + 310;
            for (label, action) in [
                ("Choose image", Action::ChoosePlaylistCover),
                ("Zoom in", Action::CropPlaylistCover(0.0, 0.0, 0.25)),
                ("Zoom out", Action::CropPlaylistCover(0.0, 0.0, -0.25)),
                ("Move left", Action::CropPlaylistCover(-0.1, 0.0, 0.0)),
                ("Move right", Action::CropPlaylistCover(0.1, 0.0, 0.0)),
                ("Move up", Action::CropPlaylistCover(0.0, -0.1, 0.0)),
                ("Move down", Action::CropPlaylistCover(0.0, 0.1, 0.0)),
                ("Export cropped cover", Action::ExportPlaylistCover),
                ("Back to playlist", Action::Back),
            ] {
                s.button(Rect::new(x, yy, cw.min(240), 36), label, "", action, false);
                if let Some(button) = s.buttons.last_mut() {
                    button.enabled = !ui.playlist_busy && (matches!(button.action, Action::ChoosePlaylistCover | Action::Back) || ui.playlist_draft.image.is_some());
                }
                yy += 44;
            }
            s.content_height = yy + 40 + ui.scroll;
        }
        View::Downloads => {
                if ui.downloads.is_empty() {
                    s.label(x, top, cw, "No downloads yet", 3);
                }
                for (index, job) in ui.downloads.iter().enumerate() {
                    let yy = top + index as i32 * 108;
                    s.label(x, yy, cw - 140, &job.title, 3);
                    s.label(
                        x,
                        yy + 38,
                        cw - 140,
                        format!("{} / {} \u{00b7} {}", job.done, job.total, job.message),
                        1,
                    );
                    if !job.finished {
                        s.button(
                            Rect::new(x + cw - 120, yy + 14, 112, 34),
                            "Cancel",
                            "",
                            Action::CancelDownload(job.id),
                            false,
                        );
                    }
                }
                s.content_height = top + ui.downloads.len() as i32 * 108 + ui.scroll + 40;
            }
            View::Add => {
                s.button(
                    Rect::new(x, top, 240, 38),
                    "Add to favorites",
                    "\u{eb51}",
                    Action::Favorite,
                    false,
                );
                s.button(
                    Rect::new(x, top + 52, 240, 38),
                    "Download for offline",
                    "\u{e896}",
                    Action::Download,
                    false,
                );
                s.button(
                    Rect::new(x, top + 104, 240, 38),
                    "Create playlist",
                    "+",
                    Action::Go(View::PlaylistEdit),
                    false,
                );
                if ui.now.kind == "track" && !ui.now.id.is_empty() {
                s.button(Rect::new(x, top + 156, 240, 38), "Add to playlist", "+", Action::ChoosePlaylist(ui.now.clone()), false);
            }
        }
            View::Storage => {
                s.label(x, top, cw, "Data and cache location", 3);
                let directory = crate::storage::data_dir().unwrap_or_default();
                s.label(x, top + 54, cw, directory.display().to_string(), 0);
                s.label(x, top + 100, cw, "Choose where DeezNats stores settings, downloads and cache.", 1);
                s.button(Rect::new(x, top + 146, 260.min(cw), 36), "Choose data folder...", "", Action::ChooseStorage, false);
                if let Ok(Some(pending)) = crate::storage::pending_dir() {
                    s.label(x, top + 202, cw, pending.display().to_string(), 0);
                    s.label(x, top + 238, cw, "Data will be copied to this folder the next time you open DeezNats. Original files are kept.", 1);
                    s.button(Rect::new(x, top + 280, 220.min(cw), 36), "Cancel location change", "", Action::CancelStorage, false);
                }
            }
            View::ArlHelp => {
                s.label(x, top, cw, "Sign in with ARL", 3);
                for (i, line) in [
                    "Open deezer.com and sign in to your account.",
                    "Open browser developer tools, then Application / Storage.",
                    "Find the arl cookie for deezer.com.",
                    "Paste its value into the masked ARL field in Account.",
                ]
                .iter()
                .enumerate()
                {
                    s.label(x, top + 60 + i as i32 * 42, cw, *line, 0);
                }
                s.button(
                    Rect::new(x, top + 260, 140, 38),
                    "Back",
                    "",
                    Action::Back,
                    false,
                );
            }
            View::CustomColors => {
                let mut cy = top;
                s.label(x, cy, cw, "Color customization", 3);
                cy += 38;
                let defaults = if ui.light {
                    [
                        ("Raised", "Button background", 0x00f0e7e3),
                        ("Text", "Text", 0x00261c18),
                        ("Icon", "Icon", 0x00261c18),
                        ("Accent", "Accent / selected state", 0x00d64554),
                        ("Panel", "Panel", 0x00ffffff),
                        ("Background", "Background", 0x00f8f4f2),
                        ("Border", "Border", 0x00d4c6bf),
                        ("Muted", "Secondary text", 0x0076665e),
                        ("GradientStart", "Gradient start", 0x00d64554),
                        ("GradientEnd", "Gradient end", 0x00a83542),
                    ]
                } else {
                    [
                        ("Raised", "Button background", 0x003f2051),
                        ("Text", "Text", 0x00f8f8f8),
                        ("Icon", "Icon", 0x00f8f8f8),
                        ("Accent", "Accent / selected state", 0x007c399f),
                        ("Panel", "Panel", 0x001d1d1d),
                        ("Background", "Background", 0x00171717),
                        ("Border", "Border", 0x00413140),
                        ("Muted", "Secondary text", 0x00bebebe),
                        ("GradientStart", "Gradient start", 0x007c399f),
                        ("GradientEnd", "Gradient end", 0x00651e3d),
                    ]
                };
                let btn_col_w = 140.min(cw / 3);
                for (i, (key, label, def)) in defaults.iter().enumerate() {
                    let yy = cy + i as i32 * 44;
                    s.label(x, yy + 4, cw - btn_col_w - 16, *label, 0);
                    let current = ui.color(key, *def);
                    s.button(
                        Rect::new(x + cw - btn_col_w, yy, btn_col_w, 32),
                        hex_color(current),
                        "",
                        Action::Color(key),
                        false,
                    );
                }
                cy += defaults.len() as i32 * 44 + 16;
                s.button(
                    Rect::new(x, cy, 230.min(cw), 36),
                    "Reset this theme's colors",
                    "",
                    Action::ResetTheme,
                    false,
                );
                s.content_height = cy + 60 + ui.scroll;
            }
            _ => {
                if connected {
                    render_skeletons(&mut s, ui, None, x, top, cw);
                } else {
                    s.label(x, top, cw, "Sign in to load your Deezer library.", 1);
                }
            }
        }
    }
    if ui.view != View::Settings && ui.view != View::Lyrics {
        s.content_buttons = content_start.0..s.buttons.len();
        s.content_labels = content_start.1..s.labels.len();
        s.content_panels = content_start.2..s.panels.len();
        s.content_pictures = picture_start..s.pictures.len();
    }
    let extent = s.buttons[s.content_buttons.clone()]
        .iter()
        .map(|b| b.rect.y + b.rect.h)
        .chain(
            s.labels[s.content_labels.clone()]
                .iter()
                .map(|l| l.rect.y + l.rect.h),
        )
        .chain(s.sliders.iter().map(|(r, _, _)| r.y + r.h))
        .max()
        .unwrap_or(0);
    if ui.view != View::FlowTune {
        s.content_height = s.content_height.max(extent + ui.scroll + 20);
    }
    if ui.view != View::Lyrics || s.lyrics_anchor.is_none() {
        let bottom=if ui.view==View::Lyrics {h-64}else{h-player_height(ui,w)-16};
        let available = bottom - s.content_top;
        let maximum = (s.content_height - bottom).max(0);
        if maximum > 0 && available > 0 {
            let thumb = (available * available / (s.content_height - s.content_top).max(1))
                .clamp(32, available);
            let scrollbar_x = w - queue_w - if ui.view==View::Lyrics {12}else if queue_w > 0 {39}else{27};
            s.panel(scrollbar_x, s.content_top, 9, available, 5);
            s.panel(
                scrollbar_x,
                s.content_top + ((ui.scroll as i64 * (available - thumb) as i64) / maximum as i64) as i32,
                9,
                thumb,
                6,
            );
        }
    }
    if ui.view != View::Lyrics {
        build_player(&mut s, ui, w, h, title, playing, volume);
    }

    if ui.deezer_login_open {
        build_deezer_login(&mut s, ui, w, h);
    }
    translate_scene(&mut s, ui);
    s
}
fn login_modal(s: &mut Scene, rect: Rect, title: &str, close: Action) {
    s.auth_modal = Some((rect, s.labels.len(), s.buttons.len()));
    s.search = None;
    s.session = None;

    s.track = None;
    s.name = None;
    s.description = None;
    s.seek = None;
    s.volume = None;
    s.sliders.clear();
    s.label(rect.x + 24, rect.y + 18, rect.w - 112, title, 3);
    s.button(
        Rect::new(rect.x + rect.w - 88, rect.y + 18, 64, 34),
        "Close",
        "",
        close,
        false,
    );
}

fn build_deezer_login(s: &mut Scene, ui: &Ui, w: i32, h: i32) {
    let width = (w - 40).min(320).max(1);
    let height = 340.min(h - 60).max(1);
    let rect = Rect::new(
        (w - width) / 2,
        ((h - height) / 2 - if ui.deezer_login_advanced { 60 } else { 0 }).max(20),
        width,
        height,
    );
    login_modal(s, rect, "Connect Deezer", Action::CloseDeezerLogin);
    let button_width = (width - 48).min(300);
    let x = rect.x + (width - button_width) / 2;
    s.button(
        Rect::new(x, rect.y + 82, button_width, 48),
        if ui.deezer_login_busy {
            "Signing in..."
        } else {
            "WebView2 login"
        },
        "",
        Action::ConnectDeezerWebView,
        false,
    );
    if let Some(button) = s.buttons.last_mut() {
        button.enabled = !ui.deezer_login_busy;
    }
    s.button(
        Rect::new(x, rect.y + 166, button_width, 36),
        "Advanced",
        "",
        Action::DeezerLoginAdvanced(true),
        false,
    );
    if !ui.deezer_login_advanced && !ui.deezer_login_status.is_empty() {
        s.label(
            rect.x + 24,
            rect.y + 138,
            width - 48,
            &ui.deezer_login_status,
            1,
        );
    }
    if ui.deezer_login_advanced {
        s.login_underlay = s.auth_modal;
        let height = 330.min(h - 60).max(1);
        let width = (w - 64).min(300).max(1);
        let rect = Rect::new(
            (w - width) / 2,
            (rect.y + 60).min(h - height - 30).max(20),
            width,
            height,
        );
        login_modal(s, rect, "Advanced", Action::DeezerLoginAdvanced(false));
        for (index, line) in wrap_text(
            "Paste your Deezer ARL cookie. It is protected locally.",
            width - 48,
            14,
        )
        .into_iter()
        .take(3)
        .enumerate()
        {
            s.label(
                rect.x + 24,
                rect.y + 76 + index as i32 * 22,
                width - 48,
                line,
                1,
            );
        }
        s.session = Some(Rect::new(rect.x + 24, rect.y + 144, width - 48, 34));
        let button_width = (width - 48).min(240);
        s.button(
            Rect::new(
                rect.x + (width - button_width) / 2,
                rect.y + 204,
                button_width,
                36,
            ),
            "Connect",
            "",
            Action::Connect,
            false,
        );
        if let Some(button) = s.buttons.last_mut() {
            button.enabled = !ui.deezer_login_busy;
        }
        for (index, line) in wrap_text(&ui.deezer_login_status, width - 48, 14)
            .into_iter()
            .take(3)
            .enumerate()
        {
            s.label(
                rect.x + 24,
                rect.y + 250 + index as i32 * 20,
                width - 48,
                line,
                1,
            );
        }
    }
}

fn translate_scene(scene: &mut Scene, ui: &Ui) {
    if ui.translations.is_empty() {
        return;
    }
    let mut media = std::collections::HashSet::new();
    media.insert(ui.now.title.as_str());
    media.insert(ui.now.subtitle.as_str());
    media.insert(ui.now.album.as_str());
    for entry in ui.queue.iter().chain(ui.offline.iter()) {
        media.insert(entry.title.as_str());
        media.insert(entry.subtitle.as_str());
        media.insert(entry.album.as_str());
    }
    if let Some(page) = &ui.page {
        for entry in page
            .rows
            .iter()
            .chain(page.sections.iter().flat_map(|s| s.entries.iter()))
            .chain(page.entity.iter())
        {
            media.insert(entry.title.as_str());
            media.insert(entry.subtitle.as_str());
            media.insert(entry.album.as_str());
        }
    }
    for label in &mut scene.labels {
        if !media.contains(label.text.as_str()) {
            label.text = ui.translated_text(&label.text);
        }
    }
    for button in &mut scene.buttons {
        if !matches!(
            button.action,
            Action::OpenItem(_)
                | Action::FlowChoice(_)
                | Action::Genre(_)
                | Action::Preset(_)
        ) {
            button.text = ui.translated_text(&button.text);
        }
    }
}

pub(crate) fn player_height(ui: &Ui, width: i32) -> i32 {
    let breakpoint = if ui.player_layout == 1 {1100} else {1180};
    if width < breakpoint { 112 } else if ui.player_layout == 1 { 64 } else { 88 }
}
pub(crate) fn player_title_rects(region: Rect, natural: i32, explicit: bool, centered: bool) -> (Rect, Option<Rect>, Rect) {
    let badge_space = if explicit { 22 } else { 0 };
    let title_w = natural.min((region.w - badge_space - 36).max(0)).max(0);
    let total = title_w + badge_space + 36;
    let x = region.x + if centered { (region.w - total).max(0) / 2 } else { 0 };
    let title = Rect::new(x, region.y, title_w, region.h);
    let badge = explicit.then(|| Rect::new(x + title_w + 6, region.y + 5, 14, 16));
    let favorite = Rect::new(x + title_w + badge_space + 6, region.y - 1, 28, 28);
    (title, badge, favorite)
}
fn player_metadata(s: &mut Scene, ui: &Ui, region: Rect, title: &str, centered: bool) {
    if title.is_empty() { return; }
    let explicit = ui.now.raw["explicit"].as_bool()
        .or(ui.now.raw["explicit_lyrics"].as_bool())
        .or_else(|| ui.now.raw["EXPLICIT_LYRICS"].as_str().map(|v| v == "1"))
        .unwrap_or(false);
    let (title_rect, badge_rect, favorite_rect) =
        player_title_rects(region, title.chars().count() as i32 * 9 + 4, explicit, centered);
    let label = s.labels.len();
    s.label_sized(title_rect.x, title_rect.y, title_rect.w, title_rect.h, title, if centered {16} else {18});
    let badge = badge_rect.map(|r| {
        let index = s.labels.len();
        s.label_sized(r.x, r.y, r.w, r.h, "E", 17);
        index
    });
    let liked = ui.liked_ids.contains(&ui.now.library_key());
    let favorite = s.buttons.len();
    s.button(favorite_rect, "", if liked {"\u{eb52}"} else {"\u{eb51}"}, Action::FavoriteNow, liked);
    s.buttons.last_mut().unwrap().accessible = if liked {"Unlike this song"} else {"Like this song"}.into();
    s.player_title = Some(PlayerTitle { region, label, badge, favorite, centered });
    let artist_labels = s.labels.len();
    s.artist_links(Rect::new(region.x, region.y + 25, region.w, 22), &ui.now, centered);
    for label in &mut s.labels[artist_labels..] { label.kind = if centered {20} else {19}; }
}
fn player_controls(s: &mut Scene, ui: &Ui, x: i32, y: i32, step: i32, playing: bool) {
    for (index, (glyph, action, selected, name)) in [
        ("\u{e8b1}", Action::Shuffle, ui.shuffle, "Shuffle"),
        ("\u{e892}", Action::Previous, false, "Previous"),
        (if playing {"\u{e769}"} else {"\u{e768}"}, Action::Toggle, false, if playing {"Pause"} else {"Play"}),
        ("\u{e893}", Action::Next, false, "Next"),
        (if ui.repeat == 2 {"\u{e8ed}"} else {"\u{e8ee}"}, Action::Repeat, ui.repeat > 0, "Repeat"),
    ].into_iter().enumerate() {
        s.button(Rect::new(x + index as i32 * step, y, 32, 32), "", glyph, action, selected);
        s.buttons.last_mut().unwrap().accessible = name.into();
    }
}
fn player_timeline(s: &mut Scene, x: i32, y: i32, width: i32) {
    let clock = 40;
    s.elapsed = Some(Rect::new(x, y + 2, clock, 20));
    s.seek = Some(Rect::new(x + clock, y, (width - clock * 2).max(20), 24));
    s.remaining = Some(Rect::new(x + width - clock + 6, y + 2, clock - 6, 20));
}
fn player_utility(s: &mut Scene, rect: Rect, glyph: &'static str, action: Action, selected: bool, name: &str) {
    s.button(rect, "", glyph, action, selected);
    s.buttons.last_mut().unwrap().accessible = name.into();
}
fn player_extras(s: &mut Scene, ui: &Ui, right: i32, y: i32, volume: u32, narrow: bool) {
    let volume_w = if narrow {94} else {140};
    let fullscreen = right - 42;
    let volume_x = fullscreen - volume_w - if narrow {70} else {24};
    let mute_x = volume_x - 38;
    if !narrow {
        let layout_x = mute_x - 150;
        player_utility(s, Rect::new(layout_x, y, 34, 32), "\u{e8a7}", Action::PlayerLayout(1 - ui.player_layout.min(1)), false, "Switch player layout");
        player_utility(s, Rect::new(layout_x + 50, y, 32, 32), "\u{ec71}", Action::Go(View::Lyrics), false, "Lyrics");
        player_utility(s, Rect::new(layout_x + 100, y, 32, 32), "\u{e8fd}", Action::ToggleQueueSidebar, ui.queue_sidebar_open, "Queue");
    } else {
        player_utility(s, Rect::new(fullscreen - 50, y, 34, 32), "\u{e8a7}", Action::PlayerLayout(1 - ui.player_layout.min(1)), false, "Switch player layout");
    }
    player_utility(s, Rect::new(mute_x, y, 32, 32), if volume == 0 {"\u{e74f}"} else {"\u{e767}"}, Action::MuteVolume, volume == 0, if volume == 0 {"Unmute"} else {"Mute"});
    s.volume = Some(Rect::new(volume_x, y + 4, volume_w, 24));
    player_utility(s, Rect::new(fullscreen, y, 32, 32), "\u{e740}", Action::PlayerFullscreen, false, "Fullscreen lyrics");
}
fn build_player(s: &mut Scene, ui: &Ui, w: i32, h: i32, title: &str, playing: bool, volume: u32) {
    let height = player_height(ui, w);
    let top = h - height;
    let right = w - 8;
    s.panel(0, top, w, height, 21);
    if height == 112 {
        build_narrow_player(s, ui, 0, top, w, title, playing, volume);
        return;
    }
    let compact = ui.player_layout == 1;
    let cover = if compact {42} else {52};
    let cover_x = if compact {20} else {16};
    if !ui.now.artwork.is_empty() {
        s.pictures.push((Rect::new(cover_x, top + (height-cover)/2, cover, cover), ui.now.artwork.clone()));
    }
    player_extras(s, ui, right, top + (height-32)/2, volume, false);
    let extras_left = right - 42 - 140 - 24 - 38 - 150;
    if compact {
        let controls_x = (w * 213 / 1000).clamp(220, 320);
        let step = (w / 30).clamp(36, 49);
        player_metadata(s, ui, Rect::new(78, top + 7, controls_x - 96, 24), title, false);
        player_controls(s, ui, controls_x, top + 16, step, playing);
        let timeline_x = controls_x + step * 4 + 64;
        player_timeline(s, timeline_x, top + 20, extras_left - timeline_x - 20);
    } else {
        player_controls(s, ui, 84, top + 28, 49, playing);
        let center = w / 2;
        let timeline_w = 440.min((center - 340).max(60) * 2).min((extras_left - center - 20).max(60) * 2);
        let metadata_w = ((center - 328).min(extras_left - center - 16) * 2).max(80);
        player_metadata(s, ui, Rect::new(center - metadata_w/2, top + 6, metadata_w, 24), title, true);
        player_timeline(s, center - timeline_w/2, top + 53, timeline_w);
    }
}
fn build_narrow_player(s: &mut Scene, ui: &Ui, _left: i32, top: i32, width: i32, title: &str, playing: bool, volume: u32) {
    let controls_x = width - 240;
    if !ui.now.artwork.is_empty() {
        s.pictures.push((Rect::new(22, top + 12, 52, 52), ui.now.artwork.clone()));
    }
    player_metadata(s, ui, Rect::new(88, top + 9, (controls_x-104).max(60), 24), title, false);
    player_controls(s, ui, controls_x, top + 22, 49, playing);
    let extras_left = width - 8 - 42 - 94 - 70 - 38;
    player_timeline(s, 40, top + 78, (extras_left - 74).max(100));
    player_extras(s, ui, width - 8, top + 74, volume, true);
}
fn render_shelf_skeleton(
    s: &mut Scene,
    ui: &Ui,
    x: i32,
    y: &mut i32,
    cw: i32,
    title_w: i32,
    circular: bool,
) {
    render_card_skeletons(s, ui, x, y, cw, title_w, circular, 1);
}

fn render_card_skeletons(
    s: &mut Scene,
    ui: &Ui,
    x: i32,
    y: &mut i32,
    cw: i32,
    title_w: i32,
    circular: bool,
    rows: usize,
) {
    let x_shelf = x - 14;
    let cw_shelf = cw + 14;
    let cols = (((cw_shelf - 12) as f64 * ui.dpi_scale) as i32 / 200).max(1);
    let cell = (cw_shelf - 12) / cols;
    let cover = ((160.0 / ui.dpi_scale).round() as i32)
        .min(cell - 16)
        .max(32);
    let label_w = (cell - 28).max(20);
    let row_height = cover + 88;

    s.panel(x_shelf + 8, *y + 2, title_w.min(cw_shelf - 30), 22, 14);
    *y += 40;

    for _ in 0..rows {
        for col in 0..cols {
            let slot = x_shelf + 6 + col * cell;
            let xx = slot + (cell - 8 - cover) / 2;
            let label_x = xx + cover / 2 - label_w / 2;
            s.panel(xx, *y, cover, cover, if circular { 15 } else { 14 });
            let title_width = { label_w };
            let subtitle_width = { label_w * 3 / 4 };
            s.panel(
                label_x + (label_w - title_width) / 2,
                *y + cover + 12,
                title_width,
                16,
                14,
            );
            s.panel(
                label_x + (label_w - subtitle_width) / 2,
                *y + cover + 35,
                subtitle_width,
                12,
                14,
            );
        }
        *y += row_height;
    }
    *y += 20;
}

fn render_grid_skeleton(
    s: &mut Scene,
    ui: &Ui,
    x: i32,
    y: &mut i32,
    cw: i32,
    title_w: i32,
    rows: usize,
) {
    if ui.view != View::Explore {
        render_card_skeletons(s, ui, x, y, cw, title_w, false, rows);
        return;
    }
    // Channel/mood tiles use the same inset, 90px height and 102px stride
    // as live_catalog, rather than square artwork with two metadata lines.
    let x_shelf = x - 14;
    let cw_shelf = cw + 14;
    let cols = (cw_shelf / 220).max(2);
    let cell = (cw_shelf - 12) / cols;
    s.panel(x_shelf + 8, *y + 2, title_w.min(cw_shelf - 30), 22, 14);
    *y += 40;

    for _ in 0..rows {
        for col in 0..cols {
            s.panel(x_shelf + 6 + col * cell, *y, cell - 12, 90, 14);
        }
        *y += 102;
    }
    *y += 20;
}

fn render_table_skeleton(
    s: &mut Scene,
    ui: &Ui,
    page: Option<&crate::catalog::Page>,
    x: i32,
    y: &mut i32,
    cw: i32,
    row_count: usize,
) {
    let entity = page.and_then(|p| p.entity.as_ref());
    let is_album = (entity.is_none() && ui.view == View::Album)
        || page
            .and_then(|p| p.entity.as_ref())
            .is_some_and(|e| e.kind == "album");
    let is_podcast = ui.view == View::Show
        || page
            .and_then(|p| p.entity.as_ref())
            .is_some_and(|e| e.kind == "podcast");
    let show_check =
        entity.is_some() || matches!(ui.view, View::Favorites | View::Album | View::Show);
    let check_w = if show_check { 38 } else { 0 };
    let dur_w = 54;
    let show_date = !is_album
        && !page
            .and_then(|p| p.entity.as_ref())
            .is_some_and(|e| matches!(e.kind.as_str(), "mix" | "flow"))
        && ((is_podcast && cw >= 480)
            || (matches!(ui.view, View::Favorites | View::Album) && cw >= 940));
    let date_w = if show_date { 108 } else { 0 };
    let show_artist = !is_podcast && cw >= 560;
    let show_album = !is_album && !is_podcast && cw >= 760;
    let index_w = 54;
    let fixed_w = index_w + dur_w + check_w + date_w;
    let flex_w = (cw - fixed_w - 12).max(120);

    let (title_width, artist_w, album_w) = if !show_artist {
        (flex_w, 0, 0)
    } else if show_album {
        let t_w = (flex_w * 44 / 100).max(60);
        let ar_w = (flex_w * 28 / 100).max(40);
        let al_w = flex_w.saturating_sub(t_w + ar_w).max(40);
        (t_w, ar_w, al_w)
    } else {
        let t_w = (flex_w * 60 / 100).max(70);
        let ar_w = flex_w.saturating_sub(t_w).max(40);
        (t_w, ar_w, 0)
    };

    let track_x = x + 54;
    let artist_x = track_x + title_width;
    let album_x = artist_x + artist_w;
    let date_x = if show_date { album_x + album_w } else { 0 };
    let dur_x = x + cw - check_w - dur_w;
    let check_x = x + cw - check_w + 4;

    // Table header buttons/labels
    s.panel(track_x, *y + 4, (title_width - 16).clamp(40, 80), 14, 14);
    if show_artist {
        s.panel(artist_x, *y + 4, (artist_w - 16).clamp(30, 60), 14, 14);
    }
    if show_album {
        s.panel(album_x, *y + 4, (album_w - 16).clamp(30, 60), 14, 14);
    }
    if show_date {
        s.panel(date_x, *y + 4, 60, 14, 14);
    }
    s.panel(dur_x, *y + 4, 36, 14, 14);
    s.panel(x, *y + 29, cw, 1, 9);
    *y += 34;

    for i in 0..row_count {
        let row_y = *y + i as i32 * 56;
        if is_album {
            s.panel(x + 16, row_y + 18, 18, 14, 14);
        } else {
            s.panel(x + 4, row_y + 6, 44, 44, 14);
        }
        s.panel(
            track_x,
            row_y + 18,
            (title_width * if is_podcast { 80 } else { 65 } / 100)
                .clamp(60, if is_podcast { 360 } else { 220 }),
            16,
            14,
        );
        if show_artist {
            s.panel(
                artist_x,
                row_y + 18,
                (artist_w * 60 / 100).clamp(40, 150),
                14,
                14,
            );
        }
        if show_album {
            s.panel(
                album_x,
                row_y + 18,
                (album_w * 60 / 100).clamp(40, 140),
                14,
                14,
            );
        }
        if show_date {
            s.panel(date_x, row_y + 18, 64, 14, 14);
        }
        s.panel(dur_x, row_y + 18, 36, 14, 14);
        if show_check {
            s.panel(check_x, row_y + 14, 28, 28, 14);
        }
    }
    *y += row_count as i32 * 56;
}

fn render_skeletons(
    s: &mut Scene,
    ui: &Ui,
    page: Option<&crate::catalog::Page>,
    x: i32,
    top: i32,
    cw: i32,
) {
    let entity = page.and_then(|p| p.entity.as_ref());
    let entity_kind = entity.map(|e| e.kind.as_str());
    let is_artist = entity_kind == Some("artist") || ui.view == View::Artist;
    let is_album = entity_kind == Some("album") || (entity.is_none() && ui.view == View::Album);
    let is_show = entity_kind == Some("podcast") || ui.view == View::Show;
    let is_grid = matches!(ui.view, View::Albums | View::Playlists | View::PlaylistPicker);
    let is_track_list = matches!(
        ui.view,
        View::Favorites | View::Recent | View::Offline | View::Flow
    ) || matches!(entity_kind, Some("playlist" | "mix" | "flow"))
        || is_album
        || is_show;

    let mut y = top;

    if is_artist {
        if entity.is_none() {
            s.panel(x + 8, 72, 148, 148, 15);
            s.panel(x + 180, 82, 260.min(cw - 200), 32, 14);
            s.panel(x + 180, 138, 140, 18, 14);
        }
        render_shelf_skeleton(s, ui, x, &mut y, cw, 140, false);
        render_shelf_skeleton(s, ui, x, &mut y, cw, 120, false);
        render_shelf_skeleton(s, ui, x, &mut y, cw, 140, true);
        s.content_height = y + ui.scroll;
    } else if is_track_list {
        if entity.is_none() {
            if matches!(ui.view, View::Album) {
                s.panel(x + 8, 102, 160, 160, 14);
                s.panel(x + 192, 102, 280.min(cw - 210), 32, 14);
                s.panel(x + 192, 158, 160, 18, 14);
                {
                    s.panel(x + 364, 158, 72.min((cw - 364).max(0)), 18, 14);
                }
                s.panel(x, 278, 56, 56, 14);
                s.panel(x + 70, 288, 140, 36, 14);
                s.panel(x + 222, 288, 100, 36, 14);
            } else if matches!(ui.view, View::Favorites | View::Offline) {
                s.panel(x, y, 130, 36, 14);
                if ui.view != View::Offline {
                    s.panel(x + 144, y, 130, 36, 14);
                }
                y += 56;
            }
        }
        render_table_skeleton(s, ui, page, x, &mut y, cw, 9);
        s.content_height = y + 40 + ui.scroll;
    } else if is_grid {
        render_grid_skeleton(s, ui, x, &mut y, cw, 160, 3);
        s.content_height = y + 40 + ui.scroll;
    } else if ui.view == View::Podcasts {
        render_shelf_skeleton(s, ui, x, &mut y, cw, 160, false);
        render_shelf_skeleton(s, ui, x, &mut y, cw, 140, false);
        render_shelf_skeleton(s, ui, x, &mut y, cw, 140, false);
        s.content_height = y + ui.scroll;
    } else {
        let titles = { [180, 200, 180] };
        for title in titles {
            render_shelf_skeleton(s, ui, x, &mut y, cw, title, false);
        }
        if ui.view == View::Explore {
            render_grid_skeleton(s, ui, x, &mut y, cw, 68, 2);
        }
        s.content_height = y + ui.scroll;
    }
}
fn live_catalog(
    s: &mut Scene,
    ui: &Ui,
    page: &crate::catalog::Page,
    x: i32,
    top: i32,
    cw: i32,
    h: i32,
) {
    let mut y = top;
    if let Some(show) = page.entity.as_ref().filter(|entry| entry.kind == "podcast") {
        let description = plain_description(show.raw["description"].as_str().unwrap_or_default());
        let lines = wrap_text(&description, cw, 14);
        let expandable = lines.len() > 3;
        let expanded = ui.expanded_podcast_description.as_deref() == Some(show.id.as_str());
        let visible = if expanded {
            lines.len()
        } else {
            lines.len().min(3)
        };
        for (index, line) in lines.iter().take(visible).enumerate() {
            let line = if expandable && !expanded && index == 2 {
                format!("{line}\u{2026}")
            } else {
                line.clone()
            };
            // Match the table/card overscan: buffered scrolling reuses this
            // scene for movements below 96px in either direction.
            if y + 22 >= s.content_top - 160 && y < h + 36 {
                s.label_sized(x, y, cw, 22, line, 1);
            }
            y += 22;
        }
        if expandable {
            let text = ui.t(if expanded { "Read less" } else { "Read more" });
            s.button(
                Rect::new(x, y + 4, cw.min(180), 24),
                text,
                "",
                Action::TogglePodcastDescription(show.id.clone()),
                false,
            );
            if let Some(button) = s.buttons.last_mut() {
                button.text_link = true;
                button.accessible = text.into();
            }
            y += 32;
        }
        if !description.is_empty() {
            y += 20;
        }
    }
    if page.entity.is_none()
        && !page.rows.is_empty()
        && matches!(ui.view, View::Favorites | View::Offline)
    {
        s.button(
            Rect::new(x, y, 130, 36),
            "Play",
            "\u{e768}",
            Action::PlayAll,
            true,
        );
        if ui.view != View::Offline {
            s.button(
                Rect::new(x + 144, y, 130, 36),
                "Download",
                "\u{e896}",
                Action::Download,
                false,
            );
        }
        y += 56;
    }
    if page.loading && page.rows.is_empty() && page.sections.iter().all(|s| s.entries.is_empty()) {
        render_skeletons(s, ui, Some(page), x, y, cw);
        return;
    }
    if !page.error.is_empty() {
        for line in wrap_text(&page.error, cw, 14) {
            s.label(x, y, cw, line, 1);
            y += 24;
        }
        y += 18;
    }
    if !page.rows.is_empty() {
        let is_album = page.entity.as_ref().is_some_and(|e| e.kind == "album");
        let is_podcast = ui.view == View::Show
            || page.entity.as_ref().is_some_and(|e| e.kind == "podcast")
            || (!page.rows.is_empty() && page.rows.iter().all(|e| e.kind == "episode"));
        let show_check = page.entity.is_some() || ui.view == View::Favorites;
        let check_w = if show_check { 38 } else { 0 };
        let dur_w = 54;
        let show_date = !is_album
            && !page
                .entity
                .as_ref()
                .is_some_and(|e| matches!(e.kind.as_str(), "mix" | "flow"))
            && ((is_podcast && cw >= 480)
                || (matches!(ui.view, View::Favorites | View::Album) && cw >= 940));
        let date_w = if show_date { 108 } else { 0 };
        let show_artist = !is_podcast && cw >= 560;
        let show_album = !is_album && !is_podcast && cw >= 760;

        let fixed_w = 54 + dur_w + check_w + date_w;
        let flex_w = (cw - fixed_w - 12).max(120);

        let (title_width, artist_w, album_w) = if !show_artist {
            (flex_w, 0, 0)
        } else if show_album {
            let t_w = (flex_w * 44 / 100).max(60);
            let ar_w = (flex_w * 28 / 100).max(40);
            let al_w = flex_w.saturating_sub(t_w + ar_w).max(40);
            (t_w, ar_w, al_w)
        } else {
            let t_w = (flex_w * 60 / 100).max(70);
            let ar_w = flex_w.saturating_sub(t_w).max(40);
            (t_w, ar_w, 0)
        };

        let track_x = x + 54;
        let artist_x = track_x + title_width;
        let album_x = artist_x + artist_w;
        let date_x = if show_date { album_x + album_w } else { 0 };
        let dur_x = x + cw - check_w - dur_w;
        let check_x = x + cw - check_w + 4;

        let sort_indicator = |col: usize| {
            if ui.sort_column == col {
                if ui.sort_descending {
                    " \u{2193}"
                } else {
                    " \u{2191}"
                }
            } else {
                ""
            }
        };
        s.button(
            Rect::new(track_x, y, (title_width - 8).max(40), 24),
            format!(
                "{}{}",
                if is_podcast {
                    ui.t("Episode")
                } else if page
                    .rows
                    .iter()
                    .any(|e| e.kind == "episode" || !e.playable())
                {
                    ui.t("Title")
                } else {
                    ui.t("Track")
                },
                sort_indicator(1)
            ),
            "",
            Action::Sort(1),
            ui.sort_column == 1,
        );
        if show_artist {
            s.button(
                Rect::new(artist_x, y, (artist_w - 8).max(40), 24),
                format!("{}{}", ui.t("Artist"), sort_indicator(2)),
                "",
                Action::Sort(2),
                ui.sort_column == 2,
            );
        }
        if show_album {
            s.button(
                Rect::new(album_x, y, (album_w - 8).max(40), 24),
                format!("{}{}", ui.t("Album"), sort_indicator(3)),
                "",
                Action::Sort(3),
                ui.sort_column == 3,
            );
        }
        if show_date {
            s.button(
                Rect::new(date_x, y, (date_w - 8).max(40), 24),
                format!(
                    "{}{}",
                    ui.t(if is_podcast {
                        "Release date"
                    } else {
                        "Date added"
                    }),
                    sort_indicator(4)
                ),
                "",
                Action::Sort(4),
                ui.sort_column == 4,
            );
        }
        s.button(
            Rect::new(dur_x, y, dur_w.max(30), 24),
            format!("{}{}", ui.t("Duration"), sort_indicator(5)),
            "",
            Action::Sort(5),
            ui.sort_column == 5,
        );
        s.panel(x, y + 29, cw, 1, 9);
        y += 34;

        let mut rows: Vec<&crate::catalog::Entry> = page.rows.iter().collect();
        if ui.sort_column > 0 {
            rows.sort_by(|a, b| {
                let cmp = match ui.sort_column {
                    1 => a.title.to_lowercase().cmp(&b.title.to_lowercase()),
                    2 => a.subtitle.to_lowercase().cmp(&b.subtitle.to_lowercase()),
                    3 => a.album.to_lowercase().cmp(&b.album.to_lowercase()),
                    4 => {
                        let da = row_date(a, is_podcast);
                        let db = row_date(b, is_podcast);
                        if is_podcast {
                            format_date(&da).cmp(&format_date(&db))
                        } else {
                            match (da.parse::<i64>(), db.parse::<i64>()) {
                                (Ok(na), Ok(nb)) => na.cmp(&nb),
                                _ => da.cmp(&db),
                            }
                        }
                    }
                    5 => a.duration.cmp(&b.duration),
                    _ => a.duration.cmp(&b.duration),
                };
                let res = if ui.sort_descending {
                    cmp.reverse()
                } else {
                    cmp
                };
                res.then_with(|| a.id.cmp(&b.id))
            });
        }
        for item in rows {
            let ahead = (h - 124 - s.content_top).max(240);
            if y + 56 >= s.content_top - ahead && y < h - 124 + ahead {
                s.artwork_ahead.push((y, item.artwork.clone()));
            }
            if y + 56 >= s.content_top - 160 && y < h + 36 {
                let active_track = !ui.now.id.is_empty() && ui.now.id == item.id;
                let checked = ui.checked_ids.contains(&item.id);
                if active_track || checked {
                    s.panel(x + 2, y + 2, cw - 4, 52, 10);
                }
                s.pictures
                    .push((Rect::new(x + 4, y + 6, 44, 44), item.artwork.clone()));
                let explicit = item.raw["explicit"]
                    .as_bool()
                    .or(item.raw["isExplicit"].as_bool())
                    .or(item.raw["EXPLICIT_LYRICS"].as_str().map(|v| v == "1"))
                    .unwrap_or(false);
                let title_display = if explicit {
                    format!("{}  E", item.title)
                } else {
                    item.title.clone()
                };
                s.button(
                    Rect::new(track_x - 6, y + 4, title_width - 4, 48),
                    title_display,
                    "",
                    Action::OpenItem(item.clone()),
                    active_track,
                );
                if let Some(button) = s.buttons.last_mut() {
                    button.accessible = item.title.clone();
                }
                if show_artist {
                    s.artist_links(
                        Rect::new(artist_x, y + 14, (artist_w - 10).max(1), 28),
                        &item,
                        false,
                    );
                }
                if show_album {
                    if let Some(album) = item.album_entry() {
                        s.link(
                            Rect::new(album_x, y + 14, (album_w - 10).max(1), 28),
                            &item.album,
                            album,
                            false,
                        );
                    } else {
                        s.label(album_x, y + 14, album_w - 10, &item.album, 1);
                    }
                }
                if show_date {
                    let raw_date = row_date(item, is_podcast);
                    let formatted_date = format_date(&raw_date);
                    s.label(
                        date_x,
                        y + 14,
                        date_w - 8,
                        if formatted_date.is_empty() {
                            "\u{2014}"
                        } else {
                            &formatted_date
                        },
                        1,
                    );
                }
                let seconds = item.duration / 1000;
                s.label(
                    dur_x,
                    y + 14,
                    dur_w,
                    format!("{}:{:02}", seconds / 60, seconds % 60),
                    1,
                );
                if show_check {
                    s.button(
                        Rect::new(check_x, y + 14, 28, 28),
                        "",
                        if checked { "\u{e73d}" } else { "\u{e739}" },
                        Action::ToggleCheck(item.id.clone()),
                        checked,
                    );
                }
            }
            y += 56;
        }
    }
    // HomeShelves extends slightly beyond the title's inset, as in the source.
    let x = x - 14;
    let cw = cw + 14;
    // ShelfGeometry.Measure: columns and the 160px cover cap match the source.
    let cols = (((cw - 12) as f64 * ui.dpi_scale) as i32 / 200).max(1);
    let cell = (cw - 12) / cols;
    let cover = ((160.0 / ui.dpi_scale).round() as i32)
        .min(cell - 16)
        .max(32);
    let discover = ui.view == View::Explore && page.is_discover();
    for section in &page.sections {
        if section.entries.is_empty() && section.error.is_empty() {
            continue;
        }
        {}
        if ui.view == View::Explore
            && ui.filter > 0
            && if page.is_discover() {
                !crate::catalog::discover_section_matches(section, ui.filter)
            } else {
                !section.entries.iter().any(|e| {
                    e.kind
                        == ["", "track", "artist", "playlist", "album", "podcast"][ui.filter.min(5)]
                })
            }
        {
            continue;
        }
        let is_tiles = section.entries.iter().all(|e| e.kind == "channel");
        let shelf = !is_tiles
            && matches!(
                ui.view,
                View::Home | View::Explore | View::Artist | View::Podcasts
            );
        let section_title = if ui.view == View::Playlists
            && (section.title == "Playlists" || section.title == "Kitaplık")
        {
            ui.t("Playlists")
        } else {
            ui.t(&section.title)
        };
        s.label(x + 8, y, (cw - 274).max(100), section_title, 3);
        let mut items: Vec<_> = section
            .entries
            .iter()
            .filter(|e| {
                ui.view != View::Explore
                    || discover
                    || ui.filter == 0
                    || e.kind
                        == ["", "track", "artist", "playlist", "album", "podcast"][ui.filter.min(5)]
            })
            .collect();
        let shelf_has_more = shelf && items.len() > cols as usize;
        if shelf_has_more && !discover {
            s.button(
                Rect::new(x + cw - 260, y, 136, 36),
                ui.t("Show all"),
                "",
                Action::ShelfAll(section.title.clone()),
                false,
            );
            s.button(
                Rect::new(x + cw - 112, y, 38, 36),
                "",
                "\u{e76b}",
                Action::Shelf(section.title.clone(), -cols),
                false,
            );
            s.button(
                Rect::new(x + cw - 66, y, 38, 36),
                "",
                "\u{e76c}",
                Action::Shelf(section.title.clone(), cols),
                false,
            );
        }
        y += 40;
        if !section.error.is_empty() {
            s.label(x, y, cw, &section.error, 1);
            y += 60;
            continue;
        }
        if is_tiles {
            let tile_cols = (cw / 220).max(2);
            let tile_w = (cw - 12) / tile_cols;
            let tile_limit = tile_cols as usize * 2;
            let tiles_have_more = discover && ui.filter == 0 && items.len() > tile_limit;
            if tiles_have_more {
                items.truncate(tile_limit);
            }
            for (row, chunk) in items.chunks(tile_cols as usize).enumerate() {
                let yy = y + row as i32 * 102;
                if yy + 94 >= s.content_top - 160 && yy <= h + 36 {
                    for (col, item) in chunk.iter().enumerate() {
                        let xx = x + 6 + col as i32 * tile_w;
                        s.button(
                            Rect::new(xx, yy, tile_w - 12, 90),
                            &item.title,
                            "",
                            Action::OpenItem((*item).clone()),
                            false,
                        );
                        if let Some(button) = s.buttons.last_mut() {
                            button.artwork = item.artwork.clone();
                            button.accessible = format!("Open {}", item.title);
                        }
                    }
                }
            }
            y += ((items.len() as i32 + tile_cols - 1) / tile_cols) * 102 + 20;
            if tiles_have_more {
                discover_show_all(s, ui, x, &mut y, cw, &section.title);
            }
            continue;
        }
        if shelf {
            let offset = ui
                .shelf_offsets
                .get(&section.title)
                .copied()
                .unwrap_or(0)
                .min(items.len().saturating_sub(1));
            let visible = (cols as usize).min(items.len());
            items = items
                .iter()
                .copied()
                .cycle()
                .skip(offset)
                .take(visible)
                .collect();
        }
        let label_w = (cell - 28).max(20);
        for chunk in items.chunks(cols as usize) {
            let metadata: Vec<_> = chunk
                .iter()
                .map(|item| {
                    let title = plain_description(&item.title);
                    let subtitle = if item.kind == "podcast" {
                        item.episode_count()
                            .map(|count| {
                                format!("{count} {}", ui.t(if count == 1 { "episode" } else { "episodes" }))
                            })
                            .unwrap_or_else(|| item.subtitle.clone())
                    } else if item.kind == "artist" && item.subtitle.ends_with(" followers") {
                        format!(
                            "{} {}",
                            item.subtitle.trim_end_matches(" followers"),
                            ui.t("followers")
                        )
                    } else {
                        item.subtitle.clone()
                    };
                    let subtitle = plain_description(&subtitle);
                    (title, subtitle)
                })
                .collect();
            // Original DrawCard: one title line + one subtitle line, regardless of text length.
            let row_height = cover + 88;
            let yy = y;
            let ahead = (h - 124 - s.content_top).max(240);
            if yy + row_height >= s.content_top - ahead && yy < h - 124 + ahead {
                s.artwork_ahead
                    .extend(chunk.iter().map(|item| (yy, item.artwork.clone())));
            }
            if yy + row_height >= s.content_top - 160 && yy < h + 36 {
                for (column, (item, (title, subtitle))) in
                    chunk.iter().zip(metadata.iter()).enumerate()
                {
                    let slot = x + 6 + column as i32 * cell;
                    let xx = slot + (cell - 8 - cover) / 2;
                    s.button(
                        Rect::new(xx, yy, cover, cover),
                        "",
                        "\u{e8d6}",
                        Action::OpenItem((*item).clone()),
                        false,
                    );
                    if let Some(button) = s.buttons.last_mut() {
                        button.artwork = item.artwork.clone();
                        button.accessible = format!("Open {}", item.title);
                    }
                    let label_x = xx + cover / 2 - label_w / 2;
                    s.link(
                        Rect::new(label_x, yy + cover + 12, label_w, 23),
                        title,
                        (*item).clone(),
                        true,
                    );
                    if let Some(button) = s.buttons.last_mut() {
                        button.accessible = item.title.clone();
                    }
                    if item.artist_entries().is_empty() {
                        s.label_sized(label_x, yy + cover + 35, label_w, 21, subtitle, 11);
                    } else {
                        s.artist_links(
                            Rect::new(label_x, yy + cover + 35, label_w, 21),
                            &item,
                            true,
                        );
                    }
                }
            }
            y += row_height;
        }
        if discover && shelf_has_more {
            discover_show_all(s, ui, x, &mut y, cw, &section.title);
        }
        y += 20;
    }
    if !page.next.is_empty() && !page.loading {
        s.button(
            Rect::new(x, y, 140, 36),
            "Load more",
            "",
            Action::More,
            false,
        );
        y += 58;
    }
    if !page.loading
        && page.error.is_empty()
        && page.rows.is_empty()
        && page.sections.iter().all(|s| s.entries.is_empty())
    {
        s.label(
            x,
            y + 40,
            cw,
            if ui.view == View::Podcasts {
                ui.t("Your saved podcasts, favorites and downloaded episodes appear here.")
            } else if ui.view == View::Offline {
                "Downloaded tracks will appear here."
            } else {
                "No items were returned by Deezer."
            },
            1,
        );
        y += 90;
    }
    s.content_height = y + ui.scroll;
}

fn text_button_width(label: &str) -> i32 {
    (label.chars().count() as i32 * 10 + 32).max(100)
}

fn discover_show_all(s: &mut Scene, ui: &Ui, x: i32, y: &mut i32, width: i32, title: &str) {
    let label = ui.t("Show all");
    let button_width = text_button_width(label);
    s.button(
        Rect::new(x + (width - button_width) / 2, *y, button_width, 36),
        label,
        "",
        Action::ShelfAll(title.to_owned()),
        false,
    );
    *y += 48;
}

fn plain_description(text: &str) -> String {
    let mut output = String::new();
    let mut in_tag = false;
    for ch in text.chars() {
        match ch {
            '<' => {
                in_tag = true;
                output.push(' ');
            }
            '>' => in_tag = false,
            _ if !in_tag => output.push(ch),
            _ => {}
        }
    }
    output
        .replace("&amp;", "&")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}
fn wrap_text(text: &str, width: i32, font: i32) -> Vec<String> {
    if text.trim().is_empty() {
        return Vec::new();
    }
    let limit = width.max(font * 2) as f32;
    let measure = |ch: char| -> f32 {
        font as f32
            * if ch.is_whitespace() {
                0.38
            } else if "ilI.,'!:;|".contains(ch) {
                0.35
            } else if "MWmw@".contains(ch) || (ch as u32) >= 0x2e80 {
                1.0
            } else {
                0.64
            }
    };
    let mut lines = Vec::new();
    let mut line = String::new();
    let mut used = 0.0;
    for word in text.split_whitespace() {
        let word_width: f32 = word.chars().map(&measure).sum();
        if !line.is_empty() && used + measure(' ') + word_width > limit {
            lines.push(std::mem::take(&mut line));
            used = 0.0;
        }
        if !line.is_empty() {
            line.push(' ');
            used += measure(' ');
        }
        for ch in word.chars() {
            let size = measure(ch);
            if used + size > limit && !line.is_empty() {
                lines.push(std::mem::take(&mut line));
                used = 0.0;
            }
            line.push(ch);
            used += size;
        }
    }
    if !line.is_empty() {
        lines.push(line);
    }
    lines
}
#[cfg(test)]
mod tests {
    #[test]
    fn compact_queue_metadata_remains_reachable_in_small_windows() {
        let name = format!("Çağrı {} 日本語", "W".repeat(54));
        let mut ui = Ui { queue_sidebar_open: true, ..Default::default() };
        ui.queue = (0..20).map(|i| crate::catalog::Entry {
            id: i.to_string(), title: "A long track title with several words".into(),
            subtitle: "An artist with a long credit".into(), artwork: "queue-cover".into(),
            raw: serde_json::json!({"added_by":name}), ..Default::default()
        }).collect();
        ui.now = ui.queue[0].clone();
        for size in [(800, 600), (1200, 800), (1600, 1000)] {
            let scene = build(&ui, size, true, false, "", false, 50);
            assert!(scene.queue_scroll_max > 0);
            assert!(scene.buttons.iter().filter(|b| matches!(b.action, Action::QueueJump(_))).all(|b| b.artwork.is_empty() && b.rect.h <= 72));
            assert!(scene.pictures.iter().any(|(r, url)| url == "queue-cover" && r.w == 44 && r.h == 44));
            let mut seen = String::new();
            for offset in 0..=scene.queue_scroll_max {
                ui.queue_scroll = offset;
                let frame = build(&ui, size, true, false, "", false, 50);
                for label in &frame.labels {
                    if label.rect.x >= frame.queue_viewport.unwrap().x {
                        assert!(label.rect.y + label.rect.h <= size.1 - player_height(&ui, size.0));
                        seen.push_str(&label.text);
                    }
                }
            }
            assert!(seen.contains("Added by"));
            assert!(seen.contains("日本語"));
            ui.queue_scroll = 0;
        }
    }
    #[test]
    fn timed_lyric_follow_resumes_after_manual_scroll_but_untimed_scroll_stays_put(){
        let now=std::time::Instant::now();let mut ui=Ui::default();ui.view=View::Lyrics;
        ui.rich_lyrics=vec![crate::lyrics::LyricLine{start_ms:1000,end_ms:2000,text:"First".into(),..Default::default()},crate::lyrics::LyricLine{start_ms:2000,end_ms:3000,text:"Second".into(),..Default::default()}];
        ui.position=2500;ui.hold_lyric_follow(now);
        assert!(build(&ui,(1200,800),true,false,"",true,50).lyrics_anchor.is_none());
        assert!(!ui.resume_lyric_follow(now+std::time::Duration::from_secs(3)));
        assert!(ui.resume_lyric_follow(now+std::time::Duration::from_secs(4)));
        assert!(build(&ui,(1200,800),true,false,"",true,50).lyrics_anchor.is_some());
        ui.rich_lyrics.iter_mut().for_each(|line|{line.start_ms=0;line.end_ms=0;});
        ui.scroll=600;ui.hold_lyric_follow(now);
        assert!(!ui.lyrics_manual_scroll);
        assert!(!ui.resume_lyric_follow(now+std::time::Duration::from_secs(10)));
        assert_eq!(ui.scroll,600);
    }
    #[test]
    fn hiding_lyrics_centers_the_cover_and_moves_its_controls_together(){
        let mut ui=Ui::default();ui.view=View::Lyrics;
        let left=build(&ui,(1200,800),true,false,"",true,50);
        ui.lyrics_cover_mode=true;ui.lyrics_cover_mix=Some(0.5);
        let middle=build(&ui,(1200,800),true,false,"",true,50);
        ui.lyrics_cover_mix=None;
        let centered=build(&ui,(1200,800),true,false,"",true,50);
        let a=left.lyrics_cover.unwrap();let b=middle.lyrics_cover.unwrap();let c=centered.lyrics_cover.unwrap();
        assert!((c.x+c.w/2-600).abs()<=1);
        assert!(a.x<b.x && b.x<c.x);
        assert_eq!(left.seek.unwrap().x-a.x,centered.seek.unwrap().x-c.x);
        assert_eq!(left.seek.unwrap().y-a.y,centered.seek.unwrap().y-c.y);
        let button=centered.buttons.iter().find(|b|matches!(b.action,Action::ToggleLyricsVisualizer)).unwrap();
        assert_eq!(button.rect.x,32);assert!(button.rect.y>700);
    }
    #[test]
    fn lyrics_metadata_uses_the_left_column_without_touching_the_words(){
        let mut ui=Ui::default();ui.view=View::Lyrics;
        ui.now.title="A very long song title that needs the entire left column".into();
        ui.now.subtitle="Jessie Reyez and an unusually long artist credit".into();
        for size in [(746,650),(1200,800),(1600,1000)]{
            let scene=build(&ui,size,true,false,"",true,50);
            let cover=scene.lyrics_cover.unwrap();
            let area=scene.lyrics_area.unwrap();
            let title=scene.labels.iter().find(|l|l.kind==10).unwrap();
            assert!(title.rect.w>=cover.w);
            assert!(title.rect.x+title.rect.w<=area.x-8);
            assert_eq!(title.text,ui.now.title);
            for artist in scene.buttons.iter().filter(|b|b.artist_credit_group.is_some()){
                assert!(artist.rect.x+artist.rect.w<=area.x-8);
            }
        }
    }
    #[test]
    fn long_untimed_lyrics_start_at_top_and_keep_an_accessible_scrollbar(){
        let mut ui=Ui::default();ui.view=View::Lyrics;
        ui.rich_lyrics=(0..150).map(|i|crate::lyrics::LyricLine{text:format!("Complete lyric line {i}"),..Default::default()}).collect();
        let scene=build(&ui,(1200,800),true,false,"",false,50);
        let first=scene.labels.iter().find(|l|l.text=="Complete lyric line 0").unwrap();
        assert_eq!(first.rect.y,40);
        assert!(scene.panels.iter().any(|p|p.kind==6));
        ui.scroll=1200;ui.lyrics_manual_scroll=true;ui.go(View::Artist);ui.back();
        assert_eq!(ui.scroll,0);assert!(!ui.lyrics_manual_scroll);
        let restored=build(&ui,(1200,800),true,false,"",false,50);
        assert_eq!(restored.labels.iter().find(|l|l.text=="Complete lyric line 0").unwrap().rect.y,40);
    }
    #[test]
    fn theme_controls_are_available_over_song_art_and_adaptive_colors_are_gone(){
        let ui=Ui{view:View::Settings,tab:2,background_mode:1,..Default::default()};
        let scene=build(&ui,(1200,900),true,false,"",false,50);
        assert!(scene.buttons.iter().any(|b|matches!(b.action,Action::SolidTheme(_))));
        assert!(!scene.buttons.iter().any(|b|matches!(b.action,Action::ToggleBool("adaptive_colors"))));
        assert!(scene.sliders.iter().any(|(_,key,_)|*key=="Surface opacity"));
    }
    use super::*;

    #[test]
    fn startup_page_selection_survives_reloading_preferences() {
        for (index, expected) in [View::Home, View::Explore, View::Flow, View::Favorites, View::Playlists, View::Offline].into_iter().enumerate() {
            let mut ui = Ui::default();
            ui.choices.insert("Show on startup", index);
            ui.save_preferences();
            let loaded = Ui::load_preferences();
            assert_eq!(loaded.value("Show on startup"), index);
            assert_eq!(loaded.view, expected);
        }
        let base = preferences_base().unwrap();
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn bundled_languages_have_matching_keys_and_valid_unicode() {
        let tr: std::collections::HashMap<String, String> = serde_json::from_str(include_str!("../../../assets/localization/tr.json")).unwrap();
        let az: std::collections::HashMap<String, String> = serde_json::from_str(include_str!("../../../assets/localization/az.json")).unwrap();
        assert_eq!(tr.keys().collect::<std::collections::HashSet<_>>(), az.keys().collect());
        for catalog in [&tr, &az] {
            assert!(catalog.iter().all(|(k, v)| !k.contains('\u{fffd}') && !v.is_empty()));
        }
    }

    #[test]
    fn language_switch_updates_catalog_options_and_formatted_labels_immediately() {
        let mut ui = Ui::default();
        for (language, choice) in [("tr", 1), ("az", 3), ("en", 2)] {
            ui.set_language(language);
            assert_eq!(ui.language, language);
            assert_eq!(ui.value("Language"), choice);
            if language == "en" {
                assert_eq!(ui.translated_text("12 tracks"), "12 tracks");
            } else {
                assert_ne!(ui.t("Language"), "Language");
                assert_ne!(ui.translated_text("English   \u{25be}"), "English   \u{25be}");
                assert_ne!(ui.translated_text("12 tracks"), "12 tracks");
                assert!(!ui.t("Welcome back").contains('?'));
            }
        }
        ui.set_language("");
        assert_eq!(ui.value("Language"), 0);
        assert!(["tr", "az", "en"].contains(&ui.language.as_str()));
    }

    #[test]
    fn translating_scene_preserves_media_names_in_the_queue() {
        let mut ui = Ui::default();
        ui.set_language("tr");
        ui.queue.push(crate::catalog::Entry { title: "Home".into(), ..Default::default() });
        let mut scene = Scene::default();
        scene.label(0, 0, 300, "Home", 8);
        scene.label(0, 30, 300, "Welcome back", 0);
        translate_scene(&mut scene, &ui);
        assert_eq!(scene.labels[0].text, "Home");
        assert_eq!(scene.labels[1].text, ui.t("Welcome back"));
    }
    #[test]
    fn discover_filters_keep_shelf_items_and_expand_tile_lists() {
        let sections = [
            ("Editorial playlists", "playlist"),
            ("Made for you", "mix"),
            ("New releases", "album"),
            ("Genres", "channel"),
            ("Categories", "channel"),
            ("Podcasts by category", "channel"),
        ]
        .into_iter()
        .map(|(title, kind)| crate::catalog::Section {
            title: title.into(),
            entries: (0..20)
                .map(|i| crate::catalog::Entry {
                    id: format!("{title}:{i}"),
                    title: format!("{title} {i}"),
                    kind: kind.into(),
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        })
        .collect();
        let page = crate::catalog::Page {
            sections,
            raw: vec![("discover".into(), serde_json::json!(true))],
            ..Default::default()
        };
        let mut ui = Ui {
            view: View::Explore,
            page: Some(std::sync::Arc::new(page.clone())),
            ..Default::default()
        };
        ui.set_language("tr");
        let scene = build(&ui, (1280, 4000), false, false, "", false, 70);
        let filters: Vec<_> = scene
            .buttons
            .iter()
            .filter(|b| matches!(b.action, Action::Filter(_)))
            .collect();
        assert_eq!(
            filters.iter().map(|b| b.text.as_str()).collect::<Vec<_>>(),
            ["All", "Playlists", "Genres", "Moods", "Podcasts"].map(|text| ui.t(text))
        );
        assert!(filters[1].rect.w > filters[0].rect.w);
        for filter in 0..=4 {
            ui.filter = filter;
            let mut scene = Scene::default();
            live_catalog(&mut scene, &ui, &page, 0, 0, 900, 10000);
            let covers: Vec<_> = scene
                .buttons
                .iter()
                .filter_map(|b| match &b.action {
                    Action::OpenItem(item) if !b.text_link => Some((b, item)),
                    _ => None,
                })
                .collect();
            if filter <= 1 {
                for title in ["Editorial playlists", "Made for you"] {
                    let rows: std::collections::HashSet<_> = covers
                        .iter()
                        .filter(|(_, item)| item.id.starts_with(title))
                        .map(|(b, _)| b.rect.y)
                        .collect();
                    assert_eq!(rows.len(), 1);
                }
                assert_eq!(
                    scene
                        .buttons
                        .iter()
                        .filter(|b| matches!(b.action, Action::ShelfAll(_)))
                        .count(),
                    if filter == 0 { 6 } else { 2 }
                );
                if filter == 0 {
                    for title in ["Genres", "Categories", "Podcasts by category"] {
                        let rows: std::collections::HashSet<_> = covers
                            .iter()
                            .filter(|(_, item)| item.id.starts_with(title))
                            .map(|(b, _)| b.rect.y)
                            .collect();
                        assert_eq!(rows.len(), 2);
                    }
                } else {
                    assert!(covers
                        .iter()
                        .all(|(_, item)| matches!(item.kind.as_str(), "playlist" | "mix")));
                }
            } else {
                let title = ["", "", "Genres", "Categories", "Podcasts by category"][filter];
                assert_eq!(covers.len(), 20);
                assert!(covers.iter().all(|(_, item)| item.id.starts_with(title)));
                assert!(!scene
                    .buttons
                    .iter()
                    .any(|b| matches!(b.action, Action::ShelfAll(_))));
            }
        }
    }

    #[test]
    fn plain_lyrics_use_large_wrapped_static_rows_and_preserve_stanzas() {
        let mut ui = Ui { view:View::Lyrics, rich_lyrics:vec![
            crate::lyrics::LyricLine { text:"A long lyric line that must wrap into several readable rows instead of shrinking or being truncated".into(),..Default::default() },
            crate::lyrics::LyricLine::default(),
            crate::lyrics::LyricLine { text:"The next stanza".into(),..Default::default() },
        ],..Default::default() };
        let first = build(&ui, (1000, 800), true, false, "", false, 50);
        ui.position = 90000;
        let later = build(&ui, (1000, 800), true, false, "", true, 50);
        let rows = |s: &Scene| {
            s.labels
                .iter()
                .filter(|l| l.kind == 14)
                .map(|l| (l.text.clone(), l.rect.y, l.rect.h))
                .collect::<Vec<_>>()
        };
        assert_eq!(rows(&first), rows(&later));
        assert!(rows(&first).len() > 4);
        assert!(rows(&first).iter().all(|(_, _, h)| *h == 44));
        assert!(first.lyrics_anchor.is_none() && first.lyrics_gap.is_none());
        assert!(!first
            .buttons
            .iter()
            .any(|b| matches!(b.action, Action::SeekMs(_))));
    }

    #[test]
    fn timed_breaks_show_notes_only_during_known_instrumental_intervals() {
        let mut ui = Ui {
            view: View::Lyrics,
            duration: 20000,
            rich_lyrics: vec![
                crate::lyrics::LyricLine {
                    start_ms: 5000,
                    end_ms: 7000,
                    text: "First line".into(),
                    ..Default::default()
                },
                crate::lyrics::LyricLine {
                    start_ms: 15000,
                    end_ms: 17000,
                    text: "Second line".into(),
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        for (position, notes) in [
            (1000, true),
            (6000, false),
            (10000, true),
            (16000, false),
            (18000, true),
        ] {
            ui.position = position;
            let scene = build(&ui, (1200, 800), true, false, "", true, 50);
            assert_eq!(scene.labels.iter().any(|l| l.kind == 15), notes);
            assert_eq!(scene.lyrics_gap.is_some(), notes);
        }
    }
    #[test]
    fn narrow_settings_stack_fields_and_wrap_descriptions_at_normal_size() {
        for width in [340, 484, 580] {
            for tab in 0..6 {
                let ui = Ui {
                    view: View::Settings,
                    tab,
                    ..Default::default()
                };
                let mut scene = Scene::default();
                settings(&mut scene, &ui, 0, 150, width, false, false);
                let tabs: Vec<_> = scene
                    .buttons
                    .iter()
                    .filter(|b| matches!(b.action, Action::Tab(_)))
                    .collect();
                assert_eq!(tabs.len(), 6);
                assert!(tabs
                    .iter()
                    .all(|b| b.text.is_empty() && !b.accessible.is_empty()));
                assert!(scene
                    .labels
                    .iter()
                    .all(|l| l.rect.w > 0 && l.rect.x >= 0 && l.rect.x + l.rect.w <= width));
                assert!(scene
                    .buttons
                    .iter()
                    .all(|b| b.rect.x >= 0 && b.rect.x + b.rect.w <= width));
                if tab == 4 {
                    let normalize = scene
                        .buttons
                        .iter()
                        .find(|b| {
                            matches!(b.action, Action::Setting("Normalize volume (ReplayGain)"))
                        })
                        .unwrap();
                    let gapless = scene
                        .buttons
                        .iter()
                        .find(|b| matches!(b.action, Action::Setting("Gapless playback")))
                        .unwrap();
                    assert_eq!(normalize.rect.x, gapless.rect.x);
                    assert!(gapless.rect.y >= normalize.rect.y + normalize.rect.h);
                }
            }
        }
    }
    #[test]
    fn resizing_folds_sidebars_and_keeps_narrow_player_controls_separate() {
        let mut ui = Ui {
            view: View::Favorites,
            queue_sidebar_open: true,
            ..Default::default()
        };
        ui.fit_sidebars(1200);
        assert!(!ui.sidebar_collapsed && ui.queue_sidebar_open);
        ui.fit_sidebars(1080);
        assert!(ui.sidebar_collapsed && ui.queue_sidebar_open);
        ui.fit_sidebars(900);
        assert!(ui.sidebar_collapsed && !ui.queue_sidebar_open);
        for width in [600, 720, 900, 1080, 1400] {
            for layout in [0, 1] {
                ui.player_layout = layout;
                ui.fit_sidebars(width);
                let scene = build(
                    &ui,
                    (width, 740),
                    true,
                    false,
                    "A long song title",
                    true,
                    50,
                );
                let seek = scene.seek.unwrap();
                let volume = scene.volume.unwrap();
                assert!(seek.x >= 0 && seek.x + seek.w <= volume.x);
                assert!(volume.x + volume.w <= width);
                assert!(seek.y + seek.h <= 740);
                let controls: Vec<_> = scene
                    .buttons
                    .iter()
                    .filter(|b| {
                        matches!(
                            b.action,
                            Action::Shuffle
                                | Action::Previous
                                | Action::Toggle
                                | Action::Next
                                | Action::Repeat
                        )
                    })
                    .collect();
                assert_eq!(controls.len(), 5);
                for button in &controls {
                    assert!(button.rect.x >= 0 && button.rect.x + button.rect.w <= width);
                }
                for clock in [scene.elapsed.unwrap(), scene.remaining.unwrap()] {
                    assert!(clock.x >= 0 && clock.x + clock.w <= width);
                    if player_height(&ui, width) == 112 {
                        assert!(controls.iter().all(|b| clock.y >= b.rect.y + b.rect.h));
                    } else {
                        assert!(controls.iter().all(|b| clock.x >= b.rect.x + b.rect.w));
                    }
                }
            }
        }
    }

    #[test]
    fn narrow_tables_drop_secondary_columns_before_squeezing_titles() {
        let page = std::sync::Arc::new(crate::catalog::Page {
            rows: vec![crate::catalog::Entry {
                id: "song".into(),
                title: "A song title".into(),
                kind: "track".into(),
                ..Default::default()
            }],
            ..Default::default()
        });
        for (width, columns) in [
            (600, vec![1, 5]),
            (900, vec![1, 2, 5]),
            (1400, vec![1, 2, 3, 4, 5]),
        ] {
            let mut ui = Ui {
                view: View::Favorites,
                page: Some(page.clone()),
                ..Default::default()
            };
            ui.fit_sidebars(width);
            let scene = build(&ui, (width, 800), true, false, "", false, 50);
            let actual: Vec<_> = scene
                .buttons
                .iter()
                .filter_map(|b| {
                    if let Action::Sort(col) = b.action {
                        Some(col)
                    } else {
                        None
                    }
                })
                .collect();
            assert_eq!(actual, columns, "width {width}");
            let title = scene
                .buttons
                .iter()
                .find(|b| matches!(b.action, Action::Sort(1)))
                .unwrap();
            assert!(title.rect.w >= 200, "title squeezed at {width}");
        }
    }
    #[test]
    fn reference_home_uses_capped_covers_and_a_main_column_player() {
        let ui = Ui {
            dpi_scale: 1.25,
            page: Some(std::sync::Arc::new(crate::catalog::Page {
                sections: vec![crate::catalog::Section {
                    title: "Recently played".into(),
                    entries: (0..8)
                        .map(|i| crate::catalog::Entry {
                            id: i.to_string(),
                            title: format!("Playlist {i}"),
                            kind: "playlist".into(),
                            artwork: "fixture".into(),
                            ..Default::default()
                        })
                        .collect(),
                    ..Default::default()
                }],
                ..Default::default()
            })),
            ..Default::default()
        };
        let scene = build(&ui, (1043, 674), true, false, "", false, 40);
        assert!(scene.search.is_none());
        let covers: Vec<_> = scene
            .buttons
            .iter()
            .filter(|b| matches!(b.action, Action::OpenItem(_)) && !b.text_link)
            .collect();
        assert_eq!(covers.len(), 4);
        assert!(covers
            .iter()
            .all(|b| (b.rect.w as f64 * ui.dpi_scale - 160.0).abs() < 1.0));
        let player = scene.panels.last().unwrap();
        assert_eq!(player.rect, Rect::new(0, 674 - 112, 1043, 112));
        assert!(scene.volume.unwrap().x > player.rect.x);
        let play_btn = scene
            .buttons
            .iter()
            .find(|b| matches!(b.action, Action::Toggle))
            .unwrap();
        let seek = scene.seek.unwrap();
        assert_eq!(seek.y - (play_btn.rect.y + play_btn.rect.h), 24);
        assert!(!scene
            .buttons
            .iter()
            .any(|b| matches!(b.action, Action::Stop)));
    }
    #[test]
    fn flow_save_controls_remain_visible_while_genres_scroll() {
        let mut ui = Ui {
            view: View::FlowTune,
            flow_loaded: true,
            flow_customizable: true,
            genres: (0..120)
                .map(|i| (i.to_string(), format!("Genre {i}"), i == 0, i == 0))
                .collect(),
            ..Default::default()
        };
        let first = build(&ui, (1043, 674), true, false, "", false, 40);
        ui.scroll = 900;
        let scrolled = build(&ui, (1043, 674), true, false, "", false, 40);
        let save = |scene: &Scene| {
            scene
                .buttons
                .iter()
                .find(|b| matches!(b.action, Action::ApplyFlow))
                .unwrap()
                .rect
                .y
        };
        assert_eq!(save(&first), save(&scrolled));
        assert!(save(&first) + 38 < 674 - 124);
        assert_eq!(
            first.genre_viewport.unwrap().y,
            scrolled.genre_viewport.unwrap().y
        );
    }
    #[test]
    fn entity_header_has_one_title_and_stays_outside_the_scrolling_rows() {
        let entity = crate::catalog::Entry {
            id: "1".into(),
            title: "Album title".into(),
            kind: "album".into(),
            artwork: "cover".into(),
            ..Default::default()
        };
        let ui = Ui {
            view: View::Album,
            page: Some(std::sync::Arc::new(crate::catalog::Page {
                title: entity.title.clone(),
                entity: Some(entity),
                ..Default::default()
            })),
            ..Default::default()
        };
        let scene = build(&ui, (1043, 674), true, false, "", false, 40);
        assert_eq!(
            scene
                .labels
                .iter()
                .filter(|l| l.text == "Album title")
                .count(),
            1
        );
        assert!(scene.content_top > 198);
        assert!(scene.pictures[0].0.y < scene.content_top);
        assert!(!scene.content_pictures.contains(&0));
    }
    #[test]
    fn original_navigation_and_settings_are_present() {
        let ui = Ui::default();
        let home = build(&ui, (1320, 840), false, false, "", false, 70);
        for (view, _) in NAV {
            assert!(home
                .buttons
                .iter()
                .any(|b| matches!(b.action, Action::Go(v) if v == view)));
        }
        let recent = home
            .buttons
            .iter()
            .find(|b| matches!(b.action, Action::Go(View::Recent)))
            .unwrap();
        let settings = home
            .buttons
            .iter()
            .find(|b| matches!(b.action, Action::Go(View::Settings)))
            .unwrap();
        assert_eq!(settings.rect.x, recent.rect.x);
        assert_eq!(settings.rect.y, recent.rect.y + 41);
        assert_eq!(settings.rect.w, recent.rect.w);
        assert_eq!(settings.rect.h, recent.rect.h);
    }
    #[test]
    fn connect_integration_exposes_only_running_server_links() {
        let mut ui = Ui::default();
        ui.go(View::Settings);
        ui.tab = 5;
        for size in [(520, 700), (1120, 700)] {
            ui.jam_enabled = true;
            ui.jam_url = "http://192.168.1.20:3132/".into();
            let scene = build(&ui, size, false, false, "", false, 70);
            assert!(scene.buttons.iter().any(|b| matches!(b.action, Action::OpenConnect)));
            assert!(scene.buttons.iter().any(|b| matches!(b.action, Action::CopyConnectLink)));
            ui.jam_enabled = false;
            let scene = build(&ui, size, false, false, "", false, 70);
            assert!(!scene.buttons.iter().any(|b| matches!(b.action, Action::OpenConnect | Action::CopyConnectLink)));
        }
    }

    #[test]
    fn settings_top_bar_and_tabs_remain_fixed_while_content_scrolls() {
        let mut ui = Ui::default();
        ui.go(View::Settings);
        ui.tab = 2;
        let s0 = build(&ui, (1200, 800), false, false, "", false, 70);
        ui.scroll = 300;
        let s1 = build(&ui, (1200, 800), false, false, "", false, 70);

        for tab in 0..TABS.len() {
            let b0 = s0
                .buttons
                .iter()
                .find(|b| matches!(b.action, Action::Tab(i) if i == tab))
                .unwrap();
            let b1 = s1
                .buttons
                .iter()
                .find(|b| matches!(b.action, Action::Tab(i) if i == tab))
                .unwrap();
            assert_eq!(b0.rect.y, b1.rect.y);
            let idx0 = s0
                .buttons
                .iter()
                .position(|b| matches!(b.action, Action::Tab(i) if i == tab))
                .unwrap();
            assert!(!s0.content_buttons.contains(&idx0));
        }

        assert!(s0.content_top >= 226);
        assert_eq!(s0.content_top, s1.content_top);

        let reset_btn_0 = s0
            .buttons
            .iter()
            .find(|b| matches!(b.action, Action::ResetTheme))
            .unwrap();
        let reset_btn_1 = s1
            .buttons
            .iter()
            .find(|b| matches!(b.action, Action::ResetTheme))
            .unwrap();
        assert_eq!(reset_btn_0.rect.y - 300, reset_btn_1.rect.y);
        let reset_idx = s0
            .buttons
            .iter()
            .position(|b| matches!(b.action, Action::ResetTheme))
            .unwrap();
        assert!(s0.content_buttons.contains(&reset_idx));
        ui.go(View::Settings);
        for tab in 0..TABS.len() {
            ui.tab = tab;
            let scene = build(&ui, (1120, 700), false, false, "", false, 70);
            assert!(scene
                .buttons
                .iter()
                .any(|b| matches!(b.action, Action::Tab(i) if i == tab) && b.selected));
            assert!(scene.buttons.iter().all(|b| b.rect.w > 0 && b.rect.h > 0));
        }
    }
    #[test]
    fn compact_sidebar_does_not_overlap_downloads() {
        for size in [(1120, 700), (1320, 840)] {
            let mut ui = Ui::default();
            ui.downloads.push(crate::downloads::Status::default());
            let scene = build(&ui, size, false, false, "", false, 70);
            let recent = scene
                .buttons
                .iter()
                .find(|b| matches!(b.action, Action::Go(View::Recent)))
                .unwrap();
            let downloads = scene
                .buttons
                .iter()
                .find(|b| matches!(b.action, Action::Go(View::Downloads)))
                .unwrap();
            assert!(recent.rect.y + recent.rect.h < downloads.rect.y);
        }
    }
    #[test]
    fn offline_tracks_use_live_rows_and_long_lists_are_virtualized() {
        let rows = (0..1562)
            .map(|i| crate::catalog::Entry {
                id: i.to_string(),
                title: format!("Track {i}"),
                kind: "track".into(),
                ..Default::default()
            })
            .collect();
        let mut ui = Ui {
            view: View::Offline,
            page: Some(std::sync::Arc::new(crate::catalog::Page {
                rows,
                ..Default::default()
            })),
            ..Default::default()
        };
        for offset in [0, 30_000, 65_000] {
            ui.scroll = offset;
            let scene = build(&ui, (1320, 840), true, false, "", false, 70);
            let visible = scene
                .buttons
                .iter()
                .filter(|b| matches!(b.action, Action::OpenItem(_)))
                .count();
            assert!(visible > 0 && visible < 20);
            assert!(scene.content_height > 70_000);
        }
    }
    #[test]
    fn account_buttons_and_scroll_extent_survive_small_windows() {
        let mut ui = Ui::default();
        ui.go(View::Account);
        let scene = build(&ui, (1120, 700), false, true, "", false, 70);
        assert!(scene.session.is_some());
        assert!(scene
            .buttons
            .iter()
            .any(|b| matches!(b.action, Action::ConnectSaved)));
        assert!(scene
            .buttons
            .iter()
            .any(|b| matches!(b.action, Action::Connect)));
        assert!(scene.content_height > 700 - 184);
        ui.go(View::Lyrics);
        ui.back();
        assert_eq!(ui.view, View::Account);
    }
    #[test]
    fn lyrics_page_covers_full_window_with_visualizer_and_back() {
        let mut ui = Ui::default();
        ui.go(View::Lyrics);
        let scene = build(&ui, (1200, 800), true, false, "Track Title", true, 80);
        assert!(scene
            .buttons
            .iter()
            .any(|b| matches!(b.action, Action::Back)));
        assert!(scene
            .buttons
            .iter()
            .any(|b| matches!(b.action, Action::ToggleLyricsCoverMode)));
        assert!(scene
            .buttons
            .iter()
            .any(|b| matches!(b.action, Action::ToggleLyricsVisualizer)));
        assert!(!scene
            .buttons
            .iter()
            .any(|b| matches!(b.action, Action::Go(View::Home))));
    }
    #[test]
    fn queue_sidebar_opens_and_shifts_content() {
        let mut ui = Ui::default();
        ui.queue_sidebar_open = true;
        let scene = build(&ui, (1200, 800), true, false, "Track Title", true, 80);
        assert!(scene
            .buttons
            .iter()
            .any(|b| matches!(b.action, Action::QueueReset)));
        assert!(scene
            .buttons
            .iter()
            .any(|b| matches!(b.action, Action::QueueClear)));
        assert!(scene
            .buttons
            .iter()
            .any(|b| matches!(b.action, Action::ToggleQueueSidebar) && b.selected));
    }
    #[test]
    fn collection_pages_do_not_duplicate_actions_in_overflow_menus() {
        for kind in ["podcast", "album", "playlist", "mix", "flow"] {
            let entry = crate::catalog::Entry {
                id: "1".into(),
                title: "Collection".into(),
                kind: kind.into(),
                artist_id: if kind == "album" {
                    "7".into()
                } else {
                    String::new()
                },
                subtitle: if kind == "album" {
                    "Artist".into()
                } else {
                    String::new()
                },
                ..Default::default()
            };
            let ui = Ui {
                view: if kind == "podcast" {
                    View::Show
                } else {
                    View::Album
                },
                page: Some(std::sync::Arc::new(crate::catalog::Page {
                    entity: Some(entry),
                    ..Default::default()
                })),
                ..Default::default()
            };
            let scene = build(&ui, (1200, 800), true, false, "", false, 50);
            assert!(!scene.buttons.iter().any(|b| b.text == "\u{22ef}"));
            assert_eq!(
                scene
                    .buttons
                    .iter()
                    .filter(|b| matches!(b.action, Action::PlayAll))
                    .count(),
                1
            );
            if kind == "album" {
                assert!(scene.buttons.iter().any(|b| matches!(&b.action, Action::OpenItem(e) if e.kind == "artist" && e.id == "7")));
            }
        }
    }
    #[test]
    fn queue_header_reserves_space_for_actions_at_narrow_widths() {
        let ui = Ui::default();
        for width in [160, 200, 240, 280, 360] {
            let mut scene = Scene::default();
            build_queue_sidebar(&mut scene, &ui, 10, width, 800);
            let title = scene.labels.iter().find(|l| l.text == "QUEUE").unwrap();
            let actions: Vec<_> = scene
                .buttons
                .iter()
                .filter(|b| matches!(b.action, Action::QueueReset | Action::QueueClear))
                .collect();
            assert_eq!(actions.len(), 2);
            assert!(title.rect.x + title.rect.w + 8 <= actions[0].rect.x);
            assert!(actions[0].rect.x + actions[0].rect.w < actions[1].rect.x);
            assert!(actions[1].rect.x + actions[1].rect.w <= 10 + width - 12);
            assert!(actions
                .iter()
                .all(|b| b.text.is_empty() && !b.accessible.is_empty()));
        }
    }
    #[test]
    fn queue_scroll_reaches_last_track_without_moving_main_page() {
        let mut ui = Ui {
            queue_sidebar_open: true,
            ..Default::default()
        };
        ui.queue = (0..100)
            .map(|i| crate::catalog::Entry {
                id: i.to_string(),
                title: format!("Song {i}"),
                kind: "track".into(),
                ..Default::default()
            })
            .collect();
        ui.now = ui.queue[0].clone();
        let first = build(&ui, (1200, 800), true, false, "", false, 50);
        assert!(first.queue_scrollbar.is_some());
        assert!(first.queue_scroll_max > 0);
        ui.queue_scroll = first.queue_scroll_max;
        let last = build(&ui, (1200, 800), true, false, "", false, 50);
        assert!(last
            .buttons
            .iter()
            .any(|b| matches!(b.action, Action::QueueJump(99))));
        assert!(last
            .buttons
            .iter()
            .any(|b| matches!(b.action, Action::QueueJump(0))));
        assert_eq!(ui.scroll, 0);
        assert!(
            last.buttons
                .iter()
                .filter(|b| matches!(b.action, Action::QueueJump(_)))
                .count()
                < 20
        );
    }
    #[test]
    fn queue_labels_are_identical_on_regular_and_lyrics_pages() {
        let mut ui = Ui { queue_sidebar_open: true, ..Default::default() };
        ui.now = crate::catalog::Entry { id: "track".into(), title: "Prismatic".into(), subtitle: "Svdden Death".into(), ..Default::default() };
        ui.queue = vec![ui.now.clone()];
        let regular = build(&ui, (1200, 800), true, false, "", true, 50);
        ui.view = View::Lyrics;
        let lyrics = build(&ui, (1200, 800), true, false, "", true, 50);
        let labels = |scene: &Scene| scene.labels[scene.queue_labels.clone()].iter()
            .map(|label| (label.text.clone(), label.kind, label.rect.w, label.rect.h))
            .collect::<Vec<_>>();
        assert!(!regular.queue_labels.is_empty());
        assert_eq!(labels(&regular), labels(&lyrics));
        assert!(lyrics.labels[lyrics.queue_labels.clone()].iter().any(|label| label.text == "Prismatic"));
    }
    #[test]
    fn queue_sidebar_shows_now_playing_and_next_up_songs() {
        let mut ui = Ui::default();
        ui.queue_sidebar_open = true;
        ui.queue = vec![
            crate::catalog::Entry {
                id: "t1".into(),
                title: "Song 1".into(),
                subtitle: "Artist 1".into(),
                kind: "track".into(),
                ..Default::default()
            },
            crate::catalog::Entry {
                id: "t2".into(),
                title: "Song 2".into(),
                subtitle: "Artist 2".into(),
                kind: "track".into(),
                ..Default::default()
            },
            crate::catalog::Entry {
                id: "t3".into(),
                title: "Song 3".into(),
                subtitle: "Artist 3".into(),
                kind: "track".into(),
                ..Default::default()
            },
        ];
        ui.queue_index = 0;
        ui.now = ui.queue[0].clone();

        let scene = build(&ui, (1200, 800), true, false, "Song 1", true, 80);
        assert!(scene.labels.iter().any(|l| l.text == "NOW PLAYING"));
        assert!(scene.labels.iter().any(|l| l.text == "NEXT UP"));
        assert!(!scene.labels.iter().any(|l| l.text == "HISTORY"));
        assert!(scene
            .buttons
            .iter()
            .any(|b| matches!(b.action, Action::QueueJump(0))));
        assert!(scene
            .buttons
            .iter()
            .any(|b| matches!(b.action, Action::QueueJump(1))));
        assert!(scene
            .buttons
            .iter()
            .any(|b| matches!(b.action, Action::QueueJump(2))));

        // When queue advances to index 1:
        ui.queue_index = 1;
        ui.now = ui.queue[1].clone();
        let scene2 = build(&ui, (1200, 800), true, false, "Song 2", true, 80);
        assert!(!scene2.labels.iter().any(|l| l.text == "HISTORY"));
        assert!(!scene2
            .buttons
            .iter()
            .any(|b| matches!(b.action, Action::QueueJump(0))));
        assert_eq!(ui.queue[0].id, "t1");
        assert_eq!(ui.queue_index, 1);
        assert!(scene2.labels.iter().any(|l| l.text == "NOW PLAYING"));
        assert!(scene2.labels.iter().any(|l| l.text == "NEXT UP"));
    }
    #[test]
    fn artist_page_and_custom_colors_views() {
        let mut ui = Ui::default();
        // 1. Verify custom colors view
        ui.go(View::CustomColors);
        let scene = build(&ui, (1200, 800), false, false, "", false, 70);
        assert!(scene.labels.iter().any(|l| l.text == "Color customization"));
        assert!(scene
            .buttons
            .iter()
            .any(|b| matches!(b.action, Action::Color(_))));
        assert!(scene
            .buttons
            .iter()
            .any(|b| matches!(b.action, Action::ResetTheme)));
        assert!(scene
            .buttons
            .iter()
            .any(|b| matches!(b.action, Action::Back)));

        // 2. Verify artist page view structure
        ui.go(View::Artist);
        let artist_entry = crate::catalog::Entry {
            id: "123".into(),
            title: "Sagopa Kajmer".into(),
            subtitle: "852.497 followers".into(),
            kind: "artist".into(),
            artwork: "https://example.com/artist.jpg".into(),
            ..Default::default()
        };
        let page = crate::catalog::Page {
            title: "Sagopa Kajmer".into(),
            entity: Some(artist_entry),
            sections: vec![
                crate::catalog::Section {
                    title: "Popular tracks".into(),
                    entries: vec![crate::catalog::Entry {
                        id: "t1".into(),
                        title: "Karikatür Komedya".into(),
                        kind: "track".into(),
                        ..Default::default()
                    }],
                    ..Default::default()
                },
                crate::catalog::Section {
                    title: "Albums".into(),
                    entries: vec![crate::catalog::Entry {
                        id: "a1".into(),
                        title: "TEK 2".into(),
                        kind: "album".into(),
                        ..Default::default()
                    }],
                    ..Default::default()
                },
                crate::catalog::Section {
                    title: "Similar artists".into(),
                    entries: vec![crate::catalog::Entry {
                        id: "art1".into(),
                        title: "Ceza".into(),
                        subtitle: "781.664 followers".into(),
                        kind: "artist".into(),
                        ..Default::default()
                    }],
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        ui.page = Some(std::sync::Arc::new(page));
        let artist_scene = build(&ui, (1200, 800), true, false, "", false, 70);
        assert!(!artist_scene.pictures.is_empty());
        assert!(!artist_scene
            .buttons
            .iter()
            .any(|b| matches!(b.action, Action::Share)));
        assert!(artist_scene
            .labels
            .iter()
            .any(|l| l.text == "Popular tracks" || l.text == "En çok dinlenenler"));
        assert!(artist_scene
            .labels
            .iter()
            .any(|l| l.text == "Albums" || l.text == "Albümler"));
        assert!(artist_scene
            .labels
            .iter()
            .any(|l| l.text == "Similar artists" || l.text == "Benzer sanatçýlar"));
    }
    #[test]
    fn track_info_view_renders_details() {
        let mut ui = Ui::default();
        ui.go(View::TrackInfo);
        let scene = build(&ui, (1200, 800), true, false, "", false, 80);
        assert!(scene
            .buttons
            .iter()
            .any(|b| matches!(b.action, Action::OpenItem(_))));
        assert!(scene.labels.iter().any(|l| l.text == "ABOUT THIS TRACK"));
        assert!(scene.labels.iter().any(|l| l.text == "CONTRIBUTORS"));
    }
    #[test]
    fn responsive_buttons_fit_across_window_sizes_and_tabs() {
        for size in [(640, 480), (800, 600), (1024, 768), (1440, 900)] {
            let mut ui = Ui::default();
            ui.go(View::Settings);
            for tab in 0..TABS.len() {
                ui.tab = tab;
                let scene = build(&ui, size, false, false, "", false, 70);
                for btn in &scene.buttons {
                    assert!(btn.rect.w > 0, "Button width must be positive");
                    assert!(btn.rect.h > 0, "Button height must be positive");
                    assert!(
                        btn.rect.x + btn.rect.w <= size.0 + 8,
                        "Button overflows viewport: btn.rect.x ({}) + w ({}) > viewport w ({}) in tab {}",
                        btn.rect.x, btn.rect.w, size.0, tab
                    );
                }
            }
        }
    }
    #[test]
    fn adaptive_colors_restore_saved_palette_and_light_themes_are_light() {
        let mut ui = Ui::default();
        ui.dark_colors.insert("Accent".into(), 0x123456);
        ui.adaptive_seed = Some(0x2040d0);
        ui.adaptive_colors = true;
        assert_eq!(ui.color("Accent", 0), 0x123456);
        assert_eq!(ui.solid_background(), ui.solid_color);
        ui.adaptive_colors = false;
        assert_eq!(ui.color("Accent", 0), 0x123456);
        for index in 12..SOLID_THEMES.len() {
            ui.apply_solid_theme(index);
            assert!(ui.light);
            for shift in [0, 8, 16] {
                assert!(((ui.solid_background() >> shift) & 255) > 220);
            }
        }
    }
    #[test]
    fn solid_themes_and_corner_styles() {
        let mut ui = Ui::default();
        ui.apply_solid_theme(3);
        assert_eq!(ui.solid_theme, 3);
        assert_eq!(ui.solid_color, 0x00463430);
        ui.apply_solid_theme(8);
        assert_eq!(ui.solid_theme, 8);
        assert_eq!(ui.solid_color, 0x00261b1a);
        assert_eq!(ui.color("Accent", 0), 0x00f7a27a);
        ui.corner_style = 0;
        assert_eq!(ui.corner_px(16), 0);
        ui.corner_style = 2;
        assert_eq!(ui.corner_px(16), 16);
    }
    #[test]
    fn context_aware_skeletons_and_clean_player_bar() {
        let mut ui = Ui::default();
        ui.dpi_scale = 1.0;

        // 1. Artist loading skeleton
        ui.go(View::Artist);
        let mut loading_page = crate::catalog::Page {
            title: "Artist".into(),
            entity: Some(crate::catalog::Entry {
                id: "1".into(),
                kind: "artist".into(),
                title: "Artist".into(),
                ..Default::default()
            }),
            loading: true,
            ..Default::default()
        };
        ui.page = Some(std::sync::Arc::new(loading_page.clone()));
        let s_artist = build(&ui, (1200, 800), true, false, "", false, 70);
        assert!(!s_artist.labels.iter().any(|l| l.text.contains("Loading")));
        assert!(s_artist.panels.iter().any(|p| p.kind == 15)); // Circular avatar skeleton
        assert!(s_artist.panels.iter().any(|p| p.kind == 14)); // Rectangular skeleton

        // 2. Album loading skeleton
        ui.go(View::Album);
        loading_page.entity = Some(crate::catalog::Entry {
            id: "2".into(),
            kind: "album".into(),
            title: "Album".into(),
            ..Default::default()
        });
        ui.page = Some(std::sync::Arc::new(loading_page.clone()));
        let s_album = build(&ui, (1200, 800), true, false, "", false, 70);
        assert!(!s_album.labels.iter().any(|l| l.text.contains("Loading")));
        assert!(s_album.panels.iter().any(|p| p.kind == 14));

        // 3. Favorites loading skeleton
        ui.go(View::Favorites);
        loading_page.entity = None;
        ui.page = Some(std::sync::Arc::new(loading_page.clone()));
        let s_fav = build(&ui, (1200, 800), true, false, "", false, 70);
        assert!(!s_fav.labels.iter().any(|l| l.text.contains("Loading")));
        assert!(s_fav.panels.iter().filter(|p| p.kind == 14).count() >= 10);

        // 4. Player bar cover art and status removal
        ui.now.artwork = "https://example.com/cover.jpg".into();
        ui.now.subtitle = "Artist Name".into();
        ui.quality = "FLAC · 16-bit 44.1 kHz".into();
        let s_player = build(&ui, (1200, 800), true, false, "Song Title", true, 70);
        let art = s_player
            .pictures
            .iter()
            .find(|(_, u)| u.contains("cover.jpg"))
            .unwrap();
        assert_eq!(art.0.w, 52);
        assert_eq!(art.0.h, 52);
        // Verify no labels placed directly under cover art (x == art.0.x)
        assert!(!s_player
            .labels
            .iter()
            .any(|l| l.rect.x == art.0.x && l.rect.y > art.0.y));
        // Verify title starts to the right of cover art with 14px padding
        let title_lbl = s_player
            .labels
            .iter()
            .find(|l| l.text == "Song Title")
            .unwrap();
        assert!(title_lbl.rect.x > art.0.x + art.0.w);
        assert!(title_lbl.rect.x + title_lbl.rect.w <= s_player.volume.unwrap().x);

        // 5. Track shelf card vs table row
        let shelf_page = crate::catalog::Page {
            sections: vec![crate::catalog::Section {
                title: "Önerilen þarkýlar".into(),
                entries: vec![crate::catalog::Entry {
                    id: "track1".into(),
                    title: "Deeper Love".into(),
                    subtitle: "Jauz".into(),
                    kind: "track".into(),
                    artwork: "https://example.com/jauz.jpg".into(),
                    ..Default::default()
                }],
                ..Default::default()
            }],
            ..Default::default()
        };
        ui.go(View::Home);
        ui.page = Some(std::sync::Arc::new(shelf_page));
        let s_shelf = build(&ui, (1200, 800), true, false, "", false, 70);
        let track_card = s_shelf
            .buttons
            .iter()
            .find(|b| matches!(&b.action, Action::OpenItem(item) if item.id == "track1"))
            .unwrap();
        assert!(
            !track_card.artwork.is_empty(),
            "Shelf card must retain artwork"
        );
        assert_eq!(
            track_card.rect.w, track_card.rect.h,
            "Shelf card must be square"
        );
        assert!(
            track_card.rect.h > 50,
            "Shelf card must be card-sized (> 50px)"
        );
        let card_title = s_shelf
            .buttons
            .iter()
            .find(|l| l.text == "Deeper Love")
            .unwrap();
        assert!(card_title.text_link && card_title.text_centered);
        let card_center = track_card.rect.x + track_card.rect.w / 2;
        let title_center = card_title.rect.x + card_title.rect.w / 2;
        assert_eq!(
            card_center, title_center,
            "Card title must be horizontally centered with the card"
        );
    }
    #[test]
    fn table_columns_do_not_overlap_with_queue_sidebar_open() {
        assert_eq!(format_date("1785401594"), "2026-07-30");
        let tracks = vec![crate::catalog::Entry {
            id: "1".into(),
            title: "Dark Knight Dummo (feat. Travis Scott)".into(),
            subtitle: "Trippie Redd".into(),
            album: "LIFE'S A TRIP".into(),
            kind: "track".into(),
            duration: 256000,
            raw: serde_json::json!({
                "addedAt": "1785401594"
            }),
            ..Default::default()
        }];
        let page = crate::catalog::Page {
            title: "Favorites".into(),
            rows: tracks,
            ..Default::default()
        };

        for sidebar_open in [false, true] {
            let mut ui = Ui::default();
            ui.view = View::Favorites;
            ui.queue_sidebar_open = sidebar_open;
            ui.page = Some(std::sync::Arc::new(page.clone()));

            let scene = build(&ui, (1000, 700), true, false, "", false, 50);
            let play_btns: Vec<_> = scene
                .buttons
                .iter()
                .filter(|b| matches!(b.action, Action::OpenItem(_)))
                .collect();
            assert!(!play_btns.is_empty());
            for b in play_btns {
                assert!(b.rect.w > 0);
                assert!(b.rect.x + b.rect.w <= 1000);
            }
        }
    }
    #[test]
    fn clicking_table_header_sorts_and_cycles_without_separate_button() {
        let tracks = vec![
            crate::catalog::Entry {
                id: "1".into(),
                title: "Bravo".into(),
                subtitle: "Zulu".into(),
                kind: "track".into(),
                duration: 200,
                ..Default::default()
            },
            crate::catalog::Entry {
                id: "2".into(),
                title: "Alpha".into(),
                subtitle: "Yankee".into(),
                kind: "track".into(),
                duration: 100,
                ..Default::default()
            },
        ];
        let page = crate::catalog::Page {
            title: "Favorites".into(),
            rows: tracks,
            ..Default::default()
        };

        let mut ui = Ui::default();
        ui.view = View::Favorites;
        ui.page = Some(std::sync::Arc::new(page));

        // 1. Verify no separate sort button in Favorites view
        let s0 = build(&ui, (1400, 700), true, false, "", false, 50);
        assert!(!s0.buttons.iter().any(|b| b.text.contains("Sort   ")));

        // 2. Verify column header buttons exist
        let sort_btns: Vec<_> = s0
            .buttons
            .iter()
            .filter(|b| matches!(b.action, Action::Sort(_)))
            .collect();
        assert_eq!(sort_btns.len(), 5); // Track, Artist, Album, Date, Duration

        // Initial default: no sort arrow
        let track_btn = sort_btns
            .iter()
            .find(|b| matches!(b.action, Action::Sort(1)))
            .unwrap();
        assert!(!track_btn.text.contains('\u{2191}') && !track_btn.text.contains('\u{2193}'));

        // 3. Sort ascending
        ui.sort_column = 1;
        ui.sort_descending = false;
        let s_asc = build(&ui, (1100, 700), true, false, "", false, 50);
        let track_asc = s_asc
            .buttons
            .iter()
            .find(|b| matches!(b.action, Action::Sort(1)))
            .unwrap();
        assert!(track_asc.text.contains('\u{2191}'));
        let track_names: Vec<_> = s_asc
            .buttons
            .iter()
            .filter_map(|b| match &b.action {
                Action::OpenItem(item) if item.playable() => Some(item.title.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(track_names, vec!["Alpha", "Bravo"]);

        // 4. Sort descending
        ui.sort_descending = true;
        let s_desc = build(&ui, (1100, 700), true, false, "", false, 50);
        let track_desc = s_desc
            .buttons
            .iter()
            .find(|b| matches!(b.action, Action::Sort(1)))
            .unwrap();
        assert!(track_desc.text.contains('\u{2193}'));
        let track_names_desc: Vec<_> = s_desc
            .buttons
            .iter()
            .filter_map(|b| match &b.action {
                Action::OpenItem(item) if item.playable() => Some(item.title.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(track_names_desc, vec!["Bravo", "Alpha"]);
    }
    #[test]
    fn flow_and_recent_views_do_not_render_batch_play_or_download_buttons() {
        let rows = vec![crate::catalog::Entry {
            id: "101".into(),
            title: "Flow Track 1".into(),
            kind: "track".into(),
            ..Default::default()
        }];
        let page = crate::catalog::Page {
            title: "Flow".into(),
            rows,
            ..Default::default()
        };

        // 1. Flow view
        let mut ui = Ui::default();
        ui.view = View::Flow;
        ui.page = Some(std::sync::Arc::new(page.clone()));
        let s_flow = build(&ui, (1100, 700), true, false, "", false, 50);
        // Verify no batch "Download" or duplicate "Play all" under page title
        assert!(!s_flow
            .buttons
            .iter()
            .any(|b| b.text == "Download" || b.accessible == "Download"));

        // 2. Recent view
        ui.view = View::Recent;
        let s_recent = build(&ui, (1100, 700), true, false, "", false, 50);
        assert!(!s_recent
            .buttons
            .iter()
            .any(|b| b.text == "Play all" || b.text == "Download"));
    }
    #[test]
    fn track_info_play_button_has_glyph_and_border_style() {
        let mut ui = Ui::default();
        ui.view = View::TrackInfo;
        ui.track_info = Some((
            crate::catalog::Entry {
                id: "1000cuts".into(),
                title: "1000 Cuts".into(),
                subtitle: "Kuuro".into(),
                album: "1000 Cuts EP".into(),
                album_id: "ep1".into(),
                kind: "track".into(),
                duration: 195000,
                ..Default::default()
            },
            serde_json::Value::Null,
            serde_json::Value::Null,
        ));
        let scene = build(&ui, (1200, 700), true, false, "", false, 50);
        let play_btn = scene
            .buttons
            .iter()
            .find(|b| b.text == "Play track")
            .unwrap();
        assert_eq!(play_btn.glyph, "\u{e768}");
        assert!(play_btn.selected);
        let album_btn = scene
            .buttons
            .iter()
            .find(|b| b.text == "View album")
            .unwrap();
        assert_eq!(album_btn.glyph, "\u{e93c}");
        assert!(play_btn.rect.x + play_btn.rect.w <= album_btn.rect.x);
    }
    #[test]
    fn general_settings_has_scrolling_options() {
        let mut ui = Ui::default();
        ui.view = View::Settings;
        ui.tab = 0;
        let scene = build(&ui, (1200, 800), true, false, "", false, 50);
        assert!(scene.labels.iter().any(|l| l.text == "SCROLLING"));
        assert!(scene
            .buttons
            .iter()
            .any(|b| matches!(b.action, Action::ToggleBool("smooth_scrolling"))));
        assert!(scene
            .buttons
            .iter()
            .any(|b| matches!(b.action, Action::ToggleBool("scroll_animation"))));
    }
    #[test]
    fn lyrics_sources_settings_reflects_supported_providers_and_disables_unavailable() {
        let mut ui = Ui::default();
        ui.view = View::Settings;
        ui.tab = 3;
        ui.lyrics_sources = crate::lyrics::SUPPORTED_SOURCES;
        let scene = build(&ui, (1200, 800), true, false, "", false, 50);

        let header = scene
            .labels
            .iter()
            .find(|l| l.text.starts_with("SOURCES ("))
            .unwrap();
        assert_eq!(
            header.text,
            format!(
                "SOURCES ({} selected)",
                crate::lyrics::SUPPORTED_SOURCES.count_ones()
            )
        );

        for (idx, source) in crate::lyrics::LYRIC_SOURCES.iter().enumerate() {
            let is_supported = (crate::lyrics::SUPPORTED_SOURCES & (1 << idx)) != 0;
            let btn = scene
                .buttons
                .iter()
                .find(|b| matches!(b.action, Action::ToggleLyricSource(i) if i == idx))
                .unwrap();
            assert_eq!(btn.enabled, is_supported);
            if is_supported {
                assert!(btn.selected);
                assert!(scene.labels.iter().any(|l| l.text == *source));
            } else {
                assert!(!btn.selected);
                assert!(scene
                    .labels
                    .iter()
                    .any(|l| l.text.contains(source) && l.text.contains("Unavailable")));
            }
        }
    }
    #[test]
    fn untimed_lyrics_render_static_unblurred_without_seek_buttons() {
        let mut ui = Ui::default();
        ui.view = View::Lyrics;
        ui.rich_lyrics = vec![
            crate::lyrics::LyricLine {
                text: "Line one without timing".into(),
                ..Default::default()
            },
            crate::lyrics::LyricLine {
                text: "Line two without timing".into(),
                ..Default::default()
            },
        ];
        ui.lyrics_provider = "LRCLIB".into();
        let scene = build(&ui, (1200, 800), true, false, "Test Track", true, 50);

        assert!(!scene
            .buttons
            .iter()
            .any(|b| matches!(b.action, Action::SeekMs(_))));
        assert!(scene.labels.iter().any(|l| l.text == "Lyrics: LRCLIB"));
        assert!(scene
            .labels
            .iter()
            .any(|l| l.text == "Line one without timing"));
        assert!(scene
            .labels
            .iter()
            .any(|l| l.text == "Line two without timing"));
    }
    #[test]
    fn lyrics_line_labels_have_adequate_height_to_prevent_text_clipping() {
        let mut ui = Ui::default();
        ui.view = View::Lyrics;
        ui.rich_lyrics = vec![crate::lyrics::LyricLine {
            start_ms: 1000,
            end_ms: 2000,
            text: "Isn't the view spectacular?".into(),
            ..Default::default()
        }];
        ui.position = 1500;
        let scene = build(&ui, (1200, 800), true, false, "Test Track", true, 50);

        let active_label = scene
            .labels
            .iter()
            .find(|l| l.text == "Isn't the view spectacular?")
            .unwrap();
        assert_eq!(active_label.kind, 13);
        assert!(
            active_label.rect.h >= 42,
            "Active label height {} must be >= 42 to prevent text clipping",
            active_label.rect.h
        );
    }
    #[test]
    fn preset_selection_updates_eq_table_and_sliders() {
        let mut ui = Ui::default();
        // Initially flat
        assert_eq!(ui.eq, [0; 11]);

        // Select Bass boost
        assert!(ui.apply_preset("Bass boost"));
        assert_eq!(
            ui.value("Preset"),
            options("Preset")
                .iter()
                .position(|&p| p == "Bass boost")
                .unwrap()
        );
        assert_eq!(ui.eq[0], 6);
        assert_eq!(ui.eq[1], 5);
        assert_eq!(ui.eq[2], 4);
        assert_eq!(ui.value("Equalizer"), 1); // automatically enables EQ

        // Select Rock
        assert!(ui.apply_preset("Rock"));
        assert_eq!(
            ui.value("Preset"),
            options("Preset").iter().position(|&p| p == "Rock").unwrap()
        );
        assert_eq!(ui.eq[0], 5);
        assert_eq!(ui.eq[1], 3);

        // Select Flat
        assert!(ui.apply_preset("Flat"));
        assert_eq!(
            ui.value("Preset"),
            options("Preset").iter().position(|&p| p == "Flat").unwrap()
        );
        assert_eq!(ui.eq[0..10], [0; 10]);
    }
    #[test]
    fn adjusting_any_eq_band_automatically_selects_custom_preset() {
        let mut ui = Ui::default();
        ui.apply_preset("Rock");
        let rock_idx = options("Preset").iter().position(|&p| p == "Rock").unwrap();
        assert_eq!(ui.value("Preset"), rock_idx);

        let custom_idx = ui.custom_preset_index();
        assert_eq!(options("Preset")[custom_idx], "Custom");
        assert_ne!(rock_idx, custom_idx);

        // User adjusts band 3 from 0 to 5 dB
        ui.adjust_eq(3, 5);
        assert_eq!(ui.eq[3], 5);
        // Automatically selects Custom for both persistence and dropdown
        assert_eq!(ui.value("Preset"), custom_idx);

        // User adjusts preamp (band 10)
        ui.apply_preset("Pop");
        assert_eq!(
            ui.value("Preset"),
            options("Preset").iter().position(|&p| p == "Pop").unwrap()
        );
        ui.adjust_eq(10, 3);
        assert_eq!(ui.eq[10], 3);
        assert_eq!(ui.value("Preset"), custom_idx);
    }
    #[test]
    fn lyrics_size_panel_and_fullscreen_scale_layout_and_font_bounds() {
        let mut ui = Ui::default();
        ui.view = View::Lyrics;
        ui.now.title = "Instinct (intro)".into();
        ui.rich_lyrics = vec![crate::lyrics::LyricLine {
            text: "Écouter son instinct".into(),
            start_ms: 1000,
            end_ms: 3000,
            ..Default::default()
        }];

        // Default 100% panel scale
        ui.lyrics_size_panel = 100;
        let s100 = build(&ui, (1200, 800), true, false, "Instinct (intro)", true, 50);
        let lbl100 = s100
            .labels
            .iter()
            .find(|l| l.text == "Écouter son instinct")
            .unwrap();
        assert_eq!(lbl100.rect.h, 44);

        // 120% panel scale (matches user image 1st row)
        ui.lyrics_size_panel = 120;
        let s120 = build(&ui, (1200, 800), true, false, "Instinct (intro)", true, 50);
        let lbl120 = s120
            .labels
            .iter()
            .find(|l| l.text == "Écouter son instinct")
            .unwrap();
        assert_eq!(lbl120.rect.h, 52);

        // 180% fullscreen scale (matches user image 2nd row)
        ui.lyrics_fullscreen = true;
        ui.lyrics_size_fullscreen = 180;
        let s180 = build(&ui, (1200, 800), true, false, "Instinct (intro)", true, 50);
        let lbl180 = s180
            .labels
            .iter()
            .find(|l| l.text == "Écouter son instinct")
            .unwrap();
        assert_eq!(lbl180.rect.h, 79);
    }
    #[test]
    fn lyrics_view_original_layout_and_hover_controls() {
        let mut ui = Ui::default();
        ui.view = View::Lyrics;
        ui.now.title = "Instinct (intro)".into();
        ui.now.subtitle = "Angèle".into();
        ui.lyrics_cover_hovered = false;

        // When unhovered: no controls on cover, seek bar below cover
        let s_unhovered = build(&ui, (1200, 800), true, false, "Instinct (intro)", true, 50);
        let cover = s_unhovered.lyrics_cover.unwrap();
        let seek = s_unhovered.seek.unwrap();
        assert!(seek.y > cover.y + cover.h);
        assert!(!s_unhovered
            .buttons
            .iter()
            .any(|b| matches!(b.action, Action::Toggle)));
        assert!(!s_unhovered
            .buttons
            .iter()
            .any(|b| matches!(b.action, Action::Shuffle)));

        // When hovered: 5 controls on cover
        ui.lyrics_cover_hovered = true;
        let s_hovered = build(&ui, (1200, 800), true, false, "Instinct (intro)", true, 50);
        let play = s_hovered
            .buttons
            .iter()
            .find(|b| matches!(b.action, Action::Toggle))
            .unwrap();
        let prev = s_hovered
            .buttons
            .iter()
            .find(|b| matches!(b.action, Action::Previous))
            .unwrap();
        let next = s_hovered
            .buttons
            .iter()
            .find(|b| matches!(b.action, Action::Next))
            .unwrap();
        let shuf = s_hovered
            .buttons
            .iter()
            .find(|b| matches!(b.action, Action::Shuffle))
            .unwrap();
        let rep = s_hovered
            .buttons
            .iter()
            .find(|b| matches!(b.action, Action::Repeat))
            .unwrap();

        // All 5 controls are inside cover bounds
        for b in [play, prev, next, shuf, rep] {
            assert!(b.rect.x >= cover.x && b.rect.x + b.rect.w <= cover.x + cover.w);
            assert!(b.rect.y >= cover.y && b.rect.y + b.rect.h <= cover.y + cover.h);
        }
    }
    #[test]
    fn shelf_pagination_loops_infinitely_in_both_directions() {
        let mut ui = Ui::default();
        let section = "Top Albums";
        let count = 10;
        let cols = 4;

        assert_eq!(ui.step_shelf(section, cols, count), 4);
        assert_eq!(ui.step_shelf(section, cols, count), 8);
        assert_eq!(ui.step_shelf(section, cols, count), 2);
        assert_eq!(ui.step_shelf(section, -cols, count), 8);
        ui.shelf_offsets.insert(section.to_string(), 0);
        assert_eq!(ui.step_shelf(section, -cols, count), 6);
        assert_eq!(ui.step_shelf(section, cols, count), 0);
        assert_eq!(ui.step_shelf(section, -cols, 12), 8);
        assert_eq!(ui.step_shelf(section, cols, 12), 0);
        let ui = Ui {
            view: View::Home,
            shelf_offsets: [(section.to_string(), 8)].into(),
            page: Some(std::sync::Arc::new(crate::catalog::Page {
                sections: vec![crate::catalog::Section {
                    title: section.into(),
                    entries: (0..10)
                        .map(|i| crate::catalog::Entry {
                            id: i.to_string(),
                            title: i.to_string(),
                            kind: "album".into(),
                            ..Default::default()
                        })
                        .collect(),
                    ..Default::default()
                }],
                ..Default::default()
            })),
            ..Default::default()
        };
        let scene = build(&ui, (1200, 800), true, false, "", false, 50);
        let cards: Vec<_> = scene
            .buttons
            .iter()
            .filter(|b| !b.text_link)
            .filter_map(|b| match &b.action {
                Action::OpenItem(e) => Some(e.id.as_str()),
                _ => None,
            })
            .collect();
        assert!(cards.starts_with(&["8", "9", "0", "1"]));
    }

    #[test]
    fn queue_sidebar_and_settings_fixes() {
        // 1. Tab icon for lyrics is \u{ec71} (vocal mic)
        assert_eq!(TAB_ICONS[3], "\u{ec71}");

        // 2. Settings view does not have the old Background button
        let mut ui = Ui::default();
        ui.view = View::Settings;
        let scene = build(&ui, (1200, 800), true, false, "", false, 50);
        assert!(!scene.buttons.iter().any(|b| b.text == "Background"));

        // 3. Queue sidebar panel does not overlap with main content panel
        ui.queue_sidebar_open = true;
        let scene_q = build(&ui, (1200, 800), true, false, "", false, 50);
        let q_header = scene_q.labels.iter().find(|l| l.text == "QUEUE").unwrap();
        assert_eq!(q_header.kind, 0);
        let q_icon = scene_q
            .buttons
            .iter()
            .find(|b| b.accessible == "Close queue")
            .unwrap();
        assert_eq!(q_icon.glyph, "\u{e8fd}");
        assert!(matches!(q_icon.action, Action::ToggleQueueSidebar));

        // Check panel boundaries
        let main_p = scene_q.panels[0].rect;
        let q_panel = scene_q
            .panels
            .iter()
            .find(|p| p.rect.x > main_p.x)
            .unwrap()
            .rect;
        assert!(
            main_p.x + main_p.w < q_panel.x,
            "Main panel right edge {} must not exceed queue panel left {}",
            main_p.x + main_p.w,
            q_panel.x
        );
    }

    #[test]
    fn podcast_library_playlists_menu_and_header_named_kitaplik() {
        let mut ui = Ui::default();
        assert_eq!(View::Playlists.title(), "Library");

        ui.view = View::Playlists;
        let s = build(&ui, (1200, 800), false, false, "", false, 50);

        // Sidebar nav button is named KitaplÄ±k
        let nav_btn = s
            .buttons
            .iter()
            .find(|b| matches!(b.action, Action::Go(View::Playlists)))
            .unwrap();
        assert_eq!(nav_btn.text, "Library");

        // Header title is KitaplÄ±k
        assert!(s.labels.iter().any(|l| l.text == "Library"));

        // Playlists page sections show playlists and saved podcasts
        let page = crate::catalog::Page {
            title: "Kitaplık".into(),
            sections: vec![
                crate::catalog::Section {
                    title: "Kitaplık".into(),
                    entries: vec![crate::catalog::Entry {
                        id: "p1".into(),
                        title: "My Playlist".into(),
                        kind: "playlist".into(),
                        ..Default::default()
                    }],
                    ..Default::default()
                },
                crate::catalog::Section {
                    title: "Saved podcasts".into(),
                    entries: vec![crate::catalog::Entry {
                        id: "pod1".into(),
                        title: "Casefile".into(),
                        kind: "podcast".into(),
                        ..Default::default()
                    }],
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        ui.page = Some(std::sync::Arc::new(page));
        let s_page = build(&ui, (1200, 800), false, false, "", false, 50);

        // Section titles: Playlists and Saved podcasts
        assert!(s_page
            .labels
            .iter()
            .any(|l| l.text == "Playlists" || l.text == "Çalma listeleri"));
        assert!(s_page
            .labels
            .iter()
            .any(|l| l.text == "Saved podcasts" || l.text == "Kaydedilen podcastler"));

        // Podcast card in Saved podcasts section is rendered and clickable
        assert!(s_page
            .buttons
            .iter()
            .any(|b| b.accessible == "Open Casefile"));

        // Entity header on Show (podcast) page uses library_key to show "In library" or "Add to library"
        let mut show_ui = Ui::default();
        show_ui.view = View::Show;
        let podcast_entry = crate::catalog::Entry {
            id: "pod1".into(),
            title: "Casefile".into(),
            kind: "podcast".into(),
            ..Default::default()
        };
        let show_page = crate::catalog::Page {
            title: "Casefile".into(),
            entity: Some(podcast_entry.clone()),
            ..Default::default()
        };
        show_ui.page = Some(std::sync::Arc::new(show_page));

        // When not saved
        let s_unliked = build(&show_ui, (1200, 800), false, false, "", false, 50);
        let fav_btn_unliked = s_unliked
            .buttons
            .iter()
            .find(|b| matches!(b.action, Action::Favorite))
            .unwrap();
        assert_eq!(fav_btn_unliked.text, "Add to library");

        // When saved with podcast namespace key
        show_ui.liked_ids.insert(podcast_entry.library_key());
        let s_liked = build(&show_ui, (1200, 800), false, false, "", false, 50);
        let fav_btn_liked = s_liked
            .buttons
            .iter()
            .find(|b| matches!(b.action, Action::Favorite))
            .unwrap();
        assert_eq!(fav_btn_liked.text, "In library");
    }
    #[test]
    fn reference_player_controls_and_metadata_never_collide_at_responsive_widths() {
        fn overlap(a: Rect, b: Rect) -> bool {
            a.x < b.x+b.w && a.x+a.w > b.x && a.y < b.y+b.h && a.y+a.h > b.y
        }
        for width in [480, 588, 600, 900, 1099, 1100, 1179, 1180, 1200, 1491, 1500, 2200] {
            for layout in [0, 1] {
                let mut ui = Ui::default();
                ui.player_layout = layout;
                ui.now = crate::catalog::Entry { id:"song".into(), kind:"track".into(),
                    title:"A long title with an explicit marker and a favorite control".into(),
                    subtitle:"An artist name".into(), artwork:"fixture".into(),
                    raw:serde_json::json!({"explicit":true}), ..Default::default() };
                let mut scene = Scene::default();
                build_player(&mut scene, &ui, width, 600, &ui.now.title, true, 70);
                let rects: Vec<_> = scene.buttons.iter().map(|b| b.rect)
                    .chain(scene.seek).chain(scene.volume).chain(scene.elapsed).chain(scene.remaining)
                    .chain(scene.player_title.as_ref().into_iter().flat_map(|t| {
                        std::iter::once(scene.labels[t.label].rect).chain(t.badge.map(|i| scene.labels[i].rect))
                    })).collect();
                for (index, rect) in rects.iter().enumerate() {
                    assert!(rect.x >= 0 && rect.x+rect.w <= width && rect.y >= 600-player_height(&ui,width) && rect.y+rect.h <= 600, "{width}/{layout}: {rect:?}");
                    for other in &rects[index+1..] {
                        assert!(!overlap(*rect,*other), "{width}/{layout}: {rect:?} overlaps {other:?}");
                    }
                }
                let meta = scene.player_title.as_ref().unwrap();
                assert_eq!(meta.centered, layout==0 && width>=1180);
                assert!(scene.buttons.iter().any(|b| matches!(b.action, Action::FavoriteNow)));
                assert!(scene.buttons.iter().any(|b| matches!(b.action, Action::MuteVolume)));
                assert!(scene.buttons.iter().any(|b| matches!(b.action, Action::PlayerFullscreen)));
            }
        }
    }

    #[test]
    fn wrapped_metadata_preserves_unicode_and_player_clocks_fit() {
        let title = "A very long podcast title with T?rk?e ??? andaverylongunbrokenword";
        let lines = wrap_text(title, 90, 14);
        assert!(lines.len() > 3);
        assert_eq!(lines.concat().replace(' ', ""), title.replace(' ', ""));
        assert_eq!(
            plain_description("<p>Hello &amp; welcome</p>"),
            "Hello & welcome"
        );
        for layout in [0, 1] {
            let ui = Ui {
                player_layout: layout,
                ..Default::default()
            };
            let scene = build(&ui, (1200, 800), true, false, "", false, 50);
            let seek = scene.seek.unwrap();
            let play = scene
                .buttons
                .iter()
                .find(|b| matches!(b.action, Action::Toggle))
                .unwrap();
            for clock in [scene.elapsed.unwrap(), scene.remaining.unwrap()] {
                assert!(clock.y >= seek.y && clock.y + clock.h <= seek.y + seek.h);
                assert!(clock.x + clock.w <= seek.x || clock.x >= seek.x + seek.w);
                if layout == 1 { assert!(clock.x >= play.rect.x + play.rect.w); }
            }
            assert!(seek.y + seek.h <= 800);
            assert!(!scene.labels.iter().any(|l| l.text == "DeezNats"));
        }
    }
    #[test]
    fn podcast_cards_keep_single_line_titles_and_episode_counts() {
        let ui = Ui {
            view: View::Podcasts,
            page: Some(std::sync::Arc::new(crate::catalog::Page {
                sections: vec![crate::catalog::Section {
                    title: "Podcasts".into(),
                    entries: vec![crate::catalog::Entry {
                        id: "pod".into(),
                        kind: "podcast".into(),
                        title: "A very long podcast title that must wrap across several lines"
                            .into(),
                        raw: serde_json::json!({"nb_episodes":42,"description":"A useful description for listeners"}),
                        ..Default::default()
                    }],
                    ..Default::default()
                }],
                ..Default::default()
            })),
            ..Default::default()
        };
        let scene = build(&ui, (1200, 800), true, false, "", false, 50);
        assert!(
            scene.labels.iter().any(|l| l.text.contains("42")),
            "episode count missing"
        );
        let titles: Vec<_> = scene
            .buttons
            .iter()
            .filter(|b| b.text_link && matches!(&b.action,Action::OpenItem(e) if e.id=="pod"))
            .collect();
        assert_eq!(titles.len(), 1);
        assert_eq!(titles[0].rect.h, 23);
    }
    #[test]
    fn appearance_options_expand_and_tint_slider_matches_background() {
        for mode in [0, 3] {
            let mut ui = Ui {
                view: View::Settings,
                tab: 2,
                background_mode: mode,
                ..Default::default()
            };
            let small = build(&ui, (1200, 900), true, false, "", false, 50);
            let option_count = |scene: &Scene| {
                scene
                    .buttons
                    .iter()
                    .filter(|b| matches!(b.action, Action::Preset(_) | Action::SolidTheme(_)))
                    .count()
            };
            let count = option_count(&small);
            assert!(count > 0);
            let expand = small
                .buttons
                .iter()
                .find(|b| {
                    matches!(
                        b.action,
                        Action::ToggleBool("expand_palettes" | "expand_themes")
                    )
                })
                .unwrap();
            assert_eq!(expand.text, ui.t("See more"));
            assert_eq!(
                small
                    .sliders
                    .iter()
                    .any(|(_, key, _)| *key == "Background tint opacity"),
                mode != 3
            );
            assert_eq!(
                small
                    .sliders
                    .iter()
                    .any(|(_, key, _)| *key == "Background brightness"),
                mode != 3
            );
            ui.expanded_palettes = true;
            ui.expanded_themes = true;
            let large = build(&ui, (1200, 900), true, false, "", false, 50);
            assert!(option_count(&large) > count);
            assert!(large.content_height > small.content_height);
            assert!(large.buttons.iter().any(|b| b.text == ui.t("See less")));
        }
    }
    #[test]
    fn lyrics_queue_and_volume_popup_fit_without_overlapping_lyrics() {
        for (width, height) in [(1200, 800), (900, 650), (746, 490)] {
            let mut ui = Ui {
                view: View::Lyrics,
                queue_sidebar_open: true,
                lyrics_volume_open: true,
                lyrics_cover_hovered: true,
                ..Default::default()
            };
            ui.now.artwork =
                "https://cdn-images.dzcdn.net/images/cover/id/250x250-000000-80-0-0.jpg".into();
            ui.now.id = "1".into();
            ui.queue = vec![ui.now.clone()];
            let scene = build(&ui, (width, height), true, false, "Track", true, 50);
            let cover = scene.lyrics_cover.unwrap();
            let area = scene.lyrics_area.unwrap();
            let popup = scene.volume_popup.unwrap();
            let volume = scene.volume.unwrap();
            let seek = scene.seek.unwrap();
            let queue = scene.labels.iter().find(|l| l.text == "QUEUE").unwrap();
            assert!(area.x + area.w < queue.rect.x);
            assert!(cover.x + cover.w < area.x);
            assert!(popup.y >= cover.y && popup.y + popup.h < seek.y);
            assert!(volume.h > volume.w && scene.volume_vertical);
            assert!(volume.y >= popup.y && volume.y + volume.h <= popup.y + popup.h);
            assert!(scene
                .pictures
                .iter()
                .any(|(_, url)| url.contains("1024x1024")));
            assert!(scene
                .buttons
                .iter()
                .any(|b| matches!(b.action, Action::QueueJump(0))));
            for button in scene.buttons.iter().filter(|b| {
                matches!(
                    b.action,
                    Action::Shuffle
                        | Action::Previous
                        | Action::Toggle
                        | Action::Next
                        | Action::Repeat
                )
            }) {
                assert!(
                    button.rect.x >= cover.x && button.rect.x + button.rect.w <= cover.x + cover.w
                );
            }
            ui.lyrics_volume_open = false;
            let closed = build(&ui, (width, height), true, false, "Track", true, 50);
            assert!(closed.volume.is_none() && closed.volume_popup.is_none());
        }
    }
    #[test]
    fn long_podcast_subtitles_do_not_expand_cards_or_push_shelves_down() {
        let make_ui = |text: &str| Ui {
            view: View::Podcasts,
            page: Some(std::sync::Arc::new(crate::catalog::Page {
                sections: vec![crate::catalog::Section {
                    title: "Podcasts".into(),
                    entries: vec![crate::catalog::Entry {
                        id: "pod".into(),
                        kind: "podcast".into(),
                        title: "MORD AUF EX".into(),
                        subtitle: text.into(),
                        raw: serde_json::json!({"description":text}),
                        ..Default::default()
                    }],
                    ..Default::default()
                }],
                ..Default::default()
            })),
            ..Default::default()
        };
        let short = build(
            &make_ui("Short subtitle"),
            (1200, 800),
            true,
            false,
            "",
            false,
            50,
        );
        let description =
            "Jeden Montag erz?hlen die Reporterinnen von einem wahren Kriminalfall. ".repeat(100);
        let long = build(
            &make_ui(&description),
            (1200, 800),
            true,
            false,
            "",
            false,
            50,
        );
        assert_eq!(short.content_height, long.content_height);
        let subtitles: Vec<_> = long
            .labels
            .iter()
            .filter(|l| l.text == description.trim())
            .collect();
        assert_eq!(subtitles.len(), 1);
        assert_eq!(subtitles[0].rect.h, 21);
        assert_eq!(short.labels.len(), long.labels.len());
        assert_eq!(short.buttons.len(), long.buttons.len());
    }
    #[test]
    fn podcast_description_defaults_to_three_lines_with_text_expansion() {
        let page = crate::catalog::Page {
            entity: Some(crate::catalog::Entry {
                id: "pod".into(),
                kind: "podcast".into(),
                raw: serde_json::json!({"description":"A detailed podcast description for curious listeners. ".repeat(12)}),
                ..Default::default()
            }),
            ..Default::default()
        };
        let render = |ui: &Ui, page: &crate::catalog::Page| {
            let mut scene = Scene::default();
            live_catalog(&mut scene, ui, page, 0, 0, 400, 2000);
            scene
        };
        let mut ui = Ui::default();
        let collapsed = render(&ui, &page);
        assert_eq!(
            collapsed.labels.iter().filter(|l| l.rect.h == 22).count(),
            3
        );
        assert!(collapsed
            .labels
            .iter()
            .any(|l| l.text.ends_with('\u{2026}')));
        let more = collapsed
            .buttons
            .iter()
            .find(|b| matches!(b.action, Action::TogglePodcastDescription(_)))
            .unwrap();
        assert!(more.text_link && more.text == "Read more");
        ui.expanded_podcast_description = Some("pod".into());
        let expanded = render(&ui, &page);
        assert!(expanded.labels.iter().filter(|l| l.rect.h == 22).count() > 3);
        assert!(expanded.content_height > collapsed.content_height);
        assert!(expanded
            .buttons
            .iter()
            .any(|b| b.text_link && b.text == "Read less"));
        ui.expanded_podcast_description = Some("different-show".into());
        assert_eq!(
            render(&ui, &page)
                .labels
                .iter()
                .filter(|l| l.rect.h == 22)
                .count(),
            3
        );
        let mut short = page.clone();
        short.entity.as_mut().unwrap().raw["description"] =
            serde_json::json!("A short description.");
        assert!(!render(&ui, &short)
            .buttons
            .iter()
            .any(|b| matches!(b.action, Action::TogglePodcastDescription(_))));
    }
    #[test]
    fn episode_tables_hide_artist_and_use_native_release_dates_for_display_and_sort() {
        let mut ui = Ui {
            view: View::Show,
            sort_column: 4,
            sort_descending: false,
            ..Default::default()
        };
        let mut page = crate::catalog::Page {
            entity: Some(crate::catalog::Entry {
                id: "show".into(),
                kind: "podcast".into(),
                ..Default::default()
            }),
            rows: vec![
                crate::catalog::Entry {
                    id: "new".into(),
                    title: "New episode".into(),
                    subtitle: "Podcast host".into(),
                    kind: "episode".into(),
                    raw: serde_json::json!({"releaseDate":"2026-07-11","DATE_ADD":"2001-01-01"}),
                    ..Default::default()
                },
                crate::catalog::Entry {
                    id: "old".into(),
                    title: "Old episode".into(),
                    subtitle: "Podcast host".into(),
                    kind: "episode".into(),
                    raw: serde_json::json!({"publicationDate":"2025-06-10T12:00:00Z"}),
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        let render = |ui: &Ui, page: &crate::catalog::Page| {
            let mut scene = Scene::default();
            live_catalog(&mut scene, ui, page, 0, 0, 1000, 1000);
            scene
        };
        let scene = render(&ui, &page);
        assert!(scene
            .buttons
            .iter()
            .any(|b| matches!(b.action, Action::Sort(1)) && b.text == "Episode"));
        let caption = entity_caption(page.entity.as_ref().unwrap(), Some(&page), "", &ui);
        assert!(caption.contains("2 episodes") && !caption.contains("tracks"));
        ui.set_language("tr");
        let localized = render(&ui, &page);
        assert!(localized
            .buttons
            .iter()
            .any(|b| matches!(b.action, Action::Sort(1)) && b.text == "B\u{f6}l\u{fc}m"));
        assert!(
            entity_caption(page.entity.as_ref().unwrap(), Some(&page), "", &ui)
                .contains("2 b\u{f6}l\u{fc}m")
        );
        ui.set_language("en");
        assert!(!scene
            .buttons
            .iter()
            .any(|b| matches!(b.action, Action::Sort(2))));
        assert!(!scene.labels.iter().any(|l| l.text == "Podcast host"));
        assert!(scene.labels.iter().any(|l| l.text == "2026-07-11"));
        assert!(scene.labels.iter().any(|l| l.text == "2025-06-10"));
        let titles: Vec<_> = scene
            .buttons
            .iter()
            .filter_map(|b| match &b.action {
                Action::OpenItem(e) => Some(e.id.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(titles, vec!["old", "new"]);
        ui.sort_descending = true;
        let reversed = render(&ui, &page);
        assert!(
            matches!(&reversed.buttons.iter().find(|b|matches!(b.action,Action::OpenItem(_))).unwrap().action,Action::OpenItem(e) if e.id=="new")
        );
        for key in [
            "releaseDate",
            "release_date",
            "publicationDate",
            "publishedAt",
            "RELEASE_DATE",
        ] {
            page.rows[0].raw = serde_json::json!({key:"2026-07-11"});
            assert_eq!(format_date(&row_date(&page.rows[0], true)), "2026-07-11");
        }
    }
    #[test]
    fn mix_pages_show_collection_cover_and_playable_track_rows() {
        let ui = Ui {
            view: View::Album,
            page: Some(std::sync::Arc::new(crate::catalog::Page {
                title: "daily".into(),
                entity: Some(crate::catalog::Entry {
                    id: "daily".into(),
                    title: "daily".into(),
                    kind: "mix".into(),
                    artwork: "fixture-mix-cover".into(),
                    ..Default::default()
                }),
                rows: vec![crate::catalog::Entry {
                    id: "song".into(),
                    title: "Actual song".into(),
                    kind: "track".into(),
                    duration: 180000,
                    ..Default::default()
                }],
                ..Default::default()
            })),
            ..Default::default()
        };
        let scene = build(&ui, (1200, 900), true, false, "", false, 50);
        assert!(scene
            .pictures
            .iter()
            .any(|(_, url)| url == "fixture-mix-cover"));
        assert!(scene
            .buttons
            .iter()
            .any(|b| matches!(b.action, Action::PlayAll)));
        assert!(scene
            .buttons
            .iter()
            .any(|b| matches!(&b.action,Action::OpenItem(e) if e.id=="song")));
        assert!(!scene
            .buttons
            .iter()
            .any(|b| matches!(&b.action,Action::OpenItem(e) if e.id=="daily")));
        assert!(!scene
            .buttons
            .iter()
            .any(|b| matches!(b.action, Action::Sort(4) | Action::Favorite)));
    }
    #[test]
    fn podcasts_navigation_is_in_library_and_offline_downloads_are_visible() {
        let episode = crate::catalog::Entry {
            id: "offline-ep".into(),
            title: "Downloaded episode".into(),
            kind: "episode".into(),
            ..Default::default()
        };
        let ui = Ui {
            view: View::Podcasts,
            page: Some(std::sync::Arc::new(crate::catalog::podcast_library_page(
                Vec::new(),
                Vec::new(),
                Vec::new(),
                vec![episode],
            ))),
            ..Default::default()
        };
        let scene = build(&ui, (1200, 900), false, false, "", false, 50);
        let library = scene.labels.iter().find(|l| l.text == "LIBRARY").unwrap();
        let podcasts = scene
            .buttons
            .iter()
            .find(|b| matches!(b.action, Action::Go(View::Podcasts)))
            .unwrap();
        let flow = scene
            .buttons
            .iter()
            .find(|b| matches!(b.action, Action::Go(View::Flow)))
            .unwrap();
        assert!(podcasts.rect.y > library.rect.y && flow.rect.y < library.rect.y);
        assert!(scene.search.is_none());
        assert!(scene
            .buttons
            .iter()
            .any(|b| matches!(&b.action,Action::OpenItem(e) if e.id=="offline-ep")));
        assert!(!scene.labels.iter().any(|l| matches!(
            l.text.as_str(),
            "Podcasts for you" | "Recommended episodes" | "Shows and episodes for you"
        )));
    }
    #[test]
    fn artist_credits_have_individual_nonoverlapping_targets() {
        let track = crate::catalog::entry(
            &serde_json::json!({"id":"song", "title":"Song",
            "artists":[{"id":"1","name":"First artist"},{"id":"2","name":"Second artist"}]}),
            "track",
        )
        .unwrap();
        let mut scene = Scene::default();
        scene.artist_links(Rect::new(10, 20, 200, 24), &track, true);
        assert_eq!(scene.buttons.len(), 2);
        assert_eq!(
            scene
                .buttons
                .iter()
                .map(|b| b.text.as_str())
                .collect::<String>(),
            "First artist, Second artist"
        );
        assert!(scene.buttons[0].rect.x + scene.buttons[0].rect.w <= scene.buttons[1].rect.x);
        for (button, id) in scene.buttons.iter().zip(["1", "2"]) {
            assert!(button.text_link);
            assert!(matches!(&button.action, Action::OpenItem(e) if e.id == id));
        }
        let ui = Ui {
            view: View::Lyrics,
            now: track,
            ..Default::default()
        };
        let scene = build(&ui, (1200, 900), false, false, "", false, 50);
        let cover = scene.lyrics_cover.unwrap();
        let add = scene
            .buttons
            .iter()
            .find(|b| b.accessible == "Add to favorites or playlist")
            .unwrap();
        assert!(matches!(add.action, Action::Go(View::Add)));
        assert!(
            add.rect.x >= cover.x
                && add.rect.y >= cover.y
                && add.rect.y + add.rect.h <= cover.y + cover.h
        );
    }

    #[test]
    fn artist_credit_line_centers_and_truncates_the_trailing_names() {
        let slot = Rect::new(10, 20, 200, 24);
        let short = artist_credit_rects(slot, &[42, 84], true);
        assert_eq!(
            short,
            vec![Rect::new(47, 20, 42, 24), Rect::new(89, 20, 84, 24)]
        );
        let long = artist_credit_rects(slot, &[70, 220], true);
        assert_eq!(long[0].w, 70);
        assert_eq!(long[1].w, 130);
        for width in [0, 1, 20, 80, 200] {
            let slots = artist_credit_rects(Rect::new(10, 0, width, 24), &[70, 220, 90], true);
            assert!(slots
                .iter()
                .all(|r| r.w >= 0 && r.x >= 10 && r.x + r.w <= 10 + width));
            assert!(slots
                .windows(2)
                .all(|pair| pair[0].x + pair[0].w <= pair[1].x));
        }
    }

    #[test]
    fn skeletons_match_actual_catalog_and_table_layout() {
        let mut ui = Ui::default();
        ui.dpi_scale = 1.0;

        // 1. Home shelf skeleton geometry matches real shelf cards
        ui.go(View::Home);
        ui.page = Some(std::sync::Arc::new(crate::catalog::Page {
            title: "Home".into(),
            loading: true,
            ..Default::default()
        }));
        let scene_home = build(&ui, (1200, 800), true, false, "", false, 70);
        let side = 214;
        let x = side + 48;
        let cw = 1200 - x - 40;
        let x_shelf = x - 14;
        let cw_shelf = cw + 14;
        let cols = (((cw_shelf - 12) as f64 * ui.dpi_scale) as i32 / 200).max(1);
        let cell = (cw_shelf - 12) / cols;
        let cover = ((160.0 / ui.dpi_scale).round() as i32)
            .min(cell - 16)
            .max(32);
        let label_w = (cell - 28).max(20);

        // Find cover panels in first shelf
        let first_slot_x = x_shelf + 6;
        let first_cover_x = first_slot_x + (cell - 8 - cover) / 2;
        let card_panels: Vec<_> = scene_home
            .panels
            .iter()
            .filter(|p| p.rect.w == cover && p.rect.h == cover && p.kind == 14)
            .collect();
        assert!(
            !card_panels.is_empty(),
            "Should have cover panels matching shelf cover size"
        );
        assert_eq!(card_panels[0].rect.x, first_cover_x);

        // Check title and subtitle label positions
        let first_label_x = first_cover_x + cover / 2 - label_w / 2;
        let title_panel = scene_home
            .panels
            .iter()
            .find(|p| p.rect.x == first_label_x && p.rect.y == card_panels[0].rect.y + cover + 12);
        assert!(
            title_panel.is_some(),
            "Title panel should be at y + cover + 12"
        );
        assert_eq!(title_panel.unwrap().rect.w, label_w);

        // 2. Album loading skeleton matches table columns and 56px row height
        ui.go(View::Album);
        ui.page = Some(std::sync::Arc::new(crate::catalog::Page {
            title: "Album".into(),
            entity: Some(crate::catalog::Entry {
                id: "album-1".into(),
                kind: "album".into(),
                title: "Album Title".into(),
                ..Default::default()
            }),
            loading: true,
            ..Default::default()
        }));
        let scene_album = build(&ui, (1200, 800), true, false, "", false, 70);
        // Track table starts at body_top (198 + 144 = 342)
        // Check for index panels (18x14) at 56px intervals
        let index_panels: Vec<_> = scene_album
            .panels
            .iter()
            .filter(|p| p.rect.w == 18 && p.rect.h == 14 && p.rect.x == x + 16)
            .collect();
        assert_eq!(
            index_panels.len(),
            9,
            "Should have 9 track index skeleton panels"
        );
        assert_eq!(
            index_panels[1].rect.y - index_panels[0].rect.y,
            56,
            "Row step must be exactly 56px"
        );

        // 3. Podcast Show page has description preview and episode table (no cards)
        ui.go(View::Show);
        ui.page = Some(std::sync::Arc::new(crate::catalog::Page {
            title: "Show".into(),
            entity: Some(crate::catalog::Entry {
                id: "show-1".into(),
                kind: "podcast".into(),
                title: "Podcast Title".into(),
                ..Default::default()
            }),
            loading: true,
            ..Default::default()
        }));
        let scene_show = build(&ui, (1200, 800), true, false, "", false, 70);
        // Episode table rows have 44x44 cover panels at x + 4
        let ep_covers: Vec<_> = scene_show
            .panels
            .iter()
            .filter(|p| p.rect.w == 44 && p.rect.h == 44 && p.rect.x == x + 4 && p.rect.y > 342)
            .collect();
        assert_eq!(ep_covers.len(), 9, "Should have 9 episode row covers");
        assert_eq!(
            ep_covers[1].rect.y - ep_covers[0].rect.y,
            56,
            "Episode row step must be 56px"
        );
        // Ensure no card shelf panels (160x160 or similar) exist below the header
        assert!(!scene_show
            .panels
            .iter()
            .any(|p| p.rect.y > 342 && p.rect.w == cover && p.rect.h == cover));
    }
    #[test]
    fn flow_search_has_glass_padding_and_mood_buttons_fit_localized_labels() {
        let mut ui = Ui {
            view: View::FlowTune,
            flow_options: vec![
                ("personal".into(), "Personal Flow".into()),
                ("focus".into(), "Focus".into()),
                ("chill".into(), "Chill".into()),
            ],
            ..Default::default()
        };
        ui.translations=std::sync::Arc::new(serde_json::from_value(serde_json::json!({"Personal Flow":"Ki?isel Flow","Focus":"Odaklanma","Chill":"Rahatlama"})).unwrap());
        let scene = build(&ui, (1100, 800), true, false, "", false, 50);
        let search = scene.search.unwrap();
        assert!(scene.panels.iter().any(|p| p.kind == 8
            && p.rect.x < search.x
            && p.rect.y < search.y
            && p.rect.x + p.rect.w > search.x + search.w
            && p.rect.y + p.rect.h > search.y + search.h));
        let choices: Vec<_> = scene
            .buttons
            .iter()
            .filter(|b| matches!(b.action, Action::FlowChoice(_)))
            .collect();
        assert_eq!(choices[0].text, "Ki?isel Flow");
        assert_eq!(choices[1].text, "Odaklanma");
        for choice in choices {
            assert!(choice.rect.w >= choice.text.chars().count() as i32 * 9 + 40);
        }
        ui.view = View::Settings;
        ui.tab = 0;
        let settings = build(&ui, (1100, 800), true, false, "", false, 50);
        assert!(settings
            .buttons
            .iter()
            .any(|b| matches!(b.action, Action::SetControlAlignment(false))));
        assert!(settings
            .buttons
            .iter()
            .any(|b| matches!(b.action, Action::SetControlAlignment(true))));
    }
    #[test]
    fn solid_themes_and_presets_meet_wcag_contrast_requirements() {
        let rel_lum = |color: u32| -> f64 {
            [0, 8, 16]
                .into_iter()
                .zip([0.2126, 0.7152, 0.0722])
                .map(|(shift, w)| {
                    let c = ((color >> shift) & 255) as f64 / 255.0;
                    w * if c <= 0.04045 {
                        c / 12.92
                    } else {
                        ((c + 0.055) / 1.055).powf(2.4)
                    }
                })
                .sum::<f64>()
        };
        let contrast = |a: u32, b: u32| -> f64 {
            let la = rel_lum(a);
            let lb = rel_lum(b);
            (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
        };

        for (idx, (name, is_light, _, _, _, _, _, _)) in SOLID_THEMES.iter().enumerate() {
            let mut ui = Ui::default();
            ui.apply_solid_theme(idx);
            assert_eq!(ui.light, *is_light);
            let text = ui.color("Text", 0);
            let bg = ui.color("Background", 0);
            let panel = ui.color("Panel", 0);
            let raised = ui.color("Raised", 0);
            let muted = ui.color("Muted", 0);
            let accent = ui.color("Accent", 0);
            let accent_text = ui.color("AccentText", 0);

            let cr_bg = contrast(text, bg);
            let cr_panel = contrast(text, panel);
            let cr_raised = contrast(text, raised);
            let cr_muted = contrast(muted, panel);
            let cr_accent = contrast(accent_text, accent);

            assert!(
                cr_bg >= 8.0,
                "Theme {name} Text:Bg contrast {cr_bg:.2} < 8.0"
            );
            assert!(
                cr_panel >= 8.0,
                "Theme {name} Text:Panel contrast {cr_panel:.2} < 8.0"
            );
            assert!(
                cr_raised >= 6.0,
                "Theme {name} Text:Raised contrast {cr_raised:.2} < 6.0"
            );
            assert!(
                cr_muted >= 4.5,
                "Theme {name} Muted:Panel contrast {cr_muted:.2} < 4.5 (WCAG AA)"
            );
            assert!(
                cr_accent >= 4.5,
                "Theme {name} AccentText:Accent contrast {cr_accent:.2} < 4.5 (WCAG AA)"
            );
        }
    }
}
