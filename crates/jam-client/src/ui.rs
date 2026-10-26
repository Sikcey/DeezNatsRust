use crate::{model::*, render::Renderer};
pub struct Ui<'a> {
    pub r: &'a mut Renderer,
    pub s: &'a mut State,
    pub t: Theme,
    pub urls: Vec<String>,
    pub clip: Rect,
    pub alpha: f32,
}
impl<'a> Ui<'a> {
    pub fn label(&mut self, s: &str, r: Rect, size: f32, weight: u32, c: Color, center: bool) {
        self.r.text(s, r, size, weight, c, center, false)
    }
    pub fn words(&mut self, s: &str, r: Rect, size: f32, weight: u32, c: Color) {
        self.r.text(s, r, size, weight, c, false, true)
    }
    pub fn hit(&mut self, r: Rect, a: Action, slider: bool) {
        let x = r.x.max(self.clip.x);
        let y = r.y.max(self.clip.y);
        let right = (r.x + r.w).min(self.clip.x + self.clip.w);
        let bottom = r.bottom().min(self.clip.bottom());
        if right > x && bottom > y {
            self.s
                .hit(Rect::new(x, y, right - x, bottom - y), a, slider)
        }
    }
    pub fn card(&self, r: Rect, radius: f32) {
        self.r.shadow(r, radius, BLACK.alpha(0.4), 20.);
        self.r.round(
            r,
            radius,
            self.t.panel.alpha(self.alpha),
            Some(self.t.border),
        );
    }
    pub fn photo(&mut self, url: &str, r: Rect, radius: f32) {
        if r.bottom() < self.clip.y || r.y > self.clip.bottom() {
            return;
        }
        if let Some(image) = self.s.images.get(url) {
            self.r.image(url, image, r, radius, false, 1.);
        } else {
            self.r
                .round(r, radius, self.t.raised.alpha(self.alpha), None);
            self.r.icon('\u{e93c}', r, 32., self.t.muted);
            if !url.is_empty() {
                self.urls.push(url.into());
            }
        }
    }
    pub fn button(
        &mut self,
        r: Rect,
        label: &str,
        icon: Option<char>,
        a: Action,
        primary: bool,
        active: bool,
    ) {
        let hover = r.contains(self.s.mouse.0, self.s.mouse.1);
        let bg = if primary {
            self.t.accent
        } else if active {
            self.t.raised.mix(self.t.accent, 0.3).alpha(self.alpha)
        } else {
            self.t
                .raised
                .alpha((self.alpha + if hover { 0.1 } else { 0. }).min(1.))
        };
        if primary {
            self.r.shadow(r, r.h / 2., self.t.accent.alpha(0.3), 12.);
        }
        self.r.round(
            r,
            if r.h == r.w { r.h / 2. } else { 10. },
            bg,
            Some(if primary || active {
                self.t.accent
            } else {
                self.t.border
            }),
        );
        let color = if primary {
            self.t.accent_text
        } else {
            self.t.text
        };
        let label = self.s.tr(label);
        if label.is_empty() {
            if let Some(icon) = icon {
                self.r.icon(
                    icon,
                    r,
                    if matches!(
                        &a,
                        Action::Tab(View::Queue | View::Lyrics | View::Settings)
                            | Action::Shuffle
                            | Action::Repeat
                    ) {
                        21.12
                    } else {
                        r.h * 0.45
                    },
                    color,
                );
            }
        } else {
            let font = if matches!(&a, Action::StartFlow) {
                18.4
            } else if primary && r.h >= 48. {
                16.8
            } else if r.y < 59. {
                12.8
            } else if r.h <= 34. {
                12.
            } else {
                13.6
            };
            let weight = if primary {
                800
            } else if r.y < 59. {
                700
            } else {
                600
            };
            let width = self.r.measure(&label, font, weight, 1000., false).0;
            let icon_size = font * 1.1;
            let whole = width + if icon.is_some() { icon_size + 6. } else { 0. };
            let left = r.x + (r.w - whole) / 2.;
            if let Some(icon) = icon {
                self.r
                    .icon(icon, Rect::new(left, r.y, icon_size, r.h), icon_size, color);
            }
            self.label(
                &label,
                Rect::new(
                    left + if icon.is_some() { icon_size + 6. } else { 0. },
                    r.y,
                    width.min(r.w - 12.),
                    r.h,
                ),
                font,
                weight,
                color,
                false,
            );
        }
        self.hit(r, a, false);
    }
    pub fn slider(&mut self, r: Rect, value: f32, a: Action) {
        let y = r.y + r.h / 2.;
        self.r.round(
            Rect::new(r.x, y - 2., r.w, 4.),
            2.,
            self.t.muted.alpha(0.35),
            None,
        );
        self.r.round(
            Rect::new(r.x, y - 2., r.w * value.clamp(0., 1.), 4.),
            2.,
            self.t.accent,
            None,
        );
        self.r.round(
            Rect::new(r.x + r.w * value.clamp(0., 1.) - 6., y - 6., 12., 12.),
            6.,
            self.t.muted,
            Some(WHITE),
        );
        self.hit(r, a, true);
    }
    pub fn section(&mut self, title: &str, x: f32, y: f32, w: f32) {
        self.r
            .round(Rect::new(x, y + 5., 3., 18.), 1.5, self.t.accent, None);
        self.label(
            &self.s.tr(title),
            Rect::new(x + 12., y, w - 12., 28.),
            16.,
            800,
            self.t.text,
            false,
        );
    }
    pub fn input(&mut self, value: &str, placeholder: &str, r: Rect, kind: &'static str) {
        self.r.round(
            r,
            12.,
            self.t.raised.alpha(self.alpha),
            Some(if self.s.focus == Some(kind) {
                self.t.accent
            } else {
                self.t.border
            }),
        );
        let focused = self.s.focus == Some(kind);
        let empty = value.is_empty();
        let value = if empty && !focused {
            self.s.tr(placeholder)
        } else {
            value.to_owned()
        };
        self.label(
            &value,
            r.inset(12.),
            15.,
            400,
            if !empty {
                self.t.text
            } else {
                self.t.muted
            },
            false,
        );
        if focused {
            let w = self
                .r
                .measure(&value, 15., 400, r.w - 24., false)
                .0
                .min(r.w - 24.);
            self.r.line(
                r.x + 12. + w,
                r.y + 13.,
                r.x + 12. + w,
                r.bottom() - 13.,
                self.t.text,
                1.,
            );
        }
        self.hit(r, Action::Focus(kind), false);
    }
    pub fn scroll_bar(&mut self, top: f32, bottom: f32, w: f32, max: f32, position: f32) {
        if max <= 0. {
            return;
        }
        let height = bottom - top;
        let thumb = (height * height / (height + max)).max(28.);
        self.r.round(
            Rect::new(w - 8., top + (height - thumb) * position / max, 4., thumb),
            2.,
            self.t.muted.alpha(0.55),
            None,
        );
        self.hit(Rect::new(w - 13., top, 13., height), Action::Scroll, true);
    }
}
pub fn player_height(w: f32) -> f32 {
    if w <= 640. {
        114.
    } else {
        95.
    }
}
pub fn draw(r: &mut Renderer, s: &mut State) -> Vec<String> {
    let now = std::time::Instant::now();
    s.paint_dt = now
        .duration_since(s.last_paint)
        .as_secs_f32()
        .clamp(0.001, 0.1);
    s.last_paint = now;
    let (w, h) = (r.width, r.height);
    let t = s.theme();
    let alpha = s.prefs.opacity / 100.;
    r.begin(t.bg);
    let art = text(&s.playback, "artwork");
    if let Some(data) = s.images.get(&art) {
        r.background(&art, data, Rect::new(0., 0., w, h), s.prefs.blur);
        r.fill(Rect::new(0., 0., w, h), t.bg.alpha(alpha));
    }
    let mut u = Ui {
        r,
        s,
        t,
        urls: Vec::new(),
        clip: Rect::new(0., 0., w, h),
        alpha,
    };
    if validate_nickname(&u.s.prefs.nickname).is_err() {
        let width = (w - 48.).min(440.);
        let x = (w - width) / 2.;
        let y = ((h - 330.) / 2.).max(24.);
        u.card(Rect::new(x, y, width, 330.), 20.);
        u.words("Welcome to DeezNats Connect", Rect::new(x + 24., y + 24., width - 48., 70.), 24., 700, u.t.text);
        u.words("Choose a nickname so everyone can see who added each song.", Rect::new(x + 24., y + 100., width - 48., 60.), 15., 400, u.t.muted);
        let name = u.s.host_nickname.clone();
        u.input(&name, "Your nickname", Rect::new(x + 24., y + 166., width - 48., 42.), "host_nickname");
        let error = u.s.toast.clone();
        u.words(&error, Rect::new(x + 24., y + 216., width - 48., 54.), 13., 400, u.t.muted);
        u.button(Rect::new(x + 24., y + 278., width - 48., 34.), "Continue", None, Action::SaveNickname, true, false);
        let _ = u.r.end();
        return Vec::new();
    }
    if !art.is_empty() && !u.s.images.contains_key(&art) {
        u.urls.push(art)
    }
    let ph = player_height(w);
    header(&mut u, w);
    nav(&mut u, w);
    let top = 110.;
    let bottom = h - ph;
    u.r.clip(Rect::new(0., top, w, bottom - top));
    u.clip = Rect::new(0., top, w, bottom - top);
    if u.s.view == View::Lyrics {
        crate::pages::lyrics(&mut u, w, top, bottom);
    } else {
        let mw = w.min(800.);
        let x = (w - mw) / 2. + 16.;
        let width = mw - 32.;
        let y = top + 16. - u.s.scroll;
        let end = crate::pages::content(&mut u, x, y, width, bottom);
        u.s.content_height = (end + u.s.scroll + 16. - bottom).max(0.);
        u.scroll_bar(top, bottom, w, u.s.content_height, u.s.scroll);
    }
    u.r.pop();
    u.clip = Rect::new(0., 0., w, h);
    bottom_player(&mut u, w, h, ph);
    if u.s.modal.is_some() {
        crate::pages::modal(&mut u, w, h - ph);
    }
    if !u.s.toast.is_empty() && u.s.toast_at.elapsed().as_secs_f32() < 2.2 {
        let label = u.s.tr(&u.s.toast);
        let tw = u.r.measure(&label, 14., 600, w - 64., false).0 + 40.;
        let rect = Rect::new((w - tw) / 2., h - ph - 58., tw, 42.);
        u.r.round(rect, 12., u.t.panel, Some(u.t.border));
        u.label(&label, rect.inset(10.), 14., 600, u.t.text, true);
    }
    let _ = u.r.end();
    u.urls
}
fn header(u: &mut Ui, w: f32) {
    u.r.fill(Rect::new(0., 0., w, 59.), u.t.panel.alpha(u.alpha));
    u.r.line(0., 58.5, w, 58.5, u.t.border, 1.);
    u.r.icon('\u{e708}', Rect::new(18., 12., 24., 34.), 24., u.t.accent);
    u.label(
        "Deez Nats",
        Rect::new(48., 12., 170., 34.),
        20.,
        800,
        u.t.text,
        false,
    );
    let settings = Rect::new(w - 52., 12., 34., 34.);
    u.button(
        settings,
        "",
        Some('\u{e713}'),
        Action::Tab(View::Settings),
        false,
        u.s.view == View::Settings,
    );
    let lang = Rect::new(w - 144., 12., 82., 34.);
    u.r.round(lang, 8., u.t.raised.alpha(u.alpha), Some(u.t.border));
    u.label(
        if u.s.prefs.turkish {
            "T\u{00fc}rk\u{00e7}e"
        } else {
            "English"
        },
        Rect::new(lang.x + 7., lang.y, lang.w - 25., lang.h),
        12.8,
        400,
        u.t.text,
        false,
    );
    u.r.icon(
        '\u{e70d}',
        Rect::new(lang.x + lang.w - 18., lang.y, 12., lang.h),
        10.,
        u.t.text,
    );
    u.hit(lang, Action::Language, false);
    let lw = if w < 640. {
        34.
    } else {
        if u.s.listening {
            136.
        } else {
            110.
        }
    };
    u.button(
        Rect::new(lang.x - 10. - lw, 12., lw, 34.),
        if w < 640. {
            ""
        } else if u.s.listening {
            "Listening live"
        } else {
            "Listen live"
        },
        Some('\u{e767}'),
        Action::Listen,
        u.s.listening,
        false,
    );
}
fn nav(u: &mut Ui, w: f32) {
    let x = (w - w.min(800.)) / 2. + 16.;
    let mut left = x - u.s.nav_scroll;
    u.r.clip(Rect::new(x, 59., w - 2. * x, 51.));
    let old = u.clip;
    u.clip = Rect::new(x, 59., w - 2. * x, 51.);
    for tab in View::TABS {
        let label = u.s.tr(tab.label());
        let width = u.r.measure(&label, 13.6, 700, 500., false).0 + 56.;
        u.button(
            Rect::new(left, 71., width, 35.),
            tab.label(),
            Some(tab.icon()),
            Action::Tab(tab),
            false,
            u.s.view == tab,
        );
        left += width + 6.;
    }
    u.r.pop();
    u.clip = old;
}
fn bottom_player(u: &mut Ui, w: f32, h: f32, height: f32) {
    let y = h - height;
    u.r.fill(Rect::new(0., y, w, height), u.t.panel.alpha(u.alpha));
    u.r.line(0., y, w, y, u.t.border, 1.);
    let small = w <= 640.;
    let py = y + 10.;
    let cover = if small { 36. } else { 48. };
    let queuew = if small { 38. } else { 38. };
    let total = if small { 288. } else { 312. };
    let cx = if small {
        w - 12. - total
    } else {
        w - 212. - total
    };
    let info_w = (cx - 32. - cover).max(0.);
    if w > 360. {
        u.photo(
            &text(&u.s.playback, "artwork"),
            Rect::new(16., py, cover, cover),
            8.,
        );
        u.label(
            &text(&u.s.playback, "title"),
            Rect::new(26. + cover, py, info_w, 24.),
            14.4,
            700,
            u.t.text,
            false,
        );
        u.label(
            &text(&u.s.playback, "artist"),
            Rect::new(26. + cover, py + 24., info_w, 18.),
            12.,
            400,
            u.t.muted,
            false,
        );
    }
    let ctrl_y = py + if small { 3. } else { 5. };
    let gap = if small { 4. } else { 8. };
    let mut x = cx;
    for (icon, a, active, big) in [
        (
            '\u{e8fd}',
            Action::Tab(View::Queue),
            u.s.view == View::Queue,
            false,
        ),
        (
            '\u{ec71}',
            Action::Tab(View::Lyrics),
            u.s.view == View::Lyrics,
            false,
        ),
        (
            '\u{e8b1}',
            Action::Shuffle,
            u.s.playback["shuffle"].as_bool().unwrap_or(false),
            false,
        ),
        ('\u{e892}', Action::Transport("previous"), false, false),
        (
            if u.s.playing() {
                '\u{e769}'
            } else {
                '\u{e768}'
            },
            Action::Transport("toggle"),
            false,
            true,
        ),
        ('\u{e893}', Action::Transport("next"), false, false),
        (
            if number(&u.s.playback, "repeat") == 2. {
                '\u{e8ed}'
            } else {
                '\u{e8ee}'
            },
            Action::Repeat,
            number(&u.s.playback, "repeat") > 0.,
            false,
        ),
    ] {
        let size = if big {
            44.
        } else if matches!(a, Action::Shuffle | Action::Repeat) {
            34.
        } else {
            queuew
        };
        u.button(
            Rect::new(x, ctrl_y - if big { 3. } else { 0. }, size, size),
            "",
            Some(icon),
            a,
            big,
            active,
        );
        x += size + gap;
    }
    let volume_rect = if small {
        Rect::new((w - 272.).max(100.), py + 52., w.min(260.), 18.)
    } else {
        Rect::new(w - 196., py + 16., 180., 18.)
    };
    u.label(
        &u.s.tr("Volume"),
        Rect::new(volume_rect.x, volume_rect.y, 44., 18.),
        12.8,
        400,
        u.t.text,
        false,
    );
    u.slider(
        Rect::new(volume_rect.x + 50., volume_rect.y, volume_rect.w - 92., 18.),
        u.s.prefs.volume / 100.,
        Action::Volume,
    );
    u.label(
        &format!("{}%", u.s.prefs.volume as u32),
        Rect::new(volume_rect.x + volume_rect.w - 36., volume_rect.y, 36., 18.),
        12.8,
        400,
        u.t.text,
        false,
    );
    let sy = h - 26.;
    u.label(
        &time(u.s.position()),
        Rect::new(16., sy, 34., 16.),
        12.,
        400,
        u.t.muted,
        false,
    );
    let dur = number(&u.s.playback, "duration_ms");
    u.slider(
        Rect::new(62., sy, w - 124., 16.),
        if dur > 0. { u.s.position() / dur } else { 0. },
        Action::Seek,
    );
    u.label(
        &time(dur),
        Rect::new(w - 50., sy, 34., 16.),
        12.,
        400,
        u.t.muted,
        false,
    );
}
