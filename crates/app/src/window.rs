use crate::{
    audio::{Command, Snapshot, Worker},
    ui::{self, Action, Rect, Scene, Ui, View},
};
use std::path::PathBuf;
use std::{
    collections::{HashMap, HashSet, VecDeque},
    ffi::c_void,
    mem::{size_of, zeroed},
    ptr::{null, null_mut},
    sync::Arc,
};
use windows_sys::Win32::UI::Shell::{
    DefSubclassProc, SetWindowSubclass, ShellExecuteW, Shell_NotifyIconW, NIF_ICON, NIF_MESSAGE,
    NIF_TIP, NIM_ADD, NIM_DELETE, NOTIFYICONDATAW,
};
use windows_sys::Win32::{
    Foundation::*,
    Graphics::{Dwm::*, Gdi::*},
    System::LibraryLoader::GetModuleHandleW,
    UI::{
        Controls::{Dialogs::*, *},
        HiDpi::*,
        Input::KeyboardAndMouse::{
            EnableWindow, GetCapture, GetFocus, IsWindowEnabled, ReleaseCapture, SetCapture,
            SetFocus, TrackMouseEvent, TME_LEAVE, TRACKMOUSEEVENT, VK_DOWN, VK_END, VK_HOME,
            VK_LEFT, VK_RIGHT, VK_UP,
        },
        WindowsAndMessaging::*,
    },
};
use crate::drawing::gpu;
mod compositor_clock;
mod artwork_damage;
mod canvas_input;
const SEARCH: i32 = 501;
const SESSION: i32 = 502;
const TRACK: i32 = 503;
const NAME: i32 = 504;
const DESCRIPTION: i32 = 505;
const SEEK: i32 = 506;
const VOLUME: i32 = 507;
const STATUS: i32 = 508;

const SEEK_STEPS: u32 = 10_000;
const COLLAPSE_BUTTON: i32 = 900;
const CAPTION_BUTTON_BASE: i32 = 901;
const TRAY_CALLBACK: u32 = WM_APP + 9;
const ART_FADE_MS: u128 = 560;
const BACKGROUND_FADE_OUT_MS: u64 = 440;
const BACKGROUND_FADE_IN_MS: u64 = 560;
const PALETTE_FADE_MS: u128 = 500;

mod frame_pacing;

// Window timers are generated only when the queue is otherwise idle, including
// inside Windows' native menu and move/resize loops. Posted animation messages
// can continuously outrank mouse and keyboard input in those nested loops.
thread_local! {
    static ANIMATION_TIMERS: std::cell::RefCell<std::collections::HashSet<usize>> =
        std::cell::RefCell::new(std::collections::HashSet::new());
}
static BACKGROUND_FRAME_INTERVAL_NS: std::sync::atomic::AtomicU64 =
    std::sync::atomic::AtomicU64::new(0);

unsafe fn start_animation_timer(hwnd: HWND, id: usize) {
    ANIMATION_TIMERS.with(|timers| {
        if timers.borrow_mut().insert(id) {
            let interval = BACKGROUND_FRAME_INTERVAL_NS.load(std::sync::atomic::Ordering::Relaxed);
            // Marquees move slowly; repainting them at 100 Hz needlessly
            // competes with the rest of the UI for render time.
            let minimum_ms = if id == 4 { 33 } else { 10 };
            let millis = ((interval + 999_999) / 1_000_000).max(minimum_ms) as u32;
            if unsafe { SetTimer(hwnd, id, millis, None) } == 0 {
                timers.borrow_mut().remove(&id);
            } else {
                frame_pacing::start(hwnd, id);
            }
        }
    });
}

unsafe fn stop_animation_timer(hwnd: HWND, id: usize) {
    frame_pacing::stop(hwnd, id);
    ANIMATION_TIMERS.with(|timers| {
        timers.borrow_mut().remove(&id);
    });
    unsafe {
        KillTimer(hwnd, id);
    }
}

unsafe fn refresh_animation_timers(hwnd: HWND) {
    let ids = ANIMATION_TIMERS.with(|timers| timers.borrow().iter().copied().collect::<Vec<_>>());
    for id in ids {
        unsafe {
            stop_animation_timer(hwnd, id);
            start_animation_timer(hwnd, id);
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
enum BackgroundTransition {
    #[default]
    Complete,
    FadeOut {
        started: std::time::Instant,
        ready_at: Option<std::time::Instant>,
    },
    AmbientHold,
    FadeIn {
        started: std::time::Instant,
    },
}

impl BackgroundTransition {
    fn texture_ready(&mut self, now: std::time::Instant) {
        match self {
            Self::FadeOut { ready_at, .. } => *ready_at = Some(now),
            Self::AmbientHold => *self = Self::FadeIn { started: now },
            _ => {}
        }
        self.advance(now);
    }

    fn advance(&mut self, now: std::time::Instant) {
        if let Self::FadeOut { started, ready_at } = *self {
            let end = started + std::time::Duration::from_millis(BACKGROUND_FADE_OUT_MS);
            if now >= end {
                *self = match ready_at {
                    Some(ready) => Self::FadeIn {
                        started: end.max(ready),
                    },
                    None => Self::AmbientHold,
                };
            }
        }
        if let Self::FadeIn { started } = *self {
            if now.saturating_duration_since(started).as_millis() >= BACKGROUND_FADE_IN_MS as u128 {
                *self = Self::Complete;
            }
        }
    }

    fn progress(mut self, now: std::time::Instant) -> f32 {
        self.advance(now);
        let ease = |started, duration: u64| {
            let t = (now.saturating_duration_since(started).as_secs_f32()
                / (duration as f32 / 1000.0))
                .clamp(0.0, 1.0);
            t * t * (3.0 - 2.0 * t)
        };
        match self {
            Self::FadeOut { started, .. } => 0.5 * ease(started, BACKGROUND_FADE_OUT_MS),
            Self::AmbientHold => 0.5,
            Self::FadeIn { started } => 0.5 + 0.5 * ease(started, BACKGROUND_FADE_IN_MS),
            Self::Complete => 1.0,
        }
    }

    fn animating(self) -> bool {
        matches!(self, Self::FadeOut { .. } | Self::FadeIn { .. })
    }
}

#[derive(Clone, Copy, Debug)]
struct PaletteState {
    target: [u32; 9],
    previous: [u32; 9],
    started: std::time::Instant,
}

impl Default for PaletteState {
    fn default() -> Self {
        Self {
            target: [0; 9],
            previous: [0; 9],
            started: std::time::Instant::now(),
        }
    }
}

fn rgb_to_hsl(r: f32, g: f32, b: f32) -> (f32, f32, f32) {
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let l = (max + min) / 2.0;
    if (max - min).abs() < 1e-4 {
        return (0.0, 0.0, l);
    }
    let d = max - min;
    let s = if l > 0.5 {
        d / (2.0 - max - min)
    } else {
        d / (max + min)
    };
    let mut h = if (max - r).abs() < 1e-4 {
        let mut segment = (g - b) / d;
        if g < b {
            segment += 6.0;
        }
        segment / 6.0
    } else if (max - g).abs() < 1e-4 {
        ((b - r) / d + 2.0) / 6.0
    } else {
        ((r - g) / d + 4.0) / 6.0
    };
    if h < 0.0 {
        h += 1.0;
    }
    if h >= 1.0 {
        h -= 1.0;
    }
    (h, s, l)
}

fn hsl_to_rgb(h: f32, s: f32, l: f32) -> (f32, f32, f32) {
    if s < 1e-4 {
        return (l, l, l);
    }
    let q = if l < 0.5 {
        l * (1.0 + s)
    } else {
        l + s - l * s
    };
    let p = 2.0 * l - q;
    let hue_to_rgb = |p: f32, q: f32, mut t: f32| {
        if t < 0.0 {
            t += 1.0;
        }
        if t > 1.0 {
            t -= 1.0;
        }
        if t < 1.0 / 6.0 {
            return p + (q - p) * 6.0 * t;
        }
        if t < 1.0 / 2.0 {
            return q;
        }
        if t < 2.0 / 3.0 {
            return p + (q - p) * (2.0 / 3.0 - t) * 6.0;
        }
        p
    };
    (
        hue_to_rgb(p, q, h + 1.0 / 3.0),
        hue_to_rgb(p, q, h),
        hue_to_rgb(p, q, h - 1.0 / 3.0),
    )
}

fn lerp_adaptive_color(c1: u32, c2: u32, t: f32) -> u32 {
    let t = t.clamp(0.0, 1.0);
    if (t - 0.0).abs() < 1e-4 {
        return c1;
    }
    if (t - 1.0).abs() < 1e-4 {
        return c2;
    }

    let r1 = (c1 & 255) as f32 / 255.0;
    let g1 = ((c1 >> 8) & 255) as f32 / 255.0;
    let b1 = ((c1 >> 16) & 255) as f32 / 255.0;

    let r2 = (c2 & 255) as f32 / 255.0;
    let g2 = ((c2 >> 8) & 255) as f32 / 255.0;
    let b2 = ((c2 >> 16) & 255) as f32 / 255.0;

    let (h1, s1, l1) = rgb_to_hsl(r1, g1, b1);
    let (h2, s2, l2) = rgb_to_hsl(r2, g2, b2);

    if s1 < 0.08 || s2 < 0.08 {
        let curve = t.powf(0.75);
        let r = (r1 + (r2 - r1) * curve).clamp(0.0, 1.0);
        let g = (g1 + (g2 - g1) * curve).clamp(0.0, 1.0);
        let b = (b1 + (b2 - b1) * curve).clamp(0.0, 1.0);
        return ((r * 255.0).round() as u32)
            | (((g * 255.0).round() as u32) << 8)
            | (((b * 255.0).round() as u32) << 16);
    }

    let mut dh = h2 - h1;
    if dh > 0.5 {
        dh -= 1.0;
    } else if dh < -0.5 {
        dh += 1.0;
    }

    let t_hue = t.powf(0.60);
    let mut h = h1 + dh * t_hue;
    if h < 0.0 {
        h += 1.0;
    }
    if h >= 1.0 {
        h -= 1.0;
    }

    let t_sat_lum = t.powf(0.80);
    let s = (s1 + (s2 - s1) * t_sat_lum).clamp(0.0, 1.0);
    let l = (l1 + (l2 - l1) * t_sat_lum).clamp(0.0, 1.0);

    let (r, g, b) = hsl_to_rgb(h, s, l);
    ((r * 255.0).round() as u32)
        | (((g * 255.0).round() as u32) << 8)
        | (((b * 255.0).round() as u32) << 16)
}

fn lerp_color(c1: u32, c2: u32, t: f32) -> u32 {
    lerp_adaptive_color(c1, c2, t)
}
thread_local! {static HOVERED_CONTROL:std::cell::Cell<HWND>=const {std::cell::Cell::new(null_mut())};}
thread_local! {static CAPTION_HOVER:std::cell::Cell<(HWND,usize)>=const {std::cell::Cell::new((null_mut(),0))};}
thread_local! {
    static MARQUEE: std::cell::RefCell<Option<(String, RECT, std::time::Instant, std::time::Instant)>> = const { std::cell::RefCell::new(None) };
}
fn marquee_offset(elapsed: f64, overflow: f64, speed: f64) -> f64 {
    let travel = overflow / speed;
    let phase = elapsed % (0.7 + travel + 1.2);
    ((phase - 0.7).max(0.0) * speed).min(overflow)
}
unsafe fn draw_hover_text(
    hwnd: HWND,
    dc: HDC,
    font: HFONT,
    text: &str,
    rect: RECT,
    color: u32,
    flags: u32,
    scale: f64,
) {
    draw_automatic_text(hwnd, dc, font, text, rect, color, flags, scale);
}
thread_local! {static CREDIT_WIDTHS: std::cell::RefCell<HashMap<usize,HashMap<String,i32>>> = std::cell::RefCell::new(HashMap::new());}
thread_local! {static AUTO_MARQUEES:std::cell::RefCell<HashMap<String,(RECT,std::time::Instant,std::time::Instant)>>=std::cell::RefCell::new(HashMap::new());}
// Cleared before DPI-dependent fonts are replaced.
unsafe fn automatic_text_width(dc: HDC, font: HFONT, text: &str) -> i32 {
    CREDIT_WIDTHS.with(|cache| {
        let mut cache = cache.borrow_mut();
        if let Some(width) = cache.get(&(font as usize)).and_then(|values|values.get(text)) {
            return *width;
        }
        if cache.values().map(|values|values.len()).sum::<usize>() >= 2048 {
            cache.clear();
        }
        let previous = SelectObject(dc, font);
        let value = wide(text);
        let mut extent: SIZE = zeroed();
        GetTextExtentPoint32W(dc, value.as_ptr(), (value.len() - 1) as i32, &mut extent);
        SelectObject(dc, previous);
        let width = extent
            .cx
            .max(crate::drawing::lyric_text_width(dc, font, text) + 4);
        cache.entry(font as usize).or_default().insert(text.to_owned(), width);
        width
    })
}

fn reset_marquees() {
    AUTO_MARQUEES.with(|states| states.borrow_mut().clear());
    MARQUEE.with(|state| *state.borrow_mut() = None);
}

unsafe fn draw_automatic_text(
    hwnd: HWND,
    dc: HDC,
    font: HFONT,
    text: &str,
    rect: RECT,
    color: u32,
    flags: u32,
    scale: f64,
) {
    if RectVisible(dc,&rect)==0 {return;}
    let width = automatic_text_width(dc, font, text);
    let overflow = width - (rect.right - rect.left);
    if overflow <= 0 {
        draw_text(
            dc,
            font,
            text,
            rect,
            color,
            flags | DT_SINGLELINE | crate::drawing::LYRICS_UNTRIMMED,
        );
        return;
    }
    let now = std::time::Instant::now();
    let key = format!(
        "{}:{}:{}:{}:{}",
        rect.left, rect.top, rect.right, rect.bottom, text
    );
    let elapsed = AUTO_MARQUEES.with(|states| {
        let mut states = states.borrow_mut();
        let state = states.entry(key).or_insert((rect, now, now));
        state.0 = rect;
        state.2 = now;
        state.1.elapsed().as_secs_f64()
    });
    start_animation_timer(hwnd, 4);
    let saved = SaveDC(dc);
    IntersectClipRect(dc, rect.left, rect.top, rect.right, rect.bottom);
    let mut scrolling = rect;
    scrolling.left -= marquee_offset(elapsed, overflow as f64, 36. * scale) as i32;
    scrolling.right = scrolling.left + width + 2;
    draw_text(
        dc,
        font,
        text,
        scrolling,
        color,
        DT_LEFT | DT_VCENTER | DT_SINGLELINE | crate::drawing::LYRICS_UNTRIMMED,
    );
    RestoreDC(dc, saved);
}
fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}
// Win32 MAKEINTRESOURCE: integer IDs are passed as tagged pointers, never read.
fn resource(id: u16) -> *const u16 {
    id as usize as *const u16
}
#[derive(Clone, PartialEq)]
struct EditBackdrop {
    width: i32,
    height: i32,
    pixels: Vec<u32>,
}
struct App {
    queue_drag: Option<(usize, i32, bool, usize, Vec<String>)>,
    queue_scroll_drag: bool,
    queue_wheel_remainder: f64,
    queue_glide: Option<(f64, f64, f64, std::time::Instant)>,
    middle_scroll: Option<(bool, i32, bool, std::time::Instant)>,
    caption_buttons: [HWND; 3],
    collapse_button: HWND,
    tray_added: bool,
    search_backdrop: std::cell::RefCell<Option<EditBackdrop>>,
    worker: Worker,
    updater: crate::updater::Worker,
    media_controls: Option<crate::media_controls::Controls>,
    shortcuts: Option<crate::shortcuts::Shortcuts>,
    snapshot: Snapshot,
    playback_clock: PlaybackClock,
    karaoke_preview_started: std::time::Instant,
    lyric_motion: Option<(std::time::Instant, i32)>,
    lyrics_mode_motion: Option<(std::time::Instant, f32, f32)>,
    lyric_motion_offset: i32,
    lyric_motion_track: String,
    ui: Ui,
    scene: Scene,
    buttons: Vec<HWND>,
    button_text: Vec<String>,
    canvas_pressed: Option<(usize, String)>,
    canvas_hover: Option<usize>,
    canvas_focus: Option<usize>,
    canvas_keyboard: HWND,
    edits: HashMap<i32, HWND>,
    fonts: [HFONT; 12],
    lyrics_fonts: Option<(u32, i32, HFONT, HFONT, HFONT)>,
    brush: HBRUSH,
    scale: f64,
    acrylic: bool,
    pending: Option<u64>,
    notice: String,
    volume: u32,
    scrolling: bool,
    render_suspended: bool,
    layout_dirty: bool,
    scroll_grab_offset: i32,
    catalog: crate::catalog::Worker,

    spotify_service: Option<crate::spotify::service::Service>,
    spotify_opened: bool,
    spotify_setup: Option<(
        Arc<std::sync::atomic::AtomicBool>,
        std::sync::mpsc::Receiver<Result<(), String>>,
    )>,
    spotify_auth: Option<(
        Arc<std::sync::atomic::AtomicBool>,
        std::sync::mpsc::Receiver<Result<(), String>>,
    )>,
    spotify_device_request: Option<u64>,
    spotify_jam_request: Option<(u64, Option<String>, bool, crate::spotify::jam::Request)>,
    deezer_auth: Option<(
        Arc<std::sync::atomic::AtomicBool>,
        std::sync::mpsc::Receiver<Result<String, String>>,
    )>,

    artwork: crate::catalog::Artwork,
    images: HashMap<String, Arc<crate::catalog::Bitmap>>,
    image_order: VecDeque<String>,
    image_pending: HashSet<String>,
    image_retry_at: HashMap<String, std::time::Instant>,
    generation: u64,
    entity: Option<crate::catalog::Entry>,
    query: String,
    navigation: Vec<(View, Option<crate::catalog::Entry>, String, i32)>,
    lyrics_id: String,
    background: Option<Arc<crate::catalog::Bitmap>>,
    background_source: String,
    background_requested: bool,
    backdrop_worker: crate::appearance::Worker,
    previous_background: Option<Arc<crate::catalog::Bitmap>>,
    background_transition: BackgroundTransition,
    previous_cover: Option<Arc<crate::catalog::Bitmap>>,
    cover_source: String,
    cover_changed: Option<std::time::Instant>,
    custom_background: String,
    image_times: HashMap<String, std::time::Instant>,
    scroll_target: i32,
    scroll_origin: i32,
    scroll_position: f64,
    scroll_velocity: f64,
    scroll_tick: std::time::Instant,
    wheel_remainder: f64,
    layout_scroll: i32,
    artwork_prefetch_scroll: i32,
    artwork_prefetch_upward: bool,
    scroll_started: std::time::Instant,
    backdrop_cache: std::cell::RefCell<(Option<BackdropKey>, Vec<u32>)>,
    palette_state: std::cell::RefCell<PaletteState>,
    cover_luminance_cache: std::cell::RefCell<Option<(u64, f32)>>,
    downloads: crate::downloads::Worker,
    download_cancels: HashMap<u64, Arc<std::sync::atomic::AtomicBool>>,
    flow_save_expected: Option<Vec<(String, bool)>>,
    flow_verify_attempt: u8,
    search_hint: String,
    page_cache: VecDeque<(String, Arc<crate::catalog::Page>)>,
    toggle_animations: HashMap<String, (f32, f32, std::time::Instant)>,
    eq_drag: Option<(u8, usize)>,
    visualizer: crate::visualizer::Visualizer,
    podcast_ids: HashSet<String>,
    library_pending: Option<(u64, crate::catalog::Entry, bool)>,
    playlist_generation: Option<u64>,
    discord_seen: serde_json::Value,
    library_loaded: bool,
    lyrics_hover_start: std::time::Instant,
    lyrics_hover_from: f32,
    lyrics_hover_to: f32,
    connect: Option<crate::connect::ConnectHub>,
}
struct PlaybackClock {
    position: u64,
    sampled_at: std::time::Instant,
    correction: i64,
}

impl Default for PlaybackClock {
    fn default() -> Self {
        Self {
            position: 0,
            sampled_at: std::time::Instant::now(),
            correction: 0,
        }
    }
}

impl PlaybackClock {
    fn position(&self, now: std::time::Instant, playing: bool) -> u64 {
        if !playing {
            return self.position;
        }
        let elapsed = now
            .saturating_duration_since(self.sampled_at)
            .as_millis()
            .min(1500) as u64;
        let correction = self.correction * elapsed.min(500) as i64 / 500;
        self.position
            .saturating_add(elapsed)
            .saturating_add_signed(correction)
    }

    fn sample(&mut self, position: u64, now: std::time::Instant, smooth: bool) {
        let predicted = self.position(now, true);
        let error = position as i128 - predicted as i128;
        if smooth && error.abs() < 500 {
            // Preserve this frame's position and absorb small engine corrections
            // over half a second. The clock always moves forward while playing.
            self.position = predicted;
            self.correction = error.clamp(-250, 250) as i64;
        } else {
            // Seeks, track changes and playback-state changes land immediately.
            self.position = position;
            self.correction = 0;
        }
        self.sampled_at = now;
    }
}

impl App {
    fn playback_position(&self) -> u64 {
        let position = self
            .playback_clock
            .position(std::time::Instant::now(), self.playing());
        if self.snapshot.duration > 0 {
            position.min(self.snapshot.duration)
        } else {
            position
        }
    }

    fn lyric_layout_changed(&self, position: u64) -> bool {
        if self.ui.view != View::Lyrics {
            return false;
        }
        if self.ui.rich_lyrics.is_empty() {
            return self.ui.lyrics.iter().rposition(|l| l.0 <= position)
                != self.ui.lyrics.iter().rposition(|l| l.0 <= self.ui.position);
        }
        let lines = &self.ui.rich_lyrics;
        lines.iter().rposition(|l| l.start_ms <= position)
            != lines.iter().rposition(|l| l.start_ms <= self.ui.position)
            || crate::lyrics::instrumental_gaps(lines, self.ui.duration)
                .iter()
                .any(|&(start, end)| {
                    (start <= position && position < end)
                        != (start <= self.ui.position && self.ui.position < end)
                })
    }

    fn lyrics_hover_progress(&self) -> f32 {
        let elapsed = self.lyrics_hover_start.elapsed().as_secs_f32();
        let dur = 0.18;
        let t = (elapsed / dur).min(1.0);
        let ease = t * t * (3.0 - 2.0 * t);
        (self.lyrics_hover_from + (self.lyrics_hover_to - self.lyrics_hover_from) * ease)
            .clamp(0.0, 1.0)
    }

    fn set_lyrics_cover_hover(&mut self, hovered: bool) {
        if self.ui.lyrics_cover_hovered != hovered {
            let current = self.lyrics_hover_progress();
            self.lyrics_hover_from = current;
            self.lyrics_hover_to = if hovered { 1.0 } else { 0.0 };
            self.lyrics_hover_start = std::time::Instant::now();
            self.ui.lyrics_cover_hovered = hovered;
        }
    }

    fn toggle_progress(&self, key: &str, fallback_on: bool) -> f32 {
        if let Some(&(start_t, target_t, started)) = self.toggle_animations.get(key) {
            let progress = (started.elapsed().as_secs_f32() / 0.18).clamp(0.0, 1.0);
            let ease = 1.0 - (1.0 - progress).powi(3);
            start_t + (target_t - start_t) * ease
        } else {
            if fallback_on {
                1.0
            } else {
                0.0
            }
        }
    }
    unsafe fn ensure_lyrics_fonts(&mut self, w: i32) {
        let is_fs = self.ui.lyrics_fullscreen || w >= 1400;
        let pct = if is_fs {
            self.ui.lyrics_size_fullscreen.clamp(60, 180)
        } else {
            self.ui.lyrics_size_panel.clamp(60, 180)
        };
        let scale_key = (self.scale * 1000.0).round() as i32;
        if let Some((cached_pct, cached_sk, f_act, f_inact, f_plain)) = self.lyrics_fonts {
            if cached_pct == pct && cached_sk == scale_key {
                return;
            }
            DeleteObject(f_act);
            DeleteObject(f_inact);
            DeleteObject(f_plain);
        }
        let factor = (pct as f64 / 100.0) * self.scale;
        let f_act = CreateFontW(
            (-29.333 * factor).round() as i32,
            0,
            0,
            0,
            700,
            0,
            0,
            0,
            DEFAULT_CHARSET as u32,
            0,
            0,
            ANTIALIASED_QUALITY as u32,
            0,
            wide("Segoe UI").as_ptr(),
        );
        let f_inact = CreateFontW(
            (-29.333 * factor).round() as i32,
            0,
            0,
            0,
            700,
            0,
            0,
            0,
            DEFAULT_CHARSET as u32,
            0,
            0,
            ANTIALIASED_QUALITY as u32,
            0,
            wide("Segoe UI").as_ptr(),
        );
        let f_plain = CreateFontW(
            (-29.333 * factor).round() as i32,
            0,
            0,
            0,
            700,
            0,
            0,
            0,
            DEFAULT_CHARSET as u32,
            0,
            0,
            ANTIALIASED_QUALITY as u32,
            0,
            wide("Segoe UI").as_ptr(),
        );
        self.lyrics_fonts = Some((pct, scale_key, f_act, f_inact, f_plain));
    }
    pub fn cover_luminance(&self) -> f32 {
        if let Some(image) = self.images.get(&self.ui.now.artwork) {
            let mut cache = self.cover_luminance_cache.borrow_mut();
            if let Some((id, lum)) = *cache {
                if id == image.id {
                    return lum;
                }
            }
            let lum = crate::appearance::artwork_luminance(image);
            *cache = Some((image.id, lum));
            lum
        } else if let Some(bg) = self.background.as_ref() {
            let mut cache = self.cover_luminance_cache.borrow_mut();
            if let Some((id, lum)) = *cache {
                if id == bg.id {
                    return lum;
                }
            }
            let lum = crate::appearance::artwork_luminance(bg);
            *cache = Some((bg.id, lum));
            lum
        } else if self.ui.light {
            0.85
        } else {
            0.15
        }
    }

    pub fn effective_backdrop_luminance(&self) -> f32 {
        if self.ui.background_mode == 3 {
            crate::appearance::colorref_luminance(self.ui.solid_background())
        } else if self.ui.background_mode == 0 {
            if self.acrylic {
                if self.ui.light {
                    0.82
                } else {
                    0.18
                }
            } else {
                crate::appearance::colorref_luminance(colors(self.ui.light)[0])
            }
        } else {
            let art_lum = self.cover_luminance();
            let tint_col = if self.ui.light {
                self.ui.light_tint
            } else {
                self.ui.color("Panel", 0x1d1d1d)
            };
            let tint_lum = crate::appearance::colorref_luminance(tint_col);
            let tint_weight = (self.ui.light_opacity as f32 / 100.0).clamp(0.0, 1.0);
            art_lum * (1.0 - tint_weight) + tint_lum * tint_weight
        }
    }

    pub fn is_effectively_light(&self) -> bool {
        if self.ui.background_mode == 3 {
            self.effective_backdrop_luminance() > 0.46
        } else if self.ui.background_mode == 0 {
            self.ui.light
        } else {
            self.effective_backdrop_luminance() > 0.46
        }
    }

    fn raw_target_palette(&self) -> [u32; 9] {
        let light = self.ui.light;
        let mut palette = colors(light);
        for (index, key) in ["Background", "Panel", "Raised", "Border", "Text", "Muted"]
            .iter()
            .enumerate()
        {
            palette[index] = self.ui.color_with_light(key, palette[index], light);
        }
        palette = crate::appearance::readable_palette(palette, light, self.ui.brightness);
        let accent = self.ui.color_with_light("Accent", rgb(159, 57, 124), light);
        let grad_start = self.ui.color_with_light("GradientStart", accent, light);
        let grad_end = self.ui.color_with_light("GradientEnd", accent, light);
        [
            palette[0], palette[1], palette[2], palette[3], palette[4], palette[5], accent,
            grad_start, grad_end,
        ]
    }

    fn interpolated_palette_and_accent(&self) -> [u32; 9] {
        if let Some(palette) = PAINT_PALETTE.with(|value| value.get()) {
            return palette;
        }
        let target = self.raw_target_palette();
        let mut state = self.palette_state.borrow_mut();
        if state.target == [0; 9] {
            state.target = target;
            state.previous = target;
            state.started = std::time::Instant::now();
            return target;
        }
        if state.target != target {
            let t = (state.started.elapsed().as_secs_f32() / (PALETTE_FADE_MS as f32 / 1000.0))
                .min(1.0);
            let ease = t * t * (3.0 - 2.0 * t);
            let mut current = [0u32; 9];
            for i in 0..9 {
                current[i] = lerp_color(state.previous[i], state.target[i], ease);
            }
            state.previous = current;
            state.target = target;
            state.started = std::time::Instant::now();
        }
        let t =
            (state.started.elapsed().as_secs_f32() / (PALETTE_FADE_MS as f32 / 1000.0)).min(1.0);
        let ease = t * t * (3.0 - 2.0 * t);
        let mut result = [0u32; 9];
        for i in 0..9 {
            result[i] = lerp_color(state.previous[i], state.target[i], ease);
        }
        result
    }

    fn palette(&self) -> [u32; 6] {
        let p9 = self.interpolated_palette_and_accent();
        [p9[0], p9[1], p9[2], p9[3], p9[4], p9[5]]
    }

    fn accent(&self) -> u32 {
        let p9 = self.interpolated_palette_and_accent();
        p9[6]
    }

    fn gradient(&self) -> (u32, u32) {
        let p9 = self.interpolated_palette_and_accent();
        (p9[7], p9[8])
    }
    fn px(&self, n: i32) -> i32 {
        (n as f64 * self.scale).round() as i32
    }
    fn playing(&self) -> bool {
        {
            self.snapshot.state == "Playing"
        }
    }
    fn corner(&self, radius: i32) -> i32 {
        self.px(self.ui.corner_px(radius))
    }
    fn rect(&self, r: Rect) -> RECT {
        RECT {
            left: self.px(r.x),
            top: self.px(r.y),
            right: self.px(r.x + r.w),
            bottom: self.px(r.y + r.h),
        }
    }
}
unsafe fn rounded(dc: HDC, r: RECT, color: u32, radius: i32) {
    if crate::drawing::fill(dc, r, crate::drawing::argb(color, 255), radius as f32 / 2.0) {
        return;
    }
    let brush = CreateSolidBrush(color);
    let old = SelectObject(dc, brush);
    let pen = SelectObject(dc, GetStockObject(NULL_PEN));
    RoundRect(dc, r.left, r.top, r.right, r.bottom, radius, radius);
    SelectObject(dc, old);
    SelectObject(dc, pen);
    DeleteObject(brush);
}
fn rgb(r: u32, g: u32, b: u32) -> u32 {
    r | (g << 8) | (b << 16)
}
fn colors(light: bool) -> [u32; 6] {
    if light {
        [
            rgb(242, 244, 248),
            rgb(255, 255, 255),
            rgb(227, 231, 240),
            rgb(191, 198, 212),
            rgb(24, 28, 38),
            rgb(94, 102, 118),
        ]
    } else {
        [
            rgb(23, 23, 23),
            rgb(29, 29, 29),
            rgb(81, 32, 63),
            rgb(64, 49, 65),
            rgb(248, 248, 248),
            rgb(190, 190, 190),
        ]
    }
}
unsafe fn select_fitting_font(dc: HDC, fonts: &[HFONT; 12], text: &str, avail_w: i32) -> HFONT {
    if text.is_empty() || avail_w <= 0 {
        return fonts[0];
    }
    let wide_text = wide(text);
    // Candidates in descending size order: fonts[0] (~10pt / 13.3px), fonts[9] (~9pt / 12px), fonts[7] (~8pt / 10.7px), fonts[6] (~7pt / 9.3px)
    let candidates = [fonts[0], fonts[9], fonts[7], fonts[6]];
    for &font in &candidates {
        if font.is_null() {
            continue;
        }
        let old_font = SelectObject(dc, font);
        let mut test_r = RECT {
            left: 0,
            top: 0,
            right: 0,
            bottom: 0,
        };
        DrawTextW(
            dc,
            wide_text.as_ptr(),
            -1,
            &mut test_r,
            DT_CALCRECT | DT_SINGLELINE,
        );
        SelectObject(dc, old_font);
        if test_r.right - test_r.left <= avail_w {
            return font;
        }
    }
    fonts[6]
}

unsafe fn draw_text(dc: HDC, font: HFONT, value: &str, mut rect: RECT, color: u32, flags: u32) {
    if crate::drawing::text(dc, font, value, rect, color, flags) {
        return;
    }
    SelectObject(dc, font);
    SetTextColor(dc, color);
    SetBkMode(dc, TRANSPARENT as i32);
    let value = wide(value);
    let untrimmed = flags & crate::drawing::LYRICS_UNTRIMMED != 0;
    let flags = flags & !crate::drawing::LYRICS_UNTRIMMED;
    let actual_flags = if untrimmed {
        flags | DT_SINGLELINE
    } else if flags & DT_WORDBREAK != 0 {
        flags | DT_END_ELLIPSIS
    } else {
        flags | DT_SINGLELINE | DT_END_ELLIPSIS
    };
    DrawTextW(dc, value.as_ptr(), -1, &mut rect, actual_flags);
}
unsafe fn draw_karaoke_row(
    dc: HDC,
    font: HFONT,
    line: &crate::lyrics::LyricLine,
    range: std::ops::Range<usize>,
    position: u64,
    rect: RECT,
    palette: [u32; 6],
    accent: u32,
    is_light: bool,
) -> bool {
    let Some(words) = crate::lyrics::word_ranges(line) else {
        return false;
    };
    let measure = |text: &str| crate::drawing::lyric_text_width(dc, font, text);
    let active_color = if is_light { palette[4] } else { 0x00ffffff };
    let inactive_color = if is_light { palette[5] } else { 0x00808080 };
    let shimmer_color = if is_light { accent } else { 0x00ffffff };
    let mut cursor = range.start;
    for (span, word) in words {
        let start = span.start.max(range.start);
        let end = span.end.min(range.end);
        if end <= start {
            continue;
        }
        if start > cursor {
            let mut gap = rect;
            gap.left += measure(&line.text[range.start..cursor]);
            gap.right = gap.left + measure(&line.text[cursor..start]) + 12;
            draw_text(
                dc,
                font,
                &line.text[cursor..start],
                gap,
                inactive_color,
                DT_LEFT | crate::drawing::LYRICS_UNTRIMMED,
            );
        }
        let mut word_rect = rect;
        word_rect.left += measure(&line.text[range.start..start]);
        word_rect.right = word_rect.left + measure(&line.text[start..end]) + 12;
        let progress = if position <= word.start_ms {
            0.0
        } else if word.end_ms <= word.start_ms || position >= word.end_ms {
            1.0
        } else {
            position.saturating_sub(word.start_ms) as f32 / (word.end_ms - word.start_ms) as f32
        };
        crate::drawing::karaoke_text(
            dc,
            font,
            &line.text[start..end],
            progress,
            word_rect,
            active_color,
            inactive_color,
            shimmer_color,
            DT_LEFT | crate::drawing::LYRICS_UNTRIMMED,
        );
        cursor = end;
    }
    if cursor < range.end {
        let mut tail = rect;
        tail.left += measure(&line.text[range.start..cursor]);
        draw_text(
            dc,
            font,
            &line.text[cursor..range.end],
            tail,
            inactive_color,
            DT_LEFT | crate::drawing::LYRICS_UNTRIMMED,
        );
    }
    true
}
unsafe fn make_control(hwnd: HWND, class: &str, id: i32, style: u32, app: &App) -> HWND {
    if class == "EDIT" && matches!(id, SEARCH | NAME | DESCRIPTION) {
        return canvas_input::create(hwnd, id, app.fonts[0]);
    }
    let h = CreateWindowExW(
        if class == "EDIT" {
            0
        } else {
            WS_EX_TRANSPARENT
        },
        wide(class).as_ptr(),
        wide("").as_ptr(),
        WS_CHILD | WS_TABSTOP | style,
        0,
        0,
        1,
        1,
        hwnd,
        id as HMENU,
        GetModuleHandleW(null()),
        null(),
    );
    SendMessageW(h, WM_SETFONT, app.fonts[0] as usize, 0);
    if class == "EDIT" {
        SetWindowSubclass(h, Some(opaque_control), 1, 0);
    } else {
        SetWindowSubclass(h, Some(canvas_control), 1, 0);
    }
    h
}
// Native controls own input and accessibility; the root owns their visuals.
unsafe extern "system" fn canvas_control(
    hwnd: HWND,
    msg: u32,
    wp: usize,
    lp: isize,
    _: usize,
    _: usize,
) -> isize {
    let id = GetDlgCtrlID(hwnd);
    if (id == SEEK || id == VOLUME || (600..604).contains(&id))
        && (msg == WM_LBUTTONDOWN
            || msg == WM_LBUTTONUP
            || (msg == WM_MOUSEMOVE && GetCapture() == hwnd))
    {
        let parent = GetParent(hwnd);
        let pointer = GetWindowLongPtrW(parent, GWLP_USERDATA) as *mut App;
        if !pointer.is_null() {
            let app = &mut *pointer;
            if msg == WM_LBUTTONDOWN {
                SetFocus(hwnd);
                SetCapture(hwnd);
            }
            let mut rect = zeroed();
            GetClientRect(hwnd, &mut rect);
            let x = (lp as u16) as i16 as i32;
            let maximum = SendMessageW(hwnd, TBM_GETRANGEMAX, 0, 0).max(1) as u32;
            let vertical = id == VOLUME && app.scene.volume_vertical;
            let value = if vertical {
                let y = ((lp as u32 >> 16) as u16) as i16 as i32;
                maximum - slider_value(y, rect.bottom - rect.top, app.px(5), maximum)
            } else {
                slider_value(x, rect.right - rect.left, app.px(5), maximum)
            };
            let previous = SendMessageW(hwnd, WM_USER, 0, 0) as u32;
            SendMessageW(hwnd, TBM_SETPOS, 1, value as isize);
            let event = if msg == WM_LBUTTONUP {
                TB_ENDTRACK
            } else {
                TB_THUMBTRACK
            };
            if previous != value || msg == WM_LBUTTONUP {
                SendMessageW(
                    parent,
                    if vertical { WM_VSCROLL } else { WM_HSCROLL },
                    event as usize,
                    hwnd as isize,
                );
            }
            InvalidateRect(parent, null(), 0);
            if msg == WM_LBUTTONUP {
                ReleaseCapture();
            }
            return 0;
        }
    }
    if id == VOLUME && msg == WM_MOUSEWHEEL {
        thread_local! {static WHEEL:std::cell::Cell<i32>=const{std::cell::Cell::new(0)};}
        let delta = ((wp >> 16) as u16 as i16) as i32;
        let steps = WHEEL.with(|value| {
            let total = value.get() + delta;
            value.set(total % 120);
            total / 120
        });
        let old = SendMessageW(hwnd, WM_USER, 0, 0) as i32;
        let value = (old + steps * 2).clamp(0, 100);
        SendMessageW(hwnd, TBM_SETPOS, 1, value as isize);
        SendMessageW(
            GetParent(hwnd),
            WM_HSCROLL,
            TB_ENDTRACK as usize,
            hwnd as isize,
        );
        return 0;
    }
    if id == VOLUME && msg == WM_KEYDOWN {
        let parent = GetParent(hwnd);
        let pointer = GetWindowLongPtrW(parent, GWLP_USERDATA) as *mut App;
        if !pointer.is_null() {
            let old = SendMessageW(hwnd, WM_USER, 0, 0) as i32;
            let value = match wp as u16 {
                VK_UP | VK_RIGHT => Some((old + 2).min(100)),
                VK_DOWN | VK_LEFT => Some((old - 2).max(0)),
                27 => {
                    SendMessageW(parent, WM_KEYDOWN, 27, 0);
                    return 0;
                }
                VK_HOME => Some(if (*pointer).scene.volume_vertical {
                    100
                } else {
                    0
                }),
                VK_END => Some(if (*pointer).scene.volume_vertical {
                    0
                } else {
                    100
                }),
                _ => None,
            };
            if let Some(value) = value {
                SendMessageW(hwnd, TBM_SETPOS, 1, value as isize);
                SendMessageW(parent, WM_VSCROLL, TB_ENDTRACK as usize, hwnd as isize);
                return 0;
            }
        }
    }
    match msg {
        WM_PAINT => {
            let mut ps = zeroed();
            BeginPaint(hwnd, &mut ps);
            EndPaint(hwnd, &ps);
            return 0;
        }
        WM_ERASEBKGND => return 1,
        WM_PRINTCLIENT => return 0,
        WM_RBUTTONDOWN => return 0,
        WM_RBUTTONUP => {
            let mut pt = POINT {
                x: ((lp as i32) & 0xffff) as i16 as i32,
                y: (((lp as i32) >> 16) & 0xffff) as i16 as i32,
            };
            ClientToScreen(hwnd, &mut pt);
            let parent = GetParent(hwnd);
            if !parent.is_null() {
                // WM_CONTEXTMENU takes screen coordinates. Let the parent convert
                // once, just as it does for a right-click on the window itself.
                let parent_lp =
                    ((pt.x as u16 as usize) | (((pt.y as u16) as usize) << 16)) as isize;
                SendMessageW(parent, WM_CONTEXTMENU, hwnd as usize, parent_lp);
            }
            return 0;
        }
        WM_CONTEXTMENU => {
            let parent = GetParent(hwnd);
            if !parent.is_null() {
                return SendMessageW(parent, WM_CONTEXTMENU, wp, lp);
            }
            return 0;
        }
        _ => {}
    }
    if msg == WM_LBUTTONDOWN {
        let parent = GetParent(hwnd);
        let pointer = GetWindowLongPtrW(parent, GWLP_USERDATA) as *mut App;
        if !pointer.is_null() {
            let app = &mut *pointer;
            let id = GetDlgCtrlID(hwnd).saturating_sub(1000) as usize;
            if let Some(ui::Button {
                action: Action::QueueJump(index),
                ..
            }) = app.scene.buttons.get(id)
            {
                let mut pt: POINT = zeroed();
                GetCursorPos(&mut pt);
                screen_to_content(parent, &mut pt);
                app.queue_drag = Some((
                    *index,
                    pt.y,
                    false,
                    *index,
                    app.ui.queue.iter().map(|e| e.id.clone()).collect(),
                ));
                SetFocus(hwnd);
                SetCapture(parent);
                return 0;
            }
        }
    }
    let result = DefSubclassProc(hwnd, msg, wp, lp);
    if msg == WM_MOUSEMOVE {
        let mut tracking = TRACKMOUSEEVENT {
            cbSize: size_of::<TRACKMOUSEEVENT>() as u32,
            dwFlags: TME_LEAVE,
            hwndTrack: hwnd,
            dwHoverTime: 0,
        };
        TrackMouseEvent(&mut tracking);
    }
    let hover_changed = if msg == WM_MOUSEMOVE {
        HOVERED_CONTROL.replace(hwnd) != hwnd
    } else if msg == WM_MOUSELEAVE {
        HOVERED_CONTROL.replace(null_mut()) == hwnd
    } else {
        false
    };
    if msg == WM_MOUSEMOVE || msg == WM_MOUSELEAVE {
        let parent = GetParent(hwnd);
        if !parent.is_null() {
            let app_ptr = GetWindowLongPtrW(parent, GWLP_USERDATA) as *mut App;
            if !app_ptr.is_null() {
                let app = &mut *app_ptr;
                if app.ui.view == View::Lyrics {
                    let mut cursor: POINT = zeroed();
                    GetCursorPos(&mut cursor);
                    screen_to_content(parent, &mut cursor);
                    let hovered = if let Some(cr) = app.scene.lyrics_cover {
                        PtInRect(&app.rect(cr), cursor) != 0
                    } else {
                        false
                    };
                    if app.ui.lyrics_cover_hovered != hovered {
                        app.set_lyrics_cover_hover(hovered);
                        start_animation_timer(parent, 2);
                        rebuild(parent, app);
                    }
                }
            }
        }
    }
    if hover_changed
        || matches!(
            msg,
            WM_LBUTTONDOWN | WM_LBUTTONUP | WM_KEYDOWN | WM_KEYUP | WM_SETFOCUS | WM_KILLFOCUS
        )
    {
        InvalidateRect(GetParent(hwnd), null(), 0);
    }
    result
}

fn slider_value(x: i32, width: i32, margin: i32, maximum: u32) -> u32 {
    let span = (width - 2 * margin).max(1) as u64;
    let offset = (x - margin).clamp(0, span as i32) as u64;
    ((offset * maximum as u64 + span / 2) / span) as u32
}

fn library_job(item: &crate::catalog::Entry, add: bool) -> Result<crate::catalog::Job, String> {
    use serde_json::json;
    let (name, args) = match item.kind.as_str() {
        "track" => (
            if add {
                "DZAddFavorite"
            } else {
                "DZRemoveFavorite"
            },
            vec![json!(item.id)],
        ),
        "album" => (
            "DZAlbumLibraryJSON",
            vec![json!(item.id), json!(add as i32)],
        ),
        "playlist" => (
            "DZPlaylistLibraryJSON",
            vec![json!(item.id), json!(add as i32)],
        ),
        "podcast" => ("RustPodcastLibraryJSON", vec![item.wire(), json!(add)]),
        _ => return Err("This item cannot be saved with the current engine".into()),
    };
    Ok(crate::catalog::Job::Api(name.into(), args))
}
fn library_operation(name: &str) -> bool {
    matches!(
        name,
        "DZAddFavorite"
            | "DZRemoveFavorite"
            | "DZAlbumLibraryJSON"
            | "DZPlaylistLibraryJSON"
            | "RustPodcastLibraryJSON"
    )
}
unsafe fn toggle_library(app: &mut App, item: crate::catalog::Entry) {
    if app.library_pending.is_some() {
        app.notice = "A library update is still in progress".into();
        return;
    }
    let add = !app.ui.liked_ids.contains(&item.library_key());
    let job = match library_job(&item, add) {
        Ok(job) => job,
        Err(error) => {
            app.notice = error;
            return;
        }
    };
    if !app.snapshot.connected {
        app.notice = format!("Connect to {} first", app.ui.provider.label());
        return;
    }
    if app
        .catalog
        .requests
        .send(crate::catalog::Request {
            #[cfg(feature = "diagnostics")]
            view_tag: app.ui.view as usize + 1,
            generation: app.generation,
            provider: app.ui.provider,
            engine: app.snapshot.engine.clone(),
            job,
        })
        .is_err()
    {
        app.notice = "Catalog worker is unavailable.".into();
        return;
    }
    app.library_pending = Some((app.generation, item, add));
    app.notice = "Updating library...".into();
}
fn adopt_podcasts(app: &mut App, value: &serde_json::Value) {
    for id in &app.podcast_ids {
        app.ui.liked_ids.remove(&format!("podcast:{id}"));
    }
    app.podcast_ids = crate::catalog::entries(&value["podcasts"], "podcast")
        .into_iter()
        .map(|item| item.id)
        .collect();
    for id in &app.podcast_ids {
        app.ui.liked_ids.insert(format!("podcast:{id}"));
    }
}

fn share_link(item: &crate::catalog::Entry) -> Option<String> {
    if let Some(rest) = item.id.strip_prefix("youtube:") {
        let (kind, id) = rest.split_once(':')?;
        if id.is_empty()
            || !id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
        {
            return None;
        }
        return match kind {
            "track" => Some(format!("https://music.youtube.com/watch?v={id}")),
            "playlist" => Some(format!("https://music.youtube.com/playlist?list={id}")),
            "artist" | "album" => Some(format!("https://music.youtube.com/browse/{id}")),
            _ => None,
        };
    }
    if let Some(rest) = item.id.strip_prefix("spotify:") {
        let (kind, id) = rest.split_once(':')?;
        if matches!(
            kind,
            "track" | "episode" | "album" | "artist" | "playlist" | "show"
        ) && id.len() == 22
            && id.bytes().all(|byte| byte.is_ascii_alphanumeric())
        {
            return Some(format!("https://open.spotify.com/{kind}/{id}"));
        }
        return None;
    }
    if item.id.is_empty() || !item.id.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let kind = match item.kind.as_str() {
        "track" | "episode" | "album" | "artist" | "playlist" | "podcast" => item.kind.as_str(),
        _ => return None,
    };
    Some(format!("https://www.deezer.com/{kind}/{}", item.id))
}
unsafe fn copy_text(hwnd: HWND, text: &str) -> Result<(), String> {
    #[link(name = "user32")]
    unsafe extern "system" {
        fn OpenClipboard(hwnd: HWND) -> i32;
        fn CloseClipboard() -> i32;
        fn EmptyClipboard() -> i32;
        fn SetClipboardData(format: u32, data: *mut c_void) -> *mut c_void;
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GlobalAlloc(flags: u32, size: usize) -> *mut c_void;
        fn GlobalLock(memory: *mut c_void) -> *mut c_void;
        fn GlobalUnlock(memory: *mut c_void) -> i32;
        fn GlobalFree(memory: *mut c_void) -> *mut c_void;
    }
    let text = wide(text);
    let memory = GlobalAlloc(2, text.len() * 2);
    if memory.is_null() {
        return Err("Could not allocate clipboard text".into());
    }
    let pointer = GlobalLock(memory);
    if pointer.is_null() {
        GlobalFree(memory);
        return Err("Could not prepare clipboard text".into());
    }
    std::ptr::copy_nonoverlapping(text.as_ptr(), pointer.cast::<u16>(), text.len());
    GlobalUnlock(memory);
    if OpenClipboard(hwnd) == 0 {
        GlobalFree(memory);
        return Err("Clipboard is busy; try again".into());
    }
    let success = EmptyClipboard() != 0 && !SetClipboardData(13, memory).is_null();
    CloseClipboard();
    if success {
        Ok(())
    } else {
        GlobalFree(memory);
        Err("Could not copy link".into())
    }
}
// The search canvas samples the shell so its background follows glass and theme changes.
unsafe fn cache_search_backdrop(app: &App, bits: *const u32, stride: i32, width: i32, height: i32) {
    if bits.is_null() {
        return;
    }
    let Some(rect) = app.scene.search.map(|r| app.rect(r)) else {
        app.search_backdrop.borrow_mut().take();
        return;
    };
    if rect.left < 0 || rect.top < 0 || rect.right > width || rect.bottom > height {
        return;
    }
    GdiFlush();
    let mut next = EditBackdrop {
        width: rect.right - rect.left,
        height: rect.bottom - rect.top,
        pixels: Vec::new(),
    };
    for y in rect.top..rect.bottom {
        next.pixels.extend_from_slice(std::slice::from_raw_parts(
            bits.offset((y * stride + rect.left) as isize),
            next.width as usize,
        ));
    }
    let mut cached = app.search_backdrop.borrow_mut();
    if cached.as_ref() != Some(&next) {
        *cached = Some(next);
        if let Some(control) = app.edits.get(&SEARCH) {
            InvalidateRect(*control, null(), 0);
        }
    }
}
// Only session and track-ID fields use the native editor.
unsafe extern "system" fn opaque_control(
    hwnd: HWND, msg: u32, wp: usize, lp: isize, _: usize, _: usize,
) -> isize {
    if msg == WM_PAINT {
        let mut ps = zeroed();
        let dc = BeginPaint(hwnd, &mut ps);
        let mut rect = zeroed();
        GetClientRect(hwnd, &mut rect);
        let params = BP_PAINTPARAMS { cbSize: size_of::<BP_PAINTPARAMS>() as u32,
            dwFlags: BPPF_ERASE, ..zeroed() };
        let mut memory = null_mut();
        let buffer = BeginBufferedPaint(dc, &rect, BPBF_TOPDOWNDIB, &params, &mut memory);
        if buffer != 0 {
            DefSubclassProc(hwnd, WM_PRINTCLIENT, memory as usize, PRF_CLIENT as isize);
            BufferedPaintSetAlpha(buffer, null(), 255);
            EndBufferedPaint(buffer, 1);
        } else { DefSubclassProc(hwnd, WM_PRINTCLIENT, dc as usize, PRF_CLIENT as isize); }
        EndPaint(hwnd, &ps);
        return 0;
    }
    DefSubclassProc(hwnd, msg, wp, lp)
}
// The caption shares the client backdrop; content coordinates remain independent of it.
fn custom_caption(ui: &Ui) -> bool {
    !ui.window_controls || !ui.controls_right || ui.traffic_lights
}
fn caption_button_rects(width: i32, height: i32, right: bool, traffic: bool) -> [RECT; 3] {
    let button_width = if traffic {
        height * 26 / 31
    } else {
        height * 46 / 31
    };
    let start = if right {
        width - 3 * button_width - 4
    } else {
        4
    };
    std::array::from_fn(|i| {
        let slot = if !right || traffic {
            match i {
                2 => 0,
                0 => 1,
                _ => 2,
            }
        } else {
            i
        };
        RECT {
            left: start + slot as i32 * button_width,
            top: 0,
            right: start + (slot as i32 + 1) * button_width,
            bottom: height,
        }
    })
}
fn caption_collapse_rect(
    width: i32,
    height: i32,
    left_controls: bool,
    traffic: bool,
    scale: f64,
) -> RECT {
    let px = |val: f64| (val * scale).round() as i32;
    let start_x = if left_controls {
        caption_button_rects(width, height, false, traffic)
            .iter()
            .map(|r| r.right)
            .max()
            .unwrap_or(0)
            + px(6.0)
    } else {
        px(6.0)
    };
    let btn_size = px(26.0).min(height - px(4.0)).max(px(16.0));
    let btn_y = (height - btn_size) / 2;
    RECT {
        left: start_x,
        top: btn_y,
        right: start_x + btn_size,
        bottom: btn_y + btn_size,
    }
}
unsafe fn sync_caption_buttons(hwnd: HWND, app: &App) {
    let mut client: RECT = zeroed();
    GetClientRect(hwnd, &mut client);
    let rects = caption_button_rects(
        client.right,
        caption_height(hwnd),
        app.ui.controls_right,
        app.ui.traffic_lights,
    );
    for (button, rect) in app.caption_buttons.iter().zip(rects) {
        if button.is_null() {
            continue;
        }
        SetWindowPos(
            *button,
            null_mut(),
            rect.left,
            rect.top,
            rect.right - rect.left,
            rect.bottom - rect.top,
            SWP_NOZORDER | SWP_NOACTIVATE | SWP_NOREDRAW,
        );
        ShowWindow(
            *button,
            if custom_caption(&app.ui) && app.ui.window_controls {
                SW_SHOW
            } else {
                SW_HIDE
            },
        );
    }
    if !app.collapse_button.is_null() {
        let height = caption_height(hwnd);
        let left_controls =
            custom_caption(&app.ui) && app.ui.window_controls && !app.ui.controls_right;
        let rect = caption_collapse_rect(
            client.right,
            height,
            left_controls,
            app.ui.traffic_lights,
            app.scale,
        );
        SetWindowPos(
            app.collapse_button,
            null_mut(),
            rect.left,
            rect.top,
            rect.right - rect.left,
            rect.bottom - rect.top,
            SWP_NOZORDER | SWP_NOACTIVATE | SWP_NOREDRAW,
        );
        ShowWindow(app.collapse_button, SW_SHOW);
        SetWindowTextW(
            app.collapse_button,
            wide(app.ui.t(if app.ui.sidebar_collapsed {
                "Expand sidebar"
            } else {
                "Collapse sidebar"
            }))
            .as_ptr(),
        );
    }
}
unsafe fn native_caption_button_rects(hwnd: HWND, width: i32, height: i32) -> [RECT; 3] {
    let mut bounds: RECT = zeroed();
    let valid = DwmGetWindowAttribute(
        hwnd,
        DWMWA_CAPTION_BUTTON_BOUNDS as u32,
        (&mut bounds as *mut RECT).cast(),
        size_of::<RECT>() as u32,
    ) == 0
        && bounds.right > bounds.left;
    let (right, button_width) = if valid {
        let mut window: RECT = zeroed();
        GetWindowRect(hwnd, &mut window);
        let mut edge = POINT {
            x: window.left + bounds.right,
            y: window.top,
        };
        ScreenToClient(hwnd, &mut edge);
        (edge.x, (bounds.right - bounds.left) / 3)
    } else {
        (width, (46 * GetDpiForWindow(hwnd).max(96) as i32 + 48) / 96)
    };
    std::array::from_fn(|index| RECT {
        left: right - (3 - index as i32) * button_width,
        right: right - (2 - index as i32) * button_width,
        top: 0,
        bottom: height,
    })
}
unsafe fn apply_window_controls(hwnd: HWND, app: &mut App) {
    let style = GetWindowLongW(hwnd, GWL_STYLE) as u32;
    let flags = WS_SYSMENU | WS_MINIMIZEBOX | WS_MAXIMIZEBOX;
    let next = if custom_caption(&app.ui) {
        style & !flags
    } else {
        style | flags
    };
    if next != style {
        SetWindowLongW(hwnd, GWL_STYLE, next as i32);
        SetWindowPos(
            hwnd,
            null_mut(),
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED,
        );
    }
    if next != style {
        material(hwnd, app);
    }
    sync_caption_buttons(hwnd, app);
    InvalidateRect(hwnd, null(), 0);
}
unsafe fn sync_tray(hwnd: HWND, app: &mut App) {
    if app.ui.show_in_tray == app.tray_added {
        return;
    }
    let mut data: NOTIFYICONDATAW = zeroed();
    data.cbSize = size_of::<NOTIFYICONDATAW>() as u32;
    data.hWnd = hwnd;
    data.uID = 1;
    if app.ui.show_in_tray {
        data.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
        data.uCallbackMessage = TRAY_CALLBACK;
        data.hIcon = GetClassLongPtrW(hwnd, GCLP_HICON) as HICON;
        if data.hIcon.is_null() {
            data.hIcon = LoadIconW(null_mut(), IDI_APPLICATION);
        }
        let tip = wide("DeezNats");
        data.szTip[..tip.len()].copy_from_slice(&tip);
        app.tray_added = Shell_NotifyIconW(NIM_ADD, &data) != 0;
    } else {
        Shell_NotifyIconW(NIM_DELETE, &data);
        app.tray_added = false;
    }
}
unsafe fn caption_height(hwnd: HWND) -> i32 {
    let dpi = GetDpiForWindow(hwnd).max(96);
    GetSystemMetricsForDpi(SM_CYCAPTION, dpi) + GetSystemMetricsForDpi(SM_CYFRAME, dpi)
}
unsafe fn screen_to_content(hwnd: HWND, point: &mut POINT) {
    ScreenToClient(hwnd, point);
    point.y -= caption_height(hwnd);
}
unsafe fn content_to_screen(hwnd: HWND, point: &mut POINT) {
    point.y += caption_height(hwnd);
    ClientToScreen(hwnd, point);
}
unsafe fn caption_message(hwnd: HWND, msg: u32, wp: usize, lp: isize) -> Option<isize> {
    if matches!(msg, WM_NCMOUSEMOVE | WM_NCMOUSELEAVE) {
        let hovered =
            if msg == WM_NCMOUSEMOVE && matches!(wp as u32, HTMINBUTTON | HTMAXBUTTON | HTCLOSE) {
                wp
            } else {
                0
            };
        if CAPTION_HOVER.replace((hwnd, hovered)) != (hwnd, hovered) {
            let mut caption: RECT = zeroed();
            GetClientRect(hwnd, &mut caption);
            caption.bottom = caption_height(hwnd);
            InvalidateRect(hwnd, &caption, 0);
        }
        if hovered != 0 {
            let mut tracking = TRACKMOUSEEVENT {
                cbSize: size_of::<TRACKMOUSEEVENT>() as u32,
                dwFlags: windows_sys::Win32::UI::Input::KeyboardAndMouse::TME_NONCLIENT | TME_LEAVE,
                hwndTrack: hwnd,
                dwHoverTime: 0,
            };
            TrackMouseEvent(&mut tracking);
        }
    }
    if msg == WM_NCCALCSIZE && wp != 0 {
        let params = &mut *(lp as *mut NCCALCSIZE_PARAMS);
        let top = params.rgrc[0].top;
        DefWindowProcW(hwnd, msg, wp, lp);
        params.rgrc[0].top = top
            + if IsZoomed(hwnd) != 0 {
                let dpi = GetDpiForWindow(hwnd).max(96);
                GetSystemMetricsForDpi(SM_CYFRAME, dpi)
                    + GetSystemMetricsForDpi(SM_CXPADDEDBORDER, dpi)
            } else {
                0
            };
        return Some(0);
    }
    if matches!(
        msg,
        WM_NCHITTEST
            | WM_NCMOUSEMOVE
            | WM_NCMOUSELEAVE
            | WM_NCLBUTTONDOWN
            | WM_NCLBUTTONUP
            | WM_NCLBUTTONDBLCLK
    ) {
        let mut result = 0;
        if DwmDefWindowProc(hwnd, msg, wp, lp, &mut result) != 0 {
            return Some(result);
        }
        if msg == WM_NCHITTEST {
            let mut point = POINT {
                x: lp as u16 as i16 as i32,
                y: (lp as u32 >> 16) as u16 as i16 as i32,
            };
            ScreenToClient(hwnd, &mut point);
            if point.y >= 0 && point.y < caption_height(hwnd) {
                let mut client: RECT = zeroed();
                GetClientRect(hwnd, &mut client);
                let rects = native_caption_button_rects(hwnd, client.right, caption_height(hwnd));
                let style = GetWindowLongW(hwnd, GWL_STYLE) as u32;
                for (rect, hit, flag) in [
                    (rects[0], HTMINBUTTON, WS_MINIMIZEBOX),
                    (rects[1], HTMAXBUTTON, WS_MAXIMIZEBOX),
                    (rects[2], HTCLOSE, WS_SYSMENU),
                ] {
                    if style & flag != 0 && PtInRect(&rect, point) != 0 {
                        return Some(hit as isize);
                    }
                }
            }
            let default = DefWindowProcW(hwnd, msg, wp, lp);
            if default != HTCLIENT as isize {
                return Some(default);
            }
            let edge = GetSystemMetricsForDpi(SM_CYFRAME, GetDpiForWindow(hwnd).max(96));
            if IsZoomed(hwnd) == 0 && point.y < edge {
                return Some(HTTOP as isize);
            }
            let pointer = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut App;
            if !pointer.is_null() && point.y >= 0 && point.y < caption_height(hwnd) {
                let app = &*pointer;
                let mut client: RECT = zeroed();
                GetClientRect(hwnd, &mut client);
                let height = caption_height(hwnd);
                let left_controls =
                    custom_caption(&app.ui) && app.ui.window_controls && !app.ui.controls_right;
                let collapse_rect = caption_collapse_rect(
                    client.right,
                    height,
                    left_controls,
                    app.ui.traffic_lights,
                    app.scale,
                );
                if PtInRect(&collapse_rect, point) != 0 {
                    return Some(HTCLIENT as isize);
                }
            }
            if point.y < caption_height(hwnd) {
                return Some(HTCAPTION as isize);
            }
            return Some(HTCLIENT as isize);
        }
    }
    None
}
unsafe fn set_control_visible(hwnd: HWND, visible: bool) {
    let style = GetWindowLongW(hwnd, GWL_STYLE) as u32;
    if (style & WS_VISIBLE != 0) != visible {
        ShowWindow(hwnd, if visible { SW_SHOW } else { SW_HIDE });
    }
}

unsafe fn set_control_enabled(hwnd: HWND, enabled: bool) {
    let style = GetWindowLongW(hwnd, GWL_STYLE) as u32;
    if (style & WS_DISABLED == 0) != enabled {
        EnableWindow(hwnd, enabled as i32);
    }
}

unsafe fn set_cached_control_text(hwnd: HWND, cached: &mut String, text: &str) -> bool {
    if cached == text || SetWindowTextW(hwnd, wide(text).as_ptr()) == 0 {
        return false;
    }
    cached.clear();
    cached.push_str(text);
    true
}

unsafe extern "system" fn canvas_keyboard_control(
    hwnd: HWND,
    msg: u32,
    wp: usize,
    lp: isize,
    _: usize,
    _: usize,
) -> isize {
    let parent = GetParent(hwnd);
    if msg == WM_GETDLGCODE {
        let mut flags = DLGC_WANTARROWS;
        if lp != 0 {
            let message = &*(lp as *const MSG);
            if message.message == WM_KEYDOWN
                && (matches!(message.wParam, 13 | 32)
                    || [VK_UP, VK_DOWN, VK_LEFT, VK_RIGHT].contains(&(message.wParam as u16)))
            {
                flags |= DLGC_WANTMESSAGE;
            }
        }
        return flags as isize;
    }
    if msg == WM_SETFOCUS && !parent.is_null() {
        let pointer = GetWindowLongPtrW(parent, GWLP_USERDATA) as *mut App;
        if let Some(app) = pointer.as_mut() {
            if app.canvas_focus.is_none() {
                let (w, h) = size(parent, app);
                app.canvas_focus = (0..app.scene.buttons.len()).find(|&index| {
                    let node = &app.scene.buttons[index];
                    let viewport = button_viewport(app, node, w, h);
                    canvas_track_button(app, index)
                        && scene_button_enabled(app, index)
                        && (matches!(node.action, Action::QueueJump(_))
                            || (node.rect.y + node.rect.h > viewport.y
                                && node.rect.y < viewport.y + viewport.h))
                });
            }
            InvalidateRect(parent, null(), 0);
        }
    }
    if matches!(msg, WM_KEYDOWN | WM_KEYUP)
        && (matches!(wp, 13 | 32) || [VK_UP, VK_DOWN, VK_LEFT, VK_RIGHT].contains(&(wp as u16)))
    {
        if msg == WM_KEYDOWN && !parent.is_null() {
            SendMessageW(parent, msg, wp, lp);
        }
        return 0;
    }
    if msg == WM_CHAR && matches!(wp, 13 | 32) {
        return 0;
    }
    DefSubclassProc(hwnd, msg, wp, lp)
}

unsafe fn put(app: &App, hwnd: HWND, r: Rect, show: bool) {
    if show {
        let mut previous: RECT = zeroed();
        GetWindowRect(hwnd, &mut previous);
        MapWindowPoints(
            null_mut(),
            GetParent(hwnd),
            (&mut previous as *mut RECT).cast(),
            2,
        );
        let mut next = app.rect(r);
        let caption = caption_height(GetParent(hwnd));
        next.top += caption;
        next.bottom += caption;
        let moved = previous.left != next.left
            || previous.top != next.top
            || previous.right != next.right
            || previous.bottom != next.bottom;
        if moved {
            SetWindowPos(
                hwnd,
                null_mut(),
                app.px(r.x),
                app.px(r.y) + caption,
                app.px(r.w),
                app.px(r.h),
                SWP_NOZORDER | SWP_NOACTIVATE | SWP_NOREDRAW,
            );
        }
        set_control_visible(hwnd, true);
        if moved {
            InvalidateRect(hwnd, null(), 0);
        }
    } else {
        set_control_visible(hwnd, false);
    }
}
fn native_ui_scale(dpi: u32) -> f64 {
    // Window size controls layout breakpoints, never text or control density.
    dpi.max(96) as f64 / 96.0
}

unsafe fn size(hwnd: HWND, app: &App) -> (i32, i32) {
    let mut r = zeroed();
    GetClientRect(hwnd, &mut r);
    (
        (r.right as f64 / app.scale) as i32,
        ((r.bottom - caption_height(hwnd)).max(1) as f64 / app.scale) as i32,
    )
}
#[repr(C)]
struct CompositionData {
    attribute: i32,
    data: *mut c_void,
    size: usize,
}
#[repr(C)]
struct AccentPolicy {
    state: i32,
    flags: i32,
    color: u32,
    animation: i32,
}
unsafe fn set_composition(hwnd: HWND, data: *mut CompositionData) -> i32 {
    use windows_sys::Win32::System::LibraryLoader::GetProcAddress;
    let module = GetModuleHandleW(wide("user32.dll").as_ptr());
    let Some(symbol) = GetProcAddress(module, c"SetWindowCompositionAttribute".as_ptr().cast())
    else {
        return 0;
    };
    let function: unsafe extern "system" fn(HWND, *mut CompositionData) -> i32 =
        std::mem::transmute(symbol);
    function(hwnd, data)
}
unsafe fn keep_acrylic(hwnd: HWND, enabled: bool) {
    let mut active = enabled as i32;
    let mut data = CompositionData {
        attribute: 15,
        data: (&mut active as *mut i32).cast(),
        size: 4,
    };
    set_composition(hwnd, &mut data);
}
unsafe fn set_preferred_dark_mode(dark: bool) {
    use windows_sys::Win32::System::LibraryLoader::{
        GetModuleHandleW, GetProcAddress, LoadLibraryW,
    };
    let mut module = GetModuleHandleW(wide("uxtheme.dll").as_ptr());
    if module.is_null() {
        module = LoadLibraryW(wide("uxtheme.dll").as_ptr());
    }
    if !module.is_null() {
        if let Some(symbol) = GetProcAddress(module, 135 as usize as *const _) {
            let set_mode: unsafe extern "system" fn(i32) -> i32 = std::mem::transmute(symbol);
            set_mode(if dark { 2 } else { 3 });
        }
        if let Some(symbol) = GetProcAddress(module, 136 as usize as *const _) {
            let flush: unsafe extern "system" fn() = std::mem::transmute(symbol);
            flush();
        }
    }
}
fn context_menu_favorite_label(item: &crate::catalog::Entry, liked: bool) -> &'static str {
    if item.kind == "track" {
        if liked {
            "Remove from favorites"
        } else {
            "Add to favorites"
        }
    } else if liked {
        "Remove from library"
    } else {
        "Add to library"
    }
}

fn dib(color: u32) -> u32 {
    ((color & 255) << 16) | (color & 0xff00) | ((color >> 16) & 255)
}
fn blend_pixel(front: u32, back: u32, alpha: u32) -> u32 {
    let mut value = 0xff000000;
    for shift in [0, 8, 16] {
        value |= ((((front >> shift) & 255) * alpha + ((back >> shift) & 255) * (255 - alpha))
            / 255)
            << shift;
    }
    value
}
unsafe fn faded_bitmap(
    dc: HDC,
    rect: RECT,
    image: &crate::catalog::Bitmap,
    radius: i32,
    alpha: u8,
) {
    if crate::drawing::image(dc, rect, image, radius as f32 / 2.0, alpha) {
        return;
    }
    if alpha == 255 {
        bitmap(dc, rect, image, radius);
        return;
    }
    let source = CreateCompatibleDC(dc);
    let target = CreateCompatibleBitmap(dc, rect.right - rect.left, rect.bottom - rect.top);
    let old = SelectObject(source, target);
    let local = RECT {
        left: 0,
        top: 0,
        right: rect.right - rect.left,
        bottom: rect.bottom - rect.top,
    };
    bitmap(source, local, image, 0);
    let saved = SaveDC(dc);
    let region = CreateRoundRectRgn(
        rect.left,
        rect.top,
        rect.right + 1,
        rect.bottom + 1,
        radius,
        radius,
    );
    ExtSelectClipRgn(dc, region, RGN_AND);
    DeleteObject(region);
    AlphaBlend(
        dc,
        rect.left,
        rect.top,
        local.right,
        local.bottom,
        source,
        0,
        0,
        local.right,
        local.bottom,
        BLENDFUNCTION {
            BlendOp: AC_SRC_OVER as u8,
            BlendFlags: 0,
            SourceConstantAlpha: alpha,
            AlphaFormat: 0,
        },
    );
    RestoreDC(dc, saved);
    SelectObject(source, old);
    DeleteObject(target);
    DeleteDC(source);
}

unsafe fn material(hwnd: HWND, app: &mut App) {
    DeleteObject(app.brush);
    app.brush = CreateSolidBrush(app.palette()[1]);
    let dark = (!app.ui.light) as i32;
    set_preferred_dark_mode(!app.ui.light);
    let uxtheme =
        windows_sys::Win32::System::LibraryLoader::GetModuleHandleW(wide("uxtheme.dll").as_ptr());
    if !uxtheme.is_null() {
        use windows_sys::Win32::System::LibraryLoader::GetProcAddress;
        if let Some(symbol) = GetProcAddress(uxtheme, c"SetWindowTheme".as_ptr().cast()) {
            let set_theme: unsafe extern "system" fn(HWND, *const u16, *const u16) -> i32 =
                std::mem::transmute(symbol);
            set_theme(
                hwnd,
                wide(if dark != 0 {
                    "DarkMode_Explorer"
                } else {
                    "Explorer"
                })
                .as_ptr(),
                null(),
            );
        }
    }
    DwmSetWindowAttribute(
        hwnd,
        DWMWA_USE_IMMERSIVE_DARK_MODE as u32,
        &dark as *const _ as _,
        4,
    );
    let wants_acrylic = !app.ui.solid && app.ui.window_blur;
    let kind = if !wants_acrylic {
        DWMSBT_NONE
    } else {
        DWMSBT_TRANSIENTWINDOW
    };
    let backdrop_result = DwmSetWindowAttribute(
        hwnd,
        DWMWA_SYSTEMBACKDROP_TYPE as u32,
        &kind as *const _ as _,
        4,
    );
    app.acrylic = wants_acrylic && backdrop_result == 0;
    // Clear the Windows 10 accent fallback when leaving Acrylic. Otherwise its
    // old accent policy can remain active beneath image and solid backgrounds.
    if !wants_acrylic {
        let mut policy = AccentPolicy {
            state: 0,
            flags: 0,
            color: 0,
            animation: 0,
        };
        let mut data = CompositionData {
            attribute: 19,
            data: (&mut policy as *mut AccentPolicy).cast(),
            size: size_of::<AccentPolicy>(),
        };
        set_composition(hwnd, &mut data);
    }
    if wants_acrylic && !app.acrylic {
        let mut policy = AccentPolicy {
            state: 4,
            flags: 2,
            color: 0xb31d1d1d,
            animation: 0,
        };
        let mut data = CompositionData {
            attribute: 19,
            data: (&mut policy as *mut AccentPolicy).cast(),
            size: size_of::<AccentPolicy>(),
        };
        app.acrylic = set_composition(hwnd, &mut data) != 0;
    }
    let blur = DWM_BLURBEHIND {
        dwFlags: DWM_BB_ENABLE,
        fEnable: app.acrylic as i32,
        ..zeroed()
    };
    DwmEnableBlurBehindWindow(hwnd, &blur);
    keep_acrylic(hwnd, app.acrylic);
    let caption = 0xfffffffeu32;
    DwmSetWindowAttribute(
        hwnd,
        DWMWA_CAPTION_COLOR as u32,
        (&caption as *const u32).cast(),
        4,
    );
    let text_color = app.palette()[4];
    DwmSetWindowAttribute(
        hwnd,
        DWMWA_TEXT_COLOR as u32,
        (&text_color as *const u32).cast(),
        4,
    );
    let corner_pref = match app.ui.corner_style {
        0 => 1,
        1 => 3,
        _ => 2,
    };
    DwmSetWindowAttribute(hwnd, 25, &corner_pref as *const _ as _, 4);
    let margins = if app.acrylic {
        MARGINS {
            cxLeftWidth: -1,
            cxRightWidth: -1,
            cyTopHeight: -1,
            cyBottomHeight: -1,
        }
    } else {
        MARGINS {
            cyTopHeight: caption_height(hwnd),
            ..zeroed()
        }
    };
    DwmExtendFrameIntoClientArea(hwnd, &margins);
}
// Inline links keep their layout alignment but only expose the glyph-sized area to input.
fn text_link_rect(slot: Rect, width: i32, height: i32, centered: bool) -> Rect {
    let width = width.clamp(1, slot.w.max(1));
    let height = height.clamp(1, slot.h.max(1));
    Rect::new(
        slot.x + if centered { (slot.w - width) / 2 } else { 0 },
        slot.y + (slot.h - height) / 2,
        width,
        height,
    )
}
fn is_track_row(node: &ui::Button) -> bool {
    !node.text_link
        && node.glyph.is_empty()
        && node.artwork.is_empty()
        && node.rect.h <= 50
        && matches!(&node.action,Action::OpenItem(item) if item.playable())
}
fn available_text_rect(scene: &Scene, slot: Rect, limit: i32) -> Rect {
    let mut right = limit.max(slot.x + slot.w);
    let overlaps = |r: Rect| r.y < slot.y + slot.h && r.y + r.h > slot.y;
    for r in scene
        .buttons
        .iter()
        .map(|b| b.rect)
        .chain(scene.labels.iter().map(|l| l.rect))
        .chain(scene.pictures.iter().map(|(r, _)| *r))
        .chain(scene.seek)
        .chain(scene.volume)
        .chain(scene.elapsed)
        .chain(scene.remaining)
    {
        // Containers and the text's own slot do not obstruct it.
        if overlaps(r) && r.x >= slot.x + slot.w && r.w > 0 {
            right = right.min(r.x - 6);
        }
    }
    Rect::new(slot.x, slot.y, (right - slot.x).max(slot.w), slot.h)
}
unsafe fn expand_media_text(scene: &mut Scene, limit: i32) {
    let labels: Vec<_> = scene
        .labels
        .iter()
        .enumerate()
        .map(|(i,l)| {
            if !scene.catalog_preserved_labels.contains(&i)&&matches!(l.kind, 8) {
                available_text_rect(scene, l.rect, limit)
            } else {
                l.rect
            }
        })
        .collect();
    let groups: HashMap<_, _> = scene
        .buttons
        .iter()
        .enumerate().filter(|(i,_)|!scene.catalog_preserved_buttons.contains(i)).map(|(_,b)|b)
        .filter_map(|b| b.artist_credit_group)
        .map(|r| ((r.x, r.y, r.w, r.h), available_text_rect(scene, r, limit)))
        .collect();
    let links: Vec<_> = scene
        .buttons
        .iter()
        .enumerate()
        .map(|(i,b)| {
            if !scene.catalog_preserved_buttons.contains(&i)&&b.text_link && !b.text_centered && b.artist_credit_group.is_none() {
                available_text_rect(scene, b.rect, limit)
            } else {
                b.rect
            }
        })
        .collect();
    for (label, rect) in scene.labels.iter_mut().zip(labels) {
        label.rect = rect;
    }
    for (button, rect) in scene.buttons.iter_mut().zip(links) {
        button.rect = rect;
        if !button.text_centered {
            if let Some(r) = button.artist_credit_group {
                if let Some(group)=groups.get(&(r.x,r.y,r.w,r.h)){button.artist_credit_group=Some(*group);}
            }
        }
    }
}
unsafe fn fit_text_links(hwnd: HWND, scene: &mut Scene, fonts: &[HFONT; 12], scale: f64) {
    let dc = GetDC(hwnd);
    if dc.is_null() {
        return;
    }
    let previous = GetCurrentObject(dc, OBJ_FONT as u32);
    if let Some(title) = &scene.player_title {
        let label = &scene.labels[title.label];
        SelectObject(dc, fonts[10]);
        let value = wide(&label.text);
        let mut extent: SIZE = zeroed();
        GetTextExtentPoint32W(dc, value.as_ptr(), (value.len() - 1) as i32, &mut extent);
        let natural = ((extent
            .cx
            .max(crate::drawing::lyric_text_width(dc, fonts[10], &label.text))
            + 4) as f64
            / scale)
            .ceil() as i32;
        let (rect, badge) =
            ui::player_title_rects(title.region, natural, title.badge.is_some(), title.centered);
        scene.labels[title.label].rect = rect;
        if let (Some(index), Some(rect)) = (title.badge, badge) {
            scene.labels[index].rect = rect;
        }
    }
    let mut index = 0;
    while index < scene.buttons.len() {
        if scene.catalog_preserved_buttons.contains(&index){index+=1;continue;}
        let node = &scene.buttons[index];
        if let Some(group) = node.artist_credit_group {
            let centered = node.text_centered;
            let end = index
                + scene.buttons[index..]
                    .iter()
                    .take_while(|b| b.artist_credit_group == Some(group))
                    .count();
            let font =
                if scene.player_title.as_ref().is_some_and(|title| {
                    group.x == title.region.x && group.y == title.region.y + 25
                }) {
                    fonts[11]
                } else {
                    fonts[0]
                };
            SelectObject(dc, font);
            let widths: Vec<_> = scene.buttons[index..end]
                .iter()
                .map(|button| {
                    let text: Vec<u16> = button.text.encode_utf16().collect();
                    let mut extent: SIZE = zeroed();
                    GetTextExtentPoint32W(dc, text.as_ptr(), text.len() as i32, &mut extent);
                    ((extent
                        .cx
                        .max(crate::drawing::lyric_text_width(dc, font, &button.text))
                        + 4) as f64
                        / scale)
                        .ceil() as i32
                })
                .collect();
            for (button, rect) in scene.buttons[index..end]
                .iter_mut()
                .zip(ui::artist_credit_rects(group, &widths, centered))
            {
                button.rect = rect;
            }
            index = end;
            continue;
        }
        let node = &mut scene.buttons[index];
        index += 1;
        if !node.text_link {
            continue;
        }
        let font = fonts[0];
        SelectObject(dc, font);
        let text: Vec<u16> = node.text.encode_utf16().collect();
        let mut extent: SIZE = zeroed();
        if GetTextExtentPoint32W(dc, text.as_ptr(), text.len() as i32, &mut extent) != 0 {
            node.rect = text_link_rect(
                node.rect,
                ((extent
                    .cx
                    .max(crate::drawing::lyric_text_width(dc, font, &node.text))
                    + 10) as f64
                    / scale)
                    .ceil() as i32,
                (extent.cy as f64 / scale).ceil() as i32,
                node.text_centered,
            );
        }
    }
    SelectObject(dc, previous);
    ReleaseDC(hwnd, dc);
}
fn smooth_scroll_step(position: f64, velocity: f64, target: f64, seconds: f64) -> (f64, f64) {
    // Exact critically damped spring: continuous velocity on retargeting,
    // independent of frame rate, with no bounce beyond the requested offset.
    let rate = 18.0;
    let dt = seconds.max(0.0);
    let offset = position - target;
    let coefficient = velocity + rate * offset;
    let decay = (-rate * dt).exp();
    let next = target + (offset + coefficient * dt) * decay;
    let speed = (velocity - rate * coefficient * dt) * decay;
    if (next - target).abs() < 0.25 && speed.abs() < 5.0 || (next - target) * offset < 0.0 {
        (target, 0.0)
    } else {
        (next, speed)
    }
}
fn wheel_pixels(delta: i32, lines: u32, viewport: i32, remainder: &mut f64) -> i32 {
    let distance = if lines == u32::MAX {
        viewport.max(1) as f64
    } else {
        lines as f64 * 28.0
    };
    let amount = delta as f64 * distance / 120.0 + *remainder;
    let pixels = amount.trunc() as i32;
    *remainder = amount - pixels as f64;
    pixels
}
fn retarget_queue_glide(state:Option<(f64,f64,f64,std::time::Instant)>,current:f64,pixels:i32,maximum:f64,now:std::time::Instant)
    ->(f64,f64,f64,std::time::Instant) {
    let (position,mut velocity,mut target,tick)=state.unwrap_or((current,0.,current,now));
    if (target-position).signum()==(pixels as f64).signum() {target=position;}
    target=(target-pixels as f64).clamp(0.,maximum);
    if (target-position)*velocity<0. {velocity=0.;}
    (position,velocity,target,tick)
}
fn shift_scroll_content(scene: &mut Scene, delta: i32) {
    for row in &mut scene.catalog_rows {row.y+=delta;}
    for (y, _) in &mut scene.artwork_ahead {
        *y += delta;
    }

    for button in &mut scene.buttons[scene.content_buttons.clone()] {
        button.rect.y += delta;
        if let Some(group) = &mut button.artist_credit_group {
            group.y += delta;
        }
    }
    for label in &mut scene.labels[scene.content_labels.clone()] {
        label.rect.y += delta;
    }
    for panel in &mut scene.panels[scene.content_panels.clone()] {
        panel.rect.y += delta;
    }
    for (rect, _) in &mut scene.pictures[scene.content_pictures.clone()] {
        rect.y += delta;
    }
    for (rect, _, _) in &mut scene.sliders {
        rect.y += delta;
    }
    for rect in [&mut scene.eq_graph, &mut scene.eq_sliders]
        .into_iter()
        .flatten()
    {
        rect.y += delta;
    }
}
#[cfg(test)]
#[test]
fn scrolling_moves_artist_credit_bounds_with_content() {
    let group = ui::Rect::new(10, 100, 200, 20);
    let mut scene = Scene::default();
    scene.buttons = vec![
        ui::Button {
            rect: group,
            artist_credit_group: Some(group),
            ..Default::default()
        },
        ui::Button {
            rect: group,
            artist_credit_group: Some(group),
            ..Default::default()
        },
    ];
    scene.content_buttons = 0..1;
    shift_scroll_content(&mut scene, -35);
    assert_eq!(scene.buttons[0].rect.y, 65);
    assert_eq!(scene.buttons[0].artist_credit_group.unwrap().y, 65);
    assert_eq!(scene.buttons[1].artist_credit_group, Some(group));
    shift_scroll_content(&mut scene, 35);
    assert_eq!(scene.buttons[0].artist_credit_group, Some(group));
}
unsafe fn position_content_buttons(app: &App, w: i32, h: i32) {
    let mut positions = Vec::new();
    for index in app.scene.content_buttons.clone() {
        if canvas_track_button(app, index) {
            continue;
        }
        let node = &app.scene.buttons[index];
        let viewport = button_viewport(app, node, w, h);
        let left = node.rect.x.max(viewport.x);
        let top = node.rect.y.max(viewport.y);
        let right = (node.rect.x + node.rect.w).min(viewport.x + viewport.w);
        let bottom = (node.rect.y + node.rect.h).min(viewport.y + viewport.h);
        if let Some(control) = app.buttons.get(index).filter(|control| !control.is_null()) {
            positions.push((
                *control,
                Rect::new(left, top, (right - left).max(0), (bottom - top).max(0)),
                right > left && bottom > top,
            ));
        }
    }
    // Move the native hit targets together. Their pixels are drawn by the
    // parent's single buffered paint, so per-control repaints are redundant.
    if positions.is_empty() {
        return;
    }
    let caption = positions
        .first()
        .map_or(0, |(control, _, _)| caption_height(GetParent(*control)));
    let mut batch = BeginDeferWindowPos(positions.len() as i32);
    for (control, rect, visible) in &positions {
        if batch.is_null() {
            break;
        }
        if !visible && IsWindowVisible(*control) == 0 {
            continue;
        }
        let mut client: RECT = zeroed();
        GetClientRect(*control, &mut client);
        let unchanged_size = client.right == app.px(rect.w) && client.bottom == app.px(rect.h);
        // Moving a hit target needs neither a resize notification nor a copy
        // of its old pixels: the parent renders the new frame in one buffer.
        batch = DeferWindowPos(
            batch,
            *control,
            null_mut(),
            app.px(rect.x),
            app.px(rect.y) + caption,
            app.px(rect.w),
            app.px(rect.h),
            SWP_NOZORDER
                | SWP_NOACTIVATE
                | SWP_NOREDRAW
                | SWP_NOCOPYBITS
                | SWP_NOSENDCHANGING
                | if unchanged_size { SWP_NOSIZE } else { 0 }
                | if *visible {
                    SWP_SHOWWINDOW
                } else {
                    SWP_HIDEWINDOW
                },
        );
    }
    if !batch.is_null() && EndDeferWindowPos(batch) != 0 {
        return;
    }
    for (control, rect, visible) in positions {
        put(app, control, rect, visible);
    }
}
fn settle_scroll(app: &mut App, position: i32) {
    if position != app.ui.scroll {
        app.artwork_prefetch_upward = position < app.ui.scroll;
    }
    app.ui.scroll = position;
    app.scroll_target = position;
    app.scroll_position = position as f64;
    app.scroll_origin = position;
    app.scroll_velocity = 0.;
    app.scroll_tick = std::time::Instant::now();
    app.scroll_started = app.scroll_tick;
}
fn main_scroll_animating(app:&App)->bool {
    app.ui.smooth_scrolling&&(app.scroll_target!=app.ui.scroll||(app.scroll_position-app.scroll_target as f64).abs()>=0.05||app.scroll_velocity.abs()>=5.)
}
unsafe fn scroll_to_scrollbar_position(hwnd: HWND, app: &mut App, position: i32) {
    if app.ui.smooth_scrolling {
        if !main_scroll_animating(app) {
            app.scroll_position = app.ui.scroll as f64;
            app.scroll_velocity = 0.0;
            app.scroll_tick = std::time::Instant::now();
        }
        app.scroll_target = position;
        if (position as f64 - app.scroll_position) * app.scroll_velocity < 0.0 {
            app.scroll_velocity = 0.0;
        }
        start_animation_timer(hwnd, 2);
    } else {
        settle_scroll(app, position);
        rebuild(hwnd, app);
    }
}
fn scrollbar_thumb_y(top:f64, height:f64, thumb:f64, offset:f64, maximum:f64)->f64 {
    top + offset.clamp(0., maximum.max(0.)) * (height-thumb).max(0.) / maximum.max(1.)
}
fn shown_scrollbar_y(app:&App, panel:&ui::Panel, h:i32)->Option<f64> {
    if panel.kind==6 {
        let track=app.scene.panels.iter().find(|p|p.kind==5&&p.rect.x==panel.rect.x)?;
        let offset=if main_scroll_animating(app){app.scroll_position}else{app.ui.scroll as f64};
        let bottom=if app.ui.view==View::Lyrics{h-64}else{content_bottom(app,h)};
        Some(scrollbar_thumb_y(track.rect.y as f64,track.rect.h as f64,panel.rect.h as f64,offset,(app.scene.content_height-bottom).max(0) as f64))
    }else if panel.kind==17&&app.scene.device_sidebar.is_none(){
        let track=app.scene.queue_scrollbar?;
        let offset=app.queue_glide.map_or((app.ui.queue_scroll*24+app.ui.queue_scroll_offset) as f64,|s|s.0);
        Some(scrollbar_thumb_y(track.y as f64,track.h as f64,panel.rect.h as f64,offset,(app.scene.queue_scroll_max*24) as f64))
    }else{None}
}
fn scrollbar_position(scene: &Scene, y: i32, grab: i32, bottom: i32) -> i32 {
    let Some(track) = scene.panels.iter().find(|p| p.kind == 5) else {
        return 0;
    };
    let Some(thumb) = scene.panels.iter().find(|p| p.kind == 6) else {
        return 0;
    };
    let maximum = (scene.content_height - bottom).max(0);
    let span = (track.rect.h - thumb.rect.h).max(1);
    (((y - track.rect.y - grab).clamp(0, span) as i64 * maximum as i64) / span as i64) as i32
}
unsafe fn scroll_to(hwnd: HWND, app: &mut App, position: i32) {
    #[cfg(feature = "diagnostics")]
    let _timing = crate::diagnostics::span("scroll.update", app.ui.view.title());
    #[cfg(feature = "diagnostics")]
    let _page_allocations = crate::memory_profile::page_scope(app.ui.view as usize + 1);
    let previous = app.ui.scroll;
    if previous == position {
        return;
    }
    // Retire rectangles from the previous screen position.
    reset_marquees();
    app.artwork_prefetch_upward = position < previous;
    app.ui.scroll = position;
    let settings = app.ui.view == View::Settings;
    let manual_lyrics = app.ui.view == View::Lyrics && app.scene.lyrics_anchor.is_none();
    let reuse = settings
        || (manual_lyrics && (position - app.layout_scroll).abs() < ui::SCROLL_OVERSCAN - 32)
        || (app.ui.page.is_some()
            && !matches!(
                app.ui.view,
                View::FlowTune
                    | View::Lyrics
                    | View::Settings
                    | View::Account
                    | View::PlaylistEdit
                    | View::TrackInfo
            )
            && (position - app.layout_scroll).abs() < ui::SCROLL_OVERSCAN - 32);
    if !reuse {
        #[cfg(feature = "diagnostics")]
        let _timing = crate::diagnostics::span("scroll.rebuild", app.ui.view.title());
        let (w,h)=size(hwnd,app);
        let focus=app.canvas_focus.and_then(|i|app.scene.buttons.get(i)).map(|b|canvas_action_key(&b.action));
        shift_scroll_content(&mut app.scene,previous-position);
        if let Some(mut patch)=ui::prepare_catalog_scroll(&mut app.scene,&app.ui,h) {
            expand_media_text(&mut patch.scene,w-16);
            fit_text_links(hwnd,&mut patch.scene,&app.fonts,app.scale);
            ui::finish_catalog_scroll(&mut app.scene,patch);
            gpu::invalidate_main_layer();
            app.layout_scroll=position;app.artwork_prefetch_scroll=position;
            app.canvas_focus=focus.and_then(|key|app.scene.buttons.iter().position(|b|canvas_action_matches(&key,&b.action)));
            app.canvas_hover=None;app.canvas_pressed=None;
            app.buttons.resize(app.scene.buttons.len(),null_mut());
            app.button_text.resize(app.scene.buttons.len(),String::new());
            request_art(app,h);trim_artwork(app,h);
        }else{
            rebuild(hwnd, app);return;
        }
    }else{
        shift_scroll_content(&mut app.scene, previous - position);
    }
    if settings && app.ui.tab == 1 {
        if let Some(rect) = &mut app.scene.session {
            rect.y += previous - position;
        }
    }
    let (w, h) = size(hwnd, app);
    if manual_lyrics && !app.ui.timed_lyrics() {
        let focused = app
            .scene
            .lyric_rows
            .iter()
            .filter(|row| row.range.start == 0)
            .min_by_key(|row| (app.scene.labels[row.label].rect.y - (h / 2 - 20)).abs())
            .map(|row| row.line);
        for row in &app.scene.lyric_rows {
            app.scene.labels[row.label].kind = if Some(row.line) == focused { 13 } else { 12 };
        }
    }
    position_content_buttons(app, w, h);
    if settings {
        for (id, rect) in [(SESSION, app.scene.session)] {
            if id == SESSION && app.ui.tab != 1 {
                continue;
            }
            if let (Some(control), Some(rect)) = (app.edits.get(&id), rect) {
                put(
                    app,
                    *control,
                    rect,
                    rect.y >= app.scene.content_top && rect.y + rect.h <= content_bottom(app, h),
                );
            }
        }
        for (index, (rect, _, _)) in app.scene.sliders.iter().enumerate() {
            if let Some(control) = app.edits.get(&(600 + index as i32)) {
                put(
                    app,
                    *control,
                    *rect,
                    rect.y >= app.scene.content_top && rect.y + rect.h <= content_bottom(app, h),
                );
            }
        }
        if (position - app.layout_scroll).abs() >= 96 {
            app.layout_scroll = position;
            request_art(app, h);
            trim_artwork(app, h);
        }
    }
    if crate::drawing::progressive_artwork_enabled()
        && !settings
        && matches!(app.ui.view, View::Home | View::Explore)
        && (position - app.artwork_prefetch_scroll).abs() >= 96
    {
        app.artwork_prefetch_scroll = position;
        request_art(app, h);
        trim_artwork(app, h);
    }
    let bottom = if app.ui.view == View::Lyrics {
        h - 64
    } else {
        content_bottom(app, h)
    };
    let available = bottom - app.scene.content_top;
    let maximum = (app.scene.content_height - bottom).max(1);
    for panel in &mut app.scene.panels {
        if panel.kind == 6 {
            panel.rect.y = app.scene.content_top
                + ((position as i64 * (available - panel.rect.h) as i64) / maximum as i64) as i32;
        }
    }
    if !settings && !manual_lyrics && scroll_repaint_enabled() {
        let mut client: RECT = zeroed();
        GetClientRect(hwnd, &mut client);
        let rect = scroll_repaint_rect(app, client.right, h, caption_height(hwnd));
        if rect.top >= 0
            && rect.bottom <= client.bottom
            && rect.top < rect.bottom
            && rect.right > 0
            && scroll_repaint_avoids_search(app, &rect, caption_height(hwnd))
        {
            InvalidateRect(hwnd, &rect, 0);
        } else {
            InvalidateRect(hwnd, null(), 0);
        }
    } else {
        InvalidateRect(hwnd, null(), 0);
    }
}
unsafe fn rebuild(hwnd: HWND, app: &mut App) {
    gpu::invalidate_layers();
    #[cfg(feature = "diagnostics")]
    let _timing = crate::diagnostics::span("ui.rebuild", app.ui.view.title());
    #[cfg(feature = "diagnostics")]
    let _page_allocations = crate::memory_profile::page_scope(app.ui.view as usize + 1);
    crate::drawing::set_lyric_cache_scope(
        (app.ui.view == View::Lyrics)
            .then_some((app.ui.provider.label(), app.snapshot.now.id.as_str())),
    );
    if app.render_suspended {
        app.layout_dirty = true;
        return;
    }
    app.layout_dirty = false;
    let scale = native_ui_scale(GetDpiForWindow(hwnd));
    if (app.scale - scale).abs() > 0.0001 {
        app.scale = scale;
        let old_fonts = app.fonts;
        CREDIT_WIDTHS.with(|cache| cache.borrow_mut().clear());
        app.fonts = make_fonts(scale);
        for control in app.edits.values() {
            SendMessageW(*control, WM_SETFONT, app.fonts[0] as usize, 1);
        }
        for font in old_fonts {
            DeleteObject(font);
        }
    }
    let (w, h) = size(hwnd, app);
    app.ui.dpi_scale = app.scale;
    app.ensure_lyrics_fonts(w);
    if let Some(control) = app.edits.get(&SEARCH) {
        let hint = app
            .ui
            .t(if app.ui.view == View::FlowTune {
                "Search genres and subgenres\u{2026}"
            } else if app.ui.view == View::Podcasts {
                "Search podcasts\u{2026}"
            } else {
                "Search songs, artists, albums, playlists or podcasts\u{2026}"
            })
            .to_owned();
        if hint != app.search_hint {
            SendMessageW(*control, EM_SETCUEBANNER, 1, wide(&hint).as_ptr() as isize);
            app.search_hint = hint;
        }
    }
    for (id, hint) in [
        (SESSION, "Paste ARL, or leave empty to use saved session"),
        (TRACK, "Deezer track ID"),
        (NAME, "Playlist name"),
        (DESCRIPTION, "Description"),
    ] {
        if let Some(control) = app.edits.get(&id) {
            SendMessageW(
                *control,
                EM_SETCUEBANNER,
                1,
                wide(app.ui.t(hint)).as_ptr() as isize,
            );
        }
    }
    let previous_anchor = app.scene.lyrics_anchor;
    let previous_scroll = app.layout_scroll;
    app.layout_scroll = app.ui.scroll;
    app.artwork_prefetch_scroll = app.ui.scroll;
    let had_karaoke_preview = app.scene.karaoke_preview.is_some();
    #[cfg(feature = "diagnostics")]
    let build_started = std::time::Instant::now();
    let previous_canvas_focus = app
        .canvas_focus
        .and_then(|index| app.scene.buttons.get(index))
        .map(|node| canvas_action_key(&node.action));
    app.scene = ui::build(
        &app.ui,
        (w, h),
        app.snapshot.connected,
        app.pending.is_some(),
        &app.ui.now.title,
        app.playing(),
        app.volume,
    );
    app.canvas_focus = previous_canvas_focus.and_then(|key| {
        (0..app.scene.buttons.len()).find(|&index| {
            canvas_track_button(app, index)
                && canvas_action_matches(&key, &app.scene.buttons[index].action)
        })
    });
    #[cfg(feature = "diagnostics")]
    crate::diagnostics::record(
        "rebuild.scene",
        app.ui.view.title(),
        build_started.elapsed(),
    );
    if let (Some(before), Some(after)) = (previous_anchor, app.scene.lyrics_anchor) {
        if app.lyric_motion_track == app.ui.now.id
            && previous_scroll == app.ui.scroll
            && app.ui.scroll_animation
        {
            if before != after && (after - before).abs() < h * 2 {
                let offset = after - before + app.lyric_motion_offset;
                app.lyric_motion = Some((std::time::Instant::now(), offset));
                app.lyric_motion_offset = offset;
            } else if before != after {
                // A seek across the song should land directly on its new verse.
                app.lyric_motion = None;
                app.lyric_motion_offset = 0;
            }
            if app.lyric_motion.is_some() {
                shift_scroll_content(&mut app.scene, app.lyric_motion_offset);
                start_animation_timer(hwnd, 2);
            }
        } else {
            app.lyric_motion = None;
            app.lyric_motion_offset = 0;
        }
    } else {
        app.lyric_motion = None;
        app.lyric_motion_offset = 0;
    }
    app.lyric_motion_track = if app.scene.lyrics_anchor.is_some() {
        app.ui.now.id.clone()
    } else {
        String::new()
    };
    if app.ui.view == View::Lyrics && app.ui.timed_lyrics() && app.playing() {
        start_animation_timer(hwnd, 2);
    }
    #[cfg(feature = "diagnostics")]
    let text_started = std::time::Instant::now();
    expand_media_text(&mut app.scene, w - 16);
    fit_text_links(hwnd, &mut app.scene, &app.fonts, app.scale);
    #[cfg(feature = "diagnostics")]
    crate::diagnostics::record("rebuild.text", app.ui.view.title(), text_started.elapsed());
    if app.scene.karaoke_preview.is_some() {
        if !had_karaoke_preview {
            app.karaoke_preview_started = std::time::Instant::now();
        }
        if app.ui.karaoke_lyrics {
            start_animation_timer(hwnd, 2);
        }
    }
    #[cfg(feature = "diagnostics")]
    let controls_started = std::time::Instant::now();
    let canvas_available =
        (0..app.scene.buttons.len()).any(|index| canvas_track_button(app, index));
    if canvas_available && app.canvas_keyboard.is_null() {
        app.canvas_keyboard = make_control(hwnd, "BUTTON", 998, BS_PUSHBUTTON as u32, app);
        if !app.canvas_keyboard.is_null() {
            SetWindowSubclass(app.canvas_keyboard, Some(canvas_keyboard_control), 2, 0);
            SetWindowTextW(app.canvas_keyboard, wide(app.ui.t("Track list")).as_ptr());
            SetWindowPos(
                app.canvas_keyboard,
                null_mut(),
                -10,
                -10,
                1,
                1,
                SWP_NOZORDER | SWP_NOACTIVATE | SWP_NOREDRAW,
            );
        }
    }
    if !app.canvas_keyboard.is_null() {
        set_control_visible(app.canvas_keyboard, canvas_available);
    }
    // Scene buttons are painted and hit-tested by the canvas. Keep the
    // index-aligned slots for drawing, without creating native child windows.
    app.buttons.resize(app.scene.buttons.len(), null_mut());
    app.button_text
        .resize(app.scene.buttons.len(), String::new());
    sync_caption_buttons(hwnd, app);
    for (id, r) in [
        (SEARCH, app.scene.search),
        (SESSION, app.scene.session),
        (TRACK, app.scene.track),
        (NAME, app.scene.name),
        (DESCRIPTION, app.scene.description),
        (SEEK, app.scene.seek),
        (VOLUME, app.scene.volume),
    ] {
        if let Some(&control) = app.edits.get(&id) {
            put(
                app,
                control,
                r.unwrap_or(Rect::new(0, 0, 1, 1)),
                r.is_some_and(|r| {
                    matches!(id, SEARCH | SEEK | VOLUME)
                        || (r.y >= app.scene.content_top && r.y + r.h <= content_bottom(app, h))
                }),
            );
        }
    }
    for id in [NAME, DESCRIPTION] {
        if let Some(&control) = app.edits.get(&id) {
            set_control_enabled(
                control,
                !(app.ui.view == View::PlaylistEdit && app.ui.playlist_busy),
            );
        }
    }
    let Some(&volume_control) = app.edits.get(&VOLUME) else {
        return;
    };
    let old_style = GetWindowLongW(volume_control, GWL_STYLE) as u32;
    let new_style = if app.scene.volume_vertical {
        old_style | TBS_VERT
    } else {
        old_style & !TBS_VERT
    };
    if new_style != old_style {
        SetWindowLongW(volume_control, GWL_STYLE, new_style as i32);
    }
    let Some(&status) = app.edits.get(&STATUS) else {
        return;
    };
    for index in 0..8 {
        let id = 600 + index;
        if let Some((rect, key, value)) = app.scene.sliders.get(index as usize).copied() {
            if !app.edits.contains_key(&id) {
                let control = make_control(hwnd, "msctls_trackbar32", id, TBS_NOTICKS, app);
                app.edits.insert(id, control);
            }
            let control = app.edits[&id];
            let min_range = 0;
            let max_range = if matches!(key, "Light-mode brightness" | "Background brightness") {
                110
            } else {
                100
            };
            SendMessageW(
                control,
                TBM_SETRANGE,
                0,
                (min_range as isize) | ((max_range as isize) << 16),
            );
            if GetCapture() != control {
                SendMessageW(control, TBM_SETPOS, 0, value as isize);
            }
            put(
                app,
                control,
                rect,
                rect.y >= app.scene.content_top && rect.y + rect.h <= content_bottom(app, h),
            );
        } else if let Some(control) = app.edits.get(&id) {
            set_control_visible(*control, false);
        }
    }
    put(app, status, Rect::new(0, 0, 0, 0), false);
    #[cfg(feature = "diagnostics")]
    crate::diagnostics::record(
        "rebuild.controls",
        app.ui.view.title(),
        controls_started.elapsed(),
    );
    status_text(app);
    #[cfg(feature = "diagnostics")]
    let artwork_started = std::time::Instant::now();
    request_art(app, h);
    trim_artwork(app, h);
    #[cfg(feature = "diagnostics")]
    crate::diagnostics::record(
        "rebuild.artwork",
        app.ui.view.title(),
        artwork_started.elapsed(),
    );
    update_background(hwnd, app);
    InvalidateRect(hwnd, null(), 0);
}
fn content_bottom(app: &App, height: i32) -> i32 {
    app.scene
        .panels
        .iter()
        .rev()
        .find(|panel| panel.kind == 21)
        .map(|panel| panel.rect.y - 16)
        .unwrap_or(height - 124)
}
// Set DEEZNATS_FULL_SCROLL_REPAINT=1 to restore the original full-window path.
fn scroll_repaint_enabled() -> bool {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var_os("DEEZNATS_FULL_SCROLL_REPAINT").is_none())
}
fn scroll_repaint_rect(app: &App, width: i32, height: i32, caption: i32) -> RECT {
    RECT {
        left: app
            .px(if app.ui.sidebar_collapsed { 70 } else { 246 })
            .min(width),
        top: caption + app.px(app.scene.content_top),
        right: width,
        bottom: caption + app.px(content_bottom(app, height)),
    }
}
fn scroll_repaint_avoids_search(app: &App, region: &RECT, caption: i32) -> bool {
    app.scene.search.is_none_or(|search| {
        let search = app.rect(search);
        search.bottom + caption <= region.top
            || search.top + caption >= region.bottom
            || search.right <= region.left
            || search.left >= region.right
    })
}
fn canvas_track_button(app: &App, index: usize) -> bool {
    index < app.scene.buttons.len()
}

fn scene_button_enabled(app: &App, index: usize) -> bool {
    app.scene.buttons.get(index).is_some_and(|node| {
        node.enabled
            && app
                .scene
                .auth_modal
                .is_none_or(|(_, _, first)| index >= first)
            && !(matches!(node.action, Action::Connect | Action::ConnectSaved)
                && app.pending.is_some())
    })
}

fn canvas_action_key(action: &Action) -> String {
    match action {
        Action::OpenItem(item) => format!("open:{}:{}", item.kind, item.id),
        Action::ToggleCheck(id) => format!("check:{id}"),
        Action::Sort(column) => format!("sort:{column}"),
        _ => format!("other:{action:?}"),
    }
}

fn canvas_action_matches(key: &str, action: &Action) -> bool {
    match action {
        Action::OpenItem(item) => key
            .strip_prefix("open:")
            .and_then(|value| value.split_once(':'))
            .is_some_and(|(kind, id)| kind == item.kind && id == item.id),
        Action::ToggleCheck(id) => key.strip_prefix("check:") == Some(id.as_str()),
        Action::Sort(column) => {
            key.strip_prefix("sort:")
                .and_then(|value| value.parse::<usize>().ok())
                == Some(*column)
        }
        _ => canvas_action_key(action) == key,
    }
}

fn rect_contains(rect: Rect, x: i32, y: i32) -> bool {
    x >= rect.x && y >= rect.y && x < rect.x + rect.w && y < rect.y + rect.h
}

fn track_row_rect(node: &ui::Button, w: i32, queue_w: i32, collapsed: bool, lyrics: bool) -> Rect {
    let base_x = (node.rect.x - 48).max(if collapsed { 74 } else { 260 });
    let cw = w - base_x - queue_w - if lyrics { 32 } else { 40 };
    Rect::new(base_x + 2, node.rect.y - 2, (cw - 4).max(100), 52)
}

#[cfg(test)]
#[test]
fn track_row_hit_area_includes_columns_beyond_title() {
    let node = ui::Button {
        rect: Rect::new(320, 104, 200, 48),
        ..Default::default()
    };
    let row = track_row_rect(&node, 1200, 280, false, false);
    assert!(rect_contains(row, 750, 120));
    assert!(!rect_contains(node.rect, 750, 120));
    assert!(!rect_contains(row, 950, 120));
    assert!(!rect_contains(row, 750, 160));
}

fn canvas_track_hit(app: &App, x: i32, y: i32, w: i32, h: i32) -> Option<usize> {
    let hit = |&index: &usize| {
        let node = &app.scene.buttons[index];
        let hit_rect = if is_track_row(node) {
            let queue_w = if app.ui.right_sidebar_open() {
                280.min(w / 3)
            } else {
                0
            };
            track_row_rect(
                node,
                w,
                queue_w,
                app.ui.sidebar_collapsed,
                app.ui.view == View::Lyrics,
            )
        } else {
            node.rect
        };
        canvas_track_button(app, index)
            && scene_button_enabled(app, index)
            && rect_contains(hit_rect, x, y)
            && (!(app.scene.content_buttons.contains(&index)||matches!(node.action,Action::QueueJump(_)))
                || rect_contains(button_viewport(app, node, w, h), x, y))
    };
    // Seek regions are invisible and laid out after the floating lyrics controls.
    // Their insertion order must never put them above an actual control.
    if app.ui.view==View::Lyrics {
        if let Some(index)=(0..app.scene.buttons.len()).rev()
            .filter(|&i|!matches!(app.scene.buttons[i].action,Action::SeekMs(_)))
            .find(hit) {return Some(index);}
    }
    (0..app.scene.buttons.len()).rev().find(hit)
}

fn button_viewport(app: &App, node: &ui::Button, w: i32, h: i32) -> Rect {
    if matches!(node.action,Action::QueueJump(_)) {if let Some(mut viewport)=app.scene.queue_viewport{if gpu::is_recording(gpu::Layer::Queue){viewport.y-=144;viewport.h+=288;}return viewport;}}
    if app.ui.view==View::Lyrics&&matches!(node.action,Action::SeekMs(_)) {
        return app.scene.lyrics_area.unwrap_or(Rect::new(20,0,(w-40).max(1),(h-64).max(0)));
    }
    let left = if app.ui.view == View::Lyrics {
        20
    } else if app.ui.sidebar_collapsed {
        70
    } else {
        246
    };
    let top = if app.ui.view == View::Lyrics {
        0
    } else {
        app.scene.content_top
    };
    let height = if app.ui.view == View::Lyrics {
        h
    } else {
        content_bottom(app, h) - app.scene.content_top
    };
    let queue_w = if app.ui.right_sidebar_open() {
        280.min(w / 3)
    } else {
        0
    };
    let right = if queue_w > 0 {
        w - queue_w - 16
    } else {
        w - 20
    };
    let margin=if gpu::is_recording(gpu::Layer::Main){ui::SCROLL_OVERSCAN}else{0};
    let default = Rect::new(left, top-margin, (right - left).max(100), height+2*margin);
    if matches!(
        node.action,
        Action::Genre(_) | Action::FlowMoreGenres | Action::FlowAddGenre
    ) {
        app.scene.genre_viewport.unwrap_or(default)
    } else {
        default
    }
}
unsafe fn show_link_copied(hwnd: HWND, app: &App) {
    const TOAST: i32 = 509;
    let old = GetDlgItem(hwnd, TOAST);
    if !old.is_null() { DestroyWindow(old); }
    let mut bounds: RECT = zeroed();
    GetClientRect(hwnd, &mut bounds);
    let width = app.px(180);
    let toast = CreateWindowExW(WS_EX_CLIENTEDGE, wide("STATIC").as_ptr(), wide(app.ui.t("Link copied")).as_ptr(),
        WS_CHILD | WS_VISIBLE | 0x0001 | 0x0200, (bounds.right - width) / 2,
        app.px(62), width, app.px(38), hwnd, TOAST as HMENU, GetModuleHandleW(null()), null());
    if !toast.is_null() {
        SendMessageW(toast, WM_SETFONT, app.fonts[0] as usize, 1);
        SetWindowPos(toast, HWND_TOP, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
        SetTimer(hwnd, 91, 2200, None);
    }
}
unsafe fn status_text(app: &App) {
    let message = if app.notice.is_empty() {
        &app.snapshot.message
    } else {
        &app.notice
    };
    let message = app.ui.t(message);
    if read(app.edits[&STATUS]) != message {
        SetWindowTextW(app.edits[&STATUS], wide(message).as_ptr());
    }
}
unsafe fn command(app: &mut App, command: Command) -> bool {
    if app.worker.commands.try_send(command).is_err() {
        app.notice = "Player is busy. Try again shortly.".into();
        status_text(app);
        false
    } else {
        true
    }
}

unsafe fn request_update(app: &mut App, request: crate::updater::Request) {
    if app.ui.update.busy() {
        return;
    }
    let phase = match request {
        crate::updater::Request::Check => crate::updater::Phase::Checking,
        crate::updater::Request::Download => crate::updater::Phase::Downloading,
        crate::updater::Request::Apply => crate::updater::Phase::Applying,
    };
    let checking = matches!(request, crate::updater::Request::Check);
    if app.updater.commands.try_send(request).is_ok() {
        app.ui.update.phase = phase;
        app.ui.update.error.clear();
        if checking {
            app.ui.last_update_check = crate::updater::now();
            app.ui.save_preferences();
        }
    }
}

unsafe fn process_updates(hwnd: HWND, app: &mut App) -> bool {
    let mut changed = false;
    while let Ok(event) = app.updater.events.try_recv() {
        match event {
            crate::updater::Event::State(state) => {
                app.ui.update = state;
                changed = true;
            }
            crate::updater::Event::Exit => {
                // This explicit restart bypasses keep-playing-on-close/tray behavior.
                DestroyWindow(hwnd);
                return true;
            }
        }
    }
    if changed && app.ui.view == View::Settings && app.ui.tab == 0 {
        rebuild(hwnd, app);
    }
    #[cfg(not(test))]
    if app.ui.auto_updates
        && crate::updater::now().saturating_sub(app.ui.last_update_check) >= 86400
        && matches!(
            app.ui.update.phase,
            crate::updater::Phase::Idle
                | crate::updater::Phase::Current
                | crate::updater::Phase::Failed
        )
    {
        request_update(app, crate::updater::Request::Check);
    }
    false
}

unsafe fn restore_shortcuts(app: &mut App) {
    if let Some(shortcuts) = &mut app.shortcuts {
        for (index, binding) in app.ui.shortcuts.iter().enumerate() {
            app.ui.shortcut_errors[index] =
                shortcuts.set(index, *binding).err().unwrap_or_default();
        }
    }
}

unsafe fn capture_shortcut(hwnd: HWND, app: &mut App, key: u32) {
    let Some(index) = app.ui.shortcut_capture else {
        return;
    };
    if key == 27 {
        app.ui.shortcut_capture = None;
        restore_shortcuts(app);
    } else if !matches!(key, 0x10..=0x12 | 0x5b | 0x5c | 0xa0..=0xa5) {
        let binding = if key == 8 {
            None
        } else {
            Some(crate::shortcuts::current_binding(key))
        };
        let duplicate = binding.is_some()
            && app
                .ui
                .shortcuts
                .iter()
                .enumerate()
                .any(|(other, value)| other != index && *value == binding);
        let result = if duplicate {
            Err("That shortcut is already assigned to another action.".into())
        } else if let Some(shortcuts) = &mut app.shortcuts {
            shortcuts.set(index, binding)
        } else {
            Err("Keyboard shortcuts are unavailable.".into())
        };
        match result {
            Ok(()) => {
                app.ui.shortcuts[index] = binding;
                app.ui.shortcut_capture = None;
                restore_shortcuts(app);
                app.ui.save_preferences();
            }
            Err(error) => app.ui.shortcut_errors[index] = error,
        }
    }
    rebuild(hwnd, app);
}

unsafe fn shortcut_action(hwnd: HWND, app: &mut App, action: usize) {
    match action {
        0 => {
            command(app, Command::Toggle);
        }
        1 => {
            command(app, Command::Previous);
        }
        2 => {
            command(app, Command::Next);
        }
        3 => {
            command(app, Command::Stop);
        }
        4..=6 => {
            let volume = match action {
                4 => (app.volume + 5).min(100),
                5 => app.volume.saturating_sub(5),
                _ => {
                    if app.volume == 0 {
                        app.ui.unmuted_volume.clamp(1, 100)
                    } else {
                        0
                    }
                }
            };
            if command(app, Command::Volume(volume)) {
                if volume == 0 && app.volume > 0 {
                    app.ui.unmuted_volume = app.volume;
                }
                app.volume = volume;
                app.ui.initial_volume = volume;
                SendMessageW(app.edits[&VOLUME], TBM_SETPOS, 1, volume as isize);
                SetTimer(hwnd, 3, 300, None);
                rebuild(hwnd, app);
            }
        }
        _ => {}
    }
}
unsafe fn read(hwnd: HWND) -> String {
    let length = GetWindowTextLengthW(hwnd).max(0) as usize;
    let mut text = vec![0u16; length + 1];
    GetWindowTextW(hwnd, text.as_mut_ptr(), text.len() as i32);
    String::from_utf16_lossy(&text[..length])
}
unsafe fn login(hwnd: HWND, app: &mut App) {
    if app.pending.is_some() {
        return;
    }
    let secret = read(app.edits[&SESSION]);
    if app.ui.provider == crate::catalog::Provider::YouTube {
        command(
            app,
            if secret.trim().is_empty() {
                Command::LoginSaved
            } else {
                Command::Login(secret.trim().to_owned())
            },
        );
        SetWindowTextW(app.edits[&SESSION], wide("").as_ptr());
        return;
    }
    let command_value = if secret.trim().is_empty() {
        Command::LoginSaved
    } else {
        if secret.trim().len() < 20 || secret.chars().any(char::is_whitespace) {
            app.notice = "Paste a valid ARL, or clear the field to use your saved session.".into();
            status_text(app);
            return;
        }
        Command::Login(secret)
    };
    if command(app, command_value) {
        app.pending = Some(app.snapshot.login_attempt + 1);
        if app.ui.deezer_login_open {
            app.ui.deezer_login_busy = true;
            app.ui.deezer_login_status = "Connecting to Deezer...".into();
        }
        app.notice = "Connecting to Deezer...".into();
        SetWindowTextW(app.edits[&SESSION], wide("").as_ptr());
    }
    rebuild(hwnd, app);
}

unsafe fn capture_playlist_fields(app: &mut App) {
    app.ui.playlist_draft.title = read(app.edits[&NAME]);
    let description = read(app.edits[&DESCRIPTION]);
    app.ui.playlist_draft.description_changed |= description != app.ui.playlist_draft.description;
    app.ui.playlist_draft.description = description;
}
unsafe fn set_playlist_fields(app: &App) {
    SetWindowTextW(
        app.edits[&NAME],
        wide(&app.ui.playlist_draft.title).as_ptr(),
    );
    SetWindowTextW(
        app.edits[&DESCRIPTION],
        wide(&app.ui.playlist_draft.description).as_ptr(),
    );
}
unsafe fn submit_playlist(app: &mut App, mutation: crate::playlist::Mutation) {
    if app.ui.playlist_busy {
        app.notice = "A playlist update is still in progress".into();
        return;
    }
    if !app.snapshot.connected {
        app.notice = format!("Connect to {} first", app.ui.provider.label());
        return;
    }
    if let crate::playlist::Mutation::Save { title, .. } = &mutation {
        if title.trim().is_empty() {
            app.notice = "Enter a playlist name".into();
            return;
        }
    }
    let request = crate::catalog::Request {
        #[cfg(feature = "diagnostics")]
        view_tag: app.ui.view as usize + 1,
        generation: app.generation,
        provider: app.ui.provider,
        engine: app.snapshot.engine.clone(),
        job: crate::catalog::Job::Playlist(mutation),
    };
    if app.catalog.requests.send(request).is_ok() {
        app.playlist_generation = Some(app.generation);
        app.ui.playlist_busy = true;
        app.notice = "Updating playlist...".into();
    } else {
        app.notice = "Catalog worker is unavailable".into();
    }
}
fn update_playlist_preview(app: &mut App) {
    match app
        .ui
        .playlist_draft
        .jpeg()
        .and_then(|bytes| crate::catalog::decode(&bytes, 500))
    {
        Ok(bitmap) => {
            static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
            let key = format!(
                "playlist-cover:{}",
                NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            );
            if app.ui.playlist_draft.preview.starts_with("playlist-cover:") {
                app.images.remove(&app.ui.playlist_draft.preview);
            }
            app.images.insert(key.clone(), Arc::new(bitmap));
            app.ui.playlist_draft.preview = key;
            app.notice = "Cover ready. Save the playlist to upload it.".into();
        }
        Err(e) => app.notice = e,
    }
}
unsafe fn playlist_export_path(hwnd: HWND) -> Option<PathBuf> {
    let mut path = vec![0u16; 32768];
    let name = wide("playlist-cover.jpg");
    path[..name.len()].copy_from_slice(&name);
    let filter = wide("JPEG image\0*.jpg\0");
    let extension = wide("jpg");
    let mut dialog: OPENFILENAMEW = zeroed();
    dialog.lStructSize = size_of::<OPENFILENAMEW>() as u32;
    dialog.hwndOwner = hwnd;
    dialog.lpstrFilter = filter.as_ptr();
    dialog.lpstrDefExt = extension.as_ptr();
    dialog.lpstrFile = path.as_mut_ptr();
    dialog.nMaxFile = path.len() as u32;
    dialog.Flags = OFN_OVERWRITEPROMPT | OFN_PATHMUSTEXIST | OFN_NOCHANGEDIR;
    if GetSaveFileNameW(&mut dialog) == 0 {
        return None;
    }
    Some(PathBuf::from(String::from_utf16_lossy(
        &path[..path.iter().position(|v| *v == 0).unwrap_or(0)],
    )))
}

unsafe fn data_request(app: &mut App, job: crate::catalog::Job) {
    if app.ui.provider != crate::catalog::Provider::Deezer || app.snapshot.engine.is_some() {
        let lyrics = matches!(
            job,
            crate::catalog::Job::Lyrics(..) | crate::catalog::Job::LyricsWindow(..)
        );
        let request = crate::catalog::Request {
            #[cfg(feature = "diagnostics")]
            view_tag: app.ui.view as usize + 1,
            generation: app.generation,
            provider: app.ui.provider,
            engine: app.snapshot.engine.clone(),
            job,
        };
        let failed = if lyrics {
            app.catalog.lyrics_requests.try_send(request).is_err()
        } else {
            app.catalog.requests.send(request).is_err()
        };
        if failed && !lyrics {
            app.notice = "Catalog worker is unavailable.".into();
            if let Some(page) = app.ui.page.as_mut() {
                let page = Arc::make_mut(page);
                page.loading = false;
                page.error = app.notice.clone();
            }
        }
    } else {
        app.notice = format!(
            "Connect your {} account to load this page.",
            app.ui.provider.label()
        );
    }
}
fn adjacent_lyric_tracks(
    queue: &[crate::catalog::Entry],
    index: usize,
) -> Vec<crate::catalog::Entry> {
    // Next first: the previous track will usually already be cached.
    [index.checked_add(1), index.checked_sub(1)]
        .into_iter()
        .flatten()
        .filter_map(|i| queue.get(i))
        .filter(|e| e.kind == "track")
        .cloned()
        .collect()
}
fn invalidate_flow(app: &mut App) {
    let key = serde_json::json!([
        provider_account_key(app),
        "Flow",
        "",
        "",
        "",
        app.ui.flow_id
    ])
    .to_string();
    app.catalog.session.refresh_flow(&key);
    app.page_cache.retain(|(k, _)| *k != key);
}
unsafe fn switch_provider(app: &mut App, provider: crate::catalog::Provider) {
    if provider != app.ui.provider {
        if let Some((cancel, _)) = app.spotify_auth.take() {
            cancel.store(true, std::sync::atomic::Ordering::Relaxed);
        }
        app.ui.spotify_catalog_busy = false;
    }
    if let Some((cancel, _)) = app.deezer_auth.take() {
        cancel.store(true, std::sync::atomic::Ordering::Relaxed);
    }
    app.ui.deezer_login_open = false;
    app.ui.deezer_login_busy = false;
    app.pending = None;
    app.ui.switch_flow_provider(provider);
    app.ui.provider = provider;
    app.ui.last_provider = Some(provider);
    app.ui.save_preferences();
    app.ui.device_menu_open = false;
    app.ui.spotify_devices.clear();
    app.ui.spotify_devices_busy = false;
    app.ui.spotify_devices_status.clear();
    app.spotify_device_request = None;
    provider.set_active();
    app.spotify_jam_request = None;
    app.ui.spotify_jam = None;
    app.ui.spotify_jam_busy = false;
    app.ui.spotify_jam_status.clear();
    app.generation = app.generation.wrapping_add(1);
    app.catalog.set_generation(app.generation);
    app.catalog.session.invalidate();
    app.page_cache.clear();
    app.navigation.clear();
    app.ui.history.clear();
    app.entity = None;
    app.query.clear();
    app.ui.page = None;
    app.ui.track_info = None;
    app.ui.liked_ids.clear();
    app.podcast_ids.clear();
    app.library_loaded = false;
    app.library_pending = None;
    app.ui.playlist_busy = false;
    app.playlist_generation = None;
    app.ui.playlist_track = None;
    app.ui.playlist_draft = Default::default();
    app.flow_save_expected = None;
    app.ui.flow_loaded = false;
    app.ui.genres.clear();
    app.ui.flow_options.clear();
    app.ui.mix_genres = None;
    app.ui.queue.clear();
    app.ui.queue_index = 0;
    app.ui.now = Default::default();
    app.ui.account = serde_json::Value::Null;
    app.ui.lyrics.clear();
    app.ui.rich_lyrics.clear();
    app.ui.lyrics_provider.clear();
    app.lyrics_id.clear();
    app.ui.view = View::Home;
    app.ui.scroll = 0;
    app.snapshot.connected = false;
    app.snapshot.account = serde_json::Value::Null;
    app.snapshot.now = Default::default();
    app.snapshot.queue = Arc::new(Vec::new());
    app.snapshot.position = 0;
    app.playback_clock = PlaybackClock::default();
    app.snapshot.duration = 0;
    SetWindowTextW(app.edits[&SESSION], wide("").as_ptr());
    SendMessageW(
        app.edits[&SESSION],
        EM_SETPASSWORDCHAR,
        if provider == crate::catalog::Provider::Spotify {
            0
        } else {
            0x25cf
        },
        0,
    );
    SendMessageW(
        app.edits[&SESSION],
        EM_SETCUEBANNER,
        1,
        wide(if provider == crate::catalog::Provider::Spotify {
            "Public Spotify application Client ID"
        } else if provider == crate::catalog::Provider::YouTube {
            "YouTube Music Cookie header (stored securely on this PC)"
        } else {
            "Paste ARL, or leave empty to use saved session"
        })
        .as_ptr() as isize,
    );
    command(app, Command::SwitchProvider(provider));
if provider == crate::catalog::Provider::YouTube {
    command(app, Command::Volume(app.volume));
    let eq = serde_json::json!({"enabled": app.ui.value("Equalizer") != 0,
        "gainsDb": &app.ui.eq[..10], "preampDb": app.ui.eq[10]});
    command(app, Command::Api("DZSetEQJSON".into(), vec![serde_json::json!(eq.to_string())]));
}
    if provider == crate::catalog::Provider::Spotify && app.ui.spotify_configured {
        start_spotify(app);
    }
    load_page(app);
}
fn provider_account_key(app: &App) -> String {
    format!(
        "{}:{}",
        app.ui.provider.label(),
        crate::catalog::string(&app.snapshot.account, &["userId", "id"])
    )
}
fn page_key(app: &App) -> String {
    serde_json::json!([
        provider_account_key(app),
        format!("{:?}", app.ui.view),
        app.entity
            .as_ref()
            .map(|entry| entry.kind.as_str())
            .unwrap_or(""),
        app.entity
            .as_ref()
            .map(|entry| entry.id.as_str())
            .unwrap_or(""),
        app.query,
        if app.ui.view == View::Flow {
            app.ui.flow_id.as_str()
        } else {
            ""
        }
    ])
    .to_string()
}
unsafe fn load_page(app: &mut App) {
    if !app.ui.provider.supports_downloads() && matches!(app.ui.view, View::Offline) {
        app.ui.view = View::Home;
    }
    if app.ui.view == View::Podcasts && !app.snapshot.connected {
        app.generation += 1;
        app.catalog.set_generation(app.generation);

        app.ui.page = Some(Arc::new(crate::catalog::podcast_library_page(
            Vec::new(),
            Vec::new(),
            Vec::new(),
            app.ui.offline.clone(),
        )));
        return;
    }
    if app.ui.view == View::Offline {
        app.generation += 1;
        app.catalog.set_generation(app.generation);

        app.ui.page = Some(Arc::new(crate::downloads::library_page(
            &app.ui.offline,
            app.entity.as_ref(),
        )));
        return;
    }
    if !matches!(
        app.ui.view,
        View::Home
            | View::Explore
            | View::Flow
            | View::StyleMix
            | View::Podcasts
            | View::Favorites
            | View::Albums
            | View::Playlists
            | View::PlaylistPicker
            | View::Recent
            | View::Queue
            | View::Album
            | View::Artist
            | View::Show
    ) {
        return;
    }
    app.generation += 1;
    app.catalog.set_generation(app.generation);

    let key = page_key(app);
    let cached = app
        .page_cache
        .iter()
        .find(|(k, _)| *k == key)
        .map(|(_, page)| page.clone());
    app.catalog
        .session
        .begin(app.generation, key, cached.clone());
    if let Some(page) = cached {
        let mut page = (*page).clone();
        page.loading = app.snapshot.connected;
        page.error.clear();
        app.ui.page = Some(Arc::new(page));
        app.notice.clear();
    } else {
        app.ui.page = Some(Arc::new(crate::catalog::Page {
            title: app
                .entity
                .as_ref()
                .map(|e| e.title.clone())
                .unwrap_or_else(|| app.ui.view.title().into()),
            entity: app.entity.clone(),
            loading: app.snapshot.connected,
            error: if app.snapshot.connected {
                String::new()
            } else {
                format!("Sign in to load your {} library.", app.ui.provider.label())
            },
            ..Default::default()
        }));
    }
    data_request(
        app,
        crate::catalog::Job::Route(
            app.ui.view,
            app.entity.clone(),
            if app.ui.view == View::Flow {
                app.ui.flow_id.clone()
            } else {
                app.query.clone()
            },
        ),
    );
    if app.ui.view == View::StyleMix
        && app.ui.mix_region != usize::MAX
        && app.ui.mix_genres.is_none()
        && app.query.is_empty()
        && app.snapshot.connected
    {
        let name = if app.ui.provider == crate::catalog::Provider::Spotify {
            "SpotifyStyleMixGenres"
        } else {
            "DeezerStyleMixGenres"
        };
        data_request(
            app,
            crate::catalog::Job::Api(name.into(), vec![serde_json::json!(app.ui.mix_region)]),
        );
    }
}
unsafe fn queue_scroll_pointer(hwnd: HWND, app: &mut App, y: i32) {
    app.queue_glide = None;
    if let Some(track) = app.scene.queue_scrollbar {
        let thumb = app
            .scene
            .panels
            .iter()
            .find(|p| p.kind == 17)
            .map(|p| p.rect.h)
            .unwrap_or(24);
        let span = (track.h - thumb).max(1);
        let offset = ((y - track.y - thumb / 2).clamp(0, span) as usize
            * app.scene.queue_scroll_max
            / span as usize)
            .min(app.scene.queue_scroll_max);
        if app.scene.device_sidebar.is_some() {
            app.ui.device_sidebar_scroll = offset as i32;
        } else {
            set_queue_scroll(hwnd,app,(offset*24).min(i32::MAX as usize) as i32);
            return;
        }
        rebuild(hwnd, app);
    }
}

unsafe fn set_queue_scroll(hwnd: HWND, app: &mut App, pixels: i32) {
    let offset = pixels.clamp(
        0,
        app.scene
            .queue_scroll_max
            .saturating_mul(24)
            .min(i32::MAX as usize) as i32,
    ) as usize;
    if app.ui.queue_scroll * 24 + app.ui.queue_scroll_offset != offset {
        let previous=app.ui.queue_scroll*24+app.ui.queue_scroll_offset;
        reset_marquees();
        app.ui.queue_scroll = offset / 24;
        app.ui.queue_scroll_offset = offset % 24;
        let focus=app.canvas_focus.and_then(|i|app.scene.buttons.get(i)).map(|b|canvas_action_key(&b.action));
        let (_,h)=size(hwnd,app);
        let queue_origin=app.scene.queue_layout_scroll;
        if ui::refresh_queue_sidebar(&mut app.scene,&app.ui,h) {
            if app.scene.queue_layout_scroll!=queue_origin {gpu::invalidate_queue_layer();}
            app.canvas_focus=focus.and_then(|key|app.scene.buttons.iter().position(|b|canvas_action_matches(&key,&b.action)));
            app.canvas_hover=None;app.canvas_pressed=None;
            app.buttons.resize(app.scene.buttons.len(),null_mut());
            app.button_text.resize(app.scene.buttons.len(),String::new());
            if previous/72!=offset/72 {request_art(app,h);}
            if let Some(frame)=app.scene.queue_frame {
                let mut rect=app.rect(frame);let caption=caption_height(hwnd);rect.top+=caption;rect.bottom+=caption;
                InvalidateRect(hwnd,&rect,0);
            }
        } else {rebuild(hwnd,app);}
    }
}
const OFFSCREEN_ARTWORK_BYTES: usize = 16 * 1024 * 1024;
const OFFSCREEN_ARTWORK_HIGH_WATER:usize=24*1024*1024;
const RESIDENT_ARTWORK_BYTES: usize = 64 * 1024 * 1024;
const MINIMIZED_CACHE_DELAY_MS: u32 = 60_000;

fn trim_inactive_pages(pages: &mut VecDeque<(String, Arc<crate::catalog::Page>)>, current: &str) {
    let mut recent = pages.iter().filter(|(key, _)| key != current).count();
    while recent > 2 {
        if let Some(index) = pages.iter().position(|(key, _)| key != current) {
            pages.remove(index);
            recent -= 1;
        } else {
            break;
        }
    }
}

fn trim_artwork_entries(
    images: &mut HashMap<String, Arc<crate::catalog::Bitmap>>,
    order: &mut VecDeque<String>,
    protected: &HashSet<String>,
) -> Vec<(String, u64)> {
    trim_artwork_recent(images,order,protected,&HashSet::new(),false)
}

fn trim_artwork_recent(images:&mut HashMap<String,Arc<crate::catalog::Bitmap>>,order:&mut VecDeque<String>,protected:&HashSet<String>,recent:&HashSet<u64>,scrolling:bool)->Vec<(String,u64)> {
    let mut total=images.values().map(|i|i.pixels.len()*4).sum::<usize>();
    let mut offscreen=images.iter().filter(|(key,_)|!protected.contains(*key)).map(|(_,i)|i.pixels.len()*4).sum::<usize>();
    let soft=!scrolling&&offscreen>OFFSCREEN_ARTWORK_HIGH_WATER;
    let mut removed=Vec::new();
    while total>RESIDENT_ARTWORK_BYTES||(soft&&offscreen>OFFSCREEN_ARTWORK_BYTES){
        let hard=total>RESIDENT_ARTWORK_BYTES;
        let Some(index)=order.iter().position(|key|!protected.contains(key)&&images.get(key).is_some_and(|i|hard||!recent.contains(&i.id)))else{break;};
        let key=order.remove(index).unwrap();
        if let Some(image)=images.remove(&key){let bytes=image.pixels.len()*4;total=total.saturating_sub(bytes);offscreen=offscreen.saturating_sub(bytes);removed.push((key,image.id));}
    }
    removed
}

fn trim_artwork_entries_to(
    images: &mut HashMap<String, Arc<crate::catalog::Bitmap>>,
    order: &mut VecDeque<String>,
    protected: &HashSet<String>,
    offscreen_limit: usize,
) -> Vec<(String, u64)> {
    let mut total = 0usize;
    let mut offscreen = 0usize;
    for (key, image) in images.iter() {
        let bytes = image.pixels.len().saturating_mul(4);
        total = total.saturating_add(bytes);
        if !protected.contains(key) {
            offscreen = offscreen.saturating_add(bytes);
        }
    }
    let mut removed = Vec::new();
    while total > RESIDENT_ARTWORK_BYTES || offscreen > offscreen_limit {
        let Some(index) = order.iter().position(|key| !protected.contains(key)) else {
            break;
        };
        let key = order.remove(index).unwrap();
        if let Some(image) = images.remove(&key) {
            let bytes = image.pixels.len().saturating_mul(4);
            total = total.saturating_sub(bytes);
            offscreen = offscreen.saturating_sub(bytes);
            removed.push((key, image.id));
        }
    }
    removed
}

fn trim_minimized_artwork(app: &mut App) {
    let mut protected = HashSet::new();
    for entry in app
        .ui
        .queue
        .iter()
        .skip(app.ui.queue_index.saturating_sub(2))
        .take(8)
        .chain(std::iter::once(&app.ui.now))
    {
        protected.insert(entry.artwork.clone());
    }
    protected.insert(format!("background:{}", app.ui.now.artwork));
    protected.insert(crate::catalog::cover_artwork_key(&app.ui.now.artwork));
    protected.insert(format!(
        "blur:cover:{}",
        crate::catalog::cover_artwork_key(&app.ui.now.artwork)
    ));
    protected.insert(app.custom_background.clone());
    protected.insert(app.ui.playlist_draft.preview.clone());
    protected.extend(
        app.scene
            .buttons
            .iter()
            .map(|button| button.artwork.clone()),
    );
    protected.extend(app.scene.pictures.iter().map(|(_, url)| url.clone()));
    protected.extend(app.scene.artwork_ahead.iter().map(|(_, url)| url.clone()));
    let removed = trim_artwork_entries_to(
        &mut app.images,
        &mut app.image_order,
        &protected,
        2 * 1024 * 1024,
    );
    let mut retired: HashSet<u64> = removed.iter().map(|(_, id)| *id).collect();
    for image in app
        .images
        .values()
        .chain(app.previous_cover.iter())
        .chain(app.background.iter())
        .chain(app.previous_background.iter())
    {
        retired.remove(&image.id);
    }
    crate::drawing::evict_resized_artwork(&retired);
    for (key, _) in removed {
        app.image_pending.remove(&key);
        app.image_times.remove(&key);
    }
    app.artwork.shrink(app.images.values().map(|i|i.id).collect());
}

fn trim_artwork(app: &mut App, height: i32) {
    let top = app.scene.content_top;
    let bottom = content_bottom(app, height);
    let ahead = (bottom - top).max(240);
    let mut protected = HashSet::new();
    let mut keep = |url: &str| {
        if !url.is_empty() {
            protected.insert(url.to_owned());
        }
    };
    for entry in app
        .ui
        .queue
        .iter()
        .skip(app.ui.queue_index.saturating_sub(2))
        .take(8)
        .chain(std::iter::once(&app.ui.now))
    {
        keep(&entry.artwork);
    }
    keep(&format!("background:{}", app.ui.now.artwork));
    keep(&crate::catalog::cover_artwork_key(&app.ui.now.artwork));
    keep(&format!(
        "blur:cover:{}",
        crate::catalog::cover_artwork_key(&app.ui.now.artwork)
    ));
    keep(&app.custom_background);
    keep(&app.ui.playlist_draft.preview);
    for button in &app.scene.buttons {
        if button.rect.y + button.rect.h > top - ahead && button.rect.y < bottom + ahead {
            keep(&button.artwork);
        }
    }
    for (rect, url) in &app.scene.pictures {
        if rect.y + rect.h > top - ahead && rect.y < bottom + ahead {
            keep(url);
        }
    }
    for (y, url) in &app.scene.artwork_ahead {
        if *y >= top - ahead && *y < bottom + ahead {
            keep(url);
        }
    }
    let before = artwork_metrics_path().map(|_| {
        let total = app
            .images
            .values()
            .map(|image| image.pixels.len().saturating_mul(4))
            .sum();
        let offscreen = app
            .images
            .iter()
            .filter(|(key, _)| !protected.contains(*key))
            .map(|(_, image)| image.pixels.len().saturating_mul(4))
            .sum();
        (total, offscreen, crate::drawing::resized_artwork_stats())
    });
    let recent=app.artwork.recent_ids();
    let scrolling=main_scroll_animating(app)||app.queue_glide.is_some()||app.scrolling||app.queue_scroll_drag;
    if scrolling {unsafe{let parent=GetParent(app.canvas_keyboard);if !parent.is_null(){SetTimer(parent,10,5100,None);}}}
    let removed = trim_artwork_recent(&mut app.images, &mut app.image_order, &protected,&recent,scrolling);
    if !removed.is_empty() {
        let mut retired: HashSet<u64> = removed.iter().map(|(_, id)| *id).collect();
        for image in app
            .images
            .values()
            .chain(app.previous_cover.iter())
            .chain(app.background.iter())
            .chain(app.previous_background.iter())
        {
            retired.remove(&image.id);
        }
        crate::drawing::evict_resized_artwork(&retired);
        for (key, _) in &removed {
            app.image_pending.remove(key);
            app.image_times.remove(key);
        }
    }
    if let Some(before) = before {
        artwork_metrics(app, &protected, before, removed.len());
    }
    let used=protected.iter().filter_map(|url|app.images.get(url).map(|i|i.id)).collect();
    app.artwork.residency(app.images.values().map(|i|i.id).collect(),&used);
}

fn artwork_metrics_path() -> Option<&'static std::ffi::OsStr> {
    static PATH: std::sync::OnceLock<Option<std::ffi::OsString>> = std::sync::OnceLock::new();
    PATH.get_or_init(|| std::env::var_os("DEEZNATS_ARTWORK_METRICS"))
        .as_deref()
}

fn render_memory_metrics(app: &App) {
    let Some(path) = artwork_metrics_path() else {
        return;
    };
    thread_local! { static LAST: std::cell::Cell<Option<std::time::Instant>> = const { std::cell::Cell::new(None) }; }
    let now = std::time::Instant::now();
    if LAST.with(|last| {
        if last
            .get()
            .is_some_and(|previous| now.duration_since(previous).as_secs_f32() < 1.0)
        {
            true
        } else {
            last.set(Some(now));
            false
        }
    }) {
        return;
    }
    let (shadows, lyrics, blurred) = crate::drawing::lyric_raster_bytes();
    struct CountWriter(usize);
    impl std::io::Write for CountWriter {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0 = self.0.saturating_add(bytes.len());
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut page_bytes = CountWriter(0);
    for (_, page) in &app.page_cache {
        let _ = serde_json::to_writer(&mut page_bytes, page.as_ref());
    }
    if let Some(page) = app.ui.page.as_ref() {
        if !app
            .page_cache
            .iter()
            .any(|(_, cached)| Arc::ptr_eq(page, cached))
        {
            let _ = serde_json::to_writer(&mut page_bytes, page.as_ref());
        }
    }
    #[cfg(feature = "diagnostics")]
    let page_scoped_rust_live = Some(crate::memory_profile::page_live(app.ui.view as usize + 1));
    #[cfg(feature = "diagnostics")]
    let page_scoped_all: Option<Vec<usize>> =
        Some((0..256).map(crate::memory_profile::page_live).collect());
    #[cfg(feature = "diagnostics")]
    let measured_rust_live = Some(crate::memory_profile::current_live());
    #[cfg(not(feature = "diagnostics"))]
    let page_scoped_rust_live: Option<usize> = None;
    #[cfg(not(feature = "diagnostics"))]
    let page_scoped_all: Option<Vec<usize>> = None;
    #[cfg(not(feature = "diagnostics"))]
    let measured_rust_live: Option<usize> = None;
    let record = serde_json::json!({
        "kind": "render_memory", "view": format!("{:?}", app.ui.view),
        "page_scoped_rust_live_bytes": page_scoped_rust_live,
        "page_tag": app.ui.view as usize + 1, "page_scoped_all_live_bytes": page_scoped_all,
        "measured_rust_live_bytes": measured_rust_live,
        "time_unix_ms": std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis(),
        "lyric_shadow_bytes": shadows, "lyric_raster_bytes": lyrics,
        "lyric_blurred_bytes": blurred, "backdrop_bytes": app.backdrop_cache.borrow().1.capacity() * 4,
        "pages": app.page_cache.len(), "page_json_bytes": page_bytes.0,
        "resized_bytes": crate::drawing::resized_artwork_stats().1,
        "artwork_bytes": app.images.values().map(|image| image.pixels.len() * 4).sum::<usize>()
    });
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        use std::io::Write as _;
        let _ = writeln!(file, "{record}");
    }
}

fn artwork_metrics(
    app: &App,
    protected: &HashSet<String>,
    before: (usize, usize, (usize, usize)),
    evicted: usize,
) {
    thread_local! { static LAST: std::cell::Cell<Option<std::time::Instant>> = const { std::cell::Cell::new(None) }; }
    let Some(path) = artwork_metrics_path() else {
        return;
    };
    let now = std::time::Instant::now();
    if LAST.with(|last| {
        if evicted == 0
            && last
                .get()
                .is_some_and(|previous| now.duration_since(previous).as_secs_f32() < 1.0)
        {
            true
        } else {
            last.set(Some(now));
            false
        }
    }) {
        return;
    }
    let mut unique = HashSet::new();
    let unique_bytes: usize = app
        .images
        .values()
        .filter(|image| unique.insert(image.id))
        .map(|image| image.pixels.len().saturating_mul(4))
        .sum();
    let total_bytes: usize = app
        .images
        .values()
        .map(|image| image.pixels.len().saturating_mul(4))
        .sum();
    let offscreen_bytes: usize = app
        .images
        .iter()
        .filter(|(key, _)| !protected.contains(*key))
        .map(|(_, image)| image.pixels.len().saturating_mul(4))
        .sum();
    let resized = crate::drawing::resized_artwork_stats();
    let record = serde_json::json!({
        "time_unix_ms": std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis(),
        "view": format!("{:?}", app.ui.view), "entries": app.images.len(),
        "unique_images": unique.len(), "unique_bytes": unique_bytes, "accounted_bytes": total_bytes,
        "offscreen_bytes": offscreen_bytes, "before_bytes": before.0,
        "before_offscreen_bytes": before.1, "evicted_entries": evicted,
        "resized_entries": resized.0, "resized_bytes": resized.1,
        "before_resized_bytes": before.2.1
    });
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        use std::io::Write as _;
        let _ = writeln!(file, "{record}");
    }
}

unsafe fn request_art(app: &mut App, h: i32) {
    if app.render_suspended {
        return;
    }
    let engine = app.snapshot.engine.clone();
    let mut urls = Vec::new();
    for node in &app.scene.buttons {
        if !node.artwork.is_empty()
            && (matches!(node.action, Action::Go(View::Account))
                || (node.rect.y + node.rect.h > app.scene.content_top - 160
                    && node.rect.y < h + 36))
        {
            urls.push(node.artwork.clone());
        }
    }
    for (rect, url) in &app.scene.pictures {
        if rect.y + rect.h > 0 && rect.y < h + 36 {
            urls.push(url.clone());
        }
    }
    if !app.ui.now.artwork.is_empty() {
        urls.push(app.ui.now.artwork.clone());
    }
    let visible: HashSet<String> = urls.iter().cloned().collect();
    // Visible artwork is submitted first; speculative requests never create
    // extra native controls or alter the virtualized page layout.
    let mut ahead: Vec<_> = app.scene.artwork_ahead.iter().collect();
    let upward = app.artwork_prefetch_upward;
    ahead.sort_by_key(|(y, _)| {
        let preferred = if upward {
            *y < app.scene.content_top
        } else {
            *y >= content_bottom(app, h)
        };
        (
            !preferred,
            (*y - if upward {
                app.scene.content_top
            } else {
                content_bottom(app, h)
            })
            .abs(),
        )
    });
    urls.extend(ahead.into_iter().map(|(_, url)| url.clone()));
    let mut seen = HashSet::new();
    urls.retain(|url| seen.insert(url.clone()));
    app.artwork.set_wanted(seen.clone(),visible.clone());
    app.image_retry_at.retain(|url, _| seen.contains(url));
    {
        let touched:HashSet<&str>=urls.iter().filter(|url|app.images.contains_key(*url)).map(String::as_str).collect();
        app.image_order.retain(|key|!touched.contains(key.as_str()));
    }
    for url in urls {
        if app.images.contains_key(&url) {
            // Actual use refreshes recency; insertion order alone evicts covers
            // that are still on screen during long browsing sessions.
            app.image_order.push_back(url.clone());
        }
        if !url.is_empty()
            && !app.images.contains_key(&url)
            && !app.image_pending.contains(&url)
            && (visible.contains(&url)
                || !crate::drawing::progressive_artwork_enabled()
                || !matches!(app.ui.view, View::Home | View::Explore)
                || app.image_pending.len() < 12)
            && app
                .image_retry_at
                .get(&url)
                .is_none_or(|at| std::time::Instant::now() >= *at)
            && app
                .artwork
                .request(crate::catalog::ArtRequest {
                    #[cfg(feature = "diagnostics")]
                    view_tag: app.ui.view as usize + 1,
                    engine: engine.clone(),
                    url: url.strip_prefix("cover:").unwrap_or(&url).to_owned(),
                    key: url.clone(),
                    background: false,
                    max_edge: if url == crate::catalog::cover_artwork_key(&app.ui.now.artwork)
                        && app.ui.view == View::Lyrics
                    {
                        1024
                    } else {
                        crate::catalog::CARD_ART_EDGE
                    },
                },visible.contains(&url))
                .is_ok()
        {
            app.image_pending.insert(url);
        }
    }
    let used=visible.iter().filter_map(|url|app.images.get(url).map(|i|i.id)).collect();
    app.artwork.residency(app.images.values().map(|i|i.id).collect(),&used);
}
unsafe fn start_spotify(app: &mut App) {
    app.spotify_service.take();
    app.spotify_opened = false;
    match crate::spotify::service::Service::start() {
        Ok(service) => {
            app.spotify_service = Some(service);
            app.ui.spotify_phase = Some(crate::spotify::service::Phase::Starting);
            app.ui.spotify_setup_status = "Starting Soloist...".into();
        }
        Err(error) => {
            app.ui.spotify_phase = Some(crate::spotify::service::Phase::Failed);
            app.ui.spotify_setup_status = error;
        }
    }
}
unsafe fn begin_spotify_auth(app: &mut App, action: crate::spotify::AuthAction) {
    if app.spotify_auth.is_some() {
        return;
    }
    app.spotify_jam_request = None;
    app.ui.spotify_jam = None;
    app.ui.spotify_jam_busy = false;
    app.ui.spotify_jam_status.clear();
    app.ui.spotify_reset_pending = matches!(action, crate::spotify::AuthAction::Refresh);
    let cancel = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let flag = cancel.clone();
    let (send, receive) = std::sync::mpsc::sync_channel(1);
    match std::thread::Builder::new()
        .name("spotify-catalog-auth".into())
        .spawn(move || {
            let result = match action {
                crate::spotify::AuthAction::Login => crate::spotify::login_web(&flag),
                crate::spotify::AuthAction::Logout => crate::spotify::logout(),
                crate::spotify::AuthAction::Refresh => crate::spotify::refresh()
                    .and_then(|_| crate::spotify::catalog_account().map(|_| ())),
            };
            let _ = send.send(result);
        }) {
        Ok(_) => {
            app.spotify_auth = Some((cancel, receive));
            app.ui.spotify_catalog_busy = true;
            app.ui.spotify_catalog_status = "Updating Spotify sign-in...".into();
        }
        Err(_) => app.ui.spotify_catalog_status = "Could not start Spotify sign-in.".into(),
    }
}
unsafe fn request_spotify_jam(app: &mut App, operation: crate::spotify::jam::Operation) {
    if app.ui.provider != crate::catalog::Provider::Spotify || app.ui.spotify_jam_busy {
        return;
    }
    let identity = crate::spotify::jam::account_key();
    let copy_invite = matches!(operation, crate::spotify::jam::Operation::Invite(_));
    app.ui.spotify_jam_busy = true;
    app.ui.spotify_jam_status = "Loading Spotify Jam...".into();
    match crate::spotify::jam::spawn(operation, identity.clone()) {
        Ok(request) => {
            app.spotify_jam_request = Some((app.generation, identity, copy_invite, request))
        }
        Err(_) => {
            app.ui.spotify_jam_busy = false;
            app.ui.spotify_jam_status = "Could not start Spotify Jam worker.".into();
        }
    }
}

unsafe fn refresh_spotify_devices(app: &mut App) {
    if app.ui.provider != crate::catalog::Provider::Spotify || app.ui.spotify_devices_busy {
        return;
    }
    app.ui.spotify_devices_busy = true;
    app.ui.spotify_devices_status = "Loading Spotify devices...".into();
    request_spotify_device_action(app, "SPDevicesJSON", vec![]);
}
unsafe fn request_spotify_device_action(app: &mut App, name: &str, args: Vec<serde_json::Value>) {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    let token = (1u64 << 63) | NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    app.spotify_device_request = Some(token);
    let request = crate::catalog::Request {
        #[cfg(feature = "diagnostics")]
        view_tag: app.ui.view as usize + 1,
        generation: token,
        provider: crate::catalog::Provider::Spotify,
        engine: None,
        job: crate::catalog::Job::Api(name.into(), args),
    };
    if app.catalog.requests.send(request).is_err() {
        app.spotify_device_request = None;
        app.ui.spotify_devices_busy = false;
        app.ui.spotify_devices_status = "Spotify device worker is unavailable.".into();
    }
}
unsafe fn open_spotify(hwnd: HWND, app: &mut App) {
    if ShellExecuteW(
        hwnd,
        wide("open").as_ptr(),
        wide("spotify:").as_ptr(),
        null(),
        null(),
        SW_SHOWNORMAL,
    ) as isize
        <= 32
    {
        app.notice="Open Spotify on this computer or another device on the same network, then select DeezNats in its device picker.".into();
    }
}
unsafe fn process_data(hwnd: HWND, app: &mut App) {
    let auth_result =
        app.spotify_auth
            .as_ref()
            .and_then(|(_, receiver)| match receiver.try_recv() {
                Ok(result) => Some(result),
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    Some(Err("Spotify sign-in worker closed.".into()))
                }
                Err(_) => None,
            });
    if let Some(result) = auth_result {
        app.spotify_auth = None;
        app.ui.spotify_catalog_busy = false;
        let reset_pending = std::mem::take(&mut app.ui.spotify_reset_pending);
        match result {
            Ok(()) => {
                app.ui.spotify_reset_failed = false;
                app.ui.spotify_catalog_status = if crate::spotify::api_configured() {
                    "Spotify sign-in updated."
                } else {
                    "Signed out of Spotify."
                }
                .into();
                if app.ui.provider == crate::catalog::Provider::Spotify {
                    switch_provider(app, crate::catalog::Provider::Spotify);
                }
            }
            Err(error) => {
                if reset_pending {
                    app.ui.spotify_reset_failed = true;
                }
                app.ui.spotify_catalog_status = error;
            }
        }
        app.notice = app.ui.spotify_catalog_status.clone();
        rebuild(hwnd, app);
    }
    let setup_result =
        app.spotify_setup
            .as_ref()
            .and_then(|(_, receive)| match receive.try_recv() {
                Ok(value) => Some(value),
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    Some(Err("Spotify setup window closed.".into()))
                }
                Err(_) => None,
            });
    if let Some(result) = setup_result {
        app.spotify_setup = None;
        app.ui.spotify_setup_busy = false;
        match result {
            Ok(()) => {
                app.ui.spotify_configured = true;
                start_spotify(app);
            }
            Err(error) => app.ui.spotify_setup_status = error,
        }
        app.notice = app.ui.spotify_setup_status.clone();
        rebuild(hwnd, app);
    }

    if !app.ui.spotify_setup_busy {
        if let Some(service) = &app.spotify_service {
            let status = service.status();
            if app.ui.spotify_phase != Some(status.phase)
                || app.ui.spotify_setup_status != status.message
            {
                if app.notice == app.ui.spotify_setup_status {
                    app.notice = status.message.clone();
                }
                app.ui.spotify_phase = Some(status.phase);
                app.ui.spotify_setup_status = status.message;
                if status.phase == crate::spotify::service::Phase::Pairing && !app.spotify_opened {
                    app.spotify_opened = true;
                    MessageBoxW(
                        hwnd,
                        wide(&app.ui.t("Open Spotify on your phone or computer and select DeezNats in Spotify Connect to finish pairing.")).as_ptr(),
                        wide(&app.ui.t("Pair Spotify")).as_ptr(),
                        MB_OK | MB_ICONINFORMATION,
                    );
                    open_spotify(hwnd, app);
                }
                rebuild(hwnd, app);
            }
        }
    }
    let auth_result = app
        .deezer_auth
        .as_ref()
        .and_then(|(_, receive)| match receive.try_recv() {
            Ok(value) => Some(value),
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                Some(Err("Deezer sign-in window closed.".into()))
            }
            Err(_) => None,
        });
    if let Some(result) = auth_result {
        app.deezer_auth = None;
        app.ui.deezer_login_busy = false;
        match result {
            Ok(secret) => {
                if command(app, Command::Login(secret)) {
                    app.pending = Some(app.snapshot.login_attempt + 1);
                    app.ui.deezer_login_busy = true;
                    app.ui.deezer_login_status = "Connecting to Deezer...".into();
                    app.notice = "Connecting to Deezer...".into();
                }
            }
            Err(error) => {
                app.ui.deezer_login_status = error.clone();
                app.notice = error;
            }
        }
        rebuild(hwnd, app);
    }

    let mut dirty = false;
    let jam_result = app
        .spotify_jam_request
        .as_ref()
        .and_then(|(_, _, _, request)| match request.receiver.try_recv() {
            Ok(result) => Some(result),
            Err(std::sync::mpsc::TryRecvError::Disconnected) => Some(Err(
                "Spotify Jam worker closed. Refresh Jam to check its state.".into(),
            )),
            Err(std::sync::mpsc::TryRecvError::Empty) => None,
        });
    if let Some(result) = jam_result {
        if let Some((_generation, identity, copy_invite, _)) = app.spotify_jam_request.take() {
            app.ui.spotify_jam_busy = false;
            if app.ui.provider == crate::catalog::Provider::Spotify
                && identity == crate::spotify::jam::account_key()
            {
                match result {
                    Ok(session) => {
                        if copy_invite {
                            if let Some(invite) = session.as_ref().and_then(|s| s.invite.as_ref()) {
                                app.notice = match copy_text(hwnd, invite) {
                                    Ok(()) => "Spotify Jam invite copied.".into(),
                                    Err(error) => error,
                                };
                            }
                        }
                        app.ui.spotify_jam_status = if session.as_ref().is_some_and(|s| s.active) {
                            "Jam is active."
                        } else {
                            "No active Spotify Jam."
                        }
                        .into();
                        app.ui.spotify_jam = session;
                    }
                    Err(error) => app.ui.spotify_jam_status = error,
                }
            } else {
                app.ui.spotify_jam = None;
                app.ui.spotify_jam_status = "Spotify session changed. Refresh Jam.".into();
            }
            dirty = true;
        }
    }

    let mut repaint = false;
    // Resolve the current target before accepting worker replies: a reply for a
    // track that was replaced during this poll must never start its fade-in.
    update_background(hwnd, app);
    while let Ok((key, texture)) = app.backdrop_worker.updates.try_recv() {
        if key == app.background_source && app.background_requested {
            app.background = Some(texture);
            app.background_transition
                .texture_ready(std::time::Instant::now());
            start_animation_timer(hwnd, 2);
            repaint = true;
        }
    }
    while let Ok(event) = app.downloads.updates.try_recv() {
        match event {
            crate::downloads::Event::Maintenance(result) => {
                app.ui.library_storage = match result {
                    Ok(summary) => summary,
                    Err(error) => error,
                };
                app.notice = app.ui.library_storage.clone();
                dirty = true;
            }
            crate::downloads::Event::RemovedSelection(result) => {
                app.downloads.finish_removal();
                app.ui.offline = crate::downloads::load_offline();
                app.page_cache.clear();
                app.notice = match result {
                    Ok(()) => "Download removed".into(),
                    Err(error) => format!("Could not remove download: {error}"),
                };
                if matches!(app.ui.view, View::Offline | View::Podcasts) {
                    load_page(app);
                }
                dirty = true;
            }
            crate::downloads::Event::Removed(result) => {
                app.downloads.finish_removal();
                app.ui.downloads.clear();
                app.download_cancels.clear();
                app.ui.offline = crate::downloads::load_offline();
                app.page_cache.clear();
                app.catalog.session.invalidate();
                app.notice = match result {
                    Ok(()) => "Downloaded files removed".into(),
                    Err(error) => format!("Some downloads could not be removed: {error}"),
                };
                if matches!(app.ui.view, View::Offline | View::Podcasts) {
                    load_page(app);
                }
                dirty = true;
            }
            crate::downloads::Event::Started(id, title) => {
                if let Some(job) = app.ui.downloads.iter_mut().find(|job| job.id == id) {
                    job.message = format!("Downloading {title}");
                }
                dirty = true;
            }
            crate::downloads::Event::Track(id, index, total, item, result) => {
                if let Some(job) = app.ui.downloads.iter_mut().find(|j| j.id == id) {
                    job.done = index;
                    job.total = total;
                    match result {
                        Ok(_) => {
                            job.message = format!("Saved {}", item.title);
                            app.ui
                                .offline
                                .retain(|e| e.id != item.id || e.kind != item.kind);
                            app.ui.offline.push(*item);
                        }
                        Err(error) => {
                            job.failed += 1;
                            job.message = error;
                        }
                    }
                }
                if matches!(app.ui.view, View::Offline | View::Podcasts) {
                    load_page(app);
                }
                dirty = true;
            }
            crate::downloads::Event::Finished(id, _cancelled) => {
                app.ui.downloads.retain(|job| job.id != id);
                app.download_cancels.remove(&id);
                app.page_cache.clear();
                if matches!(app.ui.view, View::Podcasts | View::Offline) {
                    load_page(app);
                }
                dirty = true;
            }
        }
    }
    while let Ok(event) = app.catalog.updates.try_recv() {
        match event {
            crate::catalog::Event::Playlist(generation, provider, mutation, result)
                if provider == app.ui.provider && app.playlist_generation == Some(generation) =>
            {
                app.playlist_generation = None;
                app.ui.playlist_busy = false;
                if let crate::playlist::Mutation::Save { id, seed, .. } = &mutation {
                    app.ui.playlist_draft.id = id.clone();
                    if seed.is_none() {
                        app.ui.playlist_draft.seed = None;
                    }
                }
                app.page_cache.clear();
                app.catalog.session.invalidate();
                app.library_loaded = false;
                match result {
                    Ok(outcome) => {
                        if matches!(mutation, crate::playlist::Mutation::Save { .. }) {
                            app.ui.playlist_draft.image = None;
                        }
                        if matches!(mutation, crate::playlist::Mutation::Save { .. })
                            && matches!(app.ui.view, View::PlaylistEdit | View::CoverEdit)
                        {
                            let entry = crate::catalog::Entry {
                                id: outcome.id,
                                kind: "playlist".into(),
                                title: app.ui.playlist_draft.title.clone(),
                                artwork: app.ui.playlist_draft.preview.clone(),
                                raw: serde_json::json!({"description":app.ui.playlist_draft.description, "owned": provider == crate::catalog::Provider::YouTube}),
                                ..Default::default()
                            };
                            navigate(app, View::Album, Some(entry), String::new());
                            load_page(app);
                        } else if matches!(mutation, crate::playlist::Mutation::Add(..))
                            && app.ui.view == View::PlaylistPicker
                        {
                            let entry = app
                                .ui
                                .page
                                .as_ref()
                                .and_then(|p| p.rows.iter().find(|e| e.id == outcome.id))
                                .cloned()
                                .unwrap_or(crate::catalog::Entry {
                                    id: outcome.id.clone(),
                                    kind: "playlist".into(),
                                    title: "Playlist".into(),
                                    ..Default::default()
                                });
                            app.ui.playlist_track = None;
                            navigate(app, View::Album, Some(entry), String::new());
                            load_page(app);
                        } else if outcome.deleted
                            && (app.entity.as_ref().is_some_and(|e| e.id == outcome.id)
                                || matches!(app.ui.view, View::PlaylistEdit | View::CoverEdit))
                        {
                            navigate(app, View::Playlists, None, String::new());
                            load_page(app);
                        } else if matches!(
                            app.ui.view,
                            View::Album | View::Playlists | View::PlaylistPicker
                        ) {
                            load_page(app);
                        }
                        app.notice = if outcome.deleted {
                            "Playlist deleted"
                        } else {
                            "Playlist updated"
                        }
                        .into();
                    }
                    Err(error) => app.notice = error,
                }
                dirty = true;
            }
            crate::catalog::Event::Page(generation, page) if generation == app.generation => {
                if app.ui.provider == crate::catalog::Provider::YouTube
                    && app.ui.view == View::Favorites
                    && page.error.is_empty()
                {
                    app.ui
                        .liked_ids
                        .extend(page.rows.iter().map(|t| t.id.clone()));
                }
                if page.error.is_empty() {
                    let key = page_key(app);
                    app.page_cache.retain(|(k, _)| *k != key);
                    if app.page_cache.len() >= 12 {
                        app.page_cache.pop_front();
                    }
                    app.page_cache.push_back((key, page.clone()));
                }
                app.notice = if !page.error.is_empty() {
                    page.error.clone()
                } else if page.loading {
                    format!(
                        "Loading personalized {} content...",
                        app.ui.provider.label()
                    )
                } else {
                    String::new()
                };
                let changed = app
                    .ui
                    .page
                    .as_ref()
                    .is_none_or(|old| !crate::catalog::session::same_render(old, &page));
                app.ui.page = Some(page);
                dirty |= changed;
            }
            crate::catalog::Event::Api(generation, provider, name, result)
                if provider == app.ui.provider
                    && (if matches!(name.as_str(), "SPDevicesJSON" | "SPTransferPlaybackJSON") {
                        app.spotify_device_request == Some(generation)
                    } else {
                        generation == app.generation || library_operation(&name)
                    }) =>
            {
                if name == "SPDevicesJSON" {
                    app.spotify_device_request = None;
                    app.ui.spotify_devices_busy = false;
                    match &result {
                        Ok(value) => {
                            app.ui.spotify_devices = value
                                .as_array()
                                .or_else(|| value["devices"].as_array())
                                .cloned()
                                .unwrap_or_default();
                            app.ui.spotify_devices_status = if app.ui.spotify_devices.is_empty() {
                                "No Spotify devices found. Open Spotify on a device.".into()
                            } else {
                                String::new()
                            };
                        }
                        Err(error) => app.ui.spotify_devices_status = error.clone(),
                    }
                    dirty = true;
                    continue;
                }
                if name == "SPTransferPlaybackJSON" {
                    app.spotify_device_request = None;
                    app.ui.spotify_devices_busy = false;
                    match result {
                        Ok(_) => refresh_spotify_devices(app),
                        Err(error) => app.ui.spotify_devices_status = error,
                    }
                    dirty = true;
                    continue;
                }
                match result {
                    Ok(value) => {
                        if matches!(
                            name.as_str(),
                            "DeezerStyleMixGenres" | "SpotifyStyleMixGenres"
                        ) {
                            if value["region"].as_u64() == Some(app.ui.mix_region as u64) {
                                app.ui.mix_genres = Some(value);
                                app.notice.clear();
                            }
                            dirty = true;
                            continue;
                        }
                        if library_operation(&name) {
                            if name == "RustPodcastLibraryJSON"
                                && (provider == crate::catalog::Provider::Deezer
                                    || value.get("podcasts").is_some())
                            {
                                adopt_podcasts(app, &value);
                            }
                            if let Some((_, item, add)) = if (name != "RustPodcastLibraryJSON"
                                || provider == crate::catalog::Provider::Spotify
                                || value["changed"] == true)
                                && app.library_pending.as_ref().is_some_and(
                                    |(pending_generation, _, _)| *pending_generation == generation,
                                ) {
                                app.library_pending.take()
                            } else {
                                None
                            } {
                                if value.as_i64() == Some(0) || value.as_bool() == Some(false) {
                                    app.notice = "The service rejected the library update".into();
                                } else {
                                    if add {
                                        app.ui.liked_ids.insert(item.library_key());
                                    } else {
                                        app.ui.liked_ids.remove(&item.library_key());
                                    }
                                    app.page_cache.clear();
                                    app.notice = if item.kind == "podcast"
                                        && provider == crate::catalog::Provider::Deezer
                                    {
                                        if add {
                                            "Podcast saved to this app's library"
                                        } else {
                                            "Podcast removed from this app's library"
                                        }
                                    } else if add {
                                        "Added to library"
                                    } else {
                                        "Removed from library"
                                    }
                                    .into();
                                    if matches!(app.ui.view, View::Playlists | View::Podcasts) {
                                        load_page(app);
                                    }
                                }
                            }
                        } else if name == "DZAudioDevicesJSON" {
                            let devices = value.as_array().or_else(|| {
                                value.get("devices").and_then(serde_json::Value::as_array)
                            });
                            app.ui.devices =
                                std::iter::once((String::new(), "Default Windows output".into()))
                                    .chain(
                                        devices
                                            .into_iter()
                                            .flatten()
                                            .map(|v| {
                                                (
                                                    crate::catalog::string(v, &["id"]),
                                                    crate::catalog::string(v, &["name"]),
                                                )
                                            })
                                            .filter(|(id, _)| !id.is_empty()),
                                    )
                                    .collect();
                            app.notice =
                                format!("{} audio outputs available", app.ui.devices.len());
                        } else if name == "DZEQJSON" {
                            for (index, value) in value["gainsDb"]
                                .as_array()
                                .into_iter()
                                .flatten()
                                .take(10)
                                .enumerate()
                            {
                                app.ui.eq[index] = value.as_f64().unwrap_or(0.0) as i32;
                            }
                            app.ui.eq[10] = value["preampDb"].as_f64().unwrap_or(0.0) as i32;
                            app.ui.choices.insert(
                                "Equalizer",
                                value["enabled"].as_bool().unwrap_or(false) as usize,
                            );
                            if let Some(preset_str) = value["preset"].as_str() {
                                let normalized = preset_str.replace('-', " ");
                                if let Some(idx) = ui::options("Preset")
                                    .iter()
                                    .position(|&p| p.eq_ignore_ascii_case(&normalized))
                                {
                                    app.ui.choices.insert("Preset", idx);
                                }
                            } else {
                                app.ui
                                    .choices
                                    .insert("Preset", app.ui.custom_preset_index());
                            }
                            app.notice = "Equalizer loaded from audio engine".into();
                            if app.ui.view == View::Settings {
                                rebuild(hwnd, app);
                            }
                        } else if name == "DZFlowConfigsJSON" {
                            app.ui.flow_options = value["options"]
                                .as_array()
                                .into_iter()
                                .flatten()
                                .filter(|v| v["kind"].as_str() != Some("genre"))
                                .map(|v| {
                                    (
                                        crate::catalog::string(v, &["id"]),
                                        crate::catalog::string(v, &["title"]),
                                    )
                                })
                                .collect();
                            if !(app.ui.provider != crate::catalog::Provider::Deezer
                                && crate::spotify::flow::Selection::parse(&app.ui.flow_id).is_ok())
                                && !app
                                    .ui
                                    .flow_options
                                    .iter()
                                    .any(|(id, _)| *id == app.ui.flow_id)
                            {
                                app.ui.flow_id = app
                                    .ui
                                    .flow_options
                                    .first()
                                    .map(|(id, _)| id.clone())
                                    .unwrap_or_default();
                            }
                            data_request(
                                app,
                                crate::catalog::Job::Api(
                                    "DZFlowTunerJSON".into(),
                                    vec![serde_json::json!(app.ui.flow_id)],
                                ),
                            );
                            app.notice = "Loading Flow genres...".into();
                        } else if name == "DZFlowTunerJSON" {
                            if value["id"].as_str() != Some(app.ui.flow_id.as_str()) {
                                continue;
                            }
                            app.ui.flow_customizable =
                                value["customizable"].as_bool().unwrap_or(false);
                            app.ui.flow_loaded = true;
                            app.ui.genres = value["genres"]
                                .as_array()
                                .into_iter()
                                .flatten()
                                .map(|v| {
                                    let enabled = v["isEnabled"].as_bool().unwrap_or(false);
                                    (
                                        crate::catalog::string(v, &["id"]),
                                        crate::catalog::string(v, &["title"]),
                                        enabled,
                                        enabled,
                                    )
                                })
                                .collect();
                            app.notice =
                                format!("Flow genres loaded from {}", app.ui.provider.label());
                            if let Some(expected) = app.flow_save_expected.clone() {
                                if expected.iter().all(|(id, enabled)| {
                                    app.ui.genres.iter().any(|g| g.0 == *id && g.2 == *enabled)
                                }) {
                                    app.flow_save_expected = None;
                                    app.ui.save_preferences();
                                    invalidate_flow(app);
                                    navigate(app, View::Flow, None, String::new());
                                    load_page(app);
                                    app.notice = "Flow configuration saved and verified".into();
                                } else if app.flow_verify_attempt < 3 {
                                    app.flow_verify_attempt += 1;
                                    data_request(
                                        app,
                                        crate::catalog::Job::Api(
                                            "DZFlowTunerJSON".into(),
                                            vec![serde_json::json!(app.ui.flow_id)],
                                        ),
                                    );
                                } else {
                                    app.flow_save_expected = None;
                                    app.notice="Deezer did not confirm the saved Flow selections. Please reload and try again.".into();
                                }
                            }
                        } else if name == "DZSaveFlowTunerJSON" {
                            data_request(
                                app,
                                crate::catalog::Job::Api(
                                    "DZFlowTunerJSON".into(),
                                    vec![serde_json::json!(app.ui.flow_id)],
                                ),
                            );
                            app.notice = "Verifying saved Flow genres...".into();
                        } else if name == "DZCreatePlaylist" {
                            app.notice = "Playlist created".into();
                        } else {
                            app.notice = if value.is_null() {
                                format!("{name} applied")
                            } else {
                                let result = value.to_string();
                                result.chars().take(400).collect::<String>().to_string()
                            };
                        }
                    }
                    Err(error) => {
                        if matches!(
                            name.as_str(),
                            "DeezerStyleMixGenres" | "SpotifyStyleMixGenres"
                        ) {
                            app.ui.mix_genres = Some(serde_json::json!({"error":error}));
                        }
                        if library_operation(&name)
                            && app.library_pending.as_ref().is_some_and(
                                |(pending_generation, _, _)| *pending_generation == generation,
                            )
                        {
                            app.library_pending = None;
                        }
                        app.notice = error;
                    }
                }
                dirty = true;
            }
            crate::catalog::Event::Lyrics(provider, id, sources, result)
                if provider == app.ui.provider
                    && id == app.snapshot.now.id
                    && sources == app.ui.lyrics_sources & crate::lyrics::SUPPORTED_SOURCES =>
            {
                match result {
                    Ok((provider, lines)) => {
                        let changed = app.ui.lyrics_provider != provider
                            || app.ui.rich_lyrics.len() != lines.len()
                            || app
                                .ui
                                .rich_lyrics
                                .iter()
                                .zip(&lines)
                                .any(|(a, b)| a.text != b.text || a.start_ms != b.start_ms);
                        if changed && app.ui.view == View::Lyrics {
                            settle_scroll(app, 0);
                            app.ui.lyrics_manual_scroll = false;
                            app.lyric_motion = None;
                            app.lyric_motion_offset = 0;
                        }
                        app.ui.lyrics_provider = provider;
                        app.ui.rich_lyrics = lines.clone();
                        app.ui.lyrics =
                            lines.iter().map(|l| (l.start_ms, l.text.clone())).collect();
                        if app.ui.view == View::Lyrics {
                            app.notice = if app.ui.lyrics.is_empty() {
                                "No lyrics returned for this track.".into()
                            } else {
                                String::new()
                            };
                        }
                    }
                    Err(error) => {
                        app.ui.lyrics_provider.clear();
                        app.ui.rich_lyrics.clear();
                        app.ui.lyrics.clear();
                        if app.ui.view == View::Lyrics {
                            app.notice = error;
                        }
                    }
                }
                dirty = true;
            }
            crate::catalog::Event::TrackInfo(generation, entry, meta, artist_meta)
                if generation == app.generation =>
            {
                app.ui.track_info = Some((*entry, meta, artist_meta));
                dirty = true;
            }
            _ => {}
        }
    }
    let mut artwork_cancelled=false;
    while let Ok(key)=app.artwork.cancelled.try_recv(){app.image_pending.remove(&key);artwork_cancelled=true;}
    let artwork_batch_started = std::time::Instant::now();
    let mut artwork_changed = false;
    let (_,artwork_height)=size(hwnd,app);
    let mut artwork_rects=Vec::new();
    for _ in 0..4 {
        if artwork_batch_started.elapsed() >= std::time::Duration::from_millis(2) {
            break;
        }
        let Ok((url, result)) = app.artwork.updates.try_recv() else {
            break;
        };
        app.image_pending.remove(&url);
        if result.is_err() {
            app.image_retry_at.insert(
                url.clone(),
                std::time::Instant::now() + std::time::Duration::from_secs(3),
            );
        }
        if let Ok(bitmap) = result {
            app.image_retry_at.remove(&url);
            if url.starts_with("background:") {
                let old: Vec<_> = app
                    .images
                    .keys()
                    .filter(|key| key.starts_with("background:") && **key != url)
                    .cloned()
                    .collect();
                for key in old {
                    app.images.remove(&key);
                    app.image_pending.remove(&key);
                    app.image_times.remove(&key);
                    app.image_order.retain(|k| k != &key);
                }
            }
            // Protect playing, nearby, and visible covers. Pinning the entire
            // queue made the resident byte budget unenforceable on large playlists.
            let mut queue_covers: HashSet<&str> = app
                .ui
                .queue
                .iter()
                .skip(app.ui.queue_index.saturating_sub(2))
                .take(8)
                .chain(std::iter::once(&app.ui.now))
                .map(|entry| entry.artwork.as_str())
                .filter(|url| !url.is_empty())
                .collect();
            let background_key = format!("background:{}", app.ui.now.artwork);
            let lyrics_blur_key = format!(
                "blur:cover:{}",
                crate::catalog::cover_artwork_key(&app.ui.now.artwork)
            );
            queue_covers.insert(background_key.as_str());
            queue_covers.insert(lyrics_blur_key.as_str());
            queue_covers.extend(
                app.scene
                    .buttons
                    .iter()
                    .map(|button| button.artwork.as_str()),
            );
            queue_covers.extend(app.scene.pictures.iter().map(|(_, url)| url.as_str()));
            // 192px thumbnails let 64 MiB retain more ordinary covers than
            // the previous 96 MiB / 256px policy. Queue/visible pins stay intact.
            const RESIDENT_BYTES: usize = 64 * 1024 * 1024;
            let incoming_bytes = bitmap.pixels.len() * 4;
            let mut resident_bytes: usize = app
                .images
                .values()
                .map(|image| image.pixels.len() * 4)
                .sum();
            while (resident_bytes + incoming_bytes > RESIDENT_BYTES || app.images.len() >= 1024)
                && !app.images.contains_key(&url)
            {
                let Some(index) = app
                    .image_order
                    .iter()
                    .position(|key| !queue_covers.contains(key.as_str()))
                else {
                    break;
                };
                let old = app.image_order.remove(index).unwrap();
                if let Some(image) = app.images.remove(&old) {
                    resident_bytes = resident_bytes.saturating_sub(image.pixels.len() * 4);
                }
                app.image_pending.remove(&old);
                // A decoded replacement gets a fresh arrival time when resident again.
                app.image_times.remove(&old);
            }
            app.images.insert(url.clone(), bitmap);
            let damage=artwork_damage::arrival(&app.scene,&url,content_bottom(app,artwork_height));
            if damage.main {gpu::invalidate_main_layer();}
            if damage.queue {gpu::invalidate_queue_layer();}
            artwork_rects.extend(damage.rects);
            app.image_times
                .insert(url.clone(), std::time::Instant::now());
            let cover=url==app.ui.now.artwork||url==crate::catalog::cover_artwork_key(&app.ui.now.artwork)||url.starts_with("background:")||url.starts_with("blur:cover:");
            if cover {start_animation_timer(hwnd, 2);repaint=true;}
            app.image_order.retain(|key| key != &url);
            app.image_order.push_back(url);
            artwork_changed = true;
            // A newly arrived cover may need its hover-blur variant prepared.
            dirty |= app.ui.view == View::Lyrics && app.ui.lyrics_cover_hovered;
        }
    }
    if artwork_changed {
        let (_, height) = size(hwnd, app);
        trim_artwork(app, height);
        request_art(app,height);
    }
    if dirty {
        rebuild(hwnd, app);
    } else if repaint {
        let (_, height) = size(hwnd, app);
        request_art(app, height);
        update_background(hwnd, app);
        InvalidateRect(hwnd, null(), 0);
    } else if artwork_changed {
        let caption=caption_height(hwnd);
        for rect in artwork_rects {
            let mut rect=app.rect(rect);rect.top+=caption;rect.bottom+=caption;
            InvalidateRect(hwnd,&rect,0);
        }
    } else if artwork_cancelled {
        let (_,height)=size(hwnd,app);request_art(app,height);
    } else if app
        .image_retry_at
        .values()
        .any(|at| std::time::Instant::now() >= *at)
    {
        let (_, height) = size(hwnd, app);
        request_art(app, height);
    }
}

unsafe fn choose_storage_folder(hwnd: HWND, title: &str) -> Result<Option<PathBuf>, String> {
    use windows_sys::Win32::System::Com::{
        CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED,
    };
    use windows_sys::Win32::UI::Shell::{
        SHBrowseForFolderW, SHGetPathFromIDListW, BIF_NEWDIALOGSTYLE, BIF_RETURNONLYFSDIRS,
        BROWSEINFOW,
    };
    if CoInitializeEx(null(), COINIT_APARTMENTTHREADED as u32) < 0 {
        return Err("Could not initialize the folder picker.".into());
    }
    let title = wide(title);
    let mut display = [0u16; 260];
    let info = BROWSEINFOW {
        hwndOwner: hwnd,
        pszDisplayName: display.as_mut_ptr(),
        lpszTitle: title.as_ptr(),
        ulFlags: BIF_NEWDIALOGSTYLE | BIF_RETURNONLYFSDIRS,
        ..zeroed()
    };
    let item = SHBrowseForFolderW(&info);
    if item.is_null() {
        CoUninitialize();
        return Ok(None);
    }
    let mut path = [0u16; 32768];
    let valid = SHGetPathFromIDListW(item, path.as_mut_ptr()) != 0;
    windows_sys::Win32::System::Com::CoTaskMemFree(item.cast());
    CoUninitialize();
    if !valid {
        return Err("Could not read the selected folder.".into());
    }
    let end = path.iter().position(|c| *c == 0).unwrap_or(path.len());
    Ok(Some(PathBuf::from(String::from_utf16_lossy(&path[..end]))))
}
unsafe fn open_image(hwnd: HWND) -> Option<PathBuf> {
    let mut path = vec![0u16; 32768];
    let filter = wide("Images\0*.jpg;*.jpeg;*.png;*.webp\0All files\0*.*\0");
    let mut dialog: OPENFILENAMEW = zeroed();
    dialog.lStructSize = size_of::<OPENFILENAMEW>() as u32;
    dialog.hwndOwner = hwnd;
    dialog.lpstrFilter = filter.as_ptr();
    dialog.lpstrFile = path.as_mut_ptr();
    dialog.nMaxFile = path.len() as u32;
    dialog.Flags = OFN_FILEMUSTEXIST | OFN_PATHMUSTEXIST | OFN_NOCHANGEDIR;
    if GetOpenFileNameW(&mut dialog) != 0 {
        Some(PathBuf::from(String::from_utf16_lossy(
            &path[..path.iter().position(|&v| v == 0).unwrap_or(0)],
        )))
    } else {
        None
    }
}
unsafe fn theme_path(hwnd: HWND, save: bool) -> Option<PathBuf> {
    let mut path = vec![0u16; 32768];
    if save {
        let name = wide("Custom.deeznats-theme.json");
        path[..name.len()].copy_from_slice(&name);
    }
    let filter = wide("DeezNats theme (*.json)\0*.json\0\0");
    let extension = wide("json");
    let title = wide(if save { "Export theme" } else { "Import theme" });
    let mut dialog: OPENFILENAMEW = zeroed();
    dialog.lStructSize = size_of::<OPENFILENAMEW>() as u32;
    dialog.hwndOwner = hwnd;
    dialog.lpstrFilter = filter.as_ptr();
    dialog.lpstrFile = path.as_mut_ptr();
    dialog.nMaxFile = path.len() as u32;
    dialog.lpstrDefExt = extension.as_ptr();
    dialog.lpstrTitle = title.as_ptr();
    dialog.Flags = OFN_NOCHANGEDIR
        | OFN_PATHMUSTEXIST
        | if save {
            OFN_OVERWRITEPROMPT
        } else {
            OFN_FILEMUSTEXIST
        };
    let success = if save {
        GetSaveFileNameW(&mut dialog)
    } else {
        GetOpenFileNameW(&mut dialog)
    };
    (success != 0).then(|| {
        PathBuf::from(String::from_utf16_lossy(
            &path[..path.iter().position(|v| *v == 0).unwrap_or(0)],
        ))
    })
}
unsafe fn update_background(hwnd: HWND, app: &mut App) {
    if app.render_suspended {
        app.layout_dirty = true;
        return;
    }
    let now = std::time::Instant::now();
    if app.cover_source != app.ui.now.artwork {
        if let Some(old) = app.images.get(&app.cover_source) {
            app.previous_cover = Some(old.clone());
        }
        app.cover_source = app.ui.now.artwork.clone();
        app.cover_changed = None;
        if app.cover_source.is_empty() {
            app.previous_cover = None;
        }
    }
    if app.cover_changed.is_none() && app.images.contains_key(&app.cover_source) {
        app.cover_changed = Some(now);
        start_animation_timer(hwnd, 2);
    }
    let url = match app.ui.background_mode {
        1 if !crate::catalog::is_default_cover(&app.ui.now.artwork) => app.ui.now.artwork.clone(),
        2 => app.custom_background.clone(),
        _ => String::new(),
    };
    if url.is_empty() {
        *app.backdrop_cache.borrow_mut() = Default::default();
        app.background = None;
        app.previous_background = None;
        app.background_source.clear();
        app.background_requested = false;
        app.background_transition = BackgroundTransition::Complete;
        return;
    }
    let source_key = if app.ui.background_mode == 1 {
        format!("background:{url}")
    } else {
        url.clone()
    };
    let effective_blur = if app.ui.ui_blur {
        app.ui.blur.min(100)
    } else {
        0
    };
    let key = format!("{source_key}:{effective_blur}");
    if key != app.background_source {
        // Start fading the old texture immediately, including download/decode time.
        // Retargeting during fade-out keeps its deadline; retargeting during an
        // ambient hold must not resurrect an already invisible outgoing cover.
        app.background_transition.advance(now);
        let transition = match app.background_transition {
            BackgroundTransition::FadeOut { started, .. } => BackgroundTransition::FadeOut {
                started,
                ready_at: None,
            },
            BackgroundTransition::AmbientHold => {
                app.previous_background = None;
                BackgroundTransition::AmbientHold
            }
            _ => {
                app.previous_background = app.background.take();
                BackgroundTransition::FadeOut {
                    started: now,
                    ready_at: None,
                }
            }
        };
        app.background = None;
        app.background_transition = transition;
        app.background_source = key.clone();
        app.background_requested = false;
        start_animation_timer(hwnd, 2);
    }
    if app.ui.background_mode == 1 {
        let high_key = source_key.clone();
        if !app.images.contains_key(&high_key) {
            if !app.image_pending.contains(&high_key) {
                let high_url = if url.starts_with("https://cdn-images.dzcdn.net/images/") {
                    url.replace("/320x320-", "/768x768-")
                        .replace("/250x250-", "/768x768-")
                } else {
                    url.clone()
                };
                if app
                    .artwork
                    .request(crate::catalog::ArtRequest {
                        #[cfg(feature = "diagnostics")]
                        view_tag: app.ui.view as usize + 1,
                        engine: app.snapshot.engine.clone(),
                        url: high_url,
                        key: high_key.clone(),
                        background: true,
                        max_edge: 768,
                    },true)
                    .is_ok()
                {
                    app.image_pending.insert(high_key);
                }
            }
            return;
        }
    }
    if app.background_requested {
        return;
    }
    if let Some(source) = app.images.get(&source_key) {
        if app
            .backdrop_worker
            .requests
            .try_send(crate::appearance::Request {
                key: key.clone(),
                source: source.clone(),
                blur: effective_blur,
            })
            .is_ok()
        {
            app.background_requested = true;
        }
    }
}
fn navigate(app: &mut App, view: View, entity: Option<crate::catalog::Entry>, query: String) {
    if app.navigation.len() == 32 {
        app.navigation.remove(0);
    }
    app.navigation.push((
        app.ui.view,
        app.entity.clone(),
        app.query.clone(),
        app.ui.scroll,
    ));
    if app.ui.view == view {
        app.ui.history.push(view);
    } else {
        app.ui.go(view);
    }
    if app.ui.history.len() > 32 {
        app.ui.history.remove(0);
    }
    settle_scroll(app, 0);
    app.ui.shelf_offsets.clear();
    app.ui.shelf_rows.clear();
    app.entity = entity;
    app.query = query;
}
unsafe fn apply_eq(app: &mut App, band: usize, gain: i32) {
    let clamped = gain.clamp(-12, 12);
    if app.ui.eq[band] != clamped {
        app.ui.adjust_eq(band, clamped);
        let update = if band == 10 {
            serde_json::json!({"preampDb": clamped})
        } else {
            serde_json::json!({"band": {"index": band, "gainDb": clamped}})
        };
        command(
            app,
            Command::Api(
                "DZSetEQJSON".into(),
                vec![serde_json::json!(update.to_string())],
            ),
        );
    }
}

unsafe fn update_eq_from_graph(hwnd: HWND, app: &mut App, x: i32, y: i32, graph: Rect) {
    let pad_x = 48;
    let avail_w = (graph.w - pad_x * 2).max(1);
    let rel_x = (x - (graph.x + pad_x)).clamp(0, avail_w);
    let band = ((rel_x as f32 / avail_w as f32) * 9.0)
        .round()
        .clamp(0.0, 9.0) as usize;

    let pad_y = 24;
    let avail_h = (graph.h - pad_y * 2).max(1);
    let rel_y = (y - (graph.y + pad_y)).clamp(0, avail_h);
    let t = 1.0 - (rel_y as f32 / avail_h as f32);
    let gain = ((t * 24.0) - 12.0).round().clamp(-12.0, 12.0) as i32;

    apply_eq(app, band, gain);
    rebuild(hwnd, app);
}

unsafe fn update_eq_from_sliders(hwnd: HWND, app: &mut App, band: usize, y: i32, sliders: Rect) {
    let track_top = sliders.y + 44;
    let track_h = 80;
    let rel_y = (y - track_top).clamp(0, track_h);
    let t = 1.0 - (rel_y as f32 / track_h as f32);
    let gain = ((t * 24.0) - 12.0).round().clamp(-12.0, 12.0) as i32;

    apply_eq(app, band, gain);
    rebuild(hwnd, app);
}

unsafe fn activate_scene_button(hwnd: HWND, app: &mut App, index: usize) {
    if !scene_button_enabled(app, index) {
        return;
    }
    let node = &app.scene.buttons[index];
    if node.glyph == "\u{e710}" && matches!(node.action, Action::Go(View::Add)) {
        let rect = app.rect(node.rect);
        show_context_menu(
            hwnd,
            app,
            POINT {
                x: (rect.left + rect.right) / 2,
                y: rect.top,
            },
        );
    } else {
        let action = node.action.clone();
        activate(hwnd, app, action);
    }
}

unsafe fn activate(hwnd: HWND, app: &mut App, action: Action) {
    if !(app.ui.provider == crate::catalog::Provider::Spotify && app.ui.device_menu_open)
        && !matches!(
            action,
            Action::ToggleDeviceMenu
                | Action::CloseDeviceMenu
                | Action::SpotifyDevicesRefresh
                | Action::SpotifyTransferPlayback(_)
                | Action::SpotifyJamOpen
                | Action::SpotifyJamRefresh
                | Action::SpotifyJamStart
                | Action::SpotifyJamEnd(_)
                | Action::SpotifyJamLeave(_)
                | Action::SpotifyJamRemove(_, _)
                | Action::SpotifyJamCopyInvite
                | Action::ToggleBool("jam_enabled")
                | Action::OpenConnect
                | Action::CopyConnectLink
        )
    {
        if app.ui.device_menu_open && matches!(action, Action::Go(View::Settings)) {
            app.ui.tab = 5;
        }
        app.ui.device_menu_open = false;
    }
    if app.ui.shortcut_capture.take().is_some() {
        restore_shortcuts(app);
    }
    if app.ui.deezer_login_open
        && !matches!(
            action,
            Action::CloseDeezerLogin
                | Action::DeezerLoginAdvanced(_)
                | Action::ConnectDeezerWebView
                | Action::Connect
        )
    {
        return;
    }

    if !matches!(action, Action::ToggleLyricsVolume) {
        app.ui.lyrics_volume_open = false;
    }
    match action {
        Action::ToggleDeviceMenu => {
            app.ui.toggle_device_sidebar();
            if app.ui.device_menu_open {
                refresh_spotify_devices(app);
                if app.ui.provider == crate::catalog::Provider::Spotify {
                    request_spotify_jam(app, crate::spotify::jam::Operation::Refresh);
                }
            }
        }
        Action::CloseDeviceMenu => app.ui.device_menu_open = false,
        Action::SpotifyDevicesRefresh => {
            refresh_spotify_devices(app);
            request_spotify_jam(app, crate::spotify::jam::Operation::Refresh);
        }
        Action::SpotifyJamOpen => {
            app.ui.device_menu_open = true;
            app.ui.queue_sidebar_open = false;
            refresh_spotify_devices(app);
            request_spotify_jam(app, crate::spotify::jam::Operation::Refresh);
        }
        Action::SpotifyJamRefresh => {
            request_spotify_jam(app, crate::spotify::jam::Operation::Refresh)
        }
        Action::SpotifyJamStart => request_spotify_jam(app, crate::spotify::jam::Operation::Start),
        Action::SpotifyJamEnd(id) => {
            request_spotify_jam(app, crate::spotify::jam::Operation::End(id))
        }
        Action::SpotifyJamLeave(id) => {
            request_spotify_jam(app, crate::spotify::jam::Operation::Leave(id))
        }
        Action::SpotifyJamRemove(session, member) => request_spotify_jam(
            app,
            crate::spotify::jam::Operation::Remove { session, member },
        ),
        Action::SpotifyJamCopyInvite => {
            if let Some(session) = app.ui.spotify_jam.as_ref() {
                request_spotify_jam(
                    app,
                    crate::spotify::jam::Operation::Invite(session.id.clone()),
                );
            }
        }
        Action::SpotifyTransferPlayback(id) => {
            if app.ui.provider == crate::catalog::Provider::Spotify && !app.ui.spotify_devices_busy
            {
                app.ui.spotify_devices_busy = true;
                app.ui.spotify_devices_status = "Connecting to device...".into();
                let playing = app.snapshot.state == "Playing";
                request_spotify_device_action(
                    app,
                    "SPTransferPlaybackJSON",
                    vec![serde_json::json!(id), serde_json::json!(playing)],
                );
            }
        }
        Action::SwitchProvider(provider) => switch_provider(app, provider),
        Action::OnboardingSelect(provider) => app.ui.onboarding_provider = Some(provider),
        Action::OnboardingNext => {
            if let Some(provider) = app.ui.onboarding_provider {
                app.ui.onboarding_open = false;
                activate(
                    hwnd,
                    app,
                    match provider {
                        crate::catalog::Provider::Deezer => Action::DeezerLogin,
                        crate::catalog::Provider::Spotify => Action::SpotifyLogin,
                        crate::catalog::Provider::YouTube => Action::YouTubeLogin,
                    },
                );
            }
        }
        Action::DeezerLogin => {
            switch_provider(app, crate::catalog::Provider::Deezer);
            app.ui.deezer_login_open = true;
            app.ui.deezer_login_advanced = false;
        }
        Action::YouTubeLogin => {
            if app.ui.provider != crate::catalog::Provider::YouTube {
                switch_provider(app, crate::catalog::Provider::YouTube);
            }
            command(app, Command::Login(String::new()));
        }
        Action::YouTubeLogout => {
            command(app, Command::Logout);
        }
        Action::SpotifyLogin => {
            if app.ui.provider != crate::catalog::Provider::Spotify {
                switch_provider(app, crate::catalog::Provider::Spotify);
            }
            begin_spotify_auth(app, crate::spotify::AuthAction::Login);
        }
        Action::SpotifyLogout => begin_spotify_auth(app, crate::spotify::AuthAction::Logout),
        Action::SpotifyRefresh => begin_spotify_auth(app, crate::spotify::AuthAction::Refresh),
        Action::CheckUpdate => request_update(app, crate::updater::Request::Check),
        Action::DownloadUpdate => request_update(app, crate::updater::Request::Download),
        Action::ApplyUpdate => request_update(app, crate::updater::Request::Apply),
        Action::OpenConnect => {
            if let Some(connect) = &app.connect {
                let url = wide(&format!("http://127.0.0.1:{}/", connect.port()));
                if ShellExecuteW(
                    hwnd,
                    wide("open").as_ptr(),
                    url.as_ptr(),
                    null(),
                    null(),
                    SW_SHOWNORMAL,
                ) as isize
                    <= 32
                {
                    app.notice = "Could not open Connect in your browser".into();
                }
            }
        }
        Action::CopyConnectLink => {
            if let Some(connect) = &app.connect {
                app.notice = match copy_text(hwnd, &connect.share_url()) {
                    Ok(()) => "Connect link copied".into(),
                    Err(error) => error,
                };
            }
        }
        Action::DeezerLogo | Action::SpotifyLogo | Action::YouTubeLogo => {}

        Action::ShelfAll(title) => {
            let discover_filter = app
                .ui
                .page
                .as_ref()
                .filter(|page| app.ui.view == View::Explore && page.is_discover()
                    && !(app.ui.provider == crate::catalog::Provider::YouTube && title == "Categories"))
                .and_then(|_| match title.as_str() {
                    "Genres" => Some(2),
                    "Categories" => Some(3),
                    "Podcasts by category" => Some(4),
                    _ => None,
                });
            if let Some(filter) = discover_filter {
                app.ui.filter = filter;
                app.ui.scroll = 0;
                app.notice.clear();
            } else if let Some(section) = app
                .ui
                .page
                .as_ref()
                .and_then(|page| page.sections.iter().find(|s| s.title == title))
                .cloned()
            {
                let related = app.ui.page.as_ref().and_then(|page| crate::catalog::catalogue_page(page, &title));
                navigate(app, View::Album, None, String::new());
                app.generation += 1;
                app.catalog.set_generation(app.generation);

                app.ui.page = Some(Arc::new(related.unwrap_or_else(|| crate::catalog::Page {
                    title,
                    sections: vec![section],
                    ..Default::default()
                })));
            }
        }
        Action::ShelfMore(title, cols) => {
            if let Some(page) = app.ui.page.clone().filter(|page| !page.loading) {
                if app.ui.view == View::Explore && page.is_discover() {
                    if let Some(related) = crate::catalog::catalogue_page(&page, &title) {
                        navigate(app, View::Album, None, String::new());
                        app.generation += 1;
                        app.catalog.set_generation(app.generation);
                        app.ui.page = Some(Arc::new(related));
                    }
                } else {
                    let rows = app.ui.shelf_rows.entry(title.clone()).or_insert(2);
                    *rows += 2;
                    let target = *rows * cols;
                    let loaded = page
                        .sections
                        .iter()
                        .find(|s| s.title == title)
                        .map_or(0, |s| s.entries.len());
                    if loaded < target && crate::catalog::shelf_has_more(&page, &title) {
                        let mut request = (*page).clone();
                        request.error.clear();
                        request.loading = true;
                        request.raw.push((
                            "spotify:more-shelf".into(),
                            serde_json::json!({"title":title,"target":target}),
                        ));
                        app.ui.page = Some(Arc::new(request.clone()));
                        data_request(app, crate::catalog::Job::More(request));
                    }
                }
            }
        }
        Action::BackgroundMode(mode) => {
            app.ui.background_mode = mode;
            app.ui.solid = mode != 0;
            app.ui.choices.insert("Background", mode);
            material(hwnd, app);
            update_background(hwnd, app);
            app.ui.save_preferences();
        }
        Action::ImportTheme => {
            if let Some(path) = theme_path(hwnd, false) {
                match crate::theme::Theme::read(&path) {
                    Ok(theme) => {
                        theme.apply(&mut app.ui);
                        app.background_source.clear();
                        DeleteObject(app.brush);
                        app.brush = CreateSolidBrush(app.palette()[1]);
                        material(hwnd, app);
                        update_background(hwnd, app);
                        app.ui.save_preferences();
                        app.notice = format!("Theme applied: {}", theme.name());
                        if app.ui.background_mode == 2 && app.custom_background.is_empty() {
                            app.notice
                                .push_str(". Choose a background image on this device.");
                        }
                    }
                    Err(error) => app.notice = error,
                }
            }
        }
        Action::ExportTheme => {
            if let Some(path) = theme_path(hwnd, true) {
                app.notice = match crate::theme::Theme::capture(&app.ui).write(&path) {
                    Ok(()) => "Theme exported".into(),
                    Err(error) => error,
                };
            }
        }
        Action::LibraryMaintenance(clean) => {
            app.notice = match app.downloads.maintenance(clean) {
                Ok(()) => "Inspecting library storage...".into(),
                Err(error) => error,
            };
        }
        Action::RemoveDownloads => {
            for cancel in app.download_cancels.values() {
                cancel.store(true, std::sync::atomic::Ordering::Relaxed);
            }
            app.notice = match app.downloads.remove_all(app.snapshot.engine.clone()) {
                Ok(()) => "Removing downloaded files...".into(),
                Err(error) => error,
            };
        }
        Action::CancelDownload(id) => {
            if let Some(cancel) = app.download_cancels.get(&id) {
                cancel.store(true, std::sync::atomic::Ordering::Relaxed);
                app.notice = "Download cancellation requested".into();
            }
        }
        Action::MixMore(languages) => {
            let rows = if languages {
                &mut app.ui.mix_language_rows
            } else {
                &mut app.ui.mix_genre_rows
            };
            *rows = rows.saturating_add(3);
        }
        Action::MixLess(languages) => {
            let rows = if languages {
                &mut app.ui.mix_language_rows
            } else {
                &mut app.ui.mix_genre_rows
            };
            *rows = 3;
            app.ui.scroll = 0;
        }
        Action::MixLanguages => {
            app.ui.mix_language_rows = 3;
            app.ui.mix_region = usize::MAX;
            app.ui.mix_genre = usize::MAX;
            app.ui.mix_genres = None;
            app.query.clear();
            app.ui.scroll = 0;
            load_page(app);
        }
        Action::MixRegion(index) => {
            app.ui.mix_genre_rows = 3;
            app.ui.mix_region = index.min(crate::style_mix::REGIONS.len() - 1);
            app.ui.mix_genre = usize::MAX;
            app.ui.mix_genres = None;
            app.query.clear();
            app.ui.scroll = 0;
            load_page(app);
            if !app.snapshot.connected {
                app.ui.mix_genres =
                    Some(serde_json::json!({"error":"Sign in to check available genres."}));
            }
        }
        Action::MixGenre(index) => {
            if !app
                .ui
                .mix_genres
                .as_ref()
                .and_then(|data| data["genres"].as_array())
                .is_some_and(|genres| {
                    genres
                        .iter()
                        .any(|genre| genre["id"].as_u64() == Some(index as u64))
                })
            {
                return;
            }
            app.ui.mix_genre = index.min(crate::style_mix::GENRES.len() - 1);
            app.query = crate::style_mix::Selection::new(app.ui.mix_region, app.ui.mix_genre)
                .map(|selection| selection.key())
                .unwrap_or_default();
            app.ui.scroll = 0;
            load_page(app);
        }
        Action::BuildStyleMix => {
            app.query = crate::style_mix::Selection::new(app.ui.mix_region, app.ui.mix_genre)
                .map(|s| s.key())
                .unwrap_or_default();
            app.ui.scroll = 0;
            load_page(app);
        }
        Action::FlowChoice(index) => {
            if let Some((id, _)) = app.ui.flow_options.get(index) {
                if app.ui.provider != crate::catalog::Provider::Deezer {
                    app.ui.flow_id = crate::spotify::flow::Selection {
                        mood: id.clone(),
                        genres: crate::spotify::flow::mood_genres(id),
                    }
                    .key();
                } else {
                    app.ui.flow_id = id.clone();
                }
                app.ui.genres.clear();
                app.ui.flow_loaded = false;
                app.notice = "Loading Flow genres...".into();
                data_request(
                    app,
                    crate::catalog::Job::Api(
                        "DZFlowTunerJSON".into(),
                        vec![serde_json::json!(app.ui.flow_id)],
                    ),
                );
            }
        }
        Action::FlowMoreGenres => {
            app.ui.flow_genre_limit = if app.ui.flow_filter.trim().is_empty() {
                if app.ui.flow_genre_limit > 30 {
                    30
                } else {
                    crate::spotify::flow::BROWSE_GENRES.len()
                }
            } else {
                app.ui.flow_genre_limit.saturating_add(48)
            };
        }
        Action::FlowAddGenre => {
            let genre = app.ui.flow_filter.trim().to_owned();
            if app.ui.provider != crate::catalog::Provider::Deezer
                && app.ui.flow_customizable
                && crate::spotify::flow::valid_genre(&genre)
                && !app
                    .ui
                    .genres
                    .iter()
                    .any(|g| g.1.eq_ignore_ascii_case(&genre))
            {
                app.ui.genres.push((genre.clone(), genre, true, false));
            }
        }
        Action::Genre(index) => {
            if let Some(genre) = app
                .ui
                .genres
                .get_mut(index)
                .filter(|_| app.ui.flow_customizable)
            {
                genre.2 = !genre.2;
            }
        }
        Action::ApplyFlow => {
            if !app.ui.flow_loaded {
                app.notice = "Wait for Flow genres to load.".into();
                rebuild(hwnd, app);
                return;
            }
            if app.ui.provider != crate::catalog::Provider::Deezer {
                match crate::spotify::flow::Selection::parse(&app.ui.flow_id) {
                    Ok(mut selection) => {
                        selection.genres = app
                            .ui
                            .genres
                            .iter()
                            .filter(|g| g.2)
                            .map(|g| g.0.clone())
                            .collect();
                        if let Err(error) = crate::spotify::flow::Selection::parse(&selection.key())
                        {
                            app.notice = error;
                            rebuild(hwnd, app);
                            return;
                        }
                        app.ui.flow_id = selection.key();
                        app.ui.save_preferences();
                        invalidate_flow(app);
                        navigate(app, View::Flow, None, String::new());
                        load_page(app);
                    }
                    Err(error) => app.notice = error,
                }
                rebuild(hwnd, app);
                return;
            }
            let edits: Vec<_> = app
                .ui
                .genres
                .iter()
                .filter(|g| g.2 != g.3)
                .map(|g| serde_json::json!({"clusterId":g.0,"isEnabled":g.2,"isEditedByUser":true}))
                .collect();
            if !app.ui.flow_customizable || edits.is_empty() {
                app.ui.save_preferences();
                navigate(app, View::Flow, None, String::new());
                load_page(app);
                rebuild(hwnd, app);
                return;
            }
            if !app.ui.genres.iter().any(|g| g.2) {
                app.notice = "Select at least one genre.".into();
                rebuild(hwnd, app);
                return;
            }
            app.flow_save_expected =
                Some(app.ui.genres.iter().map(|g| (g.0.clone(), g.2)).collect());
            app.flow_verify_attempt = 1;
            data_request(
                app,
                crate::catalog::Job::Api(
                    "DZSaveFlowTunerJSON".into(),
                    vec![
                        serde_json::json!(app.ui.flow_id),
                        serde_json::json!(serde_json::to_string(&edits).unwrap_or_default()),
                    ],
                ),
            );
            app.notice = "Saving Flow configuration...".into();
        }
        Action::Preset(index) => {
            if let Some((name, start, end)) = ui::GRADIENT_PRESETS.get(index).copied() {
                let blend = |a: u32, b: u32, wa: u32| {
                    let wb = 100 - wa;
                    ((((a >> 16) & 255) * wa + ((b >> 16) & 255) * wb) / 100) << 16
                        | ((((a >> 8) & 255) * wa + ((b >> 8) & 255) * wb) / 100) << 8
                        | (((a & 255) * wa + (b & 255) * wb) / 100)
                };
                if app.ui.light {
                    let map = &mut app.ui.light_colors;
                    let base_bg = map.get("Background").copied().unwrap_or(0x00f8f4f2);
                    let base_border = map.get("Border").copied().unwrap_or(0x00d4c6bf);
                    map.insert("GradientStart".into(), start);
                    map.insert("GradientEnd".into(), blend(start, end, 72));
                    map.insert("Accent".into(), start);
                    map.insert("AccentText".into(), 0x00ffffff);
                    map.insert("Raised".into(), blend(base_bg, start, 86));
                    map.insert("Border".into(), blend(base_border, start, 76));
                } else {
                    let map = &mut app.ui.dark_colors;
                    map.insert("GradientStart".into(), start);
                    map.insert("GradientEnd".into(), end);
                    map.insert("Accent".into(), start);
                    map.insert("AccentText".into(), 0x00ffffff);
                    map.insert("Raised".into(), end);
                }
                material(hwnd, app);
                app.ui.save_preferences();
                app.notice = format!("Applied {name} preset.");
            }
        }
        Action::ResetTheme => {
            if app.ui.solid_theme >= ui::SOLID_THEMES.len() {
                let light = app.ui.light;
                app.ui.apply_solid_theme(if light { 12 } else { 1 });
                app.ui.solid_theme = ui::SOLID_THEMES.len() + usize::from(light);
                if light {
                    app.ui.custom_light_colors = app.ui.light_colors.clone();
                } else {
                    app.ui.custom_dark_colors = app.ui.dark_colors.clone();
                }
                material(hwnd, app);
                app.ui.save_preferences();
                app.notice = "Default theme colors restored.".into();
            } else {
                if app.ui.light {
                    app.ui.light_colors.clear();
                    let theme = if app.ui.solid_theme >= 12 {
                        app.ui.solid_theme
                    } else {
                        12
                    };
                    app.ui.apply_solid_theme(theme);
                } else {
                    app.ui.dark_colors.clear();
                    if app.ui.background_mode == 3 {
                        let theme = if app.ui.solid_theme < 12 {
                            app.ui.solid_theme
                        } else {
                            1
                        };
                        app.ui.apply_solid_theme(theme);
                    }
                }
                material(hwnd, app);
                app.ui.save_preferences();
                app.notice = "Default theme colors restored.".into();
            }
        }
        Action::Sort(col) => {
            if app.ui.sort_column == col {
                if app.ui.sort_descending {
                    app.ui.sort_column = 0;
                    app.ui.sort_descending = false;
                } else {
                    app.ui.sort_descending = true;
                }
            } else {
                app.ui.sort_column = col;
                app.ui.sort_descending = false;
            }
        }
        Action::ToggleCheck(id) => {
            if !app.ui.checked_ids.remove(&id) {
                app.ui.checked_ids.insert(id);
            }
        }
        Action::Share => {
            let item = app.entity.as_ref().unwrap_or(&app.ui.now);
            if let Some(link) = share_link(item) {
                app.notice = match copy_text(hwnd, &link) {
                    Ok(()) => { show_link_copied(hwnd, app); "Link copied".into() },
                    Err(error) => error,
                };
            } else {
                app.notice = "This item has no shareable link".into();
            }
        }
        Action::Color(key) => {
            let mut custom = [0u32; 16];
            let mut choose: CHOOSECOLORW = zeroed();
            choose.lStructSize = size_of::<CHOOSECOLORW>() as u32;
            choose.hwndOwner = hwnd;
            choose.rgbResult = match key {
                "FixedBackgroundColor" => app.ui.solid_color,
                "LightTint" => app.ui.light_tint,
                _ => app
                    .ui
                    .custom_theme_draft
                    .as_ref()
                    .and_then(|(_, colors)| colors.get(key).copied())
                    .unwrap_or_else(|| app.ui.color(key, rgb(159, 57, 124))),
            };
            choose.lpCustColors = custom.as_mut_ptr();
            choose.Flags = CC_FULLOPEN | CC_RGBINIT;
            if ChooseColorW(&mut choose) != 0 {
                if let Some((_, colors)) = &mut app.ui.custom_theme_draft {
                    colors.insert(key.into(), choose.rgbResult);
                } else if key == "FixedBackgroundColor" {
                    app.ui.solid_color = choose.rgbResult;
                    if app.ui.light {
                        app.ui
                            .light_colors
                            .insert("Background".into(), choose.rgbResult);
                    }
                } else if key == "LightTint" {
                    app.ui.light_tint = choose.rgbResult;
                } else if app.ui.light {
                    app.ui.light_colors.insert(key.into(), choose.rgbResult);
                } else {
                    app.ui.dark_colors.insert(key.into(), choose.rgbResult);
                }
                if app.ui.custom_theme_draft.is_none() {
                    material(hwnd, app);
                    app.ui.save_preferences();
                }
            }
        }
        Action::Adjust(key, delta) => {
            let is_blur = key == "Image blur";
            let value = match key {
                "Image blur" => &mut app.ui.blur,
                "Light-mode brightness" | "Background brightness" => &mut app.ui.brightness,
                "Light-mode opacity" | "Background tint opacity" => &mut app.ui.light_opacity,
                "Player bar opacity" | "Surface opacity" => &mut app.ui.surface_opacity,
                _ => &mut app.ui.surface_opacity,
            };
            let min_val = 0;
            let max_val = if matches!(key, "Light-mode brightness" | "Background brightness") {
                110
            } else {
                100
            };
            let pos = (*value as i32 + delta).clamp(min_val, max_val) as u32;
            *value = pos;
            if let Some((_, _, ref mut s_val)) =
                app.scene.sliders.iter_mut().find(|(_, k, _)| *k == key)
            {
                *s_val = pos;
            }
            if let Some((rect, _, _)) = app.scene.sliders.iter().find(|(_, k, _)| *k == key) {
                for label in &mut app.scene.labels {
                    if label.rect.x == rect.x + rect.w + 8 && label.rect.y == rect.y + 6 {
                        label.text = format!("{}%", pos);
                    }
                }
            }
            if is_blur {
                SetTimer(hwnd, 3, 60, None);
            } else {
                material(hwnd, app);
                update_background(hwnd, app);
                SetTimer(hwnd, 3, 300, None);
            }
            InvalidateRect(hwnd, null(), 0);
        }
        Action::ChooseStorage => {
            match choose_storage_folder(hwnd, app.ui.t("Choose data folder...")) {
                Ok(Some(folder)) => match crate::storage::choose(&folder) {
                    Ok(_) => app.notice = app
                        .ui
                        .t("Storage location saved. Close and reopen DeezNats to copy your data.")
                        .into(),
                    Err(error) => app.notice = app.ui.t(&error).into(),
                },
                Ok(None) => {}
                Err(error) => app.notice = error,
            }
        }
        Action::CancelStorage => {
            app.notice = match crate::storage::cancel() {
                Ok(()) => app.ui.t("Storage location change cancelled.").into(),
                Err(error) => error,
            };
        }
        Action::BackgroundImage => {
            if let Some(path) = open_image(hwnd) {
                match std::fs::read(&path)
                    .map_err(|e| e.to_string())
                    .and_then(|bytes| crate::catalog::decode_background(&bytes, 768))
                {
                    Ok(image) => {
                        let key = format!("local:{}", path.display());
                        app.images.insert(key.clone(), Arc::new(image));
                        app.custom_background = key;
                        app.ui.background_file = path.to_string_lossy().into_owned();
                        app.ui.background_mode = 2;
                        app.ui.solid = true;
                        material(hwnd, app);
                        app.background_source.clear();
                        update_background(hwnd, app);
                        app.ui.save_preferences();
                    }
                    Err(error) => app.notice = error,
                }
            }
        }
        Action::Shelf(title, delta) => {
            let count = app
                .ui
                .page
                .as_ref()
                .and_then(|p| p.sections.iter().find(|s| s.title == title))
                .map(|s| {
                    if app.ui.view == View::Explore && app.ui.filter != 0 {
                        let filter_kind = ["", "track", "artist", "playlist", "album", "podcast"]
                            [app.ui.filter.min(5)];
                        s.entries.iter().filter(|e| e.kind == filter_kind).count()
                    } else {
                        s.entries.len()
                    }
                })
                .unwrap_or(0);
            app.ui.step_shelf(&title, delta, count);
        }
        Action::Api(name, args) => {
            data_request(app, crate::catalog::Job::Api(name.into(), args));
            app.notice = format!("Loading from {}...", app.ui.provider.label());
        }
        Action::OpenItem(item) if app.ui.view == View::PlaylistPicker => {
            if item.kind == "playlist" {
                if let Some(track) = app.ui.playlist_track.clone() {
                    submit_playlist(app, crate::playlist::Mutation::Add(item.id, track.id));
                }
            }
        }
        Action::OpenItem(item) => {
            if item.kind == "offline_collection" {
                navigate(app, View::Offline, Some(item), String::new());
                load_page(app);
            } else if item.playable() {
                let mut items = Vec::new();
                if let Some(page) = &app.ui.page {
                    if page.rows.iter().any(|e| e.id == item.id) {
                        items = crate::ui::sorted_page_rows(&app.ui, page)
                            .into_iter()
                            .filter(|e| e.playable())
                            .cloned()
                            .collect();
                    } else if let Some(section) = page
                        .sections
                        .iter()
                        .find(|s| s.entries.iter().any(|e| e.id == item.id))
                    {
                        items = section
                            .entries
                            .iter()
                            .filter(|e| e.playable())
                            .cloned()
                            .collect();
                    } else {
                        items.extend(page.rows.iter().filter(|e| e.playable()).cloned());
                        for section in &page.sections {
                            items.extend(section.entries.iter().filter(|e| e.playable()).cloned());
                        }
                    }
                }
                if items.is_empty() {
                    items.push(item.clone());
                }
                let index = items.iter().position(|e| e.id == item.id).unwrap_or(0);
                app.ui.queue = items.clone();
                app.ui.queue_index = index;
                app.ui.now = item.clone();
                command(app, Command::PlayEntry(Box::new(item), items));
                app.notice = "Loading track...".into();
            } else {
                let view = match item.kind.as_str() {
                    "artist" => View::Artist,
                    "podcast" => View::Show,
                    "flow" => View::Flow,
                    _ => View::Album,
                };
                navigate(
                    app,
                    view,
                    if view == View::Flow { None } else { Some(item) },
                    String::new(),
                );
                load_page(app);
            }
        }
        Action::PlayAll => {
            if let Some(page) = app.ui.page.clone() {
                let mut items: Vec<_> = crate::ui::sorted_page_rows(&app.ui, &page)
                    .into_iter()
                    .filter(|e| e.playable())
                    .cloned()
                    .collect();
                if items.is_empty() {
                    items = page
                        .sections
                        .iter()
                        .flat_map(|s| s.entries.iter())
                        .filter(|e| e.playable())
                        .cloned()
                        .collect();
                }
                if let Some(first) = items.first() {
                    app.ui.queue = items.clone();
                    app.ui.queue_index = 0;
                    app.ui.now = first.clone();
                    command(app, Command::PlayEntry(Box::new(first.clone()), items));
                    app.notice = "Loading track...".into();
                } else {
                    app.notice = "This page contains no playable tracks.".into();
                }
            }
        }
        Action::RefreshFlow => {
            if app.ui.view == View::Flow && !app.ui.page.as_ref().is_some_and(|p| p.loading) {
                let key = page_key(app);
                app.catalog.session.refresh_flow(&key);
                app.page_cache.retain(|(k, _)| *k != key);
                load_page(app);
            }
        }
        Action::More => {
            if let Some(page) = app.ui.page.clone().filter(|page| !page.loading) {
                let mut loading = (*page).clone();
                loading.loading = true;
                app.ui.page = Some(Arc::new(loading));
                data_request(app, crate::catalog::Job::More((*page).clone()));
            }
        }
        Action::FavoriteNow => {
            let item = app.ui.now.clone();
            if !item.id.is_empty() {
                toggle_library(app, item);
            }
        }
        Action::MuteVolume => {
            if app.volume > 0 {
                app.ui.unmuted_volume = app.volume;
                app.volume = 0;
            } else {
                app.volume = app.ui.unmuted_volume.clamp(1, 100);
            }
            app.ui.initial_volume = app.volume;
            if let Some(&slider) = app.edits.get(&VOLUME) {
                SendMessageW(slider, TBM_SETPOS, 1, app.volume as isize);
            }
            command(app, Command::Volume(app.volume));
            app.ui.save_preferences();
        }
        Action::PlayerFullscreen => {
            app.ui.go(View::Lyrics);
            app.ui.lyrics_fullscreen = true;
        }
        Action::Favorite => {
            let item = app
                .entity
                .clone()
                .unwrap_or_else(|| app.snapshot.now.clone());
            if !item.id.is_empty() {
                toggle_library(app, item);
            }
        }
        Action::Download | Action::RepairDownloads => {
            let repairing = matches!(action, Action::RepairDownloads);
            if !app.ui.provider.supports_downloads() {
                app.notice = "Downloads are unavailable for Spotify Soloist.".into();
                return;
            }
            let Some(engine) = app.snapshot.engine.clone() else {
                app.notice = "Connect to Deezer before downloading.".into();
                rebuild(hwnd, app);
                return;
            };
            let mut items = app
                .ui
                .page
                .as_ref()
                .map(|p| {
                    p.rows
                        .iter()
                        .filter(|e| e.playable())
                        .cloned()
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            if repairing {
                items = match crate::downloads::missing_downloads() {
                    Ok(items) => items,
                    Err(error) => {
                        app.notice = error;
                        return;
                    }
                };
            }
            if !repairing && items.is_empty() && !app.ui.now.id.is_empty() {
                items.push(app.ui.now.clone());
            }
            if items.is_empty() {
                app.notice = if repairing {
                    "No missing downloads found."
                } else {
                    "Open a track collection before downloading."
                }
                .into();
            } else {
                let id = app.ui.downloads.iter().map(|j| j.id).max().unwrap_or(0) + 1;
                let cancel = Arc::new(std::sync::atomic::AtomicBool::new(false));
                let title = app
                    .ui
                    .page
                    .as_ref()
                    .map(|p| p.title.clone())
                    .unwrap_or_else(|| app.ui.now.title.clone());
                let collection = app
                    .ui
                    .page
                    .as_ref()
                    .and_then(|p| p.entity.as_ref())
                    .filter(|e| matches!(e.kind.as_str(), "album" | "playlist" | "podcast"));
                for item in &mut items {
                    if repairing {
                        continue;
                    }
                    if let Some(collection) = collection {
                        item.raw["_download_groups"] = serde_json::json!([collection.wire()]);
                    } else {
                        item.raw["_download_single"] = true.into();
                    }
                }
                let total = items.len();
                if let Err(error) = app.downloads.enqueue(crate::downloads::Request {
                    id,
                    engine,
                    items,
                    quality: app.ui.value("Download quality"),
                    cancelled: cancel.clone(),
                }) {
                    app.notice = error;
                } else {
                    app.download_cancels.insert(id, cancel);
                    app.ui.downloads.push(crate::downloads::Status {
                        id,
                        title,
                        total,
                        message: "Queued".into(),
                        ..Default::default()
                    });
                    app.notice = "Download queued".into();
                }
            }
        }
        Action::Go(View::Account)
            if !app.snapshot.connected && app.ui.provider == crate::catalog::Provider::Deezer =>
        {
            app.ui.deezer_login_open = true;
            app.ui.deezer_login_advanced = false;
        }
        Action::EditPlaylist(item) => {
            if !crate::playlist::can_edit(&item, &app.ui.account) {
                return;
            }
            if app.ui.playlist_busy {
                app.notice = "Wait for the playlist update to finish".into();
            } else {
                app.ui.playlist_draft = crate::playlist::Draft::from_entry(Some(&item));
                navigate(app, View::PlaylistEdit, None, String::new());
                set_playlist_fields(app);
            }
        }
        Action::ChoosePlaylist(track) => {
            app.ui.playlist_track = Some(track);
            navigate(app, View::PlaylistPicker, None, String::new());
            load_page(app);
        }
        Action::ClearPlaylistDescription => {
            if !app.ui.playlist_busy {
                app.ui.playlist_draft.description.clear();
                app.ui.playlist_draft.description_changed = true;
                SetWindowTextW(app.edits[&DESCRIPTION], wide("").as_ptr());
            }
        }
        Action::ChoosePlaylistCover => {
            if !app.ui.playlist_busy {
                if let Some(path) = open_image(hwnd) {
                    match app.ui.playlist_draft.choose(&path) {
                        Ok(()) => update_playlist_preview(app),
                        Err(error) => app.notice = error,
                    }
                }
            }
        }
        Action::CropPlaylistCover(x, y, zoom) => {
            if !app.ui.playlist_busy {
                let draft = &mut app.ui.playlist_draft;
                draft.x = (draft.x + x).clamp(0.0, 1.0);
                draft.y = (draft.y + y).clamp(0.0, 1.0);
                draft.zoom = (draft.zoom + zoom).clamp(1.0, 4.0);
                update_playlist_preview(app);
            }
        }
        Action::ExportPlaylistCover => match app.ui.playlist_draft.jpeg() {
            Ok(bytes) => {
                if let Some(path) = playlist_export_path(hwnd) {
                    app.notice = match std::fs::write(path, bytes) {
                        Ok(()) => "Cover exported".into(),
                        Err(e) => e.to_string(),
                    };
                }
            }
            Err(e) => app.notice = e,
        },
        Action::DeletePlaylist => {
            let id = app.ui.playlist_draft.id.clone();
            if !id.is_empty()
                && !app.ui.playlist_busy
                && MessageBoxW(
                    hwnd,
                    wide(
                        app.ui
                            .t(if app.ui.provider == crate::catalog::Provider::Spotify {
                                "Remove this playlist from your Spotify library?"
                            } else {
                                "Delete this playlist from Deezer? This cannot be undone."
                            }),
                    )
                    .as_ptr(),
                    wide(app.ui.t("Delete playlist")).as_ptr(),
                    MB_YESNO | MB_ICONWARNING | MB_DEFBUTTON2,
                ) == IDYES
            {
                submit_playlist(app, crate::playlist::Mutation::Delete(id));
            }
        }
        Action::Go(view) => {
            if matches!(view, View::PlaylistEdit | View::CoverEdit) && app.ui.playlist_busy {
                return;
            }
            if view == View::PlaylistEdit {
                app.ui.playlist_draft = crate::playlist::Draft::from_entry(None);
                set_playlist_fields(app);
            }
            if view == View::CoverEdit {
                capture_playlist_fields(app);
            }

            let view = if view == View::Account {
                View::Settings
            } else {
                view
            };
            navigate(app, view, None, String::new());
            app.generation += 1;
            app.catalog.set_generation(app.generation);

            if view == View::FlowTune {
                app.ui.flow_loaded = false;
                app.ui.genres.clear();
                app.ui.flow_filter.clear();
                app.ui.flow_genre_limit = 30;
                SetWindowTextW(app.edits[&SEARCH], wide("").as_ptr());
                {
                    data_request(
                        app,
                        crate::catalog::Job::Api("DZFlowConfigsJSON".into(), vec![]),
                    );
                }
            }
            settle_scroll(app, 0);
            app.notice.clear();
            if view == View::Lyrics {
                start_animation_timer(hwnd, 2);
                let id = app.snapshot.now.id.clone();
                if !id.is_empty() {
                    app.lyrics_id = id.clone();
                    data_request(
                        app,
                        crate::catalog::Job::Lyrics(
                            app.snapshot.now.clone(),
                            id,
                            app.ui.lyrics_sources & crate::lyrics::SUPPORTED_SOURCES,
                        ),
                    );
                }
            } else if view == View::TrackInfo {
                data_request(
                    app,
                    crate::catalog::Job::TrackInfo(
                        app.ui
                            .track_info
                            .as_ref()
                            .map(|info| info.0.clone())
                            .unwrap_or_else(|| app.snapshot.now.clone()),
                    ),
                );
            } else {
                load_page(app);
            }
        }
        Action::Back => {
            if let Some((view, entity, query, scroll)) = app.navigation.pop() {
                app.ui.back();
                app.ui.view = view;
                app.entity = entity;
                app.query = query;
                if view == View::Lyrics {
                    settle_scroll(app, 0);
                    app.ui.lyrics_manual_scroll = false;
                } else if app.ui.scroll_animation {
                    app.scroll_target = scroll;
                    app.scroll_origin = app.ui.scroll;
                    app.scroll_position = app.ui.scroll as f64;
                    app.scroll_tick = std::time::Instant::now();
                    app.scroll_started = std::time::Instant::now();
                    start_animation_timer(hwnd, 2);
                } else {
                    app.ui.scroll = scroll;
                    app.scroll_target = scroll;
                }
                app.ui.shelf_offsets.clear();
            } else if app.ui.is_subpage() {
                app.ui.view = app.ui.fallback_parent_view();
                app.entity = None;
                app.query.clear();
                settle_scroll(app, 0);
            }
            app.notice.clear();
            load_page(app);
        }
        Action::Connect => {
            login(hwnd, app);
            return;
        }
        Action::ConnectSaved => {
            SetWindowTextW(app.edits[&SESSION], wide("").as_ptr());
            login(hwnd, app);
            return;
        }
        Action::StartSpotify => {
            if app.spotify_setup.is_none() {
                crate::spotify::remote::select_device(None);
                start_spotify(app);
                command(app, Command::SetPlaying(true));
            }
        }
        Action::OpenSpotify => open_spotify(hwnd, app),
        Action::ReconnectSpotifyOutput => {
            app.notice = match crate::spotify::service::reopen_output() {
                Ok(()) => "Reconnecting Spotify audio output...".into(),
                Err(error) => error,
            };
        }
        Action::OpenSpotifyOutputSettings => {
            if ShellExecuteW(
                hwnd,
                wide("open").as_ptr(),
                wide("ms-settings:apps-volume").as_ptr(),
                null(),
                null(),
                SW_SHOWNORMAL,
            ) as isize
                <= 32
            {
                app.notice = "Could not open Windows Volume mixer. Open Settings > System > Sound > Volume mixer.".into();
            }
        }
        Action::SpotifySetup => {
            if app.spotify_setup.is_none() {
                app.spotify_service.take();
                app.ui.spotify_phase = None;
                let cancel = Arc::new(std::sync::atomic::AtomicBool::new(false));
                let flag = cancel.clone();
                let translations = app.ui.translations.clone();
                let palette = app.raw_target_palette();
                let (send, receive) = std::sync::mpsc::sync_channel(1);
                match std::thread::Builder::new()
                    .name("spotify-setup".into())
                    .spawn(move || {
                        let _ =
                            send.send(crate::spotify::wizard::run(&flag, translations, palette));
                    }) {
                    Ok(_) => {
                        app.spotify_setup = Some((cancel, receive));
                        app.ui.spotify_setup_busy = true;
                        app.ui.spotify_setup_status =
                            "Complete setup in the Spotify window.".into();
                    }
                    Err(_) => app.notice = "Could not open Spotify setup.".into(),
                }
            }
        }
        Action::DeezerLoginOptions => {
            app.ui.deezer_login_open = true;
            app.ui.deezer_login_advanced = false;
        }
        Action::DeezerLoginAdvanced(advanced) => {
            app.ui.deezer_login_advanced = advanced;
            if !advanced {
                SetWindowTextW(app.edits[&SESSION], wide("").as_ptr());
            }
        }
        Action::CloseDeezerLogin => {
            if let Some((cancel, _)) = app.deezer_auth.take() {
                cancel.store(true, std::sync::atomic::Ordering::Relaxed);
            }
            app.ui.deezer_login_busy = false;
            app.ui.deezer_login_open = false;
            app.ui.deezer_login_advanced = false;
        }
        Action::ConnectDeezerWebView => {
            if app.deezer_auth.is_none() && app.pending.is_none() {
                let cancel = Arc::new(std::sync::atomic::AtomicBool::new(false));
                let flag = cancel.clone();
                let (send, receive) = std::sync::mpsc::sync_channel(1);
                match std::thread::Builder::new()
                    .name("deezer-login".into())
                    .spawn(move || {
                        let _ = send.send(crate::deezer_webview::run(&flag));
                    }) {
                    Ok(_) => {
                        app.deezer_auth = Some((cancel, receive));
                        app.ui.deezer_login_busy = true;
                        app.notice = "Finish Deezer sign-in in the login window.".into();
                        app.ui.deezer_login_status = "Complete sign-in in the login window.".into();
                    }
                    Err(_) => app.notice = "Could not start Deezer sign-in.".into(),
                }
            }
        }
        Action::BindShortcut(index) => {
            if index < app.ui.shortcuts.len() {
                app.ui.shortcut_capture = Some(index);
                app.ui.shortcut_errors[index].clear();
                if let Some(shortcuts) = &mut app.shortcuts {
                    shortcuts.clear();
                }
                SetFocus(hwnd);
            }
        }
        Action::ResetShortcut(index) => {
            if index < app.ui.shortcuts.len() {
                if let Some(shortcuts) = &mut app.shortcuts {
                    let _ = shortcuts.set(index, None);
                }
                app.ui.shortcuts[index] = None;
                app.ui.shortcut_errors[index].clear();
                app.ui.save_preferences();
            }
        }
        Action::Logout => {
            app.page_cache.clear();
            app.catalog.session.invalidate();
            app.library_loaded = false;
            app.podcast_ids.clear();
            app.library_pending = None;
            app.ui.liked_ids.retain(|key| !key.starts_with("podcast:"));
            command(app, Command::Logout);
            app.notice.clear();
        }

        Action::Toggle => {
            command(app, Command::Toggle);
            app.notice.clear();
            if app.ui.view == View::Lyrics {
                start_animation_timer(hwnd, 2);
            }
        }
        Action::Stop => {
            command(app, Command::Stop);
            app.notice.clear();
        }
        Action::PlayId => {
            let id = read(app.edits[&TRACK]);
            if !app.snapshot.connected {
                app.notice = "Connect to Deezer first.".into();
            } else if id.trim().is_empty() {
                app.notice = "Enter a Deezer track ID.".into();
            } else {
                command(app, Command::Play(id));
                app.notice = "Loading track...".into();
            }
        }
        Action::Tab(tab) => {
            app.ui.tab = tab;
            app.ui.scroll = 0;
            app.scroll_target = 0;
            if tab == 3 && app.ui.provider == crate::catalog::Provider::Deezer {
                data_request(
                    app,
                    crate::catalog::Job::Api("DZAudioDevicesJSON".into(), vec![]),
                );
            }
            if tab == 4 && app.ui.provider == crate::catalog::Provider::Deezer {
                data_request(app, crate::catalog::Job::Api("DZEQJSON".into(), vec![]));
            }
        }
        Action::Filter(index) => {
            app.ui.filter = index;
            app.ui.scroll = 0;
            app.notice.clear();
        }
        Action::Setting(key) => {
            let values: Vec<String> = if key == "Audio output device" && !app.ui.devices.is_empty()
            {
                app.ui
                    .devices
                    .iter()
                    .map(|(_, name)| name.clone())
                    .collect()
            } else {
                ui::options(key).iter().map(|v| (*v).to_string()).collect()
            };
            let menu = CreatePopupMenu();
            for (index, value) in values.iter().enumerate() {
                let flags = if key == "Show on startup"
                    && value == "Offline"
                    && !app.ui.provider.supports_downloads()
                {
                    MF_STRING | MF_GRAYED
                } else {
                    MF_STRING
                };
                AppendMenuW(menu, flags, index + 1, wide(app.ui.t(value)).as_ptr());
            }
            let mut point = zeroed();
            GetCursorPos(&mut point);
            let selected = TrackPopupMenu(
                menu,
                TPM_RETURNCMD | TPM_NONOTIFY,
                point.x,
                point.y,
                0,
                hwnd,
                null(),
            );
            DestroyMenu(menu);
            if selected > 0 {
                app.ui.choices.insert(key, (selected - 1) as usize);
                if key == "Status name" {
                    command(
                        app,
                        Command::Discord(serde_json::json!({"status_name": selected - 1})),
                    );
                }
                if key == "Language" {
                    app.ui
                        .set_language(["", "tr", "en", "az"][(selected - 1) as usize]);
                    app.notice.clear();
                } else if key == "Background" {
                    let value = selected - 1;
                    app.ui.background_mode = value as usize;
                    app.ui.solid = value != 0;
                    material(hwnd, app);
                    app.background_source.clear();
                    update_background(hwnd, app);
                    app.notice.clear();
                } else {
                    let index = selected - 1;
                    let setting = match key {
                        "Audio quality" => Some(("DZSetQuality", vec![serde_json::json!(index)])),
                        "Normalize volume (ReplayGain)" => {
                            Some(("DZSetReplayGain", vec![serde_json::json!(index)]))
                        }
                        "Gapless playback" => {
                            Some(("DZSetGapless", vec![serde_json::json!(index)]))
                        }
                        "Crossfade (seconds)" => {
                            Some(("DZSetCrossfadeMS", vec![serde_json::json!(index * 1000)]))
                        }
                        "Song disk buffering" => {
                            Some(("DZSetSongDiskBuffering", vec![serde_json::json!(index)]))
                        }
                        "Equalizer" => Some((
                            "DZSetEQJSON",
                            vec![serde_json::json!(
                                serde_json::json!({"enabled":index!=0}).to_string()
                            )],
                        )),
                        "Preset" => {
                            let preset_name = &values[index as usize];
                            app.ui.apply_preset(preset_name);
                            let update = if preset_name.eq_ignore_ascii_case("Custom") {
                                let gains: Vec<f64> =
                                    app.ui.eq[..10].iter().map(|&g| g as f64).collect();
                                serde_json::json!({
                                    "preset": "custom",
                                    "gainsDb": gains,
                                    "preampDb": app.ui.eq[10],
                                    "enabled": true,
                                })
                            } else {
                                serde_json::json!({
                                    "preset": preset_name.to_lowercase().replace(' ', "-"),
                                    "enabled": true,
                                })
                            };
                            Some(("DZSetEQJSON", vec![serde_json::json!(update.to_string())]))
                        }
                        "Audio output device" => Some((
                            "DZSetAudioDevice",
                            vec![serde_json::json!(app
                                .ui
                                .devices
                                .get(index as usize)
                                .map(|(id, _)| id.as_str())
                                .unwrap_or(""))],
                        )),
                        "Sleep timer" => Some((
                            "DZSetSleepTimer",
                            vec![
                                serde_json::json!([0, 15, 30, 60][index as usize]),
                                serde_json::json!(0),
                            ],
                        )),
                        _ => None,
                    };
                    if let Some((name, args)) = setting {
                        command(app, Command::Api(name.into(), args));
                        app.notice.clear();
                    } else {
                        app.notice = "Selection updated.".into();
                    }
                }
                app.ui.save_preferences();
            }
        }
        Action::Eq(band, delta) => {
            let new_gain = app.ui.eq[band] + delta;
            apply_eq(app, band, new_gain);
            app.notice.clear();
        }
        Action::Shuffle => {
            app.ui.shuffle = !app.ui.shuffle;
            command(app, Command::Shuffle(app.ui.shuffle));
            app.notice.clear();
        }
        Action::Repeat => {
            app.ui.repeat = (app.ui.repeat + 1) % 3;
            command(app, Command::Repeat(app.ui.repeat));
            app.notice.clear();
        }
        Action::Previous | Action::Next => {
            if matches!(action, Action::Previous) && app.snapshot.position > 3000 {
                app.snapshot.position = 0;
                app.playback_clock = PlaybackClock::default();
                app.ui.position = 0;
            } else if !app.ui.queue.is_empty()
                && !(matches!(action, Action::Next) && app.snapshot.radio_active)
            {
                if matches!(action, Action::Previous) {
                    app.ui.queue_index = app.ui.queue_index.saturating_sub(1);
                } else {
                    app.ui.queue_index = (app.ui.queue_index + 1) % app.ui.queue.len();
                }
                if let Some(entry) = app.ui.queue.get(app.ui.queue_index) {
                    app.ui.now = entry.clone();
                }
            }
            command(
                app,
                if matches!(action, Action::Previous) {
                    Command::Previous
                } else {
                    Command::Next
                },
            );
            app.notice.clear();
        }
        Action::Notice(message) => app.notice = message.into(),
        Action::SavePlaylist => {
            if app.ui.playlist_busy {
                return;
            }
            capture_playlist_fields(app);
            let draft = &app.ui.playlist_draft;
            let cover = if draft.image.is_some() {
                match draft.jpeg() {
                    Ok(bytes) => Some(bytes),
                    Err(e) => {
                        app.notice = e;
                        rebuild(hwnd, app);
                        return;
                    }
                }
            } else {
                None
            };
            let mutation = crate::playlist::Mutation::Save {
                id: draft.id.clone(),
                title: draft.title.clone(),
                description: (draft.id.is_empty() || draft.description_changed)
                    .then(|| draft.description.clone()),
                cover,
                seed: draft.seed.as_ref().map(|e| e.id.clone()),
            };
            submit_playlist(app, mutation);
        }
        Action::Search => {
            let query = read(app.edits[&SEARCH]);
            navigate(app, View::Explore, None, query);
            app.notice = format!("Searching {}...", app.ui.provider.label());
            load_page(app);
        }
        Action::Close => {
            DestroyWindow(hwnd);
            return;
        }
        Action::SetControlAlignment(right) => {
            app.ui.controls_right = right;
            apply_window_controls(hwnd, app);
            app.ui.save_preferences();
        }
        Action::ToggleSidebar => {
            app.ui.sidebar_collapsed = !app.ui.sidebar_collapsed;
            app.ui.save_preferences();
        }
        Action::ToggleLyricsVolume => {
            app.ui.lyrics_volume_open = !app.ui.lyrics_volume_open;
        }
        Action::TogglePodcastDescription(id) => {
            if app.ui.expanded_podcast_description.as_ref() == Some(&id) {
                app.ui.expanded_podcast_description = None;
                app.ui.scroll = 0;
                app.scroll_target = 0;
                app.scroll_origin = 0;
            } else {
                app.ui.expanded_podcast_description = Some(id);
            }
        }
        Action::ToggleQueueSidebar => {
            app.ui.toggle_queue_sidebar();
            if app.ui.queue_sidebar_open {
                rebuild(hwnd, app);
                let anchor = app.scene.queue_anchor;
                app.ui.queue_scroll = anchor / 24;
                app.ui.queue_scroll_offset = anchor % 24;
            }
        }
        Action::QueueReturn => {
            app.queue_glide = None;
            app.ui.queue_scroll = app.scene.queue_anchor / 24;
            app.ui.queue_scroll_offset = app.scene.queue_anchor % 24;
        }
        Action::LyricsFollow => {
            app.ui.lyrics_manual_until = None;
            if app.ui.resume_lyric_follow(std::time::Instant::now()) {
                let previous_scroll = app.ui.scroll;
                settle_scroll(app, 0);
                rebuild(hwnd, app);
                if let Some(anchor) = app.scene.lyrics_anchor {
                    let offset = anchor - previous_scroll;
                    app.lyric_motion = Some((std::time::Instant::now(), offset));
                    app.lyric_motion_offset = offset;
                    shift_scroll_content(&mut app.scene, offset);
                    let (width, height) = size(hwnd, app);
                    position_content_buttons(app, width, height);
                    start_animation_timer(hwnd, 2);
                }
            }
        }
        Action::QueueJump(idx) => {
            if let Some(entry) = app.ui.queue.get(idx).cloned() {
                app.ui.queue_index = idx;
                app.ui.now = entry.clone();
                command(app, Command::PlayQueueIndex(idx));
                app.notice = format!("Playing: {}", entry.title);
            }
        }
        Action::QueueClear => {
            app.ui.queue.clear();
            app.ui.queue_index = 0;
            command(app, Command::QueueClear);
            app.notice = "Queue cleared".into();
        }
        Action::QueueReset => {
            app.ui.queue.clear();
            app.ui.queue_index = 0;
            if !app.snapshot.now.id.is_empty() {
                let now = app.snapshot.now.clone();
                app.ui.queue.push(now.clone());
                command(app, Command::PlayEntry(Box::new(now.clone()), vec![now]));
            } else {
                command(app, Command::QueueClear);
            }
            app.notice = "Queue reset".into();
        }
        Action::SeekMs(ms) => {
            app.ui.lyrics_manual_scroll = false;
            app.ui.lyrics_manual_until = None;
            settle_scroll(app, 0);
            command(app, Command::SeekMs(ms));
            app.snapshot.position = ms;
            app.playback_clock
                .sample(ms, std::time::Instant::now(), false);
            app.ui.position = ms;
        }
        Action::ToggleLyricsCoverMode => {
            app.ui.lyrics_cover_mode = !app.ui.lyrics_cover_mode;
            let from = app
                .ui
                .lyrics_cover_mix
                .unwrap_or(if app.ui.lyrics_cover_mode { 0. } else { 1. });
            let target = if app.ui.lyrics_cover_mode { 1. } else { 0. };
            app.ui.lyrics_cover_mix = Some(from);
            app.lyrics_mode_motion = Some((std::time::Instant::now(), from, target));
            start_animation_timer(hwnd, 2);
        }
        Action::ToggleLyricsVisualizer => {
            if !app.ui.lyrics_visualizer {
                app.ui.lyrics_visualizer = true;
                app.ui.visualizer_style = 0;
            } else if app.ui.visualizer_style < 2 {
                app.ui.visualizer_style += 1;
            } else {
                app.ui.lyrics_visualizer = false;
                app.ui.visualizer_style = 0;
            }
            app.ui.save_preferences();
            if app.ui.lyrics_visualizer && app.playing() {
                start_animation_timer(hwnd, 2);
            }
            rebuild(hwnd, app);
        }
        Action::VisualizerStyle(style) => {
            app.ui.visualizer_style = style.min(2);
            app.ui.lyrics_visualizer = true;
            app.ui.save_preferences();
            if app.playing() {
                start_animation_timer(hwnd, 2);
            }
            rebuild(hwnd, app);
        }
        Action::CornerStyle(style) => {
            app.ui.corner_style = style;
            app.ui.save_preferences();
            material(hwnd, app);
        }
        Action::CustomTheme(light) => {
            let saved = if light {
                &app.ui.custom_light_colors
            } else {
                &app.ui.custom_dark_colors
            }
            .clone();
            let colors = if saved.is_empty() {
                let mut defaults = ui::Ui::default();
                defaults.apply_solid_theme(if light { 12 } else { 1 });
                if light {
                    defaults.light_colors
                } else {
                    defaults.dark_colors
                }
            } else {
                saved
            };
            app.ui.custom_theme_draft = Some((light, colors));
        }
        Action::CloseCustomTheme => {
            app.ui.custom_theme_draft = None;
        }
        Action::ResetCustomTheme => {
            if let Some((light, colors)) = &mut app.ui.custom_theme_draft {
                let mut defaults = ui::Ui::default();
                defaults.apply_solid_theme(if *light { 12 } else { 1 });
                *colors = if *light {
                    defaults.light_colors
                } else {
                    defaults.dark_colors
                };
            }
        }
        Action::SaveCustomTheme => {
            if let Some((light, colors)) = app.ui.custom_theme_draft.take() {
                if light {
                    app.ui.light_colors = colors.clone();
                    app.ui.custom_light_colors = colors;
                } else {
                    app.ui.dark_colors = colors.clone();
                    app.ui.custom_dark_colors = colors;
                }
                app.ui.light = light;
                app.ui.solid_color = app.ui.color("Background", app.ui.solid_color);
                app.ui.solid_theme = ui::SOLID_THEMES.len() + usize::from(light);
                app.ui.save_preferences();
                material(hwnd, app);
            }
        }
        Action::SolidTheme(theme) => {
            app.ui.apply_solid_theme(theme);
            app.ui.save_preferences();
            material(hwnd, app);
        }
        Action::PlayerLayout(layout) => {
            app.ui.player_layout = layout;
            app.ui.save_preferences();
        }
        Action::ToggleBool(key) => {
            let new_val = match key {
                "auto_updates" => {
                    app.ui.auto_updates = !app.ui.auto_updates;
                    app.ui.auto_updates
                }
                "adaptive_colors" => {
                    app.ui.adaptive_colors = !app.ui.adaptive_colors;
                    update_background(hwnd, app);
                    material(hwnd, app);
                    app.ui.adaptive_colors
                }
                "light_mode" => {
                    app.ui.light = !app.ui.light;
                    if app.ui.light {
                        if app.ui.light_opacity < 35 {
                            app.ui.light_opacity = 75;
                        }
                        let theme = if app.ui.solid_theme >= 12 {
                            app.ui.solid_theme
                        } else {
                            12
                        };
                        if app.ui.light_colors.is_empty() || app.ui.background_mode == 3 {
                            app.ui.apply_solid_theme(theme);
                        } else {
                            app.ui.solid_color = app.ui.color("Background", 0x00f8f4f2);
                        }
                    } else {
                        let theme = if app.ui.solid_theme < 12 {
                            app.ui.solid_theme
                        } else {
                            1
                        };
                        if app.ui.dark_colors.is_empty() || app.ui.background_mode == 3 {
                            app.ui.apply_solid_theme(theme);
                        } else {
                            app.ui.solid_color = app.ui.color("Background", 0x00171717);
                        }
                    }
                    update_background(hwnd, app);
                    material(hwnd, app);
                    app.ui.light
                }

                "keep_playing_on_close" => {
                    app.ui.keep_playing_on_close = !app.ui.keep_playing_on_close;
                    app.ui.keep_playing_on_close
                }
                "show_in_tray" => {
                    app.ui.show_in_tray = !app.ui.show_in_tray;
                    app.ui.show_in_tray
                }
                "window_controls" => {
                    app.ui.window_controls = !app.ui.window_controls;
                    app.ui.window_controls
                }
                "controls_right" => {
                    app.ui.controls_right = !app.ui.controls_right;
                    app.ui.controls_right
                }
                "traffic_lights" => {
                    app.ui.traffic_lights = !app.ui.traffic_lights;
                    app.ui.traffic_lights
                }
                "keep_played_tracks" => {
                    app.ui.keep_played_tracks = !app.ui.keep_played_tracks;
                    app.ui.keep_played_tracks
                }
                "jam_enabled" => {
                    app.ui.jam_enabled = !app.ui.jam_enabled;
                    if app.ui.jam_enabled {
                        app.connect = crate::connect::ConnectHub::start(
                            crate::connect::DEFAULT_PORT,
                            app.worker.commands.clone(),
                        );
                        if let Some(connect) = &app.connect {
                            connect.update(&app.snapshot);
                            app.ui.jam_url = connect.share_url();
                        } else {
                            app.ui.jam_enabled = false;
                            app.notice = "Could not start DeezNats Connect".into();
                        }
                    } else {
                        app.connect = None;
                        app.ui.jam_url.clear();
                    }
                    app.ui.jam_enabled
                }
                "autoplay" => {
                    app.ui.autoplay = !app.ui.autoplay;
                    command(app, Command::Autoplay(app.ui.autoplay));
                    app.ui.autoplay
                }
                "stay_awake" => {
                    app.ui.stay_awake = !app.ui.stay_awake;
                    app.ui.stay_awake
                }
                "window_blur" => {
                    app.ui.window_blur = !app.ui.window_blur;
                    material(hwnd, app);
                    app.ui.window_blur
                }
                "ui_blur" => {
                    app.ui.ui_blur = !app.ui.ui_blur;
                    material(hwnd, app);
                    app.ui.ui_blur
                }
                "transparent_player" => {
                    app.ui.transparent_player = !app.ui.transparent_player;
                    material(hwnd, app);
                    app.ui.transparent_player
                }
                "lyrics_visualizer" => {
                    app.ui.lyrics_visualizer = !app.ui.lyrics_visualizer;
                    if app.ui.lyrics_visualizer && app.playing() {
                        start_animation_timer(hwnd, 2);
                    }
                    app.ui.lyrics_visualizer
                }
                "karaoke_lyrics" => {
                    app.ui.karaoke_lyrics = !app.ui.karaoke_lyrics;
                    app.ui.karaoke_lyrics
                }
                "blur_inactive_lyrics" => {
                    app.ui.blur_inactive_lyrics = !app.ui.blur_inactive_lyrics;
                    app.ui.blur_inactive_lyrics
                }
                "show_on_discord" => {
                    app.ui.show_on_discord = !app.ui.show_on_discord;
                    app.ui.show_on_discord
                }
                "discord_paused" => {
                    app.ui.discord_paused = !app.ui.discord_paused;
                    app.ui.discord_paused
                }
                "discord_badge" => {
                    app.ui.discord_badge = !app.ui.discord_badge;
                    app.ui.discord_badge
                }
                "discord_hide_details" => {
                    app.ui.discord_hide_details = !app.ui.discord_hide_details;
                    app.ui.discord_hide_details
                }
                "discord_btn_service" => {
                    app.ui.discord_btn_service = !app.ui.discord_btn_service;
                    app.ui.discord_btn_service
                }
                "discord_btn_app" => {
                    app.ui.discord_btn_app = !app.ui.discord_btn_app;
                    app.ui.discord_btn_app
                }

                "smooth_scrolling" => {
                    app.ui.smooth_scrolling = !app.ui.smooth_scrolling;
                    app.ui.smooth_scrolling
                }
                "scroll_animation" => {
                    app.ui.scroll_animation = !app.ui.scroll_animation;
                    app.ui.scroll_animation
                }
                "expand_palettes" => {
                    app.ui.expanded_palettes = !app.ui.expanded_palettes;
                    app.ui.expanded_palettes
                }
                "expand_themes" => {
                    app.ui.expanded_themes = !app.ui.expanded_themes;
                    app.ui.expanded_themes
                }
                _ => false,
            };
            if let Some(option) = match key {
                "show_on_discord" => Some("enabled"),
                "discord_paused" => Some("show_paused"),
                "discord_badge" => Some("badge"),
                "discord_hide_details" => Some("hide_details"),
                "discord_btn_service" => Some("service_button"),
                "discord_btn_app" => Some("app_button"),
                _ => None,
            } {
                command(app, Command::Discord(serde_json::json!({option:new_val})));
            }
            if !matches!(key, "controls_right" | "expand_palettes" | "expand_themes") {
                let anim_key = format!("bool:{key}");
                let cur_t = app.toggle_progress(&anim_key, !new_val);
                let target_t = if new_val { 1.0 } else { 0.0 };
                app.toggle_animations
                    .insert(anim_key, (cur_t, target_t, std::time::Instant::now()));
                start_animation_timer(hwnd, 2);
            }
            if matches!(key, "window_controls" | "controls_right" | "traffic_lights") {
                apply_window_controls(hwnd, app);
            }
            if key == "show_in_tray" {
                sync_tray(hwnd, app);
            }
            app.ui.save_preferences();
        }
        Action::AdjustLyricsSize(is_full, delta) => {
            if is_full {
                app.ui.lyrics_size_fullscreen =
                    (app.ui.lyrics_size_fullscreen as i32 + delta).clamp(60, 180) as u32;
            } else {
                app.ui.lyrics_size_panel =
                    (app.ui.lyrics_size_panel as i32 + delta).clamp(60, 180) as u32;
            }
            app.ui.save_preferences();
            rebuild(hwnd, app);
        }
        Action::ToggleLyricsFullscreen => {
            app.ui.lyrics_fullscreen = !app.ui.lyrics_fullscreen;
            rebuild(hwnd, app);
        }
        Action::ToggleLyricSource(idx) => {
            if (crate::lyrics::SUPPORTED_SOURCES & (1 << idx)) == 0 {
                return;
            }
            app.ui.lyrics_sources ^= 1 << idx;
            let enabled = (app.ui.lyrics_sources & (1 << idx)) != 0;
            let anim_key = format!("lyric:{idx}");
            let cur_t = app.toggle_progress(&anim_key, !enabled);
            let target_t = if enabled { 1.0 } else { 0.0 };
            app.toggle_animations
                .insert(anim_key, (cur_t, target_t, std::time::Instant::now()));
            start_animation_timer(hwnd, 2);
            app.ui.save_preferences();
            if !app.snapshot.now.id.is_empty() {
                let effective_mask = app.ui.lyrics_sources & crate::lyrics::SUPPORTED_SOURCES;
                data_request(
                    app,
                    crate::catalog::Job::Lyrics(
                        app.snapshot.now.clone(),
                        app.snapshot.now.id.clone(),
                        effective_mask,
                    ),
                );
            }
        }
        Action::StreamingQuality(idx) => {
            app.ui.choices.insert("Audio quality", idx);
            command(
                app,
                Command::Api("DZSetQuality".into(), vec![serde_json::json!(idx)]),
            );
            app.ui.save_preferences();
        }
        Action::EqMode(adv) => {
            app.ui.eq_advanced = adv;
        }
        Action::EqReset => {
            app.ui.eq = [0; 11];
            let flat_idx = ui::options("Preset")
                .iter()
                .position(|&p| p == "Flat")
                .unwrap_or(0);
            app.ui.choices.insert("Preset", flat_idx);
            command(
                app,
                Command::Api(
                    "DZSetEQJSON".into(),
                    vec![serde_json::json!("{\"reset\":true}")],
                ),
            );
            app.ui.save_preferences();
        }
        Action::OpenTrackInfo(entry) => {
            app.ui.track_info = Some((entry.clone(), entry.raw.clone(), serde_json::Value::Null));
            app.generation = app.generation.wrapping_add(1);
            data_request(app, crate::catalog::Job::TrackInfo(entry));
            navigate(
                app,
                View::TrackInfo,
                app.ui.track_info.as_ref().map(|info| info.0.clone()),
                String::new(),
            );
            app.notice.clear();
        }
    }
    rebuild(hwnd, app);
}
unsafe fn bitmap(dc: HDC, rect: RECT, image: &crate::catalog::Bitmap, radius: i32) {
    if crate::drawing::image(dc, rect, image, radius as f32 / 2.0, 255) {
        return;
    }
    let saved = SaveDC(dc);
    let region = CreateRoundRectRgn(
        rect.left,
        rect.top,
        rect.right + 1,
        rect.bottom + 1,
        radius,
        radius,
    );
    ExtSelectClipRgn(dc, region, RGN_AND);
    DeleteObject(region);
    let info = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: image.width as i32,
            biHeight: -(image.height as i32),
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB,
            ..zeroed()
        },
        ..zeroed()
    };
    SetStretchBltMode(dc, HALFTONE);
    StretchDIBits(
        dc,
        rect.left,
        rect.top,
        rect.right - rect.left,
        rect.bottom - rect.top,
        0,
        0,
        image.width as i32,
        image.height as i32,
        image.pixels.as_ptr().cast(),
        &info,
        DIB_RGB_COLORS,
        SRCCOPY,
    );
    RestoreDC(dc, saved);
}

thread_local! {static PAINT_PALETTE:std::cell::Cell<Option<[u32;9]>>=const {std::cell::Cell::new(None)};}
struct PaintPalette(Option<[u32; 9]>);
impl PaintPalette {
    fn set(palette: [u32; 9]) -> Self {
        Self(PAINT_PALETTE.with(|value| value.replace(Some(palette))))
    }
}
impl Drop for PaintPalette {
    fn drop(&mut self) {
        PAINT_PALETTE.with(|value| value.set(self.0));
    }
}
unsafe fn paint(hwnd: HWND, app: &App) {
    #[cfg(feature = "diagnostics")]
    let _timing = crate::diagnostics::span("ui.paint", app.ui.view.title());
    #[cfg(feature = "diagnostics")]
    let _page_allocations = crate::memory_profile::page_scope(app.ui.view as usize + 1 + 192);
    let mut ps = zeroed();
    let update_region = CreateRectRgn(0, 0, 0, 0);
    let region_kind = GetUpdateRgn(hwnd, update_region, 0);
    let dc = BeginPaint(hwnd, &mut ps);
    if app.render_suspended {
        DeleteObject(update_region);
        EndPaint(hwnd, &ps);
        return;
    }
    let saved = SaveDC(dc);
    // Leave active text editors to Windows so repainting the player cannot erase
    // the caret, IME composition, text selection, or partially typed queries.
    for id in [SEARCH, SESSION, TRACK, NAME, DESCRIPTION] {
        if let Some(control) = app.edits.get(&id) {
            if IsWindowVisible(*control) != 0 {
                let mut rect: RECT = zeroed();
                GetWindowRect(*control, &mut rect);
                MapWindowPoints(null_mut(), hwnd, (&mut rect as *mut RECT).cast(), 2);
                ExcludeClipRect(dc, rect.left, rect.top, rect.right, rect.bottom);
            }
        }
    }
    render_region(
        hwnd,
        app,
        dc,
        Some(ps.rcPaint),
        if region_kind == COMPLEXREGION {
            update_region
        } else {
            null_mut()
        },
    );
    DeleteObject(update_region);
    if crate::drawing::artwork_preparation_pending() {
        start_animation_timer(hwnd, 2);
    }
    RestoreDC(dc, saved);
    EndPaint(hwnd, &ps);
}
#[derive(Clone, Copy, PartialEq, Eq)]
struct BackdropKey {
    current: Option<u64>,
    previous: Option<u64>,
    width: i32,
    height: i32,
    caption: i32,
    progress: u32,
    tint: u32,
    opacity: u32,
    brightness: u32,
}

struct BackgroundFrame<'a> {
    current: Option<&'a crate::catalog::Bitmap>,
    previous: Option<&'a crate::catalog::Bitmap>,
    progress: u32,
    tint: u32,
    opacity: u32,
    brightness: u32,
}
impl BackgroundFrame<'_> {
    fn active_image(&self) -> (Option<&crate::catalog::Bitmap>, u32) {
        // 128 is the exact ambient midpoint, including when either texture is
        // absent. All rendering paths must use the same two-phase weights.
        if self.progress <= 128 {
            (self.previous, (128 - self.progress) * 255 / 128)
        } else {
            (self.current, (self.progress.min(255) - 128) * 255 / 127)
        }
    }

    pub fn fill_buffer(
        &self,
        dest: &mut [u32],
        width: i32,
        height: i32,
        y_offset: i32,
        full_height: i32,
    ) {
        if width <= 0 || height <= 0 || full_height <= 0 {
            return;
        }
        let w = width as usize;
        let h = height as usize;
        let fh = full_height as f64;
        let fw = width as f64;

        let (active_image, alpha) = self.active_image();

        let tint_base = self.tint;
        let opacity = self.opacity;
        let brightness = self.brightness;

        if let Some(img) = active_image {
            if img.width == 0 || img.height == 0 || img.pixels.is_empty() {
                dest[..w * h].fill(0xff171717);
                return;
            }
            let scale = (fw / img.width as f64).max(fh / img.height as f64);
            let offset_x = (img.width as f64 * scale - fw) / 2.0;
            let offset_y = (img.height as f64 * scale - fh) / 2.0;

            let mut x_lut = Vec::with_capacity(w);
            let img_max_x = img.width - 1;
            for x in 0..w {
                let sx = (((x as f64 + offset_x) / scale) as u32).min(img_max_x) as usize;
                x_lut.push(sx);
            }

            let img_max_y = img.height - 1;
            let img_w = img.width as usize;

            // Preserve both blend roundings exactly, but do the arithmetic only
            // 256 times per channel rather than for every display pixel.
            let mut colors = [[0u32; 256]; 3];
            for (channel, table) in colors.iter_mut().enumerate() {
                let shift = channel * 8;
                let tint = (tint_base >> shift) & 255;
                for (value, color) in table.iter_mut().enumerate() {
                    let faded = (value as u32 * alpha + tint * (255 - alpha)) / 255;
                    let tinted = (tint * opacity + faded * (255 - opacity)) / 255;
                    *color = ((tinted * brightness / 100).min(255)) << shift;
                }
            }
            let mut source_row = vec![0; img_w];
            let mut last_sy = None;
            for y in 0..h {
                let actual_y = y as i32 + y_offset;
                let sy = (((actual_y as f64 + offset_y) / scale) as u32).min(img_max_y) as usize;
                let row_offset = sy * img_w;
                let dest_row_start = y * w;
                if last_sy == Some(sy) {
                    dest.copy_within(dest_row_start - w..dest_row_start, dest_row_start);
                    continue;
                }
                last_sy = Some(sy);
                for (out, &pixel) in source_row
                    .iter_mut()
                    .zip(&img.pixels[row_offset..row_offset + img_w])
                {
                    *out = 0xff000000
                        | colors[0][(pixel & 255) as usize]
                        | colors[1][((pixel >> 8) & 255) as usize]
                        | colors[2][((pixel >> 16) & 255) as usize];
                }
                let dest_row = &mut dest[dest_row_start..dest_row_start + w];
                for (pixel, &sx) in dest_row.iter_mut().zip(&x_lut) {
                    *pixel = source_row[sx];
                }
            }
        } else {
            let mut color = tint_base | 0xff000000;
            if brightness != 100 {
                let mut bright = 0xff000000;
                for shift in [0, 8, 16] {
                    bright |= ((((color >> shift) & 255) * brightness / 100).min(255)) << shift;
                }
                color = bright;
            }
            dest[..w * h].fill(color);
        }
    }

    pub fn pixel(&self, x: i32, y: i32, width: i32, height: i32) -> u32 {
        let sample = |image: &crate::catalog::Bitmap| {
            crate::appearance::cover_sample(
                image,
                x as u32,
                y as u32,
                width.max(1) as u32,
                height.max(1) as u32,
            )
        };
        let tint_base = self.tint;
        let (image, alpha) = self.active_image();
        let mut color = match image {
            Some(image) => blend_pixel(sample(image), tint_base, alpha),
            None => tint_base | 0xff000000,
        };
        color = blend_pixel(self.tint, color, self.opacity);
        if self.brightness != 100 {
            let mut bright = 0xff000000;
            for shift in [0, 8, 16] {
                bright |= ((((color >> shift) & 255) * self.brightness / 100).min(255)) << shift;
            }
            color = bright;
        }
        color
    }
}
fn premultiplied_tint(color: u32, opacity: u32) -> u32 {
    let alpha = opacity.min(100) * 255 / 100;
    let color = dib(color);
    let mut pixel = alpha << 24;
    for shift in [0, 8, 16] {
        pixel |= (((color >> shift) & 255) * alpha / 255) << shift;
    }
    pixel
}
unsafe fn draw_caption_controls(
    hwnd: HWND,
    memory: HDC,
    ui: &Ui,
    width: i32,
    height: i32,
    scale: f64,
    palette: [u32; 6],
) {
    let px = |value: i32| (value as f64 * scale).round() as i32;
    if ui.window_controls {
        let rects = if custom_caption(ui) {
            caption_button_rects(width, height, ui.controls_right, ui.traffic_lights)
        } else {
            native_caption_button_rects(hwnd, width, height)
        };
        let mut cursor: POINT = zeroed();
        GetCursorPos(&mut cursor);
        ScreenToClient(hwnd, &mut cursor);
        for (i, rect) in rects.into_iter().enumerate() {
            let hover = PtInRect(&rect, cursor) != 0;
            if ui.traffic_lights {
                let diameter = (height * 14 / 31).max(10);
                let cx = (rect.left + rect.right) / 2;
                let cy = height / 2;
                let circle = RECT {
                    left: cx - diameter / 2,
                    top: cy - diameter / 2,
                    right: cx + (diameter + 1) / 2,
                    bottom: cy + (diameter + 1) / 2,
                };
                let color = match i {
                    2 => rgb(255, 95, 87),
                    0 => rgb(255, 189, 46),
                    _ => rgb(40, 200, 64),
                };
                crate::drawing::fill(
                    memory,
                    circle,
                    crate::drawing::argb(color, 255),
                    diameter as f32 / 2.0,
                );
            } else if hover {
                crate::drawing::fill(
                    memory,
                    rect,
                    crate::drawing::argb(if i == 2 { rgb(190, 35, 55) } else { palette[3] }, 190),
                    4.0,
                );
            }
            if !ui.traffic_lights || hover {
                // Caption symbols use native-sized geometry, independent of UI icon fonts.
                let size = px(if ui.traffic_lights { 6 } else { 10 }).max(4);
                let left = ((rect.left + rect.right - size) / 2) as f32;
                let top = ((rect.top + rect.bottom - size) / 2) as f32;
                let right = left + size as f32;
                let bottom = top + size as f32;
                let color = crate::drawing::argb(
                    if ui.traffic_lights {
                        rgb(45, 45, 45)
                    } else {
                        palette[4]
                    },
                    255,
                );
                let stroke = (scale as f32).max(1.0);
                let line = |x1, y1, x2, y2| {
                    crate::drawing::line(memory, x1, y1, x2, y2, color, stroke);
                };
                match i {
                    0 => line(left, (top + bottom) / 2.0, right, (top + bottom) / 2.0),
                    1 => {
                        let inset = if IsZoomed(hwnd) != 0 { px(2) } else { 0 };
                        if inset > 0 {
                            line(left + inset as f32, top, right, top);
                            line(right, top, right, bottom - inset as f32);
                        }
                        let r = RECT {
                            left: left as i32,
                            top: top as i32 + inset,
                            right: right as i32 - inset,
                            bottom: bottom as i32,
                        };
                        crate::drawing::stroke(memory, r, color, 0.0, stroke);
                    }
                    _ => {
                        line(left, top, right, bottom);
                        line(left, bottom, right, top);
                    }
                }
            }
        }
    }
}
unsafe fn paint_caption(
    hwnd: HWND,
    app: &App,
    dc: HDC,
    memory: HDC,
    bits: *mut u32,
    stride: i32,
    width: i32,
    height: i32,
    full_height: i32,
    backdrop: &BackgroundFrame<'_>,
) {
    if bits.is_null() || width <= 0 || height <= 0 {
        return;
    }
    let palette = app.palette();
    let solid = if !app.background_source.is_empty() {
        None
    } else if app.acrylic {
        Some(premultiplied_tint(
            if app.ui.light {
                app.ui.light_tint
            } else {
                palette[1]
            },
            app.ui.light_opacity,
        ))
    } else {
        Some(
            dib(if app.ui.background_mode == 3 {
                app.ui.solid_background()
            } else {
                palette[0]
            }) | 0xff000000,
        )
    };
    if solid.is_none() && stride == width {
        backdrop.fill_buffer(
            std::slice::from_raw_parts_mut(bits.cast::<u32>(), (stride * height) as usize),
            width,
            height,
            0,
            full_height,
        );
    } else {
        for y in 0..height {
            let row =
                std::slice::from_raw_parts_mut(bits.offset((y * stride) as isize), width as usize);
            if let Some(color) = solid {
                row.fill(color);
            } else {
                for (x, pixel) in row.iter_mut().enumerate() {
                    *pixel = backdrop.pixel(x as i32, y, width, full_height);
                }
            }
        }
    }
    let frame =
        crate::drawing::Frame::begin(memory, bits.cast(), stride, width, height, app.ui.light);
    let icon_size = app.px(16);
    let left_controls = custom_caption(&app.ui) && app.ui.window_controls && !app.ui.controls_right;
    let collapse_rect = caption_collapse_rect(
        width,
        height,
        left_controls,
        app.ui.traffic_lights,
        app.scale,
    );
    let mut cursor: POINT = zeroed();
    GetCursorPos(&mut cursor);
    ScreenToClient(hwnd, &mut cursor);
    let collapse_hover = PtInRect(&collapse_rect, cursor) != 0;
    if collapse_hover {
        crate::drawing::fill(
            memory,
            collapse_rect,
            crate::drawing::argb(palette[3], 190),
            4.0,
        );
    }
    draw_text(
        memory,
        app.fonts[4],
        "\u{e700}",
        collapse_rect,
        palette[4],
        DT_CENTER | DT_VCENTER | DT_SINGLELINE,
    );
    let icon_x = collapse_rect.right + app.px(6);
    draw_text(
        memory,
        app.fonts[0],
        &read(hwnd),
        RECT {
            left: icon_x + icon_size + app.px(6),
            top: 0,
            right: if app.ui.window_controls && app.ui.controls_right {
                (width - app.px(if app.ui.traffic_lights { 95 } else { 150 })).max(0)
            } else {
                width - app.px(12)
            },
            bottom: height,
        },
        palette[4],
        DT_LEFT | DT_VCENTER | DT_SINGLELINE,
    );
    draw_caption_controls(hwnd, memory, &app.ui, width, height, app.scale, palette);
    drop(frame);
    let mut icon = SendMessageW(hwnd, WM_GETICON, ICON_SMALL as usize, 0) as HICON;
    if icon.is_null() {
        icon = GetClassLongPtrW(hwnd, GCLP_HICONSM) as HICON;
    }
    if !icon.is_null() {
        DrawIconEx(
            memory,
            icon_x,
            (height - icon_size) / 2,
            icon,
            icon_size,
            icon_size,
            0,
            null_mut(),
            DI_NORMAL,
        );
    }
    BitBlt(dc, 0, 0, width, height, memory, 0, 0, SRCCOPY);
}
fn copy_backdrop(
    source: &[u32],
    destination: &mut [u32],
    width: usize,
    height: usize,
    stride: usize,
) {
    if stride == width {
        destination[..width * height].copy_from_slice(&source[..width * height]);
    } else {
        for y in 0..height {
            destination[y * stride..y * stride + width]
                .copy_from_slice(&source[y * width..(y + 1) * width]);
        }
    }
}
unsafe fn render(hwnd: HWND, app: &App, dc: HDC, paint_rect: Option<RECT>) {
    render_region(hwnd, app, dc, paint_rect, null_mut());
}

unsafe fn render_region(
    hwnd: HWND,
    app: &App,
    dc: HDC,
    paint_rect: Option<RECT>,
    update_region: HRGN,
) {
    #[cfg(feature = "diagnostics")]
    let _timing = crate::diagnostics::span("ui.render", app.ui.view.title());
    #[cfg(test)]
    let profile_start = std::time::Instant::now();
    #[cfg(test)]
    let profile_enabled = std::env::var_os("DEEZNATS_PROFILE_RENDER").is_some();
    #[cfg(test)]
    let mut profile_marks = Vec::new();
    #[cfg(feature = "diagnostics")]
    let mut stage_start = std::time::Instant::now();
    macro_rules! render_mark {
        ($name:literal) => {
            #[cfg(feature = "diagnostics")]
            {
                crate::diagnostics::record(
                    concat!("render.", $name),
                    app.ui.view.title(),
                    stage_start.elapsed(),
                );
                stage_start = std::time::Instant::now();
            }
            #[cfg(test)]
            if profile_enabled {
                profile_marks.push(($name, profile_start.elapsed().as_secs_f64() * 1000.0));
            }
        };
    }

    let _palette = PaintPalette::set(app.interpolated_palette_and_accent());
    let mut bounds = zeroed();
    GetClientRect(hwnd, &mut bounds);
    let caption = caption_height(hwnd);
    if bounds.right <= 0 || bounds.bottom <= caption * 2 {
        return;
    }
    let (_, logical_height) = size(hwnd, app);
    let scroll_rect = scroll_repaint_rect(app, bounds.right, logical_height, caption);
    let partial_scroll = scroll_repaint_enabled()
        && !matches!(app.ui.view, View::Settings | View::Lyrics)
        && scroll_repaint_avoids_search(app, &scroll_rect, caption)
        && paint_rect.is_some_and(|rect| {
            rect.left == scroll_rect.left
                && rect.top == scroll_rect.top
                && rect.right == scroll_rect.right
                && rect.bottom == scroll_rect.bottom
        });
    let partial_paint = !partial_scroll
        && paint_rect.is_some_and(|paint| {
            paint.top >= caption
                && paint.bottom <= bounds.bottom + caption
                && scroll_repaint_avoids_search(app, &paint, caption)
        });
    bounds.bottom -= caption;
    let params = BP_PAINTPARAMS {
        cbSize: size_of::<BP_PAINTPARAMS>() as u32,
        // The backdrop initialization below writes the dirty area. Erasing
        // the entire allocation first defeats partial scroll repainting.
        dwFlags: 0,
        ..zeroed()
    };
    let mut memory = null_mut();
    let backdrop = BackgroundFrame {
        current: app.background.as_deref(),
        previous: app.previous_background.as_deref(),
        progress: (app
            .background_transition
            .progress(std::time::Instant::now())
            * 255.0)
            .round() as u32,
        tint: dib(if app.ui.light {
            app.ui.light_tint
        } else {
            app.palette()[1]
        }),
        opacity: app.ui.light_opacity * 255 / 100,
        brightness: app.ui.brightness,
    };
    let buffer = BeginBufferedPaint(dc, &bounds, BPBF_TOPDOWNDIB, &params, &mut memory);
    if buffer != 0 {
        // The destination excludes native edit HWNDs to preserve caret/IME.
        // Paint their backdrop in the offscreen frame anyway: inheriting those
        // holes caches unpainted pixels beneath the search field on solid themes.
        // The destination DC still clips the final BitBlt around the editors.
        SelectClipRgn(memory, null_mut());
        let mut initial = null_mut();
        let mut initial_stride = 0;
        GetBufferedPaintBits(buffer, &mut initial, &mut initial_stride);
        let gpu_dirty = if partial_scroll { Some(RECT {
            top: scroll_rect.top - caption, bottom: scroll_rect.bottom - caption, ..scroll_rect
        }) } else if partial_paint { paint_rect.map(|r| RECT {
            top: r.top - caption, bottom: r.bottom - caption, ..r
        }) } else { None };
        let had_gpu=gpu::available();
        let accelerated = !initial.is_null()
            && (!cfg!(test) || std::env::var_os("DEEZNATS_TEST_GPU_RENDER").is_some())
            && std::env::var_os("DEEZNATS_SOFTWARE_RENDERING").is_none()
            && gpu::begin_with_region(hwnd, null(), initial_stride, bounds.right, bounds.bottom, caption, gpu_dirty, update_region);
        if had_gpu && !accelerated && gpu_dirty.is_some() {
            // Losing a retained GPU surface also loses every untouched pixel.
            // Repaint the entire frame before switching presentation paths.
            gpu::hide(); EndBufferedPaint(buffer,0);
            render_region(hwnd,app,dc,None,null_mut()); return;
        }
        let palette = app.palette();
        if accelerated {
            if !app.background_source.is_empty() {
                let (image,alpha)=backdrop.active_image();
                gpu::background(image,alpha,backdrop.tint,backdrop.opacity,backdrop.brightness,caption,gpu_dirty,None);
            } else {
                let solid=if app.acrylic {0} else {crate::drawing::argb(
                    if app.ui.background_mode==3 {app.ui.solid_background()} else {palette[0]},255)};
                gpu::background(None,0,0,0,100,caption,gpu_dirty,Some(solid));
            }
        }
        if !accelerated && !initial.is_null()
            && app.acrylic
            && app.background_source.is_empty()
        {
            if partial_paint || partial_scroll {
                let r = if partial_scroll {
                    RECT {
                        top: scroll_rect.top - caption,
                        bottom: scroll_rect.bottom - caption,
                        ..scroll_rect
                    }
                } else {
                    let p = paint_rect.unwrap();
                    RECT {
                        top: p.top - caption,
                        bottom: p.bottom - caption,
                        ..p
                    }
                };
                let y_start = (r.top.max(0) as usize).min(bounds.bottom as usize);
                let y_end = (r.bottom.max(0) as usize).min(bounds.bottom as usize);
                let x_start = r.left.clamp(0, bounds.right) as usize;
                let x_end = r.right.clamp(x_start as i32, bounds.right) as usize;
                for y in y_start..y_end {
                    std::ptr::write_bytes(
                        initial
                            .offset((y * initial_stride as usize + x_start) as isize)
                            .cast::<u8>(),
                        0,
                        (x_end - x_start) * 4,
                    );
                }
            } else {
                std::ptr::write_bytes(
                    initial.cast::<u8>(),
                    0,
                    (initial_stride * bounds.bottom).max(0) as usize * 4,
                );
            }
        }
        if !accelerated && !app.background_source.is_empty() {
            let key = BackdropKey {
                current: backdrop.current.map(|p| p.id),
                previous: backdrop.previous.map(|p| p.id),
                width: bounds.right,
                height: bounds.bottom,
                caption,
                progress: backdrop.progress,
                tint: backdrop.tint,
                opacity: backdrop.opacity,
                brightness: backdrop.brightness,
            };
            let mut cache = app.backdrop_cache.borrow_mut();
            if cache.0 != Some(key) {
                cache.0 = Some(key);
                cache
                    .1
                    .resize((bounds.right * bounds.bottom).max(0) as usize, 0);
                // Resizing down must also release the former full-window allocation.
                cache.1.shrink_to_fit();
                backdrop.fill_buffer(
                    &mut cache.1,
                    bounds.right,
                    bounds.bottom,
                    caption,
                    bounds.bottom + caption,
                );
            }
            if !initial.is_null() {
                let destination = std::slice::from_raw_parts_mut(
                    initial.cast::<u32>(),
                    (initial_stride * bounds.bottom).max(0) as usize,
                );
                if partial_paint || partial_scroll {
                    let r = if partial_scroll {
                        RECT {
                            top: scroll_rect.top - caption,
                            bottom: scroll_rect.bottom - caption,
                            ..scroll_rect
                        }
                    } else {
                        let p = paint_rect.unwrap();
                        RECT {
                            top: p.top - caption,
                            bottom: p.bottom - caption,
                            ..p
                        }
                    };
                    let y_start = (r.top.max(0) as usize).min(bounds.bottom as usize);
                    let y_end = (r.bottom.max(0) as usize).min(bounds.bottom as usize);
                    let x_start = r.left.clamp(0, bounds.right) as usize;
                    let x_end = r.right.clamp(x_start as i32, bounds.right) as usize;
                    for y in y_start..y_end {
                        let source = y * bounds.right as usize;
                        let target = y * initial_stride as usize;
                        destination[target + x_start..target + x_end]
                            .copy_from_slice(&cache.1[source + x_start..source + x_end]);
                    }
                } else {
                    copy_backdrop(
                        &cache.1,
                        destination,
                        bounds.right as usize,
                        bounds.bottom as usize,
                        initial_stride as usize,
                    );
                }
            }
        }
        if !accelerated && !app.acrylic && app.background_source.is_empty() && !initial.is_null() {
            let color = crate::drawing::argb(
                if app.ui.background_mode == 3 {
                    app.ui.solid_background()
                } else {
                    palette[0]
                },
                255,
            );
            if partial_paint || partial_scroll {
                let r = if partial_scroll {
                    RECT {
                        top: scroll_rect.top - caption,
                        bottom: scroll_rect.bottom - caption,
                        ..scroll_rect
                    }
                } else {
                    let p = paint_rect.unwrap();
                    RECT {
                        top: p.top - caption,
                        bottom: p.bottom - caption,
                        ..p
                    }
                };
                let y_start = (r.top.max(0) as usize).min(bounds.bottom as usize);
                let y_end = (r.bottom.max(0) as usize).min(bounds.bottom as usize);
                let x_start = r.left.clamp(0, bounds.right) as usize;
                let x_end = r.right.clamp(x_start as i32, bounds.right) as usize;
                let dest = std::slice::from_raw_parts_mut(
                    initial.cast::<u32>(),
                    (initial_stride * bounds.bottom).max(0) as usize,
                );
                for y in y_start..y_end {
                    dest[y * initial_stride as usize + x_start
                        ..y * initial_stride as usize + x_end]
                        .fill(color);
                }
            } else {
                std::slice::from_raw_parts_mut(
                    initial.cast::<u32>(),
                    (initial_stride * bounds.bottom).max(0) as usize,
                )
                .fill(color);
            }
        }
        render_mark!("setup");
        let moving=main_scroll_animating(app)||app.scrolling||app.queue_glide.is_some()||app.queue_scroll_drag||app.middle_scroll.is_some();
        let (layer_w,layer_h)=size(hwnd,app);
        let safe=accelerated&&moving&&app.scene.auth_modal.is_none()&&app.scene.login_underlay.is_none()&&app.scene.volume_popup.is_none()
            &&!matches!(app.ui.view,View::Lyrics|View::Settings|View::FlowTune|View::Account|View::TrackInfo|View::PlaylistEdit);
        let queue_w=if app.ui.right_sidebar_open(){280.min(layer_w/3)}else{0};
        let main_request=(safe&&app.ui.page.is_some()&&!app.scene.content_buttons.is_empty()).then(||gpu::LayerRequest{
            viewport:app.rect(Rect::new(if app.ui.sidebar_collapsed{70}else{246},app.scene.content_top,
                (layer_w-queue_w-30-if app.ui.sidebar_collapsed{70}else{246}).max(1),(content_bottom(app,layer_h)-app.scene.content_top).max(1))),
            layout_offset:app.px(app.ui.scroll),shown_offset:(if main_scroll_animating(app){app.scroll_position}else{app.ui.scroll as f64})*app.scale,overscan:app.px(ui::SCROLL_OVERSCAN),
        });
        let queue_request=if safe&&app.scene.device_sidebar.is_none(){app.scene.queue_viewport.map(|viewport|gpu::LayerRequest{
            viewport:app.rect(viewport),layout_offset:app.px((app.ui.queue_scroll*24+app.ui.queue_scroll_offset) as i32),
            shown_offset:app.queue_glide.map_or((app.ui.queue_scroll*24+app.ui.queue_scroll_offset) as f64,|state|state.0)*app.scale,overscan:app.px(144),
        })}else{None};
        let key=app.interpolated_palette_and_accent().into_iter().fold(0u64,|key,c|key.rotate_left(7)^c as u64);
        let recording=accelerated&&gpu::configure_layers(main_request,queue_request,key);
        let retained_canvas=if recording{gpu::RetainedCanvas::new(bounds.right,bounds.bottom,app.px(ui::SCROLL_OVERSCAN))}else{None};
        if recording&&retained_canvas.is_none(){gpu::configure_layers(None,None,key);}
        let native_memory=memory;
        let memory=retained_canvas.as_ref().map_or(memory,|canvas|canvas.dc());
        let frame = crate::drawing::Frame::begin(
            memory,
            initial.cast(),
            initial_stride,
            bounds.right,
            bounds.bottom,
            app.ui.light,
        );
        if accelerated && frame.is_none() {
            gpu::abandon(); EndBufferedPaint(buffer,0);
            render_region(hwnd,app,dc,None,null_mut()); return;
        }
        if retained_canvas.is_none()&&partial_scroll {
            IntersectClipRect(
                memory,
                scroll_rect.left,
                scroll_rect.top - caption,
                scroll_rect.right,
                scroll_rect.bottom - caption,
            );
        } else if retained_canvas.is_none()&&partial_paint {
            let paint = paint_rect.unwrap();
            IntersectClipRect(
                memory,
                paint.left,
                paint.top - caption,
                paint.right,
                paint.bottom - caption,
            );
        }
        // Preserve the holes between independent marquee invalidations.
        if retained_canvas.is_none()&&!update_region.is_null() && (partial_paint || partial_scroll) {
            OffsetRgn(update_region, 0, -caption);
            ExtSelectClipRgn(memory, update_region, RGN_AND);
            OffsetRgn(update_region, 0, caption);
        }
        if app.acrylic && app.background_source.is_empty() {
            // Native Acrylic exposes no backdrop-opacity property. A true
            // alpha tint above it lets the same control work without replacing
            // the operating system's blur or repainting the desktop ourselves.
            crate::drawing::fill(
                memory,
                bounds,
                crate::drawing::argb(
                    if app.ui.light {
                        app.ui.light_tint
                    } else {
                        palette[1]
                    },
                    app.ui.light_opacity * 255 / 100,
                ),
                0.0,
            );
        }
        let (w, h) = size(hwnd, app);
        let mut cursor: POINT = zeroed();
        GetCursorPos(&mut cursor);
        screen_to_content(hwnd, &mut cursor);
        let argb = crate::drawing::argb;
        let accent = app.accent();

        if app.ui.view == View::Lyrics && app.ui.lyrics_visualizer {
            let visual_rect = if app.ui.visualizer_style == 2 {
                let mut rect =
                    app.rect(app.scene.lyrics_area.unwrap_or(Rect::new(0, 0, w, h - 64)));
                if let Some(cover) = app.scene.lyrics_cover {
                    let cover = app.rect(cover);
                    rect.left = rect.left.max((cover.right + app.px(16)).min(rect.right));
                }
                rect
            } else {
                RECT {
                    left: 0,
                    top: 0,
                    right: bounds.right,
                    bottom: bounds.bottom,
                }
            };
            let saved = SaveDC(memory);
            IntersectClipRect(
                memory,
                visual_rect.left,
                visual_rect.top,
                visual_rect.right,
                visual_rect.bottom,
            );
            crate::drawing::recessed_visualizer(memory, bounds.right, bounds.bottom, || {
                match app.ui.visualizer_style {
                    0 => {
                        // Wave: organic landscape curves across the bottom
                        let baseline_y = visual_rect.bottom as f32;
                        let fg_wave = app.visualizer.compute_wave(
                            bounds.right as f32,
                            bounds.bottom as f32,
                            baseline_y,
                            app.playing(),
                            app.snapshot.position,
                            &app.ui.eq,
                        );
                        let is_light = app.is_effectively_light();
                        let rear_wave: Vec<_> = fg_wave
                            .iter()
                            .zip(fg_wave.iter().rev())
                            .map(|((x, _), (_, y))| (*x, *y))
                            .collect();
                        crate::drawing::wave_landscape(
                            memory,
                            &rear_wave,
                            baseline_y,
                            argb(accent, if is_light { 70 } else { 45 }),
                            argb(accent, 0),
                            argb(accent, if is_light { 145 } else { 100 }),
                            app.px(2) as f32,
                            argb(accent, if is_light { 80 } else { 55 }),
                        );
                        crate::drawing::wave_landscape(
                            memory,
                            &fg_wave,
                            baseline_y,
                            argb(accent, if is_light { 70 } else { 45 }),
                            argb(accent, 0),
                            argb(accent, if is_light { 145 } else { 100 }),
                            app.px(2) as f32,
                            argb(accent, if is_light { 80 } else { 55 }),
                        );
                    }
                    1 => {
                        // Bars: modern spectrum equalizer bars spanning across bottom under lyrics
                        let bar_w_unit = app.px(15) as f32;
                        let total_w = bounds.right as f32;
                        let bar_count = (total_w / bar_w_unit).clamp(24.0, 56.0) as usize;
                        let max_h = (bounds.bottom as f32 * 0.28).clamp(70.0, 220.0);
                        let bars = app.visualizer.compute_bars(bar_count, max_h);
                        let bar_x = 0.0;
                        let bar_y = bounds.bottom as f32 - max_h;
                        crate::drawing::spectrum_bars(
                            memory,
                            &bars,
                            bar_x,
                            bar_y,
                            total_w,
                            max_h,
                            accent,
                            argb(accent, if app.is_effectively_light() { 110 } else { 70 }),
                        );
                    }
                    2 => {
                        // Trap Nation stays in the right-hand pane when lyrics are hidden.
                        let cx = (visual_rect.left + visual_rect.right) as f32 / 2.0;
                        let cy = visual_rect.top as f32
                            + (visual_rect.bottom - visual_rect.top) as f32 * 0.5;
                        let base_r = ((visual_rect.right - visual_rect.left) as f32 * 0.18)
                            .clamp(45.0, 140.0);
                        let max_spike = (base_r * 0.65).clamp(35.0, 95.0);
                        let (spikes, bass_pump) =
                            app.visualizer.compute_trap_nation(199, max_spike);
                        crate::drawing::trap_nation_circle(
                            memory,
                            cx,
                            cy,
                            base_r,
                            bass_pump,
                            &spikes,
                            accent,
                            argb(accent, if app.is_effectively_light() { 90 } else { 55 }),
                        );
                    }
                    _ => {}
                }
            });
            RestoreDC(memory, saved);
        }

        for (index, panel) in app.scene.panels.iter().enumerate() {
            if !gpu::select_layer(if app.scene.content_panels.contains(&index){gpu::Layer::Main}else{gpu::Layer::Direct}){continue;}
            let _surface = crate::drawing::SurfaceOpacity::set(app.ui.surface_opacity);
            if panel.kind == 5 {
                continue;
            }
            let content = app.scene.content_panels.contains(&index);
            let saved = SaveDC(memory);
            if content {
                let left_c = if app.ui.view == View::Lyrics {
                    app.px(20)
                } else if app.ui.sidebar_collapsed {
                    app.px(70)
                } else {
                    app.px(246)
                };
                let queue_w = if app.ui.right_sidebar_open() {
                    280.min(w / 3)
                } else {
                    0
                };
                let right_c = if queue_w > 0 {
                    app.px(w - queue_w - 16)
                } else {
                    app.px(w - 30)
                };
                IntersectClipRect(
                    memory,
                    left_c,
                    app.px(app.scene.content_top-if gpu::is_recording(gpu::Layer::Main){ui::SCROLL_OVERSCAN}else{0}),
                    right_c,
                    app.px(if app.ui.view == View::Lyrics {
                        h - 64
                    } else {
                        content_bottom(app, h)+if gpu::is_recording(gpu::Layer::Main){ui::SCROLL_OVERSCAN}else{0}
                    }),
                );
            }
            let r = app.rect(panel.rect);
            if RectVisible(memory, &r) == 0 {
                RestoreDC(memory, saved);
                continue;
            }
            match panel.kind {
                21 => {
                    crate::drawing::fill(memory, r, argb(palette[1], 255), 0.0);
                }
                0 => {
                    let alpha = 255;
                    crate::drawing::fill(memory, r, argb(palette[1], alpha), app.corner(18) as f32);
                    crate::drawing::stroke(
                        memory,
                        r,
                        argb(palette[3], 90),
                        app.corner(18) as f32,
                        1.0,
                    );
                }
                18 => {
                    crate::drawing::fill(memory, r, argb(palette[3], 180), app.corner(14) as f32);
                    crate::drawing::stroke(
                        memory,
                        r,
                        argb(accent, 255),
                        app.corner(14) as f32,
                        1.5,
                    );
                }
                6 | 17 => {
                    let hover_scroll = PtInRect(&r, cursor) != 0 || app.scrolling;
                    let thumb_color = if hover_scroll { accent } else { palette[5] };
                    let thumb_alpha = if hover_scroll { 215 } else { 170 };
                    let offset=shown_scrollbar_y(app,panel,h).map_or(0.,|y| (y*app.scale-r.top as f64) as f32);
                    crate::drawing::fill_offset_y(
                        memory,
                        r,
                        argb(thumb_color, thumb_alpha),
                        (r.right - r.left) as f32 / 2.0,
                        offset,
                    );
                }
                7 => {
                    crate::drawing::fill(
                        memory,
                        r,
                        argb(palette[1], if app.ui.light { 225 } else { 130 }),
                        app.corner(16) as f32,
                    );
                    crate::drawing::stroke(
                        memory,
                        r,
                        argb(palette[3], if app.ui.light { 195 } else { 160 }),
                        app.corner(16) as f32,
                        1.0,
                    );
                    let pad_y = app.px(24) as f32;
                    let h_act = ((r.bottom - r.top) as f32 - pad_y * 2.0).max(1.0);
                    let cy = r.top as f32 + (r.bottom - r.top) as f32 / 2.0;
                    let half = h_act / 2.0;

                    for step in [-12, -6, 0, 6, 12] {
                        let gy = cy - (step as f32 / 12.0) * half;
                        crate::drawing::line(
                            memory,
                            (r.left + app.px(42)) as f32,
                            gy,
                            (r.right - app.px(20)) as f32,
                            gy,
                            argb(palette[3], if step == 0 { 180 } else { 75 }),
                            if step == 0 { 1.2 } else { 1.0 },
                        );
                        let label_r = RECT {
                            left: r.left + app.px(10),
                            top: (gy - app.px(9) as f32) as i32,
                            right: r.left + app.px(38),
                            bottom: (gy + app.px(9) as f32) as i32,
                        };
                        let txt = if step == 0 {
                            "0 dB".to_string()
                        } else if step > 0 {
                            format!("+{}", step)
                        } else {
                            format!("{}", step)
                        };
                        draw_text(
                            memory,
                            app.fonts[1],
                            &txt,
                            label_r,
                            palette[5],
                            DT_RIGHT | DT_VCENTER,
                        );
                    }

                    let pad_x = app.px(48) as f32;
                    let w_avail = ((r.right - r.left) as f32 - pad_x * 2.0).max(1.0);

                    for i in 0..10 {
                        let px = (r.left as f32 + pad_x) + i as f32 * (w_avail / 9.0);
                        crate::drawing::line(
                            memory,
                            px,
                            r.top as f32 + app.corner(16) as f32,
                            px,
                            r.bottom as f32 - app.corner(16) as f32,
                            argb(palette[3], 45),
                            1.0,
                        );
                    }

                    let mut pts = Vec::with_capacity(10);
                    for i in 0..10 {
                        let px = (r.left as f32 + pad_x) + i as f32 * (w_avail / 9.0);
                        let py = cy - (app.ui.eq[i].clamp(-12, 12) as f32 / 12.0) * half;
                        pts.push((px, py));
                    }

                    if pts.len() == 10 {
                        let mut fill_pts = Vec::with_capacity(12);
                        fill_pts.push((pts[0].0, cy));
                        for &pt in &pts {
                            fill_pts.push(pt);
                        }
                        fill_pts.push((pts[9].0, cy));
                        crate::drawing::polygon(memory, &fill_pts, argb(accent, 45));
                    }

                    crate::drawing::curve(memory, &pts, argb(accent, 240), 2.6);

                    let r_dot = app.px(5) as f32;
                    let inner_r = app.px(3) as f32;
                    for &(px, py) in &pts {
                        let dot = RECT {
                            left: (px - r_dot) as i32,
                            top: (py - r_dot) as i32,
                            right: (px + r_dot) as i32,
                            bottom: (py + r_dot) as i32,
                        };
                        crate::drawing::stroke(memory, dot, argb(palette[4], 230), r_dot, 1.8);
                        let inner_dot = RECT {
                            left: (px - inner_r) as i32,
                            top: (py - inner_r) as i32,
                            right: (px + inner_r) as i32,
                            bottom: (py + inner_r) as i32,
                        };
                        crate::drawing::fill(memory, inner_dot, argb(accent, 255), inner_r);
                    }
                }
                13 => {
                    crate::drawing::fill(
                        memory,
                        r,
                        argb(palette[1], if app.ui.light { 225 } else { 100 }),
                        app.corner(16) as f32,
                    );
                    crate::drawing::stroke(
                        memory,
                        r,
                        argb(palette[3], if app.ui.light { 195 } else { 130 }),
                        app.corner(16) as f32,
                        1.0,
                    );
                    let pad_x = app.px(20);
                    let col_w = ((r.right - r.left - pad_x * 2).max(1) / 11).max(1);
                    let track_top = r.top + app.px(44);
                    let track_h = app.px(80);
                    let track_bottom = track_top + track_h;
                    let track_cy = track_top + track_h / 2;

                    for i in 0..11 {
                        let col_x = r.left + pad_x + i as i32 * col_w;
                        let cx = col_x + col_w / 2;
                        let gain = app.ui.eq[i].clamp(-12, 12);

                        let gain_r = RECT {
                            left: col_x,
                            top: r.top + app.px(12),
                            right: col_x + col_w,
                            bottom: r.top + app.px(34),
                        };
                        let gain_text = format!("{:+} dB", gain);
                        let text_color = if gain != 0 { accent } else { palette[5] };
                        draw_text(
                            memory,
                            app.fonts[1],
                            &gain_text,
                            gain_r,
                            text_color,
                            DT_CENTER | DT_VCENTER,
                        );

                        let track_w = app.px(4);
                        let track_rect = RECT {
                            left: cx - track_w / 2,
                            top: track_top,
                            right: cx + track_w / 2,
                            bottom: track_bottom,
                        };
                        crate::drawing::fill(
                            memory,
                            track_rect,
                            argb(palette[3], 120),
                            (track_w / 2) as f32,
                        );

                        let tick_r = RECT {
                            left: cx - app.px(7),
                            top: track_cy - app.px(1),
                            right: cx + app.px(7),
                            bottom: track_cy + app.px(1),
                        };
                        crate::drawing::fill(memory, tick_r, argb(palette[3], 180), 0.0);

                        let thumb_y =
                            track_cy - (gain as f32 / 12.0 * (track_h as f32 / 2.0)) as i32;
                        if gain != 0 {
                            let bar_top = thumb_y.min(track_cy);
                            let bar_bottom = thumb_y.max(track_cy);
                            let bar_rect = RECT {
                                left: cx - track_w / 2,
                                top: bar_top,
                                right: cx + track_w / 2,
                                bottom: bar_bottom,
                            };
                            crate::drawing::fill(
                                memory,
                                bar_rect,
                                argb(accent, 220),
                                (track_w / 2) as f32,
                            );
                        }

                        let thumb_w = app.px(24);
                        let thumb_h = app.px(12);
                        let thumb_r = (thumb_h / 2) as f32;
                        let thumb_rect = RECT {
                            left: cx - thumb_w / 2,
                            top: thumb_y - thumb_h / 2,
                            right: cx + thumb_w / 2,
                            bottom: thumb_y + thumb_h / 2,
                        };

                        crate::drawing::fill(
                            memory,
                            RECT {
                                left: thumb_rect.left,
                                top: thumb_rect.top + 1,
                                right: thumb_rect.right,
                                bottom: thumb_rect.bottom + 1,
                            },
                            0x25000000,
                            thumb_r,
                        );

                        crate::drawing::fill(memory, thumb_rect, 0xffffffff, thumb_r);
                        crate::drawing::stroke(
                            memory,
                            thumb_rect,
                            argb(if gain != 0 { accent } else { palette[3] }, 255),
                            thumb_r,
                            1.2,
                        );

                        let notch_r = RECT {
                            left: cx - app.px(4),
                            top: thumb_y - app.px(1),
                            right: cx + app.px(4),
                            bottom: thumb_y + app.px(1),
                        };
                        crate::drawing::fill(
                            memory,
                            notch_r,
                            argb(if gain != 0 { accent } else { palette[3] }, 220),
                            0.0,
                        );

                        let freq_r = RECT {
                            left: col_x,
                            top: track_bottom + app.px(12),
                            right: col_x + col_w,
                            bottom: track_bottom + app.px(34),
                        };
                        let freq_str = [
                            "32", "64", "125", "250", "500", "1k", "2k", "4k", "8k", "16k", "Pre",
                        ][i];
                        draw_text(
                            memory,
                            app.fonts[1],
                            freq_str,
                            freq_r,
                            palette[4],
                            DT_CENTER | DT_VCENTER,
                        );
                    }
                }
                8 => {
                    let rad = ((r.bottom - r.top) as f32 / 2.0).min(app.corner(14) as f32);
                    crate::drawing::fill(
                        memory,
                        r,
                        argb(palette[1], if app.ui.light { 225 } else { 115 }),
                        rad,
                    );
                    crate::drawing::stroke(
                        memory,
                        r,
                        argb(palette[3], if app.ui.light { 200 } else { 185 }),
                        rad,
                        1.2,
                    );
                }
                9 => {
                    crate::drawing::fill(memory, r, argb(palette[3], 160), 1.0);
                }
                10 => {
                    crate::drawing::fill(memory, r, argb(accent, 95), app.corner(10) as f32);
                    crate::drawing::stroke(
                        memory,
                        r,
                        argb(palette[3], 170),
                        app.corner(10) as f32,
                        1.0,
                    );
                }
                11 => {}
                12 => {
                    crate::drawing::fill(
                        memory,
                        r,
                        argb(palette[1], if app.ui.light { 225 } else { 100 }),
                        app.corner(16) as f32,
                    );
                    crate::drawing::stroke(
                        memory,
                        r,
                        argb(palette[3], if app.ui.light { 195 } else { 120 }),
                        app.corner(16) as f32,
                        1.0,
                    );
                }
                14 => {
                    let rad = if panel.rect.h == 90
                        || (panel.rect.w == panel.rect.h && panel.rect.w > 64)
                    {
                        app.corner(12) as f32
                    } else if panel.rect.w == panel.rect.h && panel.rect.w <= 48 {
                        app.corner(6) as f32
                    } else {
                        app.corner(4) as f32
                    };
                    crate::drawing::fill(memory, r, argb(palette[3], 85), rad);
                }
                15 => {
                    let rad = (r.right - r.left) as f32 / 2.0;
                    crate::drawing::fill(memory, r, argb(palette[3], 85), rad);
                }
                _ => {
                    let color = match panel.kind {
                        1 => palette[2],
                        2 => rgb(61, 30, 70),
                        3 => palette[3],
                        _ => accent,
                    };
                    rounded(
                        memory,
                        r,
                        color,
                        app.corner(if panel.kind == 3 { 4 } else { 14 }),
                    );
                }
            }
            RestoreDC(memory, saved);
        }

        render_mark!("panels");
        for (index, node) in app.scene.buttons.iter().enumerate() {
            if !gpu::select_layer(if app.scene.content_buttons.contains(&index){gpu::Layer::Main}else{gpu::Layer::Direct}){continue;}
            if !moving&&matches!(node.action, Action::OpenItem(_)) {
                if is_track_row(node) {
                    let left_c = if app.ui.view == View::Lyrics {
                        app.px(20)
                    } else if app.ui.sidebar_collapsed {
                        app.px(70)
                    } else {
                        app.px(246)
                    };
                    let queue_w = if app.ui.right_sidebar_open() {
                        280.min(w / 3)
                    } else {
                        0
                    };
                    let right_c = if queue_w > 0 {
                        app.px(w - queue_w - 16)
                    } else {
                        app.px(w - 30)
                    };
                    let row_rect = app.rect(track_row_rect(
                        node,
                        w,
                        queue_w,
                        app.ui.sidebar_collapsed,
                        app.ui.view == View::Lyrics,
                    ));
                    if PtInRect(&row_rect, cursor) != 0 {
                        let saved = SaveDC(memory);
                        if app.scene.content_buttons.contains(&index) {
                            IntersectClipRect(
                                memory,
                                left_c,
                                app.px(app.scene.content_top),
                                right_c,
                                app.px(if app.ui.view == View::Lyrics {
                                    h - 64
                                } else {
                                    content_bottom(app, h)
                                }),
                            );
                        }
                        crate::drawing::fill(
                            memory,
                            row_rect,
                            argb(accent, if node.selected { 125 } else { 75 }),
                            app.px(10) as f32,
                        );
                        crate::drawing::stroke(
                            memory,
                            row_rect,
                            argb(palette[3], if node.selected { 180 } else { 100 }),
                            app.px(10) as f32,
                            1.0,
                        );
                        RestoreDC(memory, saved);
                    }
                }
            }
        }

        let is_artist_header = app.ui.view == View::Artist
            && app
                .ui
                .page
                .as_ref()
                .and_then(|p| p.entity.as_ref())
                .is_some_and(|e| e.kind == "artist");
        render_mark!("buttons");
        for (picture_index, (rect, url)) in app.scene.pictures.iter().enumerate() {
            let layer=if app.scene.content_pictures.contains(&picture_index){gpu::Layer::Main}else if app.scene.queue_pictures.contains(&picture_index){gpu::Layer::Queue}else{gpu::Layer::Direct};
            if !gpu::select_layer(layer){continue;}
            let player = !app.scene.content_pictures.contains(&picture_index)
                && rect.y >= content_bottom(app, h);
            let is_lyrics_cover =
                app.ui.view == View::Lyrics && !player && app.scene.lyrics_cover == Some(*rect);
            let current = app
                .images
                .get(url)
                .filter(|_| !crate::catalog::is_default_cover(url))
                .or_else(|| {
                    if app.ui.view == View::Lyrics && app.scene.lyrics_cover == Some(*rect) {
                        app.images.get(&app.ui.now.artwork)
                    } else {
                        None
                    }
                });
            let previous = if (player || is_lyrics_cover)
                && !url.is_empty()
                && !crate::catalog::is_default_cover(url)
                && !app.image_retry_at.contains_key(url)
            {
                app.previous_cover.as_ref()
            } else {
                None
            };
            let circular = (picture_index == 0 && is_artist_header)
                || (is_artist_header && rect.y < app.scene.content_top);
            let rad = if circular {
                app.px(rect.w)
            } else if player {
                app.corner(5)
            } else {
                app.corner(16)
            };
            if let Some(image) = current.or(previous) {
                let saved = SaveDC(memory);
                if app.scene.queue_pictures.contains(&picture_index) {
                    if let Some(viewport)=app.scene.queue_viewport {let r=gpu::expanded_clip(app.rect(viewport),gpu::Layer::Queue);IntersectClipRect(memory,r.left,r.top,r.right,r.bottom);}
                }
                if app.scene.content_pictures.contains(&picture_index) {
                    let left_c = if app.ui.view == View::Lyrics {
                        app.px(20)
                    } else if app.ui.sidebar_collapsed {
                        app.px(70)
                    } else {
                        app.px(246)
                    };
                    let queue_w = if app.ui.right_sidebar_open() {
                        280.min(w / 3)
                    } else {
                        0
                    };
                    let right_c = if queue_w > 0 {
                        app.px(w - queue_w - 16)
                    } else {
                        app.px(w - 30)
                    };
                    IntersectClipRect(
                        memory,
                        left_c,
                        app.px(app.scene.content_top-if gpu::is_recording(gpu::Layer::Main){ui::SCROLL_OVERSCAN}else{0}),
                        right_c,
                        app.px(if app.ui.view == View::Lyrics {
                            h - 64
                        } else {
                            content_bottom(app, h)+if gpu::is_recording(gpu::Layer::Main){ui::SCROLL_OVERSCAN}else{0}
                        }),
                    );
                }
                // Avoid resizing and drawing artwork entirely outside the current clip.
                if RectVisible(memory, &app.rect(*rect)) == 0 {
                    RestoreDC(memory, saved);
                    continue;
                }
                let cover_alpha = app.cover_changed.map_or(0, |started| {
                    let t =
                        (started.elapsed().as_secs_f32() / (ART_FADE_MS as f32 / 1000.0)).min(1.0);
                    (t * t * (3.0 - 2.0 * t) * 255.0) as u8
                });
                if is_lyrics_cover {
                    let high_ready = app.images.get(url);
                    let low_image = app.images.get(&app.ui.now.artwork);
                    if let Some(previous) = previous {
                        bitmap(memory, app.rect(*rect), previous, rad);
                    }
                    let track_alpha = if previous.is_some() { cover_alpha } else { 255 };
                    if let Some(high) = high_ready {
                        if let Some(started) = app.image_times.get(url) {
                            let t = (started.elapsed().as_secs_f32()
                                / (ART_FADE_MS as f32 / 1000.0))
                                .min(1.0);
                            let alpha = (t * t * (3.0 - 2.0 * t) * track_alpha as f32) as u8;
                            if alpha < 255 {
                                if let Some(low) = low_image {
                                    faded_bitmap(memory, app.rect(*rect), low, rad, track_alpha);
                                }
                                faded_bitmap(memory, app.rect(*rect), high, rad, alpha);
                            } else {
                                bitmap(memory, app.rect(*rect), high, rad);
                            }
                        } else {
                            faded_bitmap(memory, app.rect(*rect), high, rad, track_alpha);
                        }
                    } else if let Some(low) = low_image {
                        faded_bitmap(memory, app.rect(*rect), low, rad, track_alpha);
                    } else if previous.is_none() {
                        bitmap(memory, app.rect(*rect), image, rad);
                    }
                    let hover_p = app.lyrics_hover_progress();
                    if hover_p > 0.005 {
                        let blur_key = format!("blur:cover:{}", url);
                        if let Some(blurred) = app.images.get(&blur_key) {
                            let alpha = (hover_p * track_alpha as f32).round().min(255.0) as u8;
                            faded_bitmap(memory, app.rect(*rect), blurred, rad, alpha);
                        }
                        let scrim_alpha = (hover_p * 56.0).round().min(255.0) as u32;
                        if scrim_alpha > 0 {
                            crate::drawing::fill(
                                memory,
                                app.rect(*rect),
                                crate::drawing::argb(0, scrim_alpha),
                                rad as f32 / 2.0,
                            );
                        }
                    }
                } else if let (true, Some(_), Some(previous)) = (player, current, previous) {
                    bitmap(memory, app.rect(*rect), previous, rad);
                    faded_bitmap(memory, app.rect(*rect), image, rad, cover_alpha);
                } else {
                    bitmap(memory, app.rect(*rect), image, rad);
                }
                RestoreDC(memory, saved);
            } else if player {
                crate::drawing::fill(memory, app.rect(*rect), argb(palette[1], 255), rad as f32);
                draw_text(
                    memory,
                    app.fonts[4],
                    "\u{e8d6}",
                    app.rect(*rect),
                    palette[5],
                    DT_CENTER | DT_VCENTER,
                );
            }
        }
        render_mark!("pictures");
        for (index, label) in app.scene.labels.iter().enumerate() {
            if !gpu::select_layer(if app.scene.content_labels.contains(&index){gpu::Layer::Main}else{gpu::Layer::Direct}){continue;}
            // Queue rows are painted by the button pass below. Paint their labels
            // afterwards so the row background does not cover the animation.
            if app.ui.queue_sidebar_open
                && app.scene.auth_modal.is_none()
                && app.scene.queue_labels.contains(&index)
            {
                continue;
            }
            // Modal labels are painted with their surface after all background
            // buttons. GDI+ logo paths cannot be hidden using a GDI clip box.
            if app
                .scene
                .auth_modal
                .is_some_and(|(_, first, _)| index >= first)
            {
                continue;
            }
            let content = app.scene.content_labels.contains(&index);
            if content {
                let top = if app.ui.view == View::Lyrics {
                    0
                } else {
                    app.scene.content_top
                };
                let bottom = if app.ui.view == View::Lyrics {
                    h
                } else {
                    content_bottom(app, h)
                };
                let margin=if gpu::is_recording(gpu::Layer::Main){ui::SCROLL_OVERSCAN}else{0};
                if label.rect.y + label.rect.h < top - 8-margin || label.rect.y > bottom + 8+margin {
                    continue;
                }
            }
            let saved = SaveDC(memory);
            if content {
                let left_c = if app.ui.view == View::Lyrics {
                    app.px(20)
                } else if app.ui.sidebar_collapsed {
                    app.px(70)
                } else {
                    app.px(246)
                };
                let queue_w = if app.ui.right_sidebar_open() {
                    280.min(w / 3)
                } else {
                    0
                };
                let right_c = if queue_w > 0 {
                    app.px(w - queue_w - 16)
                } else {
                    app.px(w - 20)
                };
                let top_c = if app.ui.view == View::Lyrics {
                    0
                } else {
                    app.px(app.scene.content_top-if gpu::is_recording(gpu::Layer::Main){ui::SCROLL_OVERSCAN}else{0})
                };
                let bottom_c = if app.ui.view == View::Lyrics {
                    app.px(h)
                } else {
                    app.px(content_bottom(app, h)+if gpu::is_recording(gpu::Layer::Main){ui::SCROLL_OVERSCAN}else{0})
                };
                IntersectClipRect(memory, left_c, top_c, right_c, bottom_c);
            }
            let label_bounds = app.rect(label.rect);
            let padding = app.px(if app.ui.view == View::Lyrics { 32 } else { 2 });
            let visible_bounds = RECT {
                left: label_bounds.left - padding,
                top: label_bounds.top - padding,
                right: label_bounds.right + padding,
                bottom: label_bounds.bottom + padding,
            };
            if RectVisible(memory, &visible_bounds) == 0 {
                RestoreDC(memory, saved);
                continue;
            }
            let font = if app.ui.view == View::Lyrics && !app.scene.queue_labels.contains(&index) {
                if let Some((_, _, f_act, f_inact, f_plain)) = app.lyrics_fonts {
                    match label.kind {
                        13 | 14 | 15 => f_act,
                        12 => f_inact,
                        0 => f_plain,
                        10 => app.fonts[3],
                        11 => app.fonts[0],
                        _ => app.fonts[0],
                    }
                } else {
                    match label.kind {
                        13 | 14 | 15 => app.fonts[5],
                        12 => app.fonts[3],
                        10 => app.fonts[3],
                        11 => app.fonts[0],
                        _ => app.fonts[0],
                    }
                }
            } else {
                match label.kind {
                    16 | 18 => app.fonts[10],
                    19 | 20 => app.fonts[11],
                    17 => app.fonts[9],
                    5..=9 => app.fonts[label.kind as usize],
                    10 => app.fonts[3],
                    11 => app.fonts[0],
                    12 => app.fonts[3],
                    13 | 14 | 15 => app.fonts[5],
                    2 => app.fonts[2],
                    3 => app.fonts[3],
                    4 => app.fonts[4],
                    _ => app.fonts[0],
                }
            };
            let align = if matches!(label.kind, 10 | 11 | 16 | 17 | 20) {
                DT_CENTER
            } else {
                DT_LEFT
            };
            if label.kind == 17 {
                crate::drawing::fill(
                    memory,
                    app.rect(label.rect),
                    argb(palette[5], 45),
                    app.px(2) as f32,
                );
            }
            let is_light = app.is_effectively_light();
            let color = match label.kind {
                1 | 11 | 19 | 20 => palette[5],
                12 => {
                    if is_light {
                        palette[5]
                    } else {
                        0x00808080
                    }
                }
                13 | 14 => {
                    if is_light {
                        palette[4]
                    } else {
                        0x00ffffff
                    }
                }
                15 => {
                    if is_light {
                        palette[4]
                    } else {
                        let position = app.playback_position();
                        let value = (210.0 + 35.0 * (position as f32 / 380.0).sin()) as u32;
                        rgb(value, value, value)
                    }
                }
                _ => palette[4],
            };
            let drawn_blur = if label.kind == 12 && app.ui.blur_inactive_lyrics {
                let blur_color = palette[5];
                crate::drawing::blurred_text(
                    memory,
                    font,
                    &label.text,
                    app.rect(label.rect),
                    blur_color,
                    align,
                    2.0 * app.scale as f32,
                )
            } else {
                false
            };
            // Blurred glyphs must not sit on top of a separate sharp shadow.
            if !drawn_blur && matches!(label.kind, 12 | 13 | 14 | 15) {
                crate::drawing::lyric_shadow(
                    memory,
                    font,
                    &label.text,
                    app.rect(label.rect),
                    align,
                );
            }
            let drawn_karaoke = if label.kind == 13 && app.ui.karaoke_lyrics {
                if app.scene.karaoke_preview == Some(index) {
                    let line = crate::lyrics::karaoke_example();
                    draw_karaoke_row(
                        memory,
                        font,
                        line,
                        0..line.text.len(),
                        (app.karaoke_preview_started.elapsed().as_millis() % 5400) as u64,
                        app.rect(label.rect),
                        palette,
                        accent,
                        app.is_effectively_light(),
                    )
                } else if let Some(row) = app.scene.lyric_rows.iter().find(|row| row.label == index)
                {
                    let position = app.playback_position();
                    app.ui.rich_lyrics.get(row.line).is_some_and(|line| {
                        draw_karaoke_row(
                            memory,
                            font,
                            line,
                            row.range.clone(),
                            position,
                            app.rect(label.rect),
                            palette,
                            accent,
                            app.is_effectively_light(),
                        )
                    })
                } else {
                    false
                }
            } else {
                false
            };
            if !drawn_blur && !drawn_karaoke {
                if !matches!(label.kind, 12 | 13 | 14 | 15) {
                    let draw_label = if app.ui.view == View::Lyrics && matches!(label.kind, 10 | 11)
                    {
                        draw_automatic_text
                    } else {
                        draw_hover_text
                    };
                    draw_label(
                        hwnd,
                        memory,
                        font,
                        &label.text,
                        app.rect(label.rect),
                        color,
                        if matches!(label.kind, 12 | 13 | 14 | 15) {
                            align | crate::drawing::LYRICS_UNTRIMMED
                        } else {
                            align
                        },
                        app.scale,
                    );
                } else {
                    draw_text(
                        memory,
                        font,
                        &label.text,
                        app.rect(label.rect),
                        color,
                        if matches!(label.kind, 12 | 13 | 14 | 15) {
                            align | crate::drawing::LYRICS_UNTRIMMED
                        } else {
                            align
                        },
                    );
                }
            }
            RestoreDC(memory, saved);
        }
        gpu::select_layer(gpu::Layer::Direct);
        let pos = app.ui.position / 1000;
        let hide_focus = SendMessageW(hwnd, WM_QUERYUISTATE, 0, 0) & UISF_HIDEFOCUS as isize != 0;
        for (index, node) in app.scene.buttons.iter().enumerate() {
            let layer=if app.scene.content_buttons.contains(&index){gpu::Layer::Main}else if matches!(node.action,Action::QueueJump(_)){gpu::Layer::Queue}else if matches!(node.action,Action::QueueReturn){gpu::Layer::Overlay}else{gpu::Layer::Direct};
            if !gpu::select_layer(layer){continue;}
            if let Some((rect, first_label, first_button)) = app.scene.login_underlay {
                if index == first_button {
                    crate::drawing::fill(
                        memory,
                        app.rect(rect),
                        crate::drawing::argb(palette[1], 255),
                        app.corner(24) as f32,
                    );
                    crate::drawing::stroke(
                        memory,
                        app.rect(rect),
                        crate::drawing::argb(palette[3], 220),
                        app.corner(24) as f32,
                        1.0,
                    );
                    let last = app
                        .scene
                        .auth_modal
                        .map_or(app.scene.labels.len(), |(_, first, _)| first);
                    for label in app.scene.labels.iter().take(last).skip(first_label) {
                        draw_text(
                            memory,
                            if label.kind == 3 {
                                app.fonts[3]
                            } else {
                                app.fonts[0]
                            },
                            &label.text,
                            app.rect(label.rect),
                            palette[4],
                            DT_LEFT | DT_VCENTER | DT_SINGLELINE,
                        );
                    }
                }
            }
            if let Some((rect, first_label, first_button)) = app.scene.auth_modal {
                if index == first_button {
                    let popup_alpha = if app.scene.device_popover_anchor.is_some()
                        || app.ui.deezer_login_open
                        || app.ui.onboarding_open
                    {
                        255
                    } else {
                        app.ui.surface_opacity * 255 / 100
                    };
                    crate::drawing::fill(
                        memory,
                        app.rect(rect),
                        crate::drawing::argb(palette[1], popup_alpha),
                        app.corner(24) as f32,
                    );
                    crate::drawing::stroke(
                        memory,
                        app.rect(rect),
                        crate::drawing::argb(palette[3], 220),
                        app.corner(24) as f32,
                        1.0,
                    );
                    if let Some(anchor) = app
                        .scene
                        .device_popover_anchor
                        .filter(|_| app.scene.device_sidebar.is_none())
                    {
                        let center =
                            (anchor.x + anchor.w / 2).clamp(rect.x + 16, rect.x + rect.w - 16);
                        let above = rect.y + rect.h <= anchor.y;
                        let base = if above {
                            rect.y + rect.h - 1
                        } else {
                            rect.y + 1
                        };
                        let tip = if above {
                            anchor.y - 2
                        } else {
                            anchor.y + anchor.h + 2
                        };
                        let points = [
                            (app.px(center - 9) as f32, app.px(base) as f32),
                            (app.px(center + 9) as f32, app.px(base) as f32),
                            (app.px(anchor.x + anchor.w / 2) as f32, app.px(tip) as f32),
                        ];
                        crate::drawing::polygon(
                            memory,
                            &points,
                            crate::drawing::argb(palette[1], popup_alpha),
                        );
                        for edge in [(0, 2), (1, 2)] {
                            crate::drawing::line(
                                memory,
                                points[edge.0].0,
                                points[edge.0].1,
                                points[edge.1].0,
                                points[edge.1].1,
                                crate::drawing::argb(palette[3], 220),
                                1.0,
                            );
                        }
                    }
                    for label in app.scene.labels.iter().skip(first_label) {
                        draw_text(
                            memory,
                            if label.kind == 3 {
                                app.fonts[3]
                            } else {
                                app.fonts[0]
                            },
                            &label.text,
                            app.rect(label.rect),
                            if label.kind == 1 {
                                palette[5]
                            } else {
                                palette[4]
                            },
                            DT_LEFT | DT_VCENTER | DT_SINGLELINE,
                        );
                    }
                }
            }
            let content = app.scene.content_buttons.contains(&index)||matches!(node.action,Action::QueueJump(_));
            let viewport = button_viewport(app, node, w, h);
            if content
                && (node.rect.y + node.rect.h <= viewport.y
                    || node.rect.y >= viewport.y + viewport.h)
            {
                continue;
            }
            let saved = SaveDC(memory);
            if let Some((rect, _, first)) = app.scene.auth_modal {
                if index < first {
                    let r = app.rect(rect);
                    ExcludeClipRect(memory, r.left, r.top, r.right, r.bottom);
                }
            }
            if content {
                IntersectClipRect(
                    memory,
                    app.px(viewport.x),
                    app.px(viewport.y),
                    app.px(viewport.x + viewport.w),
                    app.px(viewport.y + viewport.h),
                );
            }
            let test_rect = if let Some(group) = node.artist_credit_group {
                app.rect(group)
            } else {
                app.rect(node.rect)
            };
            if RectVisible(memory, &test_rect) == 0 {
                RestoreDC(memory, saved);
                continue;
            }
            let button = app.buttons.get(index).copied().unwrap_or(null_mut());
            let canvas = canvas_track_button(app, index);
            let state = if canvas || button.is_null() {
                0
            } else {
                SendMessageW(button, BM_GETSTATE, 0, 0) as u32
            };
            let draw = DRAWITEMSTRUCT {
                CtlID: 1000 + index as u32,
                hwndItem: if button.is_null() { hwnd } else { button },
                hDC: memory,
                rcItem: app.rect(node.rect),
                itemState: (if state & BST_PUSHED != 0
                    || (canvas
                        && app
                            .canvas_pressed
                            .as_ref()
                            .is_some_and(|(pressed, _)| *pressed == index))
                {
                    ODS_SELECTED
                } else {
                    0
                }) | (if (state & BST_FOCUS != 0
                    || (canvas
                        && app.canvas_focus == Some(index)
                        && (GetFocus() == hwnd || GetFocus() == app.canvas_keyboard)))
                    && !hide_focus
                {
                    ODS_FOCUS
                } else {
                    0
                }) | (if !scene_button_enabled(app, index)
                    || (!canvas && !button.is_null() && IsWindowEnabled(button) == 0)
                {
                    ODS_DISABLED
                } else {
                    0
                }) | ({
                    let is_hovered = if content&&moving {false} else if content {
                        let mut b = node.rect;
                        b.x = b.x.max(viewport.x);
                        b.y = b.y.max(viewport.y);
                        let r = (node.rect.x + node.rect.w).min(viewport.x + viewport.w);
                        let bot = (node.rect.y + node.rect.h).min(viewport.y + viewport.h);
                        b.w = (r - b.x).max(0);
                        b.h = (bot - b.y).max(0);
                        b.w > 0 && b.h > 0 && PtInRect(&app.rect(b), cursor) != 0
                    } else {
                        PtInRect(&app.rect(node.rect), cursor) != 0
                    };
                    if is_hovered {
                        ODS_HOTLIGHT
                    } else {
                        0
                    }
                }),
                ..zeroed()
            };
            draw_button(hwnd, app, &draw);
            RestoreDC(memory, saved);
        }
        if app.ui.queue_sidebar_open && app.scene.auth_modal.is_none() {
            for (offset,label) in app.scene.labels[app.scene.queue_labels.clone()].iter().enumerate() {
                let row=app.scene.queue_row_labels.contains(&(app.scene.queue_labels.start+offset));
                if !gpu::select_layer(if row{gpu::Layer::Queue}else{gpu::Layer::Direct}){continue;}
                let saved=SaveDC(memory);
                if app.scene.queue_row_labels.contains(&(app.scene.queue_labels.start+offset)) {
                    if let Some(viewport)=app.scene.queue_viewport {let r=gpu::expanded_clip(app.rect(viewport),gpu::Layer::Queue);IntersectClipRect(memory,r.left,r.top,r.right,r.bottom);}
                }
                let font = match label.kind {
                    5..=9 => app.fonts[label.kind as usize],
                    2 => app.fonts[2],
                    3 => app.fonts[3],
                    4 => app.fonts[4],
                    _ => app.fonts[0],
                };
                draw_automatic_text(
                    hwnd,
                    memory,
                    font,
                    &label.text,
                    app.rect(label.rect),
                    if label.kind == 1 {
                        palette[5]
                    } else {
                        palette[4]
                    },
                    DT_LEFT | DT_VCENTER,
                    app.scale,
                );
                RestoreDC(memory,saved);
            }
        }
        gpu::select_layer(gpu::Layer::Direct);
        if let Some(popup) = app.scene.volume_popup {
            crate::drawing::fill(
                memory,
                app.rect(popup),
                argb(palette[2], 245),
                app.px(12) as f32,
            );
            crate::drawing::stroke(
                memory,
                app.rect(popup),
                argb(palette[3], 230),
                app.px(12) as f32,
                1.0,
            );
        }
        let sliders = [(SEEK, app.scene.seek), (VOLUME, app.scene.volume)]
            .into_iter()
            .chain(
                app.scene
                    .sliders
                    .iter()
                    .enumerate()
                    .map(|(i, (rect, _, _))| (600 + i as i32, Some(*rect))),
            );
        for (id, rect) in sliders {
            if let Some(rect) = rect {
                if id >= 600
                    && (rect.y + rect.h <= app.scene.content_top
                        || rect.y >= content_bottom(app, h))
                {
                    continue;
                }
                let maximum = if id == SEEK {
                    SEEK_STEPS as i32
                } else if id >= 600
                    && matches!(
                        app.scene.sliders[(id - 600) as usize].1,
                        "Light-mode brightness" | "Background brightness"
                    )
                {
                    110
                } else {
                    100
                };
                let Some(&control) = app.edits.get(&id) else {
                    continue;
                };
                let value = SendMessageW(control, WM_USER, 0, 0).clamp(0, maximum as isize) as i32;
                let center = rect.y + rect.h / 2;
                if id == VOLUME && app.scene.volume_vertical {
                    let travel = (rect.h - 10).max(1);
                    let bottom = rect.y + rect.h - 5;
                    let knob_y = bottom - travel * value / maximum;
                    rounded(
                        memory,
                        app.rect(Rect::new(rect.x + rect.w / 2 - 2, rect.y + 5, 4, travel)),
                        palette[3],
                        app.px(4),
                    );
                    rounded(
                        memory,
                        app.rect(Rect::new(
                            rect.x + rect.w / 2 - 2,
                            knob_y,
                            4,
                            bottom - knob_y,
                        )),
                        accent,
                        app.px(4),
                    );
                    rounded(
                        memory,
                        app.rect(Rect::new(rect.x + rect.w / 2 - 5, knob_y - 5, 10, 10)),
                        palette[4],
                        app.px(10),
                    );
                    continue;
                }
                let saved = SaveDC(memory);
                if id >= 600 {
                    let left_c = if app.ui.view == View::Lyrics {
                        app.px(20)
                    } else if app.ui.sidebar_collapsed {
                        app.px(70)
                    } else {
                        app.px(246)
                    };
                    IntersectClipRect(
                        memory,
                        left_c,
                        app.px(app.scene.content_top),
                        app.px(w - 30),
                        app.px(if app.ui.view == View::Lyrics {
                            h - 64
                        } else {
                            content_bottom(app, h)
                        }),
                    );
                }
                rounded(
                    memory,
                    app.rect(Rect::new(rect.x + 5, center - 2, (rect.w - 10).max(1), 4)),
                    palette[3],
                    app.px(4),
                );
                rounded(
                    memory,
                    app.rect(Rect::new(
                        rect.x + 5,
                        center - 2,
                        (rect.w - 10).max(1) * value / maximum,
                        4,
                    )),
                    if id < 600 && app.ui.view != View::Lyrics {
                        palette[4]
                    } else {
                        accent
                    },
                    app.px(4),
                );
                let knob = app.rect(Rect::new(
                    rect.x + (rect.w - 10) * value / maximum,
                    center - 5,
                    10,
                    10,
                ));
                rounded(memory, knob, palette[4], app.px(10));
                crate::drawing::stroke(
                    memory,
                    knob,
                    argb(palette[3], 220),
                    (knob.right - knob.left) as f32 / 2.0,
                    1.0,
                );
                RestoreDC(memory, saved);
            }
        }

        let dur = app.ui.duration / 1000;
        draw_text(
            memory,
            if app.ui.view == View::Lyrics {
                app.fonts[7]
            } else {
                app.fonts[0]
            },
            &format!("{}:{:02}", pos / 60, pos % 60),
            app.rect(app.scene.elapsed.unwrap_or(Rect::new(0, 0, 0, 0))),
            palette[5],
            DT_LEFT | DT_VCENTER,
        );
        draw_text(
            memory,
            if app.ui.view == View::Lyrics {
                app.fonts[7]
            } else {
                app.fonts[0]
            },
            &format!("{}:{:02}", dur / 60, dur % 60),
            app.rect(app.scene.remaining.unwrap_or(Rect::new(0, 0, 0, 0))),
            palette[5],
            if app.ui.view == View::Lyrics {
                DT_RIGHT
            } else {
                DT_LEFT
            },
        );
        render_mark!("labels");
        gpu::select_layer(gpu::Layer::Direct);
        canvas_input::render(app, memory);
        let mut pixels = null_mut();
        let mut stride = 0;
        if frame.is_none() && GetBufferedPaintBits(buffer, &mut pixels, &mut stride) == 0 {
            for y in 0..bounds.bottom {
                for x in 0..bounds.right {
                    let pixel = pixels.cast::<u32>().offset((y * stride + x) as isize);
                    let color = *pixel & 0xffffff;
                    if color == dib(palette[1]) || color == dib(palette[2]) {
                        let alpha = if color == dib(palette[1]) {
                            if !app.ui.transparent_player {
                                255
                            } else {
                                app.ui.light_opacity * 255 / 100
                            }
                        } else {
                            app.ui.surface_opacity * 255 / 100
                        };
                        if app.background.is_some() {
                            let back = backdrop.pixel(
                                x,
                                y + caption,
                                bounds.right,
                                bounds.bottom + caption,
                            );
                            *pixel = blend_pixel(color, back, alpha);
                        } else if app.acrylic {
                            let color = if app.ui.light {
                                blend_pixel(
                                    dib(app.ui.light_tint),
                                    color,
                                    app.ui.light_opacity * 255 / 100,
                                ) & 0xffffff
                            } else {
                                color
                            };
                            let mut premultiplied = alpha << 24;
                            for shift in [0, 8, 16] {
                                premultiplied |= (((color >> shift) & 255) * alpha / 255) << shift;
                            }
                            *pixel = premultiplied;
                        } else if app.ui.background_mode == 3 {
                            *pixel = blend_pixel(color, dib(app.ui.solid_background()), alpha);
                        } else {
                            *pixel |= 0xff000000;
                        }
                    } else if !app.acrylic || app.background.is_some() || color != 0 {
                        *pixel |= 0xff000000;
                    }
                }
            }
        }
        render_mark!("composite");
        drop(frame);
        let memory=native_memory;
        drop(retained_canvas);
        if !accelerated && !partial_scroll && !partial_paint {
            cache_search_backdrop(
                app,
                initial.cast(),
                initial_stride,
                bounds.right,
                bounds.bottom,
            );
        }
        render_mark!("finalize");
        let gpu_presented = accelerated && gpu::finish();
        if accelerated && !gpu_presented {
            EndBufferedPaint(buffer, 0);
            render_region(hwnd, app, dc, None, null_mut());
            return;
        }
        if gpu_presented {
            if !partial_scroll && !partial_paint {
                if let Some(rect) = app.scene.search.map(|r| app.rect(r)) {
                    if let Some(pixels) = gpu::snapshot(rect) {
                        let next=EditBackdrop { width:rect.right-rect.left,height:rect.bottom-rect.top,pixels };
                        let mut cached=app.search_backdrop.borrow_mut();
                        if cached.as_ref()!=Some(&next) {
                            *cached=Some(next);
                            if let Some(control)=app.edits.get(&SEARCH) { InvalidateRect(*control,null(),0); }
                        }
                    }
                } else { app.search_backdrop.borrow_mut().take(); }
            }
            let clear=gpu_dirty.unwrap_or(RECT{left:0,top:0,right:bounds.right,bottom:bounds.bottom});
            for y in clear.top.max(0)..clear.bottom.min(bounds.bottom) {
                let left=clear.left.clamp(0,bounds.right);
                let right=clear.right.clamp(left,bounds.right);
                std::ptr::write_bytes(initial.cast::<u32>().offset((y*initial_stride+left) as isize),0,(right-left) as usize);
            }
        } else { gpu::hide(); }
        BitBlt(
            dc,
            0,
            caption,
            bounds.right,
            bounds.bottom,
            memory,
            0,
            0,
            SRCCOPY,
        );
        if !partial_scroll && !partial_paint {
            paint_caption(
                hwnd,
                app,
                dc,
                memory,
                initial.cast(),
                initial_stride,
                bounds.right,
                caption,
                bounds.bottom + caption,
                &backdrop,
            );
        }
        render_mark!("present");
        EndBufferedPaint(buffer, 0);
        render_memory_metrics(app);
        #[cfg(test)]
        if profile_enabled {
            eprintln!("RENDER_STAGES {:?}", profile_marks);
        }
    }
}
fn lerp_argb(c1: u32, c2: u32, t: f32) -> u32 {
    let t = t.clamp(0.0, 1.0);
    let a1 = ((c1 >> 24) & 0xff) as f32;
    let r1 = ((c1 >> 16) & 0xff) as f32;
    let g1 = ((c1 >> 8) & 0xff) as f32;
    let b1 = (c1 & 0xff) as f32;

    let a2 = ((c2 >> 24) & 0xff) as f32;
    let r2 = ((c2 >> 16) & 0xff) as f32;
    let g2 = ((c2 >> 8) & 0xff) as f32;
    let b2 = (c2 & 0xff) as f32;

    let a = (a1 + (a2 - a1) * t).round() as u32;
    let r = (r1 + (r2 - r1) * t).round() as u32;
    let g = (g1 + (g2 - g1) * t).round() as u32;
    let b = (b1 + (b2 - b1) * t).round() as u32;

    (a << 24) | (r << 16) | (g << 8) | b
}
unsafe fn draw_button(hwnd: HWND, app: &App, draw: &DRAWITEMSTRUCT) {
    let _surface = crate::drawing::SurfaceOpacity::set(app.ui.surface_opacity);
    let index = (draw.CtlID as usize).saturating_sub(1000);
    let Some(node) = app.scene.buttons.get(index) else {
        return;
    };
    let palette = app.palette();
    let active = draw.itemState & ODS_SELECTED != 0;
    let hover = draw.itemState & ODS_HOTLIGHT != 0;
    let disabled = draw.itemState & ODS_DISABLED != 0;
    let mut r = draw.rcItem;
    r.left += 2;
    r.top += 2;
    r.right -= 3;
    r.bottom -= 3;
    let argb = crate::drawing::argb;
    let accent = app.accent();
    if matches!(node.action, Action::SpotifyLogo) {
        crate::drawing::spotify_logo(draw.hDC, r);
        return;
    }
    if matches!(node.action, Action::YouTubeLogo) {
        crate::drawing::youtube_logo(draw.hDC, r);
        return;
    }
    if matches!(node.action, Action::DeezerLogo) {
        crate::drawing::deezer_logo(draw.hDC, r);
        return;
    }

    if let Action::Sort(col) = node.action {
        let is_current = app.ui.sort_column == col;
        let color = if hover || is_current {
            palette[4]
        } else {
            palette[5]
        };
        let font = app.fonts[0];
        let mut label_r = r;
        label_r.left += app.px(2);
        draw_text(
            draw.hDC,
            font,
            &node.text,
            label_r,
            color,
            DT_LEFT | DT_VCENTER | DT_SINGLELINE,
        );
        return;
    }

    let is_toggle = match node.action {
        Action::ToggleBool(key) => {
            !matches!(key, "controls_right" | "expand_palettes" | "expand_themes")
        }
        Action::ToggleLyricSource(_) => true,
        _ => false,
    };
    if is_toggle {
        let toggle_key = match node.action {
            Action::ToggleBool(key) => format!("bool:{key}"),
            Action::ToggleLyricSource(idx) => format!("lyric:{idx}"),
            _ => String::new(),
        };

        let anim_t = if disabled {
            0.0
        } else {
            app.toggle_progress(&toggle_key, node.selected)
        };

        let track_w = app.px(44);
        let track_h = app.px(22);
        let track_x = r.left + (r.right - r.left - track_w) / 2;
        let track_y = r.top + (r.bottom - r.top - track_h) / 2;
        let track_rect = RECT {
            left: track_x,
            top: track_y,
            right: track_x + track_w,
            bottom: track_y + track_h,
        };
        let track_radius = (app.corner(12) as f32).min(track_h as f32 / 2.0);

        let off_fill = argb(
            palette[2],
            if disabled {
                50
            } else if hover {
                160
            } else {
                110
            },
        );
        let on_fill = argb(
            accent,
            if disabled {
                70
            } else if hover {
                255
            } else {
                230
            },
        );
        let track_fill = lerp_argb(off_fill, on_fill, anim_t);

        let off_stroke = argb(
            palette[3],
            if disabled {
                60
            } else if hover {
                220
            } else {
                160
            },
        );
        let on_stroke = argb(
            accent,
            if disabled {
                80
            } else if hover {
                255
            } else {
                230
            },
        );
        let track_stroke = lerp_argb(off_stroke, on_stroke, anim_t);

        crate::drawing::fill(draw.hDC, track_rect, track_fill, track_radius);
        crate::drawing::stroke(draw.hDC, track_rect, track_stroke, track_radius, 1.5);

        let pad = app.px(3) as f32;
        let knob_d = (track_h as f32 - pad * 2.0).max(8.0);
        let travel = track_w as f32 - pad * 2.0 - knob_d;
        let knob_x = track_x as f32 + pad + anim_t * travel;
        let knob_y = track_y as f32 + pad;

        let press_stretch = if active && !disabled {
            app.px(3) as f32
        } else {
            0.0
        };
        let knob_w = knob_d + press_stretch;
        let k_x = if anim_t > 0.5 {
            knob_x - press_stretch
        } else {
            knob_x
        };
        let knob_rect = RECT {
            left: k_x.round() as i32,
            top: knob_y.round() as i32,
            right: (k_x + knob_w).round() as i32,
            bottom: (knob_y + knob_d).round() as i32,
        };
        let knob_radius = knob_d / 2.0;

        crate::drawing::fill(
            draw.hDC,
            RECT {
                left: knob_rect.left,
                top: knob_rect.top + 1,
                right: knob_rect.right,
                bottom: knob_rect.bottom + 1,
            },
            0x25000000,
            knob_radius,
        );

        let off_knob = if disabled {
            argb(palette[5], 110)
        } else if app.ui.light {
            argb(palette[4], 255)
        } else {
            0xffffffff
        };
        let on_knob = if disabled {
            argb(palette[5], 140)
        } else {
            0xffffffff
        };
        let knob_fill = lerp_argb(off_knob, on_knob, anim_t);
        crate::drawing::fill(draw.hDC, knob_rect, knob_fill, knob_radius);
        return;
    }

    if let Action::Preset(preset_idx) = node.action {
        if let Some((_, start, end)) = ui::GRADIENT_PRESETS.get(preset_idx).copied() {
            crate::drawing::gradient_angle(
                draw.hDC,
                r,
                argb(start, 240),
                argb(end, 225),
                app.corner(12) as f32,
                35.0,
            );
            crate::drawing::stroke(
                draw.hDC,
                r,
                argb(
                    if node.selected {
                        palette[4]
                    } else if hover {
                        accent
                    } else {
                        palette[3]
                    },
                    if node.selected || hover { 235 } else { 150 },
                ),
                app.corner(12) as f32,
                if node.selected { 2.0 } else { 1.2 },
            );
            let mut label = r;
            label.left += app.px(10);
            label.right -= app.px(10);
            let avail_w = label.right - label.left;
            let font = if app.ui.view == View::Settings {
                app.fonts[0]
            } else {
                select_fitting_font(draw.hDC, &app.fonts, &node.text, avail_w)
            };
            let lum = (crate::appearance::colorref_luminance(start)
                + crate::appearance::colorref_luminance(end))
                / 2.0;
            let text_color = if lum > 0.5 {
                rgb(20, 20, 24)
            } else {
                rgb(255, 255, 255)
            };
            draw_text(
                draw.hDC,
                font,
                &node.text,
                label,
                text_color,
                DT_LEFT | DT_VCENTER,
            );
            return;
        }
    }

    let theme_card =
        match node.action {
            Action::SolidTheme(index) => ui::SOLID_THEMES.get(index).map(
                |&(_, _, bg, panel, raised, border, text, accent)| {
                    (bg, panel, raised, border, text, accent)
                },
            ),
            Action::CustomTheme(light) if node.text != "Edit" => {
                let defaults = ui::SOLID_THEMES[if light { 12 } else { 1 }];
                let colors = if light {
                    &app.ui.custom_light_colors
                } else {
                    &app.ui.custom_dark_colors
                };
                let color = |key: &str, fallback| colors.get(key).copied().unwrap_or(fallback);
                Some((
                    color("Background", defaults.2),
                    color("Panel", defaults.3),
                    color("Raised", defaults.4),
                    color("Border", defaults.5),
                    color("Text", defaults.6),
                    color("Accent", defaults.7),
                ))
            }
            _ => None,
        };
    if let Some((bg, panel, raised, border, text_col, accent_col)) = theme_card {
        {
            let rad = (app.corner(14) as f32).min((r.bottom - r.top) as f32 * 0.3);
            crate::drawing::gradient(draw.hDC, r, argb(panel, 252), argb(bg, 252), rad);
            crate::drawing::stroke(
                draw.hDC,
                r,
                argb(
                    if node.selected || hover {
                        accent_col
                    } else {
                        border
                    },
                    if node.selected {
                        255
                    } else if hover {
                        225
                    } else {
                        195
                    },
                ),
                rad,
                if node.selected { 2.2 } else { 1.2 },
            );

            let pill_h = (r.bottom - r.top - app.px(28)).clamp(app.px(18), app.px(26));
            let pill_w = app.px(44);
            let pill_r = RECT {
                left: r.right - pill_w - app.px(12),
                top: r.top + (r.bottom - r.top - pill_h) / 2,
                right: r.right - app.px(12),
                bottom: r.top + (r.bottom - r.top - pill_h) / 2 + pill_h,
            };
            let pill_rad = (app.corner(12) as f32).min(pill_h as f32 / 2.0);
            crate::drawing::fill(draw.hDC, pill_r, argb(raised, 255), pill_rad);
            crate::drawing::stroke(draw.hDC, pill_r, argb(border, 220), pill_rad, 1.0);

            let dot_d = (pill_h - app.px(6)).max(10);
            let dot_x = pill_r.left + (pill_r.right - pill_r.left - dot_d) / 2;
            let dot_y = pill_r.top + (pill_r.bottom - pill_r.top - dot_d) / 2;
            let dot_r = RECT {
                left: dot_x,
                top: dot_y,
                right: dot_x + dot_d,
                bottom: dot_y + dot_d,
            };
            crate::drawing::fill(draw.hDC, dot_r, argb(accent_col, 255), dot_d as f32 / 2.0);

            let mut label = r;
            label.left += app.px(14);
            label.right = pill_r.left - app.px(8);
            let avail_w = label.right - label.left;
            let font = select_fitting_font(draw.hDC, &app.fonts, &node.text, avail_w);
            draw_text(
                draw.hDC,
                font,
                &node.text,
                label,
                text_col,
                DT_LEFT | DT_VCENTER,
            );
            return;
        }
    }

    if let Action::Color(key) = node.action {
        let rad = (app.corner(12) as f32).min((r.bottom - r.top) as f32 * 0.36);
        let fill_col = if hover { palette[1] } else { palette[2] };
        crate::drawing::fill(
            draw.hDC,
            r,
            argb(fill_col, if app.ui.light { 235 } else { 165 }),
            rad,
        );
        crate::drawing::stroke(
            draw.hDC,
            r,
            argb(if hover { accent } else { palette[3] }, 210),
            rad,
            1.2,
        );
        let swatch_col = match key {
            "FixedBackgroundColor" => app.ui.solid_color,
            "LightTint" => app.ui.light_tint,
            _ => app.ui.color(key, accent),
        };
        let sw_d = (r.bottom - r.top - app.px(12)).clamp(app.px(14), app.px(20));
        let sw_r = RECT {
            left: r.left + app.px(10),
            top: r.top + (r.bottom - r.top - sw_d) / 2,
            right: r.left + app.px(10) + sw_d,
            bottom: r.top + (r.bottom - r.top - sw_d) / 2 + sw_d,
        };
        crate::drawing::fill(draw.hDC, sw_r, argb(swatch_col, 255), sw_d as f32 / 2.0);
        crate::drawing::stroke(
            draw.hDC,
            sw_r,
            argb(palette[3], 220),
            sw_d as f32 / 2.0,
            1.0,
        );
        let mut label = r;
        label.left = sw_r.right + app.px(8);
        label.right -= app.px(8);
        draw_text(
            draw.hDC,
            app.fonts[0],
            &node.text,
            label,
            palette[4],
            DT_LEFT | DT_VCENTER,
        );
        return;
    }

    if matches!(node.action, Action::SeekMs(_)) {
        return;
    }

    if node.text.is_empty()
        && matches!(
            node.action,
            Action::PlayerLayout(_) | Action::PlayerFullscreen
        )
    {
        let ink = argb(if hover { accent } else { palette[4] }, 235);
        let cx = (r.left + r.right) as f32 / 2.;
        let cy = (r.top + r.bottom) as f32 / 2.;
        let unit = app.scale as f32;
        if matches!(node.action, Action::PlayerLayout(_)) {
            crate::drawing::fill(
                draw.hDC,
                r,
                argb(palette[2], if hover { 180 } else { 95 }),
                (r.bottom - r.top) as f32 / 2.,
            );
            let square = |x: f32, y: f32| RECT {
                left: (cx + x * unit) as i32,
                top: (cy + y * unit) as i32,
                right: (cx + (x + 10.) * unit) as i32,
                bottom: (cy + (y + 10.) * unit) as i32,
            };
            crate::drawing::stroke(draw.hDC, square(-7., -7.), ink, unit, 1.6 * unit);
            crate::drawing::fill(draw.hDC, square(-3., -3.), argb(palette[1], 255), unit);
            crate::drawing::stroke(draw.hDC, square(-3., -3.), ink, unit, 1.6 * unit);
        } else {
            for (x, y) in [(-1., -1.), (1., -1.), (-1., 1.), (1., 1.)] {
                let px = cx + x * 7. * unit;
                let py = cy + y * 7. * unit;
                crate::drawing::line(draw.hDC, px, py, px - x * 5. * unit, py, ink, 1.8 * unit);
                crate::drawing::line(draw.hDC, px, py, px, py - y * 5. * unit, ink, 1.8 * unit);
            }
        }
        return;
    }

    let is_lyrics_back = app.ui.view == View::Lyrics && matches!(node.action, Action::Back);
    if is_lyrics_back {
        let radius = (r.right - r.left) as f32 / 2.0;
        let is_light = app.ui.light;
        let fill_bg = if is_light {
            if active {
                0x55000000
            } else if hover {
                0x35000000
            } else {
                0x18000000
            }
        } else {
            if active {
                0x60ffffff
            } else if hover {
                0x40ffffff
            } else {
                0x25ffffff
            }
        };
        crate::drawing::fill(draw.hDC, r, fill_bg, radius);
        crate::drawing::stroke(
            draw.hDC,
            r,
            if is_light {
                if hover {
                    0x70000000
                } else {
                    0x35000000
                }
            } else {
                if hover {
                    0x90ffffff
                } else {
                    0x50ffffff
                }
            },
            radius,
            1.2,
        );
        draw_text(
            draw.hDC,
            app.fonts[4],
            &node.glyph,
            r,
            if is_light { palette[4] } else { 0xffffffff },
            DT_CENTER | DT_VCENTER | DT_SINGLELINE,
        );
        return;
    }

    let is_cover_control = app.ui.view == View::Lyrics
        && matches!(
            node.action,
            Action::Toggle | Action::Previous | Action::Next | Action::Shuffle | Action::Repeat
        );
    if is_cover_control {
        let radius = (r.right - r.left) as f32 / 2.0;
        let fill = if active {
            0xa0000000
        } else if hover {
            0x88000000
        } else {
            0x60000000
        };
        crate::drawing::fill(draw.hDC, r, fill, radius);
        let stroke_col = if node.selected {
            accent
        } else if hover {
            0xffffffff
        } else {
            0xccffffff
        };
        crate::drawing::stroke(
            draw.hDC,
            r,
            stroke_col,
            radius,
            if matches!(node.action, Action::Toggle) {
                2.0
            } else {
                1.5
            },
        );
        let glyph_col = if node.selected { accent } else { 0xffffffff };
        draw_text(
            draw.hDC,
            app.fonts[4],
            &node.glyph,
            r,
            glyph_col,
            DT_CENTER | DT_VCENTER | DT_SINGLELINE,
        );
        return;
    }

    let is_tile = matches!(&node.action, Action::OpenItem(item) if item.kind == "channel");
    if is_tile {
        crate::drawing::fill(
            draw.hDC,
            r,
            argb(palette[1], if hover { 165 } else { 125 }),
            app.corner(11) as f32,
        );
        crate::drawing::stroke(
            draw.hDC,
            r,
            argb(if hover { accent } else { palette[3] }, 165),
            app.corner(11) as f32,
            1.2,
        );
        if let Some(image) = app.images.get(&node.artwork) {
            let s = (r.bottom - r.top - app.px(18)).clamp(app.px(40), app.px(66));
            let art_r = RECT {
                left: r.right - s - app.px(10),
                top: r.top + (r.bottom - r.top - s) / 2,
                right: r.right - app.px(10),
                bottom: r.top + (r.bottom - r.top - s) / 2 + s,
            };
            faded_bitmap(draw.hDC, art_r, image, app.corner(10), 255);
        }
        let mut label = r;
        label.left += app.px(14);
        label.right -= app.px(82);
        draw_text(
            draw.hDC,
            app.fonts[8],
            &node.text,
            label,
            palette[4],
            DT_LEFT | DT_VCENTER,
        );
        return;
    }

    if node.text_link {
        if let Some(group) = node.artist_credit_group {
            let player_artist =
                app.scene.player_title.as_ref().is_some_and(|title| {
                    group.x == title.region.x && group.y == title.region.y + 25
                });
            let font = if player_artist {
                app.fonts[11]
            } else {
                app.fonts[0]
            };
            let group_rect = app.rect(group);
            let group_w = (group_rect.right - group_rect.left).max(0);

            // Credit buttons are consecutive. Measure each name once per font,
            // then reuse the widths on every animation frame.
            let start = (0..index)
                .rev()
                .take_while(|&i| app.scene.buttons[i].artist_credit_group == Some(group))
                .last()
                .unwrap_or(index);
            let end = index
                + app.scene.buttons[index..]
                    .iter()
                    .take_while(|button| button.artist_credit_group == Some(group))
                    .count();
            let mut total_w = 0i32;
            let mut current_offset = 0i32;
            let mut current_w = 0i32;
            let mut group_text = String::new();
            for (offset, btn) in app.scene.buttons[start..end].iter().enumerate() {
                group_text.push_str(&btn.text);
                let w = automatic_text_width(draw.hDC, font, &btn.text);
                if start + offset == index {
                    current_offset = total_w;
                    current_w = w;
                }
                total_w += w;
            }
            let overflow = total_w - group_w;
            if overflow <= 0 {
                let start_x = group_rect.left
                    + if node.text_centered {
                        (group_w - total_w) / 2
                    } else {
                        0
                    };
                let text_rect = RECT {
                    left: start_x + current_offset,
                    top: group_rect.top,
                    right: start_x + current_offset + current_w + 2,
                    bottom: group_rect.bottom,
                };
                let saved = SaveDC(draw.hDC);
                IntersectClipRect(
                    draw.hDC,
                    group_rect.left,
                    group_rect.top,
                    group_rect.right,
                    group_rect.bottom,
                );
                draw_text(
                    draw.hDC,
                    font,
                    &node.text,
                    text_rect,
                    if hover { accent } else { palette[5] },
                    DT_LEFT | DT_VCENTER | DT_SINGLELINE | crate::drawing::LYRICS_UNTRIMMED,
                );
                RestoreDC(draw.hDC, saved);
                return;
            }

            let now = std::time::Instant::now();
            let key = format!(
                "group:{}:{}:{}:{}:{}",
                group_rect.left, group_rect.top, group_rect.right, group_rect.bottom, group_text
            );
            let elapsed = AUTO_MARQUEES.with(|states| {
                let mut states = states.borrow_mut();
                let state = states.entry(key).or_insert((group_rect, now, now));
                state.0 = group_rect;
                state.2 = now;
                state.1.elapsed().as_secs_f64()
            });
            start_animation_timer(hwnd, 4);
            let shift = marquee_offset(elapsed, overflow as f64, 36. * app.scale) as i32;
            let text_rect = RECT {
                left: group_rect.left + current_offset - shift,
                top: group_rect.top,
                right: group_rect.left + current_offset - shift + current_w + 2,
                bottom: group_rect.bottom,
            };
            let saved = SaveDC(draw.hDC);
            IntersectClipRect(
                draw.hDC,
                group_rect.left,
                group_rect.top,
                group_rect.right,
                group_rect.bottom,
            );
            draw_text(
                draw.hDC,
                font,
                &node.text,
                text_rect,
                if hover { accent } else { palette[5] },
                DT_LEFT | DT_VCENTER | DT_SINGLELINE | crate::drawing::LYRICS_UNTRIMMED,
            );
            RestoreDC(draw.hDC, saved);
            return;
        }

        let font = app.fonts[0];
        draw_automatic_text(
            hwnd,
            draw.hDC,
            font,
            &node.text,
            draw.rcItem,
            if hover { accent } else { palette[5] },
            (if node.text_centered {
                DT_CENTER
            } else {
                DT_LEFT
            }) | DT_VCENTER
                | DT_SINGLELINE
                | DT_END_ELLIPSIS,
            app.scale,
        );
        return;
    }
    if matches!(node.action, Action::QueueJump(_)) {
        if app
            .queue_drag
            .as_ref()
            .is_some_and(|d| d.2 && matches!(node.action, Action::QueueJump(i) if i == d.3))
        {
            crate::drawing::line(
                draw.hDC,
                r.left as f32,
                r.top as f32,
                r.right as f32,
                r.top as f32,
                argb(accent, 255),
                app.px(3) as f32,
            );
        }
        let radius = app.corner(12) as f32;
        crate::drawing::fill(
            draw.hDC,
            r,
            argb(
                if node.selected { accent } else { palette[2] },
                if node.selected {
                    150
                } else if hover {
                    120
                } else {
                    65
                },
            ),
            radius,
        );
        crate::drawing::stroke(
            draw.hDC,
            r,
            argb(if node.selected { accent } else { palette[3] }, 180),
            radius,
            1.0,
        );
        let cover_size = (r.bottom - r.top - app.px(8)).max(1);
        let cover = RECT {
            left: r.left + app.px(4),
            top: r.top + app.px(4),
            right: r.left + app.px(4) + cover_size,
            bottom: r.bottom - app.px(4),
        };
        if let Some(image) = app.images.get(&node.artwork) {
            bitmap(draw.hDC, cover, image, app.corner(8));
        }
        let text = RECT {
            left: cover.right + app.px(10),
            right: r.right - app.px(10),
            ..r
        };
        draw_hover_text(
            hwnd,
            draw.hDC,
            app.fonts[0],
            &node.text,
            text,
            palette[4],
            DT_LEFT | DT_VCENTER | DT_SINGLELINE,
            app.scale,
        );
        return;
    }

    let profile = matches!(node.action, Action::Go(View::Account)) && node.text.is_empty();
    let card = matches!(&node.action, Action::OpenItem(_)) && !node.artwork.is_empty();
    let track = matches!(&node.action, Action::OpenItem(item) if item.playable())
        && !card
        && node.glyph.is_empty();
    let minimal = (node.text.is_empty() && !matches!(node.action, Action::Shelf(_, _))) || track;
    let round_play = matches!(node.action, Action::PlayAll) && node.text.is_empty();
    let radius = if round_play || profile {
        (r.right - r.left) as f32 / 2.0
    } else {
        (app.corner(16) as f32).min((r.bottom - r.top) as f32 * 0.36)
    };

    if round_play {
        let is_light = app.ui.light;
        let (start, end) = app.gradient();
        crate::drawing::stroke(draw.hDC, r, 0x38000000, radius, 3.0);
        crate::drawing::gradient_angle(
            draw.hDC,
            r,
            argb(start, if active { 230 } else { 250 }),
            argb(end, if active { 215 } else { 240 }),
            radius,
            40.0,
        );
        crate::drawing::stroke(
            draw.hDC,
            r,
            argb(palette[3], if hover { 230 } else { 180 }),
            radius,
            1.5,
        );
        let w = (r.right - r.left) as f32;
        let h = (r.bottom - r.top) as f32;
        let x = r.left as f32;
        let y = r.top as f32;
        let tri = [
            (x + w * 0.39, y + h * 0.29),
            (x + w * 0.39, y + h * 0.71),
            (x + w * 0.71, y + h * 0.50),
        ];
        let ink = app.ui.color_with_light(
            "AccentText",
            if crate::appearance::colorref_luminance(start) > 0.45 {
                rgb(20, 20, 24)
            } else {
                0x00ffffff
            },
            is_light,
        );
        crate::drawing::polygon(draw.hDC, &tri, crate::drawing::argb(ink, 255));
        return;
    }

    if profile {
        if hover {
            crate::drawing::fill(draw.hDC, r, argb(palette[2], 165), radius);
            draw_text(
                draw.hDC,
                app.fonts[4],
                "\u{e713}",
                r,
                accent,
                DT_CENTER | DT_VCENTER | DT_SINGLELINE,
            );
            crate::drawing::stroke(draw.hDC, r, argb(accent, 210), radius, 1.5);
            return;
        }
        // Queue grip is geometry rather than a font glyph, so every Windows font works.
        if matches!(node.action, Action::QueueJump(_)) && node.rect.h >= 64 {
            for column in 0..2 {
                for row in 0..3 {
                    let dot = app.rect(Rect::new(
                        node.rect.x + 8 + column * 5,
                        node.rect.y + 23 + row * 6,
                        2,
                        2,
                    ));
                    crate::drawing::fill(draw.hDC, dot, argb(palette[5], 230), app.px(1) as f32);
                }
            }
        }
        let has_art = app.images.get(&node.artwork);
        if let Some(image) = has_art {
            faded_bitmap(draw.hDC, r, image, r.right - r.left, 255);
        } else {
            crate::drawing::fill(
                draw.hDC,
                r,
                argb(palette[2], if hover { 145 } else { 95 }),
                radius,
            );
            crate::drawing::profile_silhouette(draw.hDC, r, argb(palette[4], 230));
        }
        crate::drawing::stroke(
            draw.hDC,
            r,
            argb(if hover { accent } else { palette[3] }, 165),
            radius,
            1.2,
        );
        return;
    }

    if card && hover {
        let mut glow = r;
        // Include the full metadata width, with clearance inside each shelf cell.
        if let Some(label) = app.scene.buttons.iter().find(|label| {
            label.text_link
                && label.rect.y == node.rect.y + node.rect.h + 12
                && label.rect.x + label.rect.w / 2 == node.rect.x + node.rect.w / 2
        }) {
            let label_rect = app.rect(label.rect);
            glow.left = glow.left.min(label_rect.left);
            glow.right = glow.right.max(label_rect.right);
        }
        glow.left -= app.px(6);
        glow.top -= app.px(8);
        glow.right += app.px(6);
        glow.bottom += app.px(64);
        crate::drawing::fill(draw.hDC, glow, argb(accent, 38), app.corner(12) as f32);
    }

    if !card && !track && (!minimal || hover || active || node.selected) {
        let is_light = app.ui.light;
        let state = app.ui.color_with_light(
            if active {
                "Pressed"
            } else if hover {
                "Hover"
            } else {
                "Raised"
            },
            palette[2],
            is_light,
        );
        if node.selected {
            let (top, bottom) = app.gradient();
            crate::drawing::stroke(
                draw.hDC,
                r,
                if is_light { 0x18000000 } else { 0x37000000 },
                radius,
                if is_light { 1.5 } else { 2.5 },
            );
            crate::drawing::gradient(draw.hDC, r, argb(top, 255), argb(bottom, 255), radius);
            crate::drawing::stroke(draw.hDC, r, argb(accent, 235), radius, 1.5);
        } else if is_light {
            let top = if hover || active { palette[1] } else { state };
            let bottom = state;
            let fill_alpha = 255;
            crate::drawing::gradient(
                draw.hDC,
                r,
                argb(top, fill_alpha),
                argb(bottom, fill_alpha),
                radius,
            );
            let border_color = if hover { accent } else { palette[3] };
            let border_alpha = if disabled {
                110
            } else if hover {
                215
            } else {
                185
            };
            crate::drawing::stroke(draw.hDC, r, argb(border_color, border_alpha), radius, 1.2);
        } else {
            let top = state;
            let bottom = app.ui.color_with_light("GradientEnd", palette[1], is_light);
            crate::drawing::stroke(draw.hDC, r, 0x37000000, radius, 2.5);
            crate::drawing::gradient(draw.hDC, r, argb(top, 255), argb(bottom, 255), radius);
            let border_color = if hover { accent } else { palette[3] };
            crate::drawing::stroke(
                draw.hDC,
                r,
                argb(border_color, if hover { 185 } else { 120 }),
                radius,
                1.0,
            );
        }
    }
    let is_light = app.ui.light;
    let ink = if disabled {
        palette[5]
    } else if node.selected {
        let def = if crate::appearance::colorref_luminance(accent) > 0.45 {
            rgb(20, 20, 24)
        } else {
            rgb(255, 255, 255)
        };
        let preferred = app.ui.color_with_light("AccentText", def, is_light);
        let (start, end) = app.gradient();
        crate::appearance::gradient_foreground(preferred, start, end)
    } else if hover {
        app.ui.color_with_light("TextHover", palette[4], is_light)
    } else {
        palette[4]
    };
    let has_art = app.images.get(&node.artwork);
    if let Some(image) = has_art {
        faded_bitmap(
            draw.hDC,
            r,
            image,
            if matches!(&node.action, Action::OpenItem(item) if item.kind == "artist") {
                r.right - r.left
            } else {
                app.corner(14)
            },
            255,
        );
    }
    if !node.glyph.is_empty() && has_art.is_none() {
        let btn_w = r.right - r.left;
        let mut icon = r;
        let icon_pad = if node.text.is_empty() {
            0
        } else if btn_w < app.px(90) {
            app.px(4)
        } else {
            app.px(10)
        };
        let icon_w = if node.text.is_empty() {
            r.right - r.left
        } else if btn_w < app.px(90) {
            app.px(18)
        } else {
            app.px(20)
        };
        if matches!(node.action, Action::Go(View::Offline))
            && app.ui.downloads.iter().any(|job| !job.finished)
        {
            let offset = app.px([0, 1, 3, 5, 6, 5, 3, 1][app.ui.download_frame as usize]);
            icon.top += offset;
            icon.bottom += offset;
        }
        icon.left += icon_pad;
        icon.right = if node.text.is_empty() {
            r.right
        } else {
            icon.left + icon_w
        };
        draw_text(
            draw.hDC,
            app.fonts[4],
            node.glyph,
            icon,
            ink,
            DT_CENTER | DT_VCENTER,
        );
    }
    if !node.text.is_empty() {
        let btn_w = r.right - r.left;
        let btn_h = r.bottom - r.top;
        let mut label = r;
        let glyph_pad = if node.glyph.is_empty() {
            app.px(6)
        } else if btn_w < app.px(90) {
            app.px(24)
        } else {
            app.px(34)
        };
        label.left += glyph_pad;
        label.right -= app.px(if btn_w < app.px(90) { 4 } else { 8 });
        let avail_w = label.right - label.left;
        let media = matches!(node.action, Action::OpenItem(_));
        let font = if media || app.ui.view == View::Settings {
            app.fonts[0]
        } else {
            select_fitting_font(draw.hDC, &app.fonts, &node.text, avail_w)
        };
        let flags = if media {
            DT_LEFT | DT_VCENTER | DT_SINGLELINE
        } else if node.glyph.is_empty() && !track {
            if btn_h >= app.px(42) && node.text.contains(' ') {
                DT_CENTER | DT_VCENTER | DT_WORDBREAK
            } else {
                DT_CENTER | DT_VCENTER
            }
        } else {
            DT_LEFT | DT_VCENTER
        };
        if media {
            draw_hover_text(
                hwnd, draw.hDC, font, &node.text, label, ink, flags, app.scale,
            );
        } else {
            if flags & DT_WORDBREAK == 0 {
                draw_automatic_text(
                    hwnd, draw.hDC, font, &node.text, label, ink, flags, app.scale,
                );
            } else {
                draw_text(draw.hDC, font, &node.text, label, ink, flags);
            }
        }
    }
    if draw.itemState & ODS_FOCUS != 0 {
        let mut focus = r;
        focus.left += 3;
        focus.top += 3;
        focus.right -= 3;
        focus.bottom -= 3;
        DrawFocusRect(draw.hDC, &focus);
    }
}
unsafe fn show_context_menu(hwnd: HWND, app: &mut App, pt: POINT) {
    let (w, h) = size(hwnd, app);
    let hit = app.scene.buttons.iter().find(|b| {
        let r = app.rect(b.rect);
        PtInRect(&r, pt) != 0
    });
    let target_item = if app.ui.view == View::Lyrics
        && hit.is_some_and(|b| matches!(b.action, Action::Go(View::Add)))
    {
        Some(app.ui.now.clone())
    } else if let Some(btn) = hit.filter(|b| matches!(b.action, Action::OpenItem(_))) {
        if let Action::OpenItem(item) = &btn.action {
            Some(item.clone())
        } else {
            None
        }
    } else {
        // Check if pt is on any track row
        app.scene
            .buttons
            .iter()
            .find_map(|b| {
                if let Action::OpenItem(item) = &b.action {
                    let r = app.rect(b.rect);
                    let is_card = !b.artwork.is_empty();
                    if !is_card
                        && !b.text_link
                        && (item.playable() || item.kind == "track" || b.rect.h <= 50)
                        && pt.y >= r.top
                        && pt.y < r.bottom
                        && pt.x >= app.px(180)
                        && pt.x <= w - app.px(20)
                    {
                        return Some(item.clone());
                    }
                    if is_card
                        && pt.x >= r.left
                        && pt.x <= r.right
                        && pt.y >= r.top
                        && pt.y <= r.bottom + app.px(64)
                    {
                        return Some(item.clone());
                    }
                }
                None
            })
            .or_else(|| {
                if pt.y >= (h - app.px(96)) && app.ui.queue_index < app.ui.queue.len() {
                    Some(app.ui.queue[app.ui.queue_index].clone())
                } else if pt.y >= (h - app.px(96)) && !app.ui.now.id.is_empty() {
                    Some(app.ui.now.clone())
                } else if pt.y < app.px(app.scene.content_top) {
                    app.entity.clone()
                } else {
                    None
                }
            })
    };

    if let Some(item) = target_item {
        let menu = CreatePopupMenu();
        let removable = app.ui.view == View::Offline
            && ((item.kind == "offline_collection" && !item.id.starts_with("__"))
                || (item.playable()
                    && app
                        .entity
                        .as_ref()
                        .is_some_and(|e| e.kind == "offline_collection")));
        if removable {
            AppendMenuW(
                menu,
                MF_STRING,
                15,
                wide(app.ui.t("Delete download")).as_ptr(),
            );
            AppendMenuW(menu, MF_SEPARATOR, 0, null());
        }

        if item.playable() {
            AppendMenuW(menu, MF_STRING, 3, wide(app.ui.t("Add to queue")).as_ptr());
        } else {
            AppendMenuW(menu, MF_STRING, 1, wide(app.ui.t("Open")).as_ptr());
        }
        AppendMenuW(menu, MF_SEPARATOR, 0, null());
        if item.kind == "track" {
            AppendMenuW(
                menu,
                MF_STRING,
                4,
                wide(app.ui.t("Track information")).as_ptr(),
            );
        }
        if item.kind != "episode" && item.album_entry().is_some() {
            AppendMenuW(menu, MF_STRING, 5, wide(app.ui.t("Go to album")).as_ptr());
        }
        if item.kind != "episode" && item.artist_entry().is_some() {
            AppendMenuW(menu, MF_STRING, 6, wide(app.ui.t("Go to artist")).as_ptr());
        }
        AppendMenuW(menu, MF_SEPARATOR, 0, null());
        let liked = app.ui.liked_ids.contains(&item.library_key());
        AppendMenuW(
            menu,
            MF_STRING
                | if library_job(&item, !liked).is_ok() {
                    0
                } else {
                    MF_GRAYED
                },
            7,
            wide(app.ui.t(context_menu_favorite_label(&item, liked))).as_ptr(),
        );
        let mut menu_playlists = Vec::new();
        if item.kind == "track" {
            let sub = CreatePopupMenu();
            AppendMenuW(sub, MF_STRING, 13, wide(app.ui.t("New playlist")).as_ptr());
            menu_playlists = app.page_cache.iter()
                .flat_map(|(_, page)| page.sections.iter().flat_map(|section| section.entries.iter()).chain(page.rows.iter()))
                .filter(|entry| entry.kind == "playlist" && crate::playlist::can_edit(entry, &app.ui.account))
                .fold(Vec::new(), |mut entries: Vec<crate::catalog::Entry>, entry| {
                    if !entries.iter().any(|e| e.id == entry.id) { entries.push(entry.clone()); }
                    entries
                });
            if !menu_playlists.is_empty() {
                AppendMenuW(sub, MF_SEPARATOR, 0, null());
                for (index, playlist) in menu_playlists.iter().take(100).enumerate() {
                    AppendMenuW(sub, MF_STRING, 1000 + index, wide(&playlist.title).as_ptr());
                }
            }
            AppendMenuW(
                sub,
                MF_STRING,
                10,
                wide(app.ui.t("Choose playlist")).as_ptr(),
            );
            AppendMenuW(
                menu,
                MF_POPUP,
                sub as usize,
                wide(app.ui.t("Add to playlist")).as_ptr(),
            );
        }
        if item.playable() {
            AppendMenuW(menu, MF_STRING, 8, wide(app.ui.t("Download")).as_ptr());
        }
        if share_link(&item).is_some() {
            AppendMenuW(menu, MF_STRING, 9, wide(app.ui.t("Share")).as_ptr());
        }

        if crate::playlist::can_edit(&item, &app.ui.account) {
            AppendMenuW(
                menu,
                MF_STRING,
                11,
                wide(app.ui.t("Edit playlist")).as_ptr(),
            );
        }
        if item.kind == "track" && app.entity.as_ref().is_some_and(|e| e.kind == "playlist") {
            AppendMenuW(
                menu,
                MF_STRING,
                14,
                wide(app.ui.t("Remove from playlist")).as_ptr(),
            );
        }
        let mut screen_pt = pt;
        content_to_screen(hwnd, &mut screen_pt);
        let sel = TrackPopupMenu(
            menu,
            TPM_RETURNCMD | TPM_RIGHTBUTTON,
            screen_pt.x,
            screen_pt.y,
            0,
            hwnd,
            null(),
        );
        DestroyMenu(menu);

        match sel {
            1000..=1099 => {
                if let Some(playlist) = menu_playlists.get((sel - 1000) as usize) {
                    submit_playlist(app, crate::playlist::Mutation::Add(playlist.id.clone(), item.id.clone()));
                    rebuild(hwnd, app);
                }
            }
            1 => activate(hwnd, app, Action::OpenItem(item.clone())),
            3 => {
                if command(app, Command::QueueAddLast(Box::new(item.clone()))) {
                    app.notice = "Updating queue...".into();
                }
                rebuild(hwnd, app);
            }
            4 => activate(hwnd, app, Action::OpenTrackInfo(item.clone())),
            5 => {
                if let Some(album) = item.album_entry() {
                    activate(hwnd, app, Action::OpenItem(album));
                }
            }
            6 => {
                if let Some(artist) = item.artist_entry() {
                    activate(hwnd, app, Action::OpenItem(artist));
                }
            }
            7 => {
                toggle_library(app, item.clone());
                rebuild(hwnd, app);
            }
            8 => {
                if let Some(engine) = app.snapshot.engine.clone() {
                    let dl_id = app.ui.downloads.iter().map(|j| j.id).max().unwrap_or(0) + 1;
                    let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
                    let queued = app.downloads.enqueue(crate::downloads::Request {
                        id: dl_id,
                        engine,
                        items: vec![{
                            let mut single = item.clone();
                            single.raw["_download_single"] = true.into();
                            single
                        }],
                        quality: app.ui.value("Download quality"),
                        cancelled: cancel.clone(),
                    });
                    if let Err(error) = queued {
                        app.notice = error;
                    } else {
                        app.download_cancels.insert(dl_id, cancel);
                        app.ui.downloads.push(crate::downloads::Status {
                            id: dl_id,
                            title: item.title.clone(),
                            total: 1,
                            message: "Queued".into(),
                            ..Default::default()
                        });
                        app.notice = "Download queued".into();
                    }
                } else {
                    app.notice = "Connect to Deezer before downloading.".into();
                }
                rebuild(hwnd, app);
            }
            9 => {
                if let Some(link) = share_link(&item) {
                    app.notice = match copy_text(hwnd, &link) {
                        Ok(()) => { show_link_copied(hwnd, app); "Link copied".into() },
                        Err(error) => error,
                    };
                }
                rebuild(hwnd, app);
            }
            10 => activate(hwnd, app, Action::ChoosePlaylist(item.clone())),
            11 => activate(hwnd, app, Action::EditPlaylist(item.clone())),
            13 => {
                if app.ui.playlist_busy {
                    return;
                }
                activate(hwnd, app, Action::Go(View::PlaylistEdit));
                app.ui.playlist_draft.seed = Some(item.clone());
            }
            15 if removable => {
                match app.downloads.remove_selection(
                    app.snapshot.engine.clone(),
                    item.clone(),
                    app.entity.clone(),
                ) {
                    Ok(()) => app.notice = "Removing download...".into(),
                    Err(error) => app.notice = error,
                }
                rebuild(hwnd, app);
            }
            14 => {
                if let Some(playlist) = app.entity.as_ref().filter(|e| e.kind == "playlist") {
                    let mutation =
                        crate::playlist::Mutation::Remove(playlist.id.clone(), item.id.clone());
                    submit_playlist(app, mutation);
                }
            }
            _ => {}
        }
    }
}
unsafe extern "system" fn procedure(hwnd: HWND, msg: u32, wp: usize, lp: isize) -> isize {
    if msg == WM_NCCREATE {
        let create = &*(lp as *const CREATESTRUCTW);
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, create.lpCreateParams as isize);
        return DefWindowProcW(hwnd, msg, wp, lp);
    }
    if let Some(result) = caption_message(hwnd, msg, wp, lp) {
        return result;
    }
    let pointer = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut App;
    if pointer.is_null() {
        return DefWindowProcW(hwnd, msg, wp, lp);
    }
    let app = &mut *pointer;
    if matches!(msg,WM_LBUTTONDOWN|WM_LBUTTONUP|WM_KEYDOWN|WM_CHAR|WM_SIZE|WM_DPICHANGED|WM_CONTEXTMENU){gpu::invalidate_layers();}
    match msg {
        gpu::UPLOAD_WAKE=>{start_animation_timer(hwnd,9);return 0;}
        WM_CREATE => {
            SendMessageW(
                hwnd,
                WM_CHANGEUISTATE,
                (UIS_SET | (UISF_HIDEFOCUS << 16)) as usize,
                0,
            );
            BufferedPaintInit();
            let round = DWMWCP_ROUND;
            DwmSetWindowAttribute(
                hwnd,
                DWMWA_WINDOW_CORNER_PREFERENCE as u32,
                &round as *const _ as _,
                4,
            );
            for id in [SEARCH, SESSION, TRACK, NAME, DESCRIPTION] {
                let style = ES_AUTOHSCROLL as u32
                    | if matches!(id, SESSION) {
                        ES_PASSWORD as u32
                    } else if id == DESCRIPTION {
                        ES_MULTILINE as u32 | ES_AUTOVSCROLL as u32
                    } else {
                        0
                    };
                let control = make_control(hwnd, "EDIT", id, style, app);
                app.edits.insert(id, control);
            }
            for (id, hint) in [
                (
                    SEARCH,
                    "Search songs, artists, albums, playlists or podcasts...",
                ),
                (SESSION, "Paste ARL, or leave empty to use saved session"),
                (TRACK, "Deezer track ID"),
                (NAME, "Playlist name"),
            ] {
                SendMessageW(
                    app.edits[&id],
                    EM_SETCUEBANNER,
                    1,
                    wide(hint).as_ptr() as isize,
                );
            }

            for id in [SEEK, VOLUME] {
                let control = make_control(hwnd, "msctls_trackbar32", id, TBS_NOTICKS, app);
                SendMessageW(
                    control,
                    TBM_SETRANGE,
                    1,
                    (if id == SEEK { SEEK_STEPS as isize } else { 100 }) << 16,
                );
                app.edits.insert(id, control);
            }
            SendMessageW(app.edits[&VOLUME], TBM_SETPOS, 1, app.volume as isize);
            command(app, Command::Volume(app.volume));
            let status = make_control(hwnd, "STATIC", STATUS, 0, app);
            app.edits.insert(STATUS, status);
            SetWindowTextW(
                status,
                wide("Open your profile to connect to Deezer.").as_ptr(),
            );
            let collapse_button =
                make_control(hwnd, "BUTTON", COLLAPSE_BUTTON, BS_OWNERDRAW as u32, app);
            SetWindowTextW(collapse_button, wide("Toggle sidebar").as_ptr());
            app.collapse_button = collapse_button;
            for (i, title) in ["Minimize", "Maximize or restore", "Close"]
                .into_iter()
                .enumerate()
            {
                let button = make_control(
                    hwnd,
                    "BUTTON",
                    CAPTION_BUTTON_BASE + i as i32,
                    BS_OWNERDRAW as u32,
                    app,
                );
                SetWindowTextW(button, wide(app.ui.t(title)).as_ptr());
                app.caption_buttons[i] = button;
            }
            material(hwnd, app);
            apply_window_controls(hwnd, app);
            sync_tray(hwnd, app);
            app.shortcuts = Some(crate::shortcuts::Shortcuts::new(hwnd));
            restore_shortcuts(app);
            match crate::media_controls::Controls::new(hwnd) {
                Ok(controls) => app.media_controls = Some(controls),
                Err(error) => app.notice = format!("Windows media controls unavailable: {error}"),
            }
            SetTimer(hwnd, 1, 200, None);
            SetWindowPos(
                hwnd,
                null_mut(),
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED,
            );
            let provider = startup_provider(
                app.ui.last_provider,
                crate::session::deezer_saved_exists(),
                crate::spotify::api_configured(),
                crate::session::provider_exists("youtube"),
            );
            if provider != app.ui.provider {
                switch_provider(app, provider);
            } else {
                load_page(app);
            }
            rebuild(hwnd, app);
            if std::env::var_os("DEEZNATS_NO_AUTOLOGIN").is_none()
                && app.ui.provider == crate::catalog::Provider::Deezer
                && command(app, Command::LoginSaved)
            {
                let q = app.ui.value("Audio quality");
                command(
                    app,
                    Command::Api("DZSetQuality".into(), vec![serde_json::json!(q)]),
                );
                app.pending = Some(1);
                app.notice = "Connecting to your saved Deezer account...".into();
                status_text(app);
            }
            0
        }
        WM_GETMINMAXINFO => {
            let limits = &mut *(lp as *mut MINMAXINFO);
            limits.ptMinTrackSize = POINT { x: 480, y: 360 };
            0
        }
        WM_SIZE => {
            if wp == SIZE_MINIMIZED as usize {
                SetTimer(hwnd, 6, MINIMIZED_CACHE_DELAY_MS, None);
                return 0;
            }
            KillTimer(hwnd, 6);
            rebuild(hwnd, app);
            let (width, _) = size(hwnd, app);
            let sidebars = (
                app.ui.sidebar_collapsed,
                app.ui.queue_sidebar_open,
                app.ui.device_menu_open,
            );
            app.ui.fit_sidebars(width);
            if sidebars
                != (
                    app.ui.sidebar_collapsed,
                    app.ui.queue_sidebar_open,
                    app.ui.device_menu_open,
                )
            {
                rebuild(hwnd, app);
            }
            0
        }
        WM_DPICHANGED => {
            frame_pacing::refresh(hwnd);
            let rect = &*(lp as *const RECT);
            SetWindowPos(
                hwnd,
                null_mut(),
                rect.left,
                rect.top,
                rect.right - rect.left,
                rect.bottom - rect.top,
                SWP_NOZORDER | SWP_NOACTIVATE,
            );
            rebuild(hwnd, app);
            0
        }
        WM_ERASEBKGND => 1,
        WM_DISPLAYCHANGE | WM_MOVE => {
            frame_pacing::refresh(hwnd);
            DefWindowProcW(hwnd, msg, wp, lp)
        }
        WM_ACTIVATE | WM_NCACTIVATE => {
            if msg == WM_ACTIVATE {
                let inactive = wp & 0xffff == WA_INACTIVE as usize;
                app.render_suspended = IsIconic(hwnd) != 0;
                let interval = if inactive {
                    match app.ui.value("Background FPS") {
                        1 => 1_000_000_000 / 90,
                        2 => 1_000_000_000 / 60,
                        3 => 1_000_000_000 / 30,
                        _ => 0,
                    }
                } else {
                    0
                };
                BACKGROUND_FRAME_INTERVAL_NS.store(interval, std::sync::atomic::Ordering::Relaxed);
                refresh_animation_timers(hwnd);
                if inactive || app.render_suspended {
                    if app.ui.shortcut_capture.take().is_some() {
                        restore_shortcuts(app);
                    }
                    if app.render_suspended {
                        app.visualizer.set_active(false);
                        stop_animation_timer(hwnd, 2);
                        stop_animation_timer(hwnd, 4);
                    }
                    KillTimer(hwnd, 5);
                    SetTimer(hwnd, 1, 1000, None);
                } else {
                    SetTimer(hwnd, 1, 100, None);
                    app.lyric_motion = None;
                    app.lyric_motion_offset = 0;
                    app.lyrics_mode_motion = None;
                    app.ui.lyrics_cover_mix = None;
                    let target = app.scroll_target;
                    settle_scroll(app, target);
                    rebuild(hwnd, app);
                    start_animation_timer(hwnd, 2);
                    InvalidateRect(hwnd, null(), 0);
                }
            }
            let result = DefWindowProcW(hwnd, msg, wp, lp);
            if app.acrylic {
                keep_acrylic(hwnd, true);
            }
            result
        }
        WM_PRINTCLIENT => {
            if !app.render_suspended {
                render(hwnd, app, wp as HDC, None);
            }
            0
        }
        WM_PAINT => {
            paint(hwnd, app);
            0
        }
        WM_DRAWITEM => 1,
        WM_CTLCOLORSTATIC | WM_CTLCOLOREDIT => {
            let palette = app.palette();
            SetTextColor(wp as HDC, palette[4]);
            SetBkColor(wp as HDC, palette[1]);
            app.brush as isize
        }
        WM_CLOSE => {
            if app.ui.keep_playing_on_close {
                ShowWindow(hwnd, if app.tray_added { SW_HIDE } else { SW_MINIMIZE });
            } else {
                DestroyWindow(hwnd);
            }
            0
        }
        TRAY_CALLBACK => {
            if matches!(lp as u32, WM_LBUTTONUP | WM_LBUTTONDBLCLK) {
                ShowWindow(hwnd, SW_RESTORE);
                SetForegroundWindow(hwnd);
            } else if lp as u32 == WM_RBUTTONUP {
                let menu = CreatePopupMenu();
                AppendMenuW(menu, MF_STRING, 1, wide(app.ui.t("Open DeezNats")).as_ptr());
                AppendMenuW(menu, MF_STRING, 2, wide(app.ui.t("Quit")).as_ptr());
                let mut point: POINT = zeroed();
                GetCursorPos(&mut point);
                SetForegroundWindow(hwnd);
                let selected = TrackPopupMenu(
                    menu,
                    TPM_RETURNCMD | TPM_RIGHTBUTTON,
                    point.x,
                    point.y,
                    0,
                    hwnd,
                    null(),
                );
                DestroyMenu(menu);
                if selected == 1 {
                    ShowWindow(hwnd, SW_RESTORE);
                    SetForegroundWindow(hwnd);
                } else if selected == 2 {
                    DestroyWindow(hwnd);
                }
            }
            0
        }
        WM_COMMAND => {
            let caption_id = (wp & 0xffff) as i32;
            if caption_id == COLLAPSE_BUTTON {
                activate(hwnd, app, Action::ToggleSidebar);
                return 0;
            }
            if (CAPTION_BUTTON_BASE..CAPTION_BUTTON_BASE + 3).contains(&caption_id) {
                match caption_id - CAPTION_BUTTON_BASE {
                    0 => {
                        ShowWindow(hwnd, SW_MINIMIZE);
                    }
                    1 => {
                        ShowWindow(
                            hwnd,
                            if IsZoomed(hwnd) != 0 {
                                SW_RESTORE
                            } else {
                                SW_MAXIMIZE
                            },
                        );
                    }
                    _ => {
                        SendMessageW(hwnd, WM_CLOSE, 0, 0);
                    }
                }
                return 0;
            }

            if wp & 0xffff == SEARCH as usize
                && wp >> 16 == EN_CHANGE as usize
                && app.ui.view == View::FlowTune
            {
                app.ui.flow_filter = read(app.edits[&SEARCH]);
                app.ui.flow_genre_limit = 30;
                app.ui.scroll = 0;
                rebuild(hwnd, app);
                return 0;
            }
            if wp >> 16 == BN_CLICKED as usize {
                let index = (wp & 0xffff).saturating_sub(1000);
                activate_scene_button(hwnd, app, index);
            }
            0
        }
        WM_MOUSEWHEEL => {
            if app.ui.right_sidebar_open() {
                let mut pt: POINT = zeroed();
                GetCursorPos(&mut pt);
                screen_to_content(hwnd, &mut pt);
                let (width, _) = size(hwnd, app);
                if pt.x >= app.px(width - 280.min(width / 3) - 12) {
                    let delta = ((wp >> 16) as u16) as i16 as i32;
                    let steps = if delta == 0 {
                        0
                    } else {
                        delta.signum() * ((delta.abs() / 120).max(1)) * 3
                    };
                    if app.scene.device_sidebar.is_some() {
                        app.ui.device_sidebar_scroll = (app.ui.device_sidebar_scroll - steps)
                            .clamp(0, app.scene.queue_scroll_max as i32);
                        rebuild(hwnd, app);
                    } else {
                        let mut lines = 3u32;
                        SystemParametersInfoW(
                            SPI_GETWHEELSCROLLLINES,
                            0,
                            (&mut lines as *mut u32).cast(),
                            0,
                        );
                        let viewport = app.scene.queue_viewport.map_or(0, |r| r.h);
                        let pixels =
                            wheel_pixels(delta, lines, viewport, &mut app.queue_wheel_remainder);
                        let current =
                            (app.ui.queue_scroll * 24 + app.ui.queue_scroll_offset) as f64;
                        if pixels==0 {return 0;}
                        if app.ui.smooth_scrolling {
                            app.queue_glide = Some(retarget_queue_glide(app.queue_glide,current,pixels,
                                (app.scene.queue_scroll_max*24) as f64,std::time::Instant::now()));
                            start_animation_timer(hwnd, 8);
                        } else {
                            app.queue_glide = None;
                            set_queue_scroll(hwnd, app, current as i32 - pixels);
                        }
                    }
                    return 0;
                }
            }

            if app.ui.view == View::Lyrics && app.ui.timed_lyrics() && !app.ui.lyrics_manual_scroll
            {
                let position = app
                    .scene
                    .lyrics_anchor
                    .map(|anchor| anchor + app.ui.scroll)
                    .unwrap_or(app.ui.scroll)
                    .max(0);
                app.ui.hold_lyric_follow(std::time::Instant::now());
                settle_scroll(app, position);
                rebuild(hwnd, app);
            }
            app.ui.hold_lyric_follow(std::time::Instant::now());
            let delta = ((wp >> 16) as u16) as i16 as i32;
            let (_, h) = size(hwnd, app);
            let bottom = if app.ui.view == View::Lyrics {
                h - 64
            } else {
                content_bottom(app, h)
            };
            let maximum = (app.scene.content_height - bottom).max(0);
            let mut lines = 3u32;
            SystemParametersInfoW(
                SPI_GETWHEELSCROLLLINES,
                0,
                (&mut lines as *mut u32).cast(),
                0,
            );
            let pixels = wheel_pixels(
                delta,
                lines,
                content_bottom(app, h) - app.scene.content_top,
                &mut app.wheel_remainder,
            );
            if pixels != 0 {
                if app.ui.smooth_scrolling && app.scroll_target != app.ui.scroll {
                    let now = std::time::Instant::now();
                    (app.scroll_position, app.scroll_velocity) = smooth_scroll_step(
                        app.scroll_position,
                        app.scroll_velocity,
                        app.scroll_target as f64,
                        now.duration_since(app.scroll_tick).as_secs_f64(),
                    );
                    app.scroll_tick = now;
                    let position = app.scroll_position.round() as i32;
                    scroll_to(hwnd, app, position);
                }
                let idle = app.scroll_target == app.ui.scroll;
                if idle || (app.scroll_target - app.ui.scroll).signum() == pixels.signum() {
                    app.scroll_target = app.ui.scroll;
                }
                app.scroll_target = (app.scroll_target - pixels).clamp(0, maximum);
                if app.ui.smooth_scrolling {
                    if idle {
                        app.scroll_position = app.ui.scroll as f64;
                        app.scroll_velocity = 0.0;
                        app.scroll_tick = std::time::Instant::now();
                    }
                    if (app.scroll_target as f64 - app.scroll_position) * app.scroll_velocity < 0.0
                    {
                        app.scroll_velocity = 0.0;
                    }
                    start_animation_timer(hwnd, 2);
                } else {
                    scroll_to(hwnd, app, app.scroll_target);
                }
            }
            0
        }
        WM_MBUTTONDOWN => {
            if app.middle_scroll.take().is_some() {
                return 0;
            }
            let x = ((lp as u32 & 0xffff) as i16) as f64 / app.scale;
            let y = ((((lp as u32 >> 16) as u16) as i16) as i32 - caption_height(hwnd)) as f64
                / app.scale;
            let inside = |r: ui::Rect| {
                x >= r.x as f64
                    && x < (r.x + r.w) as f64
                    && y >= r.y as f64
                    && y < (r.y + r.h) as f64
            };
            let queue = app.ui.queue_sidebar_open && app.scene.queue_viewport.is_some_and(inside);
            let lyrics =
                app.ui.view == View::Lyrics && app.scene.lyrics_area.is_some_and(inside) && !queue;
            if queue || lyrics {
                if lyrics && app.ui.timed_lyrics() && !app.ui.lyrics_manual_scroll {
                    let position = app
                        .scene
                        .lyrics_anchor
                        .map(|anchor| anchor + app.ui.scroll)
                        .unwrap_or(app.ui.scroll)
                        .max(0);
                    app.ui.hold_lyric_follow(std::time::Instant::now());
                    settle_scroll(app, position);
                    rebuild(hwnd, app);
                }
                app.queue_glide = None;
                app.middle_scroll = Some((queue, y as i32, false, std::time::Instant::now()));
                SetCapture(hwnd);
                start_animation_timer(hwnd, 8);
                return 0;
            }
            0
        }
        WM_MBUTTONUP => {
            if let Some((_, origin, dragged, _)) = app.middle_scroll {
                let y = ((((lp as u32 >> 16) as u16) as i16) as i32 - caption_height(hwnd)) as f64
                    / app.scale;
                if dragged || (y as i32 - origin).abs() > 8 {
                    app.middle_scroll = None;
                }
                if GetCapture() == hwnd {
                    ReleaseCapture();
                }
                return 0;
            }
            0
        }
        WM_RBUTTONDOWN if app.middle_scroll.is_some() => {
            app.middle_scroll = None;
            if GetCapture() == hwnd {
                ReleaseCapture();
            }
            0
        }
        WM_LBUTTONDOWN => {
            if app.middle_scroll.take().is_some() {
                if GetCapture() == hwnd {
                    ReleaseCapture();
                }
                return 0;
            }
            let (w, h) = size(hwnd, app);
            let x = ((lp as u32 & 0xffff) as i16) as f64 / app.scale;
            let y = ((((lp as u32 >> 16) as u16) as i16) as i32 - caption_height(hwnd)) as f64
                / app.scale;

            if app.scene.device_popover_anchor.is_some() {
                if let Some((rect, _, _)) = app.scene.auth_modal {
                    if x < rect.x as f64
                        || x > (rect.x + rect.w) as f64
                        || y < rect.y as f64
                        || y > (rect.y + rect.h) as f64
                    {
                        app.ui.device_menu_open = false;
                        rebuild(hwnd, app);
                    }
                    return 0;
                }
            }
            if let Some(track) = app.scene.queue_scrollbar {
                if x >= (track.x - 5) as f64
                    && x <= (track.x + track.w + 5) as f64
                    && y >= track.y as f64
                    && y <= (track.y + track.h) as f64
                {
                    app.queue_scroll_drag = true;
                    SetCapture(hwnd);
                    queue_scroll_pointer(hwnd, app, y as i32);
                    return 0;
                }
            }
            if app.ui.lyrics_volume_open {
                app.ui.lyrics_volume_open = false;
                rebuild(hwnd, app);
            }
            if let Some(graph) = app.scene.eq_graph {
                if y >= app.scene.content_top as f64
                    && x >= graph.x as f64
                    && x <= (graph.x + graph.w) as f64
                    && y >= graph.y as f64
                    && y <= (graph.y + graph.h) as f64
                {
                    app.eq_drag = Some((0, 0));
                    SetCapture(hwnd);
                    update_eq_from_graph(hwnd, app, x as i32, y as i32, graph);
                    return 0;
                }
            }
            if let Some(sliders) = app.scene.eq_sliders {
                if y >= app.scene.content_top as f64
                    && x >= sliders.x as f64
                    && x <= (sliders.x + sliders.w) as f64
                    && y >= sliders.y as f64
                    && y <= (sliders.y + sliders.h) as f64
                {
                    let pad_x = 20;
                    let col_w = (sliders.w - pad_x * 2).max(1) / 11;
                    let rel_x = (x as i32 - (sliders.x + pad_x)).max(0);
                    let band = (rel_x / col_w).clamp(0, 10) as usize;
                    app.eq_drag = Some((1, band));
                    SetCapture(hwnd);
                    update_eq_from_sliders(hwnd, app, band, y as i32, sliders);
                    return 0;
                }
            }

            if app.scene.panels.iter().any(|panel| {
                panel.kind == 5
                    && x >= (panel.rect.x - 5) as f64
                    && x <= (panel.rect.x + panel.rect.w + 5) as f64
                    && y >= panel.rect.y as f64
                    && y <= (panel.rect.y + panel.rect.h) as f64
            }) {
                app.scrolling = true;
                app.ui.hold_lyric_follow(std::time::Instant::now());
                SetCapture(hwnd);
                let thumb = app
                    .scene
                    .panels
                    .iter()
                    .find(|p| p.kind == 6)
                    .map(|p| p.rect)
                    .unwrap_or(Rect::new(0, 0, 0, 32));
                app.scroll_grab_offset = if y >= thumb.y as f64 && y <= (thumb.y + thumb.h) as f64 {
                    y as i32 - thumb.y
                } else {
                    thumb.h / 2
                };
                let bottom = if app.ui.view == View::Lyrics {
                    h - 64
                } else {
                    content_bottom(app, h)
                };
                let position =
                    scrollbar_position(&app.scene, y as i32, app.scroll_grab_offset, bottom);
                scroll_to_scrollbar_position(hwnd, app, position);
            }
            if !app.scrolling {
                if let Some(index) = canvas_track_hit(app, x as i32, y as i32, w, h) {
                    if let Action::QueueJump(queue_index) = app.scene.buttons[index].action {
                        app.queue_drag = Some((
                            queue_index,
                            app.px(y as i32),
                            false,
                            queue_index,
                            app.ui.queue.iter().map(|e| e.id.clone()).collect(),
                        ));
                        app.canvas_focus = Some(index);
                        SetFocus(hwnd);
                        SetCapture(hwnd);
                        InvalidateRect(hwnd, null(), 0);
                        return 0;
                    }
                    let key = canvas_action_key(&app.scene.buttons[index].action);
                    SetFocus(hwnd);
                    SetCapture(hwnd);
                    app.canvas_focus = Some(index);
                    app.canvas_pressed = Some((index, key));
                    InvalidateRect(hwnd, null(), 0);
                    return 0;
                }
                app.canvas_focus = None;
                if !app.canvas_keyboard.is_null() && GetFocus() == app.canvas_keyboard {
                    SetFocus(hwnd);
                }
            }
            0
        }
        WM_MOUSEMOVE => {
            if app.queue_scroll_drag {
                let mut pt: POINT = zeroed();
                GetCursorPos(&mut pt);
                screen_to_content(hwnd, &mut pt);
                queue_scroll_pointer(hwnd, app, (pt.y as f64 / app.scale) as i32);
                return 0;
            }
            if app.queue_drag.is_some() {
                let mut pt: POINT = zeroed();
                GetCursorPos(&mut pt);
                screen_to_content(hwnd, &mut pt);
                let y = (pt.y as f64 / app.scale) as i32;
                let (from, start, active, _, _) = app.queue_drag.as_ref().unwrap();
                let active = *active || (pt.y - *start).abs() > app.px(5);
                if active && *from > app.ui.queue_index {
                    let target = app
                        .scene
                        .buttons
                        .iter()
                        .filter_map(|b| match b.action {
                            Action::QueueJump(i) if i > app.ui.queue_index => {
                                Some((i, (b.rect.y + b.rect.h / 2 - y).abs()))
                            }
                            _ => None,
                        })
                        .min_by_key(|(_, distance)| *distance)
                        .map(|(i, _)| i)
                        .unwrap_or(*from);
                    let drag = app.queue_drag.as_mut().unwrap();
                    drag.2 = true;
                    drag.3 = target;
                    SetTimer(hwnd, 5, 80, None);
                    InvalidateRect(hwnd, null(), 0);
                }
                return 0;
            }

            let mut tracking = TRACKMOUSEEVENT {
                cbSize: size_of::<TRACKMOUSEEVENT>() as u32,
                dwFlags: TME_LEAVE,
                hwndTrack: hwnd,
                dwHoverTime: 0,
            };
            TrackMouseEvent(&mut tracking);

            let (w, h) = size(hwnd, app);
            let x = ((lp as u32 & 0xffff) as i16) as f64 / app.scale;
            let y = ((((lp as u32 >> 16) as u16) as i16) as i32 - caption_height(hwnd)) as f64
                / app.scale;
            let hovered = canvas_track_hit(app, x as i32, y as i32, w, h);
            if app.canvas_hover != hovered {
                app.canvas_hover = hovered;
                // Scrolling suppresses row hover painting. Keep its retained
                // lists intact; the final animation paint restores hover.
                if !main_scroll_animating(app)&&app.queue_glide.is_none()&&!app.scrolling&&!app.queue_scroll_drag {
                    gpu::invalidate_layers();
                    InvalidateRect(hwnd, null(), 0);
                }
            }

            if app.ui.view == View::Lyrics {
                let mut pt: POINT = zeroed();
                GetCursorPos(&mut pt);
                screen_to_content(hwnd, &mut pt);
                let hovered = if let Some(cr) = app.scene.lyrics_cover {
                    PtInRect(&app.rect(cr), pt) != 0
                } else {
                    false
                };
                if app.ui.lyrics_cover_hovered != hovered {
                    app.set_lyrics_cover_hover(hovered);
                    start_animation_timer(hwnd, 2);
                    rebuild(hwnd, app);
                }
            }

            if app.scrolling {
                app.ui.hold_lyric_follow(std::time::Instant::now());
                let (_, h) = size(hwnd, app);
                let y = ((((lp as u32 >> 16) as u16) as i16) as i32 - caption_height(hwnd)) as f64
                    / app.scale;
                let bottom = if app.ui.view == View::Lyrics {
                    h - 64
                } else {
                    content_bottom(app, h)
                };
                let position =
                    scrollbar_position(&app.scene, y as i32, app.scroll_grab_offset, bottom);
                scroll_to_scrollbar_position(hwnd, app, position);
                return 0;
            }
            if let Some((mode, band)) = app.eq_drag {
                let x = ((lp as u32 & 0xffff) as i16) as f64 / app.scale;
                let y = ((((lp as u32 >> 16) as u16) as i16) as i32 - caption_height(hwnd)) as f64
                    / app.scale;
                if mode == 0 {
                    if let Some(graph) = app.scene.eq_graph {
                        update_eq_from_graph(hwnd, app, x as i32, y as i32, graph);
                    }
                } else {
                    if let Some(sliders) = app.scene.eq_sliders {
                        update_eq_from_sliders(hwnd, app, band, y as i32, sliders);
                    }
                }
                return 0;
            }
            0
        }
        WM_MOUSELEAVE => {
            if app.canvas_hover.take().is_some() {
                InvalidateRect(hwnd, null(), 0);
            }
            if app.ui.view == View::Lyrics && app.ui.lyrics_cover_hovered {
                let mut pt: POINT = zeroed();
                GetCursorPos(&mut pt);
                screen_to_content(hwnd, &mut pt);
                let still_in_cover = if let Some(cr) = app.scene.lyrics_cover {
                    PtInRect(&app.rect(cr), pt) != 0
                } else {
                    false
                };
                if !still_in_cover {
                    app.set_lyrics_cover_hover(false);
                    start_animation_timer(hwnd, 2);
                    rebuild(hwnd, app);
                }
            }
            0
        }
        WM_CAPTURECHANGED => {
            app.canvas_pressed = None;
            app.queue_drag = None;
            app.queue_scroll_drag = false;
            KillTimer(hwnd, 5);
            0
        }
        WM_LBUTTONUP => {
            if let Some((index, key)) = app.canvas_pressed.take() {
                let (w, h) = size(hwnd, app);
                let x = ((lp as u32 & 0xffff) as i16) as f64 / app.scale;
                let y = ((((lp as u32 >> 16) as u16) as i16) as i32 - caption_height(hwnd)) as f64
                    / app.scale;
                let valid = canvas_track_hit(app, x as i32, y as i32, w, h)
                    .filter(|&hit| hit == index)
                    .and_then(|hit| app.scene.buttons.get(hit))
                    .is_some_and(|node| canvas_action_matches(&key, &node.action));
                ReleaseCapture();
                if valid {
                    activate_scene_button(hwnd, app, index);
                }
                InvalidateRect(hwnd, null(), 0);
                return 0;
            }
            if let Some((from, _, active, to, expected)) = app.queue_drag.take() {
                KillTimer(hwnd, 5);
                ReleaseCapture();
                if active && from != to {
                    command(app, Command::QueueMove { from, to, expected });
                } else if !active {
                    activate(hwnd, app, Action::QueueJump(from));
                }
                InvalidateRect(hwnd, null(), 0);
                return 0;
            }
            if app.queue_scroll_drag {
                app.queue_scroll_drag = false;
                ReleaseCapture();
                return 0;
            }

            if app.scrolling {
                app.scrolling = false;
                ReleaseCapture();
            }
            if app.eq_drag.is_some() {
                app.eq_drag = None;
                ReleaseCapture();
            }
            0
        }
        WM_RBUTTONUP => {
            let x = ((lp as i32) & 0xffff) as i16 as i32;
            let y = (((lp as i32) >> 16) & 0xffff) as i16 as i32 - caption_height(hwnd);
            show_context_menu(hwnd, app, POINT { x, y });
            0
        }
        WM_CONTEXTMENU => {
            let x = ((lp as i32) & 0xffff) as i16 as i32;
            let y = (((lp as i32) >> 16) & 0xffff) as i16 as i32;
            let pt = if x == -1 && y == -1 {
                if let Some(node) = app
                    .canvas_focus
                    .filter(|&index| canvas_track_button(app, index))
                    .and_then(|index| app.scene.buttons.get(index))
                    .filter(|_| GetFocus() == hwnd || GetFocus() == app.canvas_keyboard)
                {
                    let rect = app.rect(node.rect);
                    POINT {
                        x: (rect.left + rect.right) / 2,
                        y: (rect.top + rect.bottom) / 2,
                    }
                } else {
                    let mut cursor = zeroed();
                    GetCursorPos(&mut cursor);
                    screen_to_content(hwnd, &mut cursor);
                    cursor
                }
            } else {
                let mut pt = POINT { x, y };
                screen_to_content(hwnd, &mut pt);
                pt
            };
            show_context_menu(hwnd, app, pt);
            0
        }
        WM_HSCROLL | WM_VSCROLL => {
            let id = GetDlgCtrlID(lp as HWND);
            let value = SendMessageW(lp as HWND, 0x0400, 0, 0) as u32;
            if id >= 600 {
                let idx = (id - 600) as usize;
                if let Some((_, key, _)) = app.scene.sliders.get(idx).copied() {
                    let min_val = 0;
                    let max_val =
                        if matches!(key, "Light-mode brightness" | "Background brightness") {
                            110
                        } else {
                            100
                        };
                    let pos = value.clamp(min_val, max_val);
                    app.scene.sliders[idx].2 = pos;
                    let target = match key {
                        "Image blur" => &mut app.ui.blur,
                        "Light-mode brightness" | "Background brightness" => &mut app.ui.brightness,
                        "Light-mode opacity" | "Background tint opacity" => {
                            &mut app.ui.light_opacity
                        }
                        "Player bar opacity" | "Surface opacity" => &mut app.ui.surface_opacity,
                        _ => &mut app.ui.surface_opacity,
                    };
                    *target = pos;
                    if let Some((rect, _, _)) = app.scene.sliders.iter().find(|(_, k, _)| *k == key)
                    {
                        for label in &mut app.scene.labels {
                            if label.rect.x == rect.x + rect.w + 8 && label.rect.y == rect.y + 6 {
                                label.text = format!("{}%", pos);
                            }
                        }
                    }
                    if key == "Image blur" {
                        SetTimer(hwnd, 3, 60, None);
                    } else {
                        material(hwnd, app);
                        update_background(hwnd, app);
                        SetTimer(hwnd, 3, 300, None);
                    }
                    InvalidateRect(hwnd, null(), 0);
                }
            } else if id == VOLUME {
                app.volume = value;
                app.ui.initial_volume = value;
                SetTimer(hwnd, 3, 300, None);
                if command(app, Command::Volume(value)) {
                    KillTimer(hwnd, 7);
                } else {
                    SetTimer(hwnd, 7, 75, None);
                }
                rebuild(hwnd, app);
            } else if id == SEEK && (wp as u32 & 0xffff) == TB_ENDTRACK {
                let position = app.ui.duration.saturating_mul(value as u64) / SEEK_STEPS as u64;
                command(app, Command::SeekMs(position));
                app.snapshot.position = position;
                app.playback_clock
                    .sample(position, std::time::Instant::now(), false);
                app.ui.position = position;
            }
            0
        }
        WM_TIMER if wp == 91 => {
            KillTimer(hwnd, 91);
            let toast = GetDlgItem(hwnd, 509);
            if !toast.is_null() { DestroyWindow(toast); }
            0
        }
        WM_TIMER if wp == 7 => {
            KillTimer(hwnd, 7);
            command(app, Command::Volume(app.volume));
            0
        }
        WM_TIMER if wp == 6 => {
            KillTimer(hwnd, 6);
            if IsIconic(hwnd) != 0 {
                gpu::hide();
                *app.backdrop_cache.borrow_mut() = Default::default();
                crate::drawing::clear_lyric_rasters();
                trim_minimized_artwork(app);
                let current = page_key(app);
                trim_inactive_pages(&mut app.page_cache, &current);
                app.catalog.session.release_inactive_pages(app.generation);
                crate::catalog::expire_public_cache();
            }
            0
        }
        WM_TIMER if wp == 5 => {
            if app.queue_drag.as_ref().is_some_and(|d| d.2) {
                if let Some(area) = app.scene.queue_viewport {
                    let mut pt: POINT = zeroed();
                    GetCursorPos(&mut pt);
                    screen_to_content(hwnd, &mut pt);
                    let y = (pt.y as f64 / app.scale) as i32;
                    let step = if y < area.y + 28 {
                        -1
                    } else if y > area.y + area.h - 28 {
                        1
                    } else {
                        0
                    };
                    let next = (app.ui.queue_scroll as i32 + step)
                        .clamp(0, app.scene.queue_scroll_max as i32)
                        as usize;
                    if next != app.ui.queue_scroll {
                        app.ui.queue_scroll = next;
                        app.ui.queue_scroll_offset = 0;
                        rebuild(hwnd, app);
                        // Recompute the drop row after autoscrolling, even with a stationary pointer.
                        let target = app
                            .scene
                            .buttons
                            .iter()
                            .filter_map(|b| match b.action {
                                Action::QueueJump(i) if i > app.ui.queue_index => {
                                    Some((i, (b.rect.y + b.rect.h / 2 - y).abs()))
                                }
                                _ => None,
                            })
                            .min_by_key(|(_, distance)| *distance)
                            .map(|(i, _)| i);
                        if let (Some(drag), Some(target)) = (&mut app.queue_drag, target) {
                            drag.3 = target;
                        }
                    }
                }
            } else {
                KillTimer(hwnd, 5);
            }
            0
        }
        WM_TIMER if wp == 4 => {
            let mut cursor: POINT = zeroed();
            GetCursorPos(&mut cursor);
            screen_to_content(hwnd, &mut cursor);
            let mut marquee_rects = Vec::new();
            let active = MARQUEE.with(|state| {
                let mut state = state.borrow_mut();
                let active = state.as_ref().is_some_and(|(_, rect, _, painted)| {
                    PtInRect(rect, cursor) != 0
                        && painted.elapsed().as_millis() < 100
                        && IsWindowVisible(hwnd) != 0
                        && IsIconic(hwnd) == 0
                });
                if !active {
                    *state = None;
                } else if let Some((_, rect, _, _)) = state.as_ref() {
                    marquee_rects.push(*rect);
                }
                active
            });
            let automatic = AUTO_MARQUEES.with(|states| {
                let mut states = states.borrow_mut();
                states.retain(|_, (_, _, painted)| painted.elapsed().as_millis() < 250);
                for (rect, _, _) in states.values() {
                    marquee_rects.push(*rect);
                }
                !states.is_empty()
            }) && IsIconic(hwnd) == 0;
            if !active && !automatic {
                stop_animation_timer(hwnd, 4);
            }
            if active || automatic {
                if marquee_rects.is_empty() {
                    InvalidateRect(hwnd, null(), 0);
                } else {
                    let caption = caption_height(hwnd);
                    for rect in marquee_rects {
                        let client_rect = RECT {
                            left: rect.left,
                            top: rect.top + caption,
                            right: rect.right,
                            bottom: rect.bottom + caption,
                        };
                        InvalidateRect(hwnd, &client_rect, 0);
                    }
                }
            }
            0
        }
        WM_TIMER if wp == 9 => {
            let deferred=gpu::take_deferred_uploads();
            if IsIconic(hwnd)!=0||!deferred {
                stop_animation_timer(hwnd,9);
            }else{
                let (_,h)=size(hwnd,app);let caption=caption_height(hwnd);
                for rect in artwork_damage::all(&app.scene,content_bottom(app,h)).rects {
                    let mut rect=app.rect(rect);rect.top+=caption;rect.bottom+=caption;
                    InvalidateRect(hwnd,&rect,0);
                }
            }
            0
        }
        WM_TIMER if wp==10=>{
            KillTimer(hwnd,10);
            if !app.render_suspended {let (_,h)=size(hwnd,app);trim_artwork(app,h);}
            0
        }
        WM_TIMER if wp == 8 => {
            let now = std::time::Instant::now();
            if let Some((position, velocity, target, tick)) = app.queue_glide.take() {
                if app.ui.queue_sidebar_open && !app.queue_scroll_drag {
                    let (next, speed) = smooth_scroll_step(
                        position,
                        velocity,
                        target,
                        now.saturating_duration_since(tick).as_secs_f64(),
                    );
                    let before=app.ui.queue_scroll*24+app.ui.queue_scroll_offset;
                    set_queue_scroll(hwnd, app, next.round() as i32);
                    if before as i32==next.round() as i32&&gpu::available(){if let Some(frame)=app.scene.queue_frame{let mut r=app.rect(frame);let caption=caption_height(hwnd);r.top+=caption;r.bottom+=caption;InvalidateRect(hwnd,&r,0);}}
                    if (next - target).abs() >= 0.25 || speed.abs() >= 5.0 {
                        app.queue_glide = Some((next, speed, target, now));
                    }
                }
            }
            if let Some((queue, origin, dragged, tick)) = app.middle_scroll.take() {
                if (queue && app.ui.queue_sidebar_open) || (!queue && app.ui.view == View::Lyrics) {
                    let mut cursor: POINT = zeroed();
                    GetCursorPos(&mut cursor);
                    screen_to_content(hwnd, &mut cursor);
                    let displacement = (cursor.y as f64 / app.scale) as i32 - origin;
                    let dt = now.saturating_duration_since(tick).as_secs_f64().min(0.1);
                    let pixels = if displacement.abs() > 8 {
                        ((displacement.signum() as f64)
                            * ((displacement.abs() - 8) as f64)
                            * 4.0
                            * dt)
                            .round() as i32
                    } else {
                        0
                    };
                    if pixels != 0 {
                        if queue {
                            let current =
                                (app.ui.queue_scroll * 24 + app.ui.queue_scroll_offset) as i32;
                            set_queue_scroll(hwnd, app, current.saturating_add(pixels));
                        } else {
                            app.ui.hold_lyric_follow(now);
                            let (_, h) = size(hwnd, app);
                            let max = (app.scene.content_height - (h - 64)).max(0);
                            let position = app.ui.scroll.saturating_add(pixels).clamp(0, max);
                            scroll_to(hwnd, app, position);
                            settle_scroll(app, position);
                        }
                    }
                    app.middle_scroll =
                        Some((queue, origin, dragged || displacement.abs() > 8, now));
                }
            }
            if app.queue_glide.is_none() && app.middle_scroll.is_none() {
                stop_animation_timer(hwnd, 8);
            }
            0
        }
        WM_TIMER if wp == 3 => {
            KillTimer(hwnd, 3);
            update_background(hwnd, app);
            app.ui.save_preferences();
            InvalidateRect(hwnd, null(), 0);
            0
        }
        WM_TIMER if wp == 2 => {
            app.visualizer.set_output_volume(app.volume);
            app.visualizer.set_active(
                !app.render_suspended
                    && app.ui.view == View::Lyrics
                    && app.ui.lyrics_visualizer
                    && app.playing()
            );
            if IsWindowVisible(hwnd) == 0 || IsIconic(hwnd) != 0 {
                stop_animation_timer(hwnd, 2);
                return 0;
            }
            let distance = app.scroll_target - app.ui.scroll;
            let prepared_artwork_changed = crate::drawing::poll_prepared_artwork();
            let mut dirty = prepared_artwork_changed;
            let lyrics_active =
                app.ui.view == View::Lyrics && app.ui.timed_lyrics() && app.playing();
            if lyrics_active {
                let position = app.playback_position();
                let changed = app.lyric_layout_changed(position);
                app.ui.position = position;
                if changed {
                    rebuild(hwnd, app);
                    dirty = true;
                }
            }
            if let Some((started, from, target)) = app.lyrics_mode_motion {
                let progress = (started.elapsed().as_secs_f32() / 0.32).min(1.);
                let ease = progress * progress * (3. - 2. * progress);
                app.ui.lyrics_cover_mix = Some(from + (target - from) * ease);
                if progress >= 1. || app.ui.view != View::Lyrics {
                    app.lyrics_mode_motion = None;
                    app.ui.lyrics_cover_mix = None;
                }
                rebuild(hwnd, app);
                dirty = true;
            }
            let now = std::time::Instant::now();
            if distance != 0||main_scroll_animating(app) {
                if app.scroll_position.round() as i32 != app.ui.scroll {
                    app.scroll_position = app.ui.scroll as f64;
                    app.scroll_velocity = 0.0;
                }
                let elapsed = now.duration_since(app.scroll_tick).as_secs_f64();
                app.scroll_tick = now;
                (app.scroll_position, app.scroll_velocity) = smooth_scroll_step(
                    app.scroll_position,
                    app.scroll_velocity,
                    app.scroll_target as f64,
                    elapsed,
                );
                let position = app.scroll_position.round() as i32;
                let before=app.ui.scroll;
                scroll_to(hwnd, app, position);
                if before==position&&gpu::available(){let (w,h)=size(hwnd,app);let r=scroll_repaint_rect(app,app.px(w),h,caption_height(hwnd));InvalidateRect(hwnd,&r,0);}
                // Let WM_PAINT combine scrolling with artwork/fade invalidations
                // from this tick. Synchronous painting here rendered a second
                // frame when the remainder of the handler invalidated again.
            }
            app.background_transition.advance(now);
            let background_fading = app.background_transition.animating();
            if app.previous_background.is_some() && app.background_transition.progress(now) >= 0.5 {
                app.previous_background = None;
                dirty = true;
            }
            let cover_fading = app.previous_cover.is_some()
                && app.images.contains_key(&app.ui.now.artwork)
                && app.cover_changed.is_some_and(|started| {
                    now.saturating_duration_since(started).as_millis() < ART_FADE_MS
                });
            if app.previous_cover.is_some()
                && app.images.contains_key(&app.ui.now.artwork)
                && !cover_fading
            {
                app.previous_cover = None;
                dirty = true;
            }
            let toggling = app
                .toggle_animations
                .values()
                .any(|(_, _, started)| now.saturating_duration_since(*started).as_millis() < 200);
            app.toggle_animations.retain(|_, (_, _, started)| {
                now.saturating_duration_since(*started).as_millis() < 250
            });

            let palette_fading =
                app.palette_state.borrow().started.elapsed().as_millis() < PALETTE_FADE_MS;
            let lyrics_hover_fading = app.ui.view == View::Lyrics
                && app.lyrics_hover_from != app.lyrics_hover_to
                && now
                    .saturating_duration_since(app.lyrics_hover_start)
                    .as_millis()
                    < 180;
            // Cached offscreen artwork must not keep the entire window animating.
            let image_fading = app.ui.view==View::Lyrics && app
                .scene
                .pictures
                .iter()
                .map(|(_, url)| url)
                .any(|url| {
                    app.images.contains_key(url)
                        && app.image_times.get(url).is_some_and(|started| {
                            now.saturating_duration_since(*started).as_millis() < ART_FADE_MS + 20
                        })
                });
            let fading = background_fading
                || cover_fading
                || palette_fading
                || lyrics_hover_fading
                || image_fading;
            if fading || toggling {
                dirty = true;
            }
            let visualizer_active =
                app.ui.view == View::Lyrics && app.ui.lyrics_visualizer && app.playing();
            if let Some((started, initial)) = app.lyric_motion {
                let t = (started.elapsed().as_secs_f32() / 0.24).min(1.0);
                let ease = t * t * (3.0 - 2.0 * t);
                let offset = (initial as f32 * (1.0 - ease)).round() as i32;
                shift_scroll_content(&mut app.scene, offset - app.lyric_motion_offset);
                app.lyric_motion_offset = offset;
                let (width, height) = size(hwnd, app);
                position_content_buttons(app, width, height);
                if t >= 1.0 {
                    app.lyric_motion = None;
                }
                dirty = true;
            }
            let notes_active = app.scene.lyrics_gap.is_some() && app.playing();
            let karaoke_active = app.ui.karaoke_lyrics
                && (app.scene.karaoke_preview.is_some()
                    || (app.ui.view == View::Lyrics
                        && app.playing()
                        && app
                            .ui
                            .rich_lyrics
                            .iter()
                            .any(|line| !line.syllables.is_empty())));
            // Wave and bars only occupy the bottom of the client area. Keep
            // their animation from repainting the artwork and lyrics above it.
            let visualizer_only = visualizer_active
                && app.ui.visualizer_style < 2
                && !dirty
                && !karaoke_active
                && !notes_active
                && distance == 0
                && !main_scroll_animating(app);
            if visualizer_active || karaoke_active || notes_active {
                dirty = true;
            }
            let finished = !main_scroll_animating(app)
                && app.lyrics_mode_motion.is_none()
                && !fading
                && !toggling
                && !visualizer_active
                && !karaoke_active
                && !notes_active
                && !lyrics_active
                && app.lyric_motion.is_none()
                && !crate::drawing::artwork_preparation_pending();
            if finished {
                stop_animation_timer(hwnd, 2);
                // Commit the exact end state even if this tick crossed a fade's deadline.
                dirty = true;
            }
            // Buttons are painted into the parent's buffered frame. Invalidating
            // every native hit target adds redundant WM_PAINT/WM_DRAWITEM work.
            if dirty {
                let player_artwork_fading =
                    app.image_times
                        .get(&app.ui.now.artwork)
                        .is_some_and(|started| {
                            now.saturating_duration_since(*started).as_millis() < ART_FADE_MS + 20
                        });
                let content_only = image_fading
                    && !prepared_artwork_changed
                    && !background_fading
                    && !cover_fading
                    && !palette_fading
                    && !player_artwork_fading
                    && !toggling
                    && !matches!(app.ui.view, View::Settings | View::Lyrics);
                let mut client: RECT = zeroed();
                GetClientRect(hwnd, &mut client);
                let (_, height) = size(hwnd, app);
                let region = scroll_repaint_rect(app, client.right, height, caption_height(hwnd));
                if visualizer_only {
                    let region = RECT {
                        left: 0,
                        top: (client.bottom - 220 - app.px(4)).max(caption_height(hwnd)),
                        right: client.right,
                        bottom: client.bottom,
                    };
                    InvalidateRect(hwnd, &region, 0);
                } else if content_only
                    && scroll_repaint_enabled()
                    && scroll_repaint_avoids_search(app, &region, caption_height(hwnd))
                {
                    InvalidateRect(hwnd, &region, 0);
                } else {
                    InvalidateRect(hwnd, null(), 0);
                }
            }
            0
        }
        WM_TIMER => {
            let refreshing = match app.ui.provider {
                crate::catalog::Provider::Spotify => crate::spotify::web::refreshing(),
                crate::catalog::Provider::YouTube => crate::youtube::refreshing(),
                _ => false,
            };
            if app.ui.spotify_credentials_refreshing != refreshing {
                app.ui.spotify_credentials_refreshing = refreshing;
                if !app.render_suspended {
                    rebuild(hwnd, app);
                }
            }
            #[cfg(feature = "memory-profile")]
            if wp == 1 {
                crate::memory_profile::sample();
            }
            if !app.render_suspended && app.ui.downloads.iter().any(|job| !job.finished) {
                app.ui.download_frame = (app.ui.download_frame + 1) % 8;
                rebuild(hwnd, app);
            }
            if process_updates(hwnd, app) {
                return 0;
            }
            if !app.render_suspended
                && !app.scrolling
                && app.ui.resume_lyric_follow(std::time::Instant::now())
            {
                let previous_scroll = app.ui.scroll;
                settle_scroll(app, 0);
                rebuild(hwnd, app);
                if let Some(anchor) = app.scene.lyrics_anchor {
                    let offset = anchor - previous_scroll;
                    app.lyric_motion = Some((std::time::Instant::now(), offset));
                    app.lyric_motion_offset = offset;
                    shift_scroll_content(&mut app.scene, offset);
                    let (width, height) = size(hwnd, app);
                    position_content_buttons(app, width, height);
                    start_animation_timer(hwnd, 2);
                }
            }
            if let Some(snapshot) = app
                .worker
                .updates
                .try_iter()
                .filter(|snapshot| snapshot.provider == app.ui.provider)
                .last()
            {
                let progress_changed = app.snapshot.position != snapshot.position
                    || app.snapshot.duration != snapshot.duration
                    || app.snapshot.message != snapshot.message;
                let youtube_account_changed = snapshot.provider
                    == crate::catalog::Provider::YouTube
                    && app.snapshot.account != snapshot.account;
                let newly_connected =
                    snapshot.connected && (!app.snapshot.connected || youtube_account_changed);
                if youtube_account_changed {
                    app.page_cache.clear();
                    app.catalog.session.invalidate();
                    app.ui.liked_ids.clear();
                    if !snapshot.connected {
                        app.generation = app.generation.wrapping_add(1);
                        app.catalog.set_generation(app.generation);
                        app.ui.page = None;
                        app.entity = None;
                        app.library_loaded = false;
                    }
                }
                let queue_data_changed = !Arc::ptr_eq(&app.snapshot.queue, &snapshot.queue);
                let queue_track_changed = app.snapshot.queue_index != snapshot.queue_index
                    || app.snapshot.now.id != snapshot.now.id;
                let queue_changed =
                    app.snapshot.queue_index != snapshot.queue_index || queue_data_changed;
                let clock_changed = app.snapshot.position != snapshot.position
                    || app.snapshot.state != snapshot.state
                    || app.snapshot.now.id != snapshot.now.id;
                if clock_changed {
                    let smooth = app.playing()
                        && snapshot.state == "Playing"
                        && app.snapshot.now.id == snapshot.now.id;
                    app.playback_clock
                        .sample(snapshot.position, std::time::Instant::now(), smooth);
                }
                let position = app
                    .playback_clock
                    .position(std::time::Instant::now(), snapshot.state == "Playing");
                let lyric_layout_changed = app.lyric_layout_changed(position);
                let unavailable = snapshot
                    .unavailable_at
                    .is_some_and(|at| at.elapsed() < std::time::Duration::from_secs(4));
                let unavailable_changed = app.ui.playback_unavailable != unavailable;
                app.ui.playback_unavailable = unavailable;
                let changed = unavailable_changed
                    || app.snapshot.connected != snapshot.connected
                    || app.snapshot.account != snapshot.account
                    || app.snapshot.title != snapshot.title
                    || app.snapshot.state != snapshot.state
                    || app.snapshot.now.artwork != snapshot.now.artwork
                    || app.snapshot.now.subtitle != snapshot.now.subtitle
                    || app.snapshot.quality != snapshot.quality
                    || app.snapshot.repeat != snapshot.repeat
                    || app.snapshot.shuffle != snapshot.shuffle
                    || queue_changed
                    || lyric_layout_changed;
                let completed = app
                    .pending
                    .is_some_and(|attempt| snapshot.login_attempt >= attempt);
                if completed {
                    app.pending = None;
                    app.ui.deezer_login_busy = false;
                    app.notice.clear();
                    if snapshot.connected {
                        app.ui.deezer_login_open = false;
                        app.ui.deezer_login_advanced = false;
                        app.ui.deezer_login_status.clear();
                    } else {
                        app.ui.deezer_login_status = snapshot.message.clone();
                    }
                }
                let youtube_playback_error = snapshot.provider == crate::catalog::Provider::YouTube
                    && snapshot.state == "Stopped"
                    && snapshot.message.starts_with("YouTube playback:")
                    && (app.snapshot.message != snapshot.message || app.snapshot.state != snapshot.state);
                app.snapshot = snapshot;
                if youtube_playback_error {
                    MessageBoxW(
                        hwnd,
                        wide(&app.snapshot.message).as_ptr(),
                        wide("YouTube playback").as_ptr(),
                        MB_OK | MB_ICONINFORMATION,
                    );
                }
                if newly_connected && app.ui.provider == crate::catalog::Provider::Deezer {
                    if let Some(engine) = app.snapshot.engine.clone() {
                        for (status, cancel) in app.downloads.resume(engine) {
                            app.download_cancels.insert(status.id, cancel);
                            app.ui.downloads.push(status);
                        }
                    }
                }
                if let Some(controls) = &mut app.media_controls {
                    if let Err(error) = controls.update(&app.snapshot) {
                        app.notice = format!("Windows media controls: {error}");
                    }
                }
                if let Some(connect) = &app.connect {
                    connect.update(&app.snapshot);
                }
                if app.snapshot.connected
                    && app.ui.provider != crate::catalog::Provider::YouTube
                    && !app.library_loaded
                    && matches!(app.ui.view, View::Show | View::Playlists | View::Podcasts)
                {
                    app.library_loaded = true;
                    data_request(
                        app,
                        crate::catalog::Job::Api("RustPodcastLibraryJSON".into(), vec![]),
                    );
                }
                if !app.snapshot.discord_settings.is_null()
                    && app.discord_seen != app.snapshot.discord_settings
                {
                    let config = &app.snapshot.discord_settings;
                    app.ui.show_on_discord = config["enabled"].as_bool().unwrap_or(false);
                    app.ui.discord_paused = config["show_paused"].as_bool().unwrap_or(false);
                    app.ui.discord_badge = config["badge"].as_bool().unwrap_or(true);
                    app.ui.choices.insert(
                        "Status name",
                        config["status_name"].as_u64().unwrap_or(0).min(3) as usize,
                    );
                    app.ui.discord_hide_details = config["hide_details"].as_bool().unwrap_or(false);
                    app.ui.discord_btn_service = config["service_button"].as_bool().unwrap_or(true);
                    app.ui.discord_btn_app = config["app_button"].as_bool().unwrap_or(true);
                    app.discord_seen = config.clone();
                }
                if app.eq_drag.is_none() && !app.snapshot.eq.is_null() {
                    let mut eq_changed = false;
                    if let Some(bands) = app.snapshot.eq.get("gainsDb").and_then(|v| v.as_array()) {
                        for (i, val) in bands.iter().take(10).enumerate() {
                            if let Some(g) = val.as_f64() {
                                let clamped = (g.round() as i32).clamp(-12, 12);
                                if app.ui.eq[i] != clamped {
                                    app.ui.eq[i] = clamped;
                                    eq_changed = true;
                                }
                            }
                        }
                    }
                    if let Some(preamp) = app.snapshot.eq.get("preampDb").and_then(|v| v.as_f64()) {
                        let clamped = (preamp.round() as i32).clamp(-12, 12);
                        if app.ui.eq[10] != clamped {
                            app.ui.eq[10] = clamped;
                            eq_changed = true;
                        }
                    }
                    if let Some(enabled) = app.snapshot.eq.get("enabled").and_then(|v| v.as_bool())
                    {
                        let cur_en = app.ui.value("Equalizer");
                        let new_en = if enabled { 1 } else { 0 };
                        if cur_en != new_en {
                            app.ui.choices.insert("Equalizer", new_en);
                            eq_changed = true;
                        }
                    }
                    if let Some(preset_str) = app.snapshot.eq.get("preset").and_then(|v| v.as_str())
                    {
                        let normalized = preset_str.replace('-', " ");
                        if let Some(idx) = ui::options("Preset")
                            .iter()
                            .position(|&p| p.eq_ignore_ascii_case(&normalized))
                        {
                            if app.ui.value("Preset") != idx {
                                app.ui.choices.insert("Preset", idx);
                                eq_changed = true;
                            }
                        }
                    }
                    if eq_changed && app.ui.view == View::Settings {
                        rebuild(hwnd, app);
                    }
                }
                {
                    app.ui.now = app.snapshot.now.clone();
                }
                let caption = if app.ui.now.title.is_empty() {
                    "DeezNats".into()
                } else if app.ui.now.subtitle.is_empty() {
                    app.ui.now.title.clone()
                } else {
                    format!("{} \u{2014} {}", app.ui.now.title, app.ui.now.subtitle)
                };
                if read(hwnd) != caption {
                    SetWindowTextW(hwnd, wide(&caption).as_ptr());
                    InvalidateRect(hwnd, null(), 0);
                }
                {
                    app.ui.quality = app.snapshot.quality.clone();
                }
                app.ui.account = app.snapshot.account.clone();
                {
                    app.ui.position = app.playback_position();
                    app.ui.duration = app.snapshot.duration;
                    app.ui.repeat = app.snapshot.repeat;
                    app.ui.shuffle = app.snapshot.shuffle;
                    if app.ui.autoplay != app.snapshot.autoplay {
                        app.ui.autoplay = app.snapshot.autoplay;
                        app.ui.save_preferences();
                    }
                    if queue_track_changed {
                        let viewport = app.scene.queue_viewport.map_or(0, |r| r.h.max(0) as usize);
                        let anchor = app
                            .snapshot
                            .queue_index
                            .saturating_mul(72)
                            .saturating_sub(viewport / 4);
                        app.ui.queue_scroll = anchor / 24;
                        app.ui.queue_scroll_offset = anchor % 24;
                        app.queue_glide = None;
                    }
                    if !app.snapshot.queue.is_empty() {
                        if queue_data_changed || app.ui.queue.is_empty() {
                            app.ui.queue = (*app.snapshot.queue).clone();
                        }
                        app.ui.queue_index = app.snapshot.queue_index;
                    } else if !app.snapshot.now.id.is_empty() {
                        app.ui.queue = vec![app.snapshot.now.clone()];
                        app.ui.queue_index = 0;
                    } else {
                        app.ui.queue.clear();
                        app.ui.queue_index = 0;
                    }
                }
                if newly_connected {
                    if app.ui.provider == crate::catalog::Provider::Deezer {
                        let q = app.ui.value("Audio quality");
                        command(
                            app,
                            Command::Api("DZSetQuality".into(), vec![serde_json::json!(q)]),
                        );
                    }
                    // Preserve the startup page and any navigation made while signing in.
                    app.ui.history.clear();
                    app.navigation.clear();
                    app.entity = None;
                    app.query.clear();
                    load_page(app);
                }
                if app.lyrics_id != app.snapshot.now.id && !app.snapshot.now.id.is_empty() {
                    app.ui.lyrics_manual_scroll = false;
                    if app.ui.view == View::Lyrics {
                        settle_scroll(app, 0);
                    }
                    app.lyrics_id = app.snapshot.now.id.clone();
                    app.ui.lyrics.clear();
                    app.ui.rich_lyrics.clear();
                    app.ui.lyrics_provider.clear();
                    if app.snapshot.now.kind == "track" {
                        let neighbors =
                            adjacent_lyric_tracks(&app.snapshot.queue, app.snapshot.queue_index);
                        data_request(
                            app,
                            crate::catalog::Job::LyricsWindow(
                                app.snapshot.now.clone(),
                                neighbors,
                                app.ui.lyrics_sources & crate::lyrics::SUPPORTED_SOURCES,
                            ),
                        );
                    }
                }
                if app.pending.is_none() && app.notice == "Loading track..." {
                    app.notice.clear();
                }
                if app.ui.view == View::Lyrics
                    && (app.ui.lyrics_visualizer
                        || (app.ui.karaoke_lyrics
                            && app
                                .ui
                                .rich_lyrics
                                .iter()
                                .any(|line| !line.syllables.is_empty())))
                    && app.playing()
                {
                    start_animation_timer(hwnd, 2);
                }
                if changed || completed {
                    rebuild(hwnd, app);
                } else {
                    status_text(app);
                    if progress_changed {
                        InvalidateRect(hwnd, null(), 0);
                    }
                }
                if let Some(&slider) = app.edits.get(&SEEK) {
                    if GetCapture() != slider {
                        let p = app
                            .ui
                            .position
                            .saturating_mul(SEEK_STEPS as u64)
                            .checked_div(app.ui.duration)
                            .unwrap_or(0);
                        SendMessageW(slider, TBM_SETPOS, 1, p.min(SEEK_STEPS as u64) as isize);
                    }
                }
            }
            process_data(hwnd, app);
            0
        }
        crate::media_controls::MESSAGE => {
            use crate::media_controls as media;
            match wp {
                media::PLAY => {
                    command(app, Command::SetPlaying(true));
                }
                media::PAUSE => {
                    command(app, Command::SetPlaying(false));
                }
                media::NEXT => {
                    command(app, Command::Next);
                }
                media::PREVIOUS => {
                    command(app, Command::Previous);
                }
                media::STOP => {
                    command(app, Command::Stop);
                }
                media::SEEK if app.snapshot.duration > 0 => {
                    command(
                        app,
                        Command::SeekMs((lp.max(0) as u64).min(app.snapshot.duration)),
                    );
                }
                _ => {}
            }
            0
        }
        WM_HOTKEY => {
            if let Some(action) = app
                .shortcuts
                .as_ref()
                .and_then(|shortcuts| shortcuts.action(wp))
            {
                shortcut_action(hwnd, app, action);
            }
            0
        }
        WM_KEYDOWN => {
            if let Some(current) = app
                .canvas_focus
                .filter(|&index| canvas_track_button(app, index))
            {
                if wp == 13 || wp == 32 {
                    activate_scene_button(hwnd, app, current);
                    return 0;
                }
                if [VK_UP, VK_DOWN, VK_LEFT, VK_RIGHT].contains(&(wp as u16)) {
                    let origin = app.scene.buttons[current].rect;
                    let horizontal = wp == VK_LEFT as usize || wp == VK_RIGHT as usize;
                    let center = if horizontal {
                        origin.y + origin.h / 2
                    } else {
                        origin.x + origin.w / 2
                    };
                    let direction = if wp == VK_UP as usize || wp == VK_LEFT as usize {
                        -1
                    } else {
                        1
                    };
                    let queue_focus =
                        matches!(app.scene.buttons[current].action, Action::QueueJump(_));
                    let next = (0..app.scene.buttons.len())
                        .filter(|&index| {
                            let node = &app.scene.buttons[index];
                            let (start, end, delta) = if horizontal {
                                (
                                    node.rect.y,
                                    node.rect.y + node.rect.h,
                                    node.rect.x - origin.x,
                                )
                            } else {
                                (
                                    node.rect.x,
                                    node.rect.x + node.rect.w,
                                    node.rect.y - origin.y,
                                )
                            };
                            canvas_track_button(app, index)
                                && scene_button_enabled(app, index)
                                && matches!(node.action, Action::QueueJump(_)) == queue_focus
                                && center >= start
                                && center < end
                                && delta * direction > 0
                        })
                        .min_by_key(|&index| {
                            let rect = app.scene.buttons[index].rect;
                            if horizontal {
                                (rect.x - origin.x).abs()
                            } else {
                                (rect.y - origin.y).abs()
                            }
                        });
                    if let Some(next) = next {
                        let node = &app.scene.buttons[next];
                        let key = canvas_action_key(&node.action);
                        let (w, h) = size(hwnd, app);
                        let viewport = button_viewport(app, node, w, h);
                        let adjustment = if node.rect.y < viewport.y {
                            node.rect.y - viewport.y
                        } else if node.rect.y + node.rect.h > viewport.y + viewport.h {
                            node.rect.y + node.rect.h - viewport.y - viewport.h
                        } else {
                            0
                        };
                        if adjustment != 0 && !queue_focus {
                            let bottom = content_bottom(app, h);
                            let target = (app.ui.scroll + adjustment)
                                .clamp(0, (app.scene.content_height - bottom).max(0));
                            scroll_to(hwnd, app, target);
                            settle_scroll(app, target);
                        }
                        app.canvas_focus = (0..app.scene.buttons.len()).find(|&index| {
                            canvas_track_button(app, index)
                                && canvas_action_matches(&key, &app.scene.buttons[index].action)
                        });
                        InvalidateRect(hwnd, null(), 0);
                    }
                    return 0;
                }
            }
            if wp == 27 && app.ui.custom_theme_draft.is_some() {
                app.ui.custom_theme_draft = None;
                rebuild(hwnd, app);
                return 0;
            }
            if wp == 27
                && app.ui.device_menu_open
                && app.ui.provider != crate::catalog::Provider::Spotify
            {
                app.ui.device_menu_open = false;
                rebuild(hwnd, app);
                return 0;
            }
            if wp == 27 && app.ui.deezer_login_open {
                if app.ui.deezer_login_advanced {
                    app.ui.deezer_login_advanced = false;
                } else {
                    if let Some((cancel, _)) = app.deezer_auth.take() {
                        cancel.store(true, std::sync::atomic::Ordering::Relaxed);
                    }
                    app.ui.deezer_login_open = false;
                    app.ui.deezer_login_busy = false;
                }
                rebuild(hwnd, app);
            } else if wp == 27 && app.ui.lyrics_volume_open {
                app.ui.lyrics_volume_open = false;
                rebuild(hwnd, app);
            } else if wp == 27 {
                activate(hwnd, app, Action::Back);
            } else if wp == 32 {
                command(app, Command::Toggle);
            }
            0
        }
        WM_DESTROY => {
            gpu::hide();
            stop_animation_timer(hwnd, 2);
            stop_animation_timer(hwnd, 4);
            stop_animation_timer(hwnd, 8);
            stop_animation_timer(hwnd, 9);
            if let Some((cancel, _)) = app.spotify_auth.take() {
                cancel.store(true, std::sync::atomic::Ordering::Relaxed);
            }
            app.spotify_service.take();
            if let Some((cancel, _)) = app.spotify_setup.take() {
                cancel.store(true, std::sync::atomic::Ordering::Relaxed);
            }
            app.media_controls.take();
            app.shortcuts.take();
            let show_tray = app.ui.show_in_tray;
            app.ui.show_in_tray = false;
            sync_tray(hwnd, app);
            app.ui.show_in_tray = show_tray;

            KillTimer(hwnd, 1);
            KillTimer(hwnd, 6);
            if let Some((_, _, f_act, f_inact, f_plain)) = app.lyrics_fonts {
                DeleteObject(f_act);
                DeleteObject(f_inact);
                DeleteObject(f_plain);
            }
            app.ui.save_preferences();
            command(app, Command::Quit);
            PostQuitMessage(0);
            0
        }
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}
unsafe fn make_fonts(scale: f64) -> [HFONT; 12] {
    let mut fonts = [null_mut(); 12];
    for (i, (height, weight, face)) in [
        (-13.333, 400, "Segoe UI"),
        (-13.333, 400, "Segoe UI"),
        (-33.333, 400, "Segoe UI"),
        (-20.0, 700, "Segoe UI"),
        (-16.0, 400, "Segoe MDL2 Assets"),
        (-29.333, 700, "Segoe UI"),
        (-9.333, 400, "Segoe UI"),
        (-10.666, 400, "Segoe UI"),
        (-14.666, 400, "Segoe UI"),
        (-12.0, 400, "Segoe UI"),
        (-19.333, 400, "Segoe UI"),
        (-15.333, 400, "Segoe UI"),
    ]
    .into_iter()
    .enumerate()
    {
        fonts[i] = CreateFontW(
            (height * scale).round() as i32,
            0,
            0,
            0,
            weight,
            0,
            0,
            0,
            DEFAULT_CHARSET as u32,
            0,
            0,
            ANTIALIASED_QUALITY as u32,
            0,
            wide(face).as_ptr(),
        );
    }

    fonts
}
fn startup_provider(
    last: Option<crate::catalog::Provider>,
    deezer_saved: bool,
    spotify_saved: bool,
    youtube_saved: bool,
) -> crate::catalog::Provider {
    use crate::catalog::Provider;
    if let Some(provider) = last {
        let available = match provider {
            Provider::Deezer => deezer_saved,
            Provider::Spotify => spotify_saved,
            Provider::YouTube => youtube_saved,
        };
        if available {
            return provider;
        }
    }
    if deezer_saved {
        Provider::Deezer
    } else if spotify_saved {
        Provider::Spotify
    } else if youtube_saved {
        Provider::YouTube
    } else {
        Provider::Deezer
    }
}

#[cfg(test)]
#[test]
fn startup_provider_prefers_available_last_selection() {
    use crate::catalog::Provider;
    assert_eq!(startup_provider(Some(Provider::YouTube), true, true, true), Provider::YouTube);
    assert_eq!(startup_provider(Some(Provider::Spotify), true, true, false), Provider::Spotify);
    assert_eq!(startup_provider(Some(Provider::YouTube), true, true, false), Provider::Deezer);
    assert_eq!(startup_provider(None, false, false, true), Provider::YouTube);
}

pub fn run() -> Result<(), String> {
    let _ = std::thread::Builder::new()
        .name("soloist-update".into())
        .spawn(|| {
            let _ = crate::spotify::setup::background_update();
        });
    unsafe {
        SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        let controls = INITCOMMONCONTROLSEX {
            dwSize: size_of::<INITCOMMONCONTROLSEX>() as u32,
            dwICC: ICC_BAR_CLASSES,
        };
        InitCommonControlsEx(&controls);
        let instance = GetModuleHandleW(null());
        let class = wide("DeezNatsRustWindow");
        let wc = WNDCLASSW {
            hIcon: LoadIconW(instance, resource(1)),
            lpfnWndProc: Some(procedure),
            hInstance: instance,
            lpszClassName: class.as_ptr(),
            hCursor: LoadCursorW(null_mut(), IDC_ARROW),
            ..zeroed()
        };
        if RegisterClassW(&wc) == 0 {
            return Err("Could not register window class".into());
        }
        let scale = GetDpiForSystem() as f64 / 96.0;
        let fonts = make_fonts(scale);
        let storage_error = crate::storage::initialize().err();
        if let Some(error) = &storage_error {
            MessageBoxW(
                null_mut(),
                wide(error).as_ptr(),
                wide("DeezNats").as_ptr(),
                MB_OK | MB_ICONERROR,
            );
            if !crate::storage::data_dir().is_ok_and(|path| path.is_dir()) {
                return Err(error.clone());
            }
        }
        let mut preferences = Ui::load_preferences();
        preferences.onboarding_open =
            !crate::session::deezer_saved_exists() && !crate::spotify::api_configured()
                && !crate::session::provider_exists("youtube");
        if let Some(message) = crate::updater::startup_result() {
            preferences.update.error = message;
        }
        let initial_volume = preferences.initial_volume;
        let worker = Worker::start();
        let _ = worker
            .commands
            .send(Command::Autoplay(preferences.autoplay));
        let connect = if preferences.jam_enabled {
            crate::connect::ConnectHub::start(crate::connect::DEFAULT_PORT, worker.commands.clone())
        } else {
            None
        };
        preferences.jam_enabled = connect.is_some();
        preferences.jam_url = connect
            .as_ref()
            .map(|hub| hub.share_url())
            .unwrap_or_default();
        let mut app = Box::new(App {
            queue_drag: None,
            queue_scroll_drag: false,
            queue_wheel_remainder: 0.0,
            queue_glide: None,
            middle_scroll: None,
            caption_buttons: [null_mut(); 3],
            collapse_button: null_mut(),
            tray_added: false,
            search_backdrop: Default::default(),
            worker,
            updater: crate::updater::Worker::start(),
            media_controls: None,
            shortcuts: None,
            connect,
            snapshot: Snapshot::default(),
            playback_clock: PlaybackClock::default(),
            karaoke_preview_started: std::time::Instant::now(),
            lyric_motion: None,
            lyrics_mode_motion: None,
            lyric_motion_offset: 0,
            lyric_motion_track: String::new(),
            ui: preferences,
            scene: Scene::default(),
            buttons: Vec::new(),
            button_text: Vec::new(),
            canvas_pressed: None,
            canvas_hover: None,
            canvas_focus: None,
            canvas_keyboard: null_mut(),
            edits: HashMap::new(),
            fonts,
            lyrics_fonts: None,
            brush: CreateSolidBrush(rgb(29, 29, 29)),
            scale,
            acrylic: false,
            pending: None,
            notice: storage_error
                .unwrap_or_else(|| "Open your profile to connect to Deezer.".into()),
            volume: initial_volume,
            scrolling: false,
            render_suspended: false,
            layout_dirty: false,
            scroll_grab_offset: 0,
            catalog: crate::catalog::Worker::start(),

            spotify_service: None,
            spotify_opened: false,
            spotify_setup: None,
            spotify_auth: None,
            spotify_device_request: None,
            spotify_jam_request: None,
            deezer_auth: None,

            artwork: crate::catalog::Artwork::start(),
            images: HashMap::new(),
            image_order: VecDeque::new(),
            image_pending: HashSet::new(),
            image_retry_at: HashMap::new(),
            generation: 0,
            entity: None,
            query: String::new(),
            navigation: Vec::new(),
            lyrics_id: String::new(),
            background: None,
            background_source: String::new(),
            background_requested: false,
            backdrop_worker: crate::appearance::Worker::start(),
            previous_background: None,
            background_transition: BackgroundTransition::Complete,
            previous_cover: None,
            cover_source: String::new(),
            cover_changed: None,
            custom_background: String::new(),
            image_times: HashMap::new(),
            scroll_target: 0,
            scroll_origin: 0,
            scroll_position: 0.0,
            scroll_velocity: 0.0,
            scroll_tick: std::time::Instant::now(),
            wheel_remainder: 0.0,
            layout_scroll: 0,
            artwork_prefetch_scroll: 0,
            artwork_prefetch_upward: false,
            scroll_started: std::time::Instant::now(),
            backdrop_cache: Default::default(),
            palette_state: Default::default(),
            cover_luminance_cache: Default::default(),
            downloads: crate::downloads::Worker::start(),
            download_cancels: HashMap::new(),
            flow_save_expected: None,
            flow_verify_attempt: 0,
            search_hint: String::new(),
            page_cache: VecDeque::new(),
            toggle_animations: HashMap::new(),
            eq_drag: None,
            visualizer: crate::visualizer::Visualizer::new(),
            podcast_ids: HashSet::new(),
            library_pending: None,
            playlist_generation: None,
            discord_seen: serde_json::Value::Null,
            library_loaded: false,
            lyrics_hover_start: std::time::Instant::now(),
            lyrics_hover_from: 0.0,
            lyrics_hover_to: 0.0,
        });
        if let Ok(bytes) = std::fs::read(&app.ui.background_file) {
            if let Ok(image) = crate::catalog::decode_background(&bytes, 768) {
                let key = format!("local:{}", app.ui.background_file);
                app.images.insert(key.clone(), Arc::new(image));
                app.custom_background = key;
            }
        }
        set_preferred_dark_mode(!app.ui.light);
        let hwnd = CreateWindowExW(
            0,
            class.as_ptr(),
            wide("DeezNats").as_ptr(),
            WS_OVERLAPPEDWINDOW,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            1320,
            880,
            null_mut(),
            null_mut(),
            instance,
            (&mut *app as *mut App).cast::<c_void>(),
        );
        if hwnd.is_null() {
            return Err("Could not create native window".into());
        }
        // Recompute on every launch, using this window's monitor and DPI.
        // rcWork excludes the taskbar; leave a little room around the window.
        let mut monitor: MONITORINFO = zeroed();
        monitor.cbSize = size_of::<MONITORINFO>() as u32;
        if GetMonitorInfoW(
            MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST),
            &mut monitor,
        ) != 0
        {
            let work = monitor.rcWork;
            let available_width = (work.right - work.left).max(1);
            let available_height = (work.bottom - work.top).max(1);
            let dpi = GetDpiForWindow(hwnd).max(96) as f64 / 96.0;
            let width = ((1320.0 * dpi).round() as i32).min(available_width * 95 / 100);
            let height = ((880.0 * dpi).round() as i32).min(available_height * 95 / 100);
            SetWindowPos(
                hwnd,
                null_mut(),
                work.left + (available_width - width) / 2,
                work.top + (available_height - height) / 2,
                width.max(1),
                height.max(1),
                SWP_NOZORDER | SWP_NOACTIVATE,
            );
        }
        ShowWindow(hwnd, SW_SHOW);
        let mut message = zeroed();
        loop {
            let result = next_ui_message(&mut message);
            if result < 0 {
                return Err("Window message loop failed".into());
            }
            if result == 0 {
                break;
            }
            if app.ui.shortcut_capture.is_some()
                && matches!(message.message, WM_KEYDOWN | WM_SYSKEYDOWN)
            {
                if message.lParam & (1 << 30) == 0 {
                    capture_shortcut(hwnd, &mut app, message.wParam as u32);
                }
                continue;
            }
            if message.message == WM_KEYDOWN && message.wParam == 13 {
                let focused = windows_sys::Win32::UI::Input::KeyboardAndMouse::GetFocus();
                let id = GetDlgCtrlID(focused);

                if id == SESSION {
                    if app.ui.provider == crate::catalog::Provider::Spotify {
                        activate(hwnd, &mut app, Action::SpotifyLogin);
                    } else {
                        login(hwnd, &mut app);
                    }
                    continue;
                }

                if id == SEARCH && !canvas_input::composing(focused) {
                    if app.ui.view != View::FlowTune {
                        activate(hwnd, &mut app, Action::Search);
                    }
                    continue;
                }
                if id == TRACK {
                    activate(hwnd, &mut app, Action::PlayId);
                    continue;
                }
            }
            if IsDialogMessageW(hwnd, &message) == 0 {
                TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        }
        for font in app.fonts {
            DeleteObject(font);
        }
        DeleteObject(app.brush);
        BufferedPaintUnInit();
        Ok(())
    }
}

unsafe fn next_ui_message(message: &mut MSG) -> i32 {
    unsafe { frame_pacing::next_message(message) }
}

#[cfg(test)]
mod behavior_tests {
    use super::*;
    unsafe fn test_app() -> App {
        let scale = 1.0;
        let fonts = make_fonts(scale);
        App {
            queue_drag: None,
            queue_scroll_drag: false,
            queue_wheel_remainder: 0.0,
            queue_glide: None,
            middle_scroll: None,
            caption_buttons: [null_mut(); 3],
            collapse_button: null_mut(),
            tray_added: false,
            search_backdrop: Default::default(),
            worker: Worker::start(),
            updater: crate::updater::Worker::start(),
            media_controls: None,
            shortcuts: None,
            connect: None,
            snapshot: Snapshot::default(),
            playback_clock: PlaybackClock::default(),
            karaoke_preview_started: std::time::Instant::now(),
            lyric_motion: None,
            lyrics_mode_motion: None,
            lyric_motion_offset: 0,
            lyric_motion_track: String::new(),
            ui: Ui::default(),
            scene: Scene::default(),
            buttons: Vec::new(),
            button_text: Vec::new(),
            canvas_pressed: None,
            canvas_hover: None,
            canvas_focus: None,
            canvas_keyboard: null_mut(),
            edits: HashMap::new(),
            fonts,
            lyrics_fonts: None,
            brush: CreateSolidBrush(rgb(29, 29, 29)),
            scale,
            acrylic: false,
            pending: None,
            notice: String::new(),
            volume: 70,
            scrolling: false,
            render_suspended: false,
            layout_dirty: false,
            scroll_grab_offset: 0,
            catalog: crate::catalog::Worker::start(),
            spotify_service: None,
            spotify_opened: false,
            spotify_setup: None,
            spotify_auth: None,
            spotify_device_request: None,
            spotify_jam_request: None,
            deezer_auth: None,
            artwork: crate::catalog::Artwork::start(),
            images: HashMap::new(),
            image_order: VecDeque::new(),
            image_pending: HashSet::new(),
            image_retry_at: HashMap::new(),
            generation: 0,
            entity: None,
            query: String::new(),
            navigation: Vec::new(),
            lyrics_id: String::new(),
            background: None,
            background_source: String::new(),
            background_requested: false,
            backdrop_worker: crate::appearance::Worker::start(),
            previous_background: None,
            background_transition: BackgroundTransition::Complete,
            previous_cover: None,
            cover_source: String::new(),
            cover_changed: None,
            custom_background: String::new(),
            image_times: HashMap::new(),
            scroll_target: 0,
            scroll_origin: 0,
            scroll_position: 0.0,
            scroll_velocity: 0.0,
            scroll_tick: std::time::Instant::now(),
            wheel_remainder: 0.0,
            layout_scroll: 0,
            artwork_prefetch_scroll: 0,
            artwork_prefetch_upward: false,
            scroll_started: std::time::Instant::now(),
            backdrop_cache: Default::default(),
            palette_state: Default::default(),
            cover_luminance_cache: Default::default(),
            downloads: crate::downloads::Worker::start(),
            download_cancels: HashMap::new(),
            flow_save_expected: None,
            flow_verify_attempt: 0,
            search_hint: String::new(),
            page_cache: VecDeque::new(),
            toggle_animations: HashMap::new(),
            eq_drag: None,
            visualizer: crate::visualizer::Visualizer::new(),
            podcast_ids: HashSet::new(),
            library_pending: None,
            playlist_generation: None,
            discord_seen: serde_json::Value::Null,
            library_loaded: false,
            lyrics_hover_start: std::time::Instant::now(),
            lyrics_hover_from: 0.0,
            lyrics_hover_to: 0.0,
        }
    }

    #[test]
    fn canvas_input_preserves_playlist_cover_drafts_and_filters_flow_live() {
        unsafe {
            let hwnd = CreateWindowExW(0, wide("STATIC").as_ptr(), wide("").as_ptr(),
                WS_OVERLAPPEDWINDOW, 0, 0, 1200, 1100, null_mut(), null_mut(),
                GetModuleHandleW(null()), null());
            assert!(!hwnd.is_null());
            let mut app = test_app();
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, &mut app as *mut App as isize);
            for id in [SEARCH, NAME, DESCRIPTION] {
                let control = make_control(hwnd, "EDIT", id, 0, &app);
                app.edits.insert(id, control);
            }
            let status = make_control(hwnd, "STATIC", STATUS, 0, &app);
            app.edits.insert(STATUS, status);
            app.ui.view = View::PlaylistEdit;
            app.ui.playlist_draft.title = "Original".into();
            set_playlist_fields(&app);
            SetWindowTextW(app.edits[&NAME], wide("Edited title").as_ptr());
            SetWindowTextW(app.edits[&DESCRIPTION], wide("First\nSecond").as_ptr());
            activate(hwnd, &mut app, Action::Go(View::CoverEdit));
            assert_eq!(app.ui.playlist_draft.title, "Edited title");
            assert_eq!(app.ui.playlist_draft.description, "First\nSecond");
            assert!(app.ui.playlist_draft.description_changed);
            activate(hwnd, &mut app, Action::Back);
            assert_eq!(app.ui.view, View::PlaylistEdit);
            assert_eq!(read(app.edits[&NAME]), "Edited title");
            assert_eq!(read(app.edits[&DESCRIPTION]), "First\nSecond");
            app.ui.view = View::FlowTune;
            app.ui.flow_genre_limit = 100;
            app.ui.scroll = 80;
            SetWindowTextW(app.edits[&SEARCH], wide("ambient").as_ptr());
            procedure(hwnd, WM_COMMAND, SEARCH as usize | ((EN_CHANGE as usize) << 16), app.edits[&SEARCH] as isize);
            assert_eq!(app.ui.flow_filter, "ambient");
            assert_eq!(app.ui.flow_genre_limit, 30);
            assert_eq!(app.ui.scroll, 0);
            // Regression: checking a child WM_PRINTCLIENT is insufficient. Its
            // pixels can disappear under DirectComposition. The root frame
            // itself must contain the current editor text.
            app.ui.view = View::PlaylistEdit;
            rebuild(hwnd, &mut app);
            BufferedPaintInit();
            let dc = CreateCompatibleDC(null_mut());
            let mut bits = null_mut();
            let info = BITMAPINFO { bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32, biWidth: 1200,
                biHeight: -1100, biPlanes: 1, biBitCount: 32, biCompression: BI_RGB,
                ..zeroed() }, ..zeroed() };
            let bitmap = CreateDIBSection(dc, &info, DIB_RGB_COLORS, &mut bits, null_mut(), 0);
            assert!(!bitmap.is_null());
            let old = SelectObject(dc, bitmap);
            let field = app.rect(app.scene.name.unwrap());
            let capture = || {
                GdiFlush();
                let pixels = std::slice::from_raw_parts(bits.cast::<u32>(), 1200*1100);
                let caption = caption_height(hwnd);
                (field.top+caption..field.bottom+caption).flat_map(|y|
                    pixels[y as usize*1200+field.left as usize..y as usize*1200+field.right as usize].iter().copied()
                ).collect::<Vec<_>>()
            };
            SetWindowTextW(app.edits[&NAME], wide("iiiiiiiiiiii").as_ptr());
            render(hwnd, &app, dc, None);
            let first = capture();
            SetWindowTextW(app.edits[&NAME], wide("WWWWWWWWWWWW").as_ptr());
            render(hwnd, &app, dc, None);
            assert_ne!(first, capture(), "root canvas omitted editor glyphs");
            SelectObject(dc, old); DeleteObject(bitmap); DeleteDC(dc);
            BufferedPaintUnInit();
            // Programmatic field loading, editing, and cover navigation never replace the canvas windows.
            assert!(GetWindow(app.edits[&NAME], GW_CHILD).is_null());
            DestroyWindow(hwnd);
            for font in app.fonts { DeleteObject(font); }
            DeleteObject(app.brush);
        }
    }
    #[test]
    fn queue_rows_use_canvas_hit_testing_outside_main_content() {
        unsafe {
            let mut app = test_app();
            app.scene.buttons = vec![ui::Button {
                rect: Rect::new(900, 150, 200, 66),
                action: Action::QueueJump(2),
                enabled: true,
                ..Default::default()
            }];
            app.scene.content_buttons = 0..0;
            assert!(canvas_track_button(&app, 0));
            assert_eq!(canvas_track_hit(&app, 950, 180, 1200, 800), Some(0));
            assert_eq!(canvas_track_hit(&app, 850, 180, 1200, 800), None);
        }
    }
    #[test]
    fn queue_wheel_retargeting_keeps_elapsed_time_and_reverses_pending_motion() {
        let start=std::time::Instant::now();
        let mut state=None;let mut remainder=0.;
        for i in 0..8 {
            let pixels=wheel_pixels(-15,3,600,&mut remainder);
            assert!(pixels<0);
            state=Some(retarget_queue_glide(state,100.,pixels,1000.,start+std::time::Duration::from_millis(i)));
        }
        let (position,velocity,target,tick)=state.unwrap();
        assert_eq!(tick,start,"high-resolution input must not restart the frame clock");
        assert!(target>100.);
        let (next,speed)=smooth_scroll_step(position,velocity,target,0.016);
        assert!(next>position&&next<target&&speed>0.);
        let reversed=retarget_queue_glide(Some((next,speed,target,tick)),next,9,1000.,start+std::time::Duration::from_millis(17));
        assert!(reversed.2<next&&reversed.1==0.);
    }

    #[test]
    #[ignore = "Requires Windows hardware Direct3D and DirectComposition"]
    fn gpu_pages_render_both_providers_with_acrylic_and_opaque_backgrounds() {
        unsafe {
            std::env::set_var("DEEZNATS_TEST_GPU_RENDER","1");
            let hwnd=CreateWindowExW(0,wide("STATIC").as_ptr(),null(),WS_POPUP,0,0,1000,800,
                null_mut(),null_mut(),null_mut(),null());
            assert!(!hwnd.is_null());
            BufferedPaintInit();
            let dc=GetDC(hwnd);
            let mut app=test_app();
            app.snapshot.connected=true;
            app.images.insert("fixture-cover".into(),Arc::new(crate::catalog::Bitmap::new(16,16,vec![0xff665544;256])));
            app.image_times.insert("fixture-cover".into(),std::time::Instant::now());
            let page=Arc::new(crate::catalog::Page {
                rows:(0..100).map(|i|crate::catalog::Entry { id:format!("track-{i}"),title:format!("Track {i}"),
                    subtitle:"Fixture artist".into(),kind:"track".into(),..Default::default() }).collect(),
                sections:(0..4).map(|section|crate::catalog::Section {
                    title:format!("Shelf {section}"),entries:(0..12).map(|i|crate::catalog::Entry {
                        id:format!("playlist-{section}-{i}"),title:format!("A longer playlist title on shelf {section}, item {i}"),
                        subtitle:"Fixture artist".into(),kind:"playlist".into(),artwork:"fixture-cover".into(),..Default::default()
                    }).collect(),..Default::default()
                }).collect(),
                ..Default::default()
            });
            app.ui.queue=page.rows.iter().cloned().map(|mut row|{row.artwork="fixture-cover".into();row}).collect();
            let mut home=(*page).clone();home.rows.clear();let home=Arc::new(home);
            for provider in [crate::catalog::Provider::Deezer,crate::catalog::Provider::Spotify] {
                app.ui.provider=provider;
                app.snapshot.provider=provider;
                for acrylic in [false,true] {
                    app.acrylic=acrylic;
                    for view in [View::Favorites,View::Home,View::Settings,View::Lyrics] {
                        app.ui.view=view; app.ui.scroll=0; app.ui.page=Some(if view==View::Home{home.clone()}else{page.clone()});
                        app.scroll_target=180;app.scroll_position=0.;
                        app.ui.queue_sidebar_open=view==View::Favorites;
                        app.ui.queue_scroll=0;app.ui.queue_scroll_offset=0;
                        reset_marquees();
                        app.scene=ui::build(&app.ui,(1000,800),true,false,"",false,70);
                        app.layout_scroll=0;
                        expand_media_text(&mut app.scene,984);fit_text_links(hwnd,&mut app.scene,&app.fonts,app.scale);
                        render(hwnd,&app,dc,None);
                        if view==View::Home {
                            let art=app.scene.buttons.iter().find(|b|b.artwork=="fixture-cover"&&b.rect.y>=app.scene.content_top&&b.rect.y+50<600).unwrap();
                            assert_eq!(gpu::snapshot(RECT{left:art.rect.x+30,top:art.rect.y+30,right:art.rect.x+31,bottom:art.rect.y+31}).unwrap()[0],0xff665544,"new catalog artwork must be opaque before entering a retained scroll layer");
                        }
                        if view==View::Favorites {
                            let frame=app.scene.queue_frame.unwrap();
                            let rect=RECT{left:0,top:0,right:frame.x,bottom:600};
                            let main_before=gpu::snapshot(rect).unwrap();
                            let main_label=app.scene.content_labels.clone().next().map(|i|app.scene.labels[i].text.as_ptr());
                            set_queue_scroll(hwnd,&mut app,73);
                            assert_eq!(main_label,app.scene.content_labels.clone().next().map(|i|app.scene.labels[i].text.as_ptr()));
                            let mut dirty=app.rect(frame);let caption=caption_height(hwnd);dirty.top+=caption;dirty.bottom+=caption;
                            render(hwnd,&app,dc,Some(dirty));
                            assert_eq!(main_before,gpu::snapshot(rect).unwrap(),"queue scrolling repainted the main page");
                        }
                        let snapshot=gpu::snapshot(RECT{left:0,top:0,right:900,bottom:600});
                        assert!(snapshot.as_ref().is_some_and(|p|p.iter().any(|p|*p!=0)),
                            "GPU render fell back for {provider:?}, {view:?}, Acrylic={acrylic}");
                        if matches!(view,View::Favorites|View::Home) {
                            let caption=caption_height(hwnd);
                            let region=CreateRectRgn(420,300+caption,800,350+caption);
                            let second=CreateRectRgn(420,480+caption,800,520+caption);
                            CombineRgn(region,region,second,RGN_OR);
                            reset_marquees();
                            render_region(hwnd,&app,dc,Some(RECT{left:420,top:300+caption,right:800,bottom:520+caption}),region);
                            DeleteObject(region); DeleteObject(second);
                            let after=gpu::snapshot(RECT{left:0,top:0,right:900,bottom:600}).unwrap();
                            let before=snapshot.as_ref().unwrap();
                            let differences:Vec<_>=before.iter().zip(&after).enumerate().filter(|(_, (a,b))|a!=b).map(|(i,_)|(i%900,i/900)).collect();
                            let outside=differences.iter().filter(|&&(x,y)|x<420||x>=800||!(300..350).contains(&y)&&!(480..520).contains(&y)).count();
                            assert_eq!(outside,0,"disjoint repaint erased retained pixels between update rectangles");
                            let max_delta=differences.iter().flat_map(|&(x,y)|[0,8,16,24].map(|shift|((before[y*900+x]>>shift)&255).abs_diff((after[y*900+x]>>shift)&255))).max().unwrap_or(0);
                            let worst=differences.iter().max_by_key(|&&(x,y)|[0,8,16,24].map(|shift|((before[y*900+x]>>shift)&255).abs_diff((after[y*900+x]>>shift)&255)).into_iter().max().unwrap()).map(|&(x,y)|(x,y,before[y*900+x],after[y*900+x]));
                            assert!(max_delta<=1,"partial repaint changed static content by {max_delta} levels at {worst:?}, {provider:?} acrylic={acrylic}");
                            // A resize can replace the retained surface during
                            // a partial paint. It must reconstruct the whole
                            // frame instead of dropping untouched content.
                            SetWindowPos(hwnd,null_mut(),0,0,1010,810,SWP_NOMOVE|SWP_NOZORDER|SWP_NOACTIVATE);
                            render(hwnd,&app,dc,Some(RECT{left:420,top:300+caption,right:800,bottom:520+caption}));
                            assert!(gpu::snapshot(RECT{left:1000,top:0,right:1010,bottom:600}).is_some(),"resize must rebuild the complete GPU surface");
                            SetWindowPos(hwnd,null_mut(),0,0,1000,800,SWP_NOMOVE|SWP_NOZORDER|SWP_NOACTIVATE);
                            render(hwnd,&app,dc,None);
                            app.ui.scroll=64;app.scroll_position=64.; shift_scroll_content(&mut app.scene,-64);
                            let caption=caption_height(hwnd);
                            let rect=scroll_repaint_rect(&app,1000,800,caption);
                            render(hwnd,&app,dc,Some(rect));
                            assert!(gpu::snapshot(RECT{left:0,top:0,right:900,bottom:600}).is_some());
                            let check=RECT{left:300,top:260,right:650,bottom:560};
                            let replayed=gpu::snapshot(check).unwrap();
                            gpu::invalidate_layers();
                            render(hwnd,&app,dc,Some(rect));
                            let recaptured=gpu::snapshot(check).unwrap();
                            let delta=replayed.iter().zip(&recaptured).flat_map(|(a,b)|[0,8,16,24].map(move |shift|((a>>shift)&255).abs_diff((b>>shift)&255))).max().unwrap_or(0);
                            assert!(delta<=1,"scroll replay differs from fresh capture by {delta}, {provider:?}, {view:?}, acrylic={acrylic}");
                            for position in [360,720,300,0] {
                                app.scroll_position=position as f64;
                                scroll_to(hwnd,&mut app,position);
                                render(hwnd,&app,dc,None);
                                let retained=gpu::snapshot(check).unwrap();
                                let fresh=ui::build(&app.ui,(1000,800),true,false,"",false,70);
                                let previous=std::mem::replace(&mut app.scene,fresh);
                                expand_media_text(&mut app.scene,984);fit_text_links(hwnd,&mut app.scene,&app.fonts,app.scale);
                                gpu::invalidate_main_layer();render(hwnd,&app,dc,None);
                                let expected=gpu::snapshot(check).unwrap();
                                let delta=retained.iter().zip(&expected).flat_map(|(a,b)|[0,8,16,24].map(move |shift|((a>>shift)&255).abs_diff((b>>shift)&255))).max().unwrap_or(0);
                                assert!(delta<=1,"incremental catalog pixels differ from a fresh layout by {delta} at {position}, {provider:?}, {view:?}, acrylic={acrylic}");
                                app.scene=previous;gpu::invalidate_main_layer();
                            }
                        }
                    }
                }
            }
            gpu::hide(); ReleaseDC(hwnd,dc); DestroyWindow(hwnd); BufferedPaintUnInit();
            std::env::remove_var("DEEZNATS_TEST_GPU_RENDER");
        }
    }

    #[test]
    fn canvas_hit_tests_header_and_modal_buttons() {
        unsafe {
            let mut app = test_app();
            app.scene.buttons = vec![
                ui::Button {
                    rect: Rect::new(20, 20, 80, 30),
                    action: Action::Back,
                    enabled: true,
                    ..Default::default()
                },
                ui::Button {
                    rect: Rect::new(500, 200, 100, 30),
                    action: Action::Search,
                    enabled: true,
                    ..Default::default()
                },
            ];
            app.scene.content_buttons = 0..0;
            assert_eq!(canvas_track_hit(&app, 40, 30, 1200, 800), Some(0));
            app.scene.auth_modal = Some((Rect::new(450, 150, 300, 200), 0, 1));
            assert_eq!(canvas_track_hit(&app, 40, 30, 1200, 800), None);
            assert_eq!(canvas_track_hit(&app, 550, 210, 1200, 800), Some(1));
        }
    }

    #[test]
    fn lyrics_seek_regions_cannot_steal_floating_controls_or_bottom_strip() {
        unsafe {
            let mut app=test_app();
            app.ui.view=View::Lyrics;
            app.ui.lyrics_manual_scroll=true;
            app.ui.lyrics=vec![(0,"First lyric".into()),(1000,"Second lyric".into())];
            for (w,h) in [(746,490),(1200,800),(1600,1000)] {
                app.scene=ui::build(&app.ui,(w,h),true,false,"Track",true,70);
                for action in [Action::ToggleLyricsCoverMode,Action::ToggleQueueSidebar,Action::ToggleLyricsVisualizer] {
                    let control=app.scene.buttons.iter().position(|b|std::mem::discriminant(&b.action)==std::mem::discriminant(&action)).unwrap();
                    let rect=app.scene.buttons[control].rect;
                    let seek=app.scene.buttons.len();
                    app.scene.buttons.push(ui::Button{rect,action:Action::SeekMs(1000),enabled:true,..Default::default()});
                    app.scene.content_buttons.end=app.scene.buttons.len();
                    assert_eq!(canvas_track_hit(&app,rect.x+rect.w/2,rect.y+rect.h/2,w,h),Some(control),"floating lyrics control must win over overlapping seek region");
                    app.scene.buttons.remove(seek);app.scene.content_buttons.end=app.scene.buttons.len();
                }
                let area=app.scene.lyrics_area.unwrap();
                let seek=app.scene.buttons.len();
                app.scene.buttons.push(ui::Button{rect:Rect::new(area.x,h-100,area.w,100),action:Action::SeekMs(2000),enabled:true,..Default::default()});
                app.scene.content_buttons.end=app.scene.buttons.len();
                assert_eq!(canvas_track_hit(&app,area.x+10,h-80,w,h),Some(seek));
                assert_ne!(canvas_track_hit(&app,area.x+10,h-30,w,h),Some(seek),"lyric seek must stop at the bottom of the lyrics viewport");
            }
        }
    }

    #[test]
    fn canvas_action_identity_distinguishes_tracks_and_sort_actions() {
        use super::*;
        let track = Action::OpenItem(crate::catalog::Entry {
            kind: "track".into(),
            id: "spotify:track:123".into(),
            ..Default::default()
        });
        let other = Action::OpenItem(crate::catalog::Entry {
            kind: "track".into(),
            id: "spotify:track:456".into(),
            ..Default::default()
        });
        let key = canvas_action_key(&track);
        assert!(canvas_action_matches(&key, &track));
        assert!(!canvas_action_matches(&key, &other));
        assert!(!canvas_action_matches(
            &key,
            &Action::ToggleCheck("spotify:track:123".into())
        ));
        assert!(canvas_action_matches(
            &canvas_action_key(&Action::Sort(4)),
            &Action::Sort(4)
        ));
        assert!(!canvas_action_matches(
            &canvas_action_key(&Action::Sort(4)),
            &Action::Sort(5)
        ));
    }

    #[test]
    fn native_control_sync_preserves_visibility_enabled_and_accessible_text() {
        use super::*;
        unsafe {
            let instance = GetModuleHandleW(null());
            let parent = CreateWindowExW(
                0,
                wide("STATIC").as_ptr(),
                wide("").as_ptr(),
                WS_OVERLAPPEDWINDOW,
                0,
                0,
                100,
                100,
                null_mut(),
                null_mut(),
                instance,
                null(),
            );
            assert!(!parent.is_null());
            let button = CreateWindowExW(
                0,
                wide("BUTTON").as_ptr(),
                wide("").as_ptr(),
                WS_CHILD,
                0,
                0,
                20,
                20,
                parent,
                null_mut(),
                instance,
                null(),
            );
            assert!(!button.is_null());
            let mut cached = String::new();
            assert!(set_cached_control_text(button, &mut cached, "Play"));
            assert!(!set_cached_control_text(button, &mut cached, "Play"));
            assert!(set_cached_control_text(button, &mut cached, "Pause"));
            let mut text = [0u16; 16];
            let n = GetWindowTextW(button, text.as_mut_ptr(), text.len() as i32);
            assert_eq!(String::from_utf16_lossy(&text[..n as usize]), "Pause");
            set_control_visible(button, true);
            set_control_visible(button, true);
            assert_ne!(GetWindowLongW(button, GWL_STYLE) as u32 & WS_VISIBLE, 0);
            set_control_visible(button, false);
            assert_eq!(GetWindowLongW(button, GWL_STYLE) as u32 & WS_VISIBLE, 0);
            set_control_enabled(button, false);
            set_control_enabled(button, false);
            assert_ne!(GetWindowLongW(button, GWL_STYLE) as u32 & WS_DISABLED, 0);
            set_control_enabled(button, true);
            assert_eq!(GetWindowLongW(button, GWL_STYLE) as u32 & WS_DISABLED, 0);
            DestroyWindow(parent);
        }
    }

    #[test]
    fn lyric_clock_absorbs_sample_jitter_without_jumping_or_reversing() {
        use std::time::{Duration, Instant};
        let start = Instant::now();
        let mut clock = super::PlaybackClock::default();
        clock.sample(1000, start, false);
        let mut previous = 1000;
        for frame in 1..=100 {
            let now = start + Duration::from_millis(frame * 10);
            let before = clock.position(now, true);
            if frame % 10 == 0 {
                let jitter = if frame % 20 == 0 { -35 } else { 25 };
                clock.sample((1000 + frame as i64 * 10 + jitter) as u64, now, true);
                assert_eq!(clock.position(now, true), before, "sample must not jump");
            }
            let position = clock.position(now, true);
            assert!(position >= previous, "word animation must not reverse");
            assert!(position - previous <= 15, "correction must be gradual");
            previous = position;
        }
        assert!(previous.abs_diff(2000) < 50);
    }

    #[test]
    fn lyric_clock_crosses_sparse_updates_and_resets_for_discontinuities() {
        use std::time::{Duration, Instant};
        let start = Instant::now();
        let mut clock = super::PlaybackClock::default();
        clock.sample(1000, start, false);
        assert_eq!(
            clock.position(start + Duration::from_millis(1000), true),
            2000
        );
        // A real seek must not spend time easing across intervening words.
        clock.sample(9000, start + Duration::from_secs(1), true);
        assert_eq!(clock.position(start + Duration::from_secs(1), true), 9000);
        clock.sample(9050, start + Duration::from_millis(1050), false);
        assert_eq!(clock.position(start + Duration::from_secs(2), false), 9050);
        clock.sample(0, start + Duration::from_secs(2), false);
        assert_eq!(clock.position(start + Duration::from_secs(2), true), 0);
    }

    #[test]
    fn minimized_pages_keep_current_and_two_newest() {
        let mut pages: std::collections::VecDeque<_> = ["old", "current", "recent", "newest"]
            .into_iter()
            .map(|key| {
                (
                    key.to_owned(),
                    std::sync::Arc::new(crate::catalog::Page::default()),
                )
            })
            .collect();
        super::trim_inactive_pages(&mut pages, "current");
        assert_eq!(
            pages
                .iter()
                .map(|(key, _)| key.as_str())
                .collect::<Vec<_>>(),
            ["current", "recent", "newest"]
        );
    }
    #[test]
    fn minimized_artwork_preserves_protected_images_and_discards_cold_ones() {
        let mut images = std::collections::HashMap::new();
        let mut order = std::collections::VecDeque::new();
        for key in ["current", "cold", "newer"] {
            images.insert(
                key.to_owned(),
                std::sync::Arc::new(crate::catalog::Bitmap::new(1, 1, vec![0])),
            );
            order.push_back(key.to_owned());
        }
        let protected = std::collections::HashSet::from(["current".to_owned()]);
        let removed = super::trim_artwork_entries_to(&mut images, &mut order, &protected, 4);
        assert_eq!(removed.len(), 1);
        assert_eq!(removed[0].0, "cold");
        assert!(images.contains_key("current"));
        assert!(images.contains_key("newer"));
    }
    #[test]
    fn native_dpi_preserves_readable_size_and_narrow_layout() {
        for dpi in [96, 120, 144, 192, 288] {
            let scale = super::native_ui_scale(dpi);
            assert_eq!(scale, dpi as f64 / 96.0);
            // Resizing must change the logical viewport, not shrink its contents.
            for (width, height) in [(484, 1050), (800, 560), (1320, 800)] {
                let physical = (
                    (width as f64 * scale).round(),
                    (height as f64 * scale).round(),
                );
                let viewport = (
                    (physical.0 / scale).round() as i32,
                    (physical.1 / scale).round() as i32,
                );
                assert_eq!(viewport, (width, height));
                let mut ui = crate::ui::Ui::default();
                ui.go(crate::ui::View::Settings);
                ui.fit_sidebars(viewport.0);
                let scene = crate::ui::build(&ui, viewport, true, false, "Track", true, 50);
                for button in &scene.buttons {
                    assert!(
                        button.rect.x >= 0 && button.rect.x + button.rect.w <= viewport.0,
                        "Control exceeds native-DPI viewport at {dpi} DPI: {:?}",
                        button.rect
                    );
                }
            }
        }
    }

    #[test]
    fn artwork_trimming_keeps_protected_and_recent_covers() {
        let mut images = HashMap::new();
        let mut order = VecDeque::new();
        let mut protected = HashSet::new();
        for i in 0..300 {
            let key = format!("cover-{i}");
            let image = Arc::new(crate::catalog::Bitmap::new(192, 192, vec![0; 192 * 192]));
            images.insert(key.clone(), image);
            order.push_back(key.clone());
            if i < 20 {
                protected.insert(key);
            }
        }
        let removed = trim_artwork_entries(&mut images, &mut order, &protected);
        assert!(!removed.is_empty());
        assert!(protected.iter().all(|key| images.contains_key(key)));
        assert!(!images.contains_key("cover-20"));
        assert!(images.contains_key("cover-299"));
        let offscreen: usize = images
            .iter()
            .filter(|(key, _)| !protected.contains(*key))
            .map(|(_, image)| image.pixels.len() * 4)
            .sum();
        assert!(offscreen <= OFFSCREEN_ARTWORK_BYTES);
        assert_eq!(order.len(), images.len());
    }
    #[test]
    fn artwork_trimming_never_evicts_protected_images_over_budget() {
        let image = Arc::new(crate::catalog::Bitmap::new(512, 512, vec![0; 512 * 512]));
        let mut images = HashMap::new();
        let mut order = VecDeque::new();
        let mut protected = HashSet::new();
        for i in 0..70 {
            let key = format!("cover-{i}");
            images.insert(key.clone(), image.clone());
            order.push_back(key.clone());
            protected.insert(key);
        }
        assert!(trim_artwork_entries(&mut images, &mut order, &protected).is_empty());
        assert_eq!(images.len(), 70);
    }

    #[test]
    fn artwork_grace_defers_soft_trims_but_never_the_hard_budget(){
        let mut images=HashMap::new();let mut order=VecDeque::new();let mut recent=HashSet::new();
        for i in 0..50 {let image=Arc::new(crate::catalog::Bitmap::new(512,512,vec![0;512*512]));recent.insert(image.id);let key=i.to_string();images.insert(key.clone(),image);order.push_back(key);}
        assert!(trim_artwork_recent(&mut images,&mut order,&HashSet::new(),&recent,false).is_empty(),"recent covers need a grace period");
        assert!(trim_artwork_recent(&mut images,&mut order,&HashSet::new(),&HashSet::new(),true).is_empty(),"routine trimming must wait for scrolling to settle");
        for i in 50..70 {let image=Arc::new(crate::catalog::Bitmap::new(512,512,vec![0;512*512]));recent.insert(image.id);let key=i.to_string();images.insert(key.clone(),image);order.push_back(key);}
        assert!(!trim_artwork_recent(&mut images,&mut order,&HashSet::new(),&recent,true).is_empty());
        assert!(images.values().map(|i|i.pixels.len()*4).sum::<usize>()<=RESIDENT_ARTWORK_BYTES);
        trim_artwork_recent(&mut images,&mut order,&HashSet::new(),&HashSet::new(),false);
        assert!(images.values().map(|i|i.pixels.len()*4).sum::<usize>()<=OFFSCREEN_ARTWORK_BYTES);
    }
    #[test]
    fn scrollbar_drag_preserves_grab_position_on_very_long_lists() {
        let mut scene = Scene::default();
        scene.content_height = 10_000_000;
        scene.panels.push(ui::Panel {
            rect: Rect::new(100, 190, 9, 600),
            kind: 5,
        });
        scene.panels.push(ui::Panel {
            rect: Rect::new(100, 190, 9, 32),
            kind: 6,
        });
        let bottom = 790;
        assert_eq!(scrollbar_position(&scene, 206, 16, bottom), 0);
        assert_eq!(
            scrollbar_position(&scene, 774, 16, bottom),
            scene.content_height - bottom
        );
        let middle = scrollbar_position(&scene, 490, 16, bottom);
        assert!(middle > 4_000_000 && middle < 6_000_000);
    }
    #[test]
    fn timed_lyrics_render_full_words_and_real_blur_fixture() {
        use super::*;
        unsafe {
            let dc = CreateCompatibleDC(null_mut());
            let bitmap = CreateBitmap(840, 280, 1, 32, null());
            let previous = SelectObject(dc, bitmap);
            let font = CreateFontW(
                -36,
                0,
                0,
                0,
                700,
                0,
                0,
                0,
                DEFAULT_CHARSET as u32,
                0,
                0,
                ANTIALIASED_QUALITY as u32,
                0,
                wide("Segoe UI").as_ptr(),
            );
            let mut pixels = vec![0xff303030u32; 840 * 280];
            let frame =
                crate::drawing::Frame::begin(dc, pixels.as_mut_ptr().cast(), 840, 840, 280, false)
                    .unwrap();
            let rect = |top| RECT {
                left: 20,
                top,
                right: 820,
                bottom: top + 56,
            };
            crate::drawing::recessed_visualizer(dc, 840, 280, || {
                let points: Vec<_> = (0..85)
                    .map(|i| {
                        let x = i as f32 * 10.0;
                        (x, 155.0 + (x / 65.0).sin() * 50.0)
                    })
                    .collect();
                crate::drawing::wave_landscape(
                    dc, &points, 280.0, 0x8066aaff, 0x0066aaff, 0xb066aaff, 3.0, 0x6066aaff,
                );
            });
            assert!(crate::drawing::blurred_text(
                dc,
                font,
                "Can't change leopards spots",
                rect(10),
                0x00909090,
                DT_LEFT,
                1.8
            ));
            let words = ["Have ", "you ", "feeling ", "like ", "a ", "victim"];
            let line = crate::lyrics::LyricLine {
                text: words.concat(),
                start_ms: 0,
                end_ms: 6000,
                syllables: words
                    .iter()
                    .enumerate()
                    .map(|(i, word)| crate::lyrics::Syllable {
                        text: (*word).into(),
                        start_ms: i as u64 * 1000,
                        end_ms: (i + 1) as u64 * 1000,
                    })
                    .collect(),
                ..Default::default()
            };
            assert!(draw_karaoke_row(
                dc,
                font,
                &line,
                0..line.text.len(),
                2500,
                rect(78),
                [0, 0, 0, 0, 0xffffff, 0x909090],
                0xffffff,
                false
            ));
            assert!(crate::drawing::blurred_text(
                dc,
                font,
                "All your lies all your lies still",
                rect(146),
                0x00909090,
                DT_LEFT,
                2.0
            ));
            draw_text(
                dc,
                font,
                "keep me keep me up at night",
                rect(214),
                0xffffff,
                DT_LEFT | crate::drawing::LYRICS_UNTRIMMED,
            );
            drop(frame);
            let mut image = image::RgbaImage::new(840, 280);
            for (target, pixel) in image.pixels_mut().zip(&pixels) {
                *target =
                    image::Rgba([(*pixel >> 16) as u8, (*pixel >> 8) as u8, *pixel as u8, 255]);
            }
            let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../.local/lyrics-rendering-check.png");
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).unwrap();
            }
            image.save(path).unwrap();
            SelectObject(dc, previous);
            DeleteObject(font);
            DeleteObject(bitmap);
            DeleteDC(dc);
        }
    }

    #[test]
    fn backdrop_waits_at_ambient_until_texture_delivery_then_fades_in() {
        use std::time::{Duration, Instant};
        let start = Instant::now();
        let at = |ms| start + Duration::from_millis(ms);
        let mut transition = super::BackgroundTransition::FadeOut {
            started: start,
            ready_at: None,
        };
        assert_eq!(transition.progress(start), 0.0);
        assert!((transition.progress(at(BACKGROUND_FADE_OUT_MS / 2)) - 0.25).abs() < 0.001);
        transition.advance(at(BACKGROUND_FADE_OUT_MS));
        assert!(matches!(
            transition,
            super::BackgroundTransition::AmbientHold
        ));
        assert!(!transition.animating());
        assert_eq!(transition.progress(at(5000)), 0.5);
        transition.texture_ready(at(5000));
        assert_eq!(transition.progress(at(5000)), 0.5);
        assert!((transition.progress(at(5000 + BACKGROUND_FADE_IN_MS / 2)) - 0.75).abs() < 0.001);
        transition.advance(at(5000 + BACKGROUND_FADE_IN_MS));
        assert!(matches!(transition, super::BackgroundTransition::Complete));
        assert_eq!(transition.progress(at(5000 + BACKGROUND_FADE_IN_MS)), 1.0);
    }

    #[test]
    fn ready_backdrop_flows_through_both_phases_without_restarting_fade_out() {
        use std::time::{Duration, Instant};
        let start = Instant::now();
        let at = |ms| start + Duration::from_millis(ms);
        let mut transition = super::BackgroundTransition::FadeOut {
            started: start,
            ready_at: None,
        };
        transition.texture_ready(at(20));
        assert!((transition.progress(at(BACKGROUND_FADE_OUT_MS / 2)) - 0.25).abs() < 0.001);
        assert_eq!(transition.progress(at(BACKGROUND_FADE_OUT_MS)), 0.5);
        assert!(
            (transition.progress(at(BACKGROUND_FADE_OUT_MS + BACKGROUND_FADE_IN_MS / 2)) - 0.75)
                .abs()
                < 0.001
        );
        assert_eq!(
            transition.progress(at(BACKGROUND_FADE_OUT_MS + BACKGROUND_FADE_IN_MS)),
            1.0
        );

        // Delivery after fade-out, before a timer tick, starts at delivery time.
        let mut delayed = super::BackgroundTransition::FadeOut {
            started: start,
            ready_at: None,
        };
        delayed.texture_ready(at(5000));
        assert_eq!(delayed.progress(at(5000)), 0.5);
        assert!((delayed.progress(at(5000 + BACKGROUND_FADE_IN_MS / 2)) - 0.75).abs() < 0.001);
    }

    #[test]
    fn ambient_midpoint_is_exact_with_missing_or_ready_textures() {
        let image = crate::catalog::Bitmap::new(1, 1, vec![0xff204060]);
        for (current, previous) in [
            (Some(&image), Some(&image)),
            (Some(&image), None),
            (None, Some(&image)),
            (None, None),
        ] {
            let frame = super::BackgroundFrame {
                current,
                previous,
                progress: 128,
                tint: 0x00341256,
                opacity: 100,
                brightness: 100,
            };
            assert_eq!(frame.pixel(0, 0, 1, 1), 0xff341256);
            let mut pixels = [0; 4];
            frame.fill_buffer(&mut pixels, 2, 2, 0, 2);
            assert_eq!(pixels, [0xff341256; 4]);
        }
    }

    #[test]
    fn lyric_prefetch_uses_only_immediate_neighbors_next_first() {
        let queue: Vec<_> = (0..5)
            .map(|i| crate::catalog::Entry {
                id: i.to_string(),
                kind: "track".into(),
                ..Default::default()
            })
            .collect();
        assert_eq!(
            super::adjacent_lyric_tracks(&queue, 2)
                .iter()
                .map(|e| e.id.as_str())
                .collect::<Vec<_>>(),
            vec!["3", "1"]
        );
        assert_eq!(super::adjacent_lyric_tracks(&queue, 0)[0].id, "1");
        assert_eq!(super::adjacent_lyric_tracks(&queue, 4)[0].id, "3");
        assert!(super::adjacent_lyric_tracks(&[], 0).is_empty());
        let mut mixed = queue;
        mixed[3].kind = "episode".into();
        assert_eq!(super::adjacent_lyric_tracks(&mixed, 2).len(), 1);
    }

    #[test]
    fn player_metadata_uses_measured_title_width_at_multiple_dpis() {
        unsafe {
            for scale in [1.0, 1.25, 1.5, 2.0] {
                let fonts = make_fonts(scale);
                let dc = GetDC(null_mut());
                let previous = SelectObject(dc, fonts[10]);
                for width in [588, 1200, 1500] {
                    for layout in [0, 1] {
                        let mut ui = Ui::default();
                        ui.player_layout = layout;
                        ui.dpi_scale = scale;
                        ui.now = crate::catalog::Entry {
                            id: "fixture".into(),
                            title: "rodeo dr".into(),
                            subtitle: "Gunna".into(),
                            raw: serde_json::json!({"explicit":true}),
                            ..Default::default()
                        };
                        let mut scene =
                            ui::build(&ui, (width, 700), true, false, &ui.now.title, true, 70);
                        fit_text_links(null_mut(), &mut scene, &fonts, scale);
                        let title = scene.player_title.as_ref().unwrap();
                        let label = &scene.labels[title.label];
                        let value = wide(&label.text);
                        SelectObject(dc, fonts[10]);
                        let mut extent: SIZE = zeroed();
                        GetTextExtentPoint32W(
                            dc,
                            value.as_ptr(),
                            (value.len() - 1) as i32,
                            &mut extent,
                        );
                        let needed =
                            ((extent.cx as f64 / scale).ceil() as i32).min(title.region.w - 22);
                        assert!(label.rect.w >= needed);
                        let badge = scene.labels[title.badge.unwrap()].rect;
                        assert!(label.rect.x + label.rect.w <= badge.x);
                        assert!(badge.x + badge.w <= title.region.x + title.region.w);
                    }
                }
                SelectObject(dc, previous);
                ReleaseDC(null_mut(), dc);
                for font in fonts {
                    DeleteObject(font);
                }
            }
        }
    }

    #[test]
    fn media_text_uses_empty_space_and_stops_before_neighboring_controls() {
        let mut ui = Ui::default();
        ui.now = crate::catalog::Entry {
            title: "A long title with room to display".into(),
            subtitle: "An artist with a long name".into(),
            ..Default::default()
        };
        let mut scene = ui::build(&ui, (1400, 900), true, false, &ui.now.title, true, 50);
        let original = scene
            .labels
            .iter()
            .find(|l| l.text == ui.now.title)
            .unwrap()
            .rect;
        unsafe {
            expand_media_text(&mut scene, 1384);
        }
        let expanded = scene
            .labels
            .iter()
            .find(|l| l.text == ui.now.title)
            .unwrap()
            .rect;
        assert!(expanded.w >= original.w);
        for button in &scene.buttons {
            let r = button.rect;
            if r.y < expanded.y + expanded.h && r.y + r.h > expanded.y && r.x >= expanded.x {
                assert!(expanded.x + expanded.w <= r.x);
            }
        }
        let slot = Rect::new(10, 10, 80, 20);
        let mut empty = Scene::default();
        assert_eq!(available_text_rect(&empty, slot, 400).w, 390);
        empty
            .pictures
            .push((Rect::new(240, 10, 32, 32), String::new()));
        assert_eq!(available_text_rect(&empty, slot, 400).w, 224);
        empty.pictures[0].0.y = 50;
        assert_eq!(available_text_rect(&empty, slot, 400).w, 390);
    }

    #[test]
    fn overflow_text_pauses_then_scrolls_at_constant_speed_and_resets() {
        assert_eq!(super::marquee_offset(0.5, 120.0, 40.0), 0.0);
        assert!((super::marquee_offset(1.7, 120.0, 40.0) - 40.0).abs() < 0.01);
        assert_eq!(super::marquee_offset(4.0, 120.0, 40.0), 120.0);
        assert_eq!(super::marquee_offset(4.95, 120.0, 40.0), 0.0);
    }

    #[test]
    fn artist_credit_group_names_scroll_together_with_shared_marquee() {
        unsafe {
            let class = wide("ArtistCreditMarqueeTest");
            let instance = GetModuleHandleW(null());
            let wnd_class = WNDCLASSEXW {
                cbSize: size_of::<WNDCLASSEXW>() as u32,
                lpfnWndProc: Some(DefWindowProcW),
                hInstance: instance,
                lpszClassName: class.as_ptr(),
                ..zeroed()
            };
            RegisterClassExW(&wnd_class);
            let hwnd = CreateWindowExW(
                0,
                class.as_ptr(),
                class.as_ptr(),
                WS_POPUP,
                0,
                0,
                600,
                400,
                null_mut(),
                null_mut(),
                instance,
                null_mut(),
            );
            assert!(!hwnd.is_null());
            let dc = GetDC(hwnd);
            let mut app = test_app();
            let fonts = make_fonts(1.0);
            app.fonts = fonts;

            let group = Rect::new(50, 50, 100, 20);
            let group_rect = app.rect(group);

            let artists = vec![
                ui::Button {
                    text: "Kendrick Lamar".into(),
                    text_link: true,
                    rect: Rect::new(50, 50, 60, 20),
                    artist_credit_group: Some(group),
                    ..Default::default()
                },
                ui::Button {
                    text: ", SZA".into(),
                    text_link: true,
                    rect: Rect::new(110, 50, 40, 20),
                    artist_credit_group: Some(group),
                    ..Default::default()
                },
            ];
            app.scene.buttons = artists;

            AUTO_MARQUEES.with(|states| states.borrow_mut().clear());

            let draw0 = DRAWITEMSTRUCT {
                CtlID: 1000,
                hwndItem: hwnd,
                hDC: dc,
                rcItem: app.rect(app.scene.buttons[0].rect),
                ..zeroed()
            };
            let draw1 = DRAWITEMSTRUCT {
                CtlID: 1001,
                hwndItem: hwnd,
                hDC: dc,
                rcItem: app.rect(app.scene.buttons[1].rect),
                ..zeroed()
            };

            draw_button(hwnd, &app, &draw0);
            draw_button(hwnd, &app, &draw1);

            let states_count = AUTO_MARQUEES.with(|states| states.borrow().len());
            assert_eq!(
                states_count, 1,
                "both buttons in group must share a single marquee entry"
            );

            let has_group_key = AUTO_MARQUEES.with(|states| {
                let states = states.borrow();
                states.keys().any(|k| {
                    k.starts_with(&format!(
                        "group:{}:{}:{}:{}",
                        group_rect.left, group_rect.top, group_rect.right, group_rect.bottom
                    ))
                })
            });
            assert!(
                has_group_key,
                "group marquee key must match group rectangle"
            );

            ReleaseDC(hwnd, dc);
            for f in fonts {
                DeleteObject(f);
            }
            DestroyWindow(hwnd);
            UnregisterClassW(class.as_ptr(), instance);
            let mut msg: MSG = zeroed();
            while PeekMessageW(&mut msg, null_mut(), 0, 0, PM_REMOVE) != 0 {}
        }
    }

    #[test]
    fn marquee_timer_invalidates_partial_rect_instead_of_full_window() {
        unsafe {
            let class = wide("MarqueePartialInvalidationTest");
            let instance = GetModuleHandleW(null());
            let wnd_class = WNDCLASSEXW {
                cbSize: size_of::<WNDCLASSEXW>() as u32,
                lpfnWndProc: Some(procedure),
                hInstance: instance,
                lpszClassName: class.as_ptr(),
                ..zeroed()
            };
            RegisterClassExW(&wnd_class);
            let mut app = Box::new(test_app());
            let hwnd = CreateWindowExW(
                0,
                class.as_ptr(),
                class.as_ptr(),
                WS_POPUP,
                100,
                100,
                800,
                600,
                null_mut(),
                null_mut(),
                instance,
                (&mut *app as *mut App).cast::<c_void>(),
            );
            assert!(!hwnd.is_null());

            let target_rect = RECT {
                left: 40,
                top: 80,
                right: 200,
                bottom: 100,
            };
            // Hidden windows do not reliably retain a Win32 update region.
            ShowWindow(hwnd, SW_SHOWNOACTIVATE);
            UpdateWindow(hwnd);
            AUTO_MARQUEES.with(|states| {
                let mut states = states.borrow_mut();
                states.clear();
                let now = std::time::Instant::now();
                states.insert("test_key".into(), (target_rect, now, now));
            });

            ValidateRect(hwnd, null());

            SendMessageW(hwnd, WM_TIMER, 4, 0);

            let mut update: RECT = zeroed();
            GetUpdateRect(hwnd, &mut update, 0);

            let caption = caption_height(hwnd);
            assert_eq!(update.left, target_rect.left);
            assert_eq!(update.top, target_rect.top + caption);
            assert_eq!(update.right, target_rect.right);
            assert_eq!(update.bottom, target_rect.bottom + caption);

            let mut client: RECT = zeroed();
            GetClientRect(hwnd, &mut client);
            assert!(update.right < client.right);
            assert!(update.bottom < client.bottom);

            DestroyWindow(hwnd);
            UnregisterClassW(class.as_ptr(), instance);
        }
    }

    #[test]
    fn word_renderer_advances_individual_words_at_native_screen_offsets() {
        unsafe {
            let dc = CreateCompatibleDC(null_mut());
            let bitmap = CreateBitmap(700, 120, 1, 32, null());
            let previous = SelectObject(dc, bitmap);
            let fonts = make_fonts(1.0);
            let line = crate::lyrics::karaoke_example();
            let render = |position| {
                let mut pixels = vec![0xff000000u32; 700 * 120];
                let frame = crate::drawing::Frame::begin(
                    dc,
                    pixels.as_mut_ptr().cast(),
                    700,
                    700,
                    120,
                    false,
                )
                .unwrap();
                assert!(draw_karaoke_row(
                    dc,
                    fonts[5],
                    line,
                    0..line.text.len(),
                    position,
                    RECT {
                        left: 150,
                        top: 30,
                        right: 650,
                        bottom: 90
                    },
                    colors(false),
                    0,
                    false,
                ));
                drop(frame);
                pixels
                    .iter()
                    .filter(|p| {
                        (**p & 255) > 200 && ((**p >> 8) & 255) > 200 && ((**p >> 16) & 255) > 200
                    })
                    .count()
            };
            assert_eq!(render(0), 0);
            let during = render(1900);
            assert!(during > 20);
            assert!(render(4500) > during);
            SelectObject(dc, previous);
            for font in fonts {
                DeleteObject(font);
            }
            DeleteObject(bitmap);
            DeleteDC(dc);
        }
    }
    #[test]
    fn scroll_motion_is_frame_rate_independent_and_preserves_trackpad_deltas() {
        let simulate = |hz: u32| {
            let mut position = 0.0;
            let mut velocity = 0.0;
            for _ in 0..hz {
                (position, velocity) =
                    smooth_scroll_step(position, velocity, 1000.0, 0.35 / hz as f64);
            }
            position
        };
        assert!((simulate(60) - simulate(144)).abs() < 0.01);
        assert!(simulate(60) > 980.0);
        assert!(
            smooth_scroll_step(0., 0., 1000., 0.016).0 < 50.,
            "wheel motion should ease into the first frame"
        );
        assert!(smooth_scroll_step(100.0, 0.0, 0.0, 0.016).0 < 100.0);
        let mut remainder = 0.0;
        let small: i32 = (0..120)
            .map(|_| wheel_pixels(1, 3, 400, &mut remainder))
            .sum();
        assert!((small - 84).abs() <= 1);
        assert_eq!(wheel_pixels(120, 3, 400, &mut 0.0), 84);
        assert_eq!(wheel_pixels(120, u32::MAX, 400, &mut 0.0), 400);
    }

    #[test]
    fn scrollbar_presentation_keeps_fractional_motion_for_main_and_queue() {
        unsafe {
            let mut app=test_app();
            app.scene.content_top=100;app.scene.content_height=10000;
            app.scene.panels.push(ui::Panel{rect:Rect::new(950,100,9,500),kind:5});
            let main=ui::Panel{rect:Rect::new(950,100,9,32),kind:6};
            app.ui.smooth_scrolling=true;app.ui.scroll=100;app.scroll_position=100.25;app.scroll_target=500;
            let first=shown_scrollbar_y(&app,&main,800).unwrap();
            app.scroll_position=100.75;
            let next=shown_scrollbar_y(&app,&main,800).unwrap();
            assert!(next>first&&next-first<1.);
            app.scene.queue_scrollbar=Some(Rect::new(970,100,8,500));app.scene.queue_scroll_max=1000;
            let queue=ui::Panel{rect:Rect::new(970,100,8,24),kind:17};
            app.queue_glide=Some((100.25,100.,500.,std::time::Instant::now()));
            let first=shown_scrollbar_y(&app,&queue,800).unwrap();
            app.queue_glide.as_mut().unwrap().0=100.75;
            let next=shown_scrollbar_y(&app,&queue,800).unwrap();
            assert!(next>first&&next-first<1.);
            assert_eq!(scrollbar_thumb_y(100.,500.,32.,30000.,1000.),568.);
        }
    }

    #[test]
    fn animation_timer_yields_to_queued_input() {
        unsafe {
            let hwnd = CreateWindowExW(
                0,
                wide("STATIC").as_ptr(),
                wide("timer-input-test").as_ptr(),
                0,
                0,
                0,
                0,
                0,
                HWND_MESSAGE,
                null_mut(),
                GetModuleHandleW(null()),
                null_mut(),
            );
            assert!(!hwnd.is_null());
            start_animation_timer(hwnd, 2);
            std::thread::sleep(std::time::Duration::from_millis(30));
            assert_ne!(PostMessageW(hwnd, WM_KEYDOWN, VK_DOWN as usize, 0), 0);
            let mut message: MSG = zeroed();
            assert_ne!(PeekMessageW(&mut message, hwnd, 0, 0, PM_REMOVE), 0);
            assert_eq!(message.message, WM_KEYDOWN);
            assert_ne!(PeekMessageW(&mut message, hwnd, 0, 0, PM_REMOVE), 0);
            assert_eq!(message.message, WM_TIMER);
            stop_animation_timer(hwnd, 2);
            DestroyWindow(hwnd);
        }
    }
    #[test]
    fn scroll_retargeting_preserves_momentum_and_settles_without_overshoot() {
        let (position, velocity) = smooth_scroll_step(0.0, 0.0, 84.0, 0.03);
        assert!(position > 0.0 && position < 84.0 && velocity > 0.0);
        let (next, continued) = smooth_scroll_step(position, velocity, 168.0, 0.001);
        assert!(next > position && (continued - velocity).abs() < 150.0);
        let mut state = (next, continued);
        for _ in 0..120 {
            state = smooth_scroll_step(state.0, state.1, 168.0, 1.0 / 144.0);
            assert!(state.0 <= 168.0);
        }
        assert_eq!(state, (168.0, 0.0));
        assert_eq!(smooth_scroll_step(10.0, 0.0, 100.0, 1.0), (100.0, 0.0));
    }

    #[test]
    fn settings_scroll_translation_matches_layout_and_keeps_tabs_fixed() {
        for tab in 0..ui::TABS.len() {
            let mut ui = Ui {
                view: View::Settings,
                sidebar_collapsed: true,
                tab,
                ..Default::default()
            };
            let mut scene = ui::build(&ui, (600, 800), true, false, "", false, 50);
            let tabs: Vec<_> = scene
                .buttons
                .iter()
                .filter(|b| matches!(b.action, Action::Tab(_)))
                .map(|b| b.rect)
                .collect();
            shift_scroll_content(&mut scene, -240);
            ui.scroll = 240;
            let fresh = ui::build(&ui, (600, 800), true, false, "", false, 50);
            for (translated, expected) in scene.labels.iter().zip(&fresh.labels) {
                assert!(
                    translated.rect == expected.rect,
                    "label moved incorrectly on settings tab {tab}"
                );
            }
            for ((translated, _, _), (expected, _, _)) in scene.sliders.iter().zip(&fresh.sliders) {
                assert!(translated == expected);
            }
            assert!(scene.eq_graph == fresh.eq_graph && scene.eq_sliders == fresh.eq_sliders);
            assert!(scene
                .buttons
                .iter()
                .filter(|b| matches!(b.action, Action::Tab(_)))
                .map(|b| b.rect)
                .eq(tabs));
        }
    }
    #[test]
    fn buffered_scroll_moves_only_content_and_keeps_visible_rows_available() {
        for provider in [crate::catalog::Provider::Deezer, crate::catalog::Provider::Spotify] {
            check_buffered_scroll_for_provider(provider);
        }
    }
    fn check_buffered_scroll_for_provider(provider: crate::catalog::Provider) {
        let mut ui = Ui {
            provider,
            view: View::Favorites,
            page: Some(Arc::new(crate::catalog::Page {
                rows: (0..1000)
                    .map(|i| crate::catalog::Entry {
                        id: format!("{i}"),
                        title: format!("Song {i}"),
                        kind: "track".into(),
                        ..Default::default()
                    })
                    .collect(),
                ..Default::default()
            })),
            ..Default::default()
        };
        let mut buffered = ui::build(&ui, (1200, 800), true, false, "", false, 50);
        let player = buffered
            .buttons
            .iter()
            .find(|b| matches!(b.action, Action::Toggle))
            .unwrap()
            .rect;
        let started = std::time::Instant::now();
        for _ in 0..64 {
            shift_scroll_content(&mut buffered, -1);
        }
        let translation = started.elapsed();
        ui.scroll = 64;
        let started = std::time::Instant::now();
        let fresh = ui::build(&ui, (1200, 800), true, false, "", false, 50);
        let layout = started.elapsed();
        for button in fresh
            .buttons
            .iter()
            .filter(|b| !b.text_link && b.rect.y >= fresh.content_top && b.rect.y + b.rect.h <= 676)
        {
            if let Action::OpenItem(entry) = &button.action {
                let cached = buffered
                    .buttons
                    .iter()
                    .find(|b| matches!(&b.action,Action::OpenItem(e) if e.id==entry.id))
                    .unwrap();
                assert!(cached.rect == button.rect);
            }
        }
        assert!(
            buffered
                .buttons
                .iter()
                .find(|b| matches!(b.action, Action::Toggle))
                .unwrap()
                .rect
                == player
        );
        eprintln!("64 buffered scroll frames: {translation:?}; one full layout: {layout:?}");
    }

    #[test]
    fn scroll_overscan_covers_rows_until_the_next_layout_in_both_directions() {
        let mut ui = Ui {
            view: View::Favorites,
            page: Some(Arc::new(crate::catalog::Page {
                rows: (0..1000)
                    .map(|i| crate::catalog::Entry {
                        id: format!("{i}"),
                        title: format!("Track {i}"),
                        kind: "track".into(),
                        ..Default::default()
                    })
                    .collect(),
                ..Default::default()
            })),
            ..Default::default()
        };
        for origin in [0, 30_000] {
            for delta in [-(ui::SCROLL_OVERSCAN - 33), ui::SCROLL_OVERSCAN - 33] {
                ui.scroll = origin;
                let mut buffered = ui::build(&ui, (1200, 800), true, false, "", false, 50);
                ui.scroll = (origin + delta).max(0);
                shift_scroll_content(&mut buffered, origin - ui.scroll);
                let fresh = ui::build(&ui, (1200, 800), true, false, "", false, 50);
                for button in fresh.buttons.iter().filter(|button| {
                    button.rect.y + button.rect.h > fresh.content_top && button.rect.y < 676
                }) {
                    if let Action::OpenItem(entry) = &button.action {
                        let cached = buffered.buttons.iter().find(|button| matches!(&button.action, Action::OpenItem(other) if other.id == entry.id));
                        assert_eq!(
                            cached.map(|button| button.rect),
                            Some(button.rect),
                            "missing buffered row {} after scrolling {origin} -> {}",
                            entry.id,
                            ui.scroll
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn podcast_description_survives_buffered_scrolling_in_both_directions() {
        let description = (0..80)
            .map(|i| format!("Description sentence {i} for curious listeners. "))
            .collect::<String>();
        let mut ui = Ui {
            view: View::Show,
            expanded_podcast_description: Some("show".into()),
            page: Some(Arc::new(crate::catalog::Page {
                entity: Some(crate::catalog::Entry {
                    id: "show".into(),
                    kind: "podcast".into(),
                    title: "Show".into(),
                    raw: serde_json::json!({"description":description}),
                    ..Default::default()
                }),
                ..Default::default()
            })),
            ..Default::default()
        };
        let visible = |scene: &Scene| {
            scene
                .labels
                .iter()
                .filter(|l| l.rect.h == 22 && l.rect.y + 22 > scene.content_top && l.rect.y < 676)
                .map(|l| (l.text.clone(), l.rect.y))
                .collect::<Vec<_>>()
        };
        for base in (96..1200).step_by(96) {
            for delta in [-95, 95] {
                ui.scroll = base;
                let mut buffered = ui::build(&ui, (1200, 800), true, false, "", false, 50);
                shift_scroll_content(&mut buffered, -delta);
                ui.scroll = base + delta;
                let fresh = ui::build(&ui, (1200, 800), true, false, "", false, 50);
                assert_eq!(
                    visible(&buffered),
                    visible(&fresh),
                    "description lost after scrolling from {base} by {delta}"
                );
            }
        }
    }
    #[test]
    fn caption_control_modes_have_distinct_visibility_alignment_and_hitboxes() {
        let mut ui = Ui::default();
        assert!(!custom_caption(&ui));
        ui.window_controls = false;
        assert!(custom_caption(&ui));
        ui.window_controls = true;
        ui.controls_right = false;
        assert!(custom_caption(&ui));
        for height in [31, 46, 62] {
            for traffic in [false, true] {
                for right in [false, true] {
                    let rects = caption_button_rects(1200, height, right, traffic);
                    for rect in &rects {
                        assert!(rect.left >= 0 && rect.right <= 1200 && rect.bottom == height);
                    }
                    if right {
                        assert!(rects.iter().all(|r| r.left > 900));
                    } else {
                        assert!(rects.iter().all(|r| r.right < 300));
                    }
                    if !right || traffic {
                        assert!(rects[2].left < rects[0].left && rects[0].left < rects[1].left);
                    } else {
                        assert!(rects[0].left < rects[1].left && rects[1].left < rects[2].left);
                    }
                }
            }
        }
        for left_controls in [false, true] {
            for traffic in [false, true] {
                let collapse = caption_collapse_rect(1200, 31, left_controls, traffic, 1.0);
                let icon_x = collapse.right + 6;
                assert!(collapse.left >= 6);
                assert!(collapse.right < icon_x);
                if left_controls {
                    let rects = caption_button_rects(1200, 31, false, traffic);
                    let controls_right = rects.iter().map(|r| r.right).max().unwrap();
                    assert!(collapse.left > controls_right);
                } else {
                    assert_eq!(collapse.left, 6);
                }
            }
        }
    }
    #[test]
    fn backdrop_copy_supports_packed_and_padded_buffers() {
        let source = [1, 2, 3, 4, 5, 6];
        let mut packed = [0; 6];
        copy_backdrop(&source, &mut packed, 3, 2, 3);
        assert_eq!(packed, source);
        let mut padded = [9; 10];
        copy_backdrop(&source, &mut padded, 3, 2, 5);
        assert_eq!(padded, [1, 2, 3, 9, 9, 4, 5, 6, 9, 9]);
    }

    #[test]
    fn backdrop_scanlines_match_reference_through_fades_and_caption_crops() {
        let current = crate::catalog::Bitmap::new(
            3,
            2,
            vec![
                0xff1745ab, 0xff982031, 0xfff0d417, 0xff0a0b0c, 0xffe8f1fd, 0xff12b245,
            ],
        );
        let previous = crate::catalog::Bitmap::new(
            2,
            3,
            vec![
                0xffedcba9, 0xff020304, 0xff197bf1, 0xff1293aa, 0xfffefefe, 0xff445566,
            ],
        );
        for (curr, prev) in [
            (Some(&current), Some(&previous)),
            (Some(&current), None),
            (None, Some(&previous)),
            (None, None),
        ] {
            for progress in [0, 1, 63, 127, 128, 191, 254, 255] {
                for opacity in [0, 100, 255] {
                    for brightness in [0, 60, 100, 180] {
                        let frame = BackgroundFrame {
                            current: curr,
                            previous: prev,
                            progress,
                            tint: 0x0076ab34,
                            opacity,
                            brightness,
                        };
                        for (width, height, offset) in [(17, 13, 0), (7, 3, 8), (1, 9, 2)] {
                            let full_height = height + offset;
                            let len = (width * height) as usize;
                            let mut dest = vec![0x12345678; len + 1];
                            frame.fill_buffer(&mut dest, width, height, offset, full_height);
                            for y in 0..height {
                                for x in 0..width {
                                    assert_eq!(
                                        dest[(y * width + x) as usize],
                                        frame.pixel(x, y + offset, width, full_height),
                                        "progress={progress}, opacity={opacity}, brightness={brightness}, x={x}, y={y}"
                                    );
                                }
                            }
                            assert_eq!(dest[len], 0x12345678);
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn caption_tint_uses_premultiplied_opacity_and_image_tint() {
        let tint = rgb(80, 120, 160);
        assert_eq!(premultiplied_tint(tint, 0), 0);
        assert_eq!(premultiplied_tint(tint, 100), dib(tint) | 0xff000000);
        let half = premultiplied_tint(tint, 50);
        assert_eq!(half >> 24, 127);
        for shift in [0, 8, 16] {
            assert!((half >> shift) & 255 <= 127);
        }
        let cover = crate::catalog::Bitmap::new(1, 1, vec![0xff204060]);
        let mut backdrop = BackgroundFrame {
            current: Some(&cover),
            previous: None,
            progress: 255,
            tint: dib(tint),
            opacity: 0,
            brightness: 100,
        };
        assert_eq!(backdrop.pixel(0, 0, 100, 100), 0xff204060);
        backdrop.opacity = 255;
        assert_eq!(backdrop.pixel(0, 0, 100, 100), dib(tint) | 0xff000000);
    }
    #[test]
    fn custom_caption_keeps_native_drag_controls_and_content_coordinates() {
        unsafe extern "system" fn test_proc(hwnd: HWND, msg: u32, wp: usize, lp: isize) -> isize {
            caption_message(hwnd, msg, wp, lp).unwrap_or_else(|| DefWindowProcW(hwnd, msg, wp, lp))
        }
        unsafe {
            let name = wide("DeezNatsCaptionOpacityTest");
            let instance = GetModuleHandleW(null());
            let class = WNDCLASSW {
                lpfnWndProc: Some(test_proc),
                hInstance: instance,
                lpszClassName: name.as_ptr(),
                ..zeroed()
            };
            assert_ne!(RegisterClassW(&class), 0);
            let window = CreateWindowExW(
                0,
                name.as_ptr(),
                wide("Caption test").as_ptr(),
                WS_OVERLAPPEDWINDOW,
                100,
                100,
                900,
                600,
                null_mut(),
                null_mut(),
                instance,
                null(),
            );
            assert!(!window.is_null());
            SetWindowPos(
                window,
                null_mut(),
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED,
            );
            let mut client: RECT = zeroed();
            GetClientRect(window, &mut client);
            let caption = caption_height(window);
            assert!((20..100).contains(&caption));
            let point = POINT { x: 100, y: 80 };
            let mut screen = point;
            content_to_screen(window, &mut screen);
            screen_to_content(window, &mut screen);
            assert_eq!((screen.x, screen.y), (point.x, point.y));
            let hit = |x: i32, y: i32| {
                let mut point = POINT { x, y };
                ClientToScreen(window, &mut point);
                let packed = ((point.x as u16 as u32) | ((point.y as u16 as u32) << 16)) as isize;
                SendMessageW(window, WM_NCHITTEST, 0, packed)
            };
            assert_eq!(hit(100, caption / 2), HTCAPTION as isize);
            assert_eq!(hit(100, caption + 20), HTCLIENT as isize);
            assert_eq!(hit(client.right - 20, caption / 2), HTCLOSE as isize);
            assert_eq!(hit(client.right - 70, caption / 2), HTMAXBUTTON as isize);
            assert_eq!(hit(client.right - 120, caption / 2), HTMINBUTTON as isize);
            assert_eq!(hit(100, 1), HTTOP as isize);
            assert_eq!(IsWindowVisible(window), 0);
            // Native controls still own hit testing, but our shared glass
            // caption must paint their symbols instead of relying on DWM.
            let dc = CreateCompatibleDC(null_mut());
            let bitmap = CreateBitmap(client.right, caption, 1, 32, null());
            let previous = SelectObject(dc, bitmap);
            let mut pixels = vec![0xff000000u32; (client.right * caption) as usize];
            let frame = crate::drawing::Frame::begin(
                dc,
                pixels.as_mut_ptr().cast(),
                client.right,
                client.right,
                caption,
                false,
            )
            .unwrap();
            let ui = Ui::default();
            assert!(!custom_caption(&ui));
            draw_caption_controls(window, dc, &ui, client.right, caption, 1.0, colors(false));
            drop(frame);
            for rect in native_caption_button_rects(window, client.right, caption) {
                let visible = (rect.top..rect.bottom)
                    .flat_map(|y| {
                        (rect.left.max(0)..rect.right.min(client.right))
                            .map(move |x| (y * client.right + x) as usize)
                    })
                    .filter(|index| pixels[*index] & 0xffffff != 0)
                    .count();
                assert!(visible > 5, "default right caption symbol was not painted");
            }
            SelectObject(dc, previous);
            DeleteObject(bitmap);
            DeleteDC(dc);
            DestroyWindow(window);
            UnregisterClassW(name.as_ptr(), instance);
        }
    }

    #[test]
    fn shelf_text_links_are_never_treated_as_full_width_track_rows() {
        let track = crate::catalog::Entry {
            id: "1".into(),
            title: "Loser".into(),
            subtitle: "Tame Impala".into(),
            kind: "track".into(),
            artist_id: "2".into(),
            artwork: "fixture".into(),
            ..Default::default()
        };
        let mut ui = Ui {
            page: Some(Arc::new(crate::catalog::Page {
                sections: vec![crate::catalog::Section {
                    title: "Songs".into(),
                    entries: vec![track.clone()],
                    ..Default::default()
                }],
                ..Default::default()
            })),
            ..Default::default()
        };
        let mut scene = ui::build(&ui, (1200, 800), true, false, "", false, 50);
        assert!(scene
            .buttons
            .iter()
            .any(|b| b.text_link && b.text == "Tame Impala"));
        assert!(scene.buttons.iter().all(|b| !is_track_row(b)));
        let links: Vec<_> = scene
            .buttons
            .iter()
            .filter(|b| b.text_link)
            .map(|b| b.rect)
            .collect();
        unsafe {
            let fonts = make_fonts(1.25);
            fit_text_links(null_mut(), &mut scene, &fonts, 1.25);
            for (button, slot) in scene.buttons.iter().filter(|b| b.text_link).zip(links) {
                if let Some(group) = button.artist_credit_group {
                    assert!(button.rect.x >= group.x);
                    assert!(button.rect.x + button.rect.w <= group.x + group.w);
                } else {
                    assert!(button.rect.w < slot.w);
                }
                assert!(button.rect.h <= slot.h);
                assert!((button.rect.x + button.rect.w / 2 - slot.x - slot.w / 2).abs() <= 1);
            }
            for font in fonts {
                DeleteObject(font);
            }
        }
        ui.view = View::Favorites;
        ui.page = Some(Arc::new(crate::catalog::Page {
            rows: vec![track],
            ..Default::default()
        }));
        let table = ui::build(&ui, (1200, 800), true, false, "", false, 50);
        assert_eq!(table.buttons.iter().filter(|b| is_track_row(b)).count(), 1);
        assert!(table
            .buttons
            .iter()
            .filter(|b| b.text_link)
            .all(|b| !is_track_row(b)));
    }

    #[test]
    fn sliders_map_clicks_to_thumb_centers_at_multiple_dpis() {
        for scale in [1, 2, 3] {
            let width = 210 * scale;
            let margin = 5 * scale;
            assert_eq!(slider_value(-100, width, margin, SEEK_STEPS), 0);
            assert_eq!(slider_value(margin, width, margin, SEEK_STEPS), 0);
            assert_eq!(slider_value(width / 2, width, margin, SEEK_STEPS), 5000);
            assert_eq!(
                slider_value(width - margin, width, margin, SEEK_STEPS),
                10000
            );
            assert_eq!(slider_value(width + 100, width, margin, 100), 100);
        }
        // A pixel on a long seek bar must resolve more finely than one percent.
        assert_eq!(slider_value(106, 1010, 5, SEEK_STEPS), 1010);
    }
    #[test]
    fn track_info_play_button_is_not_a_track_row() {
        let mut ui = ui::Ui::default();
        ui.view = View::TrackInfo;
        ui.track_info = Some((
            crate::catalog::Entry {
                id: "1".into(),
                title: "Song".into(),
                kind: "track".into(),
                ..Default::default()
            },
            serde_json::Value::Null,
            serde_json::Value::Null,
        ));
        let scene = ui::build(&ui, (1200, 800), true, false, "", false, 50);
        let button = scene
            .buttons
            .iter()
            .find(|button| button.text == "Play track")
            .unwrap();
        assert!(!is_track_row(button));
    }
    #[test]
    fn library_routes_podcasts_albums_and_tracks_to_the_right_apis() {
        use serde_json::json;
        for (kind, expected) in [
            ("podcast", "RustPodcastLibraryJSON"),
            ("album", "DZAlbumLibraryJSON"),
            ("playlist", "DZPlaylistLibraryJSON"),
            ("track", "DZAddFavorite"),
        ] {
            let item = crate::catalog::Entry {
                id: "42".into(),
                title: "Example".into(),
                kind: kind.into(),
                ..Default::default()
            };
            let crate::catalog::Job::Api(name, args) = library_job(&item, true).unwrap() else {
                panic!("Expected API request");
            };
            assert_eq!(name, expected);
            if kind == "podcast" {
                assert_eq!(args[0]["kind"], "podcast");
                assert_eq!(args[1], true);
                assert_ne!(item.library_key(), "42");
            }
            if kind == "album" || kind == "playlist" {
                assert_eq!(args, vec![json!("42"), json!(1)]);
            }
        }
        assert!(library_job(
            &crate::catalog::Entry {
                id: "42".into(),
                kind: "artist".into(),
                ..Default::default()
            },
            true
        )
        .is_err());
    }
    #[test]
    fn share_links_preserve_episode_and_show_identity() {
        for kind in ["episode", "podcast", "track", "album", "artist", "playlist"] {
            assert_eq!(
                share_link(&crate::catalog::Entry {
                    id: "42".into(),
                    kind: kind.into(),
                    ..Default::default()
                }),
                Some(format!("https://www.deezer.com/{kind}/42"))
            );
        }
        assert!(share_link(&crate::catalog::Entry {
            id: "web:/channel".into(),
            kind: "channel".into(),
            ..Default::default()
        })
        .is_none());
    }
    #[test]
    fn context_menu_favorite_labels_match_reference_for_tracks_and_collections() {
        let track = crate::catalog::Entry {
            id: "1".into(),
            title: "Track".into(),
            kind: "track".into(),
            ..Default::default()
        };
        assert_eq!(
            context_menu_favorite_label(&track, false),
            "Add to favorites"
        );
        assert_eq!(
            context_menu_favorite_label(&track, true),
            "Remove from favorites"
        );

        let album = crate::catalog::Entry {
            id: "2".into(),
            title: "Album".into(),
            kind: "album".into(),
            ..Default::default()
        };
        assert_eq!(context_menu_favorite_label(&album, false), "Add to library");
        assert_eq!(
            context_menu_favorite_label(&album, true),
            "Remove from library"
        );

        let podcast = crate::catalog::Entry {
            id: "3".into(),
            title: "Podcast".into(),
            kind: "podcast".into(),
            ..Default::default()
        };
        assert_eq!(
            context_menu_favorite_label(&podcast, false),
            "Add to library"
        );
        assert_eq!(
            context_menu_favorite_label(&podcast, true),
            "Remove from library"
        );
    }
    #[test]
    fn dump_appearance_screenshots() {
        let Ok(out_dir) = std::env::var("DEEZNATS_SCREENSHOT_DIR") else {
            return;
        };
        std::env::set_var("DEEZNATS_NO_AUTOLOGIN", "1");
        unsafe {
            let controls = INITCOMMONCONTROLSEX {
                dwSize: size_of::<INITCOMMONCONTROLSEX>() as u32,
                dwICC: ICC_BAR_CLASSES,
            };
            InitCommonControlsEx(&controls);
            let instance = GetModuleHandleW(null());
            let class = wide("DeezNatsHiddenScreenshotClass");
            let wc = WNDCLASSW {
                lpfnWndProc: Some(procedure),
                hInstance: instance,
                lpszClassName: class.as_ptr(),
                ..zeroed()
            };
            let _ = RegisterClassW(&wc);
            let scale = 1.0;
            let fonts = make_fonts(scale);
            let mut ui = Ui::default();
            ui.view = View::Settings;
            ui.tab = 2;
            ui.show_in_tray = false;
            ui.expanded_themes = true;
            ui.expanded_palettes = true;
            let mut app = Box::new(App {
                queue_drag: None,
                queue_scroll_drag: false,
                queue_wheel_remainder: 0.0,
                queue_glide: None,
                middle_scroll: None,
                caption_buttons: [null_mut(); 3],
                collapse_button: null_mut(),
                tray_added: false,
                search_backdrop: Default::default(),
                worker: Worker::start(),
                updater: crate::updater::Worker::start(),
                media_controls: None,
                shortcuts: None,
                snapshot: Snapshot::default(),
                playback_clock: PlaybackClock::default(),
                karaoke_preview_started: std::time::Instant::now(),
                lyric_motion: None,
                lyrics_mode_motion: None,
                lyric_motion_offset: 0,
                lyric_motion_track: String::new(),
                ui,
                scene: Scene::default(),
                buttons: Vec::new(),
                button_text: Vec::new(),
                canvas_pressed: None,
                canvas_hover: None,
                canvas_focus: None,
                canvas_keyboard: null_mut(),
                edits: HashMap::new(),
                fonts,
                lyrics_fonts: None,
                brush: CreateSolidBrush(rgb(29, 29, 29)),
                scale,
                acrylic: false,
                pending: None,
                notice: String::new(),
                volume: 70,
                scrolling: false,
                render_suspended: false,
                layout_dirty: false,
                scroll_grab_offset: 0,
                catalog: crate::catalog::Worker::start(),

                spotify_service: None,
                spotify_opened: false,
                spotify_setup: None,
                spotify_auth: None,
                spotify_device_request: None,
                spotify_jam_request: None,
                deezer_auth: None,

                artwork: crate::catalog::Artwork::start(),
                images: HashMap::new(),
                image_order: VecDeque::new(),
                image_pending: HashSet::new(),
                image_retry_at: HashMap::new(),
                generation: 0,
                entity: None,
                query: String::new(),
                navigation: Vec::new(),
                lyrics_id: String::new(),
                background: None,
                background_source: String::new(),
                background_requested: false,
                backdrop_worker: crate::appearance::Worker::start(),
                previous_background: None,
                background_transition: BackgroundTransition::Complete,
                previous_cover: None,
                cover_source: String::new(),
                cover_changed: None,
                custom_background: String::new(),
                image_times: HashMap::new(),
                scroll_target: 0,
                scroll_origin: 0,
                scroll_position: 0.0,
                scroll_velocity: 0.0,
                scroll_tick: std::time::Instant::now(),
                wheel_remainder: 0.0,
                layout_scroll: 0,
                artwork_prefetch_scroll: 0,
                artwork_prefetch_upward: false,
                scroll_started: std::time::Instant::now(),
                backdrop_cache: Default::default(),
                palette_state: Default::default(),
                cover_luminance_cache: Default::default(),
                downloads: crate::downloads::Worker::start(),
                download_cancels: HashMap::new(),
                flow_save_expected: None,
                flow_verify_attempt: 0,
                search_hint: String::new(),
                page_cache: VecDeque::new(),
                toggle_animations: HashMap::new(),
                eq_drag: None,
                visualizer: crate::visualizer::Visualizer::new(),
                podcast_ids: HashSet::new(),
                library_pending: None,
                playlist_generation: None,
                discord_seen: serde_json::Value::Null,
                library_loaded: false,
                lyrics_hover_start: std::time::Instant::now(),
                lyrics_hover_from: 0.0,
                lyrics_hover_to: 0.0,
                connect: None,
            });
            let hwnd = CreateWindowExW(
                0,
                class.as_ptr(),
                wide("DeezNats").as_ptr(),
                WS_OVERLAPPEDWINDOW,
                0,
                0,
                1280,
                1040,
                null_mut(),
                null_mut(),
                instance,
                (&mut *app as *mut App).cast::<c_void>(),
            );
            assert!(!hwnd.is_null());
            if std::env::var_os("DEEZNATS_TEST_ARTWORK_SCROLL").is_some() {
                let streaming = std::env::var_os("DEEZNATS_TEST_ARTWORK_ARRIVALS").is_some();
                let grid = std::env::var_os("DEEZNATS_TEST_ARTWORK_GRID").is_some();
                let shelves = std::env::var_os("DEEZNATS_TEST_ARTWORK_SHELVES").is_some();
                let discovery = std::env::var_os("DEEZNATS_TEST_ARTWORK_DISCOVERY").is_some();
                let (arrivals, updates) = std::sync::mpsc::channel();
                let (requests, _unused_requests) = std::sync::mpsc::sync_channel(1);
                app.artwork = crate::catalog::Artwork::fixture(requests, updates);
                app.ui.view = View::Favorites;
                app.ui.page = Some(Arc::new(crate::catalog::Page {
                    title: "Artwork scrolling benchmark".into(),
                    rows: (0..1000)
                        .map(|i| crate::catalog::Entry {
                            id: format!("scroll-track-{i}"),
                            title: format!("Artwork track {i}"),
                            subtitle: "Background benchmark".into(),
                            artwork: format!("scroll-art-{}", i % 80),
                            kind: "track".into(),
                            ..Default::default()
                        })
                        .collect(),
                    ..Default::default()
                }));
                if grid {
                    app.ui.view = View::Home;
                    app.ui.shelf_rows.insert("Catalogue".into(), 80);
                    app.ui.page = Some(Arc::new(crate::catalog::Page {
                        title: "Artwork catalogue benchmark".into(),
                        sections: vec![crate::catalog::Section {
                            title: "Catalogue".into(),
                            entries: (0..160)
                                .map(|i| crate::catalog::Entry {
                                    id: format!("scroll-album-{i}"),
                                    title: format!("Album {i}"),
                                    artwork: format!("scroll-art-{}", i % 80),
                                    kind: "album".into(),
                                    ..Default::default()
                                })
                                .collect(),
                            ..Default::default()
                        }],
                        raw: vec![("catalogue".into(), serde_json::json!(true))],
                        ..Default::default()
                    }));
                }
                if shelves || discovery {
                    app.ui.view = if discovery { View::Explore } else { View::Home };
                    app.ui.page = Some(Arc::new(crate::catalog::Page {
                        title: "Home and Discovery scrolling benchmark".into(),
                        sections: (0..20)
                            .map(|section| crate::catalog::Section {
                                title: format!("Recommended collection {section}"),
                                entries: (0..12)
                                    .map(|item| crate::catalog::Entry {
                                        id: format!("collection-{section}-{item}"),
                                        title: format!("Collection {section} album {item}"),
                                        subtitle: "Recommended for you".into(),
                                        artwork: format!(
                                            "scroll-art-{}",
                                            (section * 12 + item) % 80
                                        ),
                                        kind: if discovery { "channel" } else { "album" }.into(),
                                        ..Default::default()
                                    })
                                    .collect(),
                                ..Default::default()
                            })
                            .collect(),
                        raw: if discovery {
                            vec![("discover".into(), serde_json::json!(true))]
                        } else {
                            vec![]
                        },
                        ..Default::default()
                    }));
                }
                for i in 0..80 {
                    let key = format!("scroll-art-{i}");
                    let pixels = (0..192 * 192)
                        .map(|p| 0xff000000 | (((p + i * 31) as u32 * 7919) & 0xffffff))
                        .collect();
                    let bitmap = Arc::new(crate::catalog::Bitmap::new(192, 192, pixels));
                    if streaming {
                        app.image_pending.insert(key.clone());
                        arrivals.send((key, Ok(bitmap))).unwrap();
                    } else {
                        app.images.insert(key.clone(), bitmap);
                        app.image_order.push_back(key);
                    }
                }
                app.ui.queue_sidebar_open = false;
                rebuild(hwnd, &mut app);
                if std::env::var_os("DEEZNATS_TEST_HOME_PREFETCH").is_some() {
                    assert!(matches!(app.ui.view, View::Home | View::Explore));
                    let (requests, queued) = std::sync::mpsc::sync_channel(48);
                    let (_updates_sender, updates) = std::sync::mpsc::channel();
                    app.artwork = crate::catalog::Artwork::fixture(requests, updates);
                    app.image_pending.clear();
                    app.images.clear();
                    app.image_order.clear();
                    let original_layout = app.layout_scroll;
                    scroll_to(hwnd, &mut app, 96);
                    assert_eq!(
                        app.layout_scroll, original_layout,
                        "prefetch must not force a layout rebuild"
                    );
                    assert_eq!(app.artwork_prefetch_scroll, 96);
                    assert!(!app.artwork_prefetch_upward);
                    let (_, height) = size(hwnd, &app);
                    let mut visible: HashSet<String> = app
                        .scene
                        .buttons
                        .iter()
                        .filter(|node| {
                            !node.artwork.is_empty()
                                && (matches!(node.action, Action::Go(View::Account))
                                    || (node.rect.y + node.rect.h > app.scene.content_top - 160
                                        && node.rect.y < height + 36))
                        })
                        .map(|node| node.artwork.clone())
                        .collect();
                    visible.extend(
                        app.scene
                            .pictures
                            .iter()
                            .filter(|(rect, _)| rect.y + rect.h > 0 && rect.y < height + 36)
                            .map(|(_, url)| url.clone()),
                    );
                    if !app.ui.now.artwork.is_empty() {
                        visible.insert(app.ui.now.artwork.clone());
                    }
                    let records: Vec<_> = queued.try_iter().collect();
                    assert!(
                        !records.is_empty(),
                        "scroll must submit missing artwork before the next layout rebuild"
                    );
                    let mut ahead = false;
                    for request in records {
                        if visible.contains(&request.key) {
                            assert!(!ahead, "visible covers must precede speculative covers");
                        } else {
                            ahead = true;
                        }
                    }
                    app.image_pending = (0..12)
                        .map(|index| format!("unrelated-pending-{index}"))
                        .collect();
                    scroll_to(hwnd, &mut app, 0);
                    assert!(app.artwork_prefetch_upward);
                    assert_eq!(app.artwork_prefetch_scroll, 0);
                    let mut visible: HashSet<String> = app
                        .scene
                        .buttons
                        .iter()
                        .filter(|node| {
                            !node.artwork.is_empty()
                                && (matches!(node.action, Action::Go(View::Account))
                                    || (node.rect.y + node.rect.h > app.scene.content_top - 160
                                        && node.rect.y < height + 36))
                        })
                        .map(|node| node.artwork.clone())
                        .collect();
                    visible.extend(
                        app.scene
                            .pictures
                            .iter()
                            .filter(|(rect, _)| rect.y + rect.h > 0 && rect.y < height + 36)
                            .map(|(_, url)| url.clone()),
                    );
                    if !app.ui.now.artwork.is_empty() {
                        visible.insert(app.ui.now.artwork.clone());
                    }
                    let urgent: Vec<_> = queued.try_iter().collect();
                    assert!(
                        !urgent.is_empty(),
                        "visible covers must still be queued when speculative capacity is full"
                    );
                    assert!(
                        urgent.iter().all(|request| visible.contains(&request.key)),
                        "speculative work must not crowd out visible covers"
                    );
                    DestroyWindow(hwnd);
                    UnregisterClassW(class.as_ptr(), instance);
                    return;
                }
                if std::env::var_os("DEEZNATS_TEST_CANVAS_TRACKS").is_some() {
                    let (w, h) = size(hwnd, &app);
                    let index = app
                        .scene
                        .content_buttons
                        .clone()
                        .find(|&index| {
                            let node = &app.scene.buttons[index];
                            if !matches!(node.action, Action::ToggleCheck(_)) {
                                return false;
                            }
                            let x = node.rect.x + node.rect.w / 2;
                            let y = node.rect.y + node.rect.h / 2;
                            canvas_track_hit(&app, x, y, w, h) == Some(index)
                        })
                        .expect("visible canvas checkbox");
                    assert!(
                        app.buttons[index].is_null()
                            || GetWindowLongW(app.buttons[index], GWL_STYLE) as u32 & WS_VISIBLE
                                == 0
                    );
                    let id = match &app.scene.buttons[index].action {
                        Action::ToggleCheck(id) => id.clone(),
                        _ => unreachable!(),
                    };
                    let rect = app.scene.buttons[index].rect;
                    let x = app.px(rect.x + rect.w / 2);
                    let y = app.px(rect.y + rect.h / 2) + caption_height(hwnd);
                    let point = ((y as u32 & 0xffff) << 16 | (x as u32 & 0xffff)) as isize;
                    app.scene.buttons[index].enabled = false;
                    assert_ne!(
                        canvas_track_hit(&app, rect.x + rect.w / 2, rect.y + rect.h / 2, w, h),
                        Some(index)
                    );
                    app.scene.buttons[index].enabled = true;
                    SendMessageW(hwnd, WM_LBUTTONDOWN, 0, point);
                    SendMessageW(hwnd, WM_LBUTTONUP, 0, 0);
                    assert!(
                        !app.ui.checked_ids.contains(&id),
                        "release outside must cancel the click"
                    );
                    SendMessageW(hwnd, WM_MOUSEMOVE, 0, point);
                    assert_eq!(app.canvas_hover, Some(index));
                    SendMessageW(hwnd, WM_LBUTTONDOWN, 0, point);
                    assert_eq!(
                        app.canvas_pressed.as_ref().map(|value| value.0),
                        Some(index)
                    );
                    SendMessageW(hwnd, WM_LBUTTONUP, 0, point);
                    assert!(app.ui.checked_ids.contains(&id));
                    assert!(!app.canvas_keyboard.is_null());
                    let mut message: MSG = zeroed();
                    message.message = WM_KEYDOWN;
                    message.wParam = 13;
                    assert_ne!(
                        SendMessageW(
                            app.canvas_keyboard,
                            WM_GETDLGCODE,
                            0,
                            (&message as *const MSG) as isize
                        ) as u32
                            & DLGC_WANTMESSAGE,
                        0
                    );
                    let focused = app
                        .canvas_focus
                        .expect("clicked canvas item must retain keyboard focus");
                    SendMessageW(app.canvas_keyboard, WM_KEYDOWN, VK_DOWN as usize, 0);
                    assert_ne!(
                        app.canvas_focus,
                        Some(focused),
                        "down arrow should advance to the next row"
                    );
                    assert!(app
                        .canvas_focus
                        .is_some_and(|next| canvas_track_button(&app, next)));
                    let sort = app
                        .scene
                        .content_buttons
                        .clone()
                        .find(|&candidate| {
                            let node = &app.scene.buttons[candidate];
                            if !matches!(node.action, Action::Sort(1)) {
                                return false;
                            }
                            canvas_track_hit(
                                &app,
                                node.rect.x + node.rect.w / 2,
                                node.rect.y + node.rect.h / 2,
                                w,
                                h,
                            ) == Some(candidate)
                        })
                        .expect("visible sort control");
                    let sort_rect = app.scene.buttons[sort].rect;
                    let sort_x = app.px(sort_rect.x + sort_rect.w / 2);
                    let sort_y = app.px(sort_rect.y + sort_rect.h / 2) + caption_height(hwnd);
                    let sort_point =
                        ((sort_y as u32 & 0xffff) << 16 | (sort_x as u32 & 0xffff)) as isize;
                    SendMessageW(hwnd, WM_LBUTTONDOWN, 0, sort_point);
                    SendMessageW(hwnd, WM_LBUTTONUP, 0, sort_point);
                    assert_eq!(app.ui.sort_column, 1);
                    for view in [
                        View::Home,
                        View::Explore,
                        View::Favorites,
                        View::Recent,
                        View::Offline,
                        View::Flow,
                        View::FlowTune,
                        View::StyleMix,
                        View::Podcasts,
                        View::Albums,
                        View::Playlists,
                        View::PlaylistPicker,
                        View::Queue,
                        View::Album,
                        View::Artist,
                        View::Show,
                        View::Settings,
                    ] {
                        app.ui.view = view;
                        assert!(canvas_track_button(&app, index), "{view:?}");
                    }
                    app.ui.view = View::Favorites;
                }
                assert!(
                    app.scene.content_height > 3000,
                    "scroll benchmark must exercise a long list or catalogue"
                );
                assert!(
                    !app.scene.pictures.is_empty()
                        || app
                            .scene
                            .buttons
                            .iter()
                            .any(|button| !button.artwork.is_empty()),
                    "scroll benchmark must include artwork"
                );
                let dc = GetDC(hwnd);
                let memory = CreateCompatibleDC(dc);
                let target = CreateCompatibleBitmap(dc, 1280, 1040);
                let previous = SelectObject(memory, target);
                let mut samples = Vec::new();
                for frame in 0..240 {
                    let started = std::time::Instant::now();
                    if streaming {
                        process_data(hwnd, &mut app);
                    }
                    crate::drawing::poll_prepared_artwork();
                    scroll_to(hwnd, &mut app, frame * 12);
                    let paint_rect =
                        std::env::var_os("DEEZNATS_TEST_PARTIAL_ARTWORK_SCROLL").map(|_| {
                            let (_, height) = size(hwnd, &app);
                            let mut client: RECT = zeroed();
                            GetClientRect(hwnd, &mut client);
                            scroll_repaint_rect(&app, client.right, height, caption_height(hwnd))
                        });
                    render(hwnd, &app, memory, paint_rect);
                    GdiFlush();
                    samples.push(started.elapsed().as_secs_f64() * 1000.0);
                }
                samples.sort_by(f64::total_cmp);
                eprintln!(
                    "ARTWORK_SCROLL 80 covers / 240 frames (grid={grid}, shelves={shelves}, discovery={discovery}, streaming={streaming}): median={:.2}ms p95={:.2}ms max={:.2}ms",
                    samples[120], samples[228], samples[239]
                );
                SelectObject(memory, previous);
                DeleteObject(target);
                DeleteDC(memory);
                ReleaseDC(hwnd, dc);
                DestroyWindow(hwnd);
                UnregisterClassW(class.as_ptr(), instance);
                return;
            }

            if std::env::var_os("DEEZNATS_TEST_ARTWORK_MEMORY").is_some() {
                app.ui.view = View::Favorites;
                app.scene.content_top = 100;
                for i in 0..130 {
                    let key = format!("fixture-{i}");
                    app.images.insert(
                        key.clone(),
                        Arc::new(crate::catalog::Bitmap::new(
                            192,
                            192,
                            vec![0xff001122; 192 * 192],
                        )),
                    );
                    app.image_order.push_back(key.clone());
                    if i < 20 {
                        app.scene.artwork_ahead.push((110, key));
                    }
                }
                let before: usize = app
                    .images
                    .values()
                    .map(|image| image.pixels.len() * 4)
                    .sum();
                trim_artwork(&mut app, 800);
                let after: usize = app
                    .images
                    .values()
                    .map(|image| image.pixels.len() * 4)
                    .sum();
                assert!(after < before);
                assert!((0..20).all(|i| app.images.contains_key(&format!("fixture-{i}"))));
                eprintln!("fixture decoded artwork before {before} bytes, after {after} bytes");
                DestroyWindow(hwnd);
                UnregisterClassW(class.as_ptr(), instance);
                return;
            }

            if std::env::var_os("DEEZNATS_TEST_PARTIAL_SCROLL").is_some() {
                app.ui.view = View::Favorites;
                app.ui.page = Some(Arc::new(crate::catalog::Page {
                    rows: (0..120)
                        .map(|i| crate::catalog::Entry {
                            id: format!("track-{i}"),
                            title: format!("Track {i}"),
                            kind: "track".into(),
                            ..Default::default()
                        })
                        .collect(),
                    ..Default::default()
                }));
                let mut client: RECT = zeroed();
                GetClientRect(hwnd, &mut client);
                let (width, height) = (client.right, client.bottom);
                let info = BITMAPINFO {
                    bmiHeader: BITMAPINFOHEADER {
                        biSize: size_of::<BITMAPINFOHEADER>() as u32,
                        biWidth: width,
                        biHeight: -height,
                        biPlanes: 1,
                        biBitCount: 32,
                        biCompression: BI_RGB,
                        ..zeroed()
                    },
                    ..zeroed()
                };
                let make_target = || {
                    let dc = CreateCompatibleDC(null_mut());
                    let mut bits = null_mut();
                    let bitmap =
                        CreateDIBSection(dc, &info, DIB_RGB_COLORS, &mut bits, null_mut(), 0);
                    assert!(!dc.is_null() && !bitmap.is_null() && !bits.is_null());
                    let previous = SelectObject(dc, bitmap);
                    (dc, bitmap, previous, bits.cast::<u32>())
                };
                for mode in 0..3 {
                    app.ui.scroll = 0;
                    app.scroll_target = 0;
                    app.acrylic = mode == 1;
                    app.background_source = if mode == 2 {
                        "test-background".into()
                    } else {
                        String::new()
                    };
                    app.background = (mode == 2).then(|| {
                        Arc::new(crate::catalog::Bitmap::new(
                            2,
                            2,
                            vec![0xff102030, 0xff405060, 0xff708090, 0xffa0b0c0],
                        ))
                    });
                    app.previous_background = None;
                    app.background_transition = BackgroundTransition::Complete;
                    rebuild(hwnd, &mut app);
                    // Rebuild may select the configured background; force each test mode
                    // after layout so the image and acrylic paths are both exercised.
                    app.acrylic = mode == 1;
                    app.background_source = if mode == 2 {
                        "test-background".into()
                    } else {
                        String::new()
                    };
                    app.background = (mode == 2).then(|| {
                        Arc::new(crate::catalog::Bitmap::new(
                            2,
                            2,
                            vec![0xff102030, 0xff405060, 0xff708090, 0xffa0b0c0],
                        ))
                    });
                    app.previous_background = None;
                    app.background_transition = BackgroundTransition::Complete;
                    let _ = app.interpolated_palette_and_accent();
                    app.palette_state.borrow_mut().started =
                        std::time::Instant::now() - std::time::Duration::from_secs(10);
                    let (partial_dc, partial_bitmap, partial_old, partial_bits) = make_target();
                    let (full_dc, full_bitmap, full_old, full_bits) = make_target();
                    render(hwnd, &app, partial_dc, None);
                    GdiFlush();
                    let count = (width * height) as usize;
                    let before = std::slice::from_raw_parts(partial_bits, count).to_vec();
                    scroll_to(hwnd, &mut app, 48);
                    assert_eq!(app.ui.scroll, 48);
                    let (_, logical_height) = size(hwnd, &app);
                    let region =
                        scroll_repaint_rect(&app, width, logical_height, caption_height(hwnd));
                    assert!(scroll_repaint_avoids_search(
                        &app,
                        &region,
                        caption_height(hwnd)
                    ));
                    let saved = SaveDC(partial_dc);
                    IntersectClipRect(
                        partial_dc,
                        region.left,
                        region.top,
                        region.right,
                        region.bottom,
                    );
                    render(hwnd, &app, partial_dc, Some(region));
                    RestoreDC(partial_dc, saved);
                    render(hwnd, &app, full_dc, None);
                    GdiFlush();
                    let actual = std::slice::from_raw_parts(partial_bits, count);
                    let expected = std::slice::from_raw_parts(full_bits, count);
                    assert!(
                        before.iter().zip(expected).any(|(a, b)| a != b),
                        "scroll did not change the rendered frame"
                    );
                    let mismatch = actual.iter().zip(expected).position(|(a, b)| a != b);
                    if let Some(index) = mismatch {
                        let baseline = expected.to_vec();
                        render(hwnd, &app, full_dc, None);
                        GdiFlush();
                        let control = baseline.iter().zip(expected).position(|(a, b)| a != b);
                        panic!(
                            "partial scroll differs in mode {mode} at ({}, {}), region ({}, {}, {}, {}), pixels {:08x}/{:08x}, control {control:?}",
                            index % width as usize,
                            index / width as usize,
                            region.left,
                            region.top,
                            region.right,
                            region.bottom,
                            actual[index],
                            expected[index]
                        );
                    }
                    let start = std::time::Instant::now();
                    for _ in 0..12 {
                        let saved = SaveDC(partial_dc);
                        IntersectClipRect(
                            partial_dc,
                            region.left,
                            region.top,
                            region.right,
                            region.bottom,
                        );
                        render(hwnd, &app, partial_dc, Some(region));
                        RestoreDC(partial_dc, saved);
                    }
                    let partial_time = start.elapsed();
                    let start = std::time::Instant::now();
                    for _ in 0..12 {
                        render(hwnd, &app, full_dc, None);
                    }
                    eprintln!(
                        "mode {mode}: 12 partial frames {partial_time:?}, 12 full frames {:?}",
                        start.elapsed()
                    );
                    SelectObject(partial_dc, partial_old);
                    SelectObject(full_dc, full_old);
                    DeleteObject(partial_bitmap);
                    DeleteObject(full_bitmap);
                    DeleteDC(partial_dc);
                    DeleteDC(full_dc);
                }
                DestroyWindow(hwnd);
                UnregisterClassW(class.as_ptr(), instance);
                return;
            }

            let capture = |hwnd: HWND, app: &mut App, filename: &str| {
                material(hwnd, app);
                app.acrylic = false;
                rebuild(hwnd, app);
                let mut bounds: RECT = zeroed();
                GetClientRect(hwnd, &mut bounds);
                let w = bounds.right;
                let h = bounds.bottom;
                let dc = CreateCompatibleDC(null_mut());
                let mut bits = null_mut();
                let info = BITMAPINFO {
                    bmiHeader: BITMAPINFOHEADER {
                        biSize: size_of::<BITMAPINFOHEADER>() as u32,
                        biWidth: w,
                        biHeight: -h,
                        biPlanes: 1,
                        biBitCount: 32,
                        biCompression: BI_RGB,
                        ..zeroed()
                    },
                    ..zeroed()
                };
                let dib = CreateDIBSection(dc, &info, DIB_RGB_COLORS, &mut bits, null_mut(), 0);
                let prev = SelectObject(dc, dib);
                render(hwnd, app, dc, None);
                let artwork_deadline =
                    std::time::Instant::now() + std::time::Duration::from_secs(5);
                while crate::drawing::artwork_preparation_pending() {
                    assert!(
                        std::time::Instant::now() < artwork_deadline,
                        "screenshot artwork preparation timed out"
                    );
                    std::thread::sleep(std::time::Duration::from_millis(1));
                    crate::drawing::poll_prepared_artwork();
                    render(hwnd, app, dc, None);
                }
                GdiFlush();
                let slice = std::slice::from_raw_parts(bits.cast::<u32>(), (w * h) as usize);
                let mut rgba = Vec::with_capacity((w * h * 4) as usize);
                for &px in slice {
                    let r = ((px >> 16) & 0xff) as u8;
                    let g = ((px >> 8) & 0xff) as u8;
                    let b = (px & 0xff) as u8;
                    rgba.extend_from_slice(&[r, g, b, 255]);
                }
                let img = image::RgbaImage::from_raw(w as u32, h as u32, rgba).unwrap();
                let path = std::path::Path::new(&out_dir).join(filename);
                img.save(&path).unwrap();
                if std::env::var_os("DEEZNATS_PLAYER_REVIEW").is_some() {
                    let footer = ui::player_height(&app.ui, w) as u32;
                    let crop =
                        image::imageops::crop_imm(&img, 0, h as u32 - footer, w as u32, footer)
                            .to_image();
                    crop.save(path.with_file_name(format!("footer-{filename}")))
                        .unwrap();
                }
                SelectObject(dc, prev);
                DeleteObject(dib);
                DeleteDC(dc);
            };

            if std::env::var_os("DEEZNATS_DEVICE_SIDEBAR_REVIEW").is_some() {
                app.ui.set_language("en");
                app.ui.provider = crate::catalog::Provider::Spotify;
                app.snapshot.provider = crate::catalog::Provider::Spotify;
                app.snapshot.state = "Playing".into();
                app.ui.view = View::Home;
                app.ui.page = Some(Arc::new(crate::catalog::Page::default()));
                app.ui.queue_sidebar_open = false;
                app.ui.device_menu_open = true;
                app.ui.jam_enabled = false;
                app.ui.spotify_devices = vec![
                    serde_json::json!({"id":"local","name":"DeezNats","type":"Computer","is_active":true}),
                    serde_json::json!({"id":"web","name":"Web Player (Microsoft Edge)","type":"Computer"}),
                ];
                app.ui.spotify_jam = Some(crate::spotify::jam::Session {
                    id: "fixture-session".into(),
                    active: true,
                    owner: true,
                    invite_uri: Some("spotify:socialsession:fixture".into()),
                    members: vec![
                        crate::spotify::jam::Member {
                            id: "host".into(),
                            name: "You".into(),
                            owner: true,
                        },
                        crate::spotify::jam::Member {
                            id: "guest".into(),
                            name: "Guest".into(),
                            owner: false,
                        },
                    ],
                    ..Default::default()
                });
                for (width, height, name) in [
                    (1200, 900, "spotify-sidebar-host-wide.png"),
                    (588, 1050, "spotify-sidebar-host-narrow.png"),
                ] {
                    SetWindowPos(
                        hwnd,
                        null_mut(),
                        0,
                        0,
                        width,
                        height,
                        SWP_NOZORDER | SWP_NOMOVE,
                    );
                    capture(hwnd, &mut app, name);
                }
                SetWindowPos(hwnd, null_mut(), 0, 0, 1200, 900, SWP_NOZORDER | SWP_NOMOVE);
                app.ui.spotify_jam.as_mut().unwrap().owner = false;
                capture(hwnd, &mut app, "spotify-sidebar-guest.png");
                app.ui.spotify_jam = None;
                capture(hwnd, &mut app, "spotify-sidebar-start.png");
                app.ui.set_language("tr");
                capture(hwnd, &mut app, "spotify-sidebar-turkish.png");
                DestroyWindow(hwnd);
                UnregisterClassW(class.as_ptr(), instance);
                return;
            }
            if std::env::var_os("DEEZNATS_JAM_REVIEW").is_some() {
                app.ui.set_language("en");
                app.ui.provider = crate::catalog::Provider::Spotify;
                app.snapshot.provider = crate::catalog::Provider::Spotify;
                app.ui.view = View::Settings;
                app.ui.tab = 5;
                app.ui.scroll = 0;
                app.ui.queue_sidebar_open = false;
                app.ui.jam_enabled = false;
                app.ui.spotify_jam = Some(crate::spotify::jam::Session {
                    id: "fixture-session".into(),
                    active: true,
                    owner: true,
                    invite_uri: Some("spotify:socialsession:fixture".into()),
                    members: vec![
                        crate::spotify::jam::Member {
                            id: "host".into(),
                            name: "You (host)".into(),
                            owner: true,
                        },
                        crate::spotify::jam::Member {
                            id: "guest".into(),
                            name: "Guest".into(),
                            owner: false,
                        },
                    ],
                    ..Default::default()
                });
                app.ui.spotify_jam_status = "Jam is active.".into();
                for (width, height, name) in [
                    (1200, 900, "spotify-jam-wide.png"),
                    (588, 1050, "spotify-jam-narrow.png"),
                ] {
                    SetWindowPos(
                        hwnd,
                        null_mut(),
                        0,
                        0,
                        width,
                        height,
                        SWP_NOZORDER | SWP_NOMOVE,
                    );
                    capture(hwnd, &mut app, name);
                }
                DestroyWindow(hwnd);
                UnregisterClassW(class.as_ptr(), instance);
                return;
            }
            if std::env::var_os("DEEZNATS_SPOTIFY_REVIEW").is_some() {
                app.ui.set_language("en");
                app.ui.provider = crate::catalog::Provider::Spotify;
                app.snapshot.provider = crate::catalog::Provider::Spotify;
                app.ui.view = View::Account;
                app.ui.queue_sidebar_open = false;
                app.ui.jam_enabled = false;
                for (width, name) in [
                    (1200, "spotify-signin-wide.png"),
                    (588, "spotify-signin-narrow.png"),
                ] {
                    SetWindowPos(
                        hwnd,
                        null_mut(),
                        0,
                        0,
                        width,
                        900,
                        SWP_NOZORDER | SWP_NOMOVE,
                    );
                    capture(hwnd, &mut app, name);
                }
                SetWindowPos(hwnd, null_mut(), 0, 0, 1200, 900, SWP_NOZORDER | SWP_NOMOVE);
                app.ui.view = View::Favorites;
                app.snapshot.connected = true;
                app.ui.account = serde_json::json!({"displayName":"Spotify fixture"});
                app.ui.page = Some(Arc::new(crate::catalog::Page {
                    title: "Liked songs".into(),
                    rows: (0..8)
                        .map(|index| crate::catalog::Entry {
                            id: format!("spotify:track:{index:022}"),
                            title: format!("A song from your Spotify library {}", index + 1),
                            subtitle: "Artist name".into(),
                            album: "Album name".into(),
                            kind: "track".into(),
                            duration: 205000,
                            ..Default::default()
                        })
                        .collect(),
                    ..Default::default()
                }));
                app.ui.device_menu_open = true;
                app.ui.spotify_devices = vec![
                    serde_json::json!({"id":"local","name":"DeezNats","type":"Computer","is_active":true,"is_restricted":false,"volume_percent":70}),
                    serde_json::json!({"id":"speaker","name":"Living room speaker with a longer device name","type":"Speaker","is_active":false,"is_restricted":false,"volume_percent":40}),
                ];
                capture(hwnd, &mut app, "spotify-library-devices.png");
                app.ui.device_menu_open = false;
                app.ui.view = View::TrackInfo;
                let mut track = app.ui.page.as_ref().unwrap().rows[0].clone();
                track.album_id = "spotify:album:2222222222222222222222".into();
                track.artist_id = "spotify:artist:3333333333333333333333".into();
                app.ui.track_info = Some((
                    track,
                    serde_json::json!({"playCount":1234567,"explicit":true}),
                    serde_json::json!({"followers":12345,"monthlyListeners":567890}),
                ));
                SetWindowPos(hwnd, null_mut(), 0, 0, 588, 1050, SWP_NOZORDER | SWP_NOMOVE);
                capture(hwnd, &mut app, "spotify-trackinfo-narrow.png");
                DestroyWindow(hwnd);
                UnregisterClassW(class.as_ptr(), instance);
                return;
            }
            if std::env::var_os("DEEZNATS_STYLE_MIX_REVIEW").is_some() {
                app.ui.queue_sidebar_open = false;
                app.ui.set_language("en");
                app.ui.page = Some(Arc::new(crate::catalog::Page {
                    title: "Flow".into(),
                    ..Default::default()
                }));
                app.ui.view = View::Flow;
                capture(hwnd, &mut app, "flow-style-mix-button.png");
                app.ui.view = View::StyleMix;
                app.ui.mix_region = usize::MAX;
                app.ui.mix_genre = usize::MAX;
                app.ui.mix_genres = None;
                app.ui.page = Some(Arc::new(crate::catalog::Page {
                    title: "Style Mix".into(),
                    ..Default::default()
                }));
                capture(hwnd, &mut app, "style-mix-languages.png");
                app.ui.mix_region = 1;
                capture(hwnd, &mut app, "style-mix-loading.png");
                app.ui.mix_genres = Some(
                    serde_json::json!({"region":1,"genres":[{"id":1,"title":"Rock"},{"id":24,"title":"Indie Rock"}]}),
                );
                capture(hwnd, &mut app, "style-mix.png");
                DestroyWindow(hwnd);
                UnregisterClassW(class.as_ptr(), instance);
                return;
            }
            if std::env::var_os("DEEZNATS_QUEUE_REVIEW").is_some() {
                app.ui.view = View::Home;
                app.ui.queue_sidebar_open = true;
                app.ui.queue = (0..8).map(|i| crate::catalog::Entry {
                    id: i.to_string(), title: "A long track title with several words".into(),
                    subtitle: "An artist with a long credit".into(),
                    raw: serde_json::json!({"added_by":format!("Çağrı {} 日本語", "W".repeat(54))}),
                    ..Default::default()
                }).collect();
                app.ui.now = app.ui.queue[0].clone();
                for (w, h) in [(800, 600), (1200, 800), (1600, 1000)] {
                    SetWindowPos(hwnd, null_mut(), 0, 0, w, h, SWP_NOMOVE | SWP_NOZORDER);
                    app.ui.queue_sidebar_open = true;
                    app.ui.queue_scroll = 0;
                    capture(hwnd, &mut app, &format!("queue-{w}.png"));
                    app.ui.queue_scroll = app.scene.queue_scroll_max;
                    capture(hwnd, &mut app, &format!("queue-{w}-last.png"));
                }
                DestroyWindow(hwnd);
                UnregisterClassW(class.as_ptr(), instance);
                return;
            }
            if std::env::var_os("DEEZNATS_PLAYER_REVIEW").is_some() {
                app.ui.view = View::Home;
                app.ui.queue_sidebar_open = false;
                app.ui.tab = 0;
                app.ui.surface_opacity = 100;
                app.ui.light = false;
                app.ui.solid = true;
                app.ui.background_mode = 2;
                app.ui.now = crate::catalog::Entry {
                    id: "fixture".into(),
                    kind: "track".into(),
                    title: "rodeo dr".into(),
                    subtitle: "Gunna".into(),
                    artwork: "player-cover".into(),
                    duration: 176000,
                    raw: serde_json::json!({"explicit":true}),
                    ..Default::default()
                };
                let pixels = (0..64 * 64)
                    .map(|i| {
                        let x = i % 64;
                        let y = i / 64;
                        0xff000000 | ((50 + x * 3) as u32) << 16 | ((80 + y * 2) as u32) << 8 | 45
                    })
                    .collect();
                app.images.insert(
                    "player-cover".into(),
                    Arc::new(crate::catalog::Bitmap::new(64, 64, pixels)),
                );
                app.snapshot.now = app.ui.now.clone();
                app.snapshot.state = "Playing".into();
                app.snapshot.position = 9000;
                app.playback_clock
                    .sample(9000, std::time::Instant::now(), false);
                app.snapshot.duration = 176000;
                app.ui.duration = 176000;
                app.ui.position = 9000;
                for (width, layout, name) in [
                    (1491, 0, "player-normal.png"),
                    (1500, 1, "player-compact.png"),
                    (588, 0, "player-narrow.png"),
                ] {
                    app.ui.player_layout = layout;
                    app.palette_state.replace(PaletteState::default());
                    SetWindowPos(
                        hwnd,
                        null_mut(),
                        0,
                        0,
                        width + 16,
                        500,
                        SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE,
                    );
                    capture(hwnd, &mut app, name);
                }
                let original_volume = app.volume;
                activate(hwnd, &mut app, Action::MuteVolume);
                assert_eq!(app.volume, 0);
                assert_eq!(SendMessageW(app.edits[&VOLUME], WM_USER, 0, 0), 0);
                activate(hwnd, &mut app, Action::MuteVolume);
                assert_eq!(app.volume, original_volume);
                assert_eq!(
                    SendMessageW(app.edits[&VOLUME], WM_USER, 0, 0),
                    original_volume as isize
                );
                DestroyWindow(hwnd);
                UnregisterClassW(class.as_ptr(), instance);
                return;
            }
            if std::env::var_os("DEEZNATS_MAIN_REVIEW").is_some() {
                app.ui.apply_solid_theme(8);
                app.ui.background_mode = 1;
                app.ui.solid = false;
                app.ui.blur = 20;
                app.ui.light_opacity = 40;
                app.ui.surface_opacity = 40;
                app.ui.now = crate::catalog::Entry {
                    id: "review".into(),
                    kind: "track".into(),
                    title:
                        "A complete long title that scrolls only when it reaches the lyric column"
                            .into(),
                    subtitle: "Jessie Reyez and a deliberately long artist credit".into(),
                    artist_id: "review-artist".into(),
                    artwork: "fixture-cover".into(),
                    duration: 220000,
                    ..Default::default()
                };
                let pixels = (0..256 * 256)
                    .map(|i| {
                        let x = i % 256;
                        let y = i / 256;
                        0xff000000 | ((x as u32) << 16) | ((y as u32) << 8) | 100
                    })
                    .collect();
                let cover = Arc::new(crate::catalog::Bitmap::new(256, 256, pixels));
                app.images.insert("fixture-cover".into(), cover.clone());
                app.images
                    .insert("background:fixture-cover".into(), cover.clone());
                app.background = Some(Arc::new(crate::appearance::prepare(&cover, 20)));
                app.background_source = "background:fixture-cover:20".into();
                app.background_requested = true;
                app.background_transition = BackgroundTransition::Complete;
                app.snapshot.now = app.ui.now.clone();
                app.snapshot.connected = true;
                let rows: Vec<_> = (0..5000)
                    .map(|i| {
                        let mut row = app.ui.now.clone();
                        row.id = i.to_string();
                        row.title = format!("Track {i}");
                        row
                    })
                    .collect();
                app.ui.view = View::Album;
                app.ui.page = Some(Arc::new(crate::catalog::Page {
                    title: "Large playlist".into(),
                    rows,
                    ..Default::default()
                }));
                app.palette_state.replace(PaletteState::default());
                capture(hwnd, &mut app, "main-opacity40.png");
                settle_scroll(&mut app, 56000);
                rebuild(hwnd, &mut app);
                assert_eq!(app.scroll_target, 56000);
                assert_eq!(app.scroll_position, 56000.);
                let start = std::time::Instant::now();
                for step in 0..120 {
                    let position = 56000 + step * 8;
                    app.scroll_target = position;
                    scroll_to(hwnd, &mut app, position);
                }
                let scroll_elapsed = start.elapsed().as_secs_f64() * 1000.;
                capture(hwnd, &mut app, "large-list-scrolled.png");
                app.ui.surface_opacity = 0;
                capture(hwnd, &mut app, "main-opacity0.png");
                app.ui.surface_opacity = 100;
                capture(hwnd, &mut app, "main-opacity100.png");
                app.ui.surface_opacity = 40;
                app.render_suspended = true;
                app.layout_dirty = false;
                let old_buttons = app.scene.buttons.len();
                rebuild(hwnd, &mut app);
                assert!(app.layout_dirty);
                assert_eq!(app.scene.buttons.len(), old_buttons);
                app.render_suspended = false;
                rebuild(hwnd, &mut app);
                assert!(!app.layout_dirty);
                app.ui.view = View::Lyrics;
                settle_scroll(&mut app, 0);
                app.ui.page = Some(Arc::new(crate::catalog::Page {
                    entity: Some(crate::catalog::Entry {
                        kind: "artist".into(),
                        ..Default::default()
                    }),
                    ..Default::default()
                }));
                app.ui.lyrics_provider = "Apple Music".into();
                app.ui.rich_lyrics = (0..80)
                    .map(|i| crate::lyrics::LyricLine {
                        text: format!("Entire readable lyric line {i}, without truncation."),
                        ..Default::default()
                    })
                    .collect();
                app.ui.lyrics.clear();
                app.ui.position = 0;
                app.ui.duration = 220000;
                app.snapshot.state = "Playing".into();
                if std::env::var_os("DEEZNATS_LYRIC_BENCH_LARGE").is_some() {
                    SetWindowPos(
                        hwnd,
                        null_mut(),
                        0,
                        0,
                        2560,
                        1440,
                        SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE,
                    );
                }
                for (style, name) in [
                    (0, "lyrics-wave.png"),
                    (1, "lyrics-bars.png"),
                    (2, "lyrics-trap-nation.png"),
                ] {
                    app.ui.visualizer_style = style;
                    app.visualizer.fixture_levels(std::array::from_fn(|i| {
                        0.22 + (i as f32 * 0.45).sin().abs() * 0.55
                    }));
                    capture(hwnd, &mut app, name);
                }
                if std::env::var_os("DEEZNATS_LYRIC_MEMORY_BENCH").is_some() {
                    let dc = GetDC(hwnd);
                    let mut times = Vec::new();
                    let mut peaks = (0, 0, 0);
                    for step in 0..40 {
                        settle_scroll(&mut app, step * 32);
                        rebuild(hwnd, &mut app);
                        for _ in 0..4 {
                            let start = std::time::Instant::now();
                            render(hwnd, &app, dc, None);
                            GdiFlush();
                            times.push(start.elapsed().as_secs_f64() * 1000.0);
                            let bytes = crate::drawing::lyric_raster_bytes();
                            peaks.0 = peaks.0.max(bytes.0);
                            peaks.1 = peaks.1.max(bytes.1);
                            peaks.2 = peaks.2.max(bytes.2);
                        }
                    }
                    ReleaseDC(hwnd, dc);
                    times.sort_by(f64::total_cmp);
                    std::fs::write(
                    std::path::Path::new(&out_dir).join("lyric-memory-bench.json"),
                    serde_json::json!({"frames":times.len(),"median_ms":times[times.len()/2],"p95_ms":times[times.len()*95/100],"max_ms":times[times.len()-1],"shadow_bytes":peaks.0,"raster_bytes":peaks.1,"blur_bytes":peaks.2}).to_string(),
                ).unwrap();
                    DestroyWindow(hwnd);
                    UnregisterClassW(class.as_ptr(), instance);
                    return;
                }
                assert_eq!(app.ui.scroll, 0);
                assert_eq!(
                    app.scene
                        .labels
                        .iter()
                        .find(|label| label.text.starts_with("Entire readable lyric line 0,"))
                        .unwrap()
                        .rect
                        .y,
                    40
                );
                assert!(app.scene.panels.iter().any(|p| p.kind == 6));
                let dc = GetDC(hwnd);
                let start = std::time::Instant::now();
                for _ in 0..30 {
                    render(hwnd, &app, dc, None);
                }
                let render_elapsed = start.elapsed().as_secs_f64() * 1000.;
                ReleaseDC(hwnd, dc);
                std::fs::write(std::path::Path::new(&out_dir).join("performance.json"),serde_json::json!({"scroll_layout_ms_per_step":scroll_elapsed/120.,"trap_render_ms_per_frame":render_elapsed/30.,"playlist_rows":5000}).to_string()).unwrap();
                app.ui.lyrics_cover_mode = true;
                app.ui.lyrics_cover_mix = None;
                app.ui.lyrics_volume_open = true;
                capture(hwnd, &mut app, "lyrics-centered.png");
                let cover = app.scene.lyrics_cover.unwrap();
                let (logical_width, _) = size(hwnd, &app);
                assert!((cover.x + cover.w / 2 - logical_width / 2).abs() <= 1);
                let volume = app.edits[&VOLUME];
                let mut volume_bounds: RECT = zeroed();
                GetClientRect(volume, &mut volume_bounds);
                let point =
                    |x: i32, y: i32| ((x as u32 & 0xffff) | ((y as u32 & 0xffff) << 16)) as isize;
                SendMessageW(volume, WM_LBUTTONDOWN, 0, point(13, 5));
                SendMessageW(volume, WM_LBUTTONUP, 0, point(13, 5));
                assert_eq!(app.volume, 100);
                SendMessageW(
                    volume,
                    WM_LBUTTONDOWN,
                    0,
                    point(13, volume_bounds.bottom - 5),
                );
                SendMessageW(volume, WM_LBUTTONUP, 0, point(13, volume_bounds.bottom - 5));
                assert_eq!(app.volume, 0);
                SendMessageW(volume, WM_MOUSEWHEEL, (120usize) << 16, 0);
                assert_eq!(app.volume, 2);
                SendMessageW(volume, WM_KEYDOWN, VK_UP as usize, 0);
                assert_eq!(app.volume, 4);
                SendMessageW(volume, WM_KEYDOWN, VK_RIGHT as usize, 0);
                assert_eq!(app.volume, 6);
                DestroyWindow(hwnd);
                UnregisterClassW(class.as_ptr(), instance);
                return;
            }
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

            // Verify all 18 SOLID_THEMES readability
            for idx in 0..ui::SOLID_THEMES.len() {
                app.ui.background_mode = 3;
                app.ui.solid = true;
                app.ui.apply_solid_theme(idx);
                let pal = app.palette();
                let name = ui::SOLID_THEMES[idx].0;
                for (surf_idx, surf_name) in [(0, "Background"), (1, "Panel"), (2, "Raised")] {
                    let cr_text = contrast(pal[4], pal[surf_idx]);
                    assert!(
                        cr_text >= 6.0,
                        "Theme {name} Text on {surf_name} contrast too low: {cr_text:.2}"
                    );
                }
                let cr_muted_panel = contrast(pal[5], pal[1]);
                assert!(
                    cr_muted_panel >= 4.5,
                    "Theme {name} Muted on Panel contrast too low: {cr_muted_panel:.2}"
                );
                let accent = app.ui.color("Accent", 0);
                let accent_text = app.ui.color("AccentText", 0x00ffffff);
                let cr_accent = contrast(accent, accent_text);
                assert!(
                    cr_accent >= 4.5,
                    "Theme {name} AccentText on Accent contrast too low: {cr_accent:.2}"
                );
            }

            // Capture all 6 Light Solid Themes
            app.ui.view = View::Settings;
            app.ui.tab = 2;
            app.ui.background_mode = 3;
            app.ui.solid = true;
            for (idx, slug) in [
                (12, "after_solid_porcelain.png"),
                (13, "after_solid_linen.png"),
                (14, "after_solid_sage.png"),
                (15, "after_solid_rosewater.png"),
                (16, "after_solid_sky.png"),
                (17, "after_solid_lavender.png"),
            ] {
                app.ui.apply_solid_theme(idx);
                capture(hwnd, &mut app, slug);
            }

            // Capture Palette mode in Light Mode with Preset 0 (Rose Violet) & Preset 3 (Sunset)
            app.ui.background_mode = 0;
            app.ui.solid = false;
            app.ui.apply_solid_theme(12);
            activate(hwnd, &mut app, Action::Preset(0));
            let pal = app.palette();
            assert!(
                contrast(pal[4], pal[2]) >= 8.0,
                "Light preset 0 Text on Raised contrast too low: {:.2}",
                contrast(pal[4], pal[2])
            );
            capture(hwnd, &mut app, "after_palette_light_preset0.png");

            activate(hwnd, &mut app, Action::Preset(3));
            capture(hwnd, &mut app, "after_palette_light_preset3.png");

            // Capture CustomColors view in Light Mode
            app.ui.view = View::CustomColors;
            capture(hwnd, &mut app, "after_custom_colors_light.png");

            // Capture the exact user scenario:
            // Dark Mode (light == false), Adaptive cover blur (mode == 1), 6% tint opacity,
            // playing track with a bright white/light cover art (Hotline Miami - Hydrogen)
            app.ui.view = View::Settings;
            app.ui.tab = 2;
            app.ui.light = false;
            app.ui.background_mode = 1;
            app.ui.light_opacity = 6;
            app.ui.surface_opacity = 80;
            let bright_artwork_key = "https://example.com/hydrogen.jpg".to_string();
            app.ui.now.artwork = bright_artwork_key.clone();
            let bright_art = Arc::new(crate::catalog::Bitmap::new(
                128,
                128,
                vec![0xffededed; 128 * 128],
            ));
            app.images
                .insert(bright_artwork_key.clone(), bright_art.clone());
            app.background = Some(bright_art);
            app.cover_luminance_cache.borrow_mut().take();
            assert!(
                app.is_effectively_light(),
                "Bright cover with 6% tint must be effectively light"
            );
            let pal_bright = app.palette();
            assert_eq!(
                pal_bright[4],
                colors(true)[4],
                "Text ink must adapt to deep slate on bright cover"
            );
            assert!(contrast(pal_bright[4], 0x00ededed) >= 10.0);
            capture(hwnd, &mut app, "after_adaptive_cover_bright_dark_mode.png");

            // Also capture Lyrics view with visualizer and karaoke on bright cover
            app.ui.view = View::Lyrics;
            app.ui.lyrics_visualizer = true;
            app.ui.karaoke_lyrics = true;
            capture(hwnd, &mut app, "after_lyrics_adaptive_bright.png");

            DestroyWindow(hwnd);
            UnregisterClassW(class.as_ptr(), instance);
        }
    }
    #[test]
    fn adaptive_contrast_and_transitions_guarantee_readability_across_all_modes() {
        let white_cover = Arc::new(crate::catalog::Bitmap::new(64, 64, vec![0xfff5f5f5; 4096]));
        assert!(crate::appearance::artwork_luminance(&white_cover) > 0.85);

        let dark_cover = Arc::new(crate::catalog::Bitmap::new(64, 64, vec![0xff121216; 4096]));
        assert!(crate::appearance::artwork_luminance(&dark_cover) < 0.15);

        assert_eq!(lerp_color(0x00000000, 0x00ffffff, 0.0), 0x00000000);
        assert_eq!(lerp_color(0x00000000, 0x00ffffff, 1.0), 0x00ffffff);
        let mid = lerp_color(0x00000000, 0x00646464, 0.5);
        assert!((mid & 255) >= 0x30 && (mid & 255) <= 0x45);
    }
}
