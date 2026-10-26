use crate::{model::*, ui::Ui};
use serde_json::Value;
pub fn content(u: &mut Ui, x: f32, y: f32, w: f32, bottom: f32) -> f32 {
    match u.s.view {
        View::Queue => queue(u, x, y, w),
        View::Discover => discover(u, x, y, w),
        View::Flow => flow(u, x, y, w),
        View::Search => search(u, x, y, w),
        View::Settings => settings(u, x, y, w),
        View::Lyrics => bottom,
    }
}
fn row(u: &mut Ui, item: &Value, index: Option<usize>, x: f32, y: f32, w: f32, current: bool) {
    if y + 72. < u.clip.y || y > u.clip.bottom() {
        return;
    }
    let r = Rect::new(x, y, w, 72.);
    u.r.round(
        r,
        12.,
        if current {
            u.t.raised.mix(u.t.accent, 0.2).alpha(u.alpha)
        } else {
            u.t.raised.alpha(u.alpha)
        },
        Some(if current { u.t.accent } else { u.t.border }),
    );
    let tx = x + 12.;
    let image_x = if index.is_some() { tx + 22. } else { tx };
    if let Some(i) = index {
        for dy in [24., 31., 38.] {
            for dx in [5., 10.] {
                u.r.round(
                    Rect::new(tx + dx, y + dy, 2., 2.),
                    1.,
                    u.t.muted
                        .alpha(if i > number(&u.s.playback, "queue_index") as usize {
                            1.
                        } else {
                            0.2
                        }),
                    None,
                );
            }
        }
    }
    u.photo(
        &text(item, "artwork"),
        Rect::new(image_x, y + 14., 44., 44.),
        8.,
    );
    let meta_x = image_x + 54.;
    let action_width = if index.is_some() {
        86.
    } else if w < 520. {
        92.
    } else {
        190.
    };
    let mw = (w - (meta_x - x) - action_width - 14.).max(20.);
    u.label(
        &text(item, "title"),
        Rect::new(meta_x, y + 12., mw, 24.),
        14.4,
        700,
        u.t.text,
        false,
    );
    let subtitle = text(item, "artist");
    let subtitle = if subtitle.is_empty() {
        text(item, "subtitle")
    } else {
        subtitle
    };
    let mut description = if duration(item) > 0. {
        format!("{subtitle} • {}", time(duration(item)))
    } else {
        subtitle
    };
    if index.is_some() {
        let added_by = text(item, "added_by");
        if !added_by.is_empty() {
            description.push_str(&format!(" • Added by {added_by}"));
        }
    }
    u.label(
        &description,
        Rect::new(meta_x, y + 38., mw, 20.),
        12.8,
        400,
        u.t.muted,
        false,
    );
    if let Some(index) = index {
        if current {
            let tw = u.r.measure(&text(item, "title"), 14.4, 700, mw, false).0;
            u.label(
                &u.s.tr("(Playing)"),
                Rect::new(meta_x + tw + 6., y + 12., 80., 24.),
                12.,
                400,
                u.t.accent,
                false,
            );
        }
        u.button(
            Rect::new(x + w - 92., y + 20., 80., 32.),
            if current { "Replay" } else { "Play" },
            if current { None } else { Some('\u{e768}') },
            Action::Jump(index),
            false,
            current,
        );
        u.hit(
            Rect::new(tx, y, (w - 110.).max(20.), 72.),
            Action::Jump(index),
            false,
        );
        return;
    }
    let kind = text(item, "kind");
    let playable = matches!(kind.as_str(), "track" | "episode") || kind.is_empty();
    if playable {
        let bx = x + w - action_width - 10.;
        if w >= 520. {
            u.button(
                Rect::new(bx, y + 20., 60., 32.),
                "Next",
                Some('\u{e710}'),
                Action::Insert(item.clone()),
                false,
                false,
            );
            u.button(
                Rect::new(bx + 66., y + 20., 78., 32.),
                "Queue",
                Some('\u{e710}'),
                Action::Append(item.clone()),
                false,
                false,
            );
            u.button(
                Rect::new(bx + 150., y + 20., 32., 32.),
                "",
                Some('\u{e768}'),
                if let Some(index) = index {
                    Action::Jump(index)
                } else {
                    Action::Play(item.clone())
                },
                false,
                true,
            );
        } else {
            u.button(
                Rect::new(bx, y + 20., 26., 32.),
                "",
                Some('\u{e710}'),
                Action::Insert(item.clone()),
                false,
                false,
            );
            u.button(
                Rect::new(bx + 31., y + 20., 26., 32.),
                "",
                Some('\u{e8fd}'),
                Action::Append(item.clone()),
                false,
                false,
            );
            u.button(
                Rect::new(bx + 62., y + 20., 30., 32.),
                "",
                Some('\u{e768}'),
                if let Some(index) = index {
                    Action::Jump(index)
                } else {
                    Action::Play(item.clone())
                },
                false,
                true,
            );
        }
    } else {
        u.button(
            Rect::new(x + w - 84., y + 20., 70., 32.),
            "Open",
            None,
            Action::Open(item.clone()),
            false,
            false,
        );
    }
    u.hit(
        Rect::new(meta_x, y, mw, 72.),
        if let Some(index) = index {
            Action::Jump(index)
        } else if playable {
            Action::Play(item.clone())
        } else {
            Action::Open(item.clone())
        },
        false,
    );
}
fn queue_description(item: &Value) -> String {
    let artist = text(item, "artist");
    let artist = if artist.is_empty() { text(item, "subtitle") } else { artist };
    let mut description = format!("{} · {}", artist, time(duration(item)));
    let name = text(item, "added_by");
    if !name.is_empty() { description.push_str(&format!("\nAdded by {name}")); }
    description
}
fn queue_row_height(u: &mut Ui, item: &Value, w: f32) -> f32 {
    let mw = (w - 108.).max(20.);
    let title = u.r.measure(&text(item, "title"), 14.4, 700, mw, true).1;
    let description = u.r.measure(&queue_description(item), 12.8, 400, mw, true).1;
    (title + description + 64.).max(110.)
}
fn queue_row(u: &mut Ui, item: &Value, index: usize, x: f32, y: f32, w: f32, h: f32, current: bool) {
    if y + h < u.clip.y || y > u.clip.bottom() { return; }
    u.r.round(Rect::new(x, y, w, h), 12., u.t.raised.alpha(u.alpha), Some(if current {u.t.accent} else {u.t.border}));
    u.photo(&text(item, "artwork"), Rect::new(x + 12., y + 12., 44., 44.), 8.);
    let mw = (w - 108.).max(20.);
    let title = text(item, "title");
    let th = u.r.measure(&title, 14.4, 700, mw, true).1;
    u.words(&title, Rect::new(x + 66., y + 10., mw, th + 2.), 14.4, 700, u.t.text);
    u.words(&queue_description(item), Rect::new(x + 66., y + 14. + th, mw, h - th - 58.), 12.8, 400, u.t.muted);
    u.button(Rect::new(x + w - 92., y + h - 40., 80., 30.), if current {"Replay"} else {"Play"}, None, Action::Jump(index), false, current);
    u.hit(Rect::new(x, y, w - 100., h), Action::Jump(index), false);
}
fn queue(u: &mut Ui, x: f32, y: f32, w: f32) -> f32 {
    let tracks = items(&u.s.playback, "queue").to_vec();
    let header = if w < 500. { 112. } else { 76. };
    let heights: Vec<f32> = tracks.iter().map(|track| queue_row_height(u, track, w - 40.)).collect();
    let h = header
        + if tracks.is_empty() {
            106.
        } else {
            heights.iter().map(|h| h + 10.).sum::<f32>() + 10.
        };
    u.card(Rect::new(x, y, w, h), 20.);
    u.section("Playback queue", x + 20., y + 20., w - 40.);
    let cb = Rect::new(x + w - 238., y + 22., 112., 28.);
    u.r.round(
        Rect::new(cb.x, cb.y + 4., 16., 16.),
        3.,
        if u.s.playback["autoplay"].as_bool().unwrap_or(false) {
            u.t.accent
        } else {
            u.t.raised
        },
        Some(u.t.border),
    );
    if u.s.playback["autoplay"].as_bool().unwrap_or(false) {
        u.r.icon(
            '\u{e73e}',
            Rect::new(cb.x, cb.y + 4., 16., 16.),
            12.,
            u.t.accent_text,
        );
    }
    u.label(
        &u.s.tr("Autoplay"),
        Rect::new(cb.x + 24., cb.y, 90., 24.),
        13.6,
        400,
        u.t.text,
        false,
    );
    u.hit(cb, Action::Autoplay, false);
    u.button(
        Rect::new(
            if w < 500. { x + 20. } else { x + w - 116. },
            if w < 500. { y + 72. } else { y + 20. },
            96.,
            32.,
        ),
        "Clear queue",
        None,
        Action::Clear,
        false,
        false,
    );
    if tracks.is_empty() {
        u.words(
            &u.s.tr("The queue is empty. Add songs from Search or Discover!"),
            Rect::new(x + 40., y + header + 20., w - 80., 70.),
            16.,
            400,
            u.t.muted,
        );
    }
    let current = number(&u.s.playback, "queue_index") as usize;
    let mut row_y = y + header;
    for (i, track) in tracks.iter().enumerate() {
        queue_row(
            u,
            track,
            i,
            x + 20.,
            row_y,
            w - 40.,
            heights[i],
            i == current,
        );
        row_y += heights[i] + 10.;
    }
    y + h
}
fn discover(u: &mut Ui, x: f32, y: f32, w: f32) -> f32 {
    let sections = items(&u.s.discover, "sections").to_vec();
    let mut cy = y;
    if sections.is_empty() {
        u.label(
            &u.s.tr(if u.s.loading.contains("discover") {
                "Loading Discover..."
            } else {
                "Nothing to discover right now."
            }),
            Rect::new(x, cy, w, 60.),
            16.,
            400,
            u.t.muted,
            true,
        );
        return cy + 60.;
    }
    for section in sections {
        let list = items(&section, "items");
        let cols = ((w - 32. + 12.) / 152.).floor().max(1.) as usize;
        let cw = (w - 32. - (cols - 1) as f32 * 12.) / cols as f32;
        let ch = cw + 62.;
        let rows = list.len().div_ceil(cols);
        let height = 60. + rows as f32 * (ch + 12.);
        u.card(Rect::new(x, cy, w, height), 18.);
        u.section(&text(&section, "title"), x + 16., cy + 16., w - 32.);
        for (i, item) in list.iter().enumerate() {
            let r = Rect::new(
                x + 16. + (i % cols) as f32 * (cw + 12.),
                cy + 56. + (i / cols) as f32 * (ch + 12.),
                cw,
                ch,
            );
            if r.bottom() < u.clip.y || r.y > u.clip.bottom() {
                continue;
            }
            u.r.round(r, 14., u.t.raised.alpha(u.alpha), Some(u.t.border));
            u.photo(
                &text(item, "artwork"),
                Rect::new(r.x + 10., r.y + 10., r.w - 20., r.w - 20.),
                10.,
            );
            u.label(
                &text(item, "title"),
                Rect::new(r.x + 10., r.y + r.w, r.w - 20., 23.),
                13.6,
                700,
                u.t.text,
                false,
            );
            u.label(
                &text(item, "subtitle"),
                Rect::new(r.x + 10., r.y + r.w + 25., r.w - 20., 18.),
                12.,
                400,
                u.t.muted,
                false,
            );
            u.hit(r, Action::Open(item.clone()), false);
        }
        cy += height + 16.;
    }
    cy
}
fn search(u: &mut Ui, x: f32, y: f32, w: f32) -> f32 {
    let mut cy = y + 20.;
    let data = u.s.search.clone();
    let mut result_height = 0.;
    for key in ["tracks", "albums", "playlists", "artists", "podcasts"] {
        if u.s.category == "all" || u.s.category == key {
            if !items(&data, key).is_empty() {
                result_height += 44. + items(&data, key).len() as f32 * 82.;
            }
        }
    }
    let filter_rows = if w < 650. { 2. } else { 1. };
    let header = 88. + filter_rows * 42.;
    let height = (header + result_height + 32.).max(186.);
    u.card(Rect::new(x, y, w, height), 20.);
    u.input(
        &u.s.search_text.clone(),
        "Songs, albums, playlists or artists...",
        Rect::new(x + 20., cy, w - 146., 48.),
        "search",
    );
    u.button(
        Rect::new(x + w - 116., cy, 96., 48.),
        "Search",
        None,
        Action::Search,
        true,
        false,
    );
    cy += 64.;
    let mut bx = x + 20.;
    for (key, label) in [
        ("all", "All"),
        ("tracks", "Songs"),
        ("albums", "Albums"),
        ("playlists", "Playlists"),
        ("artists", "Artists"),
        ("podcasts", "Podcasts"),
    ] {
        let label_t = u.s.tr(label);
        let bw = u.r.measure(&label_t, 13.6, 700, 500., false).0 + 28.;
        if bx + bw > x + w - 20. {
            bx = x + 20.;
            cy += 42.;
        }
        u.button(
            Rect::new(bx, cy, bw, 32.),
            label,
            None,
            Action::Category(key.into()),
            false,
            u.s.category == key,
        );
        bx += bw + 8.;
    }
    cy += 48.;
    if u.s.loading.contains("search") {
        u.label(
            &u.s.tr("Searching..."),
            Rect::new(x + 20., cy, w - 40., 40.),
            15.,
            400,
            u.t.muted,
            true,
        );
    }
    for (key, label) in [
        ("tracks", "Songs"),
        ("albums", "Albums"),
        ("playlists", "Playlists"),
        ("artists", "Artists"),
        ("podcasts", "Podcasts"),
    ] {
        if u.s.category != "all" && u.s.category != key {
            continue;
        }
        let list = items(&data, key);
        if list.is_empty() {
            continue;
        }
        u.section(label, x + 20., cy, w - 40.);
        cy += 40.;
        for item in list {
            row(u, item, None, x + 20., cy, w - 40., false);
            cy += 82.;
        }
    }
    y + height.max(cy - y + 20.)
}
fn flow(u: &mut Ui, x: f32, y: f32, w: f32) -> f32 {
    let data = u.s.flow.clone();
    let options = items(&data, "options");
    let cols = ((w - 56.) / 160.).floor().max(1.) as usize;
    let rows = options.len().div_ceil(cols);
    let hero_h = 300. + rows as f32 * 48.;
    u.card(Rect::new(x, y, w, hero_h), 20.);
    u.r.icon('\u{e8d6}', Rect::new(x, y + 24., w, 44.), 40., u.t.accent);
    u.label(
        &u.s.tr("Your personal music stream"),
        Rect::new(x + 20., y + 84., w - 40., 30.),
        22.4,
        800,
        u.t.text,
        true,
    );
    u.words(&u.s.tr("An endless mix of favorites and new discoveries. Choose your mood and let the music flow."),Rect::new(x+(w-380f32.min(w-56.))/2.,y+128.,380f32.min(w-56.),48.),14.4,400,u.t.muted);
    u.button(
        Rect::new(x + w / 2. - 170., y + 194., 160., 50.),
        "Start Flow",
        Some('\u{e768}'),
        Action::StartFlow,
        true,
        false,
    );
    u.button(
        Rect::new(x + w / 2., y + 194., 170., 50.),
        "Customize Flow",
        Some('\u{e713}'),
        Action::Tuner(true),
        false,
        false,
    );
    u.section("Flow moods", x + 28., y + 268., w - 56.);
    let pw = (w - 56. - (cols - 1) as f32 * 10.) / cols as f32;
    for (i, opt) in options.iter().enumerate() {
        let id = text(opt, "id");
        u.button(
            Rect::new(
                x + 28. + (i % cols) as f32 * (pw + 10.),
                y + 308. + (i / cols) as f32 * 48.,
                pw,
                36.,
            ),
            &text(opt, "title"),
            Some('\u{e8d6}'),
            Action::FlowSelect(id.clone()),
            false,
            id == u.s.flow_id,
        );
    }
    let mut cy = y + hero_h + 16.;
    let expanded = u.s.focus == Some("genres") || u.s.tuner["expanded"].as_bool().unwrap_or(false);
    let genres = items(&u.s.tuner, "genres").to_vec();
    let filter = u.s.genre_filter.to_lowercase();
    let filtered: Vec<_> = genres
        .iter()
        .enumerate()
        .filter(|(_, g)| text(g, "title").to_lowercase().contains(&filter))
        .collect();
    let gc = ((w - 48.) / 150.).floor().max(1.) as usize;
    let gh = if expanded {
        214. + filtered.len().div_ceil(gc) as f32 * 50.
    } else {
        80.
    };
    u.card(Rect::new(x, cy, w, gh), 20.);
    u.section("Flow genres (Flow Tuner)", x + 24., cy + 20., w - 150.);
    u.button(
        Rect::new(x + w - 104., cy + 20., 80., 34.),
        if expanded { "Hide" } else { "Show" },
        None,
        Action::Tuner(!expanded),
        false,
        false,
    );
    if expanded {
        u.words(&u.s.tr("Choose the genres in your Flow. Your selection syncs directly with your Deezer account."),Rect::new(x+24.,cy+68.,w-48.,38.),13.6,400,u.t.muted);
        u.input(
            &u.s.genre_filter.clone(),
            "Filter genres (Rock, Rap, Pop, Jazz)...",
            Rect::new(x + 24., cy + 112., w - 48., 42.),
            "genres",
        );
        for (bx, label, a) in [
            (x + 24., "Select all", Action::AllGenres(true)),
            (x + 128., "Clear", Action::AllGenres(false)),
            (x + w - 104., "Save", Action::FlowSave),
        ] {
            u.button(
                Rect::new(bx, cy + 166., 80., 32.),
                label,
                None,
                a,
                false,
                false,
            );
        }
        let gw = (w - 48. - (gc - 1) as f32 * 10.) / gc as f32;
        for (k, (i, g)) in filtered.iter().enumerate() {
            u.button(
                Rect::new(
                    x + 24. + (k % gc) as f32 * (gw + 10.),
                    cy + 214. + (k / gc) as f32 * 50.,
                    gw,
                    38.,
                ),
                &text(g, "title"),
                None,
                Action::FlowGenre(*i),
                false,
                g["isEnabled"].as_bool().unwrap_or(false),
            );
        }
    }
    cy += gh + 16.;
    let tracks = items(&data, "tracks");
    let th = if tracks.is_empty() {
        100.
    } else {
        68. + tracks.len() as f32 * 82.
    };
    u.card(Rect::new(x, cy, w, th), 20.);
    u.section("Flow tracks", x + 20., cy + 20., w - 40.);
    if tracks.is_empty() {
        u.label(
            &u.s.tr("Start Flow to load recommended tracks."),
            Rect::new(x + 20., cy + 56., w - 40., 32.),
            14.4,
            400,
            u.t.muted,
            true,
        );
    }
    for (i, track) in tracks.iter().enumerate() {
        row(
            u,
            track,
            None,
            x + 20.,
            cy + 64. + i as f32 * 82.,
            w - 40.,
            false,
        );
    }
    cy + th
}
fn settings(u: &mut Ui, x: f32, y: f32, w: f32) -> f32 {
    u.card(Rect::new(x, y, w, 150.), 20.);
    u.label(
        &u.s.tr("Host address"),
        Rect::new(x + 28., y + 24., w - 56., 30.),
        24.,
        700,
        u.t.text,
        false,
    );
    let host = u.s.host_text.clone();
    u.input(
        &host,
        "http://host:3131",
        Rect::new(x + 28., y + 76., w - 184., 46.),
        "host",
    );
    u.button(
        Rect::new(x + w - 144., y + 76., 116., 46.),
        "Connect",
        None,
        Action::ConnectHost,
        true,
        false,
    );
    let y = y + 170.;
    let profiles = u.s.host_profiles.clone();
    let profile_title = match &u.s.active_host_profile {
        Some(name) => format!("Temporary profiles — {name}"),
        None => format!("Nickname: {}", u.s.prefs.nickname),
    };
    let title_h = u.r.measure(&profile_title, 20., 700, w - 56., true).1.max(28.);
    let extra = title_h - 28.;
    let profile_height = 164. + extra + profiles.len() as f32 * 72.;
    u.card(Rect::new(x, y, w, profile_height), 20.);
    u.words(
        &profile_title,
        Rect::new(x + 28., y + 20., w - 56., title_h + 2.),
        20., 700, u.t.text,
    );
    u.label(
        &u.s.tr("Save the host above with a nickname. Profiles clear when Connect closes."),
        Rect::new(x + 28., y + 56. + extra, w - 56., 32.),
        12., 400, u.t.muted, true,
    );
    let nickname = u.s.host_nickname.clone();
    u.input(&nickname, "Nickname", Rect::new(x + 28., y + 100. + extra, w - 184., 42.), "host_nickname");
    u.button(Rect::new(x + w - 144., y + 100. + extra, 116., 42.), "Save profile", None, Action::SaveHostProfile, true, false);
    for (index, (nickname, host)) in profiles.iter().enumerate() {
        let row_y = y + 160. + extra + index as f32 * 72.;
        u.label(nickname, Rect::new(x + 28., row_y, w - 252., 24.), 16., 600, u.t.text, false);
        u.label(host, Rect::new(x + 28., row_y + 26., w - 252., 20.), 12., 400, u.t.muted, false);
        u.button(Rect::new(x + w - 212., row_y, 88., 42.), "Connect", None, Action::ConnectHostProfile(index), false, false);
        u.button(Rect::new(x + w - 116., row_y, 88., 42.), "Remove", None, Action::RemoveHostProfile(index), false, false);
    }
    let y = y + profile_height + 20.;
    let visible = if u.s.prefs.expanded {
        u.s.themes.len()
    } else {
        u.s.themes.len().min(4)
    };
    let columns = ((w - 56. + 10.) / 150.).floor().max(1.) as usize;
    let rows = visible.div_ceil(columns);
    let height = 407.6 + rows as f32 * 103.2;
    u.card(Rect::new(x, y, w, height), 20.);
    u.label(
        &u.s.tr("Appearance"),
        Rect::new(x + 28., y + 28., w - 56., 30.),
        24.,
        700,
        u.t.text,
        false,
    );
    u.label(
        &u.s.tr("Appearance is saved on this device."),
        Rect::new(x + 28., y + 68.8, w - 56., 20.),
        13.6,
        400,
        u.t.muted,
        false,
    );
    u.label(
        &u.s.tr("Theme").to_uppercase(),
        Rect::new(x + 28., y + 114.4, w - 56., 20.),
        12.,
        600,
        u.t.muted,
        false,
    );
    let cw = (w - 56. - (columns - 1) as f32 * 10.) / columns as f32;
    let themes = u.s.themes.clone();
    for (i, t) in themes.iter().take(visible).enumerate() {
        let r = Rect::new(
            x + 28. + (i % columns) as f32 * (cw + 10.),
            y + 144.4 + (i / columns) as f32 * 103.2,
            cw,
            93.2,
        );
        u.r.round(
            r,
            9.,
            u.t.panel.alpha(u.alpha),
            Some(if t.id == u.s.prefs.theme {
                u.t.accent
            } else {
                u.t.border
            }),
        );
        u.r.round(
            Rect::new(r.x + 9., r.y + 9., r.w - 18., 48.),
            5.,
            t.bg,
            None,
        );
        u.r.round(
            Rect::new(r.x + 16., r.y + 20., (r.w - 40.) * 0.65, 30.),
            3.,
            t.panel,
            None,
        );
        u.r.round(
            Rect::new(r.x + r.w * 0.7, r.y + 38., r.w * 0.2, 12.),
            3.,
            t.accent,
            None,
        );
        u.label(
            &t.name,
            Rect::new(r.x + 9., r.y + 64., r.w - 18., 18.),
            12.5,
            400,
            u.t.text,
            false,
        );
        u.hit(r, Action::Theme(t.id), false);
    }
    let cy = y + 150.4 + rows as f32 * 103.2;
    u.button(
        Rect::new(x + w / 2. - 38.1, cy, 76.2, 29.6),
        if u.s.prefs.expanded {
            "See less"
        } else {
            "See more"
        },
        None,
        Action::MoreThemes,
        false,
        false,
    );
    u.label(
        &u.s.tr("Backpanel").to_uppercase(),
        Rect::new(x + 28., cy + 57.6, w - 56., 20.),
        12.,
        600,
        u.t.muted,
        false,
    );
    for (i, (label, value, a)) in [
        ("Opacity", u.s.prefs.opacity, Action::Opacity),
        ("Blur", u.s.prefs.blur, Action::Blur),
    ]
    .into_iter()
    .enumerate()
    {
        let sy = cy + 95.6 + i as f32 * 66.4;
        u.label(
            &u.s.tr(label),
            Rect::new(x + 28., sy, w - 56., 20.),
            13.6,
            400,
            u.t.text,
            false,
        );
        u.slider(Rect::new(x + 28., sy + 28., w - 110., 20.), value / 100., a);
        u.label(
            &format!("{}%", value as u32),
            Rect::new(x + w - 72., sy + 28., 44., 20.),
            12.8,
            400,
            u.t.muted,
            true,
        );
    }
    y + height + 40.
}

pub fn modal(u: &mut Ui, w: f32, h: f32) {
    let data = u.s.modal.clone().unwrap_or_default();
    let width = 760f32.min(w - 40.);
    let height = (h - 60.).max(160.);
    let x = (w - width) / 2.;
    let y = 30.;
    u.r.fill(Rect::new(0., 0., w, u.r.height), BLACK.alpha(0.75));
    u.clip = Rect::new(x, y, width, height);
    u.s.hits
        .retain(|hit| matches!(hit.action, Action::Tab(_)) && false);
    u.card(Rect::new(x, y, width, height), 20.);
    let title = text(&data, "title");
    let title = if title.is_empty() {
        text(&u.s.modal_item, "title")
    } else {
        title
    };
    u.photo(
        &text(&data, "artwork"),
        Rect::new(x + 24., y + 24., 64., 64.),
        12.,
    );
    u.label(
        &title,
        Rect::new(x + 104., y + 20., width - 176., 32.),
        20.,
        800,
        u.t.text,
        false,
    );
    u.button(
        Rect::new(x + 104., y + 60., 112., 32.),
        "Play all",
        Some('\u{e768}'),
        Action::PlayCollection,
        false,
        true,
    );
    u.button(
        Rect::new(x + width - 56., y + 24., 32., 32.),
        "",
        Some('\u{e711}'),
        Action::CloseModal,
        false,
        false,
    );
    let top = y + 112.;
    u.r.clip(Rect::new(x + 16., top, width - 32., height - 128.));
    u.clip = Rect::new(x + 16., top, width - 32., height - 128.);
    let mut cy = top - u.s.modal_scroll;
    for (key, label) in [
        ("tracks", "Popular tracks"),
        ("albums", "Albums"),
        ("artists", "Similar artists"),
    ] {
        let list = items(&data, key);
        if list.is_empty() {
            continue;
        }
        if text(&data, "kind") == "artist" || key != "tracks" {
            u.section(label, x + 24., cy, width - 48.);
            cy += 40.;
        }
        for item in list {
            row(u, item, None, x + 24., cy, width - 48., false);
            cy += 82.;
        }
    }
    if let Some(error) = data["error"].as_str() {
        u.words(
            error,
            Rect::new(x + 24., cy, width - 48., 80.),
            15.,
            400,
            u.t.muted,
        );
    }
    u.r.pop();
    u.s.content_height = (cy + u.s.modal_scroll - (y + height - 16.)).max(0.);
    u.clip = Rect::new(0., 0., w, u.r.height);
}

pub fn lyrics(u: &mut Ui, w: f32, top: f32, bottom: f32) {
    let playing = u.s.playing()
        || (u.s.listening
            && u.s.audio_ready
            && u.s.transport_at.elapsed().as_secs_f32() * 1000. < u.s.audio_delay);
    let dt = u.s.paint_dt;
    for i in 0..32 {
        let target = if playing { u.s.levels[i] } else { 0. };
        let tau = if target > u.s.front[i] { 0.045 } else { 0.13 };
        u.s.front[i] += (target - u.s.front[i]) * (1. - (-dt / tau).exp());
        u.s.rear[i] += (target - u.s.rear[i]) * (1. - (-dt / 0.21).exp());
    }
    let wave_h = (bottom - top) * if u.s.prefs.bars { 0.48 } else { 0.46 };
    let baseline = bottom;
    let band = w / 32.;
    if u.s.prefs.bars {
        for i in 0..32 {
            let height = u.s.front[i] * wave_h;
            let r = Rect::new(i as f32 * band + 2., baseline - height, band - 4., height);
            u.r.gradient(r, u.t.accent.alpha(0.65), u.t.accent.alpha(0.02));
            u.r.fill(Rect::new(r.x, r.y, r.w, 1.5), u.t.accent.alpha(0.9));
        }
    } else {
        for (levels, alpha) in [(&u.s.rear, 0.18), (&u.s.front, 0.38)] {
            let points: Vec<_> = (0..=128)
                .map(|i| {
                    let at = i as f32 / 128. * 31.;
                    let index = at.floor() as usize;
                    let t = at - index as f32;
                    let a = levels[index.saturating_sub(1)];
                    let b = levels[index.min(31)];
                    let c = levels[(index + 1).min(31)];
                    let d = levels[(index + 2).min(31)];
                    let level = (0.5
                        * ((2. * b)
                            + (-a + c) * t
                            + (2. * a - 5. * b + 4. * c - d) * t * t
                            + (-a + 3. * b - 3. * c + d) * t * t * t))
                        .clamp(0., 1.);
                    (i as f32 * w / 128., baseline - level * wave_h)
                })
                .collect();
            u.r.wave(&points, baseline, u.t.accent, alpha);
        }
    }

    let data = u.s.lyrics.clone();
    let mut title = text(&u.s.playback, "title");
    if title.is_empty() {
        title = "Lyrics".into()
    }
    let titlew = (w - 320.).max(140.);
    u.label(
        &title,
        Rect::new(24., top + 16., titlew, 30.),
        24.,
        700,
        WHITE,
        false,
    );
    u.label(
        &text(&data, "provider"),
        Rect::new(24., top + 48., titlew, 20.),
        12.,
        400,
        Color::hex("#c5c5ca"),
        false,
    );
    let bx = (w - 242.).max(titlew + 30.);
    u.button(
        Rect::new(bx, top + 16., 66., 32.),
        "Wave",
        None,
        Action::Visualizer(false),
        false,
        !u.s.prefs.bars,
    );
    u.button(
        Rect::new(bx + 74., top + 16., 66., 32.),
        "Bars",
        None,
        Action::Visualizer(true),
        false,
        u.s.prefs.bars,
    );
    u.button(
        Rect::new(bx + 148., top + 16., 70., 32.),
        "Close",
        None,
        Action::CloseLyrics,
        false,
        false,
    );
    let lt = top + 82.;
    let area = Rect::new(24., lt, w - 48., bottom - lt - 16.);
    u.r.clip(area);
    let old = u.clip;
    u.clip = area;
    let timed = data["timed"].as_bool().unwrap_or(false);
    let karaoke = data["karaoke"].as_bool().unwrap_or(true);
    let position = u.s.lyric_position();
    let lines = items(&data, "lines");
    let size = (w * 0.04).clamp(24., 42.);
    let maxw = 900f32.min(w - 72.);
    let lx = (w - maxw) / 2.;
    let mut active = None;
    let mut active_y = 0.;
    let mut cy = if timed {
        lt + u.r.height * 0.25
    } else {
        lt + 24.
    };
    let mut measured = Vec::new();
    let mut gaps = items(&data, "gaps")
        .iter()
        .filter_map(|g| Some((g[0].as_f64()? as f32, g[1].as_f64()? as f32)))
        .peekable();
    let mut gap_active = false;
    for (i, line) in lines.iter().enumerate() {
        let start = number(line, "start_ms");
        while gaps.peek().is_some_and(|g| g.0 <= start) {
            let (start, end) = gaps.next().unwrap();
            if position >= start && position < end {
                gap_active = true;
                active_y = cy;
                measured.push((None, "♪   ♪   ♪".into(), cy, 54.));
                cy += 74.;
            }
        }
        if timed && start <= position {
            active = Some(i);
        }
        let s = text(line, "text");
        let height = u.r.measure(&s, size, 800, maxw, true).1;
        measured.push((Some(i), s, cy, height));
        cy += height + 24.;
        if line["translation"].as_str().is_some() {
            cy +=
                u.r.measure(&text(line, "translation"), size * 0.55, 500, maxw, true)
                    .1
                    + 10.;
        }
    }
    if !gap_active {
        if let Some(i) = active {
            if let Some(v) = measured.iter().find(|v| v.0 == Some(i)) {
                active_y = v.2 + v.3 / 2.;
            }
        }
    }
    let focus = if gap_active { Some(usize::MAX) } else { active };
    if timed
        && focus != u.s.lyric_focus
        && u.s
            .lyric_manual_until
            .is_none_or(|t| std::time::Instant::now() > t)
    {
        u.s.target_scroll = (active_y - (lt + area.h / 2.)).max(0.);
        u.s.lyric_focus = focus;
    }
    let max = (cy + if timed { u.r.height * 0.25 } else { 24. } - area.bottom()).max(0.);
    u.s.content_height = max;
    for (index, s, y, height) in measured {
        let r = Rect::new(lx, y - u.s.scroll, maxw, height + 4.);
        if r.bottom() < lt || r.y > bottom {
            continue;
        }
        let current = !timed || (!gap_active && index == active);
        let c = if current {
            WHITE
        } else {
            Color::hex("#c5c5ca")
        };
        u.r.frosted_lyric(
            &s,
            r,
            if index.is_none() { 30. } else { size },
            c,
            timed && !current && index.is_some() && data["blur"].as_bool().unwrap_or(true),
        );
        if timed && current && karaoke {
            if let Some(i) = index {
                let line = &lines[i];
                let syllables = items(line, "syllables");
                let mut cursor = 0;
                let mut ranges = Vec::new();
                let mut valid = !syllables.is_empty();
                for word in syllables {
                    let fragment = text(word, "text");
                    let fragment = fragment.trim();
                    if fragment.is_empty() {
                        continue;
                    }
                    if let Some(at) = s[cursor..].find(fragment) {
                        let start = cursor + at;
                        let utf16_start = s[..start].encode_utf16().count() as u32;
                        let len = fragment.encode_utf16().count() as u32;
                        let begin = number(word, "start_ms");
                        let end = number(word, "end_ms").max(begin + 1.);
                        ranges.push((
                            utf16_start,
                            len,
                            ((position - begin) / (end - begin)).clamp(0., 1.),
                        ));
                        cursor = start + fragment.len();
                    } else {
                        valid = false;
                        break;
                    }
                }
                if valid {
                    for (start, len, fill) in ranges {
                        u.r.highlight_range(&s, r, size, start, len, fill);
                    }
                } else {
                    let start = number(line, "start_ms");
                    let end = if number(line, "end_ms") > start {
                        number(line, "end_ms")
                    } else {
                        lines
                            .get(i + 1)
                            .map(|l| number(l, "start_ms"))
                            .unwrap_or(number(&u.s.playback, "duration_ms"))
                            .max(start + 1.)
                    };
                    let f = ((position - start) / (end - start)).clamp(0., 1.);
                    u.r.text(&s, r, size, 800, Color::hex("#c5c5ca"), false, true);
                    let actual = u.r.measure(&s, size, 800, r.w, true).0;
                    u.r.clip(Rect::new(r.x, r.y, actual * f, r.h));
                    u.r.text(&s, r, size, 800, WHITE, false, true);
                    u.r.pop();
                }
            }
        }
        if let Some(i) = index {
            let trans = text(&lines[i], "translation");
            if !trans.is_empty() {
                u.words(
                    &trans,
                    Rect::new(r.x, r.bottom() + 10., r.w, size * 0.7 + 8.),
                    size * 0.55,
                    500,
                    Color::hex("#c5c5ca"),
                );
            }
        }
    }
    if lines.is_empty() {
        u.words(
            &u.s.tr(if u.s.loading.contains("lyrics") {
                "Loading lyrics..."
            } else if u.s.last_track.is_empty() {
                "Play a track to see its lyrics."
            } else {
                "No lyrics available for this track."
            }),
            Rect::new(lx, lt + 24., maxw, 60.),
            16.,
            400,
            WHITE,
        );
    }
    u.r.pop();
    u.clip = old;
    u.scroll_bar(lt, bottom - 16., w, max, u.s.scroll);
}
