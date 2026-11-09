//! UI structure mirrors the original WinForms shell and its in-app views.
//! Coordinates are device-independent; the HWND host applies the DPI scale.
mod track_info {
    use super::*;
    use serde_json::Value;

    fn missing(ui: &Ui, value: String) -> String {
        if value.trim().is_empty() {
            ui.t("Unavailable").into()
        } else {
            value
        }
    }

    fn date(ui: &Ui, raw: &str) -> String {
        let value = format_date(raw);
        let parts: Vec<_> = value.split('-').collect();
        if parts.len() != 3 {
            return value;
        }
        let (Ok(month), Ok(day)) = (parts[1].parse::<usize>(), parts[2].parse::<u32>()) else {
            return value;
        };
        let months = match ui.language.as_str() {
            "tr" => [
                "Oca", "Şub", "Mar", "Nis", "May", "Haz", "Tem", "Ağu", "Eyl", "Eki", "Kas", "Ara",
            ],
            "az" => [
                "Yan", "Fev", "Mar", "Apr", "May", "İyn", "İyl", "Avq", "Sen", "Okt", "Noy", "Dek",
            ],
            _ => [
                "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
            ],
        };
        if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
            return value;
        }
        format!("{day} {} {}", months[month - 1], parts[0])
    }

    fn count(ui: &Ui, number: u64) -> String {
        let digits = number.to_string();
        let separator = if matches!(ui.language.as_str(), "tr" | "az") {
            '.'
        } else {
            ','
        };
        let mut result = String::new();
        for (index, c) in digits.chars().enumerate() {
            if index > 0 && (digits.len() - index) % 3 == 0 {
                result.push(separator);
            }
            result.push(c);
        }
        result
    }

    fn number(value: &Value) -> Option<u64> {
        value.as_u64().or_else(|| value.as_str()?.parse().ok())
    }

    fn title_width(value: &str, narrow: bool) -> i32 {
        use windows_sys::Win32::Graphics::Gdi::*;
        unsafe {
            let dc = GetDC(std::ptr::null_mut());
            let face: Vec<u16> = "Segoe UI\0".encode_utf16().collect();
            let font = CreateFontW(
                if narrow { -20 } else { -29 },
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
                face.as_ptr(),
            );
            let old = SelectObject(dc, font);
            let text: Vec<_> = value.encode_utf16().collect();
            let mut size = windows_sys::Win32::Foundation::SIZE { cx: 0, cy: 0 };
            GetTextExtentPoint32W(dc, text.as_ptr(), text.len() as i32, &mut size);
            SelectObject(dc, old);
            DeleteObject(font);
            ReleaseDC(std::ptr::null_mut(), dc);
            size.cx
        }
    }

    fn strings(value: &Value) -> Vec<String> {
        if let Some(value) = value.as_str() {
            return (!value.trim().is_empty())
                .then(|| value.to_owned())
                .into_iter()
                .collect();
        }
        if let Some(values) = value.as_array() {
            return values.iter().flat_map(strings).collect();
        }
        if let Some(values) = value.get("data") {
            return strings(values);
        }
        let name = crate::catalog::string(value, &["name", "text", "title"]);
        (!name.is_empty()).then_some(name).into_iter().collect()
    }

    fn rows(s: &mut Scene, x: i32, y: i32, width: i32, values: &[(&str, String)]) {
        let key_w = (width / 3).clamp(70, 160).min((width - 48) / 2);
        for (index, (key, value)) in values.iter().enumerate() {
            let row_y = y + index as i32 * 36;
            if index % 2 == 1 {
                s.panel(x + 18, row_y, width - 36, 36, 32);
            }
            s.label_sized(x + 28, row_y, key_w, 36, *key, 22);
            s.label_sized(
                x + 32 + key_w,
                row_y,
                (width - key_w - 60).max(1),
                36,
                value,
                21,
            );
        }
    }

    pub(super) fn build(s: &mut Scene, ui: &Ui, x: i32, top: i32, cw: i32) {
        let (mut track, supplied, artist_meta) = ui
            .track_info
            .clone()
            .unwrap_or_else(|| (ui.now.clone(), ui.now.raw.clone(), Value::Null));
        let mut meta = if track.raw.is_object() {
            track.raw.clone()
        } else {
            serde_json::json!({})
        };
        if let Some(values) = supplied.as_object() {
            for (key, value) in values {
                if !value.is_null() && value.as_str() != Some("") {
                    meta[key] = value.clone();
                }
            }
        }
        if let Some(full) = crate::catalog::entry(&meta, "track") {
            if track.artwork.is_empty() {
                track.artwork = full.artwork;
            }
            if track.album.is_empty() {
                track.album = full.album;
            }
            if track.album_id.is_empty() {
                track.album_id = full.album_id;
            }
            if track.artist_id.is_empty() {
                track.artist_id = full.artist_id;
            }
            if track.subtitle.is_empty() {
                track.subtitle = full.subtitle;
            }
            if track.duration == 0 {
                track.duration = full.duration;
            }
        }
        track.raw = meta.clone();
        let narrow = cw < 760;
        let gap = 20;
        let art = if narrow { 112 } else { 152 };
        let text_x = x + art + 20;
        let text_w = (cw - art - 20).max(1);
        let release = crate::catalog::string(
            &meta,
            &["release_date", "releaseDate", "physical_release_date"],
        );
        let release = if release.is_empty() {
            crate::catalog::string(&meta["album"], &["release_date", "releaseDate"])
        } else {
            release
        };
        let released = date(ui, &release);
        let explicit = meta["explicit_lyrics"]
            .as_bool()
            .or_else(|| meta["explicit"].as_bool());
        let title = if track.title.is_empty() {
            ui.t("Unknown track")
        } else {
            &track.title
        };
        let title_w = (text_w - if explicit == Some(true) { 24 } else { 0 }).max(1);
        let title_size = if narrow { 20 } else { 28 };
        let title_lines = wrap_text(title, title_w, title_size);
        let title_lines: Vec<_> = title_lines.into_iter().take(2).collect();
        let header_h = if narrow {
            214
        } else if title_lines.len() > 1 {
            190
        } else {
            156
        };
        let cy = top + header_h + gap;
        let col_w = if narrow { cw } else { (cw - gap) / 2 };
        let right_x = if narrow { x } else { x + col_w + gap };
        let artist_data = artist_meta
            .get("artist")
            .filter(|value| value.is_object())
            .unwrap_or(&artist_meta);
        let mut contributors = crate::catalog::entries(&meta["contributors"], "artist");
        if contributors.is_empty() {
            contributors = track.artist_entries();
        }
        let track_artist = track.artist_entry();
        let mut primary =
            crate::catalog::entry(artist_data, "artist").or_else(|| track_artist.clone());
        if let Some(artist) = &mut primary {
            if artist.artwork.is_empty() {
                if let Some(source) = track_artist
                    .iter()
                    .chain(contributors.iter())
                    .find(|source| source.id == artist.id && !source.artwork.is_empty())
                {
                    artist.artwork = source.artwork.clone();
                }
            }
        }
        let contributor_h = 76 + contributors.len().max(1) as i32 * 56;
        let details_h = 192;
        let about_h = contributor_h + gap + details_h;
        let contributor_y = if narrow { cy + about_h + gap } else { cy };
        let details_y = contributor_y + contributor_h + gap;
        let artist_y = if narrow {
            details_y + details_h + gap
        } else {
            cy + about_h + gap
        };
        let biography = crate::catalog::string(artist_data, &["biography", "bio", "description"]);
        let biography = if biography.is_empty() {
            crate::catalog::string(&artist_data["biography"], &["text", "content"])
        } else {
            biography
        };
        let biography = if biography.is_empty() {
            ui.t("Discover the artist's popular songs and releases.")
                .into()
        } else {
            plain_description(&biography)
        };
        let bio_x = x + 136.min(cw / 2);
        let bio_w = (cw - (bio_x - x) - 24).max(1);
        let bio_lines: Vec<_> = wrap_text(&biography, bio_w, 14)
            .into_iter()
            .take(3)
            .collect();
        let artist_h = (124 + bio_lines.len() as i32 * 24).max(176);
        let copyright = strings(meta.get("copyrights").unwrap_or(&meta["copyright"])).join(" · ");
        // The reference uses a plain page header, with surfaces only around details.
        s.panel(x, top, cw, artist_y + artist_h - top + 60, 31);
        let cover = Rect::new(x, top, art, art);
        if track.artwork.is_empty() {
            s.panel(cover.x, cover.y, cover.w, cover.h, 1);
        } else {
            s.pictures.push((cover, track.artwork.clone()));
        }
        s.label_sized(text_x, top + 4, text_w, 24, "TRACK", 9);
        for (index, line) in title_lines.iter().enumerate() {
            s.label_sized(
                text_x,
                top + 30 + index as i32 * 34,
                title_w,
                36,
                line,
                if narrow { 3 } else { 5 },
            );
        }
        if explicit == Some(true) {
            let estimate = title_lines
                .last()
                .map_or(0, |line| title_width(line, narrow));
            let badge_x = (text_x + estimate + 12).min(x + cw - 20);
            let badge_y = top + 42 + (title_lines.len().saturating_sub(1) as i32) * 34;
            s.panel(badge_x, badge_y, 16, 18, 33);
            s.label_sized(badge_x + 3, badge_y, 12, 18, "E", 9);
        }
        let mut subtitle = vec![track.subtitle.clone()];
        if !released.is_empty() {
            subtitle.push(released.clone());
        }
        if track.duration > 0 {
            subtitle.push(format!(
                "{}:{:02}",
                track.duration / 60000,
                track.duration / 1000 % 60
            ));
        }
        let subtitle_y = if narrow {
            top + 122
        } else {
            top + if title_lines.len() > 1 { 104 } else { 80 }
        };
        s.label_sized(
            if narrow { x } else { text_x },
            subtitle_y,
            if narrow { cw } else { text_w },
            28,
            subtitle
                .into_iter()
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>()
                .join(" · "),
            1,
        );
        let button_x = if narrow { x } else { text_x };
        let button_y = if narrow {
            top + 164
        } else if title_lines.len() > 1 {
            top + 144
        } else {
            top + 120
        };
        let available = cw - (button_x - x);
        let button_w = 136.min((available - 12) / 2).max(1);
        s.button(
            Rect::new(button_x, button_y, button_w, 36),
            ui.t("Play track"),
            "\u{e768}",
            Action::OpenItem(track.clone()),
            true,
        );
        if let Some(album) = track.album_entry() {
            s.button(
                Rect::new(button_x + button_w + 12, button_y, button_w, 36),
                ui.t("View album"),
                "",
                Action::OpenItem(album),
                false,
            );
        }
        s.panel(x, cy, col_w, about_h, 8);
        s.label_sized(x + 20, cy + 18, col_w - 40, 28, "ABOUT THIS TRACK", 9);
        let plays = [
            meta.get("playCount"),
            meta.get("playcount"),
            meta.get("play_count"),
            meta.get("nb_play"),
            meta.pointer("/stats/streamCount"),
        ]
        .into_iter()
        .flatten()
        .find_map(number);
        let popularity = meta
            .get("popularity")
            .and_then(number)
            .map(|n| n.min(100))
            .or_else(|| {
                meta.get("rank")
                    .and_then(number)
                    .map(|n| ((n as f64 / 10000.0).round() as u64).min(100))
            });
        let position = crate::catalog::string(&meta, &["track_position", "track_number"]);
        let label = crate::catalog::string(&meta, &["label", "label_name"]);
        let label = if label.is_empty() {
            crate::catalog::string(&meta["album"], &["label", "label_name"])
        } else {
            label
        };
        rows(
            s,
            x,
            cy + 60,
            col_w,
            &[
                ("Album", missing(ui, track.album.clone())),
                ("Released", missing(ui, released)),
                (
                    "Plays",
                    plays
                        .map(|n| count(ui, n))
                        .unwrap_or_else(|| ui.t("Unavailable").into()),
                ),
                (
                    "Order",
                    if position.is_empty() {
                        ui.t("Unavailable").into()
                    } else {
                        ui.t("Track {0}").replace("{0}", &position)
                    },
                ),
                ("Label", missing(ui, label)),
                (
                    "Popularity",
                    popularity
                        .map(|n| format!("{n}%"))
                        .unwrap_or_else(|| ui.t("Unavailable").into()),
                ),
            ],
        );
        s.panel(right_x, contributor_y, col_w, contributor_h, 8);
        s.label_sized(
            right_x + 20,
            contributor_y + 18,
            col_w - 40,
            28,
            "CONTRIBUTORS",
            9,
        );
        if contributors.is_empty() {
            s.label_sized(
                right_x + 28,
                contributor_y + 62,
                col_w - 56,
                28,
                missing(ui, track.subtitle.clone()),
                0,
            );
        }
        for (index, mut artist) in contributors.into_iter().enumerate() {
            if let Some(primary) = primary.as_ref().filter(|p| p.id == artist.id) {
                if artist.artwork.is_empty() {
                    artist.artwork = primary.artwork.clone();
                }
            }
            let y = contributor_y + 60 + index as i32 * 56;
            if !artist.artwork.is_empty() {
                s.pictures.push((
                    Rect::new(right_x + 28, y + 2, 40, 40),
                    artist.artwork.clone(),
                ));
            } else {
                s.panel(right_x + 28, y + 2, 40, 40, 15);
            }
            let role = crate::catalog::string(&artist.raw, &["role"]);
            let role = match role.as_str() {
                "Main" | "MainArtist" | "main" => "Main artist",
                "" => "Performer",
                _ => &role,
            };
            s.label_sized(right_x + 80, y + 23, col_w - 104, 24, ui.t(role), 1);
            s.link(
                Rect::new(right_x + 80, y, col_w - 104, 26),
                artist.title.clone(),
                artist,
                false,
            );
        }
        s.panel(right_x, details_y, col_w, details_h, 8);
        s.label_sized(
            right_x + 20,
            details_y + 18,
            col_w - 40,
            28,
            "GENRES AND DETAILS",
            9,
        );
        let mut genres = strings(&meta["genres"]);
        if genres.is_empty() {
            genres = strings(&meta["genre"]);
        }
        if genres.is_empty() {
            genres = strings(&meta["album"]["genres"]);
        }
        let language = crate::catalog::string(&meta, &["language", "language_of_performance"]);
        let language = match language.as_str() {
            "en" | "eng" => "English",
            "tr" | "tur" => "Turkish",
            "az" | "aze" => "Azerbaijani",
            "" => "Not specified",
            _ => &language,
        };
        rows(
            s,
            right_x,
            details_y + 60,
            col_w,
            &[
                ("Genres", missing(ui, genres.join(", "))),
                ("Language", ui.t(language).into()),
                (
                    "Content",
                    ui.t(match explicit {
                        Some(true) => "Explicit",
                        Some(false) => "Clean",
                        None => "Not specified",
                    })
                    .into(),
                ),
            ],
        );
        s.panel(x, artist_y, cw, artist_h, 8);
        if let Some(artist) = primary.as_ref().filter(|artist| !artist.artwork.is_empty()) {
            s.pictures.push((
                Rect::new(x + 20, artist_y + 48, 88, 88),
                artist.artwork.clone(),
            ));
        } else {
            s.panel(x + 20, artist_y + 48, 88, 88, 15);
        }
        s.label_sized(bio_x, artist_y + 16, bio_w, 28, "ABOUT THE ARTIST", 9);
        let artist_name = primary
            .as_ref()
            .map(|a| a.title.as_str())
            .filter(|name| !name.is_empty())
            .unwrap_or(&track.subtitle);
        s.label_sized(
            bio_x,
            artist_y + 52,
            bio_w,
            36,
            missing(ui, artist_name.into()),
            3,
        );
        for (index, line) in bio_lines.iter().enumerate() {
            s.label_sized(bio_x, artist_y + 96 + index as i32 * 24, bio_w, 24, line, 1);
        }
        if let Some(artist) = primary {
            s.link(
                Rect::new(x + 20, artist_y + 48, 88, 88),
                String::new(),
                artist,
                false,
            );
        }
        if !copyright.is_empty() {
            s.label_sized(x, artist_y + artist_h + 16, cw, 28, copyright, 1);
        }
        s.content_height = artist_y + artist_h + 60 + ui.scroll;
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn artist_artwork_falls_back_to_track_credits() {
            let mut ui = fixture(
                serde_json::json!({"artist":{"id":"2","name":"Artist","picture_medium":"https://example.com/artist.jpg"}}),
            );
            ui.track_info.as_mut().unwrap().2 = serde_json::json!({"id":"2","name":"Artist"});
            let mut scene = Scene::default();
            build(&mut scene, &ui, 0, 0, 1460);
            assert!(scene
                .pictures
                .iter()
                .any(|(_, url)| url == "https://example.com/artist.jpg"));
        }

        fn fixture(meta: Value) -> Ui {
            Ui {
                view: View::TrackInfo,
                track_info: Some((
                    crate::catalog::Entry {
                        id: "42".into(),
                        kind: "track".into(),
                        title: "Song".into(),
                        subtitle: "Artist".into(),
                        album: "Album".into(),
                        artist_id: "2".into(),
                        album_id: "3".into(),
                        duration: 128000,
                        ..Default::default()
                    },
                    meta,
                    Value::Null,
                )),
                ..Default::default()
            }
        }

        fn value<'a>(scene: &'a Scene, key: &str) -> &'a str {
            let key = scene.labels.iter().find(|label| label.text == key).unwrap();
            &scene
                .labels
                .iter()
                .find(|label| {
                    label.kind == 21
                        && label.rect.y == key.rect.y
                        && label.rect.x > key.rect.x
                        && label.rect.x < key.rect.x + 700
                })
                .unwrap()
                .text
        }

        #[test]
        fn unavailable_fields_remain_visible_and_rank_is_not_a_stream_count() {
            let ui = fixture(serde_json::json!({"rank":810000,"explicit":true}));
            let mut scene = Scene::default();
            build(&mut scene, &ui, 0, 0, 1460);
            assert_eq!(value(&scene, "Plays"), "Unavailable");
            assert_eq!(value(&scene, "Popularity"), "81%");
            assert_eq!(value(&scene, "Genres"), "Unavailable");
            assert_eq!(value(&scene, "Language"), "Not specified");
            assert_eq!(value(&scene, "Content"), "Explicit");
            assert!(scene
                .labels
                .iter()
                .any(|label| label.text == "Discover the artist's popular songs and releases."));
            assert!(!scene
                .labels
                .iter()
                .any(|label| matches!(label.text.as_str(), "Format" | "Source" | "BPM" | "ISRC")));
        }

        #[test]
        fn reference_cards_and_real_zero_popularity_are_preserved() {
            let mut ui = fixture(
                serde_json::json!({"release_date":"2019-12-11","popularity":0,"track_number":6,"label":"Mary Joy Recordings"}),
            );
            ui.set_language("tr");
            let mut scene = Scene::default();
            build(&mut scene, &ui, 0, 0, 1460);
            assert_eq!(value(&scene, "Released"), "11 Ara 2019");
            assert_eq!(value(&scene, "Popularity"), "0%");
            assert_eq!(value(&scene, "Order"), "Parça 6");
            assert_eq!(value(&scene, "Plays"), "Kullanılamıyor");
            let cards: Vec<_> = scene
                .panels
                .iter()
                .filter(|panel| panel.kind == 8)
                .collect();
            assert_eq!(cards.len(), 4);
            assert_eq!(cards[0].rect.y, cards[1].rect.y);
            assert_eq!(
                cards[0].rect.y + cards[0].rect.h,
                cards[2].rect.y + cards[2].rect.h
            );
            assert_eq!(cards[3].rect.w, 1460);
            assert!(cards[3].rect.y > cards[0].rect.y + cards[0].rect.h);
            assert_eq!(
                scene.panels.iter().filter(|panel| panel.kind == 32).count(),
                4
            );
        }

        #[test]
        fn song_header_stays_inside_the_main_canvas_clip() {
            let ui = fixture(serde_json::json!({"explicit":true}));
            let scene = super::super::build(&ui, (1200, 900), true, false, "", false, 50);
            let header = scene
                .labels
                .iter()
                .find(|label| label.text == "TRACK")
                .unwrap();
            assert!(header.rect.y >= scene.content_top);
            assert!(!scene
                .labels
                .iter()
                .any(|label| label.text == "Track information"));
        }

        #[test]
        fn explicit_badge_follows_the_measured_title_width() {
            let mut ui = fixture(serde_json::json!({"explicit":true}));
            ui.track_info.as_mut().unwrap().0.title = "WWWWWWWWWW".into();
            let mut scene = Scene::default();
            build(&mut scene, &ui, 0, 0, 1460);
            let title = scene
                .labels
                .iter()
                .find(|label| label.text == "WWWWWWWWWW")
                .unwrap();
            let badge = scene.labels.iter().find(|label| label.text == "E").unwrap();
            assert!(badge.rect.x > title.rect.x + title_width(&title.text, false));
        }
    }
}
// Extra rows let scrolling translate the scene between layout rebuilds.
pub(crate) const SCROLL_OVERSCAN: i32 = 384;
mod catalog_scroll {
    //! Refresh the catalog window without rebuilding chrome or remeasuring retained rows.
    use super::*;
    use std::{collections::HashMap, ops::Range};
    #[derive(Clone)]
    pub(crate) struct Ranges {
        buttons: Range<usize>,
        labels: Range<usize>,
        panels: Range<usize>,
        pictures: Range<usize>,
    }
    impl Ranges {
        pub(super) fn at(s: &Scene) -> Self {
            Self {
                buttons: s.buttons.len()..s.buttons.len(),
                labels: s.labels.len()..s.labels.len(),
                panels: s.panels.len()..s.panels.len(),
                pictures: s.pictures.len()..s.pictures.len(),
            }
        }
        pub(super) fn finish(mut self, s: &Scene) -> Self {
            self.buttons.end = s.buttons.len();
            self.labels.end = s.labels.len();
            self.panels.end = s.panels.len();
            self.pictures.end = s.pictures.len();
            self
        }
        fn offset(&mut self, old: &Ranges) {
            for (a, b) in [
                (&mut self.buttons, &old.buttons),
                (&mut self.labels, &old.labels),
                (&mut self.panels, &old.panels),
                (&mut self.pictures, &old.pictures),
            ] {
                a.start += b.start;
                a.end += b.start;
            }
        }
    }
    #[derive(Clone)]
    pub(crate) struct Layout {
        pub(super) x: i32,
        pub(super) origin: i32,
        pub(super) width: i32,
        pub(super) ranges: Ranges,
    }
    pub(crate) struct Row {
        key: String,
        pub(crate) y: i32,
        ranges: Ranges,
    }
    pub(super) struct Nodes {
        y: i32,
        buttons: Vec<Button>,
        labels: Vec<Label>,
        panels: Vec<Panel>,
        pictures: Vec<(Rect, String)>,
    }
    pub(super) fn record(s: &mut Scene, key: String, y: i32, start: Ranges) {
        let ranges = start.finish(s);
        s.catalog_rows.push(Row { key, y, ranges });
    }
    pub(super) fn reuse(s: &mut Scene, key: String, y: i32) -> bool {
        let Some(mut row) = s.catalog_reuse.remove(&key) else {
            return false;
        };
        let start = Ranges::at(s);
        let delta = y - row.y;
        for b in &mut row.buttons {
            b.rect.y += delta;
            if let Some(g) = &mut b.artist_credit_group {
                g.y += delta;
            }
        }
        for l in &mut row.labels {
            l.rect.y += delta;
        }
        for p in &mut row.panels {
            p.rect.y += delta;
        }
        for (r, _) in &mut row.pictures {
            r.y += delta;
        }
        s.catalog_preserved_buttons
            .extend(s.buttons.len()..s.buttons.len() + row.buttons.len());
        s.catalog_preserved_labels
            .extend(s.labels.len()..s.labels.len() + row.labels.len());
        s.buttons.extend(row.buttons);
        s.labels.extend(row.labels);
        s.panels.extend(row.panels);
        s.pictures.extend(row.pictures);
        record(s, key, y, start);
        true
    }
    fn take<T>(values: &mut [Option<T>], range: &Range<usize>, base: usize) -> Vec<T> {
        values[range.start - base..range.end - base]
            .iter_mut()
            .map(|v| v.take().unwrap())
            .collect()
    }
    pub(crate) struct Patch {
        pub(crate) scene: Scene,
        old: Layout,
    }
    pub(crate) fn prepare_catalog_scroll(scene: &mut Scene, ui: &Ui, h: i32) -> Option<Patch> {
        if scene.auth_modal.is_some() || scene.login_underlay.is_some() || ui.view == View::Lyrics {
            return None;
        }
        let old = scene.catalog_layout.clone()?;
        let page = ui.page.as_ref()?;
        let mut buttons: Vec<_> = scene
            .buttons
            .drain(old.ranges.buttons.clone())
            .map(Some)
            .collect();
        let mut labels: Vec<_> = scene
            .labels
            .drain(old.ranges.labels.clone())
            .map(Some)
            .collect();
        let mut panels: Vec<_> = scene
            .panels
            .drain(old.ranges.panels.clone())
            .map(Some)
            .collect();
        let mut pictures: Vec<_> = scene
            .pictures
            .drain(old.ranges.pictures.clone())
            .map(Some)
            .collect();
        let mut cache = HashMap::new();
        for row in scene.catalog_rows.drain(..) {
            cache.insert(
                row.key,
                Nodes {
                    y: row.y,
                    buttons: take(&mut buttons, &row.ranges.buttons, old.ranges.buttons.start),
                    labels: take(&mut labels, &row.ranges.labels, old.ranges.labels.start),
                    panels: take(&mut panels, &row.ranges.panels, old.ranges.panels.start),
                    pictures: take(
                        &mut pictures,
                        &row.ranges.pictures,
                        old.ranges.pictures.start,
                    ),
                },
            );
        }
        let mut next = Scene {
            content_top: scene.content_top,
            catalog_reuse: cache,
            ..Default::default()
        };
        live_catalog(
            &mut next,
            ui,
            page,
            old.x,
            old.origin - ui.scroll,
            old.width,
            h,
        );
        // Row media strings already carry their final text. Only structural nodes
        // need translation; avoid scanning every entry in a large page again.
        for (i, l) in next.labels.iter_mut().enumerate() {
            if !next
                .catalog_rows
                .iter()
                .any(|r| r.ranges.labels.contains(&i))
            {
                l.text = ui.translated_text(&l.text);
            }
        }
        for (i, b) in next.buttons.iter_mut().enumerate() {
            if !next
                .catalog_rows
                .iter()
                .any(|r| r.ranges.buttons.contains(&i))
            {
                b.text = ui.translated_text(&b.text);
                b.accessible = ui.translated_text(&b.accessible);
            }
        }
        Some(Patch { scene: next, old })
    }
    fn index(i: &mut usize, old: &Range<usize>, len: usize) {
        if *i >= old.end {
            *i = *i - old.len() + len;
        }
    }
    fn sibling(r: &mut Range<usize>, old: &Range<usize>, len: usize) {
        if r.start >= old.end {
            index(&mut r.start, old, len);
            index(&mut r.end, old, len);
        }
    }
    pub(super) fn rebase_queue(
        scene: &mut Scene,
        buttons: &Range<usize>,
        button_len: usize,
        labels: &Range<usize>,
        label_len: usize,
        panels: &Range<usize>,
        panel_len: usize,
        pictures: &Range<usize>,
        picture_len: usize,
    ) {
        let rebase = |r: &mut Ranges| {
            sibling(&mut r.buttons, buttons, button_len);
            sibling(&mut r.labels, labels, label_len);
            sibling(&mut r.panels, panels, panel_len);
            sibling(&mut r.pictures, pictures, picture_len);
        };
        if let Some(layout) = &mut scene.catalog_layout {
            rebase(&mut layout.ranges);
        }
        for row in &mut scene.catalog_rows {
            rebase(&mut row.ranges);
        }
    }
    pub(crate) fn finish_catalog_scroll(scene: &mut Scene, mut patch: Patch) {
        let old = &patch.old.ranges;
        let next = &mut patch.scene;
        scene.content_buttons.end =
            scene.content_buttons.end - old.buttons.len() + next.buttons.len();
        scene.content_labels.end = scene.content_labels.end - old.labels.len() + next.labels.len();
        scene.content_panels.end = scene.content_panels.end - old.panels.len() + next.panels.len();
        scene.content_pictures.end =
            scene.content_pictures.end - old.pictures.len() + next.pictures.len();
        for r in [
            &mut scene.queue_buttons,
            &mut scene.queue_row_buttons,
            &mut scene.device_sidebar_buttons,
        ] {
            sibling(r, &old.buttons, next.buttons.len());
        }
        for r in [&mut scene.queue_labels, &mut scene.queue_row_labels] {
            sibling(r, &old.labels, next.labels.len());
        }
        sibling(&mut scene.queue_panels, &old.panels, next.panels.len());
        sibling(
            &mut scene.queue_pictures,
            &old.pictures,
            next.pictures.len(),
        );
        if let Some(t) = &mut scene.player_title {
            index(&mut t.label, &old.labels, next.labels.len());
            if let Some(i) = &mut t.badge {
                index(i, &old.labels, next.labels.len());
            }
        }
        if let Some(i) = &mut scene.karaoke_preview {
            index(i, &old.labels, next.labels.len());
        }
        for row in &mut scene.lyric_rows {
            index(&mut row.label, &old.labels, next.labels.len());
        }
        scene.artwork_ahead = std::mem::take(&mut next.artwork_ahead);
        // Keep the full layout's document extent: it includes header/player insets.
        for row in &mut next.catalog_rows {
            row.ranges.offset(old);
        }
        scene.catalog_rows = std::mem::take(&mut next.catalog_rows);
        if let Some(layout) = &mut next.catalog_layout {
            layout.ranges.offset(old);
        }
        scene.catalog_layout = next.catalog_layout.take();
        scene
            .buttons
            .splice(old.buttons.start..old.buttons.start, next.buttons.drain(..));
        scene
            .labels
            .splice(old.labels.start..old.labels.start, next.labels.drain(..));
        scene
            .panels
            .splice(old.panels.start..old.panels.start, next.panels.drain(..));
        scene.pictures.splice(
            old.pictures.start..old.pictures.start,
            next.pictures.drain(..),
        );
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        fn shift(scene: &mut Scene, delta: i32) {
            for b in &mut scene.buttons[scene.content_buttons.clone()] {
                b.rect.y += delta;
                if let Some(r) = &mut b.artist_credit_group {
                    r.y += delta;
                }
            }
            for l in &mut scene.labels[scene.content_labels.clone()] {
                l.rect.y += delta;
            }
            for p in &mut scene.panels[scene.content_panels.clone()] {
                p.rect.y += delta;
            }
            for (r, _) in &mut scene.pictures[scene.content_pictures.clone()] {
                r.y += delta;
            }
            for row in &mut scene.catalog_rows {
                row.y += delta;
            }
        }
        #[test]
        fn catalog_updates_reuse_rows_and_preserve_chrome_across_queue_changes() {
            for provider in [
                crate::catalog::Provider::Deezer,
                crate::catalog::Provider::Spotify,
            ] {
                for view in [View::Albums, View::Favorites, View::Home, View::Explore] {
                    let entries: Vec<_> = (0..200)
                        .map(|i| crate::catalog::Entry {
                            id: format!("item-{i}"),
                            title: format!("Title {i}"),
                            subtitle: "Artist".into(),
                            kind: if view == View::Favorites {
                                "track"
                            } else {
                                "album"
                            }
                            .into(),
                            artwork: format!("art-{i}"),
                            ..Default::default()
                        })
                        .collect();
                    let page = crate::catalog::Page {
                        rows: if view == View::Favorites {
                            entries.clone()
                        } else {
                            Vec::new()
                        },
                        sections: if view != View::Favorites {
                            vec![crate::catalog::Section {
                                title: "Albums".into(),
                                entries: entries.clone(),
                                ..Default::default()
                            }]
                        } else {
                            Vec::new()
                        },
                        ..Default::default()
                    };
                    let mut ui = Ui {
                        provider,
                        view,
                        page: Some(std::sync::Arc::new(page)),
                        queue_sidebar_open: true,
                        queue: entries,
                        ..Default::default()
                    };
                    let mut scene = build(&ui, (1200, 800), true, false, "Track", false, 70);
                    let chrome = scene.buttons[0].text.as_ptr();
                    let player = scene
                        .player_title
                        .as_ref()
                        .map(|t| scene.labels[t.label].text.as_ptr());
                    for position in [200, 360, 700, 900, 560, 100, 0] {
                        let previous = ui.scroll;
                        ui.scroll = position;
                        shift(&mut scene, previous - position);
                        let retained = scene.catalog_rows.iter().find_map(|row| {
                            scene
                                .buttons
                                .get(row.ranges.buttons.start)
                                .map(|b| (b.accessible.clone(), b.accessible.as_ptr()))
                        });
                        let patch = prepare_catalog_scroll(&mut scene, &ui, 800).unwrap();
                        if let Some((text, pointer)) = retained {
                            if let Some(b) =
                                patch.scene.buttons.iter().find(|b| b.accessible == text)
                            {
                                assert_eq!(
                                    b.accessible.as_ptr(),
                                    pointer,
                                    "retained row strings must keep their allocation"
                                );
                            }
                        }
                        finish_catalog_scroll(&mut scene, patch);
                        let fresh = build(&ui, (1200, 800), true, false, "Track", false, 70);
                        assert_eq!(
                            scene.buttons.len(),
                            fresh.buttons.len(),
                            "{provider:?} {view:?} at {position}"
                        );
                        assert_eq!(scene.labels.len(), fresh.labels.len());
                        for (a, b) in scene.buttons[scene.content_buttons.clone()]
                            .iter()
                            .zip(&fresh.buttons[fresh.content_buttons.clone()])
                        {
                            assert_eq!(a.rect, b.rect);
                            assert_eq!(a.text, b.text);
                            assert_eq!(a.artwork, b.artwork);
                        }
                        for (a, b) in scene.labels[scene.content_labels.clone()]
                            .iter()
                            .zip(&fresh.labels[fresh.content_labels.clone()])
                        {
                            assert_eq!(a.rect, b.rect);
                            assert_eq!(a.text, b.text);
                        }
                        assert_eq!(chrome, scene.buttons[0].text.as_ptr());
                        assert_eq!(
                            player,
                            scene
                                .player_title
                                .as_ref()
                                .map(|t| scene.labels[t.label].text.as_ptr())
                        );
                        ui.queue_scroll += 8;
                        assert!(refresh_queue_sidebar(&mut scene, &ui, 800));
                    }
                }
            }
        }
    }
}
mod queue_scroll {
    //! Replace only virtualized queue nodes; keep the main page and native controls.
    use super::*;

    fn index(index: &mut usize, old: &std::ops::Range<usize>, new_len: usize) {
        if *index >= old.end {
            *index = (*index - old.len()) + new_len;
        }
    }
    fn range(range: &mut std::ops::Range<usize>, old: &std::ops::Range<usize>, new_len: usize) {
        if range.start >= old.end {
            index(&mut range.start, old, new_len);
            index(&mut range.end, old, new_len);
        }
    }
    pub(crate) fn refresh_queue_sidebar(scene: &mut Scene, ui: &Ui, h: i32) -> bool {
        let Some(frame) = scene.queue_frame else {
            return false;
        };
        if scene.device_sidebar.is_some()
            || !ui.queue_sidebar_open
            || scene.auth_modal.is_some()
            || scene.login_underlay.is_some()
        {
            return false;
        }
        let pixels = (ui.queue_scroll.min(scene.queue_scroll_max) * 24
            + ui.queue_scroll_offset.min(23)) as i32;
        let returning = (pixels - scene.queue_anchor as i32).abs() >= 216;
        let had_return = scene.buttons[scene.queue_buttons.clone()]
            .iter()
            .any(|b| matches!(b.action, Action::QueueReturn));
        if (pixels - scene.queue_layout_scroll).abs() < 112 && returning == had_return {
            let delta = scene.queue_pixel_scroll - pixels;
            for button in &mut scene.buttons[scene.queue_row_buttons.clone()] {
                button.rect.y += delta;
            }
            for label in &mut scene.labels[scene.queue_row_labels.clone()] {
                label.rect.y += delta;
            }
            for (rect, _) in &mut scene.pictures[scene.queue_pictures.clone()] {
                rect.y += delta;
            }
            scene.queue_pixel_scroll = pixels;
            if let Some(track) = scene.queue_scrollbar {
                for panel in &mut scene.panels[scene.queue_panels.clone()] {
                    if panel.kind == 17 {
                        panel.rect.y = track.y
                            + ((pixels as i64 * (track.h - panel.rect.h) as i64)
                                / (scene.queue_scroll_max.max(1) as i64 * 24))
                                as i32;
                    }
                }
            }
            return true;
        }
        let mut next = Scene::default();
        build_queue_sidebar(&mut next, ui, frame.x, frame.w, h);
        // Queue media strings are already final. Translate only the header/actions,
        // avoiding a scan of the entire catalog and playback queue on every frame.
        for (i, label) in next.labels.iter_mut().enumerate() {
            if !next.queue_row_labels.contains(&i) {
                label.text = ui.translated_text(&label.text);
            }
        }
        for button in &mut next.buttons {
            if !matches!(button.action, Action::QueueJump(_)) {
                button.text = ui.translated_text(&button.text);
                button.accessible = ui.translated_text(&button.accessible);
            }
        }
        let buttons = scene.queue_buttons.clone();
        let labels = scene.queue_labels.clone();
        let panels = scene.queue_panels.clone();
        let pictures = scene.queue_pictures.clone();
        super::catalog_scroll::rebase_queue(
            scene,
            &buttons,
            next.buttons.len(),
            &labels,
            next.labels.len(),
            &panels,
            next.panels.len(),
            &pictures,
            next.pictures.len(),
        );
        range(&mut scene.content_buttons, &buttons, next.buttons.len());
        range(
            &mut scene.device_sidebar_buttons,
            &buttons,
            next.buttons.len(),
        );
        range(&mut scene.content_labels, &labels, next.labels.len());
        range(&mut scene.content_panels, &panels, next.panels.len());
        range(&mut scene.content_pictures, &pictures, next.pictures.len());
        if let Some(title) = &mut scene.player_title {
            index(&mut title.label, &labels, next.labels.len());
            if let Some(badge) = &mut title.badge {
                index(badge, &labels, next.labels.len());
            }
        }
        if let Some(preview) = &mut scene.karaoke_preview {
            index(preview, &labels, next.labels.len());
        }
        for row in &mut scene.lyric_rows {
            index(&mut row.label, &labels, next.labels.len());
        }
        scene.queue_row_labels =
            labels.start + next.queue_row_labels.start..labels.start + next.queue_row_labels.end;
        scene.queue_row_buttons = buttons.start + next.queue_row_buttons.start
            ..buttons.start + next.queue_row_buttons.end;
        scene.queue_layout_scroll = next.queue_layout_scroll;
        scene.queue_pixel_scroll = next.queue_pixel_scroll;
        scene.queue_buttons = buttons.start..buttons.start + next.buttons.len();
        scene.queue_labels = labels.start..labels.start + next.labels.len();
        scene.queue_panels = panels.start..panels.start + next.panels.len();
        scene.queue_pictures = pictures.start..pictures.start + next.pictures.len();
        scene.queue_viewport = next.queue_viewport;
        scene.queue_scrollbar = next.queue_scrollbar;
        scene.queue_scroll_max = next.queue_scroll_max;
        scene.queue_anchor = next.queue_anchor;
        scene.buttons.splice(buttons, next.buttons);
        scene.labels.splice(labels, next.labels);
        scene.panels.splice(panels, next.panels);
        scene.pictures.splice(pictures, next.pictures);
        true
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        #[test]
        fn queue_refresh_preserves_main_page_and_matches_fresh_layout() {
            for provider in [
                crate::catalog::Provider::Deezer,
                crate::catalog::Provider::Spotify,
            ] {
                let mut ui = Ui {
                    provider,
                    view: View::Favorites,
                    queue_sidebar_open: true,
                    ..Default::default()
                };
                ui.queue = (0..200)
                    .map(|i| crate::catalog::Entry {
                        id: format!("q{i}"),
                        title: format!("Queue {i}"),
                        subtitle: "Artist".into(),
                        artwork: "fixture-cover".into(),
                        kind: "track".into(),
                        ..Default::default()
                    })
                    .collect();
                ui.now = ui.queue[0].clone();
                ui.page = Some(std::sync::Arc::new(crate::catalog::Page {
                    rows: (0..1000)
                        .map(|i| crate::catalog::Entry {
                            id: format!("p{i}"),
                            title: format!("Page {i}"),
                            kind: "track".into(),
                            ..Default::default()
                        })
                        .collect(),
                    ..Default::default()
                }));
                for view in [View::Favorites, View::Lyrics] {
                    ui.view = view;
                    ui.queue_scroll = 0;
                    ui.queue_scroll_offset = 0;
                    let mut scene = build(&ui, (1200, 800), true, false, "", false, 50);
                    let main_labels: Vec<_> = scene.labels[scene.content_labels.clone()]
                        .iter()
                        .map(|l| (l.text.as_ptr(), l.rect))
                        .collect();
                    for pixels in [1, 73, 700, scene.queue_scroll_max * 24] {
                        ui.queue_scroll = pixels / 24;
                        ui.queue_scroll_offset = pixels % 24;
                        assert!(refresh_queue_sidebar(&mut scene, &ui, 800));
                        let fresh = build(&ui, (1200, 800), true, false, "", false, 50);
                        let viewport = scene.queue_viewport.unwrap();
                        let visible =
                            |r: Rect| r.y < viewport.y + viewport.h && r.y + r.h > viewport.y;
                        assert!(scene.labels[scene.content_labels.clone()]
                            .iter()
                            .map(|l| (l.text.as_ptr(), l.rect))
                            .eq(main_labels.iter().copied()));
                        assert!(scene.labels[scene.queue_labels.clone()]
                            .iter()
                            .filter(|l| visible(l.rect))
                            .map(|l| (l.rect, l.text.as_str(), l.kind))
                            .eq(fresh.labels[fresh.queue_labels.clone()]
                                .iter()
                                .filter(|l| visible(l.rect))
                                .map(|l| (l.rect, l.text.as_str(), l.kind))));
                        assert!(
                            scene.buttons[scene.queue_buttons.clone()]
                                .iter()
                                .filter(|b| !matches!(b.action, Action::QueueJump(_))
                                    || visible(b.rect))
                                .map(|b| (b.rect, format!("{:?}", b.action)))
                                .eq(fresh.buttons[fresh.queue_buttons.clone()]
                                    .iter()
                                    .filter(|b| !matches!(b.action, Action::QueueJump(_))
                                        || visible(b.rect))
                                    .map(|b| (b.rect, format!("{:?}", b.action))))
                        );
                        assert_eq!(scene.content_height, fresh.content_height);
                        if let Some(title) = &scene.player_title {
                            assert_eq!(
                                scene.labels[title.label].text,
                                fresh.labels[fresh.player_title.as_ref().unwrap().label].text
                            );
                        }
                        assert!(scene.buttons[scene.queue_buttons.clone()]
                            .iter()
                            .filter(|b| matches!(b.action, Action::QueueJump(_)))
                            .all(|b| b.rect.h == 66));
                    }
                }
            }
        }
    }
}
pub(crate) use catalog_scroll::{finish_catalog_scroll, prepare_catalog_scroll};
pub(crate) use queue_scroll::refresh_queue_sidebar;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum View {
    Home,
    Explore,

    Flow,
    StyleMix,
    Podcasts,
    Favorites,
    Albums,
    Playlists,
    Offline,
    Recent,
    Queue,
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
            Self::StyleMix => "Style Mix",
            Self::Podcasts => "Podcasts",
            Self::Favorites => "Favorites",
            Self::Albums => "Liked albums",
            Self::Playlists => "Library",
            Self::Offline => "Offline",
            Self::Recent => "Recently played",
            Self::Queue => "Queue",
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
pub const NAV: [(View, &str); 10] = [
    (View::Home, "\u{e80f}"),
    (View::Explore, "\u{e721}"),
    (View::Flow, "\u{e8d6}"),
    (View::StyleMix, "\u{e8b1}"),
    (View::Favorites, "\u{eb51}"),
    (View::Albums, "\u{e93c}"),
    (View::Playlists, "\u{e8f1}"),
    (View::Podcasts, "\u{e720}"),
    (View::Offline, "\u{e896}"),
    (View::Recent, "\u{e81c}"),
];
pub const TABS: [&str; 7] = [
    "General",
    "Providers",
    "Appearance",
    "Lyrics",
    "Playback",
    "Integrations",
    "Keyboard shortcuts",
];
pub const TAB_ICONS: [&str; 7] = [
    "\u{e713}", "\u{e77b}", "\u{e790}", "\u{ec71}", "\u{e768}", "\u{e71b}", "\u{e765}",
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
    SwitchProvider(crate::catalog::Provider),
    DeezerLogin,
    YouTubeLogin,
    YouTubeLogout,
    SpotifyLogin,
    OnboardingSelect(crate::catalog::Provider),
    OnboardingNext,
    SpotifyLogout,
    SpotifyRefresh,
    CheckUpdate,
    DownloadUpdate,
    ApplyUpdate,
    BindShortcut(usize),
    ResetShortcut(usize),
    OpenItem(crate::catalog::Entry),
    PlayAll,
    More,
    CollectionMenu,
    RefreshFlow,
    Favorite,
    FavoriteNow,
    MuteVolume,
    PlayerFullscreen,
    Download,
    RemoveDownloads,
    LibraryMaintenance(bool),
    RepairDownloads,
    Share,
    Sort(usize),
    ToggleCheck(String),
    Preset(usize),
    ResetTheme,
    ImportTheme,
    ExportTheme,
    Shelf(String, i32),
    ShelfAll(String),
    ShelfMore(String, usize),
    Api(&'static str, Vec<serde_json::Value>),
    Adjust(&'static str, i32),
    BackgroundImage,
    ChooseStorage,
    CancelStorage,
    BackgroundMode(usize),
    Color(&'static str),
    FlowChoice(usize),
    Genre(usize),
    FlowMoreGenres,
    FlowAddGenre,
    ApplyFlow,
    MixRegion(usize),
    MixLanguages,
    MixMore(bool),
    MixLess(bool),
    MixGenre(usize),
    BuildStyleMix,
    CancelDownload(u64),
    Go(View),
    Back,
    Connect,
    ConnectSaved,
    OpenConnect,
    ToggleDeviceMenu,
    CloseDeviceMenu,
    SpotifyDevicesRefresh,
    SpotifyJamOpen,
    SpotifyJamRefresh,
    SpotifyJamStart,
    SpotifyJamEnd(String),
    SpotifyJamLeave(String),
    SpotifyJamRemove(String, String),
    SpotifyJamCopyInvite,
    SpotifyTransferPlayback(String),
    CopyConnectLink,
    SpotifySetup,
    StartSpotify,
    OpenSpotify,
    OpenSpotifyOutputSettings,
    ReconnectSpotifyOutput,
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
    QueueReturn,
    LyricsFollow,
    SeekMs(u64),
    ToggleLyricsCoverMode,
    ToggleLyricsVisualizer,
    ToggleLyricsFullscreen,
    VisualizerStyle(u8),
    CornerStyle(usize),
    SolidTheme(usize),
    ThemeShelf(i32),
    CustomTheme(bool),
    SaveCustomTheme,
    CloseCustomTheme,
    ResetCustomTheme,
    PlayerLayout(usize),
    ToggleBool(&'static str),
    AdjustLyricsSize(bool, i32),
    ToggleLyricSource(usize),

    DeezerLogo,
    SpotifyLogo,
    YouTubeLogo,

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

fn keep_content_controls_visible(scene: &mut Scene, left: i32) {
    for button in &mut scene.buttons[scene.content_buttons.clone()] {
        let shift = (left - button.rect.x).max(0);
        button.rect.x += shift;
        if let Some(group) = &mut button.artist_credit_group {
            group.x += shift;
        }
    }
    for label in &mut scene.labels[scene.content_labels.clone()] {
        label.rect.x = label.rect.x.max(left);
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

impl Default for Button {
    fn default() -> Self {
        Self {
            rect: Rect::new(0, 0, 0, 0),
            text: String::new(),
            glyph: "",
            action: Action::More,
            selected: false,
            enabled: true,
            artwork: String::new(),
            accessible: String::new(),
            text_link: false,
            text_centered: false,
            artist_credit_group: None,
        }
    }
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
    pub centered: bool,
}
pub struct LyricRow {
    pub label: usize,
    pub line: usize,
    pub range: std::ops::Range<usize>,
    pub button: Option<usize>,
    pub lag: f32,
    pub velocity: f32,
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
    pub device_sidebar: Option<Rect>,
    pub device_sidebar_buttons: std::ops::Range<usize>,
    pub device_sidebar_scroll_max: i32,
    pub device_active_card: Option<Rect>,
    pub lyric_rows: Vec<LyricRow>,
    pub lyrics_anchor: Option<i32>,
    pub lyrics_focus_y: Option<i32>,
    pub lyrics_motion_at: Option<std::time::Instant>,
    pub lyrics_gap: Option<(u64, u64)>,
    pub karaoke_preview: Option<usize>,
    pub queue_viewport: Option<Rect>,
    pub queue_scrollbar: Option<Rect>,
    pub queue_scroll_max: usize,
    pub queue_anchor: usize,
    pub artwork_ahead: Vec<(i32, String)>,
    pub content_buttons: std::ops::Range<usize>,
    pub content_labels: std::ops::Range<usize>,
    pub queue_labels: std::ops::Range<usize>,
    pub queue_buttons: std::ops::Range<usize>,
    pub queue_panels: std::ops::Range<usize>,
    pub queue_pictures: std::ops::Range<usize>,
    pub queue_row_labels: std::ops::Range<usize>,
    pub queue_row_buttons: std::ops::Range<usize>,
    pub queue_layout_scroll: i32,
    pub queue_pixel_scroll: i32,
    pub queue_frame: Option<Rect>,
    pub content_panels: std::ops::Range<usize>,
    pub content_pictures: std::ops::Range<usize>,
    pub content_top: i32,
    pub(crate) catalog_layout: Option<catalog_scroll::Layout>,
    pub(crate) catalog_rows: Vec<catalog_scroll::Row>,
    catalog_reuse: std::collections::HashMap<String, catalog_scroll::Nodes>,
    pub(crate) catalog_preserved_buttons: std::collections::HashSet<usize>,
    pub(crate) catalog_preserved_labels: std::collections::HashSet<usize>,
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
pub const LAYOUT_TRANSITION_SECONDS: f32 = 0.32;
pub const SCROLL_SPRING_RATE: f32 = 18.0;

pub fn layout_transition_ease(progress: f32) -> f32 {
    let t = progress.clamp(0., 1.);
    t * t * (3. - 2. * t)
}

pub(crate) mod mood_icons;

pub struct Ui {
    pub device_menu_open: bool,
    pub device_sidebar_scroll: i32,
    pub spotify_devices: Vec<serde_json::Value>,
    pub spotify_devices_busy: bool,
    pub spotify_devices_status: String,
    pub spotify_jam: Option<crate::spotify::jam::Session>,
    pub spotify_jam_busy: bool,
    pub spotify_jam_status: String,
    pub provider: crate::catalog::Provider,
    pub last_provider: Option<crate::catalog::Provider>,
    pub spotify_catalog_busy: bool,
    pub spotify_credentials_refreshing: bool,
    pub spotify_catalog_status: String,
    pub spotify_reset_failed: bool,
    pub spotify_reset_pending: bool,
    pub playlist_draft: crate::playlist::Draft,
    pub playlist_busy: bool,
    pub playlist_modal_save: bool,
    pub library_state_loaded: bool,
    pub playlist_track: Option<crate::catalog::Entry>,
    pub expanded_palettes: bool,
    pub expanded_themes: bool,
    pub theme_shelf_offset: usize,
    pub expanded_podcast_description: Option<String>,
    pub dpi_scale: f64,
    pub language: String,
    pub initial_volume: u32,
    pub update: crate::updater::State,
    pub auto_updates: bool,
    pub last_update_check: u64,
    pub shortcuts: crate::shortcuts::Bindings,
    pub shortcut_capture: Option<usize>,
    pub shortcut_errors: [String; 7],
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
    pub pinned: Vec<(crate::catalog::Provider, crate::catalog::Entry)>,
    pub quality: String,
    pub lyrics: Vec<(u64, String)>,
    pub position: u64,
    pub duration: u64,
    pub shelf_offsets: std::collections::HashMap<String, usize>,
    pub shelf_rows: std::collections::HashMap<String, usize>,
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
    pub custom_dark_colors: std::collections::HashMap<String, u32>,
    pub custom_light_colors: std::collections::HashMap<String, u32>,
    pub custom_theme_draft: Option<(bool, std::collections::HashMap<String, u32>)>,
    pub background_file: String,
    pub flow_options: Vec<(String, String)>,
    pub genres: Vec<(String, String, bool, bool)>,
    pub flow_genre_artwork: std::collections::HashMap<String, String>,
    pub flow_genre_aliases: std::collections::HashMap<String, Vec<String>>,
    pub flow_id: String,
    pub deezer_flow_id: String,
    pub spotify_flow_id: String,
    pub youtube_flow_id: String,
    pub flow_last_applied: std::collections::HashMap<String, String>,
    pub flow_personal: bool,
    pub flow_personal_drafts: std::collections::HashMap<String, Vec<String>>,
    pub flow_genre_order: std::collections::HashMap<String, usize>,
    pub flow_mood_genres: Option<std::collections::HashSet<String>>,
    pub flow_mood_defaults: std::collections::HashMap<String, Vec<(String, bool)>>,
    pub flow_edited_moods: std::collections::HashSet<String>,

    pub flow_customizable: bool,
    pub flow_loaded: bool,
    pub flow_filter: String,
    pub flow_genre_limit: usize,
    pub mix_region: usize,
    pub mix_genre: usize,
    pub mix_genres: Option<serde_json::Value>,
    pub mix_language_page: usize,
    pub mix_genre_page: usize,
    pub library_storage: String,
    pub downloads: Vec<crate::downloads::Status>,
    pub download_frame: u8,
    pub offline: Vec<crate::catalog::Entry>,
    pub playlist_sorts: std::collections::HashMap<String, (usize, bool)>,
    pub sort_column: usize,
    pub sort_descending: bool,
    pub checked_ids: std::collections::HashSet<String>,
    pub liked_ids: std::collections::HashSet<String>,
    pub sidebar_collapsed: bool,
    pub queue_sidebar_open: bool,
    pub queue_scroll: usize,
    pub queue_scroll_offset: usize,
    pub queue: Vec<crate::catalog::Entry>,
    pub queue_index: usize,
    pub corner_style: usize,
    pub solid_theme: usize,
    pub adaptive_colors: bool,
    pub adaptive_seed: Option<u32>,
    pub adaptive_artwork_id: u64,
    pub playback_unavailable: bool,
    pub player_layout: usize,
    pub transparent_player: bool,
    pub karaoke_lyrics: bool,
    pub blur_inactive_lyrics: bool,
    pub lyrics_size_panel: u32,
    pub lyrics_size_fullscreen: u32,
    pub lyrics_sources: u32,

    pub onboarding_open: bool,
    pub provider_reauth: [bool; 3],
    pub onboarding_provider: Option<crate::catalog::Provider>,
    pub deezer_login_open: bool,
    pub deezer_login_advanced: bool,
    pub deezer_login_busy: bool,
    pub deezer_login_status: String,
    pub spotify_setup_busy: bool,
    pub spotify_setup_status: String,
    pub spotify_configured: bool,
    pub spotify_phase: Option<crate::spotify::service::Phase>,

    pub lyrics_cover_mode: bool,
    pub lyrics_cover_mix: Option<f32>,
    pub lyrics_visualizer: bool,
    pub lyrics_manual_scroll: bool,
    pub lyrics_manual_until: Option<std::time::Instant>,
    pub lyrics_fullscreen: bool,
    pub visualizer_style: u8,
    pub rich_lyrics: Vec<crate::lyrics::LyricLine>,
    pub lyrics_provider: String,
    pub lyrics_loading: bool,
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
    pub connect_port: u16,
    pub connect_nicknames: Vec<String>,
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
            device_menu_open: false,
            device_sidebar_scroll: 0,
            spotify_devices: Vec::new(),
            spotify_devices_busy: false,
            spotify_devices_status: String::new(),
            spotify_jam: None,
            spotify_jam_busy: false,
            spotify_jam_status: String::new(),
            provider: crate::catalog::Provider::Deezer,
            last_provider: None,
            spotify_catalog_busy: false,
            spotify_credentials_refreshing: false,
            spotify_catalog_status: String::new(),
            spotify_reset_failed: false,
            spotify_reset_pending: false,
            playlist_draft: Default::default(),
            playlist_busy: false,
            playlist_modal_save: false,
            library_state_loaded: false,
            playlist_track: None,
            expanded_palettes: false,
            expanded_themes: false,
            theme_shelf_offset: 0,
            expanded_podcast_description: None,
            dpi_scale: 1.0,
            language: String::new(),
            initial_volume: 70,
            update: Default::default(),
            auto_updates: true,
            last_update_check: 0,
            shortcuts: [None; 7],
            shortcut_capture: None,
            shortcut_errors: Default::default(),
            unmuted_volume: 70,
            translations: Default::default(),
            view: View::Home,
            tab: 0,
            filter: 0,
            light: false,
            solid: true,
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
            pinned: Vec::new(),
            quality: String::new(),
            lyrics: Vec::new(),
            position: 0,
            duration: 0,
            shelf_offsets: Default::default(),
            shelf_rows: Default::default(),
            devices: Vec::new(),
            background_mode: 1,
            blur: 30,
            surface_opacity: 26,
            brightness: 50,
            light_opacity: 10,
            light_tint: 0x00f2ebe4,
            solid_color: 0x00171717,
            dark_colors: Default::default(),
            light_colors: Default::default(),
            custom_dark_colors: Default::default(),
            custom_light_colors: Default::default(),
            custom_theme_draft: None,
            background_file: String::new(),
            flow_options: Vec::new(),
            genres: Vec::new(),
            flow_genre_artwork: Default::default(),
            flow_genre_aliases: Default::default(),
            flow_id: "default".into(),
            deezer_flow_id: "default".into(),
            spotify_flow_id: "default".into(),
            youtube_flow_id: "default".into(),
            flow_last_applied: Default::default(),
            flow_personal: true,
            flow_personal_drafts: Default::default(),
            flow_genre_order: Default::default(),
            flow_mood_genres: None,
            flow_mood_defaults: Default::default(),
            flow_edited_moods: Default::default(),

            flow_customizable: false,
            flow_loaded: false,
            flow_filter: String::new(),
            flow_genre_limit: 20,
            mix_region: usize::MAX,
            mix_genre: usize::MAX,
            mix_genres: None,
            mix_language_page: 0,
            mix_genre_page: 0,
            library_storage: "Scan to measure cache and downloads".into(),
            downloads: Vec::new(),
            download_frame: 0,
            offline: Vec::new(),
            playlist_sorts: Default::default(),
            sort_column: 0,
            sort_descending: false,
            checked_ids: Default::default(),
            liked_ids: Default::default(),
            sidebar_collapsed: false,
            queue_sidebar_open: false,
            queue_scroll: 0,
            queue_scroll_offset: 0,
            queue: Vec::new(),
            queue_index: 0,
            corner_style: 2,
            solid_theme: 1,
            adaptive_colors: false,
            adaptive_seed: None,
            adaptive_artwork_id: 0,
            playback_unavailable: false,
            player_layout: 0,
            transparent_player: true,
            karaoke_lyrics: true,
            blur_inactive_lyrics: true,
            lyrics_size_panel: 100,
            lyrics_size_fullscreen: 100,
            lyrics_sources: crate::lyrics::DEFAULT_SOURCES,

            onboarding_open: false,
            provider_reauth: [false; 3],
            onboarding_provider: None,
            deezer_login_open: false,
            deezer_login_advanced: false,
            deezer_login_busy: false,
            deezer_login_status: String::new(),
            spotify_setup_busy: false,
            spotify_setup_status: String::new(),
            spotify_configured: false,
            spotify_phase: None,

            lyrics_cover_mode: false,
            lyrics_cover_mix: None,
            lyrics_visualizer: true,
            lyrics_manual_scroll: false,
            lyrics_manual_until: None,
            lyrics_fullscreen: false,
            visualizer_style: 0,
            rich_lyrics: Vec::new(),
            lyrics_provider: String::new(),
            lyrics_loading: false,
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
            connect_port: crate::connect::DEFAULT_PORT,
            connect_nicknames: Vec::new(),
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
    pub fn freeze_flow_order(&mut self) {
        let mut genres: Vec<_> = self.genres.iter().collect();
        genres.sort_by_key(|genre| (!genre.2, crate::style_mix::popularity_rank(&genre.1)));
        self.flow_genre_order = genres
            .into_iter()
            .enumerate()
            .map(|(index, genre)| (genre.0.clone(), index))
            .collect();
    }

    pub fn capture_personal_flow_draft(&mut self) {
        self.flow_personal_drafts.insert(
            self.provider.label().into(),
            self.genres
                .iter()
                .filter(|g| g.2)
                .map(|g| {
                    if self.provider == crate::catalog::Provider::Deezer {
                        g.1.clone()
                    } else {
                        g.0.clone()
                    }
                })
                .collect(),
        );
    }

    pub fn remember_applied_flow(&mut self) {
        if self.flow_loaded {
            self.capture_personal_flow_draft();
        }
        self.flow_last_applied
            .insert(self.provider.label().to_owned(), self.flow_id.clone());
    }

    pub fn personal_flow_id(&self) -> String {
        if let Some(genres) = self.flow_personal_drafts.get(self.provider.label()) {
            return if self.provider == crate::catalog::Provider::Deezer {
                "default".into()
            } else {
                crate::spotify::flow::Selection {
                    mood: "default".into(),
                    genres: genres.clone(),
                }
                .key()
            };
        }
        self.flow_last_applied
            .get(self.provider.label())
            .cloned()
            .unwrap_or_else(|| match self.provider {
                crate::catalog::Provider::Deezer => self.deezer_flow_id.clone(),
                crate::catalog::Provider::Spotify => self.spotify_flow_id.clone(),
                crate::catalog::Provider::YouTube => self.youtube_flow_id.clone(),
            })
    }

    fn restore_flow_preferences(&mut self, value: &serde_json::Value) {
        self.flow_mood_defaults =
            serde_json::from_value(value["FlowMoodDefaults"].clone()).unwrap_or_default();
        self.flow_edited_moods =
            serde_json::from_value(value["FlowEditedMoods"].clone()).unwrap_or_default();
        self.flow_personal_drafts =
            serde_json::from_value(value["FlowPersonalDrafts"].clone()).unwrap_or_default();
        self.flow_last_applied =
            serde_json::from_value(value["FlowLastApplied"].clone()).unwrap_or_default();
        let legacy = value["FlowConfigId"].as_str().unwrap_or("default");
        self.deezer_flow_id = value["DeezerFlowConfigId"]
            .as_str()
            .unwrap_or(if legacy.starts_with(crate::spotify::flow::PREFIX) {
                "default"
            } else {
                legacy
            })
            .to_owned();
        self.spotify_flow_id = value["SpotifyFlowConfigId"]
            .as_str()
            .unwrap_or(if legacy.starts_with(crate::spotify::flow::PREFIX) {
                legacy
            } else {
                "default"
            })
            .to_owned();
        self.youtube_flow_id = value["YouTubeFlowConfigId"]
            .as_str()
            .unwrap_or("default")
            .to_owned();
        self.flow_id = match self.provider {
            crate::catalog::Provider::Deezer => self.deezer_flow_id.clone(),
            crate::catalog::Provider::Spotify => self.spotify_flow_id.clone(),
            crate::catalog::Provider::YouTube => self.youtube_flow_id.clone(),
        };
    }

    fn flow_preferences(&self) -> (&str, &str) {
        match self.provider {
            crate::catalog::Provider::Deezer => (&self.flow_id, &self.spotify_flow_id),
            crate::catalog::Provider::Spotify => (&self.deezer_flow_id, &self.flow_id),
            crate::catalog::Provider::YouTube => (&self.deezer_flow_id, &self.spotify_flow_id),
        }
    }

    fn write_flow_preferences(&self, value: &mut serde_json::Value) {
        value["FlowMoodDefaults"] = serde_json::json!(self.flow_mood_defaults);
        value["FlowEditedMoods"] = serde_json::json!(self.flow_edited_moods);
        value["FlowPersonalDrafts"] = serde_json::json!(self.flow_personal_drafts);
        value["FlowLastApplied"] = serde_json::json!(self.flow_last_applied);
        let (deezer, spotify) = self.flow_preferences();
        value["FlowConfigId"] = serde_json::json!(deezer);
        value["DeezerFlowConfigId"] = serde_json::json!(deezer);
        value["SpotifyFlowConfigId"] = serde_json::json!(spotify);
        value["YouTubeFlowConfigId"] =
            serde_json::json!(if self.provider == crate::catalog::Provider::YouTube {
                &self.flow_id
            } else {
                &self.youtube_flow_id
            });
    }
    pub fn switch_flow_provider(&mut self, provider: crate::catalog::Provider) {
        match self.provider {
            crate::catalog::Provider::Deezer => self.deezer_flow_id = self.flow_id.clone(),
            crate::catalog::Provider::Spotify => self.spotify_flow_id = self.flow_id.clone(),
            crate::catalog::Provider::YouTube => self.youtube_flow_id = self.flow_id.clone(),
        }
        self.flow_id = match provider {
            crate::catalog::Provider::Deezer => self.deezer_flow_id.clone(),
            crate::catalog::Provider::Spotify => self.spotify_flow_id.clone(),
            crate::catalog::Provider::YouTube => self.youtube_flow_id.clone(),
        };
        self.flow_filter.clear();
        self.flow_personal = true;
        self.flow_genre_order.clear();
        self.flow_genre_limit = 20;
        self.flow_customizable = false;
        self.flow_loaded = false;
        self.genres.clear();
        self.flow_options.clear();
    }

    /// Fold open sidebars before they squeeze the main page below its useful width.
    /// This runs on window resize; users can still explicitly reopen a sidebar.
    pub fn right_sidebar_open(&self) -> bool {
        self.queue_sidebar_open || self.device_menu_open
    }
    pub fn sidebar_values(&self) -> (f32, f32) {
        (
            if self.sidebar_collapsed { 0. } else { 1. },
            if self.right_sidebar_open() { 1. } else { 0. },
        )
    }
    pub fn content_left(&self) -> i32 {
        (70. + 176. * self.sidebar_values().0).round() as i32
    }
    pub fn right_sidebar_width(&self, width: i32) -> i32 {
        (280.min(width / 3) as f32 * self.sidebar_values().1).round() as i32
    }
    pub fn visible_device_sidebar(&self) -> bool {
        self.device_menu_open
    }
    pub fn toggle_device_sidebar(&mut self) {
        self.device_menu_open = !self.device_menu_open;
        if self.device_menu_open {
            self.queue_sidebar_open = false;
        }
    }
    pub fn toggle_queue_sidebar(&mut self) {
        self.queue_sidebar_open = !self.queue_sidebar_open;
        if self.queue_sidebar_open {
            self.device_menu_open = false;
        }
    }
    pub fn fit_sidebars(&mut self, width: i32) {
        let lyrics = self.view == View::Lyrics;
        let minimum = if matches!(self.view, View::Settings | View::Lyrics) {
            600
        } else {
            520
        };
        let queue = if self.right_sidebar_open() {
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
        if self.right_sidebar_open() && width - left - right - queue < minimum {
            self.queue_sidebar_open = false;
            self.device_menu_open = false;
        }
    }
    pub fn t<'a>(&'a self, value: &'a str) -> &'a str {
        self.translations
            .get(value)
            .map(String::as_str)
            .unwrap_or(value)
    }
    pub fn set_language(&mut self, preference: &str) {
        let index = match preference {
            "tr" => 1,
            "en" => 2,
            "az" => 3,
            _ => 0,
        };
        self.choices.insert("Language", index);
        self.language = if index == 0 {
            #[link(name = "kernel32")]
            unsafe extern "system" {
                fn GetUserDefaultUILanguage() -> u16;
            }
            match unsafe { GetUserDefaultUILanguage() } & 0x3ff {
                0x1f => "tr",
                0x2c => "az",
                _ => "en",
            }
            .into()
        } else {
            preference.to_owned()
        };
        let catalog = match self.language.as_str() {
            "tr" => include_str!("../../../../assets/localization/tr.json"),
            "az" => include_str!("../../../../assets/localization/az.json"),
            _ => "{}",
        };
        self.translations = std::sync::Arc::new(serde_json::from_str(catalog).unwrap_or_default());
    }
    pub(crate) fn translated_text(&self, text: &str) -> String {
        // Setting values carry a dropdown marker outside the translatable text.
        if let Some(value) = text.strip_suffix("   \u{25be}") {
            return format!("{}   \u{25be}", self.t(value));
        }
        if let Some(value) = self.translations.get(text) {
            return value.clone();
        }
        for (source, translated) in self
            .translations
            .iter()
            .filter(|(key, _)| key.contains("{0}"))
        {
            let mut rest = text;
            let mut pattern = source.as_str();
            let mut values = Vec::new();
            let mut matched = true;
            for index in 0..10 {
                let token = format!("{{{index}}}");
                let Some((prefix, tail)) = pattern.split_once(&token) else {
                    break;
                };
                let Some(after_prefix) = rest.strip_prefix(prefix) else {
                    matched = false;
                    break;
                };
                let delimiter = tail.split('{').next().unwrap_or(tail);
                let end = if delimiter.is_empty() {
                    after_prefix.len()
                } else if let Some(end) = after_prefix.find(delimiter) {
                    end
                } else {
                    matched = false;
                    break;
                };
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
        ui.spotify_configured = crate::session::provider_exists("spotify-soloist")
            || crate::spotify::soloist::Config::load().is_ok();
        let Some(base) = preferences_base() else {
            return ui;
        };
        let args: Vec<_> = std::env::args().collect();
        let import = args
            .windows(2)
            .find(|a| a[0] == "--appearance-profile")
            .map(|a| std::path::PathBuf::from(&a[1]));
        let original = import.clone().unwrap_or_else(|| base.join("DeezNats"));
        let own = crate::storage::data_dir_at(&base)
            .unwrap_or_else(|_| base.join("DeezNatsRust"))
            .join("ui.json");
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
            .unwrap_or_else(|_| value["Language"].as_str().unwrap_or("tr").to_owned());
        ui.set_language(&language);
        ui.auto_updates = value["AutoCheckUpdates"].as_bool().unwrap_or(true);
        ui.last_update_check = value["LastUpdateCheck"].as_u64().unwrap_or(0);
        ui.shortcuts =
            serde_json::from_value(value["KeyboardShortcuts"].clone()).unwrap_or([None; 7]);
        for binding in &mut ui.shortcuts {
            if binding.is_some_and(|binding| !binding.valid()) {
                *binding = None;
            }
        }
        ui.choices.insert(
            "Show on startup",
            value["ShowOnStartup"].as_u64().unwrap_or(0).min(5) as usize,
        );
        ui.view = ui.startup_view();
        ui.light = value["LightTheme"].as_bool().unwrap_or(false);
        ui.adaptive_colors = false;
        ui.initial_volume =
            (value["Volume"].as_f64().unwrap_or(0.7).clamp(0.0, 1.0) * 100.0).round() as u32;
        ui.last_provider = serde_json::from_value(value["LastProvider"].clone()).ok();
        ui.playlist_sorts =
            serde_json::from_value(value["PlaylistSorts"].clone()).unwrap_or_default();
        ui.restore_flow_preferences(&value);
        ui.pinned = serde_json::from_value(value["PinnedCollections"].clone()).unwrap_or_default();
        ui.blur = value["BackgroundBlur"].as_u64().unwrap_or(30).min(100) as u32;
        ui.brightness = value["LightBrightness"].as_u64().unwrap_or(50).min(110) as u32;
        ui.surface_opacity = value["SurfaceOpacity"].as_u64().unwrap_or(26).min(100) as u32;
        ui.light_opacity = value["LightOpacity"].as_u64().unwrap_or(10).min(100) as u32;
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
        ui.background_mode = match value["BackgroundMode"].as_u64().unwrap_or(2) {
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
            ("CustomDarkColors", &mut ui.custom_dark_colors),
            ("CustomLightColors", &mut ui.custom_light_colors),
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
            .min(SOLID_THEMES.len() as u64 + 1) as usize;
        ui.player_layout = value["PlayerLayout"].as_u64().unwrap_or(0).min(1) as usize;
        ui.transparent_player = true;

        ui.connect_port = value["ConnectPort"]
            .as_u64()
            .filter(|p| (1..=65535).contains(p))
            .unwrap_or(crate::connect::DEFAULT_PORT as u64) as u16;
        ui.jam_enabled = false;
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
        ui.choices.insert(
            "Background FPS",
            value["BackgroundFps"].as_u64().unwrap_or(3).min(3) as usize,
        );
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
            let Ok(directory) = crate::storage::data_dir_at(&base) else {
                return;
            };
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
            value["BackgroundFps"] = serde_json::json!(self.value("Background FPS"));
            value["ShowOnStartup"] = serde_json::json!(self.value("Show on startup"));
            value["LastProvider"] = serde_json::json!(self.last_provider);
            value["PinnedCollections"] = serde_json::json!(self.pinned);
            value["PlaylistSorts"] = serde_json::json!(self.playlist_sorts);
            self.write_flow_preferences(&mut value);
            value["CustomDarkColors"] = encode(&self.custom_dark_colors);
            value["CustomLightColors"] = encode(&self.custom_light_colors);
            value["ConnectPort"] = serde_json::json!(self.connect_port);
            value["JamEnabled"] = serde_json::json!(false);
            value["Autoplay"] = serde_json::json!(self.autoplay);
            value["AutoCheckUpdates"] = serde_json::json!(self.auto_updates);
            value["LastUpdateCheck"] = serde_json::json!(self.last_update_check);
            value["KeyboardShortcuts"] = serde_json::json!(self.shortcuts);
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
    pub fn is_subpage(&self) -> bool {
        match self.view {
            View::Album
            | View::Artist
            | View::Show
            | View::TrackInfo
            | View::PlaylistEdit
            | View::PlaylistPicker
            | View::CoverEdit
            | View::FlowTune
            | View::StyleMix
            | View::Storage
            | View::ArlHelp
            | View::CustomColors
            | View::Add => true,
            View::Offline => self.page.as_ref().is_some_and(|page| page.entity.is_some()),
            View::Explore => self.page.as_ref().is_some_and(|page| !page.is_discover()),
            _ => false,
        }
    }
    pub fn fallback_parent_view(&self) -> View {
        match self.view {
            View::FlowTune => View::Flow,
            View::Storage | View::CustomColors | View::ArlHelp => View::Settings,
            View::CoverEdit => View::PlaylistEdit,
            View::PlaylistEdit | View::PlaylistPicker => View::Playlists,
            View::Offline => View::Offline,
            View::Explore => View::Explore,
            _ => View::Home,
        }
    }
    pub fn lyrics_scale_percent(&self, w: i32) -> u32 {
        if self.lyrics_fullscreen || w >= 1400 {
            self.lyrics_size_fullscreen.clamp(60, 180)
        } else {
            self.lyrics_size_panel.clamp(60, 180)
        }
    }
    pub fn timed_lyrics(&self) -> bool {
        if !self.rich_lyrics.is_empty() {
            crate::lyrics::has_timing(&self.rich_lyrics)
        } else {
            self.lyrics.iter().any(|(time, _)| *time > 0)
        }
    }
    pub fn hold_lyric_follow(&mut self, now: std::time::Instant) {
        if self.view == View::Lyrics && self.timed_lyrics() {
            self.lyrics_manual_scroll = true;
            self.lyrics_manual_until = Some(now + std::time::Duration::from_secs(4));
        }
    }
    pub fn resume_lyric_follow(&mut self, now: std::time::Instant) -> bool {
        if self.view != View::Lyrics || !self.timed_lyrics() {
            self.lyrics_manual_until = None;
            return false;
        }
        if !self.lyrics_manual_scroll || self.lyrics_manual_until.is_some_and(|until| now < until) {
            return false;
        }
        self.lyrics_manual_scroll = false;
        self.lyrics_manual_until = None;
        true
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
        let view = [
            View::Home,
            View::Explore,
            View::Flow,
            View::Favorites,
            View::Playlists,
            View::Offline,
        ][self.value("Show on startup").min(5)];
        if view == View::Offline && !supports_downloads(self) {
            View::Home
        } else {
            view
        }
    }
    pub fn value(&self, key: &str) -> usize {
        if let Some(&v) = self.choices.get(key) {
            v
        } else if key == "Background FPS" {
            3
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
        "Background FPS" => &["Off", "90 FPS", "60 FPS", "30 FPS"],
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
    scene.label(x, y + 8, (w / 2).max(80), key, 0);
    let values = options(key);
    let value = if key == "Audio output device" {
        ui.devices
            .get(ui.value(key))
            .map(|(_, name)| name.as_str())
            .unwrap_or(values[0])
    } else {
        values[ui.value(key).min(values.len() - 1)]
    };
    scene.button(
        Rect::new(
            x + w - (w / 2).clamp(140, 240).min(w),
            y,
            (w / 2).clamp(140, 240).min(w),
            36,
        ),
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
fn settings_slider(
    scene: &mut Scene,
    ui: &Ui,
    x: i32,
    y: i32,
    w: i32,
    key: &'static str,
    value: u32,
) -> i32 {
    let slider_w = 180.min((w / 3).max(80));
    let slider_x = x + w - slider_w - 56;
    scene.label(x, y + 6, (slider_x - x - 12).max(0), ui.t(key), 0);
    scene
        .sliders
        .push((Rect::new(slider_x, y, slider_w, 30), key, value));
    scene.label(x + w - 48, y + 6, 48, format!("{}%", value), 0);
    y + 64
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
    let icon_offset = if matches!(title, "Spotify library" | "YouTube Music") {
        56
    } else {
        0
    };
    let text_w = if narrow {
        w - 32 - icon_offset
    } else {
        w - 180 - icon_offset
    };
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
    if icon_offset > 0 {
        scene.button(
            Rect::new(x + 16, y + 12, 40, 40),
            "",
            "",
            if title == "YouTube Music" {
                Action::YouTubeLogo
            } else {
                Action::SpotifyLogo
            },
            false,
        );
        if let Some(button) = scene.buttons.last_mut() {
            button.enabled = false;
            button.accessible = if title == "YouTube Music" {
                "YouTube Music"
            } else {
                "Spotify"
            }
            .into();
        }
    }
    let mut line_y = y + 10;
    for line in titles {
        scene.label_sized(x + 16 + icon_offset, line_y, text_w, 24, line, 0);
        line_y += 24;
    }
    for line in descriptions {
        scene.label_sized(x + 16 + icon_offset, line_y, text_w, 22, line, 1);
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

fn provider_card(
    scene: &mut Scene,
    ui: &Ui,
    x: i32,
    y: i32,
    w: i32,
    name: &str,
    logo: Action,
    profile: Option<&str>,
    saved: bool,
    login: Action,
    _refresh: Option<Action>,
    busy: bool,
) -> i32 {
    let narrow = w < 500;
    let text_w = if narrow { w - 104 } else { w - 292 };
    let status = match profile {
        Some(name) if !name.is_empty() => format!("{} {name}", ui.t("Connected as")),
        Some(_) => ui.t("Connected").into(),
        None if ui.provider_reauth[match logo {
            Action::DeezerLogo => 0,
            Action::SpotifyLogo => 1,
            _ => 2,
        }] =>
        {
            ui.t("Session expired. Sign in again.").into()
        }
        None if saved => ui.t("Sign-in data saved").into(),
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
        logo.clone(),
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
    scene.button(
        Rect::new(x + w - 148, action_y, 132, 34),
        ui.t(if matches!(login, Action::SwitchProvider(_)) {
            "Switch"
        } else if profile.is_some() {
            "Log out"
        } else {
            "Log in"
        }),
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

fn spotify_provider_action(ui: &Ui) -> (&'static str, Action, bool) {
    use crate::spotify::service::Phase;
    if ui.spotify_setup_busy {
        return ("Setting up...", Action::SpotifySetup, false);
    }
    match ui.spotify_phase {
        Some(Phase::Starting | Phase::Connecting) => {
            ("Starting Soloist...", Action::StartSpotify, false)
        }
        Some(Phase::Pairing) => ("Open Spotify", Action::OpenSpotify, true),
        None | Some(Phase::Stopped) if ui.spotify_configured => {
            ("Start Spotify", Action::StartSpotify, true)
        }
        _ if ui.spotify_configured => ("Reset", Action::SpotifySetup, true),
        _ => ("Set up Spotify", Action::SpotifySetup, true),
    }
}
fn spotify_catalog_card(
    scene: &mut Scene,
    ui: &Ui,
    x: i32,
    y: i32,
    w: i32,
    connected: bool,
) -> i32 {
    let active = ui.provider == crate::catalog::Provider::Spotify;
    let signed_in = active && connected && !ui.spotify_reset_failed && !ui.provider_reauth[1];
    let name = crate::catalog::string(&ui.account, &["display_name", "name", "userName", "userId"]);
    let saved =
        crate::spotify::api_configured() && !ui.spotify_reset_failed && !ui.provider_reauth[1];
    let action = if !active && saved {
        Action::SwitchProvider(crate::catalog::Provider::Spotify)
    } else if signed_in {
        Action::SpotifyLogout
    } else {
        Action::SpotifyLogin
    };
    let mut cy = y;
    cy = provider_card(
        scene,
        ui,
        x,
        cy,
        w,
        "Spotify library",
        Action::SpotifyLogo,
        signed_in.then_some(name.as_str()),
        saved,
        action,
        None,
        ui.spotify_catalog_busy,
    );
    if active {
        let indent = 20.min((w / 10).max(0));
        cy = spotify_setup_card(scene, ui, x + indent, cy, (w - indent).max(1), signed_in);
    }
    cy
}

fn spotify_setup_card(scene: &mut Scene, ui: &Ui, x: i32, y: i32, w: i32, signed_in: bool) -> i32 {
    let narrow = w < 500;
    let status = if ui.spotify_setup_status.is_empty() {
        if ui.spotify_configured {
            "Restore saved pairing."
        } else {
            "Set up Soloist playback"
        }
    } else {
        &ui.spotify_setup_status
    };
    let left_button = ui.spotify_configured;
    let left_width = if left_button {
        86.min((w / 3).max(1))
    } else {
        0
    };
    let text_x = x + 16 + left_width;
    let text_w = if narrow {
        (w - 32 - left_width).max(1)
    } else {
        (w - 190 - left_width).max(1)
    };
    let lines = wrap_text(ui.t(status), text_w, 14);
    let height =
        (42 + lines.len().max(1) as i32 * 22).max(78) + if narrow && !left_button { 48 } else { 0 };
    scene.panel(x, y, w, height, 8);
    if left_button {
        scene.button(
            Rect::new(x + 8, y + 12, (left_width - 8).max(1), 34),
            "Reset",
            "",
            Action::SpotifyRefresh,
            false,
        );
        scene.buttons.last_mut().unwrap().enabled =
            signed_in && !ui.spotify_catalog_busy && !ui.spotify_setup_busy;
    }
    scene.label_sized(text_x, y + 10, text_w, 26, "Soloist audio device", 0);
    for (index, line) in lines.into_iter().enumerate() {
        scene.label_sized(text_x, y + 36 + index as i32 * 22, text_w, 22, line, 1);
    }
    if !left_button {
        let (label, action, enabled) = spotify_provider_action(ui);
        scene.button(
            Rect::new(
                x + w - 156.min(w - 16),
                if narrow {
                    y + height - 42
                } else {
                    y + (height - 34) / 2
                },
                140.min((w - 32).max(1)),
                34,
            ),
            label,
            "",
            action,
            false,
        );
        scene.buttons.last_mut().unwrap().enabled = enabled;
    }
    y + height + 8
}

fn settings(scene: &mut Scene, ui: &Ui, x: i32, y: i32, w: i32, connected: bool, busy: bool) {
    let narrow = w < 600;
    let cols = if narrow {
        TABS.len() as i32
    } else {
        (w / 140).clamp(2, TABS.len() as i32)
    };
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
            field(scene, ui, x, cy, w, "Background FPS");
            cy += 48;
            cy += 16;
            scene.label(x, cy, w, "STORAGE", 9);
            cy += 28;
            if supports_downloads(ui) {
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
                    "Library storage",
                    &ui.library_storage,
                    "Scan",
                    Action::LibraryMaintenance(false),
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
                    Action::RemoveDownloads,
                );
                cy = settings_action_card(
                    scene,
                    ui,
                    x,
                    cy,
                    w,
                    "Orphaned media files",
                    "Remove unreferenced files older than 24 hours",
                    "Clean up",
                    Action::LibraryMaintenance(true),
                );
                cy = settings_action_card(
                    scene,
                    ui,
                    x,
                    cy,
                    w,
                    "Missing downloads",
                    "Download missing or incomplete saved audio again",
                    "Repair",
                    Action::RepairDownloads,
                );
            }
            scene.button(
                Rect::new(x, cy, 270.min(w), 36),
                "Choose data / cache location...",
                "",
                Action::Go(View::Storage),
                false,
            );
            cy += 64;
            scene.label(x, cy, w, "UPDATES", 9);
            cy += 28;
            scene.label(
                x,
                cy,
                w,
                format!(
                    "{} {}",
                    ui.t("Installed version:"),
                    env!("CARGO_PKG_VERSION")
                ),
                0,
            );
            cy += 30;
            cy = toggle_row(
                scene,
                ui,
                x,
                cy,
                w,
                "Automatically check for updates",
                "Check once a day without popups",
                "auto_updates",
                ui.auto_updates,
            );
            for line in wrap_text(ui.t(ui.update.status()), w, 14) {
                notice(scene, x, cy, w, &line);
                cy += 22;
            }
            if !ui.update.version.is_empty() {
                scene.label(
                    x,
                    cy,
                    w,
                    format!("{} {}", ui.t("New version:"), ui.update.version),
                    0,
                );
                cy += 28;
            }
            if ui.update.phase == crate::updater::Phase::Downloading {
                scene.label(x, cy, w, format!("{}%", ui.update.progress), 0);
                cy += 28;
            }
            for line in wrap_text(&ui.update.error, w, 14) {
                notice(scene, x, cy, w, &line);
                cy += 22;
            }
            if !ui.update.busy() {
                scene.button(
                    Rect::new(x, cy, w.min(180), 36),
                    "Check now",
                    "",
                    Action::CheckUpdate,
                    false,
                );
                cy += 48;
                let (label, action) = if ui.update.phase == crate::updater::Phase::Ready {
                    ("Restart to apply", Some(Action::ApplyUpdate))
                } else if ui.update.phase == crate::updater::Phase::Available {
                    ("Download update", Some(Action::DownloadUpdate))
                } else {
                    ("", None)
                };
                if let Some(action) = action {
                    scene.button(Rect::new(x, cy, w.min(220), 36), label, "", action, true);
                    cy += 48;
                }
            }
            scene.content_height = cy + 32 + ui.scroll;
        }
        1 => {
            let mut cy = top;

            let name = crate::catalog::string(
                &ui.account,
                &["name", "display_name", "userName", "userId"],
            );
            let spotify_first = (connected && ui.provider == crate::catalog::Provider::Spotify)
                || (!connected
                    && crate::spotify::api_configured()
                    && !ui.spotify_reset_failed
                    && !crate::session::deezer_saved_exists());
            if spotify_first {
                cy = spotify_catalog_card(scene, ui, x, cy, w, connected) + 8;
            }
            cy = provider_card(
                scene,
                ui,
                x,
                cy,
                w,
                "Deezer",
                Action::DeezerLogo,
                (connected
                    && ui.provider == crate::catalog::Provider::Deezer
                    && !ui.provider_reauth[0])
                    .then_some(name.as_str()),
                crate::session::deezer_saved_exists(),
                if ui.provider != crate::catalog::Provider::Deezer
                    && crate::session::deezer_saved_exists()
                {
                    Action::SwitchProvider(crate::catalog::Provider::Deezer)
                } else if ui.provider != crate::catalog::Provider::Deezer {
                    Action::DeezerLogin
                } else if connected && !ui.provider_reauth[0] {
                    Action::Logout
                } else {
                    Action::DeezerLogin
                },
                None,
                busy,
            );
            if !spotify_first {
                cy = spotify_catalog_card(scene, ui, x, cy + 8, w, connected);
            }
            let active = ui.provider == crate::catalog::Provider::YouTube;
            let saved = crate::session::provider_exists("youtube") && !ui.provider_reauth[2];
            let action = if !active && saved {
                Action::SwitchProvider(crate::catalog::Provider::YouTube)
            } else if connected && saved {
                Action::YouTubeLogout
            } else {
                Action::YouTubeLogin
            };
            cy = provider_card(
                scene,
                ui,
                x,
                cy + 8,
                w,
                "YouTube Music",
                Action::YouTubeLogo,
                (active && connected && saved).then_some(name.as_str()),
                saved,
                action,
                None,
                busy,
            );
            scene.content_height = cy + 40 + ui.scroll;
        }
        2 => {
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
                cy = settings_slider(scene, ui, x, cy, w, "Image blur", ui.blur);
            }
            if ui.background_mode != 3 {
                cy = settings_slider(
                    scene,
                    ui,
                    x,
                    cy,
                    w,
                    "Background tint opacity",
                    ui.light_opacity,
                );
                cy = settings_slider(scene, ui, x, cy, w, "Background brightness", ui.brightness);
            }
            cy = settings_slider(scene, ui, x, cy, w, "Surface opacity", ui.surface_opacity);
            scene.label(x, cy, w, "THEMES", 9);
            cy += 28;
            let nav_w = 32.min((w / 6).max(1));
            let shelf_x = x + nav_w + 6;
            let shelf_w = (w - 2 * (nav_w + 6)).max(1);
            let cols = (shelf_w / 168).clamp(1, 4) as usize;
            let card_w = (shelf_w - (cols as i32 - 1) * 10) / cols as i32;
            let total = SOLID_THEMES.len() + 2;
            let offset = ui.theme_shelf_offset.min(total - 1);
            if offset > 0 {
                scene.button(
                    Rect::new(x, cy + 40, nav_w, 36),
                    "<",
                    "",
                    Action::ThemeShelf(-1),
                    false,
                );
            }
            for cell in 0..cols.min(total - offset) {
                let index = offset + cell;
                let card_x = shelf_x + cell as i32 * (card_w + 10);
                if let Some((name, ..)) = SOLID_THEMES.get(index) {
                    scene.button(
                        Rect::new(card_x, cy, card_w, 132),
                        *name,
                        "",
                        Action::SolidTheme(index),
                        ui.solid_theme == index,
                    );
                } else {
                    let light = index == SOLID_THEMES.len() + 1;
                    let label = if light { "Custom light" } else { "Custom dark" };
                    scene.button(
                        Rect::new(card_x, cy, card_w, 132),
                        label,
                        "",
                        Action::CustomTheme(light),
                        ui.solid_theme == index && ui.light == light,
                    );
                    scene.button(
                        Rect::new(card_x + 12, cy + 134, 54.min((card_w - 24).max(1)), 22),
                        "Edit",
                        "",
                        Action::CustomTheme(light),
                        false,
                    );
                }
            }
            if offset + cols < total {
                scene.button(
                    Rect::new(x + w - nav_w, cy + 40, nav_w, 36),
                    ">",
                    "",
                    Action::ThemeShelf(1),
                    false,
                );
            }
            cy += 172;
            scene.button(
                Rect::new(x, cy, (w - 12) / 2, 36),
                "Import theme",
                "",
                Action::ImportTheme,
                false,
            );
            scene.button(
                Rect::new(x + (w + 12) / 2, cy, (w - 12) / 2, 36),
                "Export theme",
                "",
                Action::ExportTheme,
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
                if idx == 2 || idx == 6 {
                    continue;
                }
                let source = if idx == 0 && ui.provider == crate::catalog::Provider::Spotify {
                    &"Spotify"
                } else {
                    source
                };
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
            if supports_downloads(ui) {
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
            }
            if ui.provider != crate::catalog::Provider::Spotify {
                cy = toggle_row(
                    scene,
                    ui,
                    x,
                    cy,
                    w,
                    "Autoplay",
                    "Continue with related songs, one at a time, when the queue ends",
                    "autoplay",
                    ui.autoplay,
                );
            } else {
                scene.label(
                    x,
                    cy,
                    w,
                    "Manage autoplay in the Spotify app (Soloist account setting)",
                    1,
                );
                cy += 38;
            }
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
            if supports_downloads(ui) {
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
            } else if ui.provider == crate::catalog::Provider::YouTube {
                for message in [
                    "YouTube Music uses the Windows default output device when playback opens.",
"Streams use the best audio format available to your account. Local volume and equalizer do not affect Connect listeners or visualizers. Crossfade and offline downloads are unavailable for this provider.",
                ] {
                    for line in wrap_text(ui.t(message), w, 14) {notice(scene, x, cy, w, &line); cy += 22;}
                    cy += 8;
                }
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
                scene.eq_graph = Some(Rect::new(x, cy, w, 160));
                scene.panel(x, cy, w, 160, 7);
                cy += 176;
                scene.eq_sliders = Some(Rect::new(x, cy, w, 170));
                scene.panel(x, cy, w, 170, 13);
                cy += 190;
            } else {
                for message in [
                    "Local Spotify playback follows the Windows default output. Audio reconnects automatically when the default device changes or reconnects. Windows per-app output overrides still apply.",
                    "If audio gets stuck, use Reconnect audio output. To choose a specific device, open Windows Volume mixer while playing and look for Soloist.",
                    "Soloist does not expose equalizer, normalization, crossfade or offline downloads. Its playback cache is not an offline library.",
                    "For a remote Spotify Connect device, choose the output on that device.",
                ] {
                    for line in wrap_text(ui.t(message), w, 14) {
                        notice(scene, x, cy, w, &line);
                        cy += 22;
                    }
                    cy += 8;
                }
                scene.button(
                    Rect::new(x, cy, w.min(280), 34),
                    "Windows Volume mixer",
                    "",
                    Action::OpenSpotifyOutputSettings,
                    false,
                );
                cy += 46;
                scene.button(
                    Rect::new(x, cy, w.min(280), 34),
                    "Reconnect audio output",
                    "",
                    Action::ReconnectSpotifyOutput,
                    false,
                );
                cy += 46;
            }

            scene.content_height = cy + 20 + ui.scroll;
        }
        5 => {
            // Integrations
            let mut cy = top;
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
            scene.content_height = cy + 40 + ui.scroll;
        }
        _ => {
            let mut cy = top;
            scene.label(x, cy, w, "KEYBOARD SHORTCUTS", 9);
            cy += 28;
            for line in wrap_text(ui.t("Media keys work automatically through Windows media controls. Volume keys use Windows volume."), w, 14) {
                notice(scene, x, cy, w, &line); cy += 22;
            }
            cy += 12;
            for line in wrap_text(ui.t("Add optional global shortcuts below. Use Ctrl, Alt or Win with a key, or a function key. Esc cancels; Backspace removes a custom shortcut."), w, 14) {
                notice(scene, x, cy, w, &line); cy += 22;
            }
            cy += 20;
            for (index, action) in crate::shortcuts::ACTIONS.iter().enumerate() {
                scene.label(x, cy, w, ui.t(action), 3);
                cy += 30;
                let binding = if ui.shortcut_capture == Some(index) {
                    ui.t("Press a shortcut...").to_owned()
                } else {
                    ui.shortcuts[index]
                        .map(|binding| binding.label())
                        .unwrap_or_else(|| ui.t(crate::shortcuts::DEFAULTS[index]).into())
                };
                let reset_w = 90.min(w / 3);
                scene.button(
                    Rect::new(x, cy, w - reset_w - 12, 36),
                    binding,
                    "",
                    Action::BindShortcut(index),
                    ui.shortcut_capture == Some(index),
                );
                scene.button(
                    Rect::new(x + w - reset_w, cy, reset_w, 36),
                    "Reset",
                    "",
                    Action::ResetShortcut(index),
                    false,
                );
                cy += 48;
                if !ui.shortcut_errors[index].is_empty() {
                    for line in wrap_text(ui.t(&ui.shortcut_errors[index]), w, 14) {
                        notice(scene, x, cy, w, &line);
                        cy += 22;
                    }
                    cy += 10;
                }
            }
            scene.content_height = cy + 40 + ui.scroll;
        }
    }
    scene.content_buttons = content_start.0..scene.buttons.len();
    scene.content_labels = content_start.1..scene.labels.len();
    scene.content_panels = content_start.2..scene.panels.len();
    scene.content_pictures = picture_start..scene.pictures.len();
}
fn provider_name(ui: &Ui) -> &'static str {
    match ui.provider {
        crate::catalog::Provider::Deezer => "Deezer",
        crate::catalog::Provider::Spotify => "Spotify",
        crate::catalog::Provider::YouTube => "YouTube Music",
    }
}

fn supports_downloads(ui: &Ui) -> bool {
    ui.provider == crate::catalog::Provider::Deezer
}

fn view_title(ui: &Ui, view: View) -> &'static str {
    if ui.provider == crate::catalog::Provider::YouTube && view == View::Account {
        return "Sign in to YouTube Music";
    }
    if ui.provider == crate::catalog::Provider::Spotify {
        match view {
            View::Favorites => "Liked songs",
            View::Account => "Sign in to Spotify",
            _ => view.title(),
        }
    } else {
        view.title()
    }
}

fn artist_audience(raw: &serde_json::Value, ui: &Ui) -> String {
    let mut parts = Vec::new();
    for (keys, nested, label) in [
        (
            &["nb_fan", "fans", "followers", "followers_count"][..],
            "/followers/total",
            "followers",
        ),
        (
            &["monthly_listeners", "monthlyListeners"][..],
            "/stats/monthlyListeners",
            "monthly listeners",
        ),
    ] {
        let value = crate::catalog::string(raw, keys);
        let value = if value.is_empty() {
            raw.pointer(nested)
                .and_then(|v| v.as_u64())
                .map(|v| v.to_string())
                .unwrap_or_default()
        } else {
            value
        };
        if !value.is_empty() {
            let count = value
                .parse::<u64>()
                .map(crate::catalog::format_count)
                .unwrap_or(value);
            parts.push(format!("{count} {}", ui.t(label)));
        }
    }
    parts.join(" \u{00b7} ")
}

fn entity_caption<'a>(
    entity: &'a crate::catalog::Entry,
    page: Option<&'a crate::catalog::Page>,
    fallback: &'a str,
    ui: &'a Ui,
) -> String {
    if entity.kind == "artist" {
        let audience = artist_audience(&entity.raw, ui);
        if !audience.is_empty() {
            return audience;
        }
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
                if count == 1 {
                    "episode"
                } else {
                    "episodes"
                }
            } else if count == 1 {
                "track"
            } else {
                "tracks"
            });
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
fn cached_lyric_ranges(text: &str, width: i32, font: f32) -> Vec<std::ops::Range<usize>> {
    thread_local! {
        static CACHE: std::cell::RefCell<std::collections::HashMap<(String, i32, u32), Vec<std::ops::Range<usize>>>> = std::cell::RefCell::new(std::collections::HashMap::new());
    }
    CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        let key = (text.to_owned(), width, font.to_bits());
        if let Some(ranges) = cache.get(&key) {
            return ranges.clone();
        }
        if cache.len() >= 4096 {
            cache.clear();
        }
        let ranges = crate::lyrics::wrapped_ranges(text, width, font);
        cache.insert(key, ranges.clone());
        ranges
    })
}

pub(crate) fn sorted_page_rows<'a>(
    ui: &Ui,
    page: &'a crate::catalog::Page,
) -> Vec<&'a crate::catalog::Entry> {
    let is_podcast = ui.view == View::Show
        || page.entity.as_ref().is_some_and(|e| e.kind == "podcast")
        || (!page.rows.is_empty() && page.rows.iter().all(|e| e.kind == "episode"));
    if ui.sort_column == 0 {
        return page.rows.iter().collect();
    }
    thread_local! {
        static SORTED_ROWS: std::cell::RefCell<Option<(Vec<crate::catalog::Entry>, usize, bool, bool, Vec<usize>)>> = const { std::cell::RefCell::new(None) };
    }
    let cached = SORTED_ROWS.with(|cache| {
        let cache = cache.borrow();
        cache
            .as_ref()
            .filter(|(entries, column, descending, podcast, _)| {
                *column == ui.sort_column
                    && *descending == ui.sort_descending
                    && *podcast == is_podcast
                    && entries == &page.rows
            })
            .map(|(_, _, _, _, indices)| indices.clone())
    });
    if let Some(indices) = cached {
        return indices.into_iter().map(|index| &page.rows[index]).collect();
    }
    let mut rows: Vec<usize> = (0..page.rows.len()).collect();
    if ui.sort_column > 0 {
        rows.sort_by(|&a, &b| {
            let (a, b) = (&page.rows[a], &page.rows[b]);
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
    SORTED_ROWS.with(|cache| {
        *cache.borrow_mut() = Some((
            page.rows.clone(),
            ui.sort_column,
            ui.sort_descending,
            is_podcast,
            rows.clone(),
        ))
    });
    rows.into_iter().map(|index| &page.rows[index]).collect()
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
                "played_at",
                "added_at",
                "timestampMs",
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
fn queue_button(
    s: &mut Scene,
    entry: &crate::catalog::Entry,
    index: usize,
    rect: Rect,
    current: bool,
    draggable: bool,
) {
    let text_w = queue_text_width(entry, rect.w);
    s.button(rect, "", "", Action::QueueJump(index), current);
    if let Some(btn) = s.buttons.last_mut() {
        btn.accessible = format!(
            "{}: {}",
            if current { "Now playing" } else { "Play next" },
            queue_lines(entry, text_w).join(" ")
        );
    }
    if draggable {
        s.label_sized(rect.x + 5, rect.y + 20, 18, 24, "\u{22ee}\u{22ee}", 1);
    }
    // Artwork has its own bounds: button artwork fills the entire button.
    if rect.h >= 64 {
        s.pictures.push((
            Rect::new(rect.x + 26, rect.y + 10, 44, 44),
            entry.artwork.clone(),
        ));
    }
    if rect.h >= 64 {
        for (line, text) in queue_lines(entry, text_w).into_iter().enumerate() {
            s.label_sized(
                rect.x + 82,
                rect.y + 10 + line as i32 * 24,
                text_w,
                24,
                text,
                if line == 0 { 0 } else { 1 },
            );
        }
    }
}
fn build_queue_sidebar(s: &mut Scene, ui: &Ui, qx: i32, qw: i32, h: i32) {
    let start = s.labels.len();
    let buttons = s.buttons.len();
    let panels = s.panels.len();
    let pictures = s.pictures.len();
    build_queue_sidebar_contents(s, ui, qx, qw, h);
    s.queue_labels = start..s.labels.len();
    s.queue_buttons = buttons..s.buttons.len();
    s.queue_panels = panels..s.panels.len();
    s.queue_pictures = pictures..s.pictures.len();
}
fn build_queue_sidebar_contents(s: &mut Scene, ui: &Ui, qx: i32, qw: i32, h: i32) {
    let bottom = h - if ui.view == View::Lyrics { 12 } else { 96 };
    s.queue_frame = Some(Rect::new(qx, 10, qw, bottom - 20));
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
    let items: &[crate::catalog::Entry] = if !ui.queue.is_empty() {
        &ui.queue
    } else if !ui.now.id.is_empty() {
        std::slice::from_ref(&ui.now)
    } else {
        &[]
    };
    if items.is_empty() {
        s.label(qx + 18, y + 24, qw - 36, "Queue is empty", 1);
        return;
    }
    let cur = if ui.queue_index < items.len()
        && (ui.now.id.is_empty() || items[ui.queue_index].id == ui.now.id)
    {
        ui.queue_index
    } else if !ui.now.id.is_empty() {
        items
            .iter()
            .position(|e| e.id == ui.now.id)
            .unwrap_or_else(|| ui.queue_index.min(items.len().saturating_sub(1)))
    } else {
        ui.queue_index.min(items.len().saturating_sub(1))
    };
    s.label(qx + 18, y, qw - 36, "QUEUE", 9);
    y += 28;
    let viewport = Rect::new(qx + 12, y, qw - 24, (bottom - 18 - y).max(1));
    s.queue_viewport = Some(viewport);
    let upcoming_start = 0;
    let upcoming = items;
    let total = (upcoming.len().min((i32::MAX / 72) as usize) as i32) * 72;
    let max = ((total - viewport.h).max(0) + 23) / 24;
    let offset = ui.queue_scroll.min(max as usize) as i32;
    s.queue_scroll_max = max as usize;
    s.queue_anchor = cur
        .saturating_mul(72)
        .saturating_sub((viewport.h / 4) as usize)
        .min(max as usize * 24);
    let fractional_offset = if offset < max {
        ui.queue_scroll_offset.min(23) as i32
    } else {
        0
    };
    let pixels = offset * 24 + fractional_offset;
    s.queue_layout_scroll = pixels;
    s.queue_pixel_scroll = pixels;
    let first = (pixels.saturating_sub(144).max(0) / 72) as usize;
    let mut row_y = y + first as i32 * 72 - pixels;
    let row_labels = s.labels.len();
    let row_buttons = s.buttons.len();
    for (idx, entry) in upcoming.iter().enumerate().skip(first) {
        let idx = upcoming_start + idx;
        let height = 72;
        if row_y >= viewport.y + viewport.h + 144 {
            break;
        }
        let top = row_y.max(viewport.y - 144);
        let end = (row_y + height - 6).min(viewport.y + viewport.h + 144);
        if end > top {
            let rect = Rect::new(qx + 12, row_y, qw - 38, height - 6);
            queue_button(s, entry, idx, rect, idx == cur, idx > cur);
        }
        row_y += height;
    }
    s.queue_row_labels = row_labels..s.labels.len();
    s.queue_row_buttons = row_buttons..s.buttons.len();
    if max > 0 {
        let track = Rect::new(qx + qw - 20, viewport.y, 8, viewport.h);
        let thumb = (track.h * track.h / total.max(1)).max(24).min(track.h);
        s.queue_scrollbar = Some(track);
        s.panel(
            track.x,
            track.y
                + (((offset as i64 * 24 + fractional_offset as i64) * (track.h - thumb) as i64)
                    / (max as i64 * 24)) as i32,
            8,
            thumb,
            17,
        );
    }
    if ((offset * 24 + fractional_offset) - s.queue_anchor as i32).abs() >= 216 {
        s.button(
            Rect::new(qx + (qw - 36) / 2, viewport.y + viewport.h - 38, 36, 32),
            "",
            "\u{e72e}",
            Action::QueueReturn,
            false,
        );
        if let Some(button) = s.buttons.last_mut() {
            button.accessible = "Return to now playing".into();
        }
    }
    if upcoming.is_empty() {
        s.label(qx + 18, y + 8, qw - 36, "No upcoming songs", 1);
    }
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
    let mode_mix = ui
        .lyrics_cover_mix
        .unwrap_or(if ui.lyrics_cover_mode { 1. } else { 0. })
        .clamp(0., 1.);
    let left_x = 32 + (col_w - cover_size) / 2;
    let offset_x = (((w - cover_size) / 2 - left_x) as f32 * mode_mix).round() as i32;
    let cover_x = left_x + offset_x;
    let block_h = cover_size + 130;
    let left_y = ((h - block_h) / 2).clamp(40, 90);
    let cover_y =
        left_y + ((((h - cover_size - 160) / 2).max(40) - left_y) as f32 * mode_mix).round() as i32;

    let is_fs = ui.lyrics_fullscreen;
    let cover_rect = Rect::new(cover_x, cover_y, cover_size, cover_size);
    if !is_fs {
        s.lyrics_cover = Some(cover_rect);

        if !ui.now.artwork.is_empty() {
            s.pictures.push((
                cover_rect,
                crate::catalog::cover_artwork_key(&ui.now.artwork),
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
        s.label_sized(24 + offset_x, title_y, col_w, 32, title_text, 10);

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
        s.artist_links(Rect::new(24 + offset_x, artist_y, col_w, 24), &ui.now, true);

        // Provider attribution
        if !ui.lyrics_provider.is_empty() {
            s.label_sized(
                24 + offset_x,
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

    s.lyrics_area = Some(Rect::new(right_x, 72, right_w, (h - 144).max(1)));

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

    let visualizer_label = if !ui.lyrics_visualizer {
        "Off"
    } else {
        match ui.visualizer_style {
            1 => "Bars",
            2 => "Trap Nation",
            _ => "Wave",
        }
    };
    s.button(
        Rect::new(32, h - 54, 156, 32),
        ui.t(visualizer_label),
        "\u{e9e9}",
        Action::ToggleLyricsVisualizer,
        ui.lyrics_visualizer,
    );
    if let Some(button) = s.buttons.last_mut() {
        button.accessible = format!("Visualizer: {visualizer_label}");
    }
    if ui.lyrics_cover_mode || mode_mix > 0.01 {
        return;
    }

    let lyrics_btn_start = s.buttons.len();
    let lyrics_lbl_start = s.labels.len();

    let scale_pct = ui.lyrics_scale_percent(w);
    let area = s.lyrics_area.unwrap();
    let center_y = area.y + (area.h as f32 * 0.3).round() as i32;

    if ui.rich_lyrics.is_empty() && ui.lyrics.is_empty() {
        s.label(
            right_x,
            center_y - 20,
            right_w,
            if ui.lyrics_loading {
                "Loading lyrics..."
            } else if ui.lyrics_provider == "Instrumental" {
                "This song is instrumental."
            } else {
                "No lyrics returned for this track."
            },
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
                cached_lyric_ranges(&line.text, right_w - 12, font_px),
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
            // Untimed lyrics use the same focused-row styling while the reader
            // scrolls; playback time never invents a singing position.
            let mut offset = 0;
            blocks
                .iter()
                .enumerate()
                .min_by_key(|(_, block)| {
                    let distance = (offset - ui.scroll).abs();
                    offset += block.2.len() as i32 * row_h + spacing;
                    distance
                })
                .map(|(index, _)| index)
                .unwrap_or(0)
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
            center_y - ui.scroll
        };
        // Retain the focus even when its verse is outside the virtualized rows.
        s.lyrics_focus_y = timed.then_some(ly + anchor);
        let mut total = 0;
        for (block_index, (start, source, ranges)) in blocks.iter().enumerate() {
            let height = ranges.len() as i32 * row_h;
            if let Some(source) = source {
                let line = &lines[*source];
                let active_line = block_index == active && active_gap.is_none();
                for range in ranges {
                    // Buffer a full screen for scrolling and automatic lyric motion.
                    if ly + row_h < -h || ly > h * 2 {
                        ly += row_h;
                        continue;
                    }
                    let kind = if active_line { 13 } else { 12 };
                    s.label_sized(right_x, ly, right_w, row_h, &line.text[range.clone()], kind);
                    s.lyric_rows.push(LyricRow {
                        label: s.labels.len() - 1,
                        line: *source,
                        range: range.clone(),
                        button: timed.then_some(s.buttons.len()),
                        lag: 0.0,
                        velocity: 0.0,
                    });
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
        s.content_height = total + h;
    }

    s.content_buttons = lyrics_btn_start..s.buttons.len();
    s.content_labels = lyrics_lbl_start..s.labels.len();
}

fn build_track_info(s: &mut Scene, ui: &Ui, x: i32, top: i32, cw: i32) {
    track_info::build(s, ui, x, top, cw);
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
    if ui.onboarding_open {
        build_onboarding(&mut s, ui, w, h);
        if ui.deezer_login_open {
            build_deezer_login(&mut s, ui, w, h);
        }
        translate_scene(&mut s, ui);
        return s;
    }
    if ui.spotify_credentials_refreshing {
        let width = (w - 64).clamp(1, 600);
        let left = (w - width) / 2;
        let top = (h / 2 - 35).max(30);
        let message = match ui.provider {
            crate::catalog::Provider::YouTube => "Refreshing YouTube credentials...",
            crate::catalog::Provider::Deezer => "Refreshing Deezer credentials...",
            _ => "Refreshing Spotify credentials...",
        };
        s.label(left, top, width, message, 3);
        s.label(left, top + 42, width, "This may take a moment.", 1);
        translate_scene(&mut s, ui);
        return s;
    }
    let is_lyrics = ui.view == View::Lyrics;
    let (expansion, right_expansion) = ui.sidebar_values();
    let side = if is_lyrics {
        0
    } else {
        (56. + 158. * expansion).round() as i32
    };
    let x = if is_lyrics {
        32
    } else {
        (76. + 186. * expansion).round() as i32
    };
    let queue_w = (280.min(w / 3) as f32 * right_expansion).round() as i32;
    let cw = w - x - queue_w - if is_lyrics { 32 } else { 40 };
    let (x, cw) = if ui.view == View::Settings {
        let settings_width = cw.min(800);
        (x + (cw - settings_width) / 2, settings_width)
    } else {
        (x, cw)
    };
    let bottom = h - player_height(ui, w) - 8;
    let step = 41;
    if !is_lyrics {
        if expansion > 0.5 {
            s.label(14, 22, 170, "MUSIC", 9);
        }
        let navigation: Vec<_> = NAV
            .iter()
            .filter(|(view, _)| *view != View::Offline || supports_downloads(ui))
            .collect();
        for (i, (view, glyph)) in navigation.iter().copied().enumerate() {
            let y = if i < 4 {
                42 + i as i32 * step
            } else {
                66 + 4 * step + (i as i32 - 4) * step
            };
            if i == 4 && expansion > 0.5 {
                s.label(14, 48 + 4 * step, 170, "LIBRARY", 9);
            }
            if expansion > 0.5 {
                s.button(
                    Rect::new(
                        10 + (3. * expansion).round() as i32,
                        y,
                        (38. + 140. * expansion).round() as i32,
                        step - 4,
                    ),
                    if *view == View::Home {
                        "Home"
                    } else {
                        view_title(ui, *view)
                    },
                    glyph,
                    Action::Go(*view),
                    *view == ui.view,
                );
            } else {
                s.button(
                    Rect::new(
                        10 + (3. * expansion).round() as i32,
                        y,
                        (38. + 140. * expansion).round() as i32,
                        step - 4,
                    ),
                    "",
                    glyph,
                    Action::Go(*view),
                    *view == ui.view,
                );
            }
        }
        let settings_y = 66 + 4 * step + (navigation.len() as i32 - 4) * step;
        if expansion > 0.5 {
            s.button(
                Rect::new(
                    10 + (3. * expansion).round() as i32,
                    settings_y,
                    (38. + 140. * expansion).round() as i32,
                    step - 4,
                ),
                "Settings",
                "\u{e713}",
                Action::Go(View::Settings),
                ui.view == View::Settings,
            );
        } else {
            s.button(
                Rect::new(
                    10 + (3. * expansion).round() as i32,
                    settings_y,
                    (38. + 140. * expansion).round() as i32,
                    step - 4,
                ),
                "",
                "\u{e713}",
                Action::Go(View::Settings),
                ui.view == View::Settings,
            );
        }
        let main_panel_w = w - side - queue_w - 24 - (12. * right_expansion).round() as i32;
        s.panel(side + 12, 10, main_panel_w, bottom - 20, 0);
        if !ui.history.is_empty() || ui.is_subpage() {
            s.button(
                Rect::new(x, 32, 36, 36),
                "",
                "\u{e72b}",
                Action::Back,
                false,
            );
            if let Some(button) = s.buttons.last_mut() {
                button.accessible = "Back".into();
            }
        }
        if ui.view == View::Explore {
            let search_w = (cw - 44).max(36);
            s.panel(x + 44, 30, search_w, 40, 8);
            s.search = Some(Rect::new(x + 54, 37, (search_w - 44).max(12), 26));
            s.button(
                Rect::new(x + 44 + search_w - 34, 32, 34, 34),
                "",
                "\u{e721}",
                Action::Search,
                false,
            );
        }
    }
    if queue_w > 0 {
        let qx = w - queue_w - (12. * right_expansion).round() as i32;
        let panel_w = 280.min(w / 3);
        if ui.visible_device_sidebar() {
            let first = s.labels.len();
            device_sidebar::build(&mut s, ui, qx, panel_w, h, playing);
            s.queue_labels = first..s.labels.len();
        } else {
            build_queue_sidebar(&mut s, ui, qx, panel_w, h);
        }
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
        View::Flow
            if ui
                .page
                .as_ref()
                .and_then(|page| page.entity.as_ref())
                .is_some_and(|entry| entry.kind == "track") =>
        {
            156
        }
        View::Flow => 360,
        View::Offline => 190,
        View::Playlists => 200,
        View::Favorites => 120,
        View::Settings => 196,
        _ => 0,
    };
    if !is_lyrics && ui.view != View::TrackInfo {
        let mix_title = if ui.view == View::StyleMix
            && ui.mix_region != usize::MAX
            && ui.mix_genre == usize::MAX
        {
            Some(format!(
                "{} — {}",
                ui.t("Style Mix"),
                ui.t(crate::style_mix::REGIONS[ui.mix_region].0)
            ))
        } else {
            None
        };
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
                .unwrap_or_else(|| mix_title.as_deref().unwrap_or(view_title(ui, ui.view))),
            2,
        );
        let caption = match ui.view {
            View::Home if ui.provider == crate::catalog::Provider::YouTube => {
                "Recommendations from YouTube Music"
            }
            View::Favorites if ui.provider == crate::catalog::Provider::YouTube => {
                "Your liked songs on YouTube Music"
            }
            View::Playlists if ui.provider == crate::catalog::Provider::YouTube => {
                "Your YouTube Music playlists"
            }
            View::Home if ui.provider == crate::catalog::Provider::Spotify => {
                "Personalized recommendations from Spotify"
            }
            View::Home => "Personalized recommendations from Deezer",
            View::Explore => "Discover music, genres and new releases",
            View::StyleMix => "Combine the language and music you want to hear.",
            View::Favorites if ui.provider == crate::catalog::Provider::Spotify => {
                "Your liked songs on Spotify"
            }
            View::Favorites => "Your favorite tracks",
            View::Recent if ui.provider == crate::catalog::Provider::Spotify => {
                "Recently played on Spotify"
            }
            View::Recent => "Listening history from this app",
            View::Offline => "Your music, available offline",
            View::Podcasts => "Your podcasts",
            View::Playlists => "Your playlists",
            View::Albums => "Albums saved to your library",
            View::Settings => "Make DeezNats yours",
            View::CustomColors => "Customize colors for buttons, text and panels",
            _ => "",
        };
        let subtitle_str = if ui.view == View::StyleMix && ui.mix_region != usize::MAX {
            let language = ui.t(crate::style_mix::REGIONS[ui.mix_region].0);
            if ui.mix_genre == usize::MAX {
                ui.t("What would you like to listen to in {language}?")
                    .replace("{language}", language)
            } else {
                format!(
                    "{} — {}",
                    language,
                    ui.t(crate::style_mix::GENRES[ui.mix_genre].0)
                )
            }
        } else if ui.view == View::Flow {
            ui.translated_text(&format!(
                "Get genre based recommendations from {}",
                ui.provider.label()
            ))
        } else if let Some(ent) = entity {
            entity_caption(ent, ui.page.as_deref(), &ent.subtitle, ui)
        } else if !caption.is_empty() {
            ui.t(caption).to_owned()
        } else {
            ui.page
                .as_ref()
                .filter(|p| !p.subtitle.is_empty())
                .map(|p| ui.translated_text(&p.subtitle))
                .unwrap_or_else(|| ui.t(caption).to_owned())
        };
        s.label(
            title_x,
            sub_y,
            (cw - (title_x - x) - header_actions_w).max(120),
            subtitle_str,
            1,
        );
        if ui.view == View::StyleMix && ui.mix_genre != usize::MAX {
            s.label(
                title_x,
                sub_y + 26,
                cw,
                ui.t("From matching {provider} playlists")
                    .replace("{provider}", ui.provider.label()),
                1,
            );
        }
        match ui.view {
            View::Flow
                if ui
                    .page
                    .as_ref()
                    .and_then(|page| page.entity.as_ref())
                    .is_some_and(|entry| entry.kind == "track") =>
            {
                s.button(
                    Rect::new(x + cw - 144, 114, 144, 36),
                    "Play",
                    "\u{e768}",
                    Action::PlayAll,
                    true,
                );
            }
            View::Flow => {
                s.button(
                    Rect::new(x + cw - 344, 114, 190, 36),
                    "Customize Flow",
                    "\u{e713}",
                    Action::Go(View::FlowTune),
                    false,
                );
                s.button(
                    Rect::new(x + cw - 344, 158, 344, 36),
                    "Refresh Flow",
                    "",
                    Action::RefreshFlow,
                    false,
                );
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
    } else if ui.view == View::StyleMix && ui.mix_genre != usize::MAX {
        230
    } else if ui.view == View::TrackInfo {
        82
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
            let collection = matches!(entity.kind.as_str(), "album" | "playlist");
            let play_label = match entity.kind.as_str() {
                "album" => "Play album",
                "playlist" => "Play playlist",
                _ => "Play",
            };
            let play_width = if collection { 160 } else { 100 };
            s.button(
                Rect::new(x, body_top - 54, play_width, 36),
                play_label,
                "\u{e768}",
                Action::PlayAll,
                true,
            );
            let mut ax = x + play_width + 10;
            if collection {
                s.button(
                    Rect::new(ax, body_top - 54, 48, 36),
                    "",
                    "\u{e8b1}",
                    Action::Shuffle,
                    ui.shuffle,
                );
                s.buttons.last_mut().unwrap().accessible = "Shuffle".into();
                ax += 58;
            }
            if matches!(entity.kind.as_str(), "album" | "podcast") {
                let liked = ui.liked_ids.contains(&entity.library_key())
                    || ui.liked_ids.contains(&entity.id);
                s.button(
                    Rect::new(ax, body_top - 54, 48, 36),
                    "",
                    "\u{eb51}",
                    Action::Favorite,
                    liked,
                );
                s.buttons.last_mut().unwrap().accessible = if liked {
                    "In library"
                } else {
                    "Add to library"
                }
                .into();
                ax += 58;
            }
            if supports_downloads(ui) {
                s.button(
                    Rect::new(ax, body_top - 54, 48, 36),
                    "",
                    "\u{e896}",
                    Action::Download,
                    false,
                );
                s.buttons.last_mut().unwrap().accessible = "Download".into();
                ax += 58;
            }
            s.button(
                Rect::new(ax, body_top - 54, 48, 36),
                "",
                "\u{e712}",
                Action::CollectionMenu,
                false,
            );
            s.buttons.last_mut().unwrap().accessible = "More options".into();
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
    } else if ui.view == View::StyleMix && ui.mix_genre != usize::MAX {
        222
    } else if entity.is_some() || ui.view == View::TrackInfo {
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
                    | View::StyleMix
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
                if ui.provider == crate::catalog::Provider::YouTube {
                    &["All", "Playlists", "Genres", "Moods"]
                } else {
                    &["All", "Playlists", "Genres", "Moods", "Podcasts"]
                }
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
        let mix_height = if ui.view == View::StyleMix {
            style_mix_controls(&mut s, ui, x, top, cw)
        } else if ui.view == View::Flow {
            54
        } else {
            0
        };
        if ui.view == View::StyleMix && ui.mix_genre == usize::MAX {
            s.content_height = top + mix_height + ui.scroll;
        } else {
            live_catalog(
                &mut s,
                ui,
                page,
                x,
                top + if ui.view == View::Explore {
                    54
                } else {
                    mix_height
                },
                cw,
                h,
            );
        }
    } else {
        match ui.view {
            View::Settings => settings(&mut s, ui, x, 190, cw, connected, busy),
            View::TrackInfo => build_track_info(&mut s, ui, x, top, cw),
            View::Account if ui.provider == crate::catalog::Provider::YouTube => {
                s.label(
                    x,
                    top + 48,
                    cw,
                    "Sign in with your Google account to load your music library.",
                    3,
                );
                s.button(
                    Rect::new(x, top + 90, 200, 40),
                    "Sign in to YouTube",
                    "",
                    Action::YouTubeLogin,
                    true,
                );
                s.label(x, top + 152, cw, "Or paste the Cookie header from your signed-in music.youtube.com browser session.", 1);
                s.session = Some(Rect::new(x, top + 186, cw.min(560), 34));
                s.button(
                    Rect::new(x, top + 234, 200, 38),
                    "Connect with cookies",
                    "",
                    Action::Connect,
                    true,
                );
                s.content_height = top + 300 + ui.scroll;
            }
            View::Account if ui.provider == crate::catalog::Provider::Spotify => {
                let cy = spotify_catalog_card(&mut s, ui, x, top, cw, connected);
                s.content_height = cy + 40 + ui.scroll;
            }
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
                let mood_cols = (cw / 64)
                    .clamp(1, 9)
                    .min(ui.flow_options.len().max(1) as i32);
                let mood_width = cw / mood_cols;
                let mut yy = 198;
                for (index, (id, name)) in ui.flow_options.iter().enumerate() {
                    let display = if id == "default" || id == "personal" {
                        ui.t("Personal Flow")
                    } else {
                        ui.t(name)
                    };
                    s.button(
                        Rect::new(
                            x + index as i32 % mood_cols * mood_width,
                            yy + index as i32 / mood_cols * 104,
                            mood_width - 6,
                            94,
                        ),
                        display,
                        "",
                        Action::FlowChoice(index),
                        if id == "default" || id == "personal" {
                            ui.flow_personal
                        } else if ui.flow_personal {
                            false
                        } else if ui.provider != crate::catalog::Provider::Deezer {
                            crate::spotify::flow::Selection::parse(&ui.flow_id)
                                .is_ok_and(|selection| selection.mood == *id)
                        } else {
                            id == &ui.flow_id
                        },
                    );
                    s.buttons.last_mut().unwrap().text_centered = true;
                }
                if !ui.flow_options.is_empty() {
                    yy += (ui.flow_options.len() as i32 + mood_cols - 1) / mood_cols * 104;
                }
                s.label(
                    x,
                    yy,
                    cw,
                    "Showing up to 40 genres. Search all provider genres and subgenres.",
                    1,
                );
                yy += 24;
                s.panel(x, yy, cw, 40, 8);
                s.search = Some(Rect::new(x + 14, yy + 7, (cw - 56).max(60), 26));
                s.label_sized(x + cw - 34, yy + 7, 24, 26, "\u{e721}", 4);
                yy += 54;
                let footer = h - 174;
                // Keep selected and edited entries accessible without discarding provider state.
                let mut catalog: Vec<_> = ui.genres.iter().enumerate().collect();
                catalog.sort_by_key(|(_, genre)| {
                    (
                        ui.flow_genre_order
                            .get(&genre.0)
                            .copied()
                            .unwrap_or(usize::MAX),
                        !genre.3,
                        crate::style_mix::popularity_rank(&genre.1),
                    )
                });
                let selected = catalog.iter().filter(|(_, genre)| genre.2).count();
                s.label(
                    x,
                    footer + 8,
                    (cw - 310).max(100),
                    ui.t("{0} genres selected \u{00b7} {1} options")
                        .replace("{0}", &selected.to_string())
                        .replace("{1}", &catalog.len().to_string()),
                    1,
                );
                s.genre_viewport = Some(Rect::new(x, yy, cw, (h - 188 - yy).max(24)));
                let cols = (cw / 145).clamp(1, 5);
                let width = cw / cols;
                let filter = ui.flow_filter.trim().to_lowercase();
                let visible: Vec<_> = catalog
                    .into_iter()
                    .filter(|(_, genre)| {
                        !filter.is_empty()
                            || ui.flow_personal
                            || ui
                                .flow_mood_genres
                                .as_ref()
                                .is_none_or(|ids| ids.contains(&genre.0))
                    })
                    .filter(|(_, genre)| genre.1.to_lowercase().contains(&filter))
                    .collect();
                let limit = if filter.is_empty() {
                    ui.flow_genre_limit.clamp(20, 40)
                } else {
                    usize::MAX
                };
                let displayed: Vec<_> = visible.iter().take(limit).collect();
                let shown = displayed.len();
                let first_row = ((ui.scroll - SCROLL_OVERSCAN).max(0) / 112) as usize;
                let last_row = ((ui.scroll + (h - 188 - yy).max(24) + SCROLL_OVERSCAN).max(0) / 112
                    + 1) as usize;
                for (cell, (index, (_, name, enabled, _))) in displayed
                    .iter()
                    .enumerate()
                    .skip(first_row.saturating_mul(cols as usize))
                    .take(
                        last_row
                            .saturating_sub(first_row)
                            .saturating_mul(cols as usize),
                    )
                {
                    let row = cell as i32 / cols;
                    let column = cell as i32 % cols;
                    s.button(
                        Rect::new(
                            x + column * width,
                            yy + row * 112 - ui.scroll,
                            width - 8,
                            100,
                        ),
                        name,
                        "",
                        Action::Genre(*index),
                        *enabled,
                    );
                    if let Some(button) = s.buttons.last_mut() {
                        button.enabled = ui.flow_customizable;
                        button.text_centered = true;
                        button.artwork = crate::catalog::genre_photos::key(name);
                    }
                }
                let mut genre_bottom = yy + ((shown as i32 + cols - 1) / cols) * 112;
                if filter.is_empty() && visible.len() > 20 {
                    s.button(
                        Rect::new(
                            x + (cw - cw.min(180)) / 2,
                            genre_bottom - ui.scroll,
                            cw.min(180),
                            34,
                        ),
                        if ui.flow_genre_limit >= 40 {
                            "Show less"
                        } else {
                            "Show more"
                        },
                        "",
                        Action::FlowMoreGenres,
                        false,
                    );
                    genre_bottom += 44;
                }
                let custom = ui.flow_filter.trim();
                if ui.provider != crate::catalog::Provider::Deezer
                    && crate::spotify::flow::valid_genre(custom)
                    && !ui.genres.iter().any(|g| g.1.eq_ignore_ascii_case(custom))
                {
                    s.button(
                        Rect::new(x, genre_bottom - ui.scroll, cw.min(360), 34),
                        format!("Use genre: {custom}"),
                        "",
                        Action::FlowAddGenre,
                        false,
                    );
                    genre_bottom += 44;
                }
                let extent = genre_bottom + 64;
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
                let width = cw.clamp(1, 650);
                let left = x + (cw - width) / 2;
                let inset = 24.min((width / 12).max(4));
                let inner = (width - inset * 2).max(1);
                let stacked = inner < 430;
                let cover = if stacked {
                    inner.min(180)
                } else {
                    (inner * 37 / 100).min(220)
                };
                let fields_x = if stacked {
                    left + inset
                } else {
                    left + inset + cover + 20
                };
                let fields_y = if stacked {
                    top + 68 + cover + 54
                } else {
                    top + 68
                };
                let fields_w = if stacked {
                    inner
                } else {
                    (inner - cover - 20).max(1)
                };
                let cover_y = top + 68;
                let controls_y = if stacked {
                    fields_y + 210
                } else {
                    cover_y + cover + 62
                };
                let panel_height = controls_y - top
                    + if ui.playlist_draft.id.is_empty() {
                        74
                    } else {
                        124
                    };
                s.panel(left, top, width, panel_height, 8);
                s.label(
                    left + inset,
                    top + 18,
                    (inner - 45).max(1),
                    "Edit playlist",
                    3,
                );
                s.button(
                    Rect::new(left + width - inset - 36, top + 14, 36, 36),
                    "",
                    "\u{e711}",
                    Action::Back,
                    false,
                );
                if let Some(button) = s.buttons.last_mut() {
                    button.accessible = "Close".into();
                }
                s.panel(left + inset, cover_y, cover, cover, 2);
                if !ui.playlist_draft.preview.is_empty() {
                    s.pictures.push((
                        Rect::new(left + inset, cover_y, cover, cover),
                        ui.playlist_draft.preview.clone(),
                    ));
                }
                if ui.provider != crate::catalog::Provider::YouTube {
                    s.button(
                        Rect::new(left + inset, cover_y + cover + 8, cover, 36),
                        "Choose cover",
                        "\u{e70f}",
                        Action::ChoosePlaylistCover,
                        false,
                    );
                } else {
                    s.label(
                        left + inset,
                        cover_y + cover + 8,
                        cover,
                        "Cover editing unavailable",
                        1,
                    );
                }
                s.label(fields_x, fields_y, fields_w, "Playlist name", 1);
                s.name = Some(Rect::new(fields_x, fields_y + 25, fields_w, 40));
                s.label(fields_x, fields_y + 79, fields_w, "Description", 1);
                s.description = Some(Rect::new(fields_x, fields_y + 104, fields_w, 106));
                let save_width = inner.min(150);
                s.button(
                    Rect::new(
                        left + width - inset - save_width,
                        controls_y,
                        save_width,
                        42,
                    ),
                    "Save",
                    "",
                    Action::SavePlaylist,
                    true,
                );
                if !ui.playlist_draft.id.is_empty() {
                    s.button(
                        Rect::new(left + inset, controls_y + 54, inner.min(170), 38),
                        "Delete playlist",
                        "",
                        Action::DeletePlaylist,
                        false,
                    );
                }
                if ui.playlist_busy {
                    s.label(
                        left + inset,
                        controls_y + 10,
                        (inner - save_width - 12).max(1),
                        "Saving playlist...",
                        1,
                    );
                    for button in &mut s.buttons {
                        if matches!(
                            button.action,
                            Action::SavePlaylist
                                | Action::DeletePlaylist
                                | Action::ChoosePlaylistCover
                                | Action::Back
                        ) {
                            button.enabled = false;
                        }
                    }
                }
                s.content_height = top + panel_height + 24 + ui.scroll;
            }
            View::CoverEdit => {
                s.label(x, top, cw, "Playlist cover", 3);
                let side = cw.min(240);
                s.panel(x, top + 54, side, side, 2);
                if !ui.playlist_draft.preview.is_empty() {
                    s.pictures.push((
                        Rect::new(x, top + 54, side, side),
                        ui.playlist_draft.preview.clone(),
                    ));
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
                        button.enabled = !ui.playlist_busy
                            && (matches!(
                                button.action,
                                Action::ChoosePlaylistCover | Action::Back
                            ) || ui.playlist_draft.image.is_some());
                    }
                    yy += 44;
                }
                s.content_height = yy + 40 + ui.scroll;
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
                    s.button(
                        Rect::new(x, top + 156, 240, 38),
                        "Add to playlist",
                        "+",
                        Action::ChoosePlaylist(ui.now.clone()),
                        false,
                    );
                }
            }
            View::Storage => {
                s.label(x, top, cw, "Data and cache location", 3);
                let directory = crate::storage::data_dir().unwrap_or_default();
                s.label(x, top + 54, cw, directory.display().to_string(), 0);
                s.label(
                    x,
                    top + 100,
                    cw,
                    "Choose where DeezNats stores settings, downloads and cache.",
                    1,
                );
                s.button(
                    Rect::new(x, top + 146, 260.min(cw), 36),
                    "Choose data folder...",
                    "",
                    Action::ChooseStorage,
                    false,
                );
                if let Ok(Some(pending)) = crate::storage::pending_dir() {
                    s.label(x, top + 202, cw, pending.display().to_string(), 0);
                    s.label(x, top + 238, cw, "Data will be copied to this folder the next time you open DeezNats. Original files are kept.", 1);
                    s.button(
                        Rect::new(x, top + 280, 220.min(cw), 36),
                        "Cancel location change",
                        "",
                        Action::CancelStorage,
                        false,
                    );
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
                cy += 44;
                s.button(
                    Rect::new(x, cy, 140.min(cw), 36),
                    "Save",
                    "",
                    Action::Back,
                    false,
                );
                s.content_height = cy + 60 + ui.scroll;
            }
            _ => {
                if connected {
                    render_skeletons(&mut s, ui, None, x, top, cw);
                } else {
                    s.label(
                        x,
                        top,
                        cw,
                        format!("Sign in to load your {} library.", provider_name(ui)),
                        1,
                    );
                    if ui.provider == crate::catalog::Provider::YouTube {
                        s.button(
                            Rect::new(x, top + 40, 200.min(cw), 40),
                            "Sign in to YouTube",
                            "",
                            Action::YouTubeLogin,
                            false,
                        );
                    }
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
    if ui.view == View::Offline {
        let active = ui.downloads.iter().filter(|job| !job.finished).count() as i32;
        if active > 0 {
            let progress_y = (h - player_height(ui, w) - 94 * active - 12).max(s.content_top);
            s.panel(
                x - 8,
                progress_y - 8,
                cw + 16,
                h - player_height(ui, w) - progress_y + 8,
                0,
            );
            download_progress(&mut s, ui, x, progress_y, cw, h - player_height(ui, w));
        }
    }
    if ui.view != View::Lyrics {
        keep_content_controls_visible(&mut s, ui.content_left());
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
        let bottom = if ui.view == View::Lyrics {
            h - 64
        } else {
            h - player_height(ui, w) - 16
        };
        let available = bottom - s.content_top;
        let maximum = (s.content_height - bottom).max(0);
        if maximum > 0 && available > 0 {
            let thumb = (available * available / (s.content_height - s.content_top).max(1))
                .clamp(32, available);
            let scrollbar_x = w
                - queue_w
                - if ui.view == View::Lyrics {
                    12
                } else if queue_w > 0 {
                    39
                } else {
                    27
                };
            s.panel(scrollbar_x, s.content_top, 9, available, 5);
            s.panel(
                scrollbar_x,
                s.content_top
                    + ((ui.scroll as i64 * (available - thumb) as i64) / maximum as i64) as i32,
                9,
                thumb,
                6,
            );
        }
    }
    if ui.view != View::Lyrics {
        build_player(&mut s, ui, w, h, title, playing, volume);
        if ui.playback_unavailable {
            let width = (w - 24).clamp(1, 380);
            let x = (w - width) / 2;
            let y = h - player_height(ui, w) - 52;
            s.panel(x, y, width, 42, 6);
            s.label(
                x + 14,
                y + 12,
                width - 28,
                "This item is unavailable. Skipping to the next item.",
                0,
            );
        }
    }

    if ui.onboarding_open {
        build_onboarding(&mut s, ui, w, h);
    }
    if ui.deezer_login_open {
        build_deezer_login(&mut s, ui, w, h);
    }
    if ui.custom_theme_draft.is_some() {
        build_custom_theme_modal(&mut s, ui, w, h);
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

fn build_onboarding(s: &mut Scene, ui: &Ui, w: i32, h: i32) {
    use crate::catalog::Provider;
    let width = (w - 40).clamp(1, 420);
    let height = (h - 40).clamp(1, 376);
    let rect = Rect::new((w - width) / 2, (h - height) / 2, width, height);
    s.auth_modal = Some((rect, s.labels.len(), s.buttons.len()));
    s.search = None;
    s.session = None;
    s.track = None;
    s.name = None;
    s.description = None;
    s.seek = None;
    s.volume = None;
    s.sliders.clear();
    s.label(
        rect.x + 24,
        rect.y + 24,
        width - 48,
        "Welcome to DeezNats",
        3,
    );
    s.label(
        rect.x + 24,
        rect.y + 65,
        width - 48,
        "Choose a music provider to get started.",
        1,
    );
    for (index, (name, provider)) in [
        ("Deezer", Provider::Deezer),
        ("Spotify", Provider::Spotify),
        ("YouTube Music", Provider::YouTube),
    ]
    .into_iter()
    .enumerate()
    {
        s.button(
            Rect::new(
                rect.x + 24,
                rect.y + 100 + index as i32 * 56,
                width - 48,
                44,
            ),
            name,
            "",
            Action::OnboardingSelect(provider),
            ui.onboarding_provider == Some(provider),
        );
    }
    s.button(
        Rect::new(rect.x + width - 144, rect.y + height - 58, 120, 36),
        "Next",
        "",
        Action::OnboardingNext,
        true,
    );
    if let Some(button) = s.buttons.last_mut() {
        button.enabled = ui.onboarding_provider.is_some();
    }
}

fn build_custom_theme_modal(s: &mut Scene, ui: &Ui, w: i32, h: i32) {
    let Some((light, colors)) = &ui.custom_theme_draft else {
        return;
    };
    let width = (w - 40).clamp(1, 520);
    let height = (h - 40).clamp(1, 570);
    let rect = Rect::new((w - width) / 2, (h - height) / 2, width, height);
    login_modal(
        s,
        rect,
        if *light {
            "Custom light"
        } else {
            "Custom dark"
        },
        Action::CloseCustomTheme,
    );
    let keys = [
        ("Raised", "Button background"),
        ("Text", "Text"),
        ("Icon", "Icon"),
        ("Accent", "Accent / selected state"),
        ("Panel", "Panel"),
        ("Background", "Background"),
        ("Border", "Border"),
        ("Muted", "Secondary text"),
        ("GradientStart", "Gradient start"),
        ("GradientEnd", "Gradient end"),
    ];
    let row = ((height - 120) / keys.len() as i32).clamp(16, 44);
    for (i, (key, label)) in keys.iter().enumerate() {
        let y = rect.y + 62 + i as i32 * row;
        s.label(rect.x + 24, y, (width - 170).max(1), *label, 0);
        let color = colors.get(*key).copied().unwrap_or(0);
        s.button(
            Rect::new(
                rect.x + width - 144,
                y,
                120.min(width - 48).max(1),
                (row - 4).max(12),
            ),
            hex_color(color),
            "",
            Action::Color(key),
            false,
        );
    }
    let y = rect.y + height - 46;
    s.button(
        Rect::new(rect.x + 24, y, 120.min((width - 56) / 2).max(1), 32),
        "Reset",
        "",
        Action::ResetCustomTheme,
        false,
    );
    s.button(
        Rect::new(
            rect.x + width - 144,
            y,
            120.min((width - 56) / 2).max(1),
            32,
        ),
        "Save",
        "",
        Action::SaveCustomTheme,
        true,
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
    // Only strings whose translation would change can need media protection.
    let mut candidates: std::collections::HashSet<&str> = scene
        .labels
        .iter()
        .map(|label| label.text.as_str())
        .chain(
            scene
                .buttons
                .iter()
                .map(|button| button.accessible.as_str()),
        )
        .filter(|text| ui.translated_text(text) != *text)
        .collect();
    let mut media = std::collections::HashSet::new();
    for entry in std::iter::once(&ui.now)
        .chain(ui.track_info.iter().map(|(track, _, _)| track))
        .chain(ui.pinned.iter().map(|(_, entry)| entry))
        .chain(ui.queue.iter())
        .chain(ui.offline.iter())
        .chain(ui.page.iter().flat_map(|page| {
            page.rows
                .iter()
                .chain(
                    page.sections
                        .iter()
                        .flat_map(|section| section.entries.iter()),
                )
                .chain(page.entity.iter())
        }))
    {
        for value in [&entry.title, &entry.subtitle, &entry.album] {
            if candidates.remove(value.as_str()) {
                media.insert(value.as_str());
            }
        }
        if candidates.is_empty() {
            break;
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
            Action::OpenItem(_) | Action::FlowChoice(_) | Action::Genre(_) | Action::Preset(_)
        ) {
            button.text = ui.translated_text(&button.text);
            if !media.contains(button.accessible.as_str()) {
                button.accessible = ui.translated_text(&button.accessible);
            }
        }
    }
}

pub(crate) fn player_height(ui: &Ui, width: i32) -> i32 {
    let breakpoint = if ui.player_layout == 1 { 1100 } else { 1180 };
    if width < breakpoint {
        112
    } else if ui.player_layout == 1 {
        64
    } else {
        88
    }
}
pub(crate) fn player_title_rects(
    region: Rect,
    natural: i32,
    explicit: bool,
    centered: bool,
) -> (Rect, Option<Rect>) {
    let badge_space = if explicit { 22 } else { 0 };
    let title_w = natural.min((region.w - badge_space).max(0)).max(0);
    let total = title_w + badge_space;
    let x = region.x
        + if centered {
            (region.w - total).max(0) / 2
        } else {
            0
        };
    let title = Rect::new(x, region.y, title_w, region.h);
    let badge = explicit.then(|| Rect::new(x + title_w + 6, region.y + 5, 14, 16));
    (title, badge)
}
fn player_metadata(s: &mut Scene, ui: &Ui, region: Rect, title: &str, centered: bool) {
    if title.is_empty() {
        return;
    }
    let explicit = ui.now.raw["explicit"]
        .as_bool()
        .or(ui.now.raw["explicit_lyrics"].as_bool())
        .or_else(|| ui.now.raw["EXPLICIT_LYRICS"].as_str().map(|v| v == "1"))
        .unwrap_or(false);
    let (title_rect, badge_rect) = player_title_rects(
        region,
        title.chars().count() as i32 * 9 + 4,
        explicit,
        centered,
    );
    let label = s.labels.len();
    s.label_sized(
        title_rect.x,
        title_rect.y,
        title_rect.w,
        title_rect.h,
        title,
        if centered { 16 } else { 18 },
    );
    let badge = badge_rect.map(|r| {
        let index = s.labels.len();
        s.label_sized(r.x, r.y, r.w, r.h, "E", 17);
        index
    });
    s.player_title = Some(PlayerTitle {
        region,
        label,
        badge,
        centered,
    });
    let artist_labels = s.labels.len();
    s.artist_links(
        Rect::new(region.x, region.y + 25, region.w, 22),
        &ui.now,
        centered,
    );
    for label in &mut s.labels[artist_labels..] {
        label.kind = if centered { 20 } else { 19 };
    }
}
fn player_controls(s: &mut Scene, ui: &Ui, x: i32, y: i32, step: i32, playing: bool) {
    for (index, (glyph, action, selected, name)) in [
        ("\u{e8b1}", Action::Shuffle, ui.shuffle, "Shuffle"),
        ("\u{e892}", Action::Previous, false, "Previous"),
        (
            if playing { "\u{e769}" } else { "\u{e768}" },
            Action::Toggle,
            false,
            if playing { "Pause" } else { "Play" },
        ),
        ("\u{e893}", Action::Next, false, "Next"),
        (
            if ui.repeat == 2 {
                "\u{e8ed}"
            } else {
                "\u{e8ee}"
            },
            Action::Repeat,
            ui.repeat > 0,
            "Repeat",
        ),
    ]
    .into_iter()
    .enumerate()
    {
        s.button(
            Rect::new(x + index as i32 * step, y, 32, 32),
            "",
            glyph,
            action,
            selected,
        );
        s.buttons.last_mut().unwrap().accessible = name.into();
    }
    if !ui.now.title.is_empty() {
        let liked = ui.liked_ids.contains(&ui.now.library_key());
        s.button(
            Rect::new(x + 5 * step, y, 32, 32),
            "",
            if liked { "\u{e73e}" } else { "\u{e710}" },
            Action::FavoriteNow,
            liked,
        );
        s.buttons.last_mut().unwrap().accessible = if liked {
            "Unlike song"
        } else {
            "Like this song"
        }
        .into();
    }
}
fn player_timeline(s: &mut Scene, x: i32, y: i32, width: i32) {
    let clock = 40;
    s.elapsed = Some(Rect::new(x, y + 2, clock, 20));
    s.seek = Some(Rect::new(x + clock, y, (width - clock * 2).max(20), 24));
    s.remaining = Some(Rect::new(x + width - clock + 6, y + 2, clock - 6, 20));
}
fn player_utility(
    s: &mut Scene,
    rect: Rect,
    glyph: &'static str,
    action: Action,
    selected: bool,
    name: &str,
) {
    s.button(rect, "", glyph, action, selected);
    s.buttons.last_mut().unwrap().accessible = name.into();
}
mod device_sidebar {
    //! Spotify devices and Jam share one native, independently scrollable sidebar.
    use super::*;

    fn shorten(value: &str, max: usize) -> String {
        if value.chars().count() > max {
            format!("{}…", value.chars().take(max - 1).collect::<String>())
        } else {
            value.to_owned()
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        #[test]
        fn sidebars_open_and_close_immediately() {
            for devices in [false, true] {
                let mut ui = Ui {
                    device_menu_open: devices,
                    queue_sidebar_open: !devices,
                    ..Default::default()
                };
                assert_eq!(ui.sidebar_values(), (1., 1.));
                ui.sidebar_collapsed = true;
                ui.device_menu_open = false;
                ui.queue_sidebar_open = false;
                assert_eq!(ui.sidebar_values(), (0., 0.));
                assert_eq!(ui.right_sidebar_width(1200), 0);
                let frame = super::super::build(&ui, (1200, 900), true, false, "", false, 50);
                assert!(frame.device_sidebar.is_none());
                assert!(frame.queue_frame.is_none());
                ui.sidebar_collapsed = false;
                ui.device_menu_open = devices;
                ui.queue_sidebar_open = !devices;
                assert_eq!(ui.sidebar_values(), (1., 1.));
                assert_eq!(ui.right_sidebar_width(1200), 280);
            }
        }
        #[test]
        fn devices_and_queue_share_the_same_docked_frame_and_resize_policy() {
            let mut devices = Ui {
                provider: crate::catalog::Provider::Spotify,
                device_menu_open: true,
                ..Default::default()
            };
            let mut queue = Ui {
                provider: crate::catalog::Provider::Spotify,
                queue_sidebar_open: true,
                ..Default::default()
            };
            for width in [1200, 1080, 900] {
                devices.fit_sidebars(width);
                queue.fit_sidebars(width);
                assert_eq!(devices.sidebar_collapsed, queue.sidebar_collapsed);
                assert_eq!(devices.right_sidebar_open(), queue.right_sidebar_open());
                let device_scene =
                    super::super::build(&devices, (width, 900), true, false, "", false, 50);
                let queue_scene =
                    super::super::build(&queue, (width, 900), true, false, "", false, 50);
                if devices.right_sidebar_open() {
                    let frame = device_scene.device_sidebar.unwrap();
                    assert!(device_scene.auth_modal.is_none());
                    assert!(device_scene.device_popover_anchor.is_none());
                    assert!(queue_scene.panels.iter().any(|p| p.kind == 0
                        && p.rect.x == frame.x
                        && p.rect.y == frame.y
                        && p.rect.w == frame.w
                        && p.rect.h == frame.h));
                    let main = device_scene
                        .panels
                        .iter()
                        .find(|p| p.kind == 0 && p.rect.x < frame.x && p.rect.x > 50)
                        .unwrap();
                    assert!(main.rect.x + main.rect.w < frame.x);
                }
            }
        }
        #[test]
        fn queue_and_devices_replace_each_other_in_the_right_sidebar() {
            let mut ui = Ui {
                provider: crate::catalog::Provider::Spotify,
                queue_sidebar_open: true,
                ..Default::default()
            };
            ui.toggle_device_sidebar();
            assert!(ui.device_menu_open && !ui.queue_sidebar_open);
            ui.toggle_queue_sidebar();
            assert!(ui.queue_sidebar_open && !ui.device_menu_open);
        }
        fn scene(ui: &Ui) -> Scene {
            let mut scene = Scene::default();
            build(&mut scene, ui, 608, 280, 800, true);
            scene
        }
        #[test]
        fn sidebar_has_inline_start_and_role_appropriate_leave_controls() {
            let mut ui = Ui {
                provider: crate::catalog::Provider::Spotify,
                ..Default::default()
            };
            let initial = scene(&ui);
            assert!(initial
                .buttons
                .iter()
                .any(|b| matches!(b.action, Action::SpotifyJamStart)));
            assert!(!initial
                .labels
                .iter()
                .any(|l| l.text.to_lowercase().contains("premium")));
            for owner in [true, false] {
                ui.spotify_jam = Some(crate::spotify::jam::Session {
                    id: "jam".into(),
                    active: true,
                    owner,
                    ..Default::default()
                });
                let result = scene(&ui);
                assert_eq!(
                    result
                        .buttons
                        .iter()
                        .any(|b| matches!(b.action, Action::SpotifyJamLeave(_))),
                    !owner
                );
                assert_eq!(
                    result
                        .buttons
                        .iter()
                        .any(|b| matches!(b.action, Action::SpotifyJamEnd(_))),
                    owner
                );
                assert!(!result.buttons.iter().any(|b| matches!(
                    b.action,
                    Action::OpenSpotify | Action::StartSpotify | Action::SpotifyJamOpen
                )));
            }
        }
        #[test]
        fn scrolling_keeps_current_device_fixed_and_reaches_every_device_and_guest() {
            use std::collections::HashSet;
            let mut ui = Ui {
                provider: crate::catalog::Provider::Spotify,
                ..Default::default()
            };
            ui.spotify_devices
                .push(serde_json::json!({"id":"active","name":"Chosen speaker","is_active":true}));
            ui.spotify_devices.extend((0..20).map(
                |i| serde_json::json!({"id":format!("device{i}"),"name":format!("Speaker {i}")}),
            ));
            ui.spotify_jam = Some(crate::spotify::jam::Session {
                id: "jam".into(),
                active: true,
                owner: true,
                members: (0..40)
                    .map(|i| crate::spotify::jam::Member {
                        id: format!("guest{i}"),
                        name: format!("Guest {i}"),
                        owner: false,
                    })
                    .collect(),
                ..Default::default()
            });
            let first = scene(&ui);
            let maximum = first.device_sidebar_scroll_max;
            let card_y = first.device_active_card.unwrap().y;
            let mut devices = HashSet::new();
            let mut guests = HashSet::new();
            for offset in (0..=maximum).step_by(1).chain(std::iter::once(maximum)) {
                ui.device_sidebar_scroll = offset;
                let result = scene(&ui);
                assert_eq!(result.device_active_card.unwrap().y, card_y);
                assert!(result.labels.iter().any(|l| l.text == "Chosen speaker"));
                let panel = result.device_sidebar.unwrap();
                for button in result.buttons {
                    assert!(
                        button.rect.y >= panel.y
                            && button.rect.y + button.rect.h <= panel.y + panel.h
                    );
                    match button.action {
                        Action::SpotifyTransferPlayback(id) => {
                            assert_ne!(id, "active");
                            devices.insert(id);
                        }
                        Action::SpotifyJamRemove(_, id) => {
                            guests.insert(id);
                        }
                        _ => {}
                    }
                }
            }
            assert_eq!(devices.len(), 20);
            assert_eq!(guests.len(), 40);
        }
    }
    pub(super) fn build(s: &mut Scene, ui: &Ui, qx: i32, width: i32, h: i32, playing: bool) {
        let first_button = s.buttons.len();
        let bottom = h - if ui.view == View::Lyrics { 12 } else { 96 };
        let rect = Rect::new(qx, 10, width, bottom - 20);
        s.panel(rect.x, rect.y, rect.w, rect.h, 0);
        s.device_sidebar = Some(rect);
        let x = rect.x + 16;
        let inner = width - 32;
        if ui.provider != crate::catalog::Provider::Spotify {
            s.label_sized(x, rect.y + 16, inner - 40, 28, "Devices", 0);
            s.button(
                Rect::new(x + inner - 30, rect.y + 14, 30, 30),
                "",
                "\u{e711}",
                Action::CloseDeviceMenu,
                false,
            );
            connect_controls(s, ui, x, rect.y + 65, inner);
            s.device_sidebar_buttons = first_button..s.buttons.len();
            return;
        }
        s.label_sized(x, rect.y + 16, inner - 72, 28, "Spotify Connect", 0);
        s.button(
            Rect::new(x + inner - 66, rect.y + 14, 30, 30),
            "",
            "\u{e72c}",
            Action::SpotifyDevicesRefresh,
            false,
        );
        s.buttons.last_mut().unwrap().enabled = !ui.spotify_devices_busy;
        s.buttons.last_mut().unwrap().accessible = ui.t("Refresh").into();
        s.button(
            Rect::new(x + inner - 30, rect.y + 14, 30, 30),
            "",
            "\u{e711}",
            Action::CloseDeviceMenu,
            false,
        );
        s.buttons.last_mut().unwrap().accessible = ui.t("Close").into();

        let active = ui.spotify_devices.iter().find(|d| d["is_active"] == true);
        let card = Rect::new(x, rect.y + 58, inner, 100);
        s.device_active_card = Some(card);
        s.panel(card.x, card.y, card.w, card.h, 18);
        let heading = if ui.spotify_devices_busy {
            "Updating devices..."
        } else if active.is_none() {
            "No active playback device"
        } else if playing {
            "Playing on"
        } else {
            "Playback paused on"
        };
        s.label(card.x + 12, card.y + 5, card.w - 24, heading, 0);
        if let Some(device) = active {
            let name = device["name"].as_str().unwrap_or("Spotify device");
            s.label(
                card.x + 12,
                card.y + 28,
                card.w - 24,
                shorten(name, ((card.w - 24) / 10).max(8) as usize),
                3,
            );
            s.label(
                card.x + 12,
                card.y + 66,
                card.w - 24,
                if ui.spotify_devices_busy {
                    "Last confirmed device"
                } else {
                    "✓ Active device"
                },
                0,
            );
        } else {
            s.label(
                card.x + 12,
                card.y + 39,
                card.w - 24,
                "Select a device below",
                1,
            );
        }

        let inner = inner - 12;
        let viewport = Rect::new(x, rect.y + 172, inner, (rect.h - 338).max(1));
        s.queue_viewport = Some(viewport);
        let device_count = ui
            .spotify_devices
            .iter()
            .filter(|d| d["is_active"] != true)
            .count() as i32;
        let status_rows = if ui.spotify_devices_status.is_empty() {
            0
        } else {
            wrap_text(ui.t(&ui.spotify_devices_status), inner, 14)
                .len()
                .min(4) as i32
        };
        let jam_status_rows = if ui.spotify_jam_status.is_empty() {
            0
        } else {
            wrap_text(ui.t(&ui.spotify_jam_status), inner, 14)
                .len()
                .min(4) as i32
        };
        let jam_height = if let Some(session) = ui.spotify_jam.as_ref().filter(|s| s.active) {
            36 + if session.invite.is_some() || session.invite_uri.is_some() {
                44
            } else {
                0
            } + 50
                + 34
                + session.members.len() as i32 * 40
        } else {
            48
        };
        let total_height =
            34 + if device_count == 0 {
                34
            } else {
                device_count * 52
            } + status_rows * 28
                + 16
                + 44
                + jam_status_rows * 28
                + jam_height;
        let scroll_offset = ui
            .device_sidebar_scroll
            .clamp(0, ((total_height - viewport.h).max(0) + 23) / 24)
            * 24;
        let mut body = Scene::default();
        let mut y = 0;
        body.label(x, y, inner, "Other devices", 0);
        y += 34;
        let mut count = 0;
        for device in ui.spotify_devices.iter().filter(|d| d["is_active"] != true) {
            if y + 44 < scroll_offset || y > scroll_offset + viewport.h {
                y += 52;
                count += 1;
                continue;
            }
            let id = device["id"].as_str().unwrap_or_default();
            let name = device["name"].as_str().unwrap_or("Spotify device");
            let restricted = device["is_restricted"] == true || id.is_empty();
            let detail = if restricted {
                ui.t("Control unavailable")
            } else {
                device["type"].as_str().unwrap_or("Device")
            };
            body.button(
                Rect::new(x, y, inner, 44),
                format!(
                    "{} · {detail}",
                    shorten(name, ((inner - 100) / 7).max(8) as usize)
                ),
                "\u{e772}",
                Action::SpotifyTransferPlayback(id.into()),
                false,
            );
            let button = body.buttons.last_mut().unwrap();
            button.enabled = !ui.spotify_devices_busy && !restricted;
            button.accessible = format!("{name}, {detail}");
            y += 52;
            count += 1;
        }
        if count == 0 {
            body.label(
                x,
                y,
                inner,
                if ui.spotify_devices_busy {
                    "Loading devices..."
                } else {
                    "No other devices available"
                },
                1,
            );
            y += 34;
        }
        if !ui.spotify_devices_status.is_empty() {
            for line in wrap_text(ui.t(&ui.spotify_devices_status), inner, 14)
                .into_iter()
                .take(4)
            {
                body.label(x, y, inner, line, 1);
                y += 28;
            }
        }
        y += 16;
        body.label(x, y, inner - 106, "Spotify Jam", 3);
        body.button(
            Rect::new(x + inner - 102, y + 2, 102, 30),
            "Refresh Jam",
            "",
            Action::SpotifyJamRefresh,
            false,
        );
        body.buttons.last_mut().unwrap().enabled = !ui.spotify_jam_busy;
        y += 44;
        let jam_first = body.buttons.len();
        if !ui.spotify_jam_status.is_empty() {
            for line in wrap_text(ui.t(&ui.spotify_jam_status), inner, 14)
                .into_iter()
                .take(4)
            {
                body.label(x, y, inner, line, 1);
                y += 28;
            }
        }
        if let Some(session) = ui.spotify_jam.as_ref().filter(|s| s.active) {
            body.label(
                x,
                y,
                inner,
                if session.owner {
                    "You are hosting this Jam"
                } else {
                    "Participating in a Jam"
                },
                0,
            );
            y += 36;
            if session.invite.is_some() || session.invite_uri.is_some() {
                body.button(
                    Rect::new(x, y, inner, 34),
                    "Copy invite link",
                    "",
                    Action::SpotifyJamCopyInvite,
                    false,
                );
                y += 44;
            }
            body.button(
                Rect::new(x, y, inner, 34),
                if session.owner {
                    "End Jam"
                } else {
                    "Leave Jam"
                },
                "",
                if session.owner {
                    Action::SpotifyJamEnd(session.id.clone())
                } else {
                    Action::SpotifyJamLeave(session.id.clone())
                },
                false,
            );
            y += 50;
            body.label(
                x,
                y,
                inner,
                format!("{} ({})", ui.t("Members"), session.members.len()),
                0,
            );
            y += 34;
            for member in &session.members {
                if y + 30 < scroll_offset || y > scroll_offset + viewport.h {
                    y += 40;
                    continue;
                }
                let name = if member.owner {
                    format!("{} · {}", shorten(&member.name, 25), ui.t("Host"))
                } else {
                    shorten(&member.name, 30)
                };
                body.label(x, y, inner - 102, name, 0);
                if session.owner && !member.owner && !member.id.is_empty() {
                    body.button(
                        Rect::new(x + inner - 92, y, 92, 30),
                        "Remove",
                        "",
                        Action::SpotifyJamRemove(session.id.clone(), member.id.clone()),
                        false,
                    );
                }
                y += 40;
            }
        } else {
            body.button(
                Rect::new(x, y, inner, 36),
                if ui.spotify_jam_busy {
                    "Loading..."
                } else {
                    "Start Jam"
                },
                "",
                Action::SpotifyJamStart,
                false,
            );
            y += 48;
        }
        for button in &mut body.buttons[jam_first..] {
            button.enabled = !ui.spotify_jam_busy;
        }
        let maximum = ((y - viewport.h).max(0) + 23) / 24;
        s.device_sidebar_scroll_max = maximum;
        s.queue_scroll_max = maximum as usize;
        let scroll = ui.device_sidebar_scroll.clamp(0, maximum);
        let offset = viewport.y - scroll * 24;
        for mut label in body.labels {
            label.rect.y += offset;
            if label.rect.y >= viewport.y && label.rect.y + label.rect.h <= viewport.y + viewport.h
            {
                s.labels.push(label);
            }
        }
        for mut button in body.buttons {
            button.rect.y += offset;
            if button.rect.y >= viewport.y
                && button.rect.y + button.rect.h <= viewport.y + viewport.h
            {
                s.buttons.push(button);
            }
        }
        let footer = rect.y + rect.h - 154;
        if maximum > 0 {
            let track = Rect::new(rect.x + width - 20, viewport.y, 8, viewport.h);
            let thumb = (track.h * track.h / y.max(1)).max(24).min(track.h);
            s.queue_scrollbar = Some(track);
            s.panel(track.x, track.y, 8, track.h, 16);
            s.panel(
                track.x,
                track.y + (track.h - thumb) * scroll / maximum,
                8,
                thumb,
                17,
            );
        }
        connect_controls(s, ui, x, footer, width - 32);
        s.device_sidebar_buttons = first_button..s.buttons.len();
    }

    fn connect_controls(s: &mut Scene, ui: &Ui, x: i32, y: i32, inner: i32) {
        s.label(x, y, inner - 76, "DeezNats Connect", 0);
        s.button(
            Rect::new(x + inner - 64, y, 60, 28),
            if ui.jam_enabled { "On" } else { "Off" },
            "",
            Action::ToggleBool("jam_enabled"),
            ui.jam_enabled,
        );
        if ui.jam_enabled {
            s.label(x, y + 32, inner, &ui.jam_url, 1);
            s.button(
                Rect::new(x, y + 55, 70, 28),
                "Open",
                "",
                Action::OpenConnect,
                false,
            );
            s.button(
                Rect::new(x + 76, y + 55, 70, 28),
                "Copy",
                "",
                Action::CopyConnectLink,
                false,
            );
        }
        if ui.jam_enabled && !ui.connect_nicknames.is_empty() {
            s.label(
                x,
                y + 90,
                inner,
                &format!("Joined: {}", ui.connect_nicknames.join(", ")),
                1,
            );
        }
    }
}
fn build_device_menu(s: &mut Scene, ui: &Ui, w: i32, h: i32, _playing: bool) {
    let Some(anchor) = s
        .buttons
        .iter()
        .find(|b| matches!(b.action, Action::ToggleDeviceMenu))
        .map(|b| b.rect)
    else {
        return;
    };
    if ui.provider == crate::catalog::Provider::Spotify {
        // Spotify uses the docked right-sidebar layout above.
        return;
    }
    let width = 340.min(w - 24).max(1);
    let available = (anchor.y - 16).max(1).min(h - 24);
    let height = 136.min(available);
    let rect = Rect::new(
        (anchor.x + anchor.w - width).clamp(12, (w - width - 12).max(12)),
        (anchor.y - height - 8).max(12),
        width,
        height,
    );
    s.auth_modal = Some((rect, s.labels.len(), s.buttons.len()));
    s.device_popover_anchor = Some(anchor);
    let x = rect.x + 12;
    let inner = width - 24;
    let y = rect.y + 12;
    s.label(x, y + 6, inner - 76, "DeezNats Connect", 0);
    s.button(
        Rect::new(x + inner - 64, y + 4, 60, 28),
        if ui.jam_enabled { "On" } else { "Off" },
        "",
        Action::ToggleBool("jam_enabled"),
        ui.jam_enabled,
    );
    s.button(
        Rect::new(x, y + 44, inner - 70, 32),
        "Connect settings",
        "\u{e713}",
        Action::Go(View::Settings),
        false,
    );
    s.button(
        Rect::new(x + inner - 64, y + 44, 64, 32),
        "Close",
        "",
        Action::CloseDeviceMenu,
        false,
    );
}

#[cfg(test)]
mod device_menu_scene_tests {
    use super::*;
    #[test]
    fn youtube_flow_selection_survives_switch_and_restart() {
        use crate::catalog::Provider;
        let key = crate::spotify::flow::Selection {
            mood: "focus".into(),
            genres: vec!["Jazz".into(), "Ambient".into()],
        }
        .key();
        let mut ui = Ui {
            provider: Provider::YouTube,
            flow_id: key.clone(),
            ..Default::default()
        };
        ui.switch_flow_provider(Provider::Spotify);
        ui.provider = Provider::Spotify;
        assert_eq!(ui.flow_id, "default");
        ui.flow_id = "party".into();
        ui.switch_flow_provider(Provider::YouTube);
        ui.provider = Provider::YouTube;
        assert_eq!(ui.flow_id, key);
        let mut restored = Ui {
            provider: Provider::YouTube,
            ..Default::default()
        };
        let mut preferences = serde_json::json!({});
        ui.write_flow_preferences(&mut preferences);
        restored.restore_flow_preferences(&preferences);
        assert_eq!(restored.flow_id, key);
        assert_eq!(restored.spotify_flow_id, "party");
    }
    #[test]
    fn turkish_sidebar_labels_preserve_unicode_and_translate_jam_actions() {
        let mut ui = Ui {
            provider: crate::catalog::Provider::Spotify,
            device_menu_open: true,
            spotify_devices: vec![
                serde_json::json!({"id":"active","name":"DeezNats","is_active":true}),
            ],
            ..Default::default()
        };
        ui.set_language("tr");
        let scene = build(&ui, (1180, 900), true, false, "Song", true, 50);
        assert!(scene.labels.iter().any(|l| l.text == "Şu cihazda çalıyor"));
        assert!(scene.labels.iter().any(|l| l.text == "✓ Etkin cihaz"));
        assert!(scene.buttons.iter().any(|b| b.text == "Jam başlat"));
    }
    #[test]
    fn provider_menus_fit_and_keep_device_actions_above_connect() {
        for provider in [
            crate::catalog::Provider::Deezer,
            crate::catalog::Provider::Spotify,
            crate::catalog::Provider::YouTube,
        ] {
            for (width, height) in [(640, 640), (900, 720), (1180, 900)] {
                for layout in [0, 1] {
                    let ui = Ui {
                        provider,
                        device_menu_open: true,
                        player_layout: layout,
                        spotify_devices: vec![
                            serde_json::json!({"id":"active","name":"This device","is_active":true}),
                            serde_json::json!({"id":"speaker","name":"Living room","type":"Speaker"}),
                            serde_json::json!({"id":"restricted","name":"Restricted","is_restricted":true}),
                        ],
                        ..Default::default()
                    };
                    let scene = build(&ui, (width, height), true, false, "Song", false, 50);
                    assert!(scene.auth_modal.is_none());
                    let rect = scene.device_sidebar.expect("docked devices");
                    let first_label = scene.queue_labels.start;
                    assert!(
                        rect.x >= 0
                            && rect.y >= 0
                            && rect.x + rect.w <= width
                            && rect.y + rect.h <= height
                    );
                    assert!(scene.device_popover_anchor.is_none());
                    assert!(scene.labels[first_label..]
                        .iter()
                        .any(|label| label.text == "DeezNats Connect"));
                    let buttons = &scene.buttons[scene.device_sidebar_buttons.clone()];
                    let connect = buttons
                        .iter()
                        .find(|b| matches!(b.action, Action::ToggleBool("jam_enabled")))
                        .unwrap();
                    for button in buttons {
                        assert!(
                            button.rect.x >= rect.x
                                && button.rect.y >= rect.y
                                && button.rect.x + button.rect.w <= rect.x + rect.w
                                && button.rect.y + button.rect.h <= rect.y + rect.h
                        );
                        if let Action::SpotifyTransferPlayback(id) = &button.action {
                            assert_eq!(provider, crate::catalog::Provider::Spotify);
                            assert!(button.rect.y + button.rect.h <= connect.rect.y);
                            assert_eq!(button.enabled, id == "speaker");
                        }
                    }
                    assert!(!scene
                        .buttons
                        .iter()
                        .any(|b| matches!(b.action, Action::PlayerLayout(_))));
                }
            }
        }
    }
    #[test]
    fn closed_and_loading_menus_have_truthful_actions() {
        let mut ui = Ui {
            provider: crate::catalog::Provider::Spotify,
            ..Default::default()
        };
        let closed = build(&ui, (900, 720), true, false, "", false, 50);
        assert!(closed.device_popover_anchor.is_none());
        ui.device_menu_open = true;
        ui.spotify_devices_busy = true;
        let loading = build(&ui, (900, 720), true, false, "", false, 50);
        assert!(
            !loading
                .buttons
                .iter()
                .find(|b| matches!(b.action, Action::SpotifyDevicesRefresh))
                .unwrap()
                .enabled
        );
        assert!(!loading
            .buttons
            .iter()
            .any(|b| matches!(b.action, Action::SpotifyTransferPlayback(_))));
    }
}

fn player_extras(s: &mut Scene, ui: &Ui, right: i32, y: i32, volume: u32, narrow: bool) {
    let volume_w = if narrow { 94 } else { 140 };
    let fullscreen = right - 42;
    let volume_x = fullscreen - volume_w - if narrow { 70 } else { 24 };
    let mute_x = volume_x - 38;
    if !narrow {
        let layout_x = mute_x - 150;
        player_utility(
            s,
            Rect::new(layout_x, y, 34, 32),
            "\u{e772}",
            Action::ToggleDeviceMenu,
            ui.device_menu_open,
            "Devices",
        );
        player_utility(
            s,
            Rect::new(layout_x + 50, y, 32, 32),
            "\u{ec71}",
            Action::Go(View::Lyrics),
            false,
            "Lyrics",
        );
        player_utility(
            s,
            Rect::new(layout_x + 100, y, 32, 32),
            "\u{e8fd}",
            Action::ToggleQueueSidebar,
            ui.queue_sidebar_open,
            "Queue",
        );
    } else {
        player_utility(
            s,
            Rect::new(fullscreen, y, 34, 32),
            "\u{e772}",
            Action::ToggleDeviceMenu,
            ui.device_menu_open,
            "Devices",
        );
    }
    player_utility(
        s,
        Rect::new(mute_x, y, 32, 32),
        if volume == 0 { "\u{e74f}" } else { "\u{e767}" },
        Action::MuteVolume,
        volume == 0,
        if volume == 0 { "Unmute" } else { "Mute" },
    );
    s.volume = Some(Rect::new(
        volume_x,
        y + 4,
        volume_w + if narrow { 60 } else { 56 },
        24,
    ));
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
    let cover = if compact { 42 } else { 52 };
    let cover_x = if compact { 20 } else { 16 };
    s.pictures.push((
        Rect::new(cover_x, top + (height - cover) / 2, cover, cover),
        ui.now.artwork.clone(),
    ));
    player_extras(s, ui, right, top + (height - 32) / 2, volume, false);
    let extras_left = right - 42 - 140 - 24 - 38 - 150;
    if compact {
        let controls_x = (w * 213 / 1000).clamp(220, 320);
        let step = (w / 30).clamp(36, 49);
        player_metadata(
            s,
            ui,
            Rect::new(78, top + 7, controls_x - 96, 24),
            title,
            false,
        );
        player_controls(s, ui, controls_x, top + 16, step, playing);
        let timeline_x = controls_x + step * 5 + 64;
        player_timeline(s, timeline_x, top + 20, extras_left - timeline_x - 20);
    } else {
        player_controls(s, ui, 84, top + 28, 49, playing);
        let center = w / 2;
        let timeline_w = 440
            .min((center - 340).max(60) * 2)
            .min((extras_left - center - 20).max(60) * 2);
        let metadata_w = ((center - 377).min(extras_left - center - 16) * 2).max(80);
        player_metadata(
            s,
            ui,
            Rect::new(center - metadata_w / 2, top + 6, metadata_w, 24),
            title,
            true,
        );
        player_timeline(s, center - timeline_w / 2, top + 53, timeline_w);
    }
}
fn build_narrow_player(
    s: &mut Scene,
    ui: &Ui,
    _left: i32,
    top: i32,
    width: i32,
    title: &str,
    playing: bool,
    volume: u32,
) {
    let controls_x = width - 289;
    s.pictures
        .push((Rect::new(22, top + 12, 52, 52), ui.now.artwork.clone()));
    player_metadata(
        s,
        ui,
        Rect::new(88, top + 9, (controls_x - 104).max(60), 24),
        title,
        false,
    );
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
    let is_grid = matches!(
        ui.view,
        View::Albums | View::Playlists | View::PlaylistPicker
    );
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
                if ui.view != View::Offline && supports_downloads(ui) {
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
fn mix_page(total: usize, page: usize, cols: i32) -> std::ops::Range<usize> {
    let size = cols.max(1) as usize * 2;
    0..page.saturating_add(1).saturating_mul(size).min(total)
}

fn mix_pager(
    s: &mut Scene,
    x: i32,
    y: i32,
    cw: i32,
    total: usize,
    page: usize,
    cols: i32,
    languages: bool,
) -> i32 {
    let pages = total.div_ceil(cols.max(1) as usize * 2);
    if pages <= 1 {
        return 0;
    }
    if page.saturating_add(1) >= pages {
        return 0;
    }
    s.button(
        Rect::new(x + (cw - 150.min(cw)) / 2, y, 150.min(cw), 34),
        "See more",
        "",
        Action::MixMore(languages),
        false,
    );
    46
}

fn style_mix_controls(s: &mut Scene, ui: &Ui, x: i32, top: i32, cw: i32) -> i32 {
    if ui.mix_genre != usize::MAX {
        let width = ((cw - 12) / 2).min(170);
        let right = x + cw;
        s.button(
            Rect::new(right - width * 2 - 12, top, width, 38),
            "Refresh mix",
            "\u{e721}",
            Action::BuildStyleMix,
            true,
        );
        s.button(
            Rect::new(right - width, top, width, 38),
            "Play mix",
            "\u{e768}",
            Action::PlayAll,
            false,
        );
        s.buttons.last_mut().unwrap().enabled = ui
            .page
            .as_ref()
            .is_some_and(|p| !p.loading && !p.rows.is_empty());
        return 54;
    }
    let cols = (cw / 145).clamp(1, 5);
    let width = cw / cols;
    let mut y = top;
    let filter = ui.flow_filter.trim().to_lowercase();
    if ui.mix_region == usize::MAX {
        s.label(x, y, cw, "Country / language", 0);
        y += 30;
        s.panel(x, y, cw, 40, 8);
        s.search = Some(Rect::new(x + 14, y + 7, (cw - 56).max(60), 26));
        y += 54;
        let regions: Vec<_> = crate::style_mix::REGIONS
            .iter()
            .enumerate()
            .filter(|(_, (name, _))| {
                ui.t(name).to_lowercase().contains(&filter) || name.to_lowercase().contains(&filter)
            })
            .collect();
        let range = mix_page(regions.len(), ui.mix_language_page, cols);
        for (cell, (index, (name, _))) in regions[range.clone()].iter().enumerate() {
            s.button(
                Rect::new(
                    x + cell as i32 % cols * width,
                    y + cell as i32 / cols * 112,
                    width - 8,
                    100,
                ),
                ui.t(name),
                "",
                Action::MixRegion(*index),
                false,
            );
            s.buttons.last_mut().unwrap().artwork =
                crate::style_mix::artwork::key(true, *index).unwrap_or_default();
        }
        y += (range.len() as i32 + cols - 1) / cols * 112 + 12;
        y += mix_pager(s, x, y, cw, regions.len(), ui.mix_language_page, cols, true);
        return y + 12 - top;
    }
    s.panel(x, y, cw, 40, 8);
    s.search = Some(Rect::new(x + 14, y + 7, (cw - 56).max(60), 26));
    y += 54;
    let Some(data) = ui.mix_genres.as_ref() else {
        s.label(x, y + 8, cw, "Checking available genres...", 1);
        s.panel(x, y + 40, cw.min(240), 8, 14);
        return y + 76 - top;
    };
    if let Some(error) = data["error"].as_str() {
        s.label(x, y, cw, error, 1);
        s.button(
            Rect::new(x, y + 36, 150, 34),
            "Try again",
            "",
            Action::MixRegion(ui.mix_region),
            false,
        );
        return y + 86 - top;
    }
    let genres = data["genres"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default();
    if genres.is_empty() {
        s.label(
            x,
            y,
            cw,
            "No matching genres found. Try another language.",
            1,
        );
        return y + 42 - top;
    }
    let mut filtered: Vec<_> = genres
        .iter()
        .filter(|genre| {
            let title = genre["title"].as_str().unwrap_or("");
            ui.t(title).to_lowercase().contains(&filter) || title.to_lowercase().contains(&filter)
        })
        .collect();
    filtered.sort_by_key(|genre| {
        crate::style_mix::popularity_rank(genre["title"].as_str().unwrap_or(""))
    });
    let total = filtered.len();
    let range = mix_page(total, ui.mix_genre_page, cols);
    for (cell, genre) in filtered[range.clone()].iter().enumerate() {
        let Some(index) = genre["id"]
            .as_u64()
            .and_then(|id| usize::try_from(id).ok())
            .filter(|&id| id < crate::style_mix::GENRES.len())
        else {
            continue;
        };
        s.button(
            Rect::new(
                x + cell as i32 % cols * width,
                y + cell as i32 / cols * 112,
                width - 8,
                100,
            ),
            ui.t(genre["title"].as_str().unwrap_or("")),
            "",
            Action::MixGenre(index),
            ui.mix_genre == index,
        );
        s.buttons.last_mut().unwrap().artwork =
            crate::style_mix::artwork::key(false, index).unwrap_or_default();
    }
    y += (range.len() as i32 + cols - 1) / cols * 112 + 12;
    y += mix_pager(s, x, y, cw, total, ui.mix_genre_page, cols, false);
    if data["partial"] == true {
        s.label(
            x,
            y,
            cw,
            "Some genres could not be checked. Select the language to retry.",
            1,
        );
        y += 32;
    }
    y + 12 - top
}

fn download_progress(s: &mut Scene, ui: &Ui, x: i32, mut y: i32, width: i32, bottom: i32) -> i32 {
    for job in ui.downloads.iter().rev().filter(|job| !job.finished) {
        if y >= bottom {
            break;
        }
        s.label(x, y, (width - 126).max(40), &job.title, 3);
        s.label(
            x,
            y + 32,
            width,
            format!("{} / {} · {}", job.done, job.total, job.message),
            1,
        );
        let bar_width = width.max(1);
        s.panel(x, y + 66, bar_width, 4, 8);
        let completed =
            (bar_width as u64 * job.done.min(job.total) as u64 / job.total.max(1) as u64) as i32;
        if completed > 0 {
            s.panel(x, y + 66, completed, 4, 3);
        }
        if !job.finished {
            let segment = (bar_width / 12).max(1);
            let remaining = (bar_width - completed - segment).max(0);
            let offset = remaining * ui.download_frame as i32 / 7;
            s.panel(
                x + completed + offset,
                y + 66,
                segment.min(bar_width - completed),
                4,
                3,
            );
            s.button(
                Rect::new(x + width - 112, y, 112, 32),
                "Cancel",
                "",
                Action::CancelDownload(job.id),
                false,
            );
        }
        y += 94;
    }
    y
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
    let start = catalog_scroll::Ranges::at(s);
    let pinned: Vec<_> = ui
        .pinned
        .iter()
        .filter(|(provider, _)| *provider == ui.provider)
        .map(|(_, entry)| entry.clone())
        .collect();
    if ui.view == View::Home && !pinned.is_empty() {
        let mut page = page.clone();
        page.sections.insert(
            0,
            crate::catalog::Section {
                title: "Pinned".into(),
                entries: pinned,
                ..Default::default()
            },
        );
        live_catalog_contents(s, ui, &page, x, top, cw, h);
    } else {
        live_catalog_contents(s, ui, page, x, top, cw, h);
    }
    s.catalog_layout = Some(catalog_scroll::Layout {
        x,
        origin: top + ui.scroll,
        width: cw,
        ranges: start.finish(s),
    });
}

fn live_catalog_contents(
    s: &mut Scene,
    ui: &Ui,
    page: &crate::catalog::Page,
    x: i32,
    top: i32,
    cw: i32,
    h: i32,
) {
    let mut y = top;
    if ui.view == View::Artist
        && ui.provider == crate::catalog::Provider::YouTube
        && !page.subtitle.is_empty()
    {
        let lines = wrap_text(&plain_description(&page.subtitle), cw, 14);
        let artist_id = page
            .entity
            .as_ref()
            .map(|entry| entry.id.as_str())
            .unwrap_or("");
        let expanded = ui.expanded_podcast_description.as_deref() == Some(artist_id);
        for line in lines.iter().take(if expanded { lines.len() } else { 3 }) {
            s.label(x, y, cw, line, 1);
            y += 22;
        }
        if lines.len() > 3 && !artist_id.is_empty() {
            let label = if expanded { "Read less" } else { "Read more" };
            s.button(
                Rect::new(x, y, cw.min(180), 24),
                label,
                "",
                Action::TogglePodcastDescription(artist_id.to_owned()),
                false,
            );
            if let Some(button) = s.buttons.last_mut() {
                button.text_link = true;
            }
            y += 32;
        }
        y += 18;
    }
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
            if y + 22 >= s.content_top - SCROLL_OVERSCAN && y < h - 124 + SCROLL_OVERSCAN {
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
        if ui.view == View::Favorites {
            s.button(
                Rect::new(x + 144, y, 48, 36),
                "",
                "\u{e8b1}",
                Action::Shuffle,
                ui.shuffle,
            );
            s.buttons.last_mut().unwrap().accessible = "Shuffle".into();
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

        let rows = sorted_page_rows(ui, page);
        let row_origin = y;
        let ahead = (h - 124 - s.content_top).max(240).max(SCROLL_OVERSCAN);
        let first_row = ((s.content_top - ahead - row_origin - 56).max(0) / 56) as usize;
        let last_row = ((h - 124 + ahead - row_origin).max(0) / 56 + 1) as usize;
        for (index, item) in rows
            .iter()
            .copied()
            .enumerate()
            .skip(first_row)
            .take(last_row.saturating_sub(first_row))
        {
            let y = row_origin + index as i32 * 56;
            let ahead = (h - 124 - s.content_top).max(240);
            if y + 56 >= s.content_top - ahead && y < h - 124 + ahead {
                s.artwork_ahead.push((y, item.artwork.clone()));
            }
            if y + 56 >= s.content_top - SCROLL_OVERSCAN && y < h - 124 + SCROLL_OVERSCAN {
                if catalog_scroll::reuse(s, format!("track:{index}"), y) {
                    continue;
                }
                let row_start = catalog_scroll::Ranges::at(s);
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
                catalog_scroll::record(s, format!("track:{index}"), y, row_start);
            }
        }
        y = row_origin + rows.len() as i32 * 56;
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
    let staged = page.is_catalogue();
    for (section_index, section) in page.sections.iter().enumerate() {
        if ui.view == View::Playlists && section.title == "Saved podcasts" {
            continue;
        }
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
            && !staged
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
                if ui.provider == crate::catalog::Provider::YouTube && discover && ui.filter == 1 {
                    return e.kind == "playlist";
                }
                ui.view != View::Explore
                    || discover
                    || ui.filter == 0
                    || e.kind
                        == ["", "track", "artist", "playlist", "album", "podcast"][ui.filter.min(5)]
            })
            .collect();
        let staged_cols = if is_tiles { (cw / 220).max(2) } else { cols } as usize;
        let staged_limit = ui.shelf_rows.get(&section.title).copied().unwrap_or(2) * staged_cols;
        let staged_more = staged
            && (items.len() > staged_limit || crate::catalog::shelf_has_more(page, &section.title));
        if staged {
            items.truncate(staged_limit);
        }
        let shelf_has_more = shelf && items.len() > cols as usize;
        if shelf_has_more && !discover {
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
            s.label(x + 14, y, cw - 14, &section.error, 1);
            y += 60;
            continue;
        }
        if is_tiles {
            let tile_cols = (cw / 220).max(2);
            let tile_w = (cw - 12) / tile_cols;
            let tile_limit = tile_cols as usize * 2;
            let tiles_have_more = !staged && discover && ui.filter == 0 && items.len() > tile_limit;
            if tiles_have_more {
                items.truncate(tile_limit);
            }
            let first_row = ((s.content_top - SCROLL_OVERSCAN - y - 94).max(0) / 102) as usize;
            let last_row = ((h - 124 + SCROLL_OVERSCAN - y).max(0) / 102 + 1) as usize;
            for (row, chunk) in items
                .chunks(tile_cols as usize)
                .enumerate()
                .skip(first_row)
                .take(last_row.saturating_sub(first_row))
            {
                let yy = y + row as i32 * 102;
                if yy + 94 >= s.content_top - SCROLL_OVERSCAN && yy <= h - 124 + SCROLL_OVERSCAN {
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
            if staged_more {
                shelf_see_more(
                    s,
                    ui,
                    x,
                    &mut y,
                    cw,
                    &section.title,
                    staged_cols,
                    page.loading,
                );
            } else if tiles_have_more
                || (!staged
                    && discover
                    && ui.filter == 0
                    && ui.provider == crate::catalog::Provider::YouTube
                    && section.title == "Categories")
            {
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
        let row_height = cover + 88;
        let row_count = items.len().div_ceil(cols as usize);
        let ahead = (h - 124 - s.content_top).max(240).max(SCROLL_OVERSCAN);
        let first_row = ((s.content_top - ahead - y - row_height).max(0) / row_height) as usize;
        let last_row = ((h - 124 + ahead - y).max(0) / row_height + 1) as usize;
        let row_origin = y;
        for (row, chunk) in items
            .chunks(cols as usize)
            .enumerate()
            .skip(first_row)
            .take(last_row.saturating_sub(first_row))
        {
            // Original DrawCard: one title line + one subtitle line, regardless of text length.
            let yy = row_origin + row as i32 * row_height;
            let ahead = (h - 124 - s.content_top).max(240);
            if yy + row_height >= s.content_top - ahead && yy < h - 124 + ahead {
                s.artwork_ahead
                    .extend(chunk.iter().map(|item| (yy, item.artwork.clone())));
            }
            if yy + row_height >= s.content_top - SCROLL_OVERSCAN && yy < h - 124 + SCROLL_OVERSCAN
            {
                let row_key = format!("section:{section_index}:row:{row}");
                if catalog_scroll::reuse(s, row_key.clone(), yy) {
                    continue;
                }
                let row_start = catalog_scroll::Ranges::at(s);
                let metadata: Vec<_> = chunk
                    .iter()
                    .map(|item| {
                        let title = if matches!(item.id.as_str(), "__songs" | "__episodes")
                            && item.kind == "offline_collection"
                        {
                            ui.t(&item.title).to_owned()
                        } else {
                            plain_description(&item.title)
                        };
                        let subtitle = if item.kind == "podcast" {
                            item.episode_count()
                                .map(|count| {
                                    format!(
                                        "{count} {}",
                                        ui.t(if count == 1 { "episode" } else { "episodes" })
                                    )
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
                        button.accessible = format!("{} {}", ui.t("Open"), title);
                    }
                    let label_x = xx + cover / 2 - label_w / 2;
                    s.link(
                        Rect::new(label_x, yy + cover + 12, label_w, 23),
                        title,
                        (*item).clone(),
                        true,
                    );
                    if let Some(button) = s.buttons.last_mut() {
                        button.accessible = title.clone();
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
                catalog_scroll::record(s, row_key, yy, row_start);
            }
        }
        y = row_origin + row_count as i32 * row_height;
        if staged_more {
            shelf_see_more(
                s,
                ui,
                x,
                &mut y,
                cw,
                &section.title,
                staged_cols,
                page.loading,
            );
        } else if discover && shelf_has_more {
            discover_show_all(s, ui, x, &mut y, cw, &section.title);
        }
        y += 20;
    }
    if !page.next.is_empty() && (!page.loading || ui.view == View::Flow) {
        s.button(
            Rect::new(
                if ui.view == View::Flow {
                    x + (cw - cw.min(140)) / 2
                } else {
                    x
                },
                y,
                cw.min(140),
                36,
            ),
            if page.loading {
                "Loading..."
            } else if ui.view == View::Flow {
                "See more"
            } else {
                "Load more"
            },
            "",
            Action::More,
            false,
        );
        s.buttons.last_mut().unwrap().enabled = !page.loading;
        y += 58;
    }
    if !page.loading
        && page.error.is_empty()
        && page.rows.is_empty()
        && page.sections.iter().all(|s| s.entries.is_empty())
    {
        s.label(
            x + 14,
            y + 40,
            cw - 14,
            if ui.view == View::Podcasts && !supports_downloads(ui) {
                ui.t("Your saved podcasts and episodes appear here.").to_owned()
            } else if ui.view == View::Podcasts {
                ui.t("Your saved podcasts, favorites and downloaded episodes appear here.").to_owned()
            } else if ui.view == View::StyleMix {
                ui.t("No playable tracks found in matching playlists. Try another language or genre.").to_owned()
            } else if ui.view == View::Offline {
                "Downloaded tracks will appear here.".to_owned()
            } else {
                format!("No items were returned by {}.", ui.provider.label())
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
fn shelf_see_more(
    s: &mut Scene,
    ui: &Ui,
    x: i32,
    y: &mut i32,
    width: i32,
    title: &str,
    cols: usize,
    loading: bool,
) {
    let label = ui.t("See more");
    let button_width = text_button_width(label);
    s.button(
        Rect::new(x + (width - button_width) / 2, *y, button_width, 36),
        label,
        "",
        Action::ShelfMore(title.into(), cols),
        false,
    );
    if let Some(button) = s.buttons.last_mut() {
        button.enabled = !loading;
    }
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
    fn flow_mood_shows_provider_genres_but_searches_the_full_catalog() {
        let mut ui = Ui {
            view: View::FlowTune,
            flow_personal: false,
            flow_mood_genres: Some(["jazz".to_owned()].into_iter().collect()),
            genres: vec![
                ("jazz".into(), "Jazz".into(), true, true),
                ("rock".into(), "Rock".into(), false, false),
            ],
            ..Default::default()
        };
        let listed = |ui: &Ui| {
            build(ui, (1043, 1200), true, false, "", false, 40)
                .buttons
                .into_iter()
                .filter(|b| matches!(b.action, Action::Genre(_)))
                .map(|b| b.text)
                .collect::<Vec<_>>()
        };
        assert_eq!(listed(&ui), vec!["Jazz"]);
        ui.flow_filter = "rock".into();
        assert_eq!(listed(&ui), vec!["Rock"]);
        ui.flow_filter.clear();
        ui.flow_personal = true;
        assert_eq!(listed(&ui).len(), 2);
    }

    #[test]
    fn flow_edits_keep_mood_and_positions_while_copying_personal_settings() {
        let mut ui = Ui {
            provider: crate::catalog::Provider::Spotify,
            view: View::FlowTune,
            flow_personal: false,
            flow_id: "focus".into(),
            flow_loaded: true,
            flow_customizable: true,
            genres: vec![
                ("a".into(), "Alpha".into(), false, false),
                ("z".into(), "Zulu".into(), false, false),
            ],
            ..Default::default()
        };
        ui.freeze_flow_order();
        let order = |ui: &Ui| {
            build(ui, (1043, 1200), true, false, "", false, 40)
                .buttons
                .into_iter()
                .filter(|b| matches!(b.action, Action::Genre(_)))
                .map(|b| {
                    assert!(b.glyph.is_empty());
                    b.text
                })
                .collect::<Vec<_>>()
        };
        let before = order(&ui);
        ui.genres[1].2 = true;
        ui.capture_personal_flow_draft();
        assert_eq!(order(&ui), before);
        assert!(!ui.flow_personal);
        assert_eq!(ui.flow_id, "focus");
        let selection = crate::spotify::flow::Selection::parse(&ui.personal_flow_id()).unwrap();
        assert_eq!(selection.mood, "default");
        assert_eq!(selection.genres, vec!["z"]);
        let mut preferences = serde_json::json!({});
        ui.write_flow_preferences(&mut preferences);
        let mut restored = Ui {
            provider: ui.provider,
            ..Default::default()
        };
        restored.restore_flow_preferences(&preferences);
        assert_eq!(restored.personal_flow_id(), ui.personal_flow_id());
        ui.freeze_flow_order();
        assert_eq!(order(&ui).first().unwrap(), "Zulu");
    }

    #[test]
    fn personal_flow_keeps_last_applied_selection_per_provider() {
        let mut ui = Ui::default();
        for (provider, mood) in [
            (crate::catalog::Provider::Deezer, "happy"),
            (crate::catalog::Provider::Spotify, "focus"),
            (crate::catalog::Provider::YouTube, "chill"),
        ] {
            ui.provider = provider;
            ui.flow_id = mood.into();
            ui.remember_applied_flow();
            ui.flow_id = "party".into();
            assert_eq!(ui.personal_flow_id(), mood);
        }
        let mut preferences = serde_json::json!({});
        ui.write_flow_preferences(&mut preferences);
        let mut restored = Ui::default();
        restored.restore_flow_preferences(&preferences);
        for (provider, mood) in [
            (crate::catalog::Provider::Deezer, "happy"),
            (crate::catalog::Provider::Spotify, "focus"),
            (crate::catalog::Provider::YouTube, "chill"),
        ] {
            restored.provider = provider;
            assert_eq!(restored.personal_flow_id(), mood);
        }
    }

    #[test]
    fn flow_shows_twenty_then_forty_and_searches_the_complete_catalog() {
        for provider in [
            crate::catalog::Provider::Deezer,
            crate::catalog::Provider::Spotify,
            crate::catalog::Provider::YouTube,
        ] {
            let mut ui = Ui {
                provider,
                view: View::FlowTune,
                flow_loaded: true,
                flow_customizable: true,
                genres: (0..200)
                    .map(|i| (i.to_string(), format!("Genre {i:03}"), i > 150, i > 150))
                    .collect(),
                ..Default::default()
            };
            for limit in [20, 40] {
                ui.flow_genre_limit = limit;
                let scene = build(&ui, (1043, 2400), true, false, "", false, 40);
                assert_eq!(
                    scene
                        .buttons
                        .iter()
                        .filter(|b| matches!(b.action, Action::Genre(_)))
                        .count(),
                    limit
                );
                let viewport = scene.genre_viewport.unwrap();
                let more = scene
                    .buttons
                    .iter()
                    .find(|b| matches!(b.action, Action::FlowMoreGenres))
                    .unwrap();
                assert!((more.rect.x * 2 + more.rect.w - (viewport.x * 2 + viewport.w)).abs() <= 1);
                assert!(scene
                    .buttons
                    .iter()
                    .filter(|b| matches!(b.action, Action::Genre(_)))
                    .all(
                        |b| b.artwork.starts_with(crate::catalog::genre_photos::PREFIX)
                            && b.rect.h == 100
                            && b.text_centered
                    ));
            }
            ui.flow_filter = "Genre 149".into();
            let scene = build(&ui, (1043, 2400), true, false, "", false, 40);
            assert!(scene
                .buttons
                .iter()
                .any(|b| matches!(b.action, Action::Genre(149))));
            assert_eq!(ui.genres.len(), 200);
            assert_eq!(ui.genres.iter().filter(|g| g.2).count(), 49);
        }
    }

    #[test]
    fn flow_catalog_is_limited_and_counted_for_every_provider() {
        for provider in [
            crate::catalog::Provider::Deezer,
            crate::catalog::Provider::Spotify,
            crate::catalog::Provider::YouTube,
        ] {
            let mut ui = Ui {
                provider,
                view: View::FlowTune,
                flow_loaded: true,
                flow_customizable: true,
                genres: (0..120)
                    .map(|i| {
                        (
                            i.to_string(),
                            format!("TestGenre{i:03}"),
                            i == 119,
                            i == 119,
                        )
                    })
                    .collect(),
                flow_genre_limit: usize::MAX,
                ..Default::default()
            };
            let scene = build(&ui, (1043, 674), true, false, "", false, 40);
            assert!(scene
                .labels
                .iter()
                .any(|label| label.text.contains("1 genres selected")
                    && label.text.contains("120 options")));
            ui.flow_filter = "TestGenre119".into();
            let scene = build(&ui, (1043, 674), true, false, "", false, 40);
            assert!(scene
                .buttons
                .iter()
                .any(|button| matches!(button.action, Action::Genre(119))));
            ui.flow_filter = "TestGenre118".into();
            let scene = build(&ui, (1043, 674), true, false, "", false, 40);
            assert!(scene
                .buttons
                .iter()
                .any(|button| matches!(button.action, Action::Genre(118))));
        }
    }

    #[test]
    fn style_mix_limits_initial_options_and_searches_all() {
        let mut ui = Ui {
            view: View::StyleMix,
            ..Default::default()
        };
        let mut scene = Scene::default();
        style_mix_controls(&mut scene, &ui, 0, 0, 725);
        assert_eq!(
            scene
                .buttons
                .iter()
                .filter(|b| matches!(b.action, Action::MixRegion(_)))
                .count(),
            10
        );
        let last = crate::style_mix::REGIONS.last().unwrap().0;
        ui.flow_filter = last.to_string();
        let mut scene = Scene::default();
        style_mix_controls(&mut scene, &ui, 0, 0, 725);
        assert!(scene.buttons.iter().any(
            |b| matches!(b.action, Action::MixRegion(i) if i == crate::style_mix::REGIONS.len() - 1)
        ));
        ui.mix_region = 0;
        ui.flow_filter.clear();
        ui.mix_genres = Some(
            serde_json::json!({"genres": crate::style_mix::GENRES.iter().enumerate().map(|(id, g)| serde_json::json!({"id": id, "title": g.0})).collect::<Vec<_>>() }),
        );
        let mut scene = Scene::default();
        style_mix_controls(&mut scene, &ui, 0, 0, 725);
        assert_eq!(
            scene
                .buttons
                .iter()
                .filter(|b| matches!(b.action, Action::MixGenre(_)))
                .count(),
            10
        );
        ui.flow_filter = crate::style_mix::GENRES.last().unwrap().0.to_string();
        let mut scene = Scene::default();
        style_mix_controls(&mut scene, &ui, 0, 0, 725);
        assert!(scene.buttons.iter().any(
            |b| matches!(b.action, Action::MixGenre(i) if i == crate::style_mix::GENRES.len() - 1)
        ));
    }

    #[test]
    fn style_mix_pages_are_disjoint_bounded_and_searchable() {
        for cols in 1..=5 {
            let total = crate::style_mix::REGIONS.len();
            let ranges: Vec<_> = (0..total.div_ceil(cols as usize * 2))
                .map(|page| mix_page(total, page, cols))
                .collect();
            assert_eq!(ranges.last().unwrap().end, total);
            assert!(ranges
                .windows(2)
                .all(|pair| pair[0].start == 0 && pair[0].end < pair[1].end));
            assert!(mix_page(0, usize::MAX, cols).is_empty());
            assert!(mix_page(total, usize::MAX, cols).end <= total);
        }
        let mut ui = Ui {
            view: View::StyleMix,
            mix_language_page: 1,
            ..Default::default()
        };
        let mut scene = Scene::default();
        style_mix_controls(&mut scene, &ui, 0, 0, 725);
        assert!(scene
            .buttons
            .iter()
            .any(|b| matches!(b.action, Action::MixRegion(10)) && b.rect.h == 100));
        assert!(scene
            .buttons
            .iter()
            .any(|b| matches!(b.action, Action::MixRegion(0))));
        assert!(scene
            .buttons
            .iter()
            .any(|b| matches!(b.action, Action::MixMore(true)) && b.enabled));
        ui.mix_language_page = usize::MAX;
        ui.flow_filter = crate::style_mix::REGIONS.last().unwrap().0.into();
        let mut scene = Scene::default();
        style_mix_controls(&mut scene, &ui, 0, 0, 145);
        assert_eq!(
            scene
                .buttons
                .iter()
                .filter(|b| matches!(b.action, Action::MixRegion(_)))
                .count(),
            1
        );
        assert!(!scene
            .buttons
            .iter()
            .any(|b| matches!(b.action, Action::MixMore(_))));
    }

    #[test]
    fn style_mix_genre_pages_keep_provider_ids_before_opening_playlist() {
        for provider in [
            crate::catalog::Provider::Deezer,
            crate::catalog::Provider::Spotify,
            crate::catalog::Provider::YouTube,
        ] {
            let mut ui = Ui {
                provider,
                view: View::StyleMix,
                mix_region: 0,
                mix_genre: usize::MAX,
                mix_genres: Some(
                    serde_json::json!({"genres": (0..15).map(|id| serde_json::json!({"id":id,"title":format!("Fixture {id}")})).collect::<Vec<_>>()}),
                ),
                ..Default::default()
            };
            let mut first = Scene::default();
            style_mix_controls(&mut first, &ui, 0, 0, 725);
            ui.mix_genre_page = 1;
            let mut second = Scene::default();
            style_mix_controls(&mut second, &ui, 0, 0, 725);
            let ids = |scene: &Scene| {
                scene
                    .buttons
                    .iter()
                    .filter_map(|b| match b.action {
                        Action::MixGenre(id) => Some(id),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
            };
            assert_eq!(ids(&first), (0..10).collect::<Vec<_>>());
            assert_eq!(ids(&second), (0..15).collect::<Vec<_>>());
            assert!(second
                .buttons
                .iter()
                .any(|b| matches!(b.action, Action::MixGenre(14))));
            assert_eq!(ids(&second).len(), 15);
            assert!(!second
                .buttons
                .iter()
                .any(|b| matches!(b.action, Action::MixMore(false))));
            assert_eq!(ui.mix_genre, usize::MAX);
            ui.mix_genre = 14;
            let mut playlist = Scene::default();
            style_mix_controls(&mut playlist, &ui, 0, 0, 725);
            assert!(ids(&playlist).is_empty());
            assert!(playlist
                .buttons
                .iter()
                .any(|b| matches!(b.action, Action::BuildStyleMix)));
        }
    }

    #[test]
    fn flow_continuation_extends_scrolled_track_list() {
        let rows: Vec<_> = (0..30)
            .map(|i| crate::catalog::Entry {
                id: format!("track-{i}"),
                kind: "track".into(),
                title: format!("Track {i}"),
                ..Default::default()
            })
            .collect();
        let mut ui = Ui {
            view: View::Flow,
            scroll: 800,
            page: Some(std::sync::Arc::new(crate::catalog::Page {
                rows: rows[..20].to_vec(),
                next: "cursor".into(),
                ..Default::default()
            })),
            ..Default::default()
        };
        let before = build(&ui, (1200, 900), true, false, "", false, 50);
        let anchor = before
            .buttons
            .iter()
            .find(|button| button.text == "Track 15")
            .unwrap()
            .rect;
        std::sync::Arc::make_mut(ui.page.as_mut().unwrap()).loading = true;
        let loading = build(&ui, (1200, 900), true, false, "", false, 50);
        assert_eq!(loading.content_height, before.content_height);
        assert_eq!(
            loading
                .buttons
                .iter()
                .find(|button| button.text == "Track 15")
                .unwrap()
                .rect
                .y,
            anchor.y
        );
        assert!(loading
            .buttons
            .iter()
            .any(|button| matches!(button.action, Action::More) && !button.enabled));
        std::sync::Arc::make_mut(ui.page.as_mut().unwrap()).loading = false;
        std::sync::Arc::make_mut(ui.page.as_mut().unwrap()).rows = rows;
        let after = build(&ui, (1200, 900), true, false, "", false, 50);
        assert_eq!(after.content_height - before.content_height, 10 * 56);
        assert_eq!(ui.scroll, 800);
        assert_eq!(
            after
                .buttons
                .iter()
                .find(|button| button.text == "Track 15")
                .unwrap()
                .rect
                .y,
            anchor.y
        );
        assert!(after.buttons.iter().any(|button| button.text == "Track 20"));
    }

    #[test]
    fn style_mix_opens_from_navigation_and_selects_country_and_genre_independently() {
        let mut ui = Ui {
            view: View::Flow,
            page: Some(std::sync::Arc::new(crate::catalog::Page::default())),
            ..Default::default()
        };
        let scene = build(&ui, (1200, 900), true, false, "", false, 50);
        assert!(scene
            .buttons
            .iter()
            .any(|b| matches!(b.action, Action::Go(View::StyleMix))));
        ui.view = View::StyleMix;
        let initial = build(&ui, (1200, 900), true, false, "", false, 50);
        assert_eq!(
            initial
                .buttons
                .iter()
                .filter(|b| matches!(b.action, Action::MixRegion(_)))
                .count(),
            10
        );
        assert!(!initial.buttons.iter().any(|b| matches!(
            b.action,
            Action::MixGenre(_) | Action::PlayAll | Action::BuildStyleMix
        )));
        ui.mix_region = 1;
        let loading = build(&ui, (1200, 900), true, false, "", false, 50);
        assert!(loading
            .labels
            .iter()
            .any(|label| label.text == "Checking available genres..."));
        assert!(!loading
            .buttons
            .iter()
            .any(|b| matches!(b.action, Action::MixGenre(_))));
        ui.mix_genre = usize::MAX;
        ui.mix_genres = Some(
            serde_json::json!({"region":1,"genres":[{"id":1,"title":"Rock"},{"id":24,"title":"Indie Rock"}]}),
        );
        for size in [(800, 600), (1200, 900)] {
            let scene = build(&ui, size, true, false, "", false, 50);
            assert!(!scene
                .buttons
                .iter()
                .any(|b| matches!(b.action, Action::MixRegion(_))));
            assert_eq!(
                scene
                    .buttons
                    .iter()
                    .filter(|b| matches!(b.action, Action::MixGenre(_)))
                    .count(),
                2
            );
            assert!(!scene
                .buttons
                .iter()
                .any(|b| matches!(b.action, Action::MixLanguages)));
            assert!(!scene
                .buttons
                .iter()
                .any(|b| matches!(b.action, Action::PlayAll | Action::BuildStyleMix)));
            ui.mix_genre = 1;
            let scene = build(&ui, size, true, false, "", false, 50);
            assert!(!scene
                .buttons
                .iter()
                .any(|b| matches!(b.action, Action::MixGenre(_) | Action::MixRegion(_))));
            assert!(scene
                .buttons
                .iter()
                .any(|b| matches!(b.action, Action::PlayAll) && !b.enabled));
            assert!(scene
                .buttons
                .iter()
                .any(|b| matches!(b.action, Action::BuildStyleMix)));
            assert!(scene.search.is_none());
            let provider_caption = scene
                .labels
                .iter()
                .find(|label| label.text.starts_with("From matching "))
                .unwrap();
            assert!(provider_caption.rect.y + provider_caption.rect.h <= scene.content_top);
            ui.mix_genre = usize::MAX;
            for b in scene.buttons.iter().filter(|b| {
                matches!(
                    b.action,
                    Action::MixRegion(_)
                        | Action::MixGenre(_)
                        | Action::BuildStyleMix
                        | Action::PlayAll
                )
            }) {
                assert!(b.rect.x >= 0 && b.rect.x + b.rect.w <= size.0);
            }
        }
    }

    #[test]
    fn compact_queue_metadata_remains_reachable_in_small_windows() {
        let name = format!("Çağrı {} 日本語", "W".repeat(54));
        let mut ui = Ui {
            queue_sidebar_open: true,
            ..Default::default()
        };
        ui.queue = (0..20)
            .map(|i| crate::catalog::Entry {
                id: i.to_string(),
                title: "A long track title with several words".into(),
                subtitle: "An artist with a long credit".into(),
                artwork: "queue-cover".into(),
                raw: serde_json::json!({"added_by":name}),
                ..Default::default()
            })
            .collect();
        ui.now = ui.queue[0].clone();
        for size in [(800, 600), (1200, 800), (1600, 1000)] {
            let scene = build(&ui, size, true, false, "", false, 50);
            assert!(scene.queue_scroll_max > 0);
            assert!(scene
                .buttons
                .iter()
                .filter(|b| matches!(b.action, Action::QueueJump(_)))
                .all(|b| b.artwork.is_empty() && b.rect.h <= 72));
            assert!(scene
                .pictures
                .iter()
                .any(|(r, url)| url == "queue-cover" && r.w == 44 && r.h == 44));
            let mut seen = String::new();
            for offset in 0..=scene.queue_scroll_max {
                ui.queue_scroll = offset;
                let frame = build(&ui, size, true, false, "", false, 50);
                for (index, label) in frame.labels.iter().enumerate() {
                    if label.rect.x >= frame.queue_viewport.unwrap().x {
                        let viewport = frame.queue_viewport.unwrap();
                        let clipped = frame.queue_row_labels.contains(&index);
                        let bottom = if clipped {
                            (label.rect.y + label.rect.h).min(viewport.y + viewport.h)
                        } else {
                            label.rect.y + label.rect.h
                        };
                        assert!(bottom <= size.1 - player_height(&ui, size.0));
                        if !clipped
                            || label.rect.y < viewport.y + viewport.h
                                && label.rect.y + label.rect.h > viewport.y
                        {
                            seen.push_str(&label.text);
                        }
                    }
                }
            }
            assert!(seen.contains("Added by"));
            assert!(seen.contains("日本語"));
            ui.queue_scroll = 0;
        }
    }
    #[test]
    fn timed_lyric_follow_resumes_after_manual_scroll_but_untimed_scroll_stays_put() {
        let now = std::time::Instant::now();
        let mut ui = Ui::default();
        ui.view = View::Lyrics;
        ui.rich_lyrics = vec![
            crate::lyrics::LyricLine {
                start_ms: 1000,
                end_ms: 2000,
                text: "First".into(),
                ..Default::default()
            },
            crate::lyrics::LyricLine {
                start_ms: 2000,
                end_ms: 3000,
                text: "Second".into(),
                ..Default::default()
            },
        ];
        ui.position = 2500;
        ui.hold_lyric_follow(now);
        assert!(build(&ui, (1200, 800), true, false, "", true, 50)
            .lyrics_anchor
            .is_none());
        assert!(!ui.resume_lyric_follow(now + std::time::Duration::from_secs(3)));
        assert!(ui.resume_lyric_follow(now + std::time::Duration::from_secs(4)));
        assert!(build(&ui, (1200, 800), true, false, "", true, 50)
            .lyrics_anchor
            .is_some());
        ui.rich_lyrics.iter_mut().for_each(|line| {
            line.start_ms = 0;
            line.end_ms = 0;
        });
        ui.scroll = 600;
        ui.hold_lyric_follow(now);
        assert!(!ui.lyrics_manual_scroll);
        assert!(!ui.resume_lyric_follow(now + std::time::Duration::from_secs(10)));
        assert_eq!(ui.scroll, 600);
    }
    #[test]
    fn hiding_lyrics_centers_the_cover_and_moves_its_controls_together() {
        let mut ui = Ui::default();
        ui.view = View::Lyrics;
        let left = build(&ui, (1200, 800), true, false, "", true, 50);
        ui.lyrics_cover_mode = true;
        ui.lyrics_cover_mix = Some(0.5);
        let middle = build(&ui, (1200, 800), true, false, "", true, 50);
        ui.lyrics_cover_mix = None;
        let centered = build(&ui, (1200, 800), true, false, "", true, 50);
        let a = left.lyrics_cover.unwrap();
        let b = middle.lyrics_cover.unwrap();
        let c = centered.lyrics_cover.unwrap();
        assert!((c.x + c.w / 2 - 600).abs() <= 1);
        assert!(a.x < b.x && b.x < c.x);
        assert_eq!(left.seek.unwrap().x - a.x, centered.seek.unwrap().x - c.x);
        assert_eq!(left.seek.unwrap().y - a.y, centered.seek.unwrap().y - c.y);
        let button = centered
            .buttons
            .iter()
            .find(|b| matches!(b.action, Action::ToggleLyricsVisualizer))
            .unwrap();
        assert_eq!(button.rect.x, 32);
        assert!(button.rect.y > 700);
    }
    #[test]
    fn lyrics_metadata_uses_the_left_column_without_touching_the_words() {
        let mut ui = Ui::default();
        ui.view = View::Lyrics;
        ui.now.title = "A very long song title that needs the entire left column".into();
        ui.now.subtitle = "Jessie Reyez and an unusually long artist credit".into();
        for size in [(746, 650), (1200, 800), (1600, 1000)] {
            let scene = build(&ui, size, true, false, "", true, 50);
            let cover = scene.lyrics_cover.unwrap();
            let area = scene.lyrics_area.unwrap();
            let title = scene.labels.iter().find(|l| l.kind == 10).unwrap();
            assert!(title.rect.w >= cover.w);
            assert!(title.rect.x + title.rect.w <= area.x - 8);
            assert_eq!(title.text, ui.now.title);
            for artist in scene
                .buttons
                .iter()
                .filter(|b| b.artist_credit_group.is_some())
            {
                assert!(artist.rect.x + artist.rect.w <= area.x - 8);
            }
        }
    }
    #[test]
    fn long_untimed_lyrics_start_at_the_shared_focus_and_keep_an_accessible_scrollbar() {
        let mut ui = Ui::default();
        ui.view = View::Lyrics;
        ui.rich_lyrics = (0..150)
            .map(|i| crate::lyrics::LyricLine {
                text: format!("Complete lyric line {i}"),
                ..Default::default()
            })
            .collect();
        let scene = build(&ui, (1200, 800), true, false, "", false, 50);
        let first = scene
            .labels
            .iter()
            .find(|l| l.text == "Complete lyric line 0")
            .unwrap();
        let area = scene.lyrics_area.unwrap();
        assert_eq!(first.rect.y, area.y + (area.h as f32 * 0.3).round() as i32);
        assert!(scene.panels.iter().any(|p| p.kind == 6));
        ui.scroll = 1200;
        ui.lyrics_manual_scroll = true;
        ui.go(View::Artist);
        ui.back();
        assert_eq!(ui.scroll, 0);
        assert!(!ui.lyrics_manual_scroll);
        let restored = build(&ui, (1200, 800), true, false, "", false, 50);
        assert_eq!(
            restored
                .labels
                .iter()
                .find(|l| l.text == "Complete lyric line 0")
                .unwrap()
                .rect
                .y,
            first.rect.y
        );
    }
    #[test]
    fn theme_controls_are_available_over_song_art_and_adaptive_colors_are_gone() {
        let ui = Ui {
            view: View::Settings,
            tab: 2,
            background_mode: 1,
            ..Default::default()
        };
        let scene = build(&ui, (1200, 900), true, false, "", false, 50);
        assert!(scene
            .buttons
            .iter()
            .any(|b| matches!(b.action, Action::SolidTheme(_))));
        assert!(!scene
            .buttons
            .iter()
            .any(|b| matches!(b.action, Action::ToggleBool("adaptive_colors"))));
        assert!(scene
            .sliders
            .iter()
            .any(|(_, key, _)| *key == "Surface opacity"));
    }
    use super::*;

    #[test]
    fn startup_page_selection_survives_reloading_preferences() {
        for (index, expected) in [
            View::Home,
            View::Explore,
            View::Flow,
            View::Favorites,
            View::Playlists,
            View::Offline,
        ]
        .into_iter()
        .enumerate()
        {
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
    fn keyboard_shortcuts_persist_and_settings_show_capture_and_defaults() {
        let mut ui = Ui {
            view: View::Settings,
            tab: TABS.len() - 1,
            ..Default::default()
        };
        assert!(ui.shortcuts.iter().all(Option::is_none));
        ui.shortcuts[0] = Some(crate::shortcuts::Binding {
            modifiers: 2,
            key: 0x50,
        });
        ui.save_preferences();
        assert_eq!(Ui::load_preferences().shortcuts, ui.shortcuts);
        ui.shortcut_capture = Some(2);
        for width in [340, 580, 1000] {
            let mut scene = Scene::default();
            settings(&mut scene, &ui, 0, 150, width, true, false);
            assert_eq!(
                scene
                    .buttons
                    .iter()
                    .filter(|b| matches!(b.action, Action::BindShortcut(_)))
                    .count(),
                crate::shortcuts::ACTIONS.len()
            );
            assert!(scene.buttons.iter().any(|b| b.text == "Ctrl + P"));
            assert!(scene.buttons.iter().any(|b| b.text == "Media previous"));
            assert!(scene
                .buttons
                .iter()
                .any(|b| b.text == "Press a shortcut..."));
        }
        ui.shortcuts[0] = None;
        ui.save_preferences();
        assert!(Ui::load_preferences().shortcuts.iter().all(Option::is_none));
        let _ = std::fs::remove_dir_all(preferences_base().unwrap());
    }
    #[test]
    fn update_settings_expose_only_actions_valid_for_the_current_phase() {
        use crate::updater::{Phase, State};
        let mut ui = Ui {
            view: View::Settings,
            tab: 0,
            ..Default::default()
        };
        ui.auto_updates = false;
        ui.last_update_check = 1234;
        ui.save_preferences();
        let loaded = Ui::load_preferences();
        assert!(!loaded.auto_updates);
        assert_eq!(loaded.last_update_check, 1234);
        for width in [340, 1000] {
            for phase in [
                Phase::Idle,
                Phase::Available,
                Phase::Downloading,
                Phase::Ready,
                Phase::Applying,
                Phase::Failed,
            ] {
                ui.update = State {
                    phase: phase.clone(),
                    version: "0.2.0".into(),
                    ..Default::default()
                };
                let mut scene = Scene::default();
                settings(&mut scene, &ui, 0, 150, width, true, false);
                assert_eq!(
                    scene
                        .buttons
                        .iter()
                        .any(|b| matches!(b.action, Action::DownloadUpdate)),
                    phase == Phase::Available
                );
                assert_eq!(
                    scene
                        .buttons
                        .iter()
                        .any(|b| matches!(b.action, Action::ApplyUpdate)),
                    phase == Phase::Ready
                );
                assert_eq!(
                    scene
                        .buttons
                        .iter()
                        .any(|b| matches!(b.action, Action::CheckUpdate)),
                    !ui.update.busy()
                );
            }
        }
        let _ = std::fs::remove_dir_all(preferences_base().unwrap());
    }

    #[test]
    fn content_controls_are_kept_inside_left_boundary() {
        let mut scene = Scene::default();
        scene.button(Rect::new(40, 0, 80, 30), "Outside", "", Action::More, false);
        scene.button(Rect::new(50, 0, 80, 30), "Content", "", Action::More, false);
        scene.buttons[1].artist_credit_group = Some(Rect::new(45, 0, 100, 30));
        scene.label(30, 0, 100, "Error", 1);
        scene.content_buttons = 1..2;
        scene.content_labels = 0..1;

        keep_content_controls_visible(&mut scene, 70);

        assert_eq!(scene.buttons[0].rect.x, 40);
        assert_eq!(scene.buttons[1].rect, Rect::new(70, 0, 80, 30));
        assert_eq!(scene.buttons[1].artist_credit_group.unwrap().x, 65);
        assert_eq!(scene.labels[0].rect.x, 70);
    }

    #[test]
    fn catalog_messages_stay_inside_sidebar_clip() {
        for collapsed in [false, true] {
            let ui = Ui {
                sidebar_collapsed: collapsed,
                ..Default::default()
            };
            let x = (76. + 186. * ui.sidebar_values().0).round() as i32;
            for page in [
                crate::catalog::Page::default(),
                crate::catalog::Page {
                    sections: vec![crate::catalog::Section {
                        error: "Section error".into(),
                        ..Default::default()
                    }],
                    ..Default::default()
                },
            ] {
                let mut scene = Scene::default();
                live_catalog_contents(&mut scene, &ui, &page, x, 200, 600, 800);
                let message = scene
                    .labels
                    .iter()
                    .find(|label| {
                        label.text == "Section error"
                            || label.text.starts_with("No items were returned")
                    })
                    .unwrap();
                assert!(message.rect.x >= ui.content_left());
            }
        }
    }
    #[test]
    fn empty_results_message_localizes_each_provider() {
        let mut ui = Ui::default();
        for (language, expected) in [
            ("tr", "üzerinden sonuç gelmedi"),
            ("az", "vasitəsilə nəticə gəlmədi"),
        ] {
            ui.set_language(language);
            for provider in [
                crate::catalog::Provider::Deezer,
                crate::catalog::Provider::Spotify,
                crate::catalog::Provider::YouTube,
            ] {
                let text = format!("No items were returned by {}.", provider.label());
                let translated = ui.translated_text(&text);
                assert!(translated.contains(provider.label()));
                assert!(translated.contains(expected));
            }
        }
    }

    #[test]
    fn bundled_languages_have_matching_keys_and_valid_unicode() {
        let tr: std::collections::HashMap<String, String> =
            serde_json::from_str(include_str!("../../../../assets/localization/tr.json")).unwrap();
        let az: std::collections::HashMap<String, String> =
            serde_json::from_str(include_str!("../../../../assets/localization/az.json")).unwrap();
        assert_eq!(
            tr.keys().collect::<std::collections::HashSet<_>>(),
            az.keys().collect()
        );
        for catalog in [&tr, &az] {
            assert!(catalog
                .iter()
                .all(|(k, v)| !k.contains('\u{fffd}') && !v.contains('\u{fffd}') && !v.is_empty()));
            for (key, value) in catalog {
                let chars: Vec<_> = value.chars().collect();
                assert!(
                    !chars
                        .windows(2)
                        .any(|pair| pair[0] == '?' && (pair[1].is_alphabetic() || pair[1] == '?')),
                    "possible corrupted translation for {key}: {value}"
                );
            }
            let setup_notice = &catalog[crate::spotify::wizard::NOTICE];
            assert_eq!(setup_notice.matches('?').count(), 1);
        }
    }

    #[test]
    fn new_ui_messages_and_provider_notices_are_localized() {
        let mut ui = Ui::default();
        for language in ["tr", "az"] {
            ui.set_language(language);
            for text in [
                "Custom light",
                "Custom dark",
                "More options",
                "Enter a playlist name",
                "Delete download",
                "Go to album",
                "Loading song radio…",
                "Click digits to change port",
                "Select all",
                "Connect to Spotify first",
            ] {
                assert_ne!(ui.translated_text(text), text, "{language}: {text}");
            }
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
                assert_ne!(
                    ui.translated_text("English   \u{25be}"),
                    "English   \u{25be}"
                );
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
        ui.queue.push(crate::catalog::Entry {
            title: "Home".into(),
            ..Default::default()
        });
        let mut scene = Scene::default();
        scene.label(0, 0, 300, "Home", 8);
        scene.label(0, 30, 300, "Welcome back", 0);
        scene.button(Rect::new(0, 60, 100, 30), "", "", Action::Toggle, false);
        scene.buttons[0].accessible = "Play".into();
        translate_scene(&mut scene, &ui);
        assert_eq!(scene.labels[0].text, "Home");
        assert_eq!(scene.labels[1].text, ui.t("Welcome back"));
        assert_eq!(scene.buttons[0].accessible, ui.t("Play"));
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
            let cards: Vec<_> = scene
                .buttons
                .iter()
                .filter_map(|button| match &button.action {
                    Action::OpenItem(item) if !button.text_link => Some((button, item)),
                    _ => None,
                })
                .collect();
            let shelves = if filter == 0 {
                6
            } else if filter == 1 {
                2
            } else {
                1
            };
            assert_eq!(cards.len(), shelves * 8);
            assert_eq!(
                scene
                    .buttons
                    .iter()
                    .filter(|b| matches!(b.action, Action::ShelfMore(_, 4)))
                    .count(),
                shelves
            );
            assert!(!scene
                .buttons
                .iter()
                .any(|b| matches!(b.action, Action::ShelfAll(_))));
            if filter >= 2 {
                let title = ["", "", "Genres", "Categories", "Podcasts by category"][filter];
                assert!(cards.iter().all(|(_, item)| item.id.starts_with(title)));
            }
        }
    }
    #[test]
    fn spotify_discovery_reveals_exactly_two_more_rows_per_shelf() {
        let page = crate::catalog::Page {
            sections: [
                ("Genres", 28),
                ("Categories", 35),
                ("Podcasts by category", 4),
            ]
            .into_iter()
            .map(|(title, count)| crate::catalog::Section {
                title: title.into(),
                entries: (0..count)
                    .map(|i| crate::catalog::Entry {
                        id: format!("spotify:page:{title}{i}"),
                        title: format!("{title} {i}"),
                        kind: "channel".into(),
                        artwork: format!("https://i.scdn.co/image/{title}{i}"),
                        ..Default::default()
                    })
                    .collect(),
                ..Default::default()
            })
            .collect(),
            raw: vec![("discover".into(), serde_json::json!(true))],
            ..Default::default()
        };
        let mut ui = Ui {
            provider: crate::catalog::Provider::Spotify,
            view: View::Explore,
            ..Default::default()
        };
        let mut scene = Scene::default();
        live_catalog(&mut scene, &ui, &page, 0, 0, 900, 10000);
        let cards: Vec<_> = scene
            .buttons
            .iter()
            .filter(|b| matches!(&b.action, Action::OpenItem(item) if item.kind == "channel"))
            .collect();
        assert_eq!(cards.len(), 20);
        assert!(cards.iter().all(|card| !card.artwork.is_empty()));
        assert!(!scene
            .buttons
            .iter()
            .any(|b| matches!(b.action, Action::ShelfAll(_))));
        assert_eq!(
            scene
                .buttons
                .iter()
                .filter(|b| matches!(b.action, Action::ShelfMore(_, 4)))
                .count(),
            2
        );
        for (rows, expected) in [(4, 28), (6, 36)] {
            ui.shelf_rows.insert("Genres".into(), rows);
            let mut scene = Scene::default();
            live_catalog(&mut scene, &ui, &page, 0, 0, 900, 10000);
            assert_eq!(
                scene
                    .buttons
                    .iter()
                    .filter(|b| matches!(&b.action,Action::OpenItem(item) if item.kind=="channel"))
                    .count(),
                expected
            );
        }
    }
    #[test]
    fn spotify_subcatalogue_cards_use_two_row_batches_in_place() {
        let page = crate::catalog::Page {
            entity: Some(crate::catalog::Entry {
                kind: "channel".into(),
                id: "spotify:page:rock".into(),
                ..Default::default()
            }),
            sections: vec![crate::catalog::Section {
                title: "Rock Classics".into(),
                entries: (0..20)
                    .map(|i| crate::catalog::Entry {
                        kind: "playlist".into(),
                        id: format!("spotify:playlist:{i}"),
                        title: format!("Playlist {i}"),
                        ..Default::default()
                    })
                    .collect(),
                ..Default::default()
            }],
            ..Default::default()
        };
        let mut ui = Ui {
            provider: crate::catalog::Provider::Spotify,
            view: View::Artist,
            ..Default::default()
        };
        for (rows, expected) in [(2, 8), (4, 16), (6, 20)] {
            ui.shelf_rows.insert("Rock Classics".into(), rows);
            let mut scene = Scene::default();
            live_catalog(&mut scene, &ui, &page, 0, 0, 900, 10000);
            assert_eq!(
                scene
                    .buttons
                    .iter()
                    .filter(|b| !b.text_link
                        && matches!(&b.action,Action::OpenItem(item) if item.kind=="playlist"))
                    .count(),
                expected
            );
            assert_eq!(
                scene
                    .buttons
                    .iter()
                    .any(|b| matches!(b.action, Action::ShelfMore(_, 4))),
                expected < 20
            );
        }
    }

    #[test]
    fn timed_lyrics_virtualize_rows_without_breaking_manual_scroll() {
        let mut ui = Ui {
            view: View::Lyrics,
            position: 60_000,
            duration: 180_000,
            rich_lyrics: (0..150)
                .map(|i| crate::lyrics::LyricLine {
                    start_ms: i * 1000,
                    text: format!("Line {i}"),
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        };
        let automatic = build(&ui, (1000, 800), true, false, "", true, 50);
        assert!(automatic.lyric_rows.len() < 100);
        assert!(automatic.lyric_rows.iter().any(|row| row.line == 60));
        let active = automatic
            .labels
            .iter()
            .find(|label| label.kind == 13)
            .unwrap();
        assert_eq!(automatic.lyrics_focus_y, Some(active.rect.y));
        ui.lyrics_manual_scroll = true;
        let manual = build(&ui, (1000, 800), true, false, "", true, 50);
        assert!(manual.lyric_rows.len() < 100);
        assert!(manual.lyric_rows.iter().any(|row| row.line == 0));
        assert!(!manual.lyric_rows.iter().any(|row| row.line == 60));
        assert!(manual.lyrics_focus_y.unwrap() > 1600);
        ui.scroll = 4_000;
        let scrolled = build(&ui, (1000, 800), true, false, "", true, 50);
        assert!(scrolled.lyric_rows.len() < 100);
        assert!(scrolled.lyric_rows.iter().any(|row| row.line > 70));
        assert_eq!(
            scrolled.lyrics_focus_y,
            manual.lyrics_focus_y.map(|y| y - 4000)
        );
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
                .filter(|l| matches!(l.kind, 12 | 13))
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
    fn lyric_formats_share_geometry_focus_and_scroll_extent() {
        for size in [(800, 600), (1200, 800), (1600, 1000)] {
            for scale in [60, 100, 180] {
                let mut ui = Ui {
                    view: View::Lyrics,
                    lyrics_size_panel: scale,
                    lyrics_size_fullscreen: scale,
                    ..Default::default()
                };
                ui.rich_lyrics = ["First lyric", "Second lyric", "Last lyric"]
                    .into_iter()
                    .map(|text| crate::lyrics::LyricLine {
                        text: text.into(),
                        ..Default::default()
                    })
                    .collect();
                let geometry = |scene: &Scene| {
                    scene
                        .lyric_rows
                        .iter()
                        .map(|row| {
                            let label = &scene.labels[row.label];
                            (label.text.clone(), label.rect, label.kind)
                        })
                        .collect::<Vec<_>>()
                };
                let plain = build(&ui, size, true, false, "", false, 50);
                for (i, line) in ui.rich_lyrics.iter_mut().enumerate() {
                    line.start_ms = i as u64 * 1000;
                }
                let line = build(&ui, size, true, false, "", true, 50);
                for lyric in &mut ui.rich_lyrics {
                    lyric.syllables.push(crate::lyrics::Syllable {
                        text: lyric.text.clone(),
                        start_ms: lyric.start_ms,
                        end_ms: lyric.start_ms + 900,
                    });
                }
                let word = build(&ui, size, true, false, "", true, 50);
                assert_eq!(geometry(&plain), geometry(&line));
                assert_eq!(geometry(&line), geometry(&word));
                assert_eq!(plain.content_height, word.content_height);
                let offset = geometry(&plain)[1].1.y - geometry(&plain)[0].1.y;
                ui.position = 1000;
                let followed = build(&ui, size, true, false, "", true, 50);
                ui.rich_lyrics.iter_mut().for_each(|line| {
                    line.start_ms = 0;
                    line.syllables.clear();
                });
                ui.scroll = offset;
                let scrolled = build(&ui, size, true, false, "", false, 50);
                assert_eq!(geometry(&followed), geometry(&scrolled));
                assert!(!scrolled
                    .buttons
                    .iter()
                    .any(|b| matches!(b.action, Action::SeekMs(_))));
            }
        }
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
            for tab in 0..TABS.len() {
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
                assert_eq!(tabs.len(), TABS.len());
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
        assert!(
            first
                .buttons
                .iter()
                .filter(|b| matches!(b.action, Action::Genre(_)))
                .count()
                < 120
        );
        assert!(first
            .buttons
            .iter()
            .any(|b| matches!(b.action, Action::FlowMoreGenres)));
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
    fn flow_provider_selections_survive_switches_and_legacy_preferences() {
        use crate::catalog::Provider;
        let mut ui = Ui::default();
        ui.restore_flow_preferences(&serde_json::json!({"FlowConfigId":"happy"}));
        assert_eq!(ui.flow_id, "happy");
        ui.switch_flow_provider(Provider::Spotify);
        ui.provider = Provider::Spotify;
        assert_eq!(ui.flow_id, "default");
        let spotify = crate::spotify::flow::Selection {
            mood: "focus".into(),
            genres: vec!["Ambient".into()],
        }
        .key();
        ui.flow_id = spotify.clone();
        assert_eq!(ui.flow_preferences(), ("happy", spotify.as_str()));
        ui.flow_filter = "Ambient".into();
        ui.switch_flow_provider(Provider::Deezer);
        ui.provider = Provider::Deezer;
        assert_eq!(ui.flow_id, "happy");
        assert!(ui.flow_filter.is_empty());
        ui.switch_flow_provider(Provider::Spotify);
        ui.provider = Provider::Spotify;
        assert_eq!(ui.flow_id, spotify);
        let mut restored = Ui::default();
        restored.restore_flow_preferences(&serde_json::json!({"FlowConfigId":spotify}));
        assert_eq!(restored.flow_id, "default");
        assert_eq!(restored.spotify_flow_id, spotify);
        restored.restore_flow_preferences(&serde_json::json!({
            "DeezerFlowConfigId":"happy", "SpotifyFlowConfigId":spotify
        }));
        assert_eq!(restored.flow_preferences(), ("happy", spotify.as_str()));
    }
    #[test]
    fn playlist_flow_restores_mood_and_offers_see_more() {
        for provider in [
            crate::catalog::Provider::Spotify,
            crate::catalog::Provider::YouTube,
        ] {
            let selection = crate::spotify::flow::Selection {
                mood: "chill".into(),
                genres: vec!["Jazz".into()],
            };
            let mut ui = Ui {
                provider,
                view: View::FlowTune,
                flow_id: selection.key(),
                flow_personal: false,
                flow_loaded: true,
                flow_customizable: true,
                flow_options: crate::spotify::flow::MOODS
                    .iter()
                    .map(|(id, title)| (id.to_string(), title.to_string()))
                    .collect(),
                genres: crate::spotify::flow::genres()
                    .iter()
                    .map(|g| (g.to_string(), g.to_string(), *g == "Jazz", *g == "Jazz"))
                    .collect(),
                ..Default::default()
            };
            let scene = build(&ui, (1043, 674), true, false, "", false, 40);
            assert!(scene
                .buttons
                .iter()
                .any(|b| matches!(b.action, Action::FlowChoice(1)) && b.selected));
            assert!(scene.genre_viewport.unwrap().h >= 42);
            assert!(scene
                .labels
                .iter()
                .any(|label| label.text.contains("Search all provider genres")));
            assert_eq!(
                scene
                    .buttons
                    .iter()
                    .filter(|b| matches!(b.action, Action::Genre(_)))
                    .count(),
                20
            );
            assert!(scene
                .buttons
                .iter()
                .any(|b| matches!(b.action, Action::FlowMoreGenres)));
            ui.flow_genre_limit = usize::MAX;
            let expanded = build(&ui, (1043, 674), true, false, "", false, 40);
            assert!(
                expanded
                    .buttons
                    .iter()
                    .filter(|b| matches!(b.action, Action::Genre(_)))
                    .count()
                    < 60
            );
            assert!(!expanded
                .buttons
                .iter()
                .any(|b| matches!(b.action, Action::Genre(_)) && b.text == "melodic death metal"));
            assert!(expanded
                .buttons
                .iter()
                .any(|b| matches!(b.action, Action::FlowMoreGenres) && b.text == "Show less"));
            ui.flow_filter = "arabesk".into();
            let searched = build(&ui, (1043, 674), true, false, "", false, 40);
            assert!(searched
                .buttons
                .iter()
                .any(|b| matches!(b.action, Action::Genre(_))
                    && b.text.eq_ignore_ascii_case("arabesk")));
            ui.flow_filter = "My emerging subgenre".into();
            let custom = build(&ui, (1043, 674), true, false, "", false, 40);
            assert!(custom
                .buttons
                .iter()
                .any(|b| matches!(b.action, Action::FlowAddGenre)));
            ui.flow_filter.clear();
            ui.flow_genre_limit = 30;
            ui.genres
                .iter_mut()
                .find(|g| g.1 == "melodic death metal")
                .unwrap()
                .2 = true;
            ui.freeze_flow_order();
            let collapsed = build(&ui, (1043, 674), true, false, "", false, 40);
            assert!(collapsed
                .buttons
                .iter()
                .any(|b| matches!(b.action, Action::Genre(_))
                    && b.text == "melodic death metal"
                    && b.selected));
            assert!(
                collapsed
                    .buttons
                    .iter()
                    .filter(|b| matches!(b.action, Action::Genre(_)))
                    .count()
                    <= 30
            );
            ui.view = View::Flow;
            ui.page = Some(std::sync::Arc::new(crate::catalog::Page {
                rows: vec![crate::catalog::Entry {
                    id: "spotify:track:test".into(),
                    kind: "track".into(),
                    title: "Test".into(),
                    ..Default::default()
                }],
                next: "spotify-flow-page:test".into(),
                ..Default::default()
            }));
            let scene = build(&ui, (1043, 674), true, false, "", false, 40);
            assert!(scene
                .buttons
                .iter()
                .any(|b| matches!(b.action, Action::More) && b.text == "See more"));
        }
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
    fn page_headers_display_descriptive_captions_instead_of_collections_counters() {
        for view in [
            View::Favorites,
            View::Recent,
            View::Albums,
            View::Playlists,
            View::Home,
            View::Explore,
        ] {
            let ui = Ui {
                view,
                page: Some(std::sync::Arc::new(crate::catalog::Page {
                    title: view_title(&Ui::default(), view).into(),
                    subtitle: "123 tracks / 0 collections".into(),
                    rows: vec![crate::catalog::Entry {
                        id: "1".into(),
                        kind: "track".into(),
                        title: "Track".into(),
                        ..Default::default()
                    }],
                    ..Default::default()
                })),
                ..Default::default()
            };
            let scene = build(&ui, (1000, 700), true, false, "", false, 40);
            assert!(
                !scene.labels.iter().any(|l| l.text.contains("collections")),
                "view {view:?} had collections counter in header"
            );
        }
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
        assert!(
            !ui.jam_enabled,
            "Connect must be opt-in to avoid network prompts on startup"
        );
        ui.go(View::Settings);
        ui.tab = 5;
        ui.provider = crate::catalog::Provider::Spotify;
        ui.jam_enabled = true;
        ui.jam_url = "http://192.168.1.20:3132/".into();
        for size in [(520, 700), (1120, 700)] {
            let scene = build(&ui, size, false, false, "", false, 70);
            assert!(!scene
                .labels
                .iter()
                .any(|label| matches!(label.text.as_str(), "DeezNats Connect" | "Spotify Jam")));
            assert!(!scene.buttons.iter().any(|button| matches!(
                button.action,
                Action::OpenConnect
                    | Action::CopyConnectLink
                    | Action::ToggleBool("jam_enabled")
                    | Action::SpotifyJamRefresh
            )));
        }
    }

    #[test]
    fn settings_top_bar_and_tabs_remain_fixed_while_content_scrolls() {
        let mut ui = Ui::default();
        ui.go(View::Settings);
        ui.tab = 2;
        // Custom themes live at the end of the paged theme shelf.
        ui.theme_shelf_offset = SOLID_THEMES.len();
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

        let theme_button = |scene: &Scene| {
            scene
                .buttons
                .iter()
                .position(|b| matches!(b.action, Action::CustomTheme(false)))
                .unwrap()
        };
        let idx0 = theme_button(&s0);
        let idx1 = theme_button(&s1);
        assert_eq!(s0.buttons[idx0].rect.y - 300, s1.buttons[idx1].rect.y);
        assert!(s0.content_buttons.contains(&idx0));
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
    fn settings_remove_downloads_dispatches_real_removal() {
        let mut ui = Ui::default();
        ui.view = View::Settings;
        let scene = build(&ui, (1120, 900), true, false, "", false, 70);
        assert!(scene.buttons.iter().any(|button| {
            button.text == "Remove all" && matches!(button.action, Action::RemoveDownloads)
        }));
    }

    #[test]
    fn download_progress_lives_in_offline_catalog() {
        let mut ui = Ui::default();
        ui.view = View::Offline;
        ui.downloads.push(crate::downloads::Status {
            id: 7,
            total: 4,
            done: 1,
            title: "Album".into(),
            ..Default::default()
        });
        ui.page = Some(std::sync::Arc::new(crate::catalog::Page::default()));
        let scene = build(&ui, (1120, 700), false, false, "", false, 70);
        assert_eq!(
            scene
                .buttons
                .iter()
                .filter(|b| matches!(b.action, Action::Go(View::Offline)))
                .count(),
            1
        );
        assert!(scene
            .buttons
            .iter()
            .any(|b| matches!(b.action, Action::CancelDownload(7))));
        assert!(scene.labels.iter().any(|l| l.text.contains("1 / 4")));
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
            let buffered_row_limit = ((840 + SCROLL_OVERSCAN * 2) / 56 + 2) as usize;
            assert!(visible > 0 && visible <= buffered_row_limit);
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
    fn subpages_always_render_back_button_even_with_empty_history() {
        for view in [
            View::Album,
            View::Artist,
            View::Show,
            View::TrackInfo,
            View::PlaylistEdit,
            View::PlaylistPicker,
            View::CoverEdit,
            View::FlowTune,
            View::StyleMix,
            View::Storage,
            View::ArlHelp,
            View::CustomColors,
            View::Add,
        ] {
            let ui = Ui {
                view,
                ..Default::default()
            };
            assert!(ui.history.is_empty());
            assert!(ui.is_subpage());
            let scene = build(&ui, (1200, 800), true, false, "DeezNats", false, 50);
            assert!(
                scene
                    .buttons
                    .iter()
                    .any(|b| matches!(b.action, Action::Back)),
                "Back button missing for subpage {:?}",
                view
            );
        }

        let offline_collection_ui = Ui {
            view: View::Offline,
            page: Some(std::sync::Arc::new(crate::catalog::Page {
                entity: Some(crate::catalog::Entry {
                    kind: "offline_collection".into(),
                    id: "artist:123".into(),
                    ..Default::default()
                }),
                ..Default::default()
            })),
            ..Default::default()
        };
        assert!(offline_collection_ui.is_subpage());
        let scene = build(
            &offline_collection_ui,
            (1200, 800),
            true,
            false,
            "DeezNats",
            false,
            50,
        );
        assert!(scene
            .buttons
            .iter()
            .any(|b| matches!(b.action, Action::Back)));

        let search_ui = Ui {
            view: View::Explore,
            page: Some(std::sync::Arc::new(crate::catalog::Page {
                title: "Search results".into(),
                ..Default::default()
            })),
            ..Default::default()
        };
        assert!(search_ui.is_subpage());
        let scene = build(&search_ui, (1200, 800), true, false, "DeezNats", false, 50);
        assert!(scene
            .buttons
            .iter()
            .any(|b| matches!(b.action, Action::Back)));

        for view in [
            View::Home,
            View::Settings,
            View::Podcasts,
            View::Favorites,
            View::Albums,
            View::Playlists,
            View::Recent,
        ] {
            let ui = Ui {
                view,
                ..Default::default()
            };
            assert!(!ui.is_subpage());
            let scene = build(&ui, (1200, 800), true, false, "DeezNats", false, 50);
            assert!(
                !scene
                    .buttons
                    .iter()
                    .any(|b| matches!(b.action, Action::Back)),
                "Back button should not appear on root page {:?}",
                view
            );
        }
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
    fn queue_drag_handles_appear_only_on_upcoming_rows_for_each_provider() {
        for provider in [
            crate::catalog::Provider::Deezer,
            crate::catalog::Provider::Spotify,
            crate::catalog::Provider::YouTube,
        ] {
            let mut ui = Ui {
                provider,
                queue_sidebar_open: true,
                ..Default::default()
            };
            ui.queue = (0..3)
                .map(|i| crate::catalog::Entry {
                    id: i.to_string(),
                    kind: "track".into(),
                    ..Default::default()
                })
                .collect();
            ui.now = ui.queue[0].clone();
            let scene = build(&ui, (1200, 800), true, false, "", false, 50);
            assert_eq!(
                scene
                    .labels
                    .iter()
                    .filter(|label| label.text == "\u{22ee}\u{22ee}")
                    .count(),
                2
            );
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
        let row_y = |scene: &Scene| {
            scene
                .buttons
                .iter()
                .find(|b| matches!(b.action, Action::QueueJump(2)))
                .unwrap()
                .rect
                .y
        };
        ui.queue_scroll_offset = 1;
        let shifted = build(&ui, (1200, 800), true, false, "", false, 50);
        assert_eq!(row_y(&shifted), row_y(&first) - 1);
        ui.queue_scroll_offset = 0;
        ui.queue_scroll = first.queue_scroll_max;
        let last = build(&ui, (1200, 800), true, false, "", false, 50);
        assert!(last
            .buttons
            .iter()
            .any(|b| matches!(b.action, Action::QueueJump(99))));
        assert!(!last
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
    fn queue_anchor_places_current_row_near_quarter_of_viewport() {
        let mut ui = Ui {
            queue_sidebar_open: true,
            ..Default::default()
        };
        ui.queue = (0..40)
            .map(|i| crate::catalog::Entry {
                id: i.to_string(),
                title: format!("Track {i}"),
                kind: "track".into(),
                ..Default::default()
            })
            .collect();
        ui.queue_index = 20;
        ui.now = ui.queue[20].clone();
        let initial = build(&ui, (1200, 800), true, false, "", false, 50);
        ui.queue_scroll = initial.queue_anchor / 24;
        ui.queue_scroll_offset = initial.queue_anchor % 24;
        let scene = build(&ui, (1200, 800), true, false, "", false, 50);
        let viewport = scene.queue_viewport.unwrap();
        let current = scene
            .buttons
            .iter()
            .find(|b| matches!(b.action, Action::QueueJump(20)))
            .unwrap();
        assert_eq!(current.rect.y, viewport.y + viewport.h / 4);
    }
    #[test]
    fn queue_labels_are_identical_on_regular_and_lyrics_pages() {
        let mut ui = Ui {
            queue_sidebar_open: true,
            ..Default::default()
        };
        ui.now = crate::catalog::Entry {
            id: "track".into(),
            title: "Prismatic".into(),
            subtitle: "Svdden Death".into(),
            ..Default::default()
        };
        ui.queue = vec![ui.now.clone()];
        let regular = build(&ui, (1200, 800), true, false, "", true, 50);
        ui.view = View::Lyrics;
        let lyrics = build(&ui, (1200, 800), true, false, "", true, 50);
        let labels = |scene: &Scene| {
            scene.labels[scene.queue_labels.clone()]
                .iter()
                .map(|label| (label.text.clone(), label.kind, label.rect.w, label.rect.h))
                .collect::<Vec<_>>()
        };
        assert!(!regular.queue_labels.is_empty());
        assert_eq!(labels(&regular), labels(&lyrics));
        assert!(lyrics.labels[lyrics.queue_labels.clone()]
            .iter()
            .any(|label| label.text == "Prismatic"));
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
        assert!(scene.labels.iter().any(|l| l.text == "QUEUE"));
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
        assert!(scene2
            .buttons
            .iter()
            .any(|b| matches!(b.action, Action::QueueJump(0))));
        assert_eq!(ui.queue[0].id, "t1");
        assert_eq!(ui.queue_index, 1);
        assert!(scene2.labels.iter().any(|l| l.text == "QUEUE"));
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
    fn playlist_editor_layout_keeps_fields_and_save_separate() {
        let mut ui = Ui::default();
        ui.view = View::PlaylistEdit;
        for width in [700, 1200] {
            let scene = build(&ui, (width, 1000), true, false, "", false, 80);
            let name = scene.name.unwrap();
            let description = scene.description.unwrap();
            let save = scene
                .buttons
                .iter()
                .find(|button| matches!(button.action, Action::SavePlaylist))
                .unwrap();
            assert!(name.y + name.h <= description.y);
            assert!(description.y + description.h <= save.rect.y);
            assert!(scene
                .buttons
                .iter()
                .any(|button| matches!(button.action, Action::ChoosePlaylistCover)));
        }
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
    fn track_info_enriches_sparse_rows_and_keeps_controls_inside_header() {
        let mut ui = Ui::default();
        ui.view = View::TrackInfo;
        let track = crate::catalog::Entry {
            id: "1".into(),
            title: "Song".into(),
            kind: "track".into(),
            ..Default::default()
        };
        ui.track_info = Some((
            track,
            serde_json::json!({
                "id": "1", "title": "Song", "artist": {"id": "2", "name": "Artist"},
                "album": {"id": "3", "title": "Album", "cover_medium": "https://example.com/cover.jpg", "release_date": "2024-01-01"}
            }),
            serde_json::json!({"id": "2", "name": "Artist", "picture_medium": "https://example.com/artist.jpg"}),
        ));
        for width in [640, 1200] {
            let scene = build(&ui, (width, 800), true, false, "", false, 80);
            assert!(scene
                .pictures
                .iter()
                .any(|(_, url)| url == "https://example.com/cover.jpg"));
            assert!(scene
                .pictures
                .iter()
                .any(|(_, url)| url == "https://example.com/artist.jpg"));
            assert!(scene.labels.iter().any(|label| label.text == "1 Jan 2024"));
            let play = scene
                .buttons
                .iter()
                .find(|button| button.text == "Play track")
                .unwrap();
            let album = scene
                .buttons
                .iter()
                .find(|button| button.text == "View album")
                .unwrap();
            assert!(play.rect.x + play.rect.w <= album.rect.x);
            assert!(scene.content_buttons.contains(
                &scene
                    .buttons
                    .iter()
                    .position(|button| button.text == "Play track")
                    .unwrap()
            ));
        }
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
                        btn.rect.x,
                        btn.rect.w,
                        size.0,
                        tab
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
        assert!(album_btn.glyph.is_empty());
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
            .any(|b| matches!(b.action, Action::Setting("Background FPS"))));
        assert_eq!(
            options("Background FPS"),
            &["Off", "90 FPS", "60 FPS", "30 FPS"]
        );
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
            if idx == 2 || idx == 6 {
                assert!(!scene
                    .buttons
                    .iter()
                    .any(|b| matches!(b.action, Action::ToggleLyricSource(i) if i == idx)));
                continue;
            }
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
    fn home_pins_form_the_first_separate_shelf_and_stay_provider_specific() {
        use crate::catalog::{Entry, Page, Provider, Section};
        let entry = |id: &str| Entry {
            id: id.into(),
            title: id.into(),
            kind: "album".into(),
            ..Default::default()
        };
        let ui = Ui {
            view: View::Home,
            provider: Provider::Deezer,
            pinned: vec![
                (Provider::Deezer, entry("pin1")),
                (Provider::Spotify, entry("spotify:album:other")),
                (Provider::Deezer, entry("pin2")),
            ],
            page: Some(std::sync::Arc::new(Page {
                sections: vec![Section {
                    title: "Recommendations".into(),
                    entries: vec![entry("recommended")],
                    ..Default::default()
                }],
                ..Default::default()
            })),
            ..Default::default()
        };
        let scene = build(&ui, (1200, 1000), true, false, "", false, 50);
        let position = |id: &str| {
            scene
                .buttons
                .iter()
                .find_map(|button| match &button.action {
                    Action::OpenItem(entry) if entry.id == id && !button.text_link => {
                        Some(button.rect.y)
                    }
                    _ => None,
                })
        };
        assert!(scene.labels.iter().any(|label| label.text == "Pinned"));
        assert_eq!(position("pin1"), position("pin2"));
        assert!(position("pin1").unwrap() < position("recommended").unwrap());
        assert!(position("spotify:album:other").is_none());
        let restored: Vec<(Provider, Entry)> =
            serde_json::from_value(serde_json::to_value(&ui.pinned).unwrap()).unwrap();
        assert_eq!(restored.len(), 3);
        assert_eq!(restored[1].0, Provider::Spotify);
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

        // The library shows playlists without a duplicate saved podcasts section
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
        assert!(!s_page
            .labels
            .iter()
            .any(|l| l.text == "Saved podcasts" || l.text == "Kaydedilen podcastler"));

        // Saved podcasts remain accessible from the Podcasts view
        assert!(!s_page
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
        assert_eq!(fav_btn_unliked.accessible, "Add to library");
        assert!(!fav_btn_unliked.selected);

        // When saved with podcast namespace key
        show_ui.liked_ids.insert(podcast_entry.library_key());
        let s_liked = build(&show_ui, (1200, 800), false, false, "", false, 50);
        let fav_btn_liked = s_liked
            .buttons
            .iter()
            .find(|b| matches!(b.action, Action::Favorite))
            .unwrap();
        assert_eq!(fav_btn_liked.accessible, "In library");
        assert!(fav_btn_liked.selected);
    }
    #[test]
    fn reference_player_controls_and_metadata_never_collide_at_responsive_widths() {
        fn overlap(a: Rect, b: Rect) -> bool {
            a.x < b.x + b.w && a.x + a.w > b.x && a.y < b.y + b.h && a.y + a.h > b.y
        }
        for width in [
            480, 588, 600, 900, 1099, 1100, 1179, 1180, 1200, 1491, 1500, 2200,
        ] {
            for layout in [0, 1] {
                let mut ui = Ui::default();
                ui.player_layout = layout;
                ui.now = crate::catalog::Entry {
                    id: "song".into(),
                    kind: "track".into(),
                    title: "A long title with an explicit marker and a favorite control".into(),
                    subtitle: "An artist name".into(),
                    artwork: "fixture".into(),
                    raw: serde_json::json!({"explicit":true}),
                    ..Default::default()
                };
                let mut scene = Scene::default();
                build_player(&mut scene, &ui, width, 600, &ui.now.title, true, 70);
                let rects: Vec<_> = scene
                    .buttons
                    .iter()
                    .map(|b| b.rect)
                    .chain(scene.seek)
                    .chain(scene.volume)
                    .chain(scene.elapsed)
                    .chain(scene.remaining)
                    .chain(scene.player_title.as_ref().into_iter().flat_map(|t| {
                        std::iter::once(scene.labels[t.label].rect)
                            .chain(t.badge.map(|i| scene.labels[i].rect))
                    }))
                    .collect();
                for (index, rect) in rects.iter().enumerate() {
                    assert!(
                        rect.x >= 0
                            && rect.x + rect.w <= width
                            && rect.y >= 600 - player_height(&ui, width)
                            && rect.y + rect.h <= 600,
                        "{width}/{layout}: {rect:?}"
                    );
                    for other in &rects[index + 1..] {
                        assert!(
                            !overlap(*rect, *other),
                            "{width}/{layout}: {rect:?} overlaps {other:?}"
                        );
                    }
                }
                let meta = scene.player_title.as_ref().unwrap();
                assert_eq!(meta.centered, layout == 0 && width >= 1180);
                assert!(scene
                    .buttons
                    .iter()
                    .any(|b| matches!(b.action, Action::FavoriteNow)));
                assert!(scene
                    .buttons
                    .iter()
                    .any(|b| matches!(b.action, Action::MuteVolume)));
                assert!(scene
                    .buttons
                    .iter()
                    .all(|b| !matches!(b.action, Action::PlayerFullscreen)));
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
                if layout == 1 {
                    assert!(clock.x >= play.rect.x + play.rect.w);
                }
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
    fn appearance_theme_shelf_pages_and_tint_slider_matches_background() {
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
            let next = small
                .buttons
                .iter()
                .find(|b| matches!(b.action, Action::ThemeShelf(1)))
                .unwrap();
            assert!(next.enabled);
            assert!(!small
                .buttons
                .iter()
                .any(|b| matches!(b.action, Action::ThemeShelf(-1))));
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
            let mut seen = std::collections::HashSet::new();
            for offset in (0..SOLID_THEMES.len() + 2).step_by(count) {
                ui.theme_shelf_offset = offset;
                let page = build(&ui, (1200, 900), true, false, "", false, 50);
                assert_eq!(page.content_height, small.content_height);
                for button in &page.buttons {
                    match button.action {
                        Action::SolidTheme(index) => {
                            seen.insert(index);
                        }
                        Action::CustomTheme(light) => {
                            seen.insert(SOLID_THEMES.len() + usize::from(light));
                        }
                        _ => {}
                    }
                }
                assert_eq!(
                    page.buttons
                        .iter()
                        .any(|b| matches!(b.action, Action::ThemeShelf(-1))),
                    offset > 0
                );
                assert_eq!(
                    page.buttons
                        .iter()
                        .any(|b| matches!(b.action, Action::ThemeShelf(1))),
                    offset + count < SOLID_THEMES.len() + 2
                );
            }
            assert_eq!(seen.len(), SOLID_THEMES.len() + 2);
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
            assert!(choice.rect.w >= 54 && choice.rect.h == 94 && choice.text_centered);
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
    #[test]
    fn provider_settings_keep_deezer_and_offer_spotify_setup_without_overlap() {
        for width in [340, 1000] {
            for language in ["en", "tr", "az"] {
                let mut ui = Ui {
                    view: View::Settings,
                    tab: 1,
                    ..Default::default()
                };
                ui.set_language(language);
                for busy in [false, true] {
                    ui.spotify_setup_busy = busy;
                    let mut scene = Scene::default();
                    settings(&mut scene, &ui, 0, 150, width, false, false);
                    assert_eq!(
                        scene
                            .labels
                            .iter()
                            .any(|label| label.text == "Soloist audio device"),
                        ui.provider == crate::catalog::Provider::Spotify
                    );
                    assert!(scene.labels.iter().any(|label| label.text == "Deezer"));
                    if ui.provider == crate::catalog::Provider::Spotify {
                        let button = scene
                            .buttons
                            .iter()
                            .rev()
                            .find(|button| matches!(button.action, Action::SpotifySetup))
                            .unwrap();
                        assert_eq!(button.enabled, !busy);
                        assert!(button.rect.x >= 0 && button.rect.x + button.rect.w <= width);
                    }
                    assert!(scene
                        .buttons
                        .iter()
                        .any(|button| matches!(button.action, Action::Go(View::Account))));
                    assert!(scene.session.is_none());
                }
            }
        }
    }

    #[test]
    fn spotify_provider_action_tracks_startup_pairing_and_readiness() {
        use crate::spotify::service::Phase;
        let mut ui = Ui::default();
        ui.spotify_configured = true;
        assert!(matches!(
            spotify_provider_action(&ui),
            ("Start Spotify", Action::StartSpotify, true)
        ));
        for phase in [Phase::Starting, Phase::Connecting] {
            ui.spotify_phase = Some(phase);
            assert!(!spotify_provider_action(&ui).2);
        }
        ui.spotify_phase = Some(Phase::Pairing);
        assert!(matches!(
            spotify_provider_action(&ui),
            ("Open Spotify", Action::OpenSpotify, true)
        ));
        ui.spotify_phase = Some(Phase::Ready);
        assert!(matches!(
            spotify_provider_action(&ui),
            ("Reset", Action::SpotifySetup, true)
        ));
        ui.spotify_setup_busy = true;
        assert!(!spotify_provider_action(&ui).2);
    }
}

#[cfg(test)]
mod spotify_scene_tests {
    use super::*;

    #[test]
    fn catalog_sign_in_fits_narrow_cards_and_keeps_playback_setup_separate() {
        for width in [180, 280, 340, 700] {
            let mut ui = Ui {
                provider: crate::catalog::Provider::Spotify,
                ..Default::default()
            };
            let mut normal = Scene::default();
            spotify_catalog_card(&mut normal, &ui, 0, 0, width, false);
            assert!(
                normal.session.is_none(),
                "normal Spotify sign-in needs no developer configuration"
            );
            assert!(normal
                .buttons
                .iter()
                .any(|b| matches!(b.action, Action::SpotifyLogin)));
            assert!(normal
                .buttons
                .iter()
                .any(|b| matches!(b.action, Action::SpotifySetup)));
            for button in &normal.buttons {
                assert!(
                    button.rect.x >= 0
                        && button.rect.w > 0
                        && button.rect.x + button.rect.w <= width
                );
            }
            ui.spotify_catalog_busy = true;
            let mut busy = Scene::default();
            spotify_catalog_card(&mut busy, &ui, 0, 0, width, false);
            assert!(
                !busy
                    .buttons
                    .iter()
                    .find(|b| matches!(b.action, Action::SpotifyLogin))
                    .unwrap()
                    .enabled
            );
            let mut connected = Scene::default();
            spotify_catalog_card(&mut connected, &ui, 0, 0, width, true);
            assert!(connected.session.is_none());
            assert!(connected
                .buttons
                .iter()
                .any(|b| matches!(b.action, Action::SpotifyLogout)));
            assert!(!connected
                .buttons
                .iter()
                .any(|b| matches!(b.action, Action::SpotifyRefresh)));
        }
    }

    #[test]
    fn onboarding_hides_application_navigation_until_signed_in() {
        let ui = Ui {
            onboarding_open: true,
            ..Default::default()
        };
        let scene = build(&ui, (1200, 800), false, false, "", false, 50);
        assert!(scene
            .buttons
            .iter()
            .any(|b| matches!(b.action, Action::OnboardingNext)));
        assert!(!scene
            .buttons
            .iter()
            .any(|b| matches!(b.action, Action::Go(_))));
    }

    #[test]
    fn expired_provider_card_requests_login_without_refresh() {
        let mut ui = Ui::default();
        ui.provider_reauth[0] = true;
        let mut scene = Scene::default();
        provider_card(
            &mut scene,
            &ui,
            0,
            0,
            700,
            "Deezer",
            Action::DeezerLogo,
            None,
            true,
            Action::DeezerLogin,
            None,
            false,
        );
        assert!(scene
            .labels
            .iter()
            .any(|label| label.text == "Session expired. Sign in again."));
        assert!(!scene
            .buttons
            .iter()
            .any(|button| matches!(button.action, Action::ConnectSaved)));
    }

    #[test]
    fn spotify_reset_failure_requires_login_and_disables_reset() {
        let mut ui = Ui {
            provider: crate::catalog::Provider::Spotify,
            spotify_configured: true,
            spotify_reset_failed: true,
            ..Default::default()
        };
        let mut scene = Scene::default();
        spotify_catalog_card(&mut scene, &ui, 0, 0, 700, true);
        assert!(scene
            .buttons
            .iter()
            .any(|b| matches!(b.action, Action::SpotifyLogin)));
        let reset = scene
            .buttons
            .iter()
            .find(|b| matches!(b.action, Action::SpotifyRefresh))
            .unwrap();
        assert!(!reset.enabled);
        ui.spotify_reset_failed = false;
        let mut scene = Scene::default();
        spotify_catalog_card(&mut scene, &ui, 0, 0, 700, true);
        assert!(scene
            .buttons
            .iter()
            .any(|b| matches!(b.action, Action::SpotifyLogout)));
        assert!(scene
            .buttons
            .iter()
            .any(|b| matches!(b.action, Action::SpotifyRefresh) && b.enabled));
    }
    #[test]
    fn custom_theme_editor_is_a_modal_with_save_and_close() {
        let mut ui = Ui::default();
        ui.view = View::Settings;
        ui.tab = 2;
        ui.custom_theme_draft = Some((
            false,
            [("Text".to_string(), 0x123456)].into_iter().collect(),
        ));
        let scene = build(&ui, (1120, 700), false, false, "", false, 70);
        let (rect, _, first) = scene.auth_modal.expect("custom theme modal");
        assert!(rect.x >= 0 && rect.y >= 0 && rect.x + rect.w <= 1120 && rect.y + rect.h <= 700);
        let buttons = &scene.buttons[first..];
        assert!(buttons
            .iter()
            .any(|b| matches!(b.action, Action::SaveCustomTheme)));
        assert!(buttons
            .iter()
            .any(|b| matches!(b.action, Action::CloseCustomTheme)));
        assert!(buttons
            .iter()
            .any(|b| matches!(b.action, Action::Color("Text")) && b.text == "#563412"));
        assert_eq!(ui.view, View::Settings);
    }
    #[test]
    fn spotify_liked_songs_keep_table_and_play_without_download_affordances() {
        for provider in [
            crate::catalog::Provider::Deezer,
            crate::catalog::Provider::Spotify,
        ] {
            let mut ui = Ui {
                provider,
                view: View::Favorites,
                ..Default::default()
            };
            ui.page = Some(std::sync::Arc::new(crate::catalog::Page {
                title: view_title(&ui, View::Favorites).into(),
                rows: vec![crate::catalog::Entry {
                    id: "song".into(),
                    title: "A real song".into(),
                    kind: "track".into(),
                    duration: 123_000,
                    ..Default::default()
                }],
                ..Default::default()
            }));
            let scene = build(&ui, (1100, 800), true, false, "", false, 50);
            assert!(scene.buttons.iter().any(
                |b| matches!(&b.action, Action::OpenItem(item) if item.title == "A real song")
            ));
            assert!(scene
                .buttons
                .iter()
                .any(|b| matches!(b.action, Action::PlayAll)));
            assert_eq!(
                scene
                    .buttons
                    .iter()
                    .any(|b| matches!(b.action, Action::Download)),
                false
            );
            assert_eq!(
                scene
                    .buttons
                    .iter()
                    .any(|b| matches!(b.action, Action::Go(View::Offline))),
                supports_downloads(&ui)
            );
            assert!(!scene
                .buttons
                .iter()
                .any(|b| matches!(b.action, Action::SwitchProvider(_))));
        }
    }

    #[test]
    fn track_metadata_stacks_at_narrow_widths_and_only_shows_measured_values() {
        for width in [260, 340, 588, 1000] {
            let mut ui = Ui {
                provider: crate::catalog::Provider::Spotify,
                ..Default::default()
            };
            ui.track_info = Some((
                crate::catalog::Entry {
                    id: "spotify:track:1111111111111111111111".into(),
                    kind: "track".into(),
                    title: "A long track title that must fit".into(),
                    subtitle: "Artist".into(),
                    artist_id: "spotify:artist:2222222222222222222222".into(),
                    album_id: "spotify:album:3333333333333333333333".into(),
                    album: "Album".into(),
                    ..Default::default()
                },
                serde_json::json!({"playCount":12345}),
                serde_json::Value::Null,
            ));
            let mut scene = Scene::default();
            build_track_info(&mut scene, &ui, 0, 0, width);
            for label in &scene.labels {
                assert!(
                    label.rect.x >= 0 && label.rect.w > 0 && label.rect.x + label.rect.w <= width,
                    "label geometry at {width}"
                );
            }
            for button in &scene.buttons {
                assert!(
                    button.rect.x >= 0
                        && button.rect.w > 0
                        && button.rect.x + button.rect.w <= width,
                    "button geometry at {width}"
                );
            }
            for (picture, _) in &scene.pictures {
                assert!(picture.x >= 0 && picture.w > 0 && picture.x + picture.w <= width);
            }
            let panels: Vec<_> = scene
                .panels
                .iter()
                .filter(|panel| panel.kind == 8)
                .collect();
            for (index, panel) in panels.iter().enumerate() {
                for other in panels.iter().skip(index + 1) {
                    let a = panel.rect;
                    let b = other.rect;
                    assert!(
                        a.x + a.w <= b.x
                            || b.x + b.w <= a.x
                            || a.y + a.h <= b.y
                            || b.y + b.h <= a.y,
                        "overlapping panels at {width}"
                    );
                }
            }
            assert!(scene.labels.iter().any(|label| label.text == "Plays"));
            assert!(scene.labels.iter().any(|label| label.text == "12,345"));
            for fake in ["2024", "1", "DEEZNATS", "88%", "120"] {
                assert!(!scene.labels.iter().any(|label| label.text == fake));
            }
        }
    }
    #[test]
    fn spotify_artist_audience_is_optional_and_does_not_invent_counts() {
        let ui = Ui {
            provider: crate::catalog::Provider::Spotify,
            ..Default::default()
        };
        let artist = crate::catalog::Entry {
            kind: "artist".into(),
            ..Default::default()
        };
        assert!(entity_caption(&artist, None, "", &ui).is_empty());
        let artist = crate::catalog::Entry {
            raw: serde_json::json!({"followers":{"total":1234},"monthly_listeners":5678}),
            ..artist
        };
        let caption = entity_caption(&artist, None, "", &ui);
        assert!(caption.contains("followers") && caption.contains("monthly listeners"));
        assert!(!artist_audience(&serde_json::json!({"followers":{"total":0}}), &ui).is_empty());
        let mut scene = Scene::default();
        build_track_info(&mut scene, &ui, 0, 0, 900);
        assert!(!scene.labels.iter().any(|l| l.text.contains("12,400")
            || l.text.contains("8 releases")
            || l.text.contains("FLAC")));
    }

    #[test]
    fn spotify_playback_settings_do_not_offer_unverified_pcm_controls() {
        let ui = Ui {
            provider: crate::catalog::Provider::Spotify,
            view: View::Settings,
            tab: 4,
            ..Default::default()
        };
        let mut scene = Scene::default();
        settings(&mut scene, &ui, 0, 100, 340, true, false);
        assert!(scene
            .buttons
            .iter()
            .any(|b| matches!(b.action, Action::ReconnectSpotifyOutput)));
        assert!(scene
            .buttons
            .iter()
            .any(|b| matches!(b.action, Action::OpenSpotifyOutputSettings)));
        assert!(scene.eq_graph.is_none() && scene.eq_sliders.is_none());
        assert!(!scene.buttons.iter().any(|b| matches!(
            b.action,
            Action::StreamingQuality(_) | Action::EqReset | Action::Setting("Audio output device")
        )));
    }
}

pub(crate) mod drawing {
    //! The original application's GDI+ paths, text and glass gradients, rendered
    //! directly into one premultiplied frame instead of independently painted HWNDs.
    use std::{
        cell::Cell,
        mem::{size_of, zeroed},
        ptr::{null, null_mut},
        sync::OnceLock,
    };
    pub const LYRICS_UNTRIMMED: u32 = 0x80000000;

    use windows_sys::Win32::{
        Foundation::RECT,
        Graphics::{Gdi::*, GdiPlus::*},
    };
    // Rounding is baked into the cached pixels rather than clipped every frame.
    type ImageKey = (u64, i32, i32, u32);
    mod composite {
        //! Premultiplied source-over blending for cached UI rasters.

        #[inline]
        fn scalar_pixel(destination: u32, source: u32) -> u32 {
            let inverse = 255 - (source >> 24);
            let mut result = 0;
            for shift in [0, 8, 16, 24] {
                let product = ((destination >> shift) & 255) * inverse + 128;
                let background = (product + (product >> 8)) >> 8;
                result |= ((((source >> shift) & 255) + background).min(255)) << shift;
            }
            result
        }

        pub(super) fn blend_row(destination: &mut [u32], source: &[u32]) {
            assert_eq!(destination.len(), source.len());
            let mut index = 0;
            // SSE2 is part of the x86-64 baseline, with no runtime CPU requirement.
            #[cfg(target_arch = "x86_64")]
            unsafe {
                use std::arch::x86_64::*;
                let zero = _mm_setzero_si128();
                let full = _mm_set1_epi16(255);
                let rounding = _mm_set1_epi16(128);
                while index + 4 <= destination.len() {
                    let foreground = _mm_loadu_si128(source.as_ptr().add(index).cast());
                    let alpha = _mm_srli_epi32::<24>(foreground);
                    if _mm_movemask_epi8(_mm_cmpeq_epi32(alpha, zero)) == 0xffff {
                        index += 4;
                        continue;
                    }
                    if _mm_movemask_epi8(_mm_cmpeq_epi32(alpha, _mm_set1_epi32(255))) == 0xffff {
                        _mm_storeu_si128(destination.as_mut_ptr().add(index).cast(), foreground);
                        index += 4;
                        continue;
                    }
                    let background = _mm_loadu_si128(destination.as_ptr().add(index).cast());
                    let blend = |foreground, background| {
                        let alpha =
                            _mm_shufflehi_epi16::<255>(_mm_shufflelo_epi16::<255>(foreground));
                        let inverse = _mm_sub_epi16(full, alpha);
                        let product = _mm_add_epi16(_mm_mullo_epi16(background, inverse), rounding);
                        let scaled = _mm_srli_epi16::<8>(_mm_add_epi16(
                            product,
                            _mm_srli_epi16::<8>(product),
                        ));
                        _mm_add_epi16(foreground, scaled)
                    };
                    let low = blend(
                        _mm_unpacklo_epi8(foreground, zero),
                        _mm_unpacklo_epi8(background, zero),
                    );
                    let high = blend(
                        _mm_unpackhi_epi8(foreground, zero),
                        _mm_unpackhi_epi8(background, zero),
                    );
                    _mm_storeu_si128(
                        destination.as_mut_ptr().add(index).cast(),
                        _mm_packus_epi16(low, high),
                    );
                    index += 4;
                }
            }
            for (destination, source) in destination[index..].iter_mut().zip(&source[index..]) {
                *destination = scalar_pixel(*destination, *source);
            }
        }

        #[cfg(test)]
        mod tests {
            use super::*;

            #[test]
            fn uniform_blocks_mixed_blocks_and_unaligned_tails_match_scalar() {
                let source = [
                    0x12345678, 0, 0, 0, 0, 0xff123456, 0xffabcdef, 0xff000000, 0xffffffff, 0,
                    0x80402010, 0xff345678, 0x01010101, 0, 0xffabcdef, 0x40201008, 0,
                ];
                let original: Vec<u32> = (0..source.len())
                    .map(|i| 0x80402010u32.wrapping_add(i as u32 * 0x010101))
                    .collect();
                for len in 1..source.len() {
                    let mut actual = original.clone();
                    blend_row(&mut actual[1..1 + len], &source[1..1 + len]);
                    for i in 0..source.len() {
                        let expected = if (1..1 + len).contains(&i) {
                            scalar_pixel(original[i], source[i])
                        } else {
                            original[i]
                        };
                        assert_eq!(actual[i], expected, "length {len}, pixel {i}");
                    }
                }
            }

            #[test]
            fn vector_blending_matches_source_over_for_every_alpha_and_unaligned_tail() {
                let mut source = vec![0; 1031];
                let mut original = vec![0; source.len()];
                for (index, (source, destination)) in
                    source.iter_mut().zip(&mut original).enumerate()
                {
                    let alpha = index as u32 % 256;
                    let destination_alpha = (index as u32 * 71) % 256;
                    *source = (alpha << 24) | ((alpha / 2) << 16) | ((alpha / 3) << 8) | alpha;
                    *destination = (destination_alpha << 24)
                        | (destination_alpha << 16)
                        | ((destination_alpha / 5) << 8)
                        | (destination_alpha / 2);
                }
                let expected: Vec<_> = original
                    .iter()
                    .zip(&source)
                    .map(|(&destination, &source)| scalar_pixel(destination, source))
                    .collect();
                let mut actual = original.clone();
                blend_row(&mut actual[1..], &source[1..]);
                assert_eq!(&actual[1..], &expected[1..]);
                assert_eq!(actual[0], original[0]);
                assert_eq!(scalar_pixel(0x80402010, 0), 0x80402010);
                assert_eq!(scalar_pixel(0x80402010, 0xffabcdef), 0xffabcdef);
            }
        }
    }
    pub(crate) mod gpu {
        use std::result::Result::Ok;
        use windows_numerics::{Matrix3x2, Vector2};
        // Direct3D rasterization and alpha-preserving DirectComposition presentation.
        use std::{
            cell::RefCell,
            collections::VecDeque,
            ffi::c_void,
            time::{Duration, Instant},
        };
        use windows::{
            core::Interface,
            Win32::{
                Foundation::{HMODULE, HWND},
                Graphics::{
                    Direct2D::{Common::*, *},
                    Direct3D::*,
                    Direct3D11::*,
                    DirectComposition::*,
                    Dxgi::{Common::*, *},
                },
            },
        };
        use windows_sys::Win32::{Foundation::RECT, Graphics::GdiPlus::*};
        type Result<T> = windows::core::Result<T>;
        pub(crate) const UPLOAD_WAKE: u32 = 0x8000 + 11;
        mod retained {
            //! Retain independently clipped GPU paint lists across scroll presentation frames.
            use super::*;
            use windows_sys::Win32::Graphics::Gdi::*;
            #[derive(Clone, Copy, PartialEq, Eq, Default)]
            pub(crate) enum Layer {
                #[default]
                Direct,
                Main,
                Queue,
                Overlay,
            }
            #[derive(Clone, Copy)]
            pub(crate) struct LayerRequest {
                pub viewport: RECT,
                pub layout_offset: i32,
                pub shown_offset: f64,
                pub overscan: i32,
            }
            struct Saved {
                list: ID2D1CommandList,
                origin: i32,
                viewport: RECT,
            }
            #[derive(Default)]
            struct Slot {
                saved: Option<Saved>,
                recording: Option<ID2D1CommandList>,
                request: Option<LayerRequest>,
                reuse: bool,
            }
            #[derive(Default)]
            pub(super) struct Retention {
                main: Slot,
                queue: Slot,
                overlay: Option<ID2D1CommandList>,
                current: Layer,
                visual_key: u64,
            }
            impl Slot {
                unsafe fn configure(
                    &mut self,
                    context: &ID2D1DeviceContext,
                    request: Option<LayerRequest>,
                ) -> Result<()> {
                    self.request = request;
                    self.recording = None;
                    self.reuse = false;
                    let Some(request) = request else {
                        self.saved = None;
                        return Ok(());
                    };
                    self.reuse = self.saved.as_ref().is_some_and(|s| {
                        s.viewport.left == request.viewport.left
                            && s.viewport.top == request.viewport.top
                            && s.viewport.right == request.viewport.right
                            && s.viewport.bottom == request.viewport.bottom
                            && (request.shown_offset - s.origin as f64).abs()
                                < (request.overscan - 16).max(1) as f64
                    });
                    if !self.reuse {
                        self.saved = None;
                        self.recording = Some(context.CreateCommandList()?);
                    }
                    Ok(())
                }
                fn recording_bounds(&self) -> Option<RECT> {
                    self.recording.as_ref()?;
                    let request = self.request?;
                    Some(RECT {
                        top: request.viewport.top - request.overscan,
                        bottom: request.viewport.bottom + request.overscan,
                        ..request.viewport
                    })
                }
                unsafe fn finish(&mut self) -> Result<()> {
                    if let Some(list) = self.recording.take() {
                        list.Close()?;
                        let request = self.request.unwrap();
                        self.saved = Some(Saved {
                            list,
                            origin: request.layout_offset,
                            viewport: request.viewport,
                        });
                    }
                    Ok(())
                }
            }
            impl Retention {
                pub(super) fn layer_mask(&self) -> u8 {
                    match self.current {
                        Layer::Main => 1,
                        Layer::Queue => 2,
                        _ => 0,
                    }
                }
                pub(super) fn invalidate_deferred(&mut self, mask: u8) {
                    if mask & 1 != 0 {
                        self.main.saved = None;
                    }
                    if mask & 2 != 0 {
                        self.queue.saved = None;
                    }
                }
                pub(super) fn recording_bounds(&self) -> Option<RECT> {
                    match self.current {
                        Layer::Main => self.main.recording_bounds(),
                        Layer::Queue => self.queue.recording_bounds(),
                        _ => None,
                    }
                }
                pub(super) fn reusing(&self) -> bool {
                    match self.current {
                        Layer::Main => self.main.reuse,
                        Layer::Queue => self.queue.reuse,
                        _ => false,
                    }
                }
            }
            pub(crate) fn invalidate_layers() {
                let _ = SURFACE.try_with(|s| {
                    if let Some(s) = s.borrow_mut().as_mut() {
                        s.retained.main.saved = None;
                        s.retained.queue.saved = None;
                    }
                });
            }
            pub(crate) fn invalidate_queue_layer() {
                let _ = SURFACE.try_with(|s| {
                    if let Some(s) = s.borrow_mut().as_mut() {
                        s.retained.queue.saved = None;
                    }
                });
            }
            pub(crate) fn invalidate_main_layer() {
                let _ = SURFACE.try_with(|s| {
                    if let Some(s) = s.borrow_mut().as_mut() {
                        s.retained.main.saved = None;
                    }
                });
            }
            pub(crate) unsafe fn configure_layers(
                main: Option<LayerRequest>,
                queue: Option<LayerRequest>,
                visual_key: u64,
            ) -> bool {
                SURFACE.with(|slot| {
                    let mut slot = slot.borrow_mut();
                    let Some(s) = slot.as_mut().filter(|s| s.active) else {
                        return false;
                    };
                    if s.retained.visual_key != visual_key {
                        s.retained.main.saved = None;
                        s.retained.queue.saved = None;
                        s.retained.visual_key = visual_key;
                    }
                    s.retained.current = Layer::Direct;
                    s.retained.overlay = None;
                    let result = s
                        .retained
                        .main
                        .configure(&s.context, main)
                        .and_then(|_| s.retained.queue.configure(&s.context, queue));
                    if result.is_err() {
                        s.failed = true;
                    }
                    s.retained.main.recording.is_some() || s.retained.queue.recording.is_some()
                })
            }
            pub(crate) fn is_recording(layer: Layer) -> bool {
                SURFACE.with(|s| {
                    s.borrow().as_ref().is_some_and(|s| match layer {
                        Layer::Main => s.retained.main.recording.is_some(),
                        Layer::Queue => s.retained.queue.recording.is_some(),
                        _ => false,
                    })
                })
            }
            pub(crate) unsafe fn select_layer(layer: Layer) -> bool {
                SURFACE.with(|slot| {
                    let mut slot = slot.borrow_mut();
                    let Some(s) = slot.as_mut().filter(|s| s.active) else {
                        return true;
                    };
                    if s.retained.current != layer {
                        s.retained.current = layer;
                        match layer {
                            Layer::Main => {
                                if let Some(list) = &s.retained.main.recording {
                                    s.context.SetTarget(list);
                                } else {
                                    s.context.SetTarget(&s.backing);
                                }
                            }
                            Layer::Queue => {
                                if let Some(list) = &s.retained.queue.recording {
                                    s.context.SetTarget(list);
                                } else {
                                    s.context.SetTarget(&s.backing);
                                }
                            }
                            Layer::Overlay
                                if s.retained.main.request.is_some()
                                    || s.retained.queue.request.is_some() =>
                            {
                                if s.retained.overlay.is_none() {
                                    match s.context.CreateCommandList() {
                                        Ok(list) => s.retained.overlay = Some(list),
                                        Err(_) => s.failed = true,
                                    }
                                }
                                if let Some(list) = &s.retained.overlay {
                                    s.context.SetTarget(list);
                                }
                            }
                            _ => s.context.SetTarget(&s.backing),
                        }
                    }
                    !s.retained.reusing()
                })
            }
            pub(crate) fn expanded_clip(mut rect: RECT, layer: Layer) -> RECT {
                let margin = SURFACE.with(|s| {
                    s.borrow()
                        .as_ref()
                        .and_then(|s| match layer {
                            Layer::Main => s
                                .retained
                                .main
                                .recording
                                .as_ref()
                                .and(s.retained.main.request),
                            Layer::Queue => s
                                .retained
                                .queue
                                .recording
                                .as_ref()
                                .and(s.retained.queue.request),
                            _ => None,
                        })
                        .map_or(0, |r| r.overscan)
                });
                rect.top -= margin;
                rect.bottom += margin;
                rect
            }
            impl Surface {
                pub(super) unsafe fn composite_layers(&mut self) -> Result<()> {
                    self.context.SetTarget(&self.backing);
                    self.retained.main.finish()?;
                    self.retained.queue.finish()?;
                    if let Some(overlay) = &self.retained.overlay {
                        overlay.Close()?;
                    }
                    for slot in [&self.retained.main, &self.retained.queue] {
                        let (Some(saved), Some(request)) = (&slot.saved, slot.request) else {
                            continue;
                        };
                        for damage in &self.damage {
                            let clip = D2D_RECT_F {
                                left: request.viewport.left.max(damage.left) as f32,
                                top: request.viewport.top.max(damage.top) as f32,
                                right: request.viewport.right.min(damage.right) as f32,
                                bottom: request.viewport.bottom.min(damage.bottom) as f32,
                            };
                            if clip.left >= clip.right || clip.top >= clip.bottom {
                                continue;
                            }
                            self.context.SetTransform(&identity());
                            self.context
                                .PushAxisAlignedClip(&clip, D2D1_ANTIALIAS_MODE_ALIASED);
                            let mut transform = identity();
                            transform.M32 = (saved.origin as f64 - request.shown_offset) as f32;
                            self.context.SetTransform(&transform);
                            self.context.DrawImage(
                                &saved.list,
                                None,
                                None,
                                D2D1_INTERPOLATION_MODE_LINEAR,
                                D2D1_COMPOSITE_MODE_SOURCE_OVER,
                            );
                            self.context.PopAxisAlignedClip();
                        }
                    }
                    if let Some(overlay) = self.retained.overlay.take() {
                        self.context.SetTransform(&identity());
                        self.context.DrawImage(
                            &overlay,
                            None,
                            None,
                            D2D1_INTERPOLATION_MODE_LINEAR,
                            D2D1_COMPOSITE_MODE_SOURCE_OVER,
                        );
                    }
                    self.context.SetTransform(&identity());
                    self.retained.current = Layer::Direct;
                    Ok(())
                }
            }

            /// A small monochrome measurement canvas lets GDI clip queries see overscan.
            /// All actual pixels are generated by Direct2D; the native paint bitmap stays intact.
            pub(crate) struct RetainedCanvas {
                dc: HDC,
                bitmap: HBITMAP,
                previous: HGDIOBJ,
            }
            impl RetainedCanvas {
                pub(crate) unsafe fn new(width: i32, height: i32, margin: i32) -> Option<Self> {
                    let dc = CreateCompatibleDC(std::ptr::null_mut());
                    if dc.is_null() {
                        return None;
                    }
                    let bitmap = CreateBitmap(width, height + 2 * margin, 1, 1, std::ptr::null());
                    if bitmap.is_null() {
                        DeleteDC(dc);
                        return None;
                    }
                    let previous = SelectObject(dc, bitmap);
                    SetWindowOrgEx(dc, 0, -margin, std::ptr::null_mut());
                    Some(Self {
                        dc,
                        bitmap,
                        previous,
                    })
                }
                pub(crate) fn dc(&self) -> HDC {
                    self.dc
                }
            }
            impl Drop for RetainedCanvas {
                fn drop(&mut self) {
                    unsafe {
                        SelectObject(self.dc, self.previous);
                        DeleteObject(self.bitmap);
                        DeleteDC(self.dc);
                    }
                }
            }

            #[cfg(test)]
            mod tests {
                use super::*;
                use windows_sys::Win32::UI::WindowsAndMessaging::*;
                #[test]
                #[ignore = "Requires Windows hardware Direct3D and DirectComposition"]
                fn gpu_retained_layer_replays_overscan_without_submitting_elements() {
                    unsafe {
                        let hwnd = CreateWindowExW(
                            0,
                            windows::core::w!("STATIC").as_ptr(),
                            std::ptr::null(),
                            WS_POPUP,
                            0,
                            0,
                            128,
                            128,
                            std::ptr::null_mut(),
                            std::ptr::null_mut(),
                            std::ptr::null_mut(),
                            std::ptr::null(),
                        );
                        assert!(!hwnd.is_null());
                        let mut pixels = vec![0u32; 128 * 128];
                        assert!(begin(hwnd, std::ptr::null(), 128, 128, 128, 0, None));
                        background(None, 0, 0, 0, 100, 0, None, Some(0xff111111));
                        let viewport = RECT {
                            left: 10,
                            top: 10,
                            right: 118,
                            bottom: 100,
                        };
                        let queue = Some(LayerRequest {
                            viewport: RECT {
                                left: 90,
                                right: 118,
                                ..viewport
                            },
                            layout_offset: 0,
                            shown_offset: 0.,
                            overscan: 96,
                        });
                        assert!(configure_layers(
                            Some(LayerRequest {
                                viewport,
                                layout_offset: 0,
                                shown_offset: 0.,
                                overscan: 96
                            }),
                            queue,
                            1
                        ));
                        let canvas = RetainedCanvas::new(128, 128, 96).unwrap();
                        let frame = crate::drawing::Frame::begin(
                            canvas.dc(),
                            pixels.as_mut_ptr().cast(),
                            128,
                            128,
                            128,
                            false,
                        )
                        .unwrap();
                        assert!(select_layer(Layer::Main));
                        crate::drawing::fill(
                            canvas.dc(),
                            RECT {
                                left: 20,
                                top: 140,
                                right: 40,
                                bottom: 160,
                            },
                            0xffff0000,
                            0.,
                        );
                        assert!(select_layer(Layer::Queue));
                        crate::drawing::fill(
                            canvas.dc(),
                            RECT {
                                left: 94,
                                top: 20,
                                right: 110,
                                bottom: 40,
                            },
                            0xff00ff00,
                            0.,
                        );
                        select_layer(Layer::Direct);
                        drop(frame);
                        assert!(finish());
                        assert!(begin(hwnd, std::ptr::null(), 128, 128, 128, 0, None));
                        background(None, 0, 0, 0, 100, 0, None, Some(0xff111111));
                        assert!(!configure_layers(
                            Some(LayerRequest {
                                viewport,
                                layout_offset: 70,
                                shown_offset: 70.,
                                overscan: 96
                            }),
                            queue,
                            1
                        ));
                        assert!(
                            !select_layer(Layer::Main),
                            "replay must bypass element submission"
                        );
                        select_layer(Layer::Direct);
                        assert!(finish());
                        assert_eq!(
                            snapshot(RECT {
                                left: 25,
                                top: 75,
                                right: 26,
                                bottom: 76
                            })
                            .unwrap()[0],
                            0xffff0000,
                            "offscreen content must survive until it scrolls into view"
                        );
                        assert_eq!(
                            snapshot(RECT {
                                left: 25,
                                top: 5,
                                right: 26,
                                bottom: 6
                            })
                            .unwrap()[0],
                            0xff111111,
                            "presentation must stay clipped to its own viewport"
                        );
                        SURFACE.with(|slot| {
                            slot.borrow_mut()
                                .as_mut()
                                .unwrap()
                                .retained
                                .invalidate_deferred(1)
                        });
                        assert!(begin(hwnd, std::ptr::null(), 128, 128, 128, 0, None));
                        assert!(configure_layers(
                            Some(LayerRequest {
                                viewport,
                                layout_offset: 70,
                                shown_offset: 70.,
                                overscan: 96
                            }),
                            queue,
                            1
                        ));
                        select_layer(Layer::Main);
                        assert!(
                            !select_layer(Layer::Queue),
                            "a deferred main upload must preserve the queue paint cache"
                        );
                        select_layer(Layer::Direct);
                        assert!(finish());
                        hide();
                        DestroyWindow(hwnd);
                    }
                }
            }
        }
        pub(crate) use retained::{
            configure_layers, expanded_clip, invalidate_layers, invalidate_main_layer,
            invalidate_queue_layer, is_recording, select_layer, Layer, LayerRequest,
            RetainedCanvas,
        };
        pub(super) unsafe fn region_rects(
            region: windows_sys::Win32::Graphics::Gdi::HRGN,
        ) -> Option<Vec<RECT>> {
            use windows_sys::Win32::Graphics::Gdi::{GetRegionData, RGNDATA, RGNDATAHEADER};
            let bytes = GetRegionData(region, 0, std::ptr::null_mut());
            if bytes < std::mem::size_of::<RGNDATAHEADER>() as u32 {
                return None;
            }
            let mut buffer = vec![0usize; (bytes as usize).div_ceil(std::mem::size_of::<usize>())];
            let data = buffer.as_mut_ptr().cast::<RGNDATA>();
            if GetRegionData(region, bytes, data) != bytes {
                return None;
            }
            let count = (*data).rdh.nCount as usize;
            if count
                > (bytes as usize - std::mem::size_of::<RGNDATAHEADER>())
                    / std::mem::size_of::<RECT>()
            {
                return None;
            }
            Some(
                std::slice::from_raw_parts(
                    buffer
                        .as_ptr()
                        .cast::<u8>()
                        .add(std::mem::size_of::<RGNDATAHEADER>())
                        .cast::<RECT>(),
                    count,
                )
                .to_vec(),
            )
        }
        pub(super) struct Surface {
            hwnd: HWND,
            width: i32,
            height: i32,
            factory: ID2D1Factory1,
            context: ID2D1DeviceContext,
            backing: ID2D1Bitmap1,
            output: ID2D1Bitmap1,
            swap: IDXGISwapChain1,
            composition: IDCompositionDevice,
            _target: IDCompositionTarget,
            _visual: IDCompositionVisual,
            base: ID2D1Bitmap,
            images: VecDeque<(u64, ID2D1Bitmap, usize)>,
            active: bool,
            failed: bool,
            geometries: VecDeque<(usize, ID2D1PathGeometry, usize)>,
            damage: Vec<RECT>,
            brush: ID2D1SolidColorBrush,
            stroke_style: ID2D1StrokeStyle,
            image_brush: ID2D1BitmapBrush,
            gdi_matrix: *mut Matrix,
            text_rasters: VecDeque<((usize, i32, i32), ID2D1Bitmap1, usize)>,
            current_clip: Option<D2D_RECT_F>,
            retained: retained::Retention,
            uploads: usize,
            upload_bytes: usize,
            deferred_uploads: bool,
            deferred_layers: u8,
            wake_posted: bool,
        }
        impl Drop for Surface {
            fn drop(&mut self) {
                unsafe {
                    if !self.gdi_matrix.is_null() {
                        GdipDeleteMatrix(self.gdi_matrix);
                    }
                    if self.active {
                        let _ = self.context.EndDraw(None, None);
                    }
                    let _ = self._target.SetRoot(None::<&IDCompositionVisual>);
                    let _ = self.composition.Commit();
                }
            }
        }
        thread_local! {
            static SURFACE: RefCell<Option<Surface>> = const { RefCell::new(None) };
            static RETRY: RefCell<Option<Instant>> = const { RefCell::new(None) };
        }
        pub(crate) fn hide() {
            SURFACE.with(|s| {
                s.borrow_mut().take();
            });
        }
        pub(super) fn active() -> bool {
            SURFACE.with(|s| s.borrow().as_ref().is_some_and(|s| s.active))
        }
        pub(crate) fn available() -> bool {
            SURFACE.with(|s| s.borrow().is_some())
        }
        fn retry_later() {
            RETRY.with(|r| *r.borrow_mut() = Some(Instant::now() + Duration::from_secs(5)));
        }
        fn identity() -> Matrix3x2 {
            Matrix3x2 {
                M11: 1.,
                M22: 1.,
                ..Default::default()
            }
        }

        unsafe fn create(hwnd: HWND, width: i32, height: i32, caption: i32) -> Result<Surface> {
            let mut device = None;
            D3D11CreateDevice(
                None,
                D3D_DRIVER_TYPE_HARDWARE,
                HMODULE::default(),
                D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                None,
                D3D11_SDK_VERSION,
                Some(&mut device),
                None,
                None,
            )?;
            let device = device.unwrap();
            let dxgi: IDXGIDevice = device.cast()?;
            let factory: ID2D1Factory1 =
                D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)?;
            let context = factory
                .CreateDevice(&dxgi)?
                .CreateDeviceContext(D2D1_DEVICE_CONTEXT_OPTIONS_NONE)?;
            let dxgi_factory: IDXGIFactory2 = CreateDXGIFactory2(DXGI_CREATE_FACTORY_FLAGS(0))?;
            let swap = dxgi_factory.CreateSwapChainForComposition(
                &device,
                &DXGI_SWAP_CHAIN_DESC1 {
                    Width: width as u32,
                    Height: height as u32,
                    Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                    SampleDesc: DXGI_SAMPLE_DESC {
                        Count: 1,
                        Quality: 0,
                    },
                    BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
                    BufferCount: 2,
                    Scaling: DXGI_SCALING_STRETCH,
                    SwapEffect: DXGI_SWAP_EFFECT_FLIP_SEQUENTIAL,
                    AlphaMode: DXGI_ALPHA_MODE_PREMULTIPLIED,
                    ..Default::default()
                },
                None,
            )?;
            let buffer: IDXGISurface = swap.GetBuffer(0)?;
            let props = D2D1_BITMAP_PROPERTIES1 {
                pixelFormat: D2D1_PIXEL_FORMAT {
                    format: DXGI_FORMAT_B8G8R8A8_UNORM,
                    alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
                },
                dpiX: 96.,
                dpiY: 96.,
                bitmapOptions: D2D1_BITMAP_OPTIONS_TARGET,
                ..Default::default()
            };
            let output_props = D2D1_BITMAP_PROPERTIES1 {
                bitmapOptions: D2D1_BITMAP_OPTIONS_TARGET | D2D1_BITMAP_OPTIONS_CANNOT_DRAW,
                ..props.clone()
            };
            let output = context.CreateBitmapFromDxgiSurface(&buffer, Some(&output_props))?;
            let size = D2D_SIZE_U {
                width: width as u32,
                height: height as u32,
            };
            let backing = context.CreateBitmap(size, None, 0, &props)?;
            let base = ID2D1RenderTarget::CreateBitmap(
                &context,
                size,
                None,
                0,
                &D2D1_BITMAP_PROPERTIES {
                    pixelFormat: props.pixelFormat,
                    dpiX: 96.,
                    dpiY: 96.,
                },
            )?;
            let composition: IDCompositionDevice = DCompositionCreateDevice(&dxgi)?;
            let target = composition.CreateTargetForHwnd(hwnd, false)?;
            let visual = composition.CreateVisual()?;
            visual.SetContent(&swap)?;
            visual.SetOffsetY2(caption as f32)?;
            target.SetRoot(&visual)?;
            composition.Commit()?;
            let brush = context.CreateSolidColorBrush(&D2D1_COLOR_F::default(), None)?;
            let stroke_style = ID2D1Factory::CreateStrokeStyle(
                &factory,
                &D2D1_STROKE_STYLE_PROPERTIES {
                    startCap: D2D1_CAP_STYLE_ROUND,
                    endCap: D2D1_CAP_STYLE_ROUND,
                    lineJoin: D2D1_LINE_JOIN_ROUND,
                    ..Default::default()
                },
                None,
            )?;
            let image_brush = ID2D1RenderTarget::CreateBitmapBrush(
                &context,
                None::<&ID2D1Bitmap>,
                Some(&D2D1_BITMAP_BRUSH_PROPERTIES {
                    extendModeX: D2D1_EXTEND_MODE_CLAMP,
                    extendModeY: D2D1_EXTEND_MODE_CLAMP,
                    interpolationMode: D2D1_BITMAP_INTERPOLATION_MODE_LINEAR,
                }),
                None,
            )?;
            Ok(Surface {
                hwnd,
                width,
                height,
                factory,
                context,
                backing,
                output,
                swap,
                composition,
                _target: target,
                _visual: visual,
                base,
                images: VecDeque::new(),
                geometries: VecDeque::new(),
                damage: Vec::new(),
                brush,
                stroke_style,
                image_brush,
                gdi_matrix: std::ptr::null_mut(),
                text_rasters: VecDeque::new(),
                current_clip: None,
                retained: retained::Retention::default(),
                uploads: 0,
                upload_bytes: 0,
                deferred_uploads: false,
                deferred_layers: 0,
                wake_posted: false,
                active: false,
                failed: false,
            })
        }

        unsafe fn resize_surface(
            s: &mut Surface,
            width: i32,
            height: i32,
            caption: i32,
        ) -> Result<()> {
            let props = D2D1_BITMAP_PROPERTIES1 {
                pixelFormat: D2D1_PIXEL_FORMAT {
                    format: DXGI_FORMAT_B8G8R8A8_UNORM,
                    alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
                },
                dpiX: 96.,
                dpiY: 96.,
                bitmapOptions: D2D1_BITMAP_OPTIONS_TARGET,
                ..Default::default()
            };
            let size = D2D_SIZE_U {
                width: width as u32,
                height: height as u32,
            };
            let backing = s.context.CreateBitmap(size, None, 0, &props)?;
            let base = ID2D1RenderTarget::CreateBitmap(
                &s.context,
                size,
                None,
                0,
                &D2D1_BITMAP_PROPERTIES {
                    pixelFormat: props.pixelFormat,
                    dpiX: 96.,
                    dpiY: 96.,
                },
            )?;
            s.context.SetTarget(None::<&ID2D1Image>);
            s.output = backing.clone();
            s.swap.ResizeBuffers(
                0,
                width as u32,
                height as u32,
                DXGI_FORMAT_UNKNOWN,
                DXGI_SWAP_CHAIN_FLAG(0),
            )?;
            let buffer: IDXGISurface = s.swap.GetBuffer(0)?;
            let output_props = D2D1_BITMAP_PROPERTIES1 {
                bitmapOptions: D2D1_BITMAP_OPTIONS_TARGET | D2D1_BITMAP_OPTIONS_CANNOT_DRAW,
                ..props
            };
            s.output = s
                .context
                .CreateBitmapFromDxgiSurface(&buffer, Some(&output_props))?;
            s.backing = backing;
            s.base = base;
            s.width = width;
            s.height = height;
            s._visual.SetOffsetY2(caption as f32)?;
            s.retained = retained::Retention::default();
            Ok(())
        }
        pub(crate) unsafe fn begin(
            hwnd: windows_sys::Win32::Foundation::HWND,
            bits: *const c_void,
            stride: i32,
            width: i32,
            height: i32,
            caption: i32,
            dirty: Option<RECT>,
        ) -> bool {
            begin_with_region(
                hwnd,
                bits,
                stride,
                width,
                height,
                caption,
                dirty,
                std::ptr::null_mut(),
            )
        }
        pub(crate) unsafe fn begin_with_region(
            hwnd: windows_sys::Win32::Foundation::HWND,
            bits: *const c_void,
            stride: i32,
            width: i32,
            height: i32,
            caption: i32,
            dirty: Option<RECT>,
            region: windows_sys::Win32::Graphics::Gdi::HRGN,
        ) -> bool {
            use windows_sys::Win32::Graphics::Gdi::{GetRegionData, RGNDATA, RGNDATAHEADER};
            let damage = if !region.is_null() && dirty.is_some() {
                let bytes = GetRegionData(region, 0, std::ptr::null_mut());
                if bytes < std::mem::size_of::<RGNDATAHEADER>() as u32 {
                    return false;
                }
                let mut buffer =
                    vec![0usize; (bytes as usize).div_ceil(std::mem::size_of::<usize>())];
                let data = buffer.as_mut_ptr().cast::<RGNDATA>();
                if GetRegionData(region, bytes, data) != bytes {
                    return false;
                }
                let count = (*data).rdh.nCount as usize;
                if count
                    > (bytes as usize - std::mem::size_of::<RGNDATAHEADER>())
                        / std::mem::size_of::<RECT>()
                {
                    return false;
                }
                let bounds = dirty.unwrap();
                std::slice::from_raw_parts(
                    buffer
                        .as_ptr()
                        .cast::<u8>()
                        .add(std::mem::size_of::<RGNDATAHEADER>())
                        .cast::<RECT>(),
                    count,
                )
                .iter()
                .filter_map(|r| {
                    let r = RECT {
                        left: r.left.max(bounds.left),
                        top: (r.top - caption).max(bounds.top),
                        right: r.right.min(bounds.right),
                        bottom: (r.bottom - caption).min(bounds.bottom),
                    };
                    (r.left < r.right && r.top < r.bottom).then_some(r)
                })
                .collect::<Vec<_>>()
            } else {
                vec![dirty.unwrap_or(RECT {
                    left: 0,
                    top: 0,
                    right: width,
                    bottom: height,
                })]
            };
            if (!bits.is_null() && stride < width) || width <= 0 || height <= 0 {
                return false;
            }
            if RETRY.with(|r| r.borrow().is_some_and(|t| Instant::now() < t)) {
                return false;
            }
            SURFACE.with(|slot| {
                let mut slot = slot.borrow_mut();
                let hwnd = HWND(hwnd);
                if slot
                    .as_ref()
                    .is_some_and(|s| s.hwnd == hwnd && (s.width != width || s.height != height))
                {
                    if dirty.is_some() {
                        return false;
                    }
                    if resize_surface(slot.as_mut().unwrap(), width, height, caption).is_err() {
                        slot.take();
                    }
                }
                if slot.as_ref().is_none_or(|s| s.hwnd != hwnd) {
                    slot.take();
                    if dirty.is_some() {
                        return false;
                    }
                    *slot = match create(hwnd, width, height, caption) {
                        Ok(surface) => Some(surface),
                        Err(_error) => {
                            #[cfg(test)]
                            eprintln!("GPU initialization failed: {_error}");
                            None
                        }
                    };
                }
                let Some(s) = slot.as_mut() else {
                    retry_later();
                    return false;
                };
                let upload = dirty.map(|r| D2D_RECT_U {
                    left: r.left.clamp(0, width) as u32,
                    top: r.top.clamp(0, height) as u32,
                    right: r.right.clamp(0, width) as u32,
                    bottom: r.bottom.clamp(0, height) as u32,
                });
                let source = if bits.is_null() {
                    bits
                } else {
                    upload.map_or(bits, |r| {
                        bits.cast::<u8>()
                            .add((r.top as usize * stride as usize + r.left as usize) * 4)
                            .cast()
                    })
                };
                if !bits.is_null()
                    && s.base
                        .CopyFromMemory(
                            upload.as_ref().map(|r| r as *const _),
                            source,
                            stride as u32 * 4,
                        )
                        .is_err()
                {
                    slot.take();
                    retry_later();
                    return false;
                }
                s.context.SetTarget(&s.backing);
                s.uploads = 0;
                s.upload_bytes = 0;
                s.deferred_layers = 0;
                s.context.SetTransform(&identity());
                s.context.BeginDraw();
                s.damage = damage;
                for r in &s.damage {
                    let rect = D2D_RECT_F {
                        left: r.left as f32,
                        top: r.top as f32,
                        right: r.right as f32,
                        bottom: r.bottom as f32,
                    };
                    s.context
                        .PushAxisAlignedClip(&rect, D2D1_ANTIALIAS_MODE_ALIASED);
                    s.context.Clear(Some(&D2D1_COLOR_F::default()));
                    if !bits.is_null() {
                        ID2D1RenderTarget::DrawBitmap(
                            &s.context,
                            &s.base,
                            None,
                            1.,
                            D2D1_BITMAP_INTERPOLATION_MODE_NEAREST_NEIGHBOR,
                            None,
                        );
                    }
                    s.context.PopAxisAlignedClip();
                }
                s.active = true;
                s.failed = false;
                true
            })
        }
        /// Background crop, transition, tint and brightness are GPU operations too.
        pub(crate) unsafe fn background(
            image: Option<&crate::catalog::Bitmap>,
            alpha: u32,
            tint: u32,
            opacity: u32,
            brightness: u32,
            caption: i32,
            _dirty: Option<RECT>,
            solid: Option<u32>,
        ) {
            SURFACE.with(|slot| {
                let mut slot = slot.borrow_mut();
                let Some(s) = slot.as_mut().filter(|s| s.active) else {
                    return;
                };
                for r in s.damage.clone() {
                    s.context.PushAxisAlignedClip(
                        &D2D_RECT_F {
                            left: r.left as f32,
                            top: r.top as f32,
                            right: r.right as f32,
                            bottom: r.bottom as f32,
                        },
                        D2D1_ANTIALIAS_MODE_ALIASED,
                    );
                    let result = (|| -> Result<()> {
                        if let Some(solid) = solid {
                            s.context.Clear(Some(&color(solid)));
                            return Ok(());
                        }
                        let gain = brightness as f32 / 100.;
                        let tint = color(tint | 0xff000000);
                        let base = D2D1_COLOR_F {
                            r: (tint.r * gain).min(1.),
                            g: (tint.g * gain).min(1.),
                            b: (tint.b * gain).min(1.),
                            a: 1.,
                        };
                        s.context.Clear(Some(&base));
                        let Some(image) = image.filter(|i| i.width > 0 && i.height > 0) else {
                            return Ok(());
                        };
                        let scale = (s.width as f32 / image.width as f32)
                            .max((s.height + caption) as f32 / image.height as f32);
                        let width = (image.width as f32 * scale).ceil() as i32;
                        let height = (image.height as f32 * scale).ceil() as i32;
                        let left = (s.width - width) / 2;
                        let top = (s.height + caption - height) / 2 - caption;
                        let list = s.context.CreateCommandList()?;
                        s.context.SetTarget(&list);
                        let result = s.image(
                            (image.id << 1) | 1,
                            image.pixels.as_ptr().cast(),
                            image.width,
                            image.height,
                            image.width * 4,
                            RECT {
                                left,
                                top,
                                right: left + width,
                                bottom: top + height,
                            },
                            0.,
                            255,
                        );
                        s.context.SetTarget(&s.backing);
                        result?;
                        list.Close()?;
                        let effect = s.context.CreateEffect(&CLSID_D2D1ColorMatrix)?;
                        effect.SetInput(0, &list, false);
                        let weight =
                            alpha.min(255) as f32 / 255. * (1. - opacity.min(255) as f32 / 255.);
                        let mut matrix = [0f32; 20];
                        matrix[0] = weight * gain;
                        matrix[5] = weight * gain;
                        matrix[10] = weight * gain;
                        matrix[15] = 1.;
                        matrix[16] = tint.r * (1. - weight) * gain;
                        matrix[17] = tint.g * (1. - weight) * gain;
                        matrix[18] = tint.b * (1. - weight) * gain;
                        let bytes = std::slice::from_raw_parts(
                            matrix.as_ptr().cast::<u8>(),
                            std::mem::size_of_val(&matrix),
                        );
                        effect.SetValue(
                            D2D1_COLORMATRIX_PROP_COLOR_MATRIX.0 as u32,
                            D2D1_PROPERTY_TYPE_MATRIX_5X4,
                            bytes,
                        )?;
                        s.context.DrawImage(
                            &effect.GetOutput()?,
                            None,
                            None,
                            D2D1_INTERPOLATION_MODE_NEAREST_NEIGHBOR,
                            D2D1_COMPOSITE_MODE_SOURCE_OVER,
                        );
                        Ok(())
                    })();
                    if result.is_err() {
                        s.failed = true;
                    }
                    s.context.PopAxisAlignedClip();
                }
            });
        }
        pub(crate) unsafe fn finish() -> bool {
            SURFACE.with(|slot| {
                let mut slot = slot.borrow_mut();
                let Some(s) = slot.as_mut().filter(|s| s.active) else {
                    return false;
                };
                s.active = false;
                let result = (|| -> Result<()> {
                    s.composite_layers()?;
                    s.context.EndDraw(None, None)?;
                    if s.failed {
                        return Err(windows::core::Error::empty());
                    }
                    s.context.SetTarget(&s.output);
                    s.context.SetTransform(&identity());
                    s.context.BeginDraw();
                    s.context.Clear(Some(&D2D1_COLOR_F::default()));
                    ID2D1RenderTarget::DrawBitmap(
                        &s.context,
                        &s.backing,
                        None,
                        1.,
                        D2D1_BITMAP_INTERPOLATION_MODE_NEAREST_NEIGHBOR,
                        None,
                    );
                    s.context.EndDraw(None, None)?;
                    s.swap.Present(0, DXGI_PRESENT(0)).ok()?;
                    if s.deferred_uploads {
                        s.retained.invalidate_deferred(s.deferred_layers);
                        if !s.wake_posted {
                            s.wake_posted =
                                windows_sys::Win32::UI::WindowsAndMessaging::PostMessageW(
                                    s.hwnd.0,
                                    UPLOAD_WAKE,
                                    0,
                                    0,
                                ) != 0;
                        }
                    }
                    s.composition.Commit()
                })();
                if result.is_err() {
                    slot.take();
                    retry_later();
                    return false;
                }
                true
            })
        }
        /// Read back only the native search editor's small background snapshot.
        pub(crate) unsafe fn snapshot(rect: RECT) -> Option<Vec<u32>> {
            SURFACE.with(|slot| {
                let slot = slot.borrow();
                let s = slot.as_ref()?;
                if rect.left < 0 || rect.top < 0 || rect.right > s.width || rect.bottom > s.height {
                    return None;
                }
                let (width, height) = (rect.right - rect.left, rect.bottom - rect.top);
                if width <= 0 || height <= 0 {
                    return None;
                }
                let bitmap = s
                    .context
                    .CreateBitmap(
                        D2D_SIZE_U {
                            width: width as u32,
                            height: height as u32,
                        },
                        None,
                        0,
                        &D2D1_BITMAP_PROPERTIES1 {
                            pixelFormat: D2D1_PIXEL_FORMAT {
                                format: DXGI_FORMAT_B8G8R8A8_UNORM,
                                alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
                            },
                            dpiX: 96.,
                            dpiY: 96.,
                            bitmapOptions: D2D1_BITMAP_OPTIONS_CPU_READ
                                | D2D1_BITMAP_OPTIONS_CANNOT_DRAW,
                            ..Default::default()
                        },
                    )
                    .ok()?;
                bitmap
                    .CopyFromBitmap(
                        None,
                        &s.backing,
                        Some(&D2D_RECT_U {
                            left: rect.left as u32,
                            top: rect.top as u32,
                            right: rect.right as u32,
                            bottom: rect.bottom as u32,
                        }),
                    )
                    .ok()?;
                let mapped = bitmap.Map(D2D1_MAP_OPTIONS_READ).ok()?;
                let mut pixels = Vec::with_capacity((width * height) as usize);
                for y in 0..height {
                    pixels.extend_from_slice(std::slice::from_raw_parts(
                        mapped
                            .bits
                            .add(y as usize * mapped.pitch as usize)
                            .cast::<u32>(),
                        width as usize,
                    ));
                }
                let _ = bitmap.Unmap();
                Some(pixels)
            })
        }
        pub(super) unsafe fn forget_image(key: u64) {
            let _ = SURFACE.try_with(|s| {
                if let Some(s) = s.borrow_mut().as_mut() {
                    s.images.retain(|i| i.0 != key);
                }
            });
        }
        pub(super) fn forget_path(path: usize) {
            let _ = SURFACE.try_with(|s| {
                if let Some(s) = s.borrow_mut().as_mut() {
                    s.geometries.retain(|g| g.0 != path);
                    s.text_rasters.retain(|g| g.0 .0 != path);
                }
            });
        }
        pub(crate) fn abandon() {
            hide();
            retry_later();
        }
        pub(crate) fn take_deferred_uploads() -> bool {
            SURFACE.with(|slot| {
                let mut slot = slot.borrow_mut();
                let Some(s) = slot.as_mut() else {
                    return false;
                };
                s.wake_posted = false;
                std::mem::take(&mut s.deferred_uploads)
            })
        }
        /// Preserve the exact GDI+ clip (including holes) and transform for every draw.
        pub(super) unsafe fn draw(
            g: *mut GpGraphics,
            operation: impl Fn(&mut Surface) -> Result<()>,
        ) -> bool {
            if !super::is_current_graphics(g) {
                return false;
            }
            super::GPU_DC_CLIPS.with(|dc_clips| {
                let dc_clips = dc_clips.borrow();
                SURFACE.with(|slot| {
                    let mut slot = slot.borrow_mut();
                    let Some(s) = slot.as_mut().filter(|s| s.active) else {
                        return false;
                    };
                    if s.retained.reusing() {
                        return true;
                    }
                    let result = (|| -> Result<()> {
                        if s.gdi_matrix.is_null() {
                            GdipCreateMatrix(&mut s.gdi_matrix);
                        }
                        GdipGetWorldTransform(g, s.gdi_matrix);
                        let mut v = [0f32; 6];
                        GdipGetMatrixElements(s.gdi_matrix, v.as_mut_ptr());
                        let transform = Matrix3x2 {
                            M11: v[0],
                            M12: v[1],
                            M21: v[2],
                            M22: v[3],
                            M31: v[4],
                            M32: v[5],
                        };
                        // Region scans round outward to pixels and can let text spill
                        // beyond its layout rectangle. Use exact world bounds plus the
                        // original Win32 rectangles for holes instead of scan conversion.
                        let mut bounds = RectF::default();
                        GdipGetClipBounds(g, &mut bounds);
                        let scans = [bounds];
                        let actual = 1;
                        s.context.SetTransform(&transform);
                        for r in scans.iter().take(actual.max(0) as usize) {
                            // GDI+ region scans may include a boundary pixel beyond an
                            // HRGN edge. Never allow a draw to modify retained pixels
                            // outside the actual Windows update region.
                            let corners = [
                                (r.X, r.Y),
                                (r.X + r.Width, r.Y),
                                (r.X, r.Y + r.Height),
                                (r.X + r.Width, r.Y + r.Height),
                            ]
                            .map(|(x, y)| (x * v[0] + y * v[2] + v[4], x * v[1] + y * v[3] + v[5]));
                            let left = corners.iter().map(|p| p.0).fold(f32::INFINITY, f32::min);
                            let top = corners.iter().map(|p| p.1).fold(f32::INFINITY, f32::min);
                            let right = corners
                                .iter()
                                .map(|p| p.0)
                                .fold(f32::NEG_INFINITY, f32::max);
                            let bottom = corners
                                .iter()
                                .map(|p| p.1)
                                .fold(f32::NEG_INFINITY, f32::max);
                            let recording = s.retained.recording_bounds();
                            for index in 0..if recording.is_some() {
                                1
                            } else {
                                s.damage.len()
                            } {
                                let damage = recording.unwrap_or_else(|| s.damage[index]);
                                for dc_clip in dc_clips.iter() {
                                    let clip = D2D_RECT_F {
                                        left: left.max(damage.left.max(dc_clip.left) as f32),
                                        top: top.max(damage.top.max(dc_clip.top) as f32),
                                        right: right.min(damage.right.min(dc_clip.right) as f32),
                                        bottom: bottom
                                            .min(damage.bottom.min(dc_clip.bottom) as f32),
                                    };
                                    if clip.left >= clip.right || clip.top >= clip.bottom {
                                        continue;
                                    }
                                    s.context.SetTransform(&identity());
                                    s.context
                                        .PushAxisAlignedClip(&clip, D2D1_ANTIALIAS_MODE_ALIASED);
                                    s.context.SetTransform(&transform);
                                    s.current_clip = Some(clip);
                                    let result = operation(s);
                                    s.context.PopAxisAlignedClip();
                                    s.current_clip = None;
                                    result?;
                                }
                            }
                        }
                        Ok(())
                    })();
                    if result.is_err() {
                        s.failed = true;
                    }
                    true
                })
            })
        }
        fn color(c: u32) -> D2D1_COLOR_F {
            D2D1_COLOR_F {
                r: ((c >> 16) & 255) as f32 / 255.,
                g: ((c >> 8) & 255) as f32 / 255.,
                b: (c & 255) as f32 / 255.,
                a: (c >> 24) as f32 / 255.,
            }
        }
        impl Surface {
            pub(super) unsafe fn text(
                &mut self,
                path: *mut GpPath,
                value: u32,
                width: i32,
                height: i32,
            ) -> Result<()> {
                if width <= 0 || height <= 0 {
                    return Ok(());
                }
                if width > 4096 || height > 1024 {
                    self.solid(path, (value & 0xffffff) | 0x30000000, Some(0.3))?;
                    return self.solid(path, value, None);
                }
                // Cache coverage, not color: hover/theme changes reuse the same mask.
                let key = (path as usize, width, height);
                let bitmap = if let Some(index) =
                    self.text_rasters.iter().position(|entry| entry.0 == key)
                {
                    let entry = self.text_rasters.remove(index).unwrap();
                    let bitmap = entry.1.clone();
                    self.text_rasters.push_back(entry);
                    bitmap
                } else {
                    let bitmap = self.context.CreateBitmap(
                        D2D_SIZE_U {
                            width: width as u32,
                            height: height as u32,
                        },
                        None,
                        0,
                        &D2D1_BITMAP_PROPERTIES1 {
                            pixelFormat: D2D1_PIXEL_FORMAT {
                                format: DXGI_FORMAT_B8G8R8A8_UNORM,
                                alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
                            },
                            dpiX: 96.,
                            dpiY: 96.,
                            bitmapOptions: D2D1_BITMAP_OPTIONS_TARGET,
                            ..Default::default()
                        },
                    )?;
                    let previous = self.context.GetTarget()?;
                    let mut transform = Matrix3x2::default();
                    self.context.GetTransform(&mut transform);
                    if self.current_clip.is_some() {
                        self.context.PopAxisAlignedClip();
                    }
                    self.context.SetTarget(&bitmap);
                    self.context.SetTransform(&identity());
                    self.context.Clear(Some(&D2D1_COLOR_F::default()));
                    let result = (|| -> Result<()> {
                        self.solid(path, 0x30ffffff, Some(0.3))?;
                        self.solid(path, 0xffffffff, None)
                    })();
                    self.context.SetTarget(&previous);
                    if let Some(clip) = self.current_clip {
                        self.context
                            .PushAxisAlignedClip(&clip, D2D1_ANTIALIAS_MODE_ALIASED);
                    }
                    self.context.SetTransform(&transform);
                    result?;
                    let bytes = width as usize * height as usize * 4;
                    while self.text_rasters.len() >= 256
                        || self.text_rasters.iter().map(|e| e.2).sum::<usize>() + bytes
                            > 16 * 1024 * 1024
                    {
                        if self.text_rasters.pop_front().is_none() {
                            break;
                        }
                    }
                    self.text_rasters.push_back((key, bitmap.clone(), bytes));
                    bitmap
                };
                self.brush.SetColor(&color(value));
                let mode = self.context.GetAntialiasMode();
                self.context.SetAntialiasMode(D2D1_ANTIALIAS_MODE_ALIASED);
                ID2D1RenderTarget::FillOpacityMask(
                    &self.context,
                    &bitmap,
                    &self.brush,
                    D2D1_OPACITY_MASK_CONTENT_GRAPHICS,
                    None,
                    None,
                );
                self.context.SetAntialiasMode(mode);
                Ok(())
            }
            pub(super) unsafe fn rounded(
                &mut self,
                r: RECT,
                value: u32,
                radius: f32,
                stroke: Option<f32>,
            ) -> Result<()> {
                self.rounded_offset(r, value, radius, stroke, 0.)
            }
            pub(super) unsafe fn rounded_offset(
                &mut self,
                r: RECT,
                value: u32,
                radius: f32,
                stroke: Option<f32>,
                y: f32,
            ) -> Result<()> {
                self.brush.SetColor(&color(value));
                let rounded = D2D1_ROUNDED_RECT {
                    rect: D2D_RECT_F {
                        left: r.left as f32,
                        top: r.top as f32 + y,
                        right: r.right as f32,
                        bottom: r.bottom as f32 + y,
                    },
                    radiusX: radius
                        .max(0.)
                        .min((r.right - r.left).min(r.bottom - r.top) as f32 / 2.),
                    radiusY: radius
                        .max(0.)
                        .min((r.right - r.left).min(r.bottom - r.top) as f32 / 2.),
                };
                if let Some(width) = stroke {
                    self.context.DrawRoundedRectangle(
                        &rounded,
                        &self.brush,
                        width,
                        &self.stroke_style,
                    );
                } else {
                    self.context.FillRoundedRectangle(&rounded, &self.brush);
                }
                Ok(())
            }
            pub(super) unsafe fn line(
                &mut self,
                x: f32,
                y: f32,
                u: f32,
                v: f32,
                value: u32,
                width: f32,
            ) -> Result<()> {
                self.brush.SetColor(&color(value));
                self.context.DrawLine(
                    Vector2 { X: x, Y: y },
                    Vector2 { X: u, Y: v },
                    &self.brush,
                    width,
                    &self.stroke_style,
                );
                Ok(())
            }
            pub(super) unsafe fn blur(
                &mut self,
                path: *mut GpPath,
                value: u32,
                sigma: f32,
                stroke: Option<f32>,
            ) -> Result<()> {
                let list = self.context.CreateCommandList()?;
                let previous = self.context.GetTarget()?;
                let mut transform = Matrix3x2::default();
                self.context.GetTransform(&mut transform);
                self.context.SetTarget(&list);
                let result = self.solid(path, value, stroke);
                self.context.SetTarget(&previous);
                result?;
                list.Close()?;
                let effect = self.context.CreateEffect(&CLSID_D2D1GaussianBlur)?;
                effect.SetInput(0, &list, false);
                effect.SetValue(
                    D2D1_GAUSSIANBLUR_PROP_STANDARD_DEVIATION.0 as u32,
                    D2D1_PROPERTY_TYPE_FLOAT,
                    &sigma.to_ne_bytes(),
                )?;
                self.context.SetTransform(&identity());
                self.context.DrawImage(
                    &effect.GetOutput()?,
                    None,
                    None,
                    D2D1_INTERPOLATION_MODE_LINEAR,
                    D2D1_COMPOSITE_MODE_SOURCE_OVER,
                );
                self.context.SetTransform(&transform);
                Ok(())
            }
            pub(super) unsafe fn path(&mut self, path: *mut GpPath) -> Result<ID2D1PathGeometry> {
                if let Some(index) = self.geometries.iter().position(|g| g.0 == path as usize) {
                    let entry = self.geometries.remove(index).unwrap();
                    let geometry = entry.1.clone();
                    self.geometries.push_back(entry);
                    return Ok(geometry);
                }
                let mut count = 0;
                GdipGetPointCount(path, &mut count);
                let mut points = vec![PointF::default(); count.max(0) as usize];
                let mut types = vec![0u8; points.len()];
                GdipGetPathPoints(path, points.as_mut_ptr(), count);
                GdipGetPathTypes(path, types.as_mut_ptr(), count);
                let geometry = self.factory.CreatePathGeometry()?;
                let sink = geometry.Open()?;
                let mut mode = 0;
                GdipGetPathFillMode(path, &mut mode);
                sink.SetFillMode(if mode == FillModeAlternate {
                    D2D1_FILL_MODE_ALTERNATE
                } else {
                    D2D1_FILL_MODE_WINDING
                });
                let point = |i: usize| Vector2 {
                    X: points[i].X,
                    Y: points[i].Y,
                };
                let mut i = 0;
                let mut open = false;
                while i < points.len() {
                    match types[i] & 7 {
                        0 => {
                            if open {
                                sink.EndFigure(D2D1_FIGURE_END_OPEN);
                            }
                            sink.BeginFigure(point(i), D2D1_FIGURE_BEGIN_FILLED);
                            open = true;
                        }
                        3 if i + 2 < points.len() => {
                            sink.AddBezier(&D2D1_BEZIER_SEGMENT {
                                point1: point(i),
                                point2: point(i + 1),
                                point3: point(i + 2),
                            });
                            i += 2;
                        }
                        _ => sink.AddLine(point(i)),
                    }
                    if types[i] & 0x80 != 0 && open {
                        sink.EndFigure(D2D1_FIGURE_END_CLOSED);
                        open = false;
                    }
                    i += 1;
                }
                if open {
                    sink.EndFigure(D2D1_FIGURE_END_OPEN);
                }
                sink.Close()?;
                let geometry: ID2D1PathGeometry = geometry.into();
                let bytes = points.len() * std::mem::size_of::<PointF>();
                while self.geometries.len() >= 512
                    || self.geometries.iter().map(|g| g.2).sum::<usize>() + bytes > 16 * 1024 * 1024
                {
                    if self.geometries.pop_front().is_none() {
                        break;
                    }
                }
                self.geometries
                    .push_back((path as usize, geometry.clone(), bytes));
                Ok(geometry)
            }
            pub(super) unsafe fn solid(
                &mut self,
                path: *mut GpPath,
                value: u32,
                stroke: Option<f32>,
            ) -> Result<()> {
                let geometry = self.path(path)?;
                let brush = &self.brush;
                brush.SetColor(&color(value));
                if let Some(width) = stroke {
                    self.context
                        .DrawGeometry(&geometry, brush, width, &self.stroke_style);
                } else {
                    self.context.FillGeometry(&geometry, brush, None);
                }
                Ok(())
            }
            pub(super) unsafe fn gradient(
                &mut self,
                path: *mut GpPath,
                colors: [u32; 2],
                r: RectF,
                angle: f32,
                stroke: Option<f32>,
            ) -> Result<()> {
                let geometry = self.path(path)?;
                let a = angle.to_radians();
                let (dx, dy) = (a.cos(), a.sin());
                let extent = (dx.abs() * r.Width + dy.abs() * r.Height) / 2.;
                let (x, y) = (r.X + r.Width / 2., r.Y + r.Height / 2.);
                let stops = ID2D1RenderTarget::CreateGradientStopCollection(
                    &self.context,
                    &[
                        D2D1_GRADIENT_STOP {
                            position: 0.,
                            color: color(colors[0]),
                        },
                        D2D1_GRADIENT_STOP {
                            position: 1.,
                            color: color(colors[1]),
                        },
                    ],
                    D2D1_GAMMA_2_2,
                    D2D1_EXTEND_MODE_MIRROR,
                )?;
                let brush = self.context.CreateLinearGradientBrush(
                    &D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES {
                        startPoint: Vector2 {
                            X: x - dx * extent,
                            Y: y - dy * extent,
                        },
                        endPoint: Vector2 {
                            X: x + dx * extent,
                            Y: y + dy * extent,
                        },
                    },
                    None,
                    &stops,
                )?;
                if let Some(width) = stroke {
                    self.context.DrawGeometry(&geometry, &brush, width, None);
                } else {
                    self.context.FillGeometry(&geometry, &brush, None);
                }
                Ok(())
            }
            pub(super) unsafe fn image(
                &mut self,
                key: u64,
                pixels: *const c_void,
                width: u32,
                height: u32,
                stride: u32,
                r: RECT,
                radius: f32,
                alpha: u8,
            ) -> Result<()> {
                if width == 0 || height == 0 || r.right <= r.left || r.bottom <= r.top {
                    return Ok(());
                }
                let bitmap = if let Some(i) = self.images.iter().position(|i| i.0 == key) {
                    let entry = self.images.remove(i).unwrap();
                    let bitmap = entry.1.clone();
                    self.images.push_back(entry);
                    bitmap
                } else {
                    let bytes = width as usize * height as usize * 4;
                    if self.uploads >= 2
                        || self.uploads > 0 && self.upload_bytes + bytes > 4 * 1024 * 1024
                    {
                        self.deferred_uploads = true;
                        self.deferred_layers |= self.retained.layer_mask();
                        // Decoding has finished, but GPU admission can still wait. Keep
                        // a static tile visible instead of recording a blank image.
                        self.rounded(
                            r,
                            ((alpha as u32 * 128 / 255) << 24) | 0x006b6b73,
                            radius,
                            None,
                        )?;
                        return Ok(());
                    }
                    let bitmap = ID2D1RenderTarget::CreateBitmap(
                        &self.context,
                        D2D_SIZE_U { width, height },
                        Some(pixels),
                        stride,
                        &D2D1_BITMAP_PROPERTIES {
                            pixelFormat: D2D1_PIXEL_FORMAT {
                                format: DXGI_FORMAT_B8G8R8A8_UNORM,
                                alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
                            },
                            dpiX: 96.,
                            dpiY: 96.,
                        },
                    )?;
                    self.uploads += 1;
                    self.upload_bytes += bytes;
                    let bytes = width as usize * height as usize * 4;
                    while self.images.len() >= 256
                        || self.images.iter().map(|i| i.2).sum::<usize>() + bytes > 64 * 1024 * 1024
                    {
                        if self.images.pop_front().is_none() {
                            break;
                        }
                    }
                    if bytes <= 64 * 1024 * 1024 {
                        self.images.push_back((key, bitmap.clone(), bytes));
                    }
                    bitmap
                };
                let brush = &self.image_brush;
                brush.SetBitmap(&bitmap);
                brush.SetTransform(&Matrix3x2 {
                    M11: (r.right - r.left) as f32 / width as f32,
                    M22: (r.bottom - r.top) as f32 / height as f32,
                    M31: r.left as f32,
                    M32: r.top as f32,
                    ..Default::default()
                });
                brush.SetOpacity(alpha as f32 / 255.);
                self.context.FillRoundedRectangle(
                    &D2D1_ROUNDED_RECT {
                        rect: D2D_RECT_F {
                            left: r.left as f32,
                            top: r.top as f32,
                            right: r.right as f32,
                            bottom: r.bottom as f32,
                        },
                        radiusX: radius,
                        radiusY: radius,
                    },
                    brush,
                );
                Ok(())
            }
        }

        #[cfg(test)]
        mod tests {
            use super::*;
            use windows_sys::Win32::{Graphics::Gdi::*, UI::WindowsAndMessaging::*};

            #[test]
            #[ignore = "Requires Windows hardware Direct3D and DirectComposition"]
            fn gpu_upload_budget_defers_misses_and_reuses_cached_artwork() {
                unsafe {
                    let hwnd = CreateWindowExW(
                        0,
                        windows::core::w!("STATIC").as_ptr(),
                        std::ptr::null(),
                        WS_POPUP,
                        0,
                        0,
                        64,
                        64,
                        std::ptr::null_mut(),
                        std::ptr::null_mut(),
                        std::ptr::null_mut(),
                        std::ptr::null(),
                    );
                    assert!(!hwnd.is_null());
                    let pixel = 0xff123456u32;
                    let rect = RECT {
                        left: 0,
                        top: 0,
                        right: 16,
                        bottom: 16,
                    };
                    assert!(begin(hwnd, std::ptr::null(), 64, 64, 64, 0, None));
                    SURFACE.with(|slot| {
                        let mut slot = slot.borrow_mut();
                        let surface = slot.as_mut().unwrap();
                        for key in 1..=3 {
                            surface
                                .image(
                                    key,
                                    (&pixel as *const u32).cast(),
                                    1,
                                    1,
                                    4,
                                    if key == 3 {
                                        RECT {
                                            left: 32,
                                            right: 48,
                                            ..rect
                                        }
                                    } else {
                                        rect
                                    },
                                    0.,
                                    255,
                                )
                                .unwrap();
                        }
                        assert_eq!(surface.uploads, 2);
                        assert_eq!(surface.images.len(), 2);
                    });
                    assert!(finish());
                    let placeholder = snapshot(RECT {
                        left: 36,
                        top: 4,
                        right: 37,
                        bottom: 5,
                    })
                    .unwrap()[0];
                    assert!(
                        placeholder >> 24 > 0,
                        "upload-deferred artwork must have a visible placeholder"
                    );
                    assert_ne!(placeholder, pixel);
                    assert!(take_deferred_uploads());
                    assert!(begin(hwnd, std::ptr::null(), 64, 64, 64, 0, None));
                    SURFACE.with(|slot| {
                        let mut slot = slot.borrow_mut();
                        let surface = slot.as_mut().unwrap();
                        for key in 1..=3 {
                            surface
                                .image(
                                    key,
                                    (&pixel as *const u32).cast(),
                                    1,
                                    1,
                                    4,
                                    if key == 3 {
                                        RECT {
                                            left: 32,
                                            right: 48,
                                            ..rect
                                        }
                                    } else {
                                        rect
                                    },
                                    0.,
                                    255,
                                )
                                .unwrap();
                        }
                        assert_eq!(surface.uploads, 1);
                        assert_eq!(surface.images.len(), 3);
                    });
                    assert!(finish());
                    assert!(!take_deferred_uploads());
                    assert_eq!(
                        snapshot(RECT {
                            left: 36,
                            top: 4,
                            right: 37,
                            bottom: 5
                        })
                        .unwrap()[0],
                        pixel,
                        "the admitted texture must replace the placeholder"
                    );
                    assert_eq!(
                        snapshot(RECT {
                            left: 4,
                            top: 4,
                            right: 5,
                            bottom: 5
                        })
                        .unwrap()[0],
                        pixel
                    );
                    SURFACE.with(|slot| *slot.borrow_mut() = None);
                    DestroyWindow(hwnd);
                }
            }

            #[test]
            #[ignore = "Requires Windows hardware Direct3D and DirectComposition"]
            fn gpu_rasterizes_alpha_clips_text_artwork_and_blur_without_cpu_foreground() {
                unsafe {
                    let hwnd = CreateWindowExW(
                        0,
                        windows::core::w!("STATIC").as_ptr(),
                        std::ptr::null(),
                        WS_POPUP,
                        0,
                        0,
                        160,
                        160,
                        std::ptr::null_mut(),
                        std::ptr::null_mut(),
                        std::ptr::null_mut(),
                        std::ptr::null(),
                    );
                    assert!(!hwnd.is_null());
                    let dc = CreateCompatibleDC(std::ptr::null_mut());
                    let dc_bitmap = CreateBitmap(160, 160, 1, 32, std::ptr::null());
                    let old_bitmap = SelectObject(dc, dc_bitmap);
                    let mut pixels = vec![0u32; 160 * 160];
                    assert!(
                        begin(hwnd, pixels.as_ptr().cast(), 160, 160, 160, 0, None),
                        "hardware backend unavailable"
                    );
                    let frame = crate::drawing::Frame::begin(
                        dc,
                        pixels.as_mut_ptr().cast(),
                        160,
                        160,
                        160,
                        false,
                    )
                    .unwrap();
                    IntersectClipRect(dc, 0, 0, 160, 160);
                    assert!(crate::drawing::fill(
                        dc,
                        RECT {
                            left: 8,
                            top: 8,
                            right: 48,
                            bottom: 48
                        },
                        0x80ff0000,
                        0.
                    ));
                    let saved = SaveDC(dc);
                    IntersectClipRect(dc, 64, 8, 80, 24);
                    crate::drawing::fill(
                        dc,
                        RECT {
                            left: 56,
                            top: 0,
                            right: 96,
                            bottom: 40,
                        },
                        0xff00ff00,
                        0.,
                    );
                    RestoreDC(dc, saved);
                    let font = CreateFontW(
                        -18,
                        0,
                        0,
                        0,
                        400,
                        0,
                        0,
                        0,
                        1,
                        0,
                        0,
                        0,
                        0,
                        windows::core::w!("Segoe UI").as_ptr(),
                    );
                    crate::drawing::text(
                        dc,
                        font,
                        "GPU",
                        RECT {
                            left: 8,
                            top: 60,
                            right: 80,
                            bottom: 88,
                        },
                        0xffffff,
                        0,
                    );
                    let masks = SURFACE.with(|s| s.borrow().as_ref().unwrap().text_rasters.len());
                    crate::drawing::text(
                        dc,
                        font,
                        "GPU",
                        RECT {
                            left: 8,
                            top: 130,
                            right: 80,
                            bottom: 158,
                        },
                        0x00ff00,
                        0,
                    );
                    assert_eq!(
                        masks,
                        SURFACE.with(|s| s.borrow().as_ref().unwrap().text_rasters.len()),
                        "text color changes must reuse the GPU glyph mask"
                    );
                    crate::drawing::blurred_text(
                        dc,
                        font,
                        "Blur",
                        RECT {
                            left: 8,
                            top: 100,
                            right: 90,
                            bottom: 130,
                        },
                        0xffffff,
                        0,
                        2.,
                    );
                    crate::drawing::spotify_logo(
                        dc,
                        RECT {
                            left: 100,
                            top: 60,
                            right: 140,
                            bottom: 100,
                        },
                    );
                    crate::drawing::fill_offset_y(
                        dc,
                        RECT {
                            left: 148,
                            top: 80,
                            right: 158,
                            bottom: 90,
                        },
                        0xffffffff,
                        0.,
                        0.5,
                    );
                    crate::drawing::line(dc, 20., 52., 40., 52., 0xffff0000, 4.);
                    crate::drawing::line(dc, 60., 52., 80., 52., 0xff00ff00, 4.);
                    crate::drawing::gradient(
                        dc,
                        RECT {
                            left: 100,
                            top: 8,
                            right: 140,
                            bottom: 48,
                        },
                        0xff0000ff,
                        0xffffffff,
                        4.,
                    );
                    let artwork = crate::catalog::Bitmap::new(2, 2, vec![0xff112233; 4]);
                    crate::drawing::image(
                        dc,
                        RECT {
                            left: 100,
                            top: 110,
                            right: 140,
                            bottom: 150,
                        },
                        &artwork,
                        0.,
                        255,
                    );
                    assert!(
                        pixels.iter().all(|p| *p == 0),
                        "foreground must stay on the GPU"
                    );
                    drop(frame);
                    assert!(finish(), "GPU drawing or composition failed");
                    let rendered = snapshot(RECT {
                        left: 0,
                        top: 0,
                        right: 160,
                        bottom: 160,
                    })
                    .unwrap();
                    let red = rendered[20 * 160 + 20];
                    let fractional = rendered[80 * 160 + 152] >> 24;
                    assert!(
                        (120..=136).contains(&fractional),
                        "fractional scrollbar coverage: {fractional}"
                    );
                    assert!(
                        (red >> 24).abs_diff(128) <= 1 && ((red >> 16) & 255).abs_diff(128) <= 1,
                        "{red:08x}"
                    );
                    assert_eq!(rendered[12 * 160 + 68], 0xff00ff00);
                    assert_eq!(rendered[4 * 160 + 60], 0);
                    assert_eq!(rendered[52 * 160 + 30], 0xffff0000);
                    assert_eq!(
                        rendered[52 * 160 + 70],
                        0xff00ff00,
                        "temporary path geometry must not be reused after deletion"
                    );
                    assert!(snapshot(RECT {
                        left: 0,
                        top: 60,
                        right: 80,
                        bottom: 88
                    })
                    .unwrap()
                    .iter()
                    .any(|p| *p != 0));
                    assert!(
                        snapshot(RECT {
                            left: 0,
                            top: 100,
                            right: 90,
                            bottom: 128
                        })
                        .unwrap()
                        .iter()
                        .any(|p| *p != 0),
                        "GPU blur output missing"
                    );
                    assert!(
                        snapshot(RECT {
                            left: 0,
                            top: 130,
                            right: 80,
                            bottom: 158
                        })
                        .unwrap()
                        .iter()
                        .any(|p| p & 0xff00ff == 0 && p & 0x00ff00 != 0),
                        "cached glyph mask must accept a new color"
                    );
                    assert_eq!(rendered[120 * 160 + 120], 0xff112233);
                    let dirty = RECT {
                        left: 100,
                        top: 110,
                        right: 140,
                        bottom: 150,
                    };
                    assert!(begin(
                        hwnd,
                        pixels.as_ptr().cast(),
                        160,
                        160,
                        160,
                        0,
                        Some(dirty)
                    ));
                    let frame = crate::drawing::Frame::begin(
                        dc,
                        pixels.as_mut_ptr().cast(),
                        160,
                        160,
                        160,
                        false,
                    )
                    .unwrap();
                    let saved = SaveDC(dc);
                    IntersectClipRect(dc, dirty.left, dirty.top, dirty.right, dirty.bottom);
                    ExcludeClipRect(dc, 112, 122, 128, 138);
                    crate::drawing::fill(dc, dirty, 0xffabcdef, 0.);
                    RestoreDC(dc, saved);
                    drop(frame);
                    assert!(finish());
                    let partial = snapshot(RECT {
                        left: 0,
                        top: 0,
                        right: 160,
                        bottom: 160,
                    })
                    .unwrap();
                    assert_eq!(
                        partial[20 * 160 + 20],
                        red,
                        "partial paint must preserve untouched pixels"
                    );
                    assert_eq!(partial[120 * 160 + 104], 0xffabcdef);
                    assert_eq!(
                        partial[128 * 160 + 120],
                        0,
                        "excluded clip must remain clear"
                    );
                    assert!(begin(hwnd, std::ptr::null(), 160, 160, 160, 0, None));
                    background(Some(&artwork), 255, 0, 0, 100, 0, None, None);
                    assert!(finish());
                    assert_eq!(
                        snapshot(RECT {
                            left: 0,
                            top: 0,
                            right: 1,
                            bottom: 1
                        })
                        .unwrap()[0],
                        0xff112233
                    );
                    assert!(begin(hwnd, std::ptr::null(), 160, 160, 160, 0, None));
                    background(Some(&artwork), 0, 0xff224466, 0, 100, 0, None, None);
                    assert!(finish());
                    assert_eq!(
                        snapshot(RECT {
                            left: 0,
                            top: 0,
                            right: 1,
                            bottom: 1
                        })
                        .unwrap()[0],
                        0xff224466
                    );
                    assert!(begin(hwnd, std::ptr::null(), 160, 160, 160, 0, None));
                    SURFACE.with(|s| s.borrow_mut().as_mut().unwrap().failed = true);
                    assert!(
                        !finish(),
                        "failed device frame must discard hardware resources"
                    );
                    assert!(snapshot(RECT {
                        left: 0,
                        top: 0,
                        right: 1,
                        bottom: 1
                    })
                    .is_none());
                    assert!(
                        !begin(hwnd, std::ptr::null(), 160, 160, 160, 0, None),
                        "failed hardware must back off"
                    );
                    hide();
                    DeleteObject(font);
                    SelectObject(dc, old_bitmap);
                    DeleteObject(dc_bitmap);
                    DeleteDC(dc);
                    DestroyWindow(hwnd);
                }
            }
        }
    }
    mod gpu_dispatch {
        //! Keep GDI+ geometry/layout, dispatch raster operations to Direct2D.
        #![allow(non_snake_case)]
        use super::{gpu, *};
        use windows_sys::Win32::Graphics::GdiPlus as gp;
        pub(super) unsafe fn GdipDeletePath(path: *mut GpPath) -> Status {
            gpu::forget_path(path as usize);
            gp::GdipDeletePath(path)
        }
        thread_local! { static ANGLES: std::cell::RefCell<std::collections::HashMap<usize, f32>> = std::cell::RefCell::new(std::collections::HashMap::new()); }

        pub(super) unsafe fn GdipCreateLineBrushFromRectWithAngle(
            r: *const RectF,
            a: u32,
            b: u32,
            angle: f32,
            scalable: i32,
            wrap: WrapMode,
            brush: *mut *mut GpLineGradient,
        ) -> Status {
            let status =
                gp::GdipCreateLineBrushFromRectWithAngle(r, a, b, angle, scalable, wrap, brush);
            if status == 0 {
                ANGLES.with(|m| m.borrow_mut().insert(*brush as usize, angle));
            }
            status
        }
        pub(super) unsafe fn GdipDeleteBrush(brush: *mut GpBrush) -> Status {
            let _ = ANGLES.try_with(|m| m.borrow_mut().remove(&(brush as usize)));
            gp::GdipDeleteBrush(brush)
        }
        unsafe fn painted(
            g: *mut GpGraphics,
            brush: *mut GpBrush,
            path: *mut GpPath,
            stroke: Option<f32>,
        ) -> bool {
            gpu::draw(g, |s| {
                let mut kind = 0;
                gp::GdipGetBrushType(brush, &mut kind);
                if kind == BrushTypeLinearGradient {
                    let mut colors = [0u32; 2];
                    let mut rect = RectF::default();
                    gp::GdipGetLineColors(brush.cast(), colors.as_mut_ptr());
                    gp::GdipGetLineRect(brush.cast(), &mut rect);
                    let angle =
                        ANGLES.with(|m| m.borrow().get(&(brush as usize)).copied().unwrap_or(90.));
                    s.gradient(path, colors, rect, angle, stroke)
                } else {
                    let mut color = 0;
                    gp::GdipGetSolidFillColor(brush.cast(), &mut color);
                    s.solid(path, color, stroke)
                }
            })
        }
        pub(super) unsafe fn GdipFillPath(
            g: *mut GpGraphics,
            brush: *mut GpBrush,
            path: *mut GpPath,
        ) -> Status {
            if painted(g, brush, path, None) {
                0
            } else {
                gp::GdipFillPath(g, brush, path)
            }
        }
        pub(super) unsafe fn GdipDrawPath(
            g: *mut GpGraphics,
            pen: *mut GpPen,
            path: *mut GpPath,
        ) -> Status {
            if gpu::active() && is_current_graphics(g) {
                let mut brush = null_mut();
                let mut width = 1.;
                gp::GdipGetPenWidth(pen, &mut width);
                gp::GdipGetPenBrushFill(pen, &mut brush);
                painted(g, brush, path, Some(width));
                gp::GdipDeleteBrush(brush);
                0
            } else {
                gp::GdipDrawPath(g, pen, path)
            }
        }
        unsafe fn shape(
            g: *mut GpGraphics,
            pen: *mut GpPen,
            brush: *mut GpBrush,
            build: impl FnOnce(*mut GpPath),
        ) -> bool {
            if !gpu::active() || !is_current_graphics(g) {
                return false;
            }
            let mut path = null_mut();
            gp::GdipCreatePath(FillModeAlternate, &mut path);
            build(path);
            if !pen.is_null() {
                GdipDrawPath(g, pen, path);
            } else {
                GdipFillPath(g, brush, path);
            }
            GdipDeletePath(path);
            true
        }
        pub(super) unsafe fn GdipFillEllipse(
            g: *mut GpGraphics,
            b: *mut GpBrush,
            x: f32,
            y: f32,
            w: f32,
            h: f32,
        ) -> Status {
            if shape(g, null_mut(), b, |p| {
                gp::GdipAddPathEllipse(p, x, y, w, h);
            }) {
                0
            } else {
                gp::GdipFillEllipse(g, b, x, y, w, h)
            }
        }
        pub(super) unsafe fn GdipDrawEllipse(
            g: *mut GpGraphics,
            p: *mut GpPen,
            x: f32,
            y: f32,
            w: f32,
            h: f32,
        ) -> Status {
            if shape(g, p, null_mut(), |p| {
                gp::GdipAddPathEllipse(p, x, y, w, h);
            }) {
                0
            } else {
                gp::GdipDrawEllipse(g, p, x, y, w, h)
            }
        }
        pub(super) unsafe fn GdipDrawArc(
            g: *mut GpGraphics,
            p: *mut GpPen,
            x: f32,
            y: f32,
            w: f32,
            h: f32,
            a: f32,
            b: f32,
        ) -> Status {
            if shape(g, p, null_mut(), |p| {
                gp::GdipAddPathArc(p, x, y, w, h, a, b);
            }) {
                0
            } else {
                gp::GdipDrawArc(g, p, x, y, w, h, a, b)
            }
        }
        pub(super) unsafe fn GdipDrawLine(
            g: *mut GpGraphics,
            p: *mut GpPen,
            x: f32,
            y: f32,
            u: f32,
            v: f32,
        ) -> Status {
            if shape(g, p, null_mut(), |p| {
                gp::GdipAddPathLine(p, x, y, u, v);
            }) {
                0
            } else {
                gp::GdipDrawLine(g, p, x, y, u, v)
            }
        }
        pub(super) unsafe fn GdipDrawBezier(
            g: *mut GpGraphics,
            p: *mut GpPen,
            x: f32,
            y: f32,
            a: f32,
            b: f32,
            c: f32,
            d: f32,
            u: f32,
            v: f32,
        ) -> Status {
            if shape(g, p, null_mut(), |p| {
                gp::GdipAddPathBezier(p, x, y, a, b, c, d, u, v);
            }) {
                0
            } else {
                gp::GdipDrawBezier(g, p, x, y, a, b, c, d, u, v)
            }
        }
        pub(super) unsafe fn GdipDrawCurve(
            g: *mut GpGraphics,
            p: *mut GpPen,
            points: *const PointF,
            count: i32,
        ) -> Status {
            GdipDrawCurve2(g, p, points, count, 0.5)
        }
        pub(super) unsafe fn GdipDrawCurve2(
            g: *mut GpGraphics,
            p: *mut GpPen,
            points: *const PointF,
            count: i32,
            tension: f32,
        ) -> Status {
            if shape(g, p, null_mut(), |p| {
                gp::GdipAddPathCurve2(p, points, count, tension);
            }) {
                0
            } else {
                gp::GdipDrawCurve2(g, p, points, count, tension)
            }
        }
        pub(super) unsafe fn GdipFillPolygon(
            g: *mut GpGraphics,
            b: *mut GpBrush,
            points: *const PointF,
            count: i32,
            mode: FillMode,
        ) -> Status {
            if shape(g, null_mut(), b, |p| {
                gp::GdipSetPathFillMode(p, mode);
                gp::GdipAddPathPolygon(p, points, count);
            }) {
                0
            } else {
                gp::GdipFillPolygon(g, b, points, count, mode)
            }
        }
        pub(super) unsafe fn GdipDrawImageRectI(
            g: *mut GpGraphics,
            image: *mut GpImage,
            x: i32,
            y: i32,
            w: i32,
            h: i32,
        ) -> Status {
            if !gpu::active() || !is_current_graphics(g) {
                return gp::GdipDrawImageRectI(g, image, x, y, w, h);
            }
            let mut width = 0;
            let mut height = 0;
            gp::GdipGetImageWidth(image, &mut width);
            gp::GdipGetImageHeight(image, &mut height);
            let rect = Rect {
                X: 0,
                Y: 0,
                Width: width as i32,
                Height: height as i32,
            };
            let mut data = BitmapData::default();
            if gp::GdipBitmapLockBits(
                image.cast(),
                &rect,
                ImageLockModeRead as u32,
                0xe200b,
                &mut data,
            ) != 0
            {
                return 1;
            }
            gpu::draw(g, |s| {
                s.image(
                    (image as u64) << 1,
                    data.Scan0,
                    width,
                    height,
                    data.Stride as u32,
                    RECT {
                        left: x,
                        top: y,
                        right: x + w,
                        bottom: y + h,
                    },
                    0.,
                    255,
                )
            });
            gp::GdipBitmapUnlockBits(image.cast(), &mut data);
            0
        }
        pub(super) unsafe fn GdipDisposeImage(image: *mut GpImage) -> Status {
            gpu::forget_image((image as u64) << 1);
            gp::GdipDisposeImage(image)
        }
    }
    mod provider_logos {
        // Provider logo outlines from Simple Icons (CC0), rendered as native Bezier paths.
        pub const DEEZER: &[&[[f32; 8]]] = &[
            &[
                [
                    0.693000, 10.024000, 1.074000, 10.024000, 1.386000, 8.768000, 1.386000,
                    7.217000,
                ],
                [
                    1.386000, 7.217000, 1.386000, 5.667000, 1.074000, 4.410000, 0.693000, 4.410000,
                ],
                [
                    0.693000, 4.410000, 0.312000, 4.410000, 0.000000, 5.666000, 0.000000, 7.217000,
                ],
                [
                    0.000000, 7.217000, 0.000000, 8.768000, 0.312000, 10.025000, 0.693000,
                    10.025000,
                ],
            ],
            &[
                [
                    21.038000, 1.560000, 20.674000, 1.560000, 20.354000, 2.365000, 20.128000,
                    3.656000,
                ],
                [
                    20.128000, 3.656000, 19.765000, 1.446000, 19.184000, 0.000000, 18.526000,
                    0.000000,
                ],
                [
                    18.526000, 0.000000, 17.746000, 0.000000, 17.062000, 2.036000, 16.742000,
                    5.000000,
                ],
                [
                    16.742000, 5.000000, 16.430000, 2.842000, 15.954000, 1.464000, 15.417000,
                    1.464000,
                ],
                [
                    15.417000, 1.464000, 14.672000, 1.464000, 14.031000, 4.168000, 13.797000,
                    7.936000,
                ],
                [
                    13.797000, 7.936000, 13.355000, 6.004000, 12.714000, 4.791000, 12.004000,
                    4.791000,
                ],
                [
                    12.004000, 4.791000, 11.294000, 4.791000, 10.654000, 6.004000, 10.211000,
                    7.936000,
                ],
                [
                    10.211000, 7.936000, 9.969000, 4.176000, 9.337000, 1.473000, 8.583000, 1.473000,
                ],
                [
                    8.583000, 1.473000, 8.046000, 1.473000, 7.570000, 2.851000, 7.258000, 5.008000,
                ],
                [
                    7.258000, 5.008000, 6.938000, 2.036000, 6.262000, 0.000000, 5.474000, 0.000000,
                ],
                [
                    5.474000, 0.000000, 4.816000, 0.000000, 4.227000, 1.447000, 3.872000, 3.665000,
                ],
                [
                    3.872000, 3.665000, 3.655000, 2.374000, 3.326000, 1.560000, 2.962000, 1.560000,
                ],
                [
                    2.962000, 1.560000, 2.287000, 1.560000, 1.741000, 4.367000, 1.741000, 7.832000,
                ],
                [
                    1.741000, 7.832000, 1.741000, 11.298000, 2.287000, 14.105000, 2.962000,
                    14.105000,
                ],
                [
                    2.962000, 14.105000, 3.239000, 14.105000, 3.499000, 13.629000, 3.698000,
                    12.832000,
                ],
                [
                    3.698000, 12.832000, 4.018000, 15.760000, 4.694000, 17.770000, 5.474000,
                    17.770000,
                ],
                [
                    5.474000, 17.770000, 6.080000, 17.770000, 6.617000, 16.566000, 6.981000,
                    14.660000,
                ],
                [
                    6.981000, 14.660000, 7.232000, 18.282000, 7.856000, 20.855000, 8.583000,
                    20.855000,
                ],
                [
                    8.583000, 20.855000, 9.043000, 20.855000, 9.458000, 19.832000, 9.770000,
                    18.178000,
                ],
                [
                    9.770000, 18.178000, 10.142000, 21.600000, 11.000000, 24.000000, 12.004000,
                    24.000000,
                ],
                [
                    12.004000, 24.000000, 13.009000, 24.000000, 13.867000, 21.600000, 14.239000,
                    18.178000,
                ],
                [
                    14.239000, 18.178000, 14.551000, 19.832000, 14.966000, 20.855000, 15.425000,
                    20.855000,
                ],
                [
                    15.425000, 20.855000, 16.153000, 20.855000, 16.777000, 18.282000, 17.028000,
                    14.660000,
                ],
                [
                    17.028000, 14.660000, 17.392000, 16.566000, 17.928000, 17.770000, 18.535000,
                    17.770000,
                ],
                [
                    18.535000, 17.770000, 19.315000, 17.770000, 19.990000, 15.760000, 20.310000,
                    12.832000,
                ],
                [
                    20.310000, 12.832000, 20.518000, 13.629000, 20.770000, 14.105000, 21.047000,
                    14.105000,
                ],
                [
                    21.047000, 14.105000, 21.722000, 14.105000, 22.267000, 11.298000, 22.267000,
                    7.832000,
                ],
                [
                    22.267000, 7.832000, 22.259000, 4.375000, 21.714000, 1.560000, 21.037000,
                    1.560000,
                ],
            ],
            &[
                [
                    23.307000, 10.024000, 23.688000, 10.024000, 24.000000, 8.768000, 24.000000,
                    7.217000,
                ],
                [
                    24.000000, 7.217000, 24.000000, 5.667000, 23.688000, 4.410000, 23.307000,
                    4.410000,
                ],
                [
                    23.307000, 4.410000, 22.926000, 4.410000, 22.614000, 5.666000, 22.614000,
                    7.217000,
                ],
                [
                    22.614000, 7.217000, 22.614000, 8.768000, 22.926000, 10.025000, 23.307000,
                    10.025000,
                ],
            ],
        ];
    }
    pub unsafe fn spotify_logo(dc: HDC, r: RECT) {
        let Some(g) = graphics(dc) else {
            return;
        };
        let size = (r.right - r.left).min(r.bottom - r.top).max(0) as f32;
        let x = r.left as f32 + ((r.right - r.left) as f32 - size) / 2.0;
        let y = r.top as f32 + ((r.bottom - r.top) as f32 - size) / 2.0;
        let mut brush = null_mut();
        GdipCreateSolidFill(0xff1ed760, &mut brush);
        gpu_dispatch::GdipFillEllipse(g, brush.cast(), x, y, size, size);
        gpu_dispatch::GdipDeleteBrush(brush.cast());
        let mut pen = null_mut();
        GdipCreatePen1(0xff121212, size * 0.065, UnitPixel, &mut pen);
        GdipSetPenStartCap(pen, LineCapRound);
        GdipSetPenEndCap(pen, LineCapRound);
        for (left, top, right, bottom) in [
            (0.25, 0.36, 0.76, 0.43),
            (0.29, 0.49, 0.70, 0.55),
            (0.33, 0.61, 0.65, 0.66),
        ] {
            gpu_dispatch::GdipDrawBezier(
                g,
                pen,
                x + size * left,
                y + size * top,
                x + size * 0.42,
                y + size * (top - 0.06),
                x + size * 0.62,
                y + size * (top - 0.03),
                x + size * right,
                y + size * bottom,
            );
        }
        GdipDeletePen(pen);
    }
    pub unsafe fn youtube_logo(dc: HDC, r: RECT) {
        let Some(g) = graphics(dc) else {
            return;
        };
        let size = (r.right - r.left).min(r.bottom - r.top).max(0) as f32;
        let x = r.left as f32 + ((r.right - r.left) as f32 - size) / 2.0;
        let y = r.top as f32 + ((r.bottom - r.top) as f32 - size) / 2.0;
        let mut brush = null_mut();
        GdipCreateSolidFill(0xffff0033, &mut brush);
        gpu_dispatch::GdipFillEllipse(g, brush.cast(), x, y, size, size);
        gpu_dispatch::GdipDeleteBrush(brush.cast());
        polygon(
            dc,
            &[
                (x + size * 0.40, y + size * 0.29),
                (x + size * 0.40, y + size * 0.71),
                (x + size * 0.71, y + size * 0.50),
            ],
            0xffffffff,
        );
    }

    pub unsafe fn deezer_logo(dc: HDC, r: RECT) -> bool {
        let Some(g) = graphics(dc) else {
            return false;
        };
        let size = (r.right - r.left).min(r.bottom - r.top).max(0) as f32;
        let x = r.left as f32 + ((r.right - r.left) as f32 - size) / 2.0;
        let y = r.top as f32 + ((r.bottom - r.top) as f32 - size) / 2.0;
        let scale = size / 24.0;
        let mut path = null_mut();
        GdipCreatePath(FillModeAlternate, &mut path);
        let (figures, color) = (provider_logos::DEEZER, 0xffa238ff);
        for figure in figures {
            GdipStartPathFigure(path);
            for p in *figure {
                GdipAddPathBezier(
                    path,
                    x + p[0] * scale,
                    y + p[1] * scale,
                    x + p[2] * scale,
                    y + p[3] * scale,
                    x + p[4] * scale,
                    y + p[5] * scale,
                    x + p[6] * scale,
                    y + p[7] * scale,
                );
            }
            GdipClosePathFigure(path);
        }
        let mut brush = null_mut();
        GdipCreateSolidFill(color, &mut brush);
        gpu_dispatch::GdipFillPath(g, brush.cast(), path);
        gpu_dispatch::GdipDeleteBrush(brush.cast());
        gpu_dispatch::GdipDeletePath(path);
        true
    }
    type CachedImage = (ImageKey, *mut GpBitmap);
    const RESIZED_IMAGE_BYTES: usize = 8 * 1024 * 1024;
    fn resized_bytes(key: ImageKey) -> usize {
        (key.1.max(0) as usize)
            .saturating_mul(key.2.max(0) as usize)
            .saturating_mul(4)
    }
    thread_local! {static RESIZED_IMAGES:std::cell::RefCell<std::collections::VecDeque<CachedImage>>=const {std::cell::RefCell::new(std::collections::VecDeque::new())};}
    pub(crate) fn resized_artwork_stats() -> (usize, usize) {
        RESIZED_IMAGES.with(|cache| {
            let cache = cache.borrow();
            (
                cache.len(),
                cache.iter().map(|(key, _)| resized_bytes(*key)).sum(),
            )
        })
    }
    pub(crate) fn evict_resized_artwork(ids: &std::collections::HashSet<u64>) {
        if ids.is_empty() {
            return;
        }
        ARTWORK_PREPARATION.with(|state| {
            if let Some(state) = state.borrow_mut().as_mut() {
                state.pending.retain(|key| !ids.contains(&key.0));
            }
        });
        RESIZED_IMAGES.with(|cache| {
            let mut cache = cache.borrow_mut();
            cache.retain(|(key, bitmap)| {
                if ids.contains(&key.0) {
                    unsafe {
                        gpu_dispatch::GdipDisposeImage((*bitmap).cast());
                    }
                    false
                } else {
                    true
                }
            });
        });
    }
    thread_local! {static CURRENT:Cell<(HDC,*mut GpGraphics)>=const {Cell::new((null_mut(),null_mut()))};}
    fn is_current_graphics(g: *mut GpGraphics) -> bool {
        CURRENT.get().1 == g && !g.is_null()
    }
    thread_local! {static CURRENT_CLIP:Cell<Option<(i32,i32,i32,i32)>>=const {Cell::new(None)};}
    thread_local! {static COMPLEX_CLIP:Cell<bool>=const {Cell::new(false)};}
    thread_local! {static GPU_DC_CLIPS:std::cell::RefCell<Vec<RECT>>=const {std::cell::RefCell::new(Vec::new())};}
    struct DcRegions {
        current: HRGN,
        scratch: HRGN,
    }
    impl Drop for DcRegions {
        fn drop(&mut self) {
            unsafe {
                DeleteObject(self.current);
                DeleteObject(self.scratch);
            }
        }
    }
    thread_local! {static DC_REGIONS:std::cell::RefCell<DcRegions>=std::cell::RefCell::new(DcRegions {
        current:unsafe{CreateRectRgn(0,0,0,0)},scratch:unsafe{CreateRectRgn(0,0,0,0)}
    });}
    thread_local! {static TEXT_PATHS:std::cell::RefCell<std::collections::VecDeque<(String,*mut GpPath)>>=const {std::cell::RefCell::new(std::collections::VecDeque::new())};}
    thread_local! {static LIGHT:Cell<bool>=const {Cell::new(false)};}
    thread_local! {static FRAME_FONTS:std::cell::RefCell<std::collections::HashMap<usize,LOGFONTW>>=std::cell::RefCell::new(std::collections::HashMap::new());}
    unsafe fn font_properties(font: HFONT) -> LOGFONTW {
        let read = || {
            let mut lf: LOGFONTW = zeroed();
            GetObjectW(
                font,
                size_of::<LOGFONTW>() as i32,
                (&mut lf as *mut LOGFONTW).cast(),
            );
            lf
        };
        if CURRENT.get().1.is_null() {
            return read();
        }
        FRAME_FONTS.with(|fonts| *fonts.borrow_mut().entry(font as usize).or_insert_with(read))
    }
    pub fn argb(color: u32, alpha: u32) -> u32 {
        (alpha << 24) | ((color & 255) << 16) | (color & 0xff00) | ((color >> 16) & 255)
    }
    thread_local! {static SURFACE_OPACITY:Cell<u32>=const {Cell::new(100)};}
    pub struct SurfaceOpacity(u32);
    impl SurfaceOpacity {
        pub fn set(percent: u32) -> Self {
            Self(SURFACE_OPACITY.with(|value| value.replace(percent.min(100))))
        }
    }
    impl Drop for SurfaceOpacity {
        fn drop(&mut self) {
            SURFACE_OPACITY.with(|value| value.set(self.0));
        }
    }
    fn surface_color(color: u32) -> u32 {
        (color & 0xffffff) | ((((color >> 24) * SURFACE_OPACITY.with(Cell::get) + 50) / 100) << 24)
    }
    pub struct Frame {
        image: *mut GpBitmap,
        graphics: *mut GpGraphics,
    }
    #[derive(Clone, Copy)]
    struct FramePixels {
        bits: *mut u32,
        stride: usize,
        width: i32,
        height: i32,
    }
    thread_local! { static FRAME_PIXELS: Cell<Option<FramePixels>> = const { Cell::new(None) }; }

    // All rasters and the destination use the same premultiplied BGRA format.
    // Flush GDI+ before writing and intersect the current DC clip with the frame.
    unsafe fn composite_raster(
        g: *mut GpGraphics,
        pixels: &[u32],
        width: i32,
        height: i32,
        x: i32,
        y: i32,
    ) -> bool {
        // The direct pixel blender only supports rectangular clips. GDI+ handles
        // disjoint update regions without painting into the gaps between them.
        if COMPLEX_CLIP.get() {
            return false;
        }
        let Some(frame) = FRAME_PIXELS.get() else {
            return false;
        };
        let Some((clip_left, clip_top, clip_right, clip_bottom)) = CURRENT_CLIP.get() else {
            return false;
        };
        if width <= 0 || height <= 0 || pixels.len() < width as usize * height as usize {
            return false;
        }
        let left = x.max(clip_left).max(0);
        let top = y.max(clip_top).max(0);
        let right = x.saturating_add(width).min(clip_right).min(frame.width);
        let bottom = y.saturating_add(height).min(clip_bottom).min(frame.height);
        if left >= right || top >= bottom {
            return true;
        }
        GdipFlush(g, FlushIntentionSync);
        let count = (right - left) as usize;
        for row in top..bottom {
            let start = (row - y) as usize * width as usize + (left - x) as usize;
            let destination = std::slice::from_raw_parts_mut(
                frame.bits.add(row as usize * frame.stride + left as usize),
                count,
            );
            composite::blend_row(destination, &pixels[start..start + count]);
        }
        true
    }
    impl Frame {
        pub unsafe fn begin(
            dc: HDC,
            bits: *mut u8,
            stride: i32,
            width: i32,
            height: i32,
            light: bool,
        ) -> Option<Self> {
            static TOKEN: OnceLock<usize> = OnceLock::new();
            if *TOKEN.get_or_init(|| {
                let mut token = 0;
                let input = GdiplusStartupInput {
                    GdiplusVersion: 1,
                    ..zeroed()
                };
                if GdiplusStartup(&mut token, &input, null_mut()) == 0 {
                    token
                } else {
                    0
                }
            }) == 0
            {
                return None;
            }
            let mut image = null_mut();
            let mut graphics = null_mut();
            // PixelFormat32bppPARGB = 0xE200B.
            if GdipCreateBitmapFromScan0(width, height, stride * 4, 0xe200b, bits, &mut image) != 0
            {
                return None;
            }
            if GdipGetImageGraphicsContext(image.cast(), &mut graphics) != 0 {
                gpu_dispatch::GdipDisposeImage(image.cast());
                return None;
            }
            GdipSetSmoothingMode(graphics, SmoothingModeAntiAlias);
            GdipSetInterpolationMode(graphics, InterpolationModeHighQualityBicubic);
            GdipSetPixelOffsetMode(graphics, PixelOffsetModeHighQuality);
            FRAME_FONTS.with(|fonts| fonts.borrow_mut().clear());
            CURRENT.set((dc, graphics));
            FRAME_PIXELS.set(
                (!bits.is_null() && stride >= width && width > 0 && height > 0).then_some(
                    FramePixels {
                        bits: bits.cast(),
                        stride: stride as usize,
                        width,
                        height,
                    },
                ),
            );
            CURRENT_CLIP.set(None);
            COMPLEX_CLIP.set(false);
            LIGHT.set(light);
            Some(Self { image, graphics })
        }
    }
    impl Drop for Frame {
        fn drop(&mut self) {
            unsafe {
                CURRENT.set((null_mut(), null_mut()));
                FRAME_PIXELS.set(None);
                GdipFlush(self.graphics, FlushIntentionSync);
                GdipDeleteGraphics(self.graphics);
                gpu_dispatch::GdipDisposeImage(self.image.cast());
            }
        }
    }
    unsafe fn graphics(dc: HDC) -> Option<*mut GpGraphics> {
        let (target, g) = CURRENT.get();
        if target != dc || g.is_null() {
            return None;
        }
        let mut clip: RECT = zeroed();
        let complex = GetClipBox(dc, &mut clip) == COMPLEXREGION;
        let key = (clip.left, clip.top, clip.right, clip.bottom);
        let was_complex = COMPLEX_CLIP.replace(complex);
        if complex {
            DC_REGIONS.with(|regions| {
                let regions = regions.borrow_mut();
                let has_region = GetClipRgn(dc, regions.scratch) == 1;
                if has_region && gpu::active() {
                    let mut window: windows_sys::Win32::Foundation::POINT = zeroed();
                    let mut viewport = zeroed();
                    GetWindowOrgEx(dc, &mut window);
                    GetViewportOrgEx(dc, &mut viewport);
                    OffsetRgn(
                        regions.scratch,
                        window.x - viewport.x,
                        window.y - viewport.y,
                    );
                }
                if has_region
                    && (!was_complex
                        || CURRENT_CLIP.get() != Some(key)
                        || EqualRgn(regions.current, regions.scratch) == 0)
                {
                    GdipSetClipHrgn(g, regions.scratch, CombineModeReplace);
                    if gpu::active() {
                        GPU_DC_CLIPS.with(|clips| {
                            *clips.borrow_mut() =
                                gpu::region_rects(regions.scratch).unwrap_or_default()
                        });
                    }
                    CombineRgn(regions.current, regions.scratch, null_mut(), RGN_COPY);
                }
            });
            CURRENT_CLIP.set(Some(key));
        } else if was_complex || CURRENT_CLIP.get() != Some(key) {
            GPU_DC_CLIPS.with(|clips| {
                let mut clips = clips.borrow_mut();
                clips.clear();
                clips.push(clip);
            });
            GdipSetClipRectI(
                g,
                clip.left,
                clip.top,
                clip.right - clip.left,
                clip.bottom - clip.top,
                CombineModeReplace,
            );
            CURRENT_CLIP.set(Some(key));
        }
        Some(g)
    }
    unsafe fn path(r: RECT, radius: f32) -> *mut GpPath {
        let mut p = null_mut();
        GdipCreatePath(FillModeAlternate, &mut p);
        let x = r.left as f32;
        let y = r.top as f32;
        let w = (r.right - r.left).max(1) as f32;
        let h = (r.bottom - r.top).max(1) as f32;
        if radius <= 0.0 {
            GdipAddPathRectangle(p, x, y, w, h);
            GdipClosePathFigure(p);
            return p;
        }
        let d = (radius * 2.0).min(w).min(h).max(1.0);
        for (xx, yy, angle) in [
            (x, y, 180.0),
            (x + w - d, y, 270.0),
            (x + w - d, y + h - d, 0.0),
            (x, y + h - d, 90.0),
        ] {
            GdipAddPathArc(p, xx, yy, d, d, angle, 90.0);
        }
        GdipClosePathFigure(p);
        p
    }
    type SurfaceKey = (i32, i32, u32, u32, u32);
    const SURFACE_CACHE_BYTES: usize = 16 * 1024 * 1024;
    thread_local! {
        static RASTER_SURFACES: std::cell::RefCell<std::collections::VecDeque<(SurfaceKey, BlurredText)>> = const {std::cell::RefCell::new(std::collections::VecDeque::new())};
    }

    // Rounded panels and button borders retain their geometry while scrolling.
    // Cache their antialiased pixels instead of rasterizing paths every frame.
    unsafe fn raster_surface(
        g: *mut GpGraphics,
        r: RECT,
        color: u32,
        radius: f32,
        stroke: f32,
    ) -> bool {
        if gpu::active() && is_current_graphics(g) {
            return false;
        }
        let pad = if stroke > 0.0 {
            stroke.ceil() as i32 + 1
        } else {
            0
        };
        let width = r.right - r.left;
        let height = r.bottom - r.top;
        let bitmap_width = width.saturating_add(pad * 2);
        let bitmap_height = height.saturating_add(pad * 2);
        let bytes = (bitmap_width.max(0) as usize)
            .saturating_mul(bitmap_height.max(0) as usize)
            .saturating_mul(4);
        if width <= 0 || height <= 0 || bytes > SURFACE_CACHE_BYTES / 2 {
            return false;
        }
        let key = (width, height, color, radius.to_bits(), stroke.to_bits());
        RASTER_SURFACES.with(|cache| {
            let mut cache = cache.borrow_mut();
            let entry = if let Some(index) = cache.iter().position(|(old, _)| *old == key) {
                cache.remove(index).unwrap()
            } else {
                let mut pixels = vec![0u32; bytes / 4];
                let mut bitmap = null_mut();
                if GdipCreateBitmapFromScan0(
                    bitmap_width,
                    bitmap_height,
                    bitmap_width * 4,
                    0xe200b,
                    pixels.as_mut_ptr().cast(),
                    &mut bitmap,
                ) != 0
                {
                    return false;
                }
                let mut canvas = null_mut();
                if GdipGetImageGraphicsContext(bitmap.cast(), &mut canvas) != 0 {
                    gpu_dispatch::GdipDisposeImage(bitmap.cast());
                    return false;
                }
                GdipSetSmoothingMode(canvas, SmoothingModeAntiAlias);
                GdipSetPixelOffsetMode(canvas, PixelOffsetModeHighQuality);
                let shape = path(
                    RECT {
                        left: pad,
                        top: pad,
                        right: pad + width,
                        bottom: pad + height,
                    },
                    radius,
                );
                if stroke > 0.0 {
                    let mut pen = null_mut();
                    GdipCreatePen1(color, stroke, UnitPixel, &mut pen);
                    gpu_dispatch::GdipDrawPath(canvas, pen, shape);
                    GdipDeletePen(pen);
                } else {
                    let mut brush = null_mut();
                    GdipCreateSolidFill(color, &mut brush);
                    gpu_dispatch::GdipFillPath(canvas, brush.cast(), shape);
                    gpu_dispatch::GdipDeleteBrush(brush.cast());
                }
                gpu_dispatch::GdipDeletePath(shape);
                GdipDeleteGraphics(canvas);
                let mut resident: usize =
                    cache.iter().map(|(_, image)| image.pixels.len() * 4).sum();
                while cache.len() >= 128 || resident + bytes > SURFACE_CACHE_BYTES {
                    if let Some((_, old)) = cache.pop_front() {
                        resident = resident.saturating_sub(old.pixels.len() * 4);
                    } else {
                        break;
                    }
                }
                (key, BlurredText { bitmap, pixels })
            };
            if cfg!(debug_assertions)
                || !composite_raster(
                    g,
                    &entry.1.pixels,
                    bitmap_width,
                    bitmap_height,
                    r.left - pad,
                    r.top - pad,
                )
            {
                gpu_dispatch::GdipDrawImageRectI(
                    g,
                    entry.1.bitmap.cast(),
                    r.left - pad,
                    r.top - pad,
                    bitmap_width,
                    bitmap_height,
                );
            }
            cache.push_back(entry);
            true
        })
    }

    pub unsafe fn fill(dc: HDC, r: RECT, color: u32, radius: f32) -> bool {
        let color = surface_color(color);
        let Some(g) = graphics(dc) else {
            return false;
        };
        if RectVisible(dc, &r) == 0 {
            return true;
        }
        if r.right <= r.left || r.bottom <= r.top {
            return true;
        }
        if gpu::draw(g, |s| s.rounded(r, color, radius, None)) {
            return true;
        }
        if raster_surface(g, r, color, radius, 0.0) {
            return true;
        }
        let p = path(r, radius);
        let mut b = null_mut();
        GdipCreateSolidFill(color, &mut b);
        gpu_dispatch::GdipFillPath(g, b.cast(), p);
        gpu_dispatch::GdipDeleteBrush(b.cast());
        gpu_dispatch::GdipDeletePath(p);
        true
    }
    /// Keep scrollbar movement fractional instead of rounding it to layout pixels.
    pub unsafe fn fill_offset_y(dc: HDC, r: RECT, color: u32, radius: f32, y: f32) -> bool {
        let Some(g) = graphics(dc) else {
            return false;
        };
        let color = surface_color(color);
        if gpu::draw(g, |s| s.rounded_offset(r, color, radius, None, y)) {
            return true;
        }
        let p = path(r, radius);
        let mut matrix = null_mut();
        GdipCreateMatrix2(1., 0., 0., 1., 0., y, &mut matrix);
        GdipTransformPath(p, matrix);
        GdipDeleteMatrix(matrix);
        let mut brush = null_mut();
        GdipCreateSolidFill(color, &mut brush);
        gpu_dispatch::GdipFillPath(g, brush.cast(), p);
        gpu_dispatch::GdipDeleteBrush(brush.cast());
        gpu_dispatch::GdipDeletePath(p);
        true
    }
    pub unsafe fn stroke(dc: HDC, r: RECT, color: u32, radius: f32, width: f32) -> bool {
        let color = surface_color(color);
        let Some(g) = graphics(dc) else {
            return false;
        };
        if r.right <= r.left || r.bottom <= r.top {
            return true;
        }
        if gpu::draw(g, |s| s.rounded(r, color, radius, Some(width))) {
            return true;
        }
        if width > 0.0 && raster_surface(g, r, color, radius, width) {
            return true;
        }
        let p = path(r, radius);
        let mut pen = null_mut();
        GdipCreatePen1(color, width, UnitPixel, &mut pen);
        gpu_dispatch::GdipDrawPath(g, pen, p);
        GdipDeletePen(pen);
        gpu_dispatch::GdipDeletePath(p);
        true
    }
    pub unsafe fn stroke_gradient(
        dc: HDC,
        r: RECT,
        top: u32,
        bottom: u32,
        radius: f32,
        width: f32,
    ) -> bool {
        let (top, bottom) = (surface_color(top), surface_color(bottom));
        let Some(g) = graphics(dc) else {
            return false;
        };
        if r.right <= r.left || r.bottom <= r.top {
            return true;
        }
        let p = path(r, radius);
        let mut brush = null_mut();
        let bounds = RectF {
            X: r.left as f32,
            Y: r.top as f32,
            Width: (r.right - r.left) as f32,
            Height: (r.bottom - r.top) as f32,
        };
        gpu_dispatch::GdipCreateLineBrushFromRectWithAngle(
            &bounds,
            top,
            bottom,
            90.0,
            0,
            WrapModeTileFlipXY,
            &mut brush,
        );
        let mut pen = null_mut();
        GdipCreatePen2(brush.cast(), width, UnitPixel, &mut pen);
        gpu_dispatch::GdipDrawPath(g, pen, p);
        GdipDeletePen(pen);
        gpu_dispatch::GdipDeleteBrush(brush.cast());
        gpu_dispatch::GdipDeletePath(p);
        true
    }
    pub unsafe fn gradient(dc: HDC, r: RECT, top: u32, bottom: u32, radius: f32) -> bool {
        gradient_angle(dc, r, top, bottom, radius, 100.0)
    }
    pub unsafe fn gradient_angle(
        dc: HDC,
        r: RECT,
        top: u32,
        bottom: u32,
        radius: f32,
        angle: f32,
    ) -> bool {
        let (top, bottom) = (surface_color(top), surface_color(bottom));
        let Some(g) = graphics(dc) else {
            return false;
        };
        if r.right <= r.left || r.bottom <= r.top {
            return true;
        }
        let p = path(r, radius);
        let mut brush = null_mut();
        let bounds = RectF {
            X: r.left as f32,
            Y: r.top as f32,
            Width: (r.right - r.left) as f32,
            Height: (r.bottom - r.top) as f32,
        };
        gpu_dispatch::GdipCreateLineBrushFromRectWithAngle(
            &bounds,
            top,
            bottom,
            angle,
            0,
            WrapModeTileFlipXY,
            &mut brush,
        );
        gpu_dispatch::GdipFillPath(g, brush.cast(), p);
        gpu_dispatch::GdipDeleteBrush(brush.cast());
        gpu_dispatch::GdipDeletePath(p);
        true
    }
    pub unsafe fn line(
        dc: HDC,
        x1: f32,
        y1: f32,
        x2: f32,
        y2: f32,
        color: u32,
        width: f32,
    ) -> bool {
        let Some(g) = graphics(dc) else {
            return false;
        };
        if gpu::draw(g, |s| s.line(x1, y1, x2, y2, color, width)) {
            return true;
        }
        let mut pen = null_mut();
        GdipCreatePen1(color, width, UnitPixel, &mut pen);
        gpu_dispatch::GdipDrawLine(g, pen, x1, y1, x2, y2);
        GdipDeletePen(pen);
        true
    }
    pub unsafe fn curve(dc: HDC, points: &[(f32, f32)], color: u32, width: f32) -> bool {
        let Some(g) = graphics(dc) else {
            return false;
        };
        if points.len() < 2 {
            return true;
        }
        let pts: Vec<PointF> = points.iter().map(|&(x, y)| PointF { X: x, Y: y }).collect();
        let mut pen = null_mut();
        GdipCreatePen1(color, width, UnitPixel, &mut pen);
        gpu_dispatch::GdipDrawCurve(g, pen, pts.as_ptr(), pts.len() as i32);
        GdipDeletePen(pen);
        true
    }
    pub unsafe fn polygon(dc: HDC, points: &[(f32, f32)], color: u32) -> bool {
        let Some(g) = graphics(dc) else {
            return false;
        };
        if points.len() < 3 {
            return true;
        }
        let pts: Vec<PointF> = points.iter().map(|&(x, y)| PointF { X: x, Y: y }).collect();
        let mut brush = null_mut();
        GdipCreateSolidFill(color, &mut brush);
        gpu_dispatch::GdipFillPolygon(
            g,
            brush.cast(),
            pts.as_ptr(),
            pts.len() as i32,
            FillModeAlternate,
        );
        gpu_dispatch::GdipDeleteBrush(brush.cast());
        true
    }
    pub unsafe fn profile_silhouette(dc: HDC, r: RECT, color: u32) -> bool {
        let Some(g) = graphics(dc) else {
            return false;
        };
        let w = (r.right - r.left).max(1) as f32;
        let h = (r.bottom - r.top).max(1) as f32;
        let x = r.left as f32;
        let y = r.top as f32;
        let mut pen = null_mut();
        GdipCreatePen1(color, 1.8, UnitPixel, &mut pen);
        gpu_dispatch::GdipDrawEllipse(g, pen, x + w * 0.34, y + h * 0.22, w * 0.32, h * 0.32);
        gpu_dispatch::GdipDrawArc(
            g,
            pen,
            x + w * 0.20,
            y + h * 0.50,
            w * 0.60,
            h * 0.44,
            200.0,
            140.0,
        );
        GdipDeletePen(pen);
        true
    }
    unsafe fn cached_text_path(
        font: HFONT,
        value: &str,
        r: RECT,
        flags: u32,
    ) -> Option<*mut GpPath> {
        let lf = font_properties(font);
        let key = format!(
            "{:p}:{}:{}:{}:{}:{}:{}",
            font,
            lf.lfHeight,
            lf.lfWeight,
            r.right - r.left,
            r.bottom - r.top,
            flags,
            value
        );
        let cached = TEXT_PATHS.with(|cache| {
            let mut cache = cache.borrow_mut();
            let index = cache.iter().position(|(k, _)| *k == key)?;
            let entry = cache.remove(index)?;
            let path = entry.1;
            cache.push_back(entry);
            Some(path)
        });
        if let Some(path) = cached {
            return Some(path);
        }
        let mut family = null_mut();
        GdipCreateFontFamilyFromName(lf.lfFaceName.as_ptr(), null_mut(), &mut family);
        if family.is_null() {
            return None;
        }
        let mut format = null_mut();
        GdipCreateStringFormat(
            if flags & DT_WORDBREAK != 0 {
                0
            } else {
                StringFormatFlagsNoWrap
            },
            0,
            &mut format,
        );
        if flags & LYRICS_UNTRIMMED != 0 {
            GdipDeleteStringFormat(format);
            GdipStringFormatGetGenericTypographic(&mut format);
            GdipSetStringFormatFlags(
                format,
                StringFormatFlagsNoWrap
                    | StringFormatFlagsNoClip
                    | StringFormatFlagsMeasureTrailingSpaces,
            );
        }
        GdipSetStringFormatTrimming(
            format,
            if flags & LYRICS_UNTRIMMED != 0 {
                StringTrimmingNone
            } else {
                StringTrimmingEllipsisCharacter
            },
        );
        GdipSetStringFormatAlign(
            format,
            if flags & DT_CENTER != 0 {
                StringAlignmentCenter
            } else if flags & DT_RIGHT != 0 {
                StringAlignmentFar
            } else {
                StringAlignmentNear
            },
        );
        GdipSetStringFormatLineAlign(
            format,
            if flags & DT_VCENTER != 0 {
                StringAlignmentCenter
            } else {
                StringAlignmentNear
            },
        );
        let bounds = RectF {
            X: 0.0,
            Y: 0.0,
            Width: (r.right - r.left) as f32,
            Height: (r.bottom - r.top) as f32,
        };
        let utf16: Vec<_> = value.encode_utf16().collect();
        let mut p = null_mut();
        GdipCreatePath(FillModeAlternate, &mut p);
        GdipAddPathString(
            p,
            utf16.as_ptr(),
            utf16.len() as i32,
            family,
            if lf.lfWeight >= 600 {
                FontStyleBold
            } else {
                FontStyleRegular
            },
            lf.lfHeight.unsigned_abs() as f32,
            &bounds,
            format,
        );
        GdipDeleteStringFormat(format);
        GdipDeleteFontFamily(family);
        TEXT_PATHS.with(|cache| {
            let mut cache = cache.borrow_mut();
            if cache.len() >= 256 {
                if let Some((_, old)) = cache.pop_front() {
                    gpu_dispatch::GdipDeletePath(old);
                }
            }
            cache.push_back((key, p));
        });
        Some(p)
    }
    pub unsafe fn text(dc: HDC, font: HFONT, value: &str, r: RECT, color: u32, flags: u32) -> bool {
        let Some(g) = graphics(dc) else {
            return false;
        };
        if RectVisible(dc, &r) == 0 {
            return true;
        }
        if value.is_empty() || r.right <= r.left || r.bottom <= r.top {
            return true;
        }
        // Reuse rasterized glyphs while scrolling instead of stroking and filling
        // the same vector paths at every new screen position.
        if !gpu::active() && raster_text(g, font, value, r, color, flags) {
            return true;
        }
        let Some(p) = cached_text_path(font, value, r, flags) else {
            return true;
        };
        let mut state = 0;
        GdipSaveGraphics(g, &mut state);
        GdipSetClipRectI(
            g,
            r.left,
            r.top,
            r.right - r.left,
            r.bottom - r.top,
            CombineModeIntersect,
        );
        GdipTranslateWorldTransform(g, r.left as f32, r.top as f32, MatrixOrderPrepend);
        if gpu::draw(g, |s| {
            s.text(p, argb(color, 255), r.right - r.left, r.bottom - r.top)
        }) {
            GdipRestoreGraphics(g, state);
            return true;
        }
        let _ = LIGHT.get();
        let mut pen = null_mut();
        GdipCreatePen1(argb(color, 48), 0.3, UnitPixel, &mut pen);
        GdipSetPenLineJoin(pen, LineJoinRound);
        gpu_dispatch::GdipDrawPath(g, pen, p);
        GdipDeletePen(pen);
        let mut brush = null_mut();
        GdipCreateSolidFill(argb(color, 255), &mut brush);
        gpu_dispatch::GdipFillPath(g, brush.cast(), p);
        gpu_dispatch::GdipDeleteBrush(brush.cast());
        GdipRestoreGraphics(g, state);
        true
    }
    // A cached glyph-shaped shadow, never a rectangular lyric backpanel.
    thread_local! {static LYRIC_SHADOWS:std::cell::RefCell<std::collections::VecDeque<(String,BlurredText)>>=const {std::cell::RefCell::new(std::collections::VecDeque::new())};}
    fn lyric_cache_budget(default: usize, _name: &str, entry_bytes: usize) -> usize {
        #[cfg(test)]
        let default = std::env::var(_name)
            .ok()
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(default);
        // Very large text needs enough entries to keep a visible lyric window hot.
        default
            .max(entry_bytes.saturating_mul(16))
            .min(default.saturating_mul(2))
    }
    pub unsafe fn lyric_shadow(dc: HDC, font: HFONT, value: &str, r: RECT, flags: u32) -> bool {
        let Some(g) = graphics(dc) else { return false };
        if gpu::active() {
            if let Some(p) = cached_text_path(font, value, r, flags | LYRICS_UNTRIMMED) {
                let mut state = 0;
                GdipSaveGraphics(g, &mut state);
                GdipTranslateWorldTransform(
                    g,
                    r.left as f32,
                    r.top as f32 + 1.,
                    MatrixOrderPrepend,
                );
                let drawn = gpu::draw(g, |s| s.blur(p, 0xa0000000, 2., Some(3.)));
                GdipRestoreGraphics(g, state);
                if drawn {
                    return true;
                }
            }
        }
        let key = format!(
            "{:p}:{}:{}:{}:{}",
            font,
            r.right - r.left,
            r.bottom - r.top,
            flags,
            value
        );
        let pad = 12;
        let width = r.right - r.left + pad * 2;
        let height = r.bottom - r.top + pad * 2;
        if width <= 0 || height <= 0 || width as i64 * height as i64 > 2_000_000 {
            return false;
        }
        LYRIC_SHADOWS.with(|cache| {
            let mut cache = cache.borrow_mut();
            let index = cache.iter().position(|(old, _)| *old == key);
            let entry = if let Some(index) = index {
                cache.remove(index).unwrap()
            } else {
                let Some(path) = cached_text_path(font, value, r, flags | LYRICS_UNTRIMMED) else {
                    return false;
                };
                let mut pixels = vec![0u32; (width * height) as usize];
                let mut bitmap = null_mut();
                if GdipCreateBitmapFromScan0(
                    width,
                    height,
                    width * 4,
                    0xe200b,
                    pixels.as_mut_ptr().cast(),
                    &mut bitmap,
                ) != 0
                {
                    return false;
                }
                let mut canvas = null_mut();
                if GdipGetImageGraphicsContext(bitmap.cast(), &mut canvas) != 0 {
                    gpu_dispatch::GdipDisposeImage(bitmap.cast());
                    return false;
                }
                GdipSetSmoothingMode(canvas, SmoothingModeAntiAlias);
                GdipSetPixelOffsetMode(canvas, PixelOffsetModeHighQuality);
                GdipTranslateWorldTransform(
                    canvas,
                    pad as f32,
                    pad as f32 + 1.,
                    MatrixOrderPrepend,
                );
                for (stroke, alpha) in [(8., 45), (3., 190)] {
                    let mut pen = null_mut();
                    GdipCreatePen1((alpha as u32) << 24, stroke, UnitPixel, &mut pen);
                    GdipSetPenLineJoin(pen, LineJoinRound);
                    gpu_dispatch::GdipDrawPath(canvas, pen, path);
                    GdipDeletePen(pen);
                }
                GdipDeleteGraphics(canvas);
                while cache.len() >= 96
                    || cache
                        .iter()
                        .map(|(_, image)| image.pixels.len() * 4)
                        .sum::<usize>()
                        + pixels.len() * 4
                        > lyric_cache_budget(
                            2 * 1024 * 1024,
                            "DEEZNATS_TEST_SHADOW_CACHE_BYTES",
                            pixels.len() * 4,
                        )
                {
                    if cache.pop_front().is_none() {
                        break;
                    }
                }
                (key, BlurredText { bitmap, pixels })
            };
            gpu_dispatch::GdipDrawImageRectI(
                g,
                entry.1.bitmap.cast(),
                r.left - pad,
                r.top - pad,
                width,
                height,
            );
            cache.push_back(entry);
            true
        })
    }

    struct BlurredText {
        bitmap: *mut GpBitmap,
        pixels: Vec<u32>,
    }
    struct TextRaster {
        image: BlurredText,
        bounds: RECT,
    }
    thread_local! {static RASTER_LYRICS:std::cell::RefCell<std::collections::VecDeque<(String,TextRaster)>>=const {std::cell::RefCell::new(std::collections::VecDeque::new())};}

    // Keep alignment and ellipsis based on the original layout rectangle, but
    // retain and blend only the pixels occupied by glyphs.
    fn ink_bounds(pixels: &[u32], width: usize, height: usize) -> Option<RECT> {
        let mut bounds = RECT {
            left: width as i32,
            top: height as i32,
            right: 0,
            bottom: 0,
        };
        for (y, row) in pixels.chunks_exact(width).take(height).enumerate() {
            if let Some(left) = row.iter().position(|pixel| pixel >> 24 != 0) {
                let right = row.iter().rposition(|pixel| pixel >> 24 != 0).unwrap() + 1;
                bounds.left = bounds.left.min(left as i32);
                bounds.right = bounds.right.max(right as i32);
                bounds.top = bounds.top.min(y as i32);
                bounds.bottom = y as i32 + 1;
            }
        }
        (bounds.right > bounds.left).then_some(bounds)
    }
    unsafe fn raster_text(
        g: *mut GpGraphics,
        font: HFONT,
        value: &str,
        r: RECT,
        color: u32,
        flags: u32,
    ) -> bool {
        let width = r.right - r.left;
        let height = r.bottom - r.top;
        if width <= 0 || height <= 0 || width as i64 * height as i64 > 2_000_000 {
            return false;
        }
        let lf = font_properties(font);
        let key = format!(
            "{:p}:{}:{}:{width}:{height}:{color}:{flags}:{value}",
            font, lf.lfHeight, lf.lfWeight
        );
        RASTER_LYRICS.with(|cache| {
            let mut cache = cache.borrow_mut();
            let entry = if let Some(index) = cache.iter().position(|(old, _)| *old == key) {
                cache.remove(index).unwrap()
            } else {
                let Some(path) = cached_text_path(font, value, r, flags) else {
                    return true;
                };
                let mut pixels = vec![0u32; (width * height) as usize];
                let mut bitmap = null_mut();
                if GdipCreateBitmapFromScan0(
                    width,
                    height,
                    width * 4,
                    0xe200b,
                    pixels.as_mut_ptr().cast(),
                    &mut bitmap,
                ) != 0
                {
                    return false;
                }
                let mut canvas = null_mut();
                if GdipGetImageGraphicsContext(bitmap.cast(), &mut canvas) != 0 {
                    gpu_dispatch::GdipDisposeImage(bitmap.cast());
                    return false;
                }
                GdipSetSmoothingMode(canvas, SmoothingModeAntiAlias);
                GdipSetPixelOffsetMode(canvas, PixelOffsetModeHighQuality);
                let mut pen = null_mut();
                GdipCreatePen1(argb(color, 48), 0.3, UnitPixel, &mut pen);
                GdipSetPenLineJoin(pen, LineJoinRound);
                gpu_dispatch::GdipDrawPath(canvas, pen, path);
                GdipDeletePen(pen);
                let mut brush = null_mut();
                GdipCreateSolidFill(argb(color, 255), &mut brush);
                gpu_dispatch::GdipFillPath(canvas, brush.cast(), path);
                gpu_dispatch::GdipDeleteBrush(brush.cast());
                GdipDeleteGraphics(canvas);
                let ink = ink_bounds(&pixels, width as usize, height as usize).unwrap_or(RECT {
                    left: 0,
                    top: 0,
                    right: 1,
                    bottom: 1,
                });
                let cropped_width = ink.right - ink.left;
                let cropped_height = ink.bottom - ink.top;
                let mut cropped = Vec::with_capacity((cropped_width * cropped_height) as usize);
                for y in ink.top..ink.bottom {
                    let start = (y * width + ink.left) as usize;
                    cropped.extend_from_slice(&pixels[start..start + cropped_width as usize]);
                }
                gpu_dispatch::GdipDisposeImage(bitmap.cast());
                bitmap = null_mut();
                if GdipCreateBitmapFromScan0(
                    cropped_width,
                    cropped_height,
                    cropped_width * 4,
                    0xe200b,
                    cropped.as_mut_ptr().cast(),
                    &mut bitmap,
                ) != 0
                {
                    return false;
                }
                let pixels = cropped;
                while cache.len() >= 512
                    || cache
                        .iter()
                        .map(|(_, raster)| raster.image.pixels.len() * 4)
                        .sum::<usize>()
                        + pixels.len() * 4
                        > lyric_cache_budget(
                            4 * 1024 * 1024,
                            "DEEZNATS_TEST_TEXT_CACHE_BYTES",
                            pixels.len() * 4,
                        )
                {
                    if cache.pop_front().is_none() {
                        break;
                    }
                }
                (
                    key,
                    TextRaster {
                        image: BlurredText { bitmap, pixels },
                        bounds: ink,
                    },
                )
            };
            let ink = entry.1.bounds;
            let width = ink.right - ink.left;
            let height = ink.bottom - ink.top;
            let x = r.left + ink.left;
            let y = r.top + ink.top;
            if cfg!(debug_assertions)
                || !composite_raster(g, &entry.1.image.pixels, width, height, x, y)
            {
                gpu_dispatch::GdipDrawImageRectI(
                    g,
                    entry.1.image.bitmap.cast(),
                    x,
                    y,
                    width,
                    height,
                );
            }
            cache.push_back(entry);
            true
        })
    }
    impl Drop for BlurredText {
        fn drop(&mut self) {
            unsafe {
                gpu_dispatch::GdipDisposeImage(self.bitmap.cast());
            }
        }
    }
    thread_local! {
        static BLURRED_TEXT: std::cell::RefCell<std::collections::VecDeque<(String,BlurredText)>> = const {std::cell::RefCell::new(std::collections::VecDeque::new())};
        static LYRIC_WIDTHS: std::cell::RefCell<std::collections::VecDeque<(String,i32)>> = const {std::cell::RefCell::new(std::collections::VecDeque::new())};
        static LYRIC_CACHE_SCOPE: std::cell::RefCell<Option<(String, String)>> = const {std::cell::RefCell::new(None)};
    }
    pub(crate) fn lyric_raster_bytes() -> (usize, usize, usize) {
        let bytes = |items: &std::collections::VecDeque<(String, BlurredText)>| {
            items.iter().map(|(_, image)| image.pixels.len() * 4).sum()
        };
        (
            LYRIC_SHADOWS.with(|cache| bytes(&cache.borrow())),
            RASTER_LYRICS.with(|cache| {
                cache
                    .borrow()
                    .iter()
                    .map(|(_, raster)| raster.image.pixels.len() * 4)
                    .sum()
            }),
            BLURRED_TEXT.with(|cache| bytes(&cache.borrow())),
        )
    }
    pub(crate) fn clear_lyric_rasters() {
        LYRIC_CACHE_SCOPE.with(|scope| *scope.borrow_mut() = None);
        LYRIC_SHADOWS.with(|cache| cache.borrow_mut().clear());
        RASTER_LYRICS.with(|cache| cache.borrow_mut().clear());
        BLURRED_TEXT.with(|cache| cache.borrow_mut().clear());
        LYRIC_WIDTHS.with(|cache| cache.borrow_mut().clear());
    }
    pub(crate) fn set_lyric_cache_scope(scope: Option<(&str, &str)>) {
        let changed = LYRIC_CACHE_SCOPE.with(|current| {
            let mut current = current.borrow_mut();
            if current
                .as_ref()
                .map(|(provider, track)| (provider.as_str(), track.as_str()))
                == scope
            {
                return false;
            }
            *current = scope.map(|(provider, track)| (provider.to_owned(), track.to_owned()));
            true
        });
        if changed {
            LYRIC_SHADOWS.with(|cache| *cache.borrow_mut() = Default::default());
            RASTER_LYRICS.with(|cache| *cache.borrow_mut() = Default::default());
            BLURRED_TEXT.with(|cache| *cache.borrow_mut() = Default::default());
            LYRIC_WIDTHS.with(|cache| *cache.borrow_mut() = Default::default());
        }
    }
    pub unsafe fn lyric_text_width(dc: HDC, font: HFONT, value: &str) -> i32 {
        if value.is_empty() {
            return 0;
        }
        let lf = font_properties(font);
        let key = format!("{:p}:{}:{}:{}", font, lf.lfHeight, lf.lfWeight, value);
        if let Some(width) =
            LYRIC_WIDTHS.with(|c| c.borrow().iter().find(|(k, _)| *k == key).map(|(_, w)| *w))
        {
            return width;
        }
        let Some(g) = graphics(dc) else {
            return 0;
        };
        let mut family = null_mut();
        let mut gp_font = null_mut();
        let mut format = null_mut();
        GdipCreateFontFamilyFromName(lf.lfFaceName.as_ptr(), null_mut(), &mut family);
        if family.is_null() {
            return 0;
        }
        GdipCreateFont(
            family,
            lf.lfHeight.unsigned_abs() as f32,
            if lf.lfWeight >= 600 {
                FontStyleBold
            } else {
                FontStyleRegular
            },
            UnitPixel,
            &mut gp_font,
        );
        GdipStringFormatGetGenericTypographic(&mut format);
        GdipSetStringFormatFlags(
            format,
            StringFormatFlagsNoWrap
                | StringFormatFlagsNoClip
                | StringFormatFlagsMeasureTrailingSpaces,
        );
        let text: Vec<_> = value.encode_utf16().collect();
        let layout = RectF {
            X: 0.0,
            Y: 0.0,
            Width: 100000.0,
            Height: 10000.0,
        };
        let mut bounds: RectF = zeroed();
        let status = GdipMeasureString(
            g,
            text.as_ptr(),
            text.len() as i32,
            gp_font,
            &layout,
            format,
            &mut bounds,
            null_mut(),
            null_mut(),
        );
        GdipDeleteStringFormat(format);
        GdipDeleteFont(gp_font);
        GdipDeleteFontFamily(family);
        let width = if status == 0 {
            bounds.Width.ceil() as i32
        } else {
            0
        };
        LYRIC_WIDTHS.with(|c| {
            let mut c = c.borrow_mut();
            if c.len() >= 512 {
                c.pop_front();
            }
            c.push_back((key, width));
        });
        width
    }
    pub unsafe fn blurred_text(
        dc: HDC,
        font: HFONT,
        value: &str,
        r: RECT,
        color: u32,
        flags: u32,
        blur_px: f32,
    ) -> bool {
        let Some(g) = graphics(dc) else {
            return false;
        };
        if gpu::active() {
            if let Some(p) = cached_text_path(font, value, r, flags | LYRICS_UNTRIMMED) {
                let mut state = 0;
                GdipSaveGraphics(g, &mut state);
                GdipTranslateWorldTransform(g, r.left as f32, r.top as f32, MatrixOrderPrepend);
                let drawn = gpu::draw(g, |s| {
                    s.blur(p, argb(color, 255), blur_px.clamp(0.5, 6.), None)
                });
                GdipRestoreGraphics(g, state);
                if drawn {
                    return true;
                }
            }
        }
        if value.is_empty() || r.right <= r.left || r.bottom <= r.top {
            return true;
        }
        let radius = blur_px.clamp(0.5, 6.0);
        let pad = (radius * 3.0).ceil() as i32 + 2;
        let width = r.right - r.left + pad * 2;
        let height = r.bottom - r.top + pad * 2;
        if width as usize * height as usize * 4 > 8 * 1024 * 1024 {
            return false;
        }
        let lf = font_properties(font);
        let key = format!(
            "{:p}:{}:{}:{}:{}:{}:{}:{}:{}",
            font, lf.lfHeight, lf.lfWeight, width, height, color, flags, radius, value
        );
        BLURRED_TEXT.with(|cache| {
            let mut cache = cache.borrow_mut();
            if let Some(index) = cache.iter().position(|(k, _)| *k == key) {
                let entry = cache.remove(index).unwrap();
                cache.push_back(entry);
            } else {
                let Some(path) = cached_text_path(font, value, r, flags | LYRICS_UNTRIMMED) else {
                    return false;
                };
                let mut pixels = vec![0u32; width as usize * height as usize];
                let mut bitmap = null_mut();
                let mut mask_graphics = null_mut();
                if GdipCreateBitmapFromScan0(
                    width,
                    height,
                    width * 4,
                    0xe200b,
                    pixels.as_mut_ptr().cast(),
                    &mut bitmap,
                ) != 0
                {
                    return false;
                }
                if GdipGetImageGraphicsContext(bitmap.cast(), &mut mask_graphics) != 0 {
                    gpu_dispatch::GdipDisposeImage(bitmap.cast());
                    return false;
                }
                GdipSetSmoothingMode(mask_graphics, SmoothingModeAntiAlias);
                GdipTranslateWorldTransform(
                    mask_graphics,
                    pad as f32,
                    pad as f32,
                    MatrixOrderPrepend,
                );
                let mut brush = null_mut();
                GdipCreateSolidFill(0xffffffff, &mut brush);
                gpu_dispatch::GdipFillPath(mask_graphics, brush.cast(), path);
                gpu_dispatch::GdipDeleteBrush(brush.cast());
                GdipDeleteGraphics(mask_graphics);
                let mask = image::GrayImage::from_vec(
                    width as u32,
                    height as u32,
                    pixels.iter().map(|p| (p >> 24) as u8).collect(),
                )
                .unwrap();
                let blurred = image::imageops::blur(&mask, radius);
                let red = color & 255;
                let green = (color >> 8) & 255;
                let blue = (color >> 16) & 255;
                for (pixel, alpha) in pixels.iter_mut().zip(blurred.as_raw()) {
                    let alpha = *alpha as u32;
                    *pixel = (alpha << 24)
                        | ((red * alpha / 255) << 16)
                        | ((green * alpha / 255) << 8)
                        | (blue * alpha / 255);
                }
                while cache.len() >= 64
                    || cache.iter().map(|(_, b)| b.pixels.len() * 4).sum::<usize>()
                        + pixels.len() * 4
                        > lyric_cache_budget(
                            2 * 1024 * 1024,
                            "DEEZNATS_TEST_BLUR_CACHE_BYTES",
                            pixels.len() * 4,
                        )
                {
                    cache.pop_front();
                }
                cache.push_back((key, BlurredText { bitmap, pixels }));
            }
            let bitmap = cache.back().unwrap().1.bitmap;
            gpu_dispatch::GdipDrawImageRectI(
                g,
                bitmap.cast(),
                r.left - pad,
                r.top - pad,
                width,
                height,
            ) == 0
        })
    }

    pub unsafe fn karaoke_text(
        dc: HDC,
        font: HFONT,
        value: &str,
        progress: f32,
        r: RECT,
        lit_color: u32,
        unlit_color: u32,
        glow_color: u32,
        flags: u32,
    ) -> bool {
        let Some(g) = graphics(dc) else { return false };
        if value.is_empty() || r.right <= r.left || r.bottom <= r.top {
            return true;
        }
        let Some(p) = cached_text_path(font, value, r, flags | LYRICS_UNTRIMMED) else {
            return true;
        };
        let mut bounds = RectF {
            X: 0.0,
            Y: 0.0,
            Width: (r.right - r.left) as f32,
            Height: (r.bottom - r.top) as f32,
        };
        GdipGetPathWorldBounds(p, &mut bounds, null_mut(), null_mut());
        let phase = progress.clamp(0.0, 1.0);
        let lift = (phase * std::f32::consts::PI).sin() * (r.bottom - r.top) as f32 * 0.055;
        let mut state = 0;
        GdipSaveGraphics(g, &mut state);
        GdipSetClipRectI(
            g,
            r.left - 8,
            r.top - 8,
            r.right - r.left + 16,
            r.bottom - r.top + 16,
            CombineModeIntersect,
        );
        GdipTranslateWorldTransform(g, r.left as f32, r.top as f32 - lift, MatrixOrderPrepend);
        // Keep the normal lyric outline/opacity while each sung word lifts smoothly.
        let mut pen = null_mut();
        GdipCreatePen1(argb(unlit_color, 48), 0.3, UnitPixel, &mut pen);
        GdipSetPenLineJoin(pen, LineJoinRound);
        gpu_dispatch::GdipDrawPath(g, pen, p);
        GdipDeletePen(pen);
        let mut brush = null_mut();
        GdipCreateSolidFill(argb(unlit_color, 255), &mut brush);
        gpu_dispatch::GdipFillPath(g, brush.cast(), p);
        gpu_dispatch::GdipDeleteBrush(brush.cast());
        let eased = phase * phase * (3.0 - 2.0 * phase);
        if eased > 0.005 {
            let lit_right = (bounds.X + (bounds.Width + 6.0) * eased).ceil() as i32;
            GdipSetClipRectI(
                g,
                -8,
                -8,
                (lit_right + 14).max(1),
                r.bottom - r.top + 16,
                CombineModeIntersect,
            );
            if phase < 1.0 {
                let mut glow = null_mut();
                GdipCreatePen1(
                    argb(
                        glow_color,
                        (70.0 * (phase * std::f32::consts::PI).sin()) as u32,
                    ),
                    3.0,
                    UnitPixel,
                    &mut glow,
                );
                GdipSetPenLineJoin(glow, LineJoinRound);
                gpu_dispatch::GdipDrawPath(g, glow, p);
                GdipDeletePen(glow);
            }
            let layers: &[(i32, u32)] = if phase >= 1.0 {
                &[(8, 255)]
            } else {
                &[(6, 45), (2, 95), (-3, 255)]
            };
            for &(feather, alpha) in layers {
                GdipSetClipRectI(
                    g,
                    -8,
                    -8,
                    (lit_right + 8 + feather).max(1),
                    r.bottom - r.top + 16,
                    CombineModeIntersect,
                );
                let mut lit = null_mut();
                GdipCreateSolidFill(argb(lit_color, alpha), &mut lit);
                gpu_dispatch::GdipFillPath(g, lit.cast(), p);
                gpu_dispatch::GdipDeleteBrush(lit.cast());
            }
        }
        GdipRestoreGraphics(g, state);
        true
    }

    // Each GDI+ bitmap has one owner. Workers finish drawing before transferring
    // ownership through the channel; only the UI touches a completed bitmap.
    struct PreparedArtwork(*mut GpBitmap);
    unsafe impl Send for PreparedArtwork {}
    impl Drop for PreparedArtwork {
        fn drop(&mut self) {
            unsafe {
                gpu_dispatch::GdipDisposeImage(self.0.cast());
            }
        }
    }

    struct ArtworkPreparation {
        requests: std::sync::mpsc::SyncSender<(ImageKey, crate::catalog::Bitmap)>,
        ready: std::sync::mpsc::Receiver<(ImageKey, Option<PreparedArtwork>)>,
        pending: std::collections::HashSet<ImageKey>,
    }

    type ArtworkJob = (ImageKey, crate::catalog::Bitmap);
    type ArtworkPreparer = std::sync::Arc<
        dyn Fn(ImageKey, crate::catalog::Bitmap) -> Option<PreparedArtwork> + Send + Sync,
    >;

    fn spawn_artwork_workers(
        work: std::sync::mpsc::Receiver<ArtworkJob>,
        finished: std::sync::mpsc::SyncSender<(ImageKey, Option<PreparedArtwork>)>,
        count: usize,
        prepare: ArtworkPreparer,
    ) {
        let work = std::sync::Arc::new(std::sync::Mutex::new(work));
        for index in 0..count {
            let work = work.clone();
            let finished = finished.clone();
            let prepare = prepare.clone();
            std::thread::Builder::new()
                .name(format!("artwork-resize-{index}"))
                .spawn(move || {
                    crate::catalog::artwork_background_priority();
                    loop {
                        let result = match work.lock() {
                            std::result::Result::Ok(receiver) => receiver.recv(),
                            Err(_) => return,
                        };
                        let std::result::Result::Ok((key, image)) = result else {
                            break;
                        };
                        let bitmap = prepare(key, image);
                        if finished.send((key, bitmap)).is_err() {
                            break;
                        }
                    }
                })
                .expect("start artwork preparation worker");
        }
    }

    pub(crate) fn progressive_artwork_enabled() -> bool {
        static ENABLED: OnceLock<bool> = OnceLock::new();
        *ENABLED.get_or_init(|| std::env::var_os("DEEZNATS_LEGACY_ARTWORK").is_none())
    }

    impl ArtworkPreparation {
        fn new() -> Self {
            let (requests, work) =
                std::sync::mpsc::sync_channel::<(ImageKey, crate::catalog::Bitmap)>(8);
            let (finished, ready) = std::sync::mpsc::sync_channel(8);
            let prepare = std::sync::Arc::new(|key, image| unsafe {
                prepare_artwork(&image, key).map(PreparedArtwork)
            });
            spawn_artwork_workers(
                work,
                finished,
                if progressive_artwork_enabled() { 3 } else { 1 },
                prepare,
            );
            Self {
                requests,
                ready,
                pending: Default::default(),
            }
        }
    }

    thread_local! {
        static ARTWORK_PREPARATION: std::cell::RefCell<Option<ArtworkPreparation>> = const { std::cell::RefCell::new(None) };
    }

    fn cache_prepared_artwork(key: ImageKey, bitmap: *mut GpBitmap) {
        RESIZED_IMAGES.with(|cache| {
            let mut cache = cache.borrow_mut();
            let mut bytes: usize = cache.iter().map(|(key, _)| resized_bytes(*key)).sum();
            while cache.len() >= 512 || bytes + resized_bytes(key) > RESIZED_IMAGE_BYTES {
                if let Some((old_key, old)) = cache.pop_front() {
                    bytes = bytes.saturating_sub(resized_bytes(old_key));
                    unsafe {
                        gpu_dispatch::GdipDisposeImage(old.cast());
                    }
                } else {
                    break;
                }
            }
            cache.push_back((key, bitmap));
        });
    }

    // Bounded polling leaves time for input even when a catalog delivers many covers.
    pub(crate) fn poll_prepared_artwork() -> bool {
        ARTWORK_PREPARATION.with(|state| {
            let mut state = state.borrow_mut();
            let Some(state) = state.as_mut() else {
                return false;
            };
            let mut changed = false;
            for _ in 0..4 {
                let std::result::Result::Ok((key, bitmap)) = state.ready.try_recv() else {
                    break;
                };
                if state.pending.remove(&key) {
                    if let Some(bitmap) = bitmap {
                        cache_prepared_artwork(key, bitmap.0);
                        std::mem::forget(bitmap);
                        changed = true;
                    }
                }
            }
            changed
        })
    }

    pub(crate) fn artwork_preparation_pending() -> bool {
        ARTWORK_PREPARATION.with(|state| {
            state
                .borrow()
                .as_ref()
                .is_some_and(|state| !state.pending.is_empty())
        })
    }

    unsafe fn prepare_artwork(
        image: &crate::catalog::Bitmap,
        key: ImageKey,
    ) -> Option<*mut GpBitmap> {
        let (_, width, height, radius) = key;
        let radius = f32::from_bits(radius);
        let mut source = null_mut();
        let mut bitmap = null_mut();
        let mut resize = null_mut();
        let mut attributes = null_mut();
        if GdipCreateBitmapFromScan0(
            image.width as i32,
            image.height as i32,
            image.width as i32 * 4,
            0xe200b,
            image.pixels.as_ptr().cast(),
            &mut source,
        ) != 0
        {
            return None;
        }
        if GdipCreateBitmapFromScan0(width, height, 0, 0xe200b, null(), &mut bitmap) != 0 {
            gpu_dispatch::GdipDisposeImage(source.cast());
            return None;
        }
        GdipGetImageGraphicsContext(bitmap.cast(), &mut resize);
        GdipGraphicsClear(resize, 0);
        let rounded = path(
            RECT {
                left: 0,
                top: 0,
                right: width,
                bottom: height,
            },
            radius,
        );
        GdipSetClipPath(resize, rounded, CombineModeIntersect);
        gpu_dispatch::GdipDeletePath(rounded);
        GdipSetInterpolationMode(resize, InterpolationModeHighQualityBicubic);
        GdipSetPixelOffsetMode(resize, PixelOffsetModeHighQuality);
        GdipCreateImageAttributes(&mut attributes);
        GdipSetImageAttributesWrapMode(attributes, WrapModeTileFlipXY, 0, 0);
        GdipDrawImageRectRectI(
            resize,
            source.cast(),
            0,
            0,
            width,
            height,
            0,
            0,
            image.width as i32,
            image.height as i32,
            UnitPixel,
            attributes,
            0,
            null_mut(),
        );
        GdipDisposeImageAttributes(attributes);
        GdipDeleteGraphics(resize);
        gpu_dispatch::GdipDisposeImage(source.cast());
        Some(bitmap)
    }

    pub unsafe fn image(
        dc: HDC,
        r: RECT,
        image: &crate::catalog::Bitmap,
        radius: f32,
        alpha: u8,
    ) -> bool {
        let Some(g) = graphics(dc) else {
            return false;
        };
        if gpu::draw(g, |s| {
            s.image(
                (image.id << 1) | 1,
                image.pixels.as_ptr().cast(),
                image.width,
                image.height,
                image.width * 4,
                r,
                radius
                    .max(0.)
                    .min(((r.right - r.left).min(r.bottom - r.top)) as f32 / 2.),
                alpha,
            )
        }) {
            return true;
        }
        if r.right <= r.left || r.bottom <= r.top {
            return true;
        }
        let width = r.right - r.left;
        let height = r.bottom - r.top;
        let radius = radius.max(0.0).min(width.min(height) as f32 / 2.0);
        let key = (image.id, width, height, radius.to_bits());
        let cacheable = resized_bytes(key) <= RESIZED_IMAGE_BYTES;
        let cached = RESIZED_IMAGES.with(|cache| {
            let mut cache = cache.borrow_mut();
            let index = cache.iter().position(|(k, _)| *k == key)?;
            let entry = cache.remove(index)?;
            let bitmap = entry.1;
            cache.push_back(entry);
            Some(bitmap)
        });
        let (bitmap, source_width, source_height, direct) = if let Some(bitmap) = cached {
            (bitmap, width, height, false)
        } else {
            if cacheable {
                #[cfg(test)]
                if std::env::var_os("DEEZNATS_BENCH_SYNC_ARTWORK").is_some() {
                    if let Some(bitmap) = prepare_artwork(image, key) {
                        cache_prepared_artwork(key, bitmap);
                        return self::image(dc, r, image, radius, alpha);
                    }
                    return true;
                }
                ARTWORK_PREPARATION.with(|state| {
                    let mut state = state.borrow_mut();
                    let state = state.get_or_insert_with(ArtworkPreparation::new);
                    if !state.pending.contains(&key) && state.pending.len() < 8 {
                        if state.requests.try_send((key, image.clone())).is_ok() {
                            state.pending.insert(key);
                        }
                    }
                });
                if !progressive_artwork_enabled() {
                    return true;
                }
                // Display the decoded pixels immediately while the bounded worker
                // queue prepares the rounded, device-sized cached version.
                let mut source = null_mut();
                if GdipCreateBitmapFromScan0(
                    image.width as i32,
                    image.height as i32,
                    image.width as i32 * 4,
                    0xe200b,
                    image.pixels.as_ptr().cast(),
                    &mut source,
                ) != 0
                {
                    return true;
                }
                (source, image.width as i32, image.height as i32, true)
            } else {
                // Oversized, uncached one-off surfaces retain their existing behavior.
                let Some(bitmap) = prepare_artwork(image, key) else {
                    return true;
                };
                (bitmap, width, height, false)
            }
        };
        let mut state = 0;
        GdipSaveGraphics(g, &mut state);
        if direct {
            let rounded = path(r, radius);
            GdipSetClipPath(g, rounded, CombineModeIntersect);
            gpu_dispatch::GdipDeletePath(rounded);
        }
        let mut attributes = null_mut();
        if alpha < 255 {
            GdipCreateImageAttributes(&mut attributes);
            let mut matrix = ColorMatrix::default();
            for i in 0..5 {
                matrix.m[i * 5 + i] = 1.0;
            }
            matrix.m[18] = alpha as f32 / 255.0;
            GdipSetImageAttributesColorMatrix(
                attributes,
                ColorAdjustTypeBitmap,
                1,
                &matrix,
                null(),
                ColorMatrixFlagsDefault,
            );
        }
        GdipSetInterpolationMode(
            g,
            if direct {
                InterpolationModeBilinear
            } else {
                InterpolationModeNearestNeighbor
            },
        );
        if alpha == 255 && !direct {
            // The cached bitmap is already at device size with transparent corners.
            gpu_dispatch::GdipDrawImageRectI(g, bitmap.cast(), r.left, r.top, width, height);
        } else {
            GdipDrawImageRectRectI(
                g,
                bitmap.cast(),
                r.left,
                r.top,
                r.right - r.left,
                r.bottom - r.top,
                0,
                0,
                source_width,
                source_height,
                UnitPixel,
                attributes,
                0,
                null_mut(),
            );
            if !attributes.is_null() {
                GdipDisposeImageAttributes(attributes);
            }
        }
        GdipRestoreGraphics(g, state);

        if direct || !cacheable {
            gpu_dispatch::GdipDisposeImage(bitmap.cast());
        }

        true
    }

    // Draw at device resolution: no intermediate downsampling or bilinear blur.
    pub unsafe fn recessed_visualizer(_dc: HDC, width: i32, height: i32, draw: impl FnOnce()) {
        if width > 0 && height > 0 {
            draw();
        }
    }
    pub unsafe fn wave_landscape(
        dc: HDC,
        points: &[(f32, f32)],
        baseline_y: f32,
        fill_top: u32,
        fill_bottom: u32,
        stroke: u32,
        stroke_width: f32,
        glow: u32,
    ) -> bool {
        let Some(g) = graphics(dc) else {
            return false;
        };
        if points.len() < 2 {
            return true;
        }
        let pts: Vec<PointF> = points.iter().map(|&(x, y)| PointF { X: x, Y: y }).collect();
        let min_y = pts.iter().map(|p| p.Y).fold(baseline_y, f32::min);

        let bounds = RectF {
            X: pts[0].X,
            Y: min_y,
            Width: (pts[pts.len() - 1].X - pts[0].X).max(1.0),
            Height: (baseline_y - min_y).max(1.0),
        };

        let mut brush = null_mut();
        gpu_dispatch::GdipCreateLineBrushFromRectWithAngle(
            &bounds,
            fill_top,
            fill_bottom,
            90.0,
            0,
            WrapModeTileFlipXY,
            &mut brush,
        );

        let mut path = null_mut();
        GdipCreatePath(FillModeAlternate, &mut path);
        GdipAddPathCurve2(path, pts.as_ptr(), pts.len() as i32, 0.45);
        GdipAddPathLine(path, pts[pts.len() - 1].X, baseline_y, pts[0].X, baseline_y);
        GdipClosePathFigure(path);

        gpu_dispatch::GdipFillPath(g, brush.cast(), path);
        gpu_dispatch::GdipDeleteBrush(brush.cast());
        gpu_dispatch::GdipDeletePath(path);

        if glow != 0 {
            // Approximate Sonora's broad, soft wave glow with translucent strokes.
            let alpha = glow >> 24;
            for (width, opacity) in [(10.0, 0.08), (6.0, 0.12), (3.0, 0.18)] {
                let color = (glow & 0x00ff_ffff) | (((alpha as f32 * opacity) as u32) << 24);
                let mut glow_pen = null_mut();
                GdipCreatePen1(color, stroke_width * width, UnitPixel, &mut glow_pen);
                gpu_dispatch::GdipDrawCurve2(g, glow_pen, pts.as_ptr(), pts.len() as i32, 0.45);
                GdipDeletePen(glow_pen);
            }
        }

        let mut pen = null_mut();
        GdipCreatePen1(stroke, stroke_width, UnitPixel, &mut pen);
        gpu_dispatch::GdipDrawCurve2(g, pen, pts.as_ptr(), pts.len() as i32, 0.45);
        GdipDeletePen(pen);

        true
    }

    pub unsafe fn spectrum_bars(
        dc: HDC,
        bars: &[f32],
        x: f32,
        y: f32,
        width: f32,
        max_height: f32,
        accent: u32,
        _glow: u32,
    ) -> bool {
        let Some(g) = graphics(dc) else {
            return false;
        };
        if bars.is_empty() {
            return true;
        }
        let count = bars.len();
        let spacing = 3.0f32;
        let total_spacing = (count - 1) as f32 * spacing;
        let bar_w = ((width - total_spacing) / count as f32).max(2.0);

        for (i, &mag) in bars.iter().enumerate() {
            let bh = (mag * max_height).clamp(4.0, max_height);
            let bx = x + i as f32 * (bar_w + spacing);
            let by = y + max_height - bh;

            // Sonora uses a flat, translucent fill for its bars.
            let mut brush = null_mut();
            GdipCreateSolidFill(argb(accent, 82), &mut brush);

            let mut path = null_mut();
            let radius = (bar_w * 0.45).clamp(1.0, 4.0);
            GdipCreatePath(FillModeAlternate, &mut path);
            let d = radius * 2.0;
            GdipAddPathArc(path, bx + bar_w - d, by, d, d, 270.0, 90.0);
            GdipAddPathLine(path, bx + bar_w, by + bh, bx, by + bh);
            GdipAddPathArc(path, bx, by, d, d, 180.0, 90.0);
            GdipClosePathFigure(path);

            gpu_dispatch::GdipFillPath(g, brush.cast(), path);
            gpu_dispatch::GdipDeleteBrush(brush.cast());
            gpu_dispatch::GdipDeletePath(path);
        }

        true
    }

    pub unsafe fn trap_nation_rectangle(
        dc: HDC,
        cover: RECT,
        corner_radius: f32,
        bass_pump: f32,
        layers: &[Vec<f32>],
        accent: u32,
    ) -> bool {
        let Some(g) = graphics(dc) else {
            return false;
        };
        let width = (cover.right - cover.left) as f32;
        let height = (cover.bottom - cover.top) as f32;
        let radius = corner_radius.clamp(0., width.min(height) * 0.5);
        let horizontal = width - 2. * radius;
        let vertical = height - 2. * radius;
        let arc = std::f32::consts::FRAC_PI_2 * radius;
        let lengths = [
            horizontal, arc, vertical, arc, horizontal, arc, vertical, arc,
        ];
        let perimeter: f32 = lengths.iter().sum();
        for (layer, spikes) in layers.iter().take(2).enumerate().rev() {
            let points: Vec<PointF> = spikes
                .iter()
                .enumerate()
                .map(|(k, _)| {
                    let mut distance = k as f32 / spikes.len() as f32 * perimeter;
                    let mut segment = 0;
                    while segment < 7 && distance >= lengths[segment] {
                        distance -= lengths[segment];
                        segment += 1;
                    }
                    // Each straight edge gets its own complete bass-to-treble response.
                    // Fade into corners to keep the selected cover outline intact.
                    let spike = if segment % 2 == 0 && lengths[segment] > 0. {
                        let progress = (distance / lengths[segment]).clamp(0., 1.);
                        // The circular contour contains bass-to-treble and its reflection.
                        // Use only its first half, so each edge traverses frequencies once.
                        let at = progress * spikes.len() as f32 * 0.5;
                        let index = at.floor() as usize;
                        let value = spikes[index] * (1. - at.fract())
                            + spikes[(index + 1).min(spikes.len() - 1)] * at.fract();
                        let taper = (progress * 12.).min((1. - progress) * 12.).min(1.);
                        value * taper
                    } else {
                        0.
                    };
                    let offset = spike * 0.55 + layer as f32 * 0.8
                        + bass_pump.clamp(0., 1.) * 2. + 1.;
                    let (x, y) = match segment {
                        0 => (radius + distance, -offset),
                        2 => (width + offset, radius + distance),
                        4 => (width - radius - distance, height + offset),
                        6 => (-offset, height - radius - distance),
                        _ => {
                            let corner = segment / 2;
                            let centers = [
                                (width - radius, radius),
                                (width - radius, height - radius),
                                (radius, height - radius),
                                (radius, radius),
                            ];
                            let angle = (corner as f32 - 1.) * std::f32::consts::FRAC_PI_2
                                + if radius > 0. { distance / radius } else { 0. };
                            let (cx, cy) = centers[corner];
                            (
                                cx + (radius + offset) * angle.cos(),
                                cy + (radius + offset) * angle.sin(),
                            )
                        }
                    };
                    PointF {
                        X: cover.left as f32 + x,
                        Y: cover.top as f32 + y,
                    }
                })
                .collect();
            if points.len() < 3 {
                continue;
            }
            let mut path = null_mut();
            GdipCreatePath(FillModeAlternate, &mut path);
            GdipAddPathPolygon(path, points.as_ptr(), points.len() as i32);
            // The cover is painted afterward, leaving only the outward filled response.
            let mut brush = null_mut();
            GdipCreateSolidFill(
                argb(accent, if layer == 0 { 48 } else { 12 }), &mut brush,
            );
            gpu_dispatch::GdipFillPath(g, brush.cast(), path);
            gpu_dispatch::GdipDeleteBrush(brush.cast());
            let mut pen = null_mut();
            GdipCreatePen1(
                argb(accent, if layer == 0 { 140 } else { 28 }),
                if layer == 0 { 1.4 } else { 0.8 },
                UnitPixel,
                &mut pen,
            );
            gpu_dispatch::GdipDrawPath(g, pen, path);
            GdipDeletePen(pen);
            gpu_dispatch::GdipDeletePath(path);
        }
        true
    }

    pub unsafe fn trap_nation_circle(
        dc: HDC,
        cx: f32,
        cy: f32,
        base_radius: f32,
        bass_pump: f32,
        layers: &[Vec<f32>],
        accent: u32,
        glow_col: u32,
    ) -> bool {
        let Some(g) = graphics(dc) else {
            return false;
        };
        if layers.is_empty() {
            return true;
        }

        let inner = base_radius * (1. + bass_pump.clamp(0., 1.) * 0.25);
        for (layer, spikes) in layers.iter().enumerate().rev() {
            let count = spikes.len();
            let alpha = if layer == 0 { 48 } else { 12 };
            let points: Vec<PointF> = spikes
                .iter()
                .enumerate()
                .map(|(k, spike)| {
                    let theta = k as f32 * std::f32::consts::TAU / count as f32
                        - std::f32::consts::FRAC_PI_2;
                    let radius = inner + spike + layer as f32 * 1.2;
                    PointF {
                        X: cx + radius * theta.cos(),
                        Y: cy + radius * theta.sin(),
                    }
                })
                .collect();
            if points.len() < 3 {
                continue;
            }
            let mut halo = null_mut();
            GdipCreatePath(FillModeAlternate, &mut halo);
            GdipAddPathClosedCurve2(halo, points.as_ptr(), points.len() as i32, 0.5);
            GdipAddPathEllipse(halo, cx - inner, cy - inner, inner * 2., inner * 2.);
            let color = if layer == 1 {
                let r = (accent & 255) * 3 / 4 + 63;
                let g = ((accent >> 8) & 255) * 3 / 4 + 63;
                let b = ((accent >> 16) & 255) * 3 / 4 + 63;
                r | (g << 8) | (b << 16)
            } else {
                accent
            };
            let mut brush = null_mut();
            GdipCreateSolidFill(argb(color, alpha), &mut brush);
            gpu_dispatch::GdipFillPath(g, brush.cast(), halo);
            gpu_dispatch::GdipDeleteBrush(brush.cast());
            gpu_dispatch::GdipDeletePath(halo);
            let mut edge = null_mut();
            GdipCreatePath(FillModeAlternate, &mut edge);
            GdipAddPathClosedCurve2(edge, points.as_ptr(), points.len() as i32, 0.5);
            let mut pen = null_mut();
            GdipCreatePen1(
                argb(color, if layer == 0 { 155 } else { alpha * 2 }),
                if layer == 0 { 1.8 } else { 1.2 },
                UnitPixel,
                &mut pen,
            );
            gpu_dispatch::GdipDrawPath(g, pen, edge);
            GdipDeletePen(pen);
            gpu_dispatch::GdipDeletePath(edge);
        }
        let _ = glow_col;

        true
    }

    #[cfg(test)]
    mod tests {
        #[test]
        fn cropped_text_raster_reconstructs_full_gdiplus_rendering() {
            unsafe {
                let dc = CreateCompatibleDC(null_mut());
                let mut output = vec![0u32; 240 * 100];
                let frame =
                    Frame::begin(dc, output.as_mut_ptr().cast(), 240, 240, 100, false).unwrap();
                let (_, graphics) = CURRENT.get();
                let mut lf: LOGFONTW = zeroed();
                lf.lfHeight = -24;
                lf.lfWeight = 700;
                for (slot, ch) in lf.lfFaceName.iter_mut().zip("Segoe UI".encode_utf16()) {
                    *slot = ch;
                }
                let font = CreateFontIndirectW(&lf);
                let cases = [
                    ("Left text", DT_LEFT, 10, 8),
                    ("Centered text", DT_CENTER, -30, 5),
                    ("Right text", DT_RIGHT, 190, -12),
                    (
                        "A wrapped line of text with several words",
                        DT_LEFT | DT_WORDBREAK,
                        -15,
                        70,
                    ),
                    (
                        "H\u{e9}llo \u{4e16}\u{754c} \u{1f3b5}",
                        DT_CENTER | DT_WORDBREAK,
                        20,
                        10,
                    ),
                    ("   ", DT_RIGHT, 0, 0),
                ];
                for (value, flags, x, y) in cases {
                    let width = 180;
                    let height = 80;
                    let rect = RECT {
                        left: x,
                        top: y,
                        right: x + width,
                        bottom: y + height,
                    };
                    let color = 0x009a52d3;
                    let mut reference = vec![0u32; (width * height) as usize];
                    let mut bitmap = null_mut();
                    assert_eq!(
                        GdipCreateBitmapFromScan0(
                            width,
                            height,
                            width * 4,
                            0xe200b,
                            reference.as_mut_ptr().cast(),
                            &mut bitmap
                        ),
                        0
                    );
                    let mut canvas = null_mut();
                    assert_eq!(GdipGetImageGraphicsContext(bitmap.cast(), &mut canvas), 0);
                    GdipSetSmoothingMode(canvas, SmoothingModeAntiAlias);
                    GdipSetPixelOffsetMode(canvas, PixelOffsetModeHighQuality);
                    let path = cached_text_path(font, value, rect, flags).unwrap();
                    let mut pen = null_mut();
                    GdipCreatePen1(argb(color, 48), 0.3, UnitPixel, &mut pen);
                    GdipSetPenLineJoin(pen, LineJoinRound);
                    gpu_dispatch::GdipDrawPath(canvas, pen, path);
                    GdipDeletePen(pen);
                    let mut brush = null_mut();
                    GdipCreateSolidFill(argb(color, 255), &mut brush);
                    gpu_dispatch::GdipFillPath(canvas, brush.cast(), path);
                    gpu_dispatch::GdipDeleteBrush(brush.cast());
                    GdipDeleteGraphics(canvas);
                    gpu_dispatch::GdipDisposeImage(bitmap.cast());

                    RASTER_LYRICS.with(|cache| cache.borrow_mut().clear());
                    assert!(raster_text(graphics, font, value, rect, color, flags));
                    RASTER_LYRICS.with(|cache| {
                        let cache = cache.borrow();
                        let raster = &cache.back().unwrap().1;
                        let bounds = raster.bounds;
                        let crop_width = (bounds.right - bounds.left) as usize;
                        let mut reconstructed = vec![0u32; reference.len()];
                        for row in bounds.top..bounds.bottom {
                            let src = (row - bounds.top) as usize * crop_width;
                            let dst = row as usize * width as usize + bounds.left as usize;
                            reconstructed[dst..dst + crop_width]
                                .copy_from_slice(&raster.image.pixels[src..src + crop_width]);
                        }
                        assert_eq!(
                            reconstructed, reference,
                            "text {value:?}, flags {flags}, position ({x}, {y})"
                        );
                    });
                }
                RASTER_LYRICS.with(|cache| cache.borrow_mut().clear());
                drop(frame);
                DeleteObject(font);
                DeleteDC(dc);
            }
        }

        #[test]
        fn lyric_budget_grows_for_large_entries_but_has_a_ceiling() {
            let base = 4 * 1024 * 1024;
            assert_eq!(
                super::lyric_cache_budget(base, "DEEZNATS_TEST_UNUSED_BUDGET", 1024),
                base
            );
            assert_eq!(
                super::lyric_cache_budget(base, "DEEZNATS_TEST_UNUSED_BUDGET", 384 * 1024),
                6 * 1024 * 1024
            );
            assert_eq!(
                super::lyric_cache_budget(base, "DEEZNATS_TEST_UNUSED_BUDGET", 2 * 1024 * 1024),
                2 * base
            );
        }

        unsafe fn prepared_test_image(
            dc: HDC,
            rect: RECT,
            source: &crate::catalog::Bitmap,
            radius: f32,
            alpha: u8,
        ) -> bool {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            loop {
                poll_prepared_artwork();
                let result = image(dc, rect, source, radius, alpha);
                if !artwork_preparation_pending() {
                    return result;
                }
                assert!(
                    std::time::Instant::now() < deadline,
                    "artwork preparation timed out"
                );
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
        }
        #[test]
        fn surface_opacity_scales_backgrounds_once_and_restores_nested_scopes() {
            assert_eq!(surface_color(0xffffffff), 0xffffffff);
            {
                let _surface = SurfaceOpacity::set(40);
                assert_eq!(surface_color(0xffffffff), 0x66ffffff);
                assert_eq!(surface_color(0x80aabbcc), 0x33aabbcc);
                {
                    let _surface = SurfaceOpacity::set(0);
                    assert_eq!(surface_color(0xffabcdef), 0x00abcdef);
                }
                assert_eq!(surface_color(0xffffffff), 0x66ffffff);
            }
            assert_eq!(surface_color(0xffffffff), 0xffffffff);
        }
        #[test]
        fn visualizer_preserves_device_resolution_and_foreground_layer() {
            unsafe {
                let dc = CreateCompatibleDC(null_mut());
                let bitmap = CreateBitmap(320, 120, 1, 32, null());
                let previous = SelectObject(dc, bitmap);
                let mut pixels = vec![0u32; 320 * 120];
                let frame =
                    Frame::begin(dc, pixels.as_mut_ptr().cast(), 320, 320, 120, false).unwrap();
                let before = CURRENT.get();
                recessed_visualizer(dc, 320, 120, || {
                    fill(
                        dc,
                        RECT {
                            left: 100,
                            top: 30,
                            right: 140,
                            bottom: 80,
                        },
                        0xffffffff,
                        0.0,
                    );
                });
                assert_eq!(CURRENT.get(), before);
                let mut foreground: RECT = zeroed();
                foreground.left = 200;
                foreground.top = 30;
                foreground.right = 240;
                foreground.bottom = 80;
                fill(dc, foreground, 0xffffffff, 0.0);
                drop(frame);
                let alpha = |x: usize, y: usize| pixels[y * 320 + x] >> 24;
                assert!(
                    alpha(120, 50) == 255,
                    "visualizer must preserve crisp device-resolution coverage"
                );
                assert_eq!(
                    alpha(80, 50),
                    0,
                    "depth must not bleed into unrelated content"
                );
                assert_eq!(alpha(220, 50), 255, "foreground must stay sharp and opaque");
                SelectObject(dc, previous);
                DeleteObject(bitmap);
                DeleteDC(dc);
            }
        }

        #[test]
        fn lyric_paths_never_ellipsize_and_blur_is_cached_filled_coverage() {
            unsafe {
                set_lyric_cache_scope(Some(("Spotify", "track-a")));
                let dc = CreateCompatibleDC(null_mut());
                let mut lf: LOGFONTW = zeroed();
                lf.lfHeight = -36;
                lf.lfWeight = 700;
                for (target, value) in lf.lfFaceName.iter_mut().zip("Segoe UI".encode_utf16()) {
                    *target = value;
                }
                let font = CreateFontIndirectW(&lf);
                let mut pixels = vec![0u32; 400 * 80];
                let frame =
                    Frame::begin(dc, pixels.as_mut_ptr().cast(), 400, 400, 80, false).unwrap();
                let small = RECT {
                    left: 10,
                    top: 10,
                    right: 25,
                    bottom: 65,
                };
                let path =
                    cached_text_path(font, "Have", small, DT_LEFT | LYRICS_UNTRIMMED).unwrap();
                let mut bounds: RectF = zeroed();
                GdipGetPathWorldBounds(path, &mut bounds, null_mut(), null_mut());
                assert!(bounds.Width > 65.0, "full word was replaced by an ellipsis");
                let rect = RECT {
                    left: 10,
                    top: 10,
                    right: 390,
                    bottom: 65,
                };
                assert!(text(
                    dc,
                    font,
                    "All your lies",
                    rect,
                    0xffffff,
                    DT_LEFT | LYRICS_UNTRIMMED
                ));
                assert!(lyric_shadow(dc, font, "All your lies", rect, DT_LEFT));
                assert!(lyric_text_width(dc, font, "All your lies") > 0);
                assert!(blurred_text(
                    dc,
                    font,
                    "All your lies",
                    rect,
                    0xffffff,
                    DT_LEFT,
                    2.0
                ));
                let cached = BLURRED_TEXT.with(|c| {
                    let c = c.borrow();
                    let entry = &c.back().unwrap().1;
                    assert!(
                        entry.pixels.iter().any(|p| p >> 24 > 150),
                        "glyph interiors must remain filled"
                    );
                    assert!(
                        entry.pixels.iter().any(|p| {
                            let a = p >> 24;
                            a > 0 && a < 100
                        }),
                        "Gaussian blur must produce a soft fringe"
                    );
                    entry.bitmap
                });
                assert!(blurred_text(
                    dc,
                    font,
                    "All your lies",
                    rect,
                    0xffffff,
                    DT_LEFT,
                    2.0
                ));
                assert_eq!(
                    cached,
                    BLURRED_TEXT.with(|c| c.borrow().back().unwrap().1.bitmap)
                );
                set_lyric_cache_scope(Some(("Spotify", "track-a")));
                assert!(!BLURRED_TEXT.with(|c| c.borrow().is_empty()));
                set_lyric_cache_scope(Some(("Spotify", "track-b")));
                assert!(BLURRED_TEXT.with(|c| c.borrow().is_empty()));
                assert!(RASTER_LYRICS.with(|c| c.borrow().is_empty()));
                assert!(LYRIC_SHADOWS.with(|c| c.borrow().is_empty()));
                assert!(LYRIC_WIDTHS.with(|c| c.borrow().is_empty()));
                assert!(blurred_text(
                    dc,
                    font,
                    "All your lies",
                    rect,
                    0xffffff,
                    DT_LEFT,
                    2.0
                ));
                set_lyric_cache_scope(None);
                assert!(BLURRED_TEXT.with(|c| c.borrow().is_empty()));
                drop(frame);
                DeleteObject(font);
                DeleteDC(dc);
            }
        }

        use super::*;
        #[test]
        fn spotify_logo_preserves_the_frame_for_following_draws() {
            unsafe {
                let dc = CreateCompatibleDC(null_mut());
                let bitmap = CreateBitmap(64, 64, 1, 32, null());
                let previous = SelectObject(dc, bitmap);
                let mut pixels = vec![0u32; 64 * 64];
                {
                    let _frame =
                        Frame::begin(dc, pixels.as_mut_ptr().cast(), 64, 64, 64, false).unwrap();
                    spotify_logo(
                        dc,
                        RECT {
                            left: 0,
                            top: 0,
                            right: 32,
                            bottom: 32,
                        },
                    );
                    fill(
                        dc,
                        RECT {
                            left: 40,
                            top: 40,
                            right: 60,
                            bottom: 60,
                        },
                        0xffffffff,
                        0.0,
                    );
                }
                assert_ne!(
                    pixels[50 * 64 + 50],
                    0,
                    "drawing after the logo must remain visible"
                );
                SelectObject(dc, previous);
                DeleteObject(bitmap);
                DeleteDC(dc);
            }
        }

        #[test]
        fn deezer_logo_draws_visible_shape_inside_its_bounds() {
            unsafe {
                let dc = CreateCompatibleDC(null_mut());
                let bitmap = CreateBitmap(64, 64, 1, 32, null());
                let previous = SelectObject(dc, bitmap);
                let mut outlines = Vec::new();
                for _ in 0..1 {
                    let mut pixels = vec![0u32; 64 * 64];
                    let frame =
                        Frame::begin(dc, pixels.as_mut_ptr().cast(), 64, 64, 64, false).unwrap();
                    assert!(deezer_logo(
                        dc,
                        RECT {
                            left: 8,
                            top: 8,
                            right: 56,
                            bottom: 56
                        }
                    ));
                    drop(frame);
                    assert!(pixels.iter().filter(|pixel| (**pixel >> 24) > 100).count() > 150);
                    assert!(pixels[..8 * 64].iter().all(|pixel| *pixel == 0));
                    outlines.push(pixels);
                }
                assert_eq!(outlines.len(), 1);
                SelectObject(dc, previous);
                DeleteObject(bitmap);
                DeleteDC(dc);
            }
        }
        #[test]
        fn surface_corner_styles_render_square_through_extra_rounded() {
            unsafe {
                let dc = CreateCompatibleDC(null_mut());
                let bitmap = CreateBitmap(120, 48, 1, 32, null());
                let previous = SelectObject(dc, bitmap);
                let mut ui = crate::ui::Ui::default();
                let mut top_edges = Vec::new();
                for style in 0..4 {
                    ui.corner_style = style;
                    let mut pixels = vec![0u32; 120 * 48];
                    let frame =
                        Frame::begin(dc, pixels.as_mut_ptr().cast(), 120, 120, 48, false).unwrap();
                    assert!(fill(
                        dc,
                        RECT {
                            left: 0,
                            top: 0,
                            right: 120,
                            bottom: 48
                        },
                        0xffffffff,
                        ui.corner_px(16) as f32
                    ));
                    drop(frame);
                    top_edges.push(pixels[..120].iter().filter(|p| (**p >> 24) > 200).count());
                    assert_eq!(pixels[24 * 120 + 60], 0xffffffff);
                    if style == 0 {
                        assert_eq!(
                            pixels[0], 0xffffffff,
                            "square style still rounded its corner"
                        );
                    } else {
                        assert_eq!(pixels[0], 0, "rounded surface lost its corner cutout");
                    }
                }
                assert!(
                    top_edges.windows(2).all(|pair| pair[0] > pair[1]),
                    "corner styles must visibly increase rounding: {top_edges:?}"
                );
                SelectObject(dc, previous);
                DeleteObject(bitmap);
                DeleteDC(dc);
            }
        }

        #[test]
        fn resized_cover_cache_obeys_byte_budget_and_retains_recent_covers() {
            unsafe {
                let dc = CreateCompatibleDC(null_mut());
                let mut pixels = vec![0xff000000u32; 16];
                let frame = Frame::begin(dc, pixels.as_mut_ptr().cast(), 4, 4, 4, false).unwrap();
                RESIZED_IMAGES.with(|cache| {
                    for (_, bitmap) in cache.borrow_mut().drain(..) {
                        gpu_dispatch::GdipDisposeImage(bitmap.cast());
                    }
                });
                let sources: Vec<_> = (0..9)
                    .map(|_| crate::catalog::Bitmap::new(1, 1, vec![0xffffffff]))
                    .collect();
                let rect = RECT {
                    left: 0,
                    top: 0,
                    right: 1024,
                    bottom: 1024,
                };
                for source in sources.iter().take(8) {
                    assert!(prepared_test_image(dc, rect, source, 0.0, 255));
                    RESIZED_IMAGES.with(|cache| {
                        let bytes: usize = cache
                            .borrow()
                            .iter()
                            .map(|(key, _)| resized_bytes(*key))
                            .sum();
                        assert!(bytes <= RESIZED_IMAGE_BYTES);
                    });
                }
                assert!(prepared_test_image(dc, rect, &sources[2], 0.0, 255));
                assert!(prepared_test_image(dc, rect, &sources[8], 0.0, 255));
                evict_resized_artwork(&std::collections::HashSet::from([sources[2].id]));
                RESIZED_IMAGES.with(|cache| {
                    let mut cache = cache.borrow_mut();
                    assert!(!cache.iter().any(|(key, _)| key.0 == sources[2].id));
                    assert!(cache.iter().any(|(key, _)| key.0 == sources[8].id));
                    assert!(!cache.iter().any(|(key, _)| key.0 == sources[3].id));
                    for (_, bitmap) in cache.drain(..) {
                        gpu_dispatch::GdipDisposeImage(bitmap.cast());
                    }
                });
                drop(frame);
                DeleteDC(dc);
            }
        }

        #[test]
        fn artwork_cache_retains_dense_lists_and_distinguishes_corner_shapes() {
            unsafe {
                let dc = CreateCompatibleDC(null_mut());
                let target = CreateBitmap(64, 64, 1, 32, null());
                let previous = SelectObject(dc, target);
                let mut pixels = vec![0u32; 64 * 64];
                let frame =
                    Frame::begin(dc, pixels.as_mut_ptr().cast(), 64, 64, 64, false).unwrap();
                RESIZED_IMAGES.with(|cache| {
                    for (_, bitmap) in cache.borrow_mut().drain(..) {
                        gpu_dispatch::GdipDisposeImage(bitmap.cast());
                    }
                });
                let sources: Vec<_> = (0..80)
                    .map(|_| crate::catalog::Bitmap::new(1, 1, vec![0xffffffff]))
                    .collect();
                let rect = RECT {
                    left: 0,
                    top: 0,
                    right: 48,
                    bottom: 48,
                };
                for source in &sources {
                    assert!(prepared_test_image(dc, rect, source, 8.0, 255));
                }
                let cached = || {
                    RESIZED_IMAGES.with(|cache| {
                        cache
                            .borrow()
                            .iter()
                            .map(|(key, _)| *key)
                            .collect::<std::collections::HashSet<_>>()
                    })
                };
                let first = cached();
                assert_eq!(first.len(), sources.len());
                for source in &sources {
                    assert!(prepared_test_image(dc, rect, source, 8.0, 255));
                }
                assert_eq!(
                    cached(),
                    first,
                    "scrolling a dense list must reuse resized covers"
                );

                // One source can appear as both an artist circle and square cover.
                let source = &sources[0];
                let mut corner_pixels = |radius, alpha| {
                    GdipGraphicsClear(frame.graphics, 0);
                    assert!(prepared_test_image(dc, rect, source, radius, alpha));
                    GdipFlush(frame.graphics, FlushIntentionSync);
                    (pixels[0], pixels[24 * 64 + 24])
                };
                let (corner, center) = corner_pixels(0.0, 255);
                assert_eq!(corner, 0xffffffff);
                assert_eq!(center, 0xffffffff);
                let (corner, center) = corner_pixels(24.0, 255);
                assert_eq!(corner, 0);
                assert_eq!(center, 0xffffffff);
                let (corner, center) = corner_pixels(24.0, 128);
                assert_eq!(corner, 0);
                assert!(
                    (127..=129).contains(&(center >> 24)),
                    "fade opacity changed: {center:#x}"
                );
                RESIZED_IMAGES.with(|cache| {
                    for (_, bitmap) in cache.borrow_mut().drain(..) {
                        gpu_dispatch::GdipDisposeImage(bitmap.cast());
                    }
                });
                drop(frame);
                SelectObject(dc, previous);
                DeleteObject(target);
                DeleteDC(dc);
            }
        }

        #[test]
        fn cached_surfaces_preserve_translucent_corners_borders_and_scroll_positions() {
            unsafe {
                let dc = CreateCompatibleDC(null_mut());
                let target = CreateBitmap(192, 80, 1, 32, null());
                let previous = SelectObject(dc, target);
                let mut pixels = vec![0xff123456u32; 192 * 80];
                let frame =
                    Frame::begin(dc, pixels.as_mut_ptr().cast(), 192, 192, 80, false).unwrap();
                RASTER_SURFACES.with(|cache| cache.borrow_mut().clear());
                let _opacity = SurfaceOpacity::set(100);
                let color = 0xb4a060d0;
                let border = 0xd0ffffff;
                for left in [8, 104] {
                    let rect = RECT {
                        left,
                        top: 8,
                        right: left + 64,
                        bottom: 64,
                    };
                    assert!(fill(dc, rect, color, 12.0));
                    assert!(stroke(dc, rect, border, 12.0, 1.2));
                }
                GdipFlush(frame.graphics, FlushIntentionSync);
                assert_eq!(
                    pixels[8 * 192 + 8],
                    0xff123456,
                    "rounded corner must retain the backdrop"
                );
                assert_ne!(
                    pixels[32 * 192 + 32],
                    0xff123456,
                    "translucent panel did not render"
                );
                for y in 4..68 {
                    for x in 4..76 {
                        assert_eq!(
                            pixels[y * 192 + x],
                            pixels[y * 192 + x + 96],
                            "moving a cached surface changed its pixels"
                        );
                    }
                }
                RASTER_SURFACES.with(|cache| {
                    let cache = cache.borrow();
                    assert_eq!(
                        cache.len(),
                        2,
                        "fills and borders should each reuse their raster at a new position"
                    );
                    assert!(
                        cache
                            .iter()
                            .map(|(_, surface)| surface.pixels.len() * 4)
                            .sum::<usize>()
                            <= SURFACE_CACHE_BYTES
                    );
                });
                drop(frame);
                SelectObject(dc, previous);
                DeleteObject(target);
                DeleteDC(dc);
            }
        }

        #[test]
        fn disjoint_clips_preserve_gaps_and_restore_rectangular_compositing() {
            unsafe {
                let dc = CreateCompatibleDC(null_mut());
                let target = CreateBitmap(32, 16, 1, 32, null());
                let previous = SelectObject(dc, target);
                let mut pixels = vec![0xff000000u32; 32 * 16];
                let frame =
                    Frame::begin(dc, pixels.as_mut_ptr().cast(), 32, 32, 16, false).unwrap();
                let region = CreateRectRgn(0, 0, 8, 16);
                let other = CreateRectRgn(24, 0, 32, 16);
                CombineRgn(region, region, other, RGN_OR);
                SelectClipRgn(dc, region);
                let rect = RECT {
                    left: 0,
                    top: 0,
                    right: 32,
                    bottom: 16,
                };
                assert!(fill(dc, rect, 0xffffffff, 0.0));
                GdipFlush(frame.graphics, FlushIntentionSync);
                assert_ne!(pixels[8 * 32 + 4], 0xff000000);
                assert_eq!(pixels[8 * 32 + 16], 0xff000000);
                assert_ne!(pixels[8 * 32 + 28], 0xff000000);
                assert!(!composite_raster(
                    graphics(dc).unwrap(),
                    &[0xffffffff; 512],
                    32,
                    16,
                    0,
                    0
                ));
                SelectClipRgn(dc, null_mut());
                assert!(composite_raster(
                    graphics(dc).unwrap(),
                    &[0xffffffff; 512],
                    32,
                    16,
                    0,
                    0
                ));
                assert_eq!(pixels[8 * 32 + 16], 0xffffffff);
                drop(frame);
                DeleteObject(region);
                DeleteObject(other);
                SelectObject(dc, previous);
                DeleteObject(target);
                DeleteDC(dc);
            }
        }

        #[test]
        fn direct_compositing_respects_clipping_negative_origins_and_padded_stride() {
            unsafe {
                let dc = CreateCompatibleDC(null_mut());
                let target = CreateBitmap(4, 4, 1, 32, null());
                let previous = SelectObject(dc, target);
                let mut pixels = vec![0xff0000ffu32; 6 * 4];
                let frame = Frame::begin(dc, pixels.as_mut_ptr().cast(), 6, 4, 4, false).unwrap();
                IntersectClipRect(dc, 1, 1, 2, 3);
                let graphics = graphics(dc).unwrap();
                let source = vec![0x80800000; 4 * 4];
                assert!(composite_raster(graphics, &source, 4, 4, -1, -1));
                GdipFlush(graphics, FlushIntentionSync);
                for (index, &pixel) in pixels.iter().enumerate() {
                    let expected = if index == 7 || index == 13 {
                        0xff80007f
                    } else {
                        0xff0000ff
                    };
                    assert_eq!(
                        pixel, expected,
                        "compositing touched a clipped pixel or row padding at {index}"
                    );
                }
                drop(frame);
                SelectObject(dc, previous);
                DeleteObject(target);
                DeleteDC(dc);
            }
        }

        #[test]
        fn artwork_workers_do_not_block_other_covers_behind_a_slow_resize() {
            use std::sync::{mpsc, Arc, Mutex};
            use std::time::Duration;
            let (requests, work) = mpsc::sync_channel(8);
            let (finished, ready) = mpsc::sync_channel(8);
            let (started, first_started) = mpsc::sync_channel(1);
            let (release, blocked) = mpsc::sync_channel(1);
            let blocked = Mutex::new(blocked);
            let prepare: ArtworkPreparer = Arc::new(move |key, _image| {
                if key.0 == 1 {
                    let _ = started.send(());
                    let _ = blocked.lock().unwrap().recv();
                }
                None
            });
            spawn_artwork_workers(work, finished, 3, prepare);
            let bitmap = crate::catalog::Bitmap::new(1, 1, vec![0xffffffff]);
            requests.send(((1, 1, 1, 0), bitmap.clone())).unwrap();
            first_started
                .recv_timeout(Duration::from_secs(2))
                .expect("first resize did not start");
            requests.send(((2, 1, 1, 0), bitmap)).unwrap();
            let completed = ready.recv_timeout(Duration::from_secs(2));
            let _ = release.send(());
            let (key, _) = completed.expect("later cover was blocked by an earlier resize");
            assert_eq!(key.0, 2);
        }

        #[test]
        fn artwork_cache_misses_queue_bounded_work_without_resizing_in_paint() {
            unsafe {
                let dc = CreateCompatibleDC(null_mut());
                let target = CreateBitmap(64, 64, 1, 32, null());
                let previous = SelectObject(dc, target);
                let mut pixels = vec![0u32; 64 * 64];
                let frame =
                    Frame::begin(dc, pixels.as_mut_ptr().cast(), 64, 64, 64, false).unwrap();
                let sources: Vec<_> = (0..20)
                    .map(|_| crate::catalog::Bitmap::new(192, 192, vec![0xffffffff; 192 * 192]))
                    .collect();
                let rect = RECT {
                    left: 0,
                    top: 0,
                    right: 64,
                    bottom: 64,
                };
                assert!(image(dc, rect, &sources[0], 8.0, 255));
                GdipFlush(frame.graphics, FlushIntentionSync);
                assert_ne!(
                    pixels[32 * 64 + 32] & 0x00ffffff,
                    0,
                    "a decoded cover must appear before resizing completes"
                );
                assert_eq!(
                    pixels[0] & 0x00ffffff,
                    0,
                    "the immediate cover must preserve rounded corners"
                );
                pixels.fill(0);
                assert!(image(dc, rect, &sources[0], 8.0, 128));
                GdipFlush(frame.graphics, FlushIntentionSync);
                let alpha = pixels[32 * 64 + 32] >> 24;
                assert!(
                    (120..=136).contains(&alpha),
                    "immediate cover must preserve fade opacity, got {alpha}"
                );
                for source in &sources {
                    assert!(image(dc, rect, source, 8.0, 255));
                    assert!(image(dc, rect, source, 8.0, 255));
                }
                RESIZED_IMAGES.with(|cache| {
                    assert!(
                        cache.borrow().is_empty(),
                        "paint must not resize cache misses"
                    )
                });
                ARTWORK_PREPARATION.with(|state| {
                    let state = state.borrow();
                    assert_eq!(
                        state.as_ref().unwrap().pending.len(),
                        8,
                        "bound work and deduplicate repeated paints"
                    );
                });
                assert!(artwork_preparation_pending());
                evict_resized_artwork(&sources.iter().map(|source| source.id).collect());
                assert!(!artwork_preparation_pending());
                // Release both channel ends; the worker discards its in-flight result.
                ARTWORK_PREPARATION.with(|state| {
                    state.borrow_mut().take();
                });
                drop(frame);
                SelectObject(dc, previous);
                DeleteObject(target);
                DeleteDC(dc);
            }
        }
        #[test]
        fn karaoke_lights_progressively_at_nonzero_screen_coordinates() {
            unsafe {
                let dc = CreateCompatibleDC(null_mut());
                let bitmap = CreateBitmap(640, 100, 1, 32, null());
                let previous = SelectObject(dc, bitmap);
                let mut description: LOGFONTW = zeroed();
                description.lfHeight = -32;
                description.lfWeight = 700;
                for (target, value) in description
                    .lfFaceName
                    .iter_mut()
                    .zip("Segoe UI".encode_utf16())
                {
                    *target = value;
                }
                let font = CreateFontIndirectW(&description);
                let render = |progress| {
                    let mut pixels = vec![0xff000000u32; 640 * 100];
                    let frame =
                        Frame::begin(dc, pixels.as_mut_ptr().cast(), 640, 640, 100, false).unwrap();
                    assert!(karaoke_text(
                        dc,
                        font,
                        "Wide words here",
                        progress,
                        RECT {
                            left: 220,
                            top: 30,
                            right: 620,
                            bottom: 90
                        },
                        0x000000ff,
                        0x00808080,
                        0x000000ff,
                        DT_LEFT
                    ));
                    drop(frame);
                    pixels
                        .iter()
                        .filter(|pixel| {
                            let red = (**pixel >> 16) & 255;
                            let green = (**pixel >> 8) & 255;
                            red > green + 20
                        })
                        .count()
                };
                assert_eq!(render(0.0), 0);
                let half = render(0.5);
                let full = render(1.0);
                assert!(
                    half > 20,
                    "progressive text was clipped away at its screen offset"
                );
                assert!(full > half + 20, "word highlighting did not advance");
                SelectObject(dc, previous);
                DeleteObject(font);
                DeleteObject(bitmap);
                DeleteDC(dc);
            }
        }
        #[test]
        fn premultiplied_canvas_preserves_backdrop_and_blends_foreground() {
            unsafe {
                let dc = CreateCompatibleDC(null_mut());
                let mut pixels = vec![0xff80c040u32; 16];
                let frame = Frame::begin(dc, pixels.as_mut_ptr().cast(), 4, 4, 4, false).unwrap();
                assert!(fill(
                    dc,
                    RECT {
                        left: 0,
                        top: 0,
                        right: 4,
                        bottom: 4
                    },
                    0x800000ff,
                    0.0
                ));
                drop(frame);
                assert_eq!(pixels[15], 0xff80c040);
                assert_ne!(pixels[0], 0xff80c040);
            }
        }
    }
}

pub(crate) mod appearance {
    //! Bounded background preparation, independent of UI and audio threads.
    use crate::catalog::Bitmap;
    use std::sync::{mpsc, Arc};

    pub const PREPARE_LIMIT: u32 = 512;

    pub struct Request {
        pub key: String,
        pub source: Arc<Bitmap>,
        pub blur: u32,
    }
    pub struct Worker {
        pub requests: mpsc::SyncSender<Request>,
        pub updates: mpsc::Receiver<(String, Arc<Bitmap>)>,
    }
    impl Worker {
        pub fn start() -> Self {
            let (requests, rx) = mpsc::sync_channel::<Request>(1);
            let (tx, updates) = mpsc::sync_channel(1);
            std::thread::spawn(move || {
                crate::catalog::artwork_background_priority();
                while let Ok(mut request) = rx.recv() {
                    while let Ok(newer) = rx.try_recv() {
                        request = newer;
                    }
                    let texture = prepare(&request.source, request.blur);
                    if tx.send((request.key, Arc::new(texture))).is_err() {
                        break;
                    }
                }
            });
            Self { requests, updates }
        }
    }
    pub fn prepare(source: &Bitmap, blur: u32) -> Bitmap {
        prepare_limited(source, blur, PREPARE_LIMIT)
    }

    pub fn prepare_genre_photo(source: &Bitmap) -> Bitmap {
        prepare_limited(source, 30, 1024)
    }

    pub fn prepare_style_mix_photo(source: &Bitmap) -> Bitmap {
        const BLUR_PERCENT: u32 = 30;
        prepare_limited(source, BLUR_PERCENT, source.width.max(source.height).max(1))
    }

    fn prepare_limited(source: &Bitmap, blur: u32, limit: u32) -> Bitmap {
        if blur == 0 {
            return source.clone();
        }
        if source.width == 0 || source.height == 0 {
            return source.clone();
        }
        // Keep packed DIB pixels throughout. Three running-sum box passes approximate
        // a Gaussian in linear time, independent of the requested blur radius.
        let (w, h, mut pixels) = downscale(source, limit);
        let scale = w.max(h) as f64 / source.width.max(source.height) as f64;
        let sigma = (blur.min(100) as f64 * 0.35 * scale).max(0.5);
        let ideal = (4.0 * sigma * sigma + 1.0).sqrt();
        let lower = (ideal.floor() as usize).max(1) | 1;
        let lower = if lower as f64 > ideal {
            lower.saturating_sub(2).max(1)
        } else {
            lower
        };
        let upper = lower + 2;
        let l = lower as f64;
        let lower_passes = ((12.0 * sigma * sigma - 3.0 * l * l - 12.0 * l - 9.0)
            / (-4.0 * l - 4.0))
            .round()
            .clamp(0.0, 3.0) as usize;
        // At the minimum sigma, rounding must not turn a requested blur into a no-op.
        let lower_passes = if lower == 1 {
            lower_passes.min(2)
        } else {
            lower_passes
        };
        let mut scratch = vec![0; pixels.len()];
        let mut columns = vec![0u64; w as usize];
        for pass in 0..3 {
            let radius = (if pass < lower_passes { lower } else { upper }) / 2;
            if radius != 0 {
                box_blur_horizontal(&pixels, &mut scratch, w as usize, radius);
                box_blur_vertical(
                    &scratch,
                    &mut pixels,
                    w as usize,
                    h as usize,
                    radius,
                    &mut columns,
                );
            }
        }
        Bitmap::new(w, h, pixels)
    }

    #[cfg(test)]
    fn channels(pixel: u32) -> [u32; 3] {
        [pixel & 255, (pixel >> 8) & 255, (pixel >> 16) & 255]
    }

    // Three independent 21-bit lanes allow RGB running sums with one integer add.
    #[inline]
    fn packed_channels(pixel: u32) -> u64 {
        (pixel & 255) as u64
            | ((((pixel >> 8) & 255) as u64) << 21)
            | ((((pixel >> 16) & 255) as u64) << 42)
    }

    #[inline]
    fn unpack_sum(sum: u64) -> [u32; 3] {
        [
            (sum & 0x1fffff) as u32,
            ((sum >> 21) & 0x1fffff) as u32,
            (sum >> 42) as u32,
        ]
    }

    struct Average {
        rounding: u32,
        reciprocal: u64,
    }
    impl Average {
        fn new(count: u32) -> Self {
            // 40 fractional bits make the rounded reciprocal exact for all channel
            // sums here (at most 8192 samples), without per-channel integer division.
            debug_assert!(count > 0 && count <= 8192);
            Self {
                rounding: count / 2,
                reciprocal: (1u64 << 40).div_ceil(count as u64),
            }
        }
        #[inline]
        fn channel(&self, sum: u32) -> u32 {
            (((sum + self.rounding) as u64 * self.reciprocal) >> 40) as u32
        }
        #[inline]
        fn pixel(&self, sum: [u32; 3]) -> u32 {
            0xff000000
                | self.channel(sum[0])
                | (self.channel(sum[1]) << 8)
                | (self.channel(sum[2]) << 16)
        }
    }

    fn downscale(source: &Bitmap, limit: u32) -> (u32, u32, Vec<u32>) {
        let max_dim = source.width.max(source.height);
        if max_dim <= limit {
            return (
                source.width,
                source.height,
                source.pixels.iter().map(|p| p | 0xff000000).collect(),
            );
        }
        let dimension = |n: u32| {
            ((n as u64 * limit as u64 + max_dim as u64 / 2) / max_dim as u64).max(1) as u32
        };
        let (w, h) = (dimension(source.width), dimension(source.height));
        let mut pixels = Vec::with_capacity((w * h) as usize);
        // Retain the existing sampling grid, with integer coordinates computed once
        // per column. Preparation stays bounded even for very large source images.
        let x_lut: Vec<_> = (0..w)
            .map(|x| {
                (x as u64 * max_dim as u64 / limit as u64).min(source.width as u64 - 1) as usize
            })
            .collect();
        for y in 0..h {
            let sy =
                (y as u64 * max_dim as u64 / limit as u64).min(source.height as u64 - 1) as usize;
            let row = sy * source.width as usize;
            for &sx in &x_lut {
                pixels.push(source.pixels[row + sx] | 0xff000000);
            }
        }
        (w, h, pixels)
    }

    fn box_blur_horizontal(source: &[u32], dest: &mut [u32], width: usize, radius: usize) {
        let average = Average::new((radius * 2 + 1) as u32);
        for (row, out) in source.chunks_exact(width).zip(dest.chunks_exact_mut(width)) {
            let mut sum = packed_channels(row[0]) * (radius + 1) as u64;
            for x in 1..=radius {
                sum += packed_channels(row[x.min(width - 1)]);
            }
            for (x, pixel) in out.iter_mut().enumerate() {
                *pixel = average.pixel(unpack_sum(sum));
                let remove = packed_channels(row[x.saturating_sub(radius)]);
                let add = packed_channels(row[(x + radius + 1).min(width - 1)]);
                sum = sum - remove + add;
            }
        }
    }

    fn box_blur_vertical(
        source: &[u32],
        dest: &mut [u32],
        width: usize,
        height: usize,
        radius: usize,
        columns: &mut [u64],
    ) {
        let average = Average::new((radius * 2 + 1) as u32);
        for (sum, &pixel) in columns.iter_mut().zip(&source[..width]) {
            *sum = packed_channels(pixel) * (radius + 1) as u64;
        }
        for y in 1..=radius {
            let start = y.min(height - 1) * width;
            for (sum, &pixel) in columns.iter_mut().zip(&source[start..start + width]) {
                *sum += packed_channels(pixel);
            }
        }
        // Traverse rows contiguously instead of walking strided columns.
        for (y, out) in dest.chunks_exact_mut(width).enumerate() {
            let remove = y.saturating_sub(radius) * width;
            let add = (y + radius + 1).min(height - 1) * width;
            for (x, (pixel, sum)) in out.iter_mut().zip(columns.iter_mut()).enumerate() {
                *pixel = average.pixel(unpack_sum(*sum));
                *sum =
                    *sum - packed_channels(source[remove + x]) + packed_channels(source[add + x]);
            }
        }
    }
    pub fn cover_sample(image: &Bitmap, x: u32, y: u32, width: u32, height: u32) -> u32 {
        let scale = (width as f64 / image.width as f64).max(height as f64 / image.height as f64);
        let sx = ((x as f64 + (image.width as f64 * scale - width as f64) / 2.0) / scale) as u32;
        let sy = ((y as f64 + (image.height as f64 * scale - height as f64) / 2.0) / scale) as u32;
        image.pixels[(sy.min(image.height - 1) * image.width + sx.min(image.width - 1)) as usize]
    }
    // Quantized dominant color from at most 4096 pixels; never scans artwork per frame.
    pub fn artwork_color(image: &Bitmap) -> u32 {
        let mut bins = [[0u64; 4]; 512];
        let step = image.pixels.len().div_ceil(4096).max(1);
        for &pixel in image.pixels.iter().step_by(step) {
            if pixel >> 24 < 128 {
                continue;
            }
            let r = (pixel >> 16) & 255;
            let g = (pixel >> 8) & 255;
            let b = pixel & 255;
            let hi = r.max(g).max(b);
            let lo = r.min(g).min(b);
            if hi < 35 || lo > 235 {
                continue;
            }
            let weight = (hi - lo + 16) as u64;
            let bin = &mut bins[((r >> 5) * 64 + (g >> 5) * 8 + (b >> 5)) as usize];
            bin[0] += weight;
            bin[1] += r as u64 * weight;
            bin[2] += g as u64 * weight;
            bin[3] += b as u64 * weight;
        }
        let bin = bins.iter().max_by_key(|bin| bin[0]).unwrap();
        if bin[0] == 0 {
            return 0x00a08060;
        }
        ((bin[1] / bin[0]) | ((bin[2] / bin[0]) << 8) | ((bin[3] / bin[0]) << 16)) as u32
    }

    pub fn artwork_luminance(image: &Bitmap) -> f32 {
        if image.pixels.is_empty() {
            return 0.15;
        }
        let step = image.pixels.len().div_ceil(1024).max(1);
        let mut total_lum = 0.0f32;
        let mut count = 0usize;
        for &pixel in image.pixels.iter().step_by(step) {
            if pixel >> 24 < 128 {
                continue;
            }
            let r = ((pixel >> 16) & 255) as f32 / 255.0;
            let g = ((pixel >> 8) & 255) as f32 / 255.0;
            let b = (pixel & 255) as f32 / 255.0;
            total_lum += 0.2126 * r + 0.7152 * g + 0.0722 * b;
            count += 1;
        }
        if count == 0 {
            0.15
        } else {
            total_lum / count as f32
        }
    }

    pub fn colorref_luminance(color: u32) -> f32 {
        let r = (color & 255) as f32 / 255.0;
        let g = ((color >> 8) & 255) as f32 / 255.0;
        let b = ((color >> 16) & 255) as f32 / 255.0;
        0.2126 * r + 0.7152 * g + 0.0722 * b
    }

    /// Keep a preferred foreground when it is readable across a gradient; otherwise
    /// choose the black or white foreground with the best worst-case contrast.
    pub fn gradient_foreground(preferred: u32, start: u32, end: u32) -> u32 {
        fn luminance(color: u32) -> f64 {
            [0, 8, 16]
                .into_iter()
                .zip([0.2126, 0.7152, 0.0722])
                .map(|(shift, weight)| {
                    let channel = ((color >> shift) & 255) as f64 / 255.0;
                    weight
                        * if channel <= 0.04045 {
                            channel / 12.92
                        } else {
                            ((channel + 0.055) / 1.055).powf(2.4)
                        }
                })
                .sum()
        }
        let contrast = |fg: u32, bg: u32| {
            let (a, b) = (luminance(fg), luminance(bg));
            (a.max(b) + 0.05) / (a.min(b) + 0.05)
        };
        let worst = |fg| contrast(fg, start).min(contrast(fg, end));
        if worst(preferred) >= 4.5 {
            preferred
        } else if worst(0x00000000) > worst(0x00ffffff) {
            0x00000000
        } else {
            0x00ffffff
        }
    }

    /// Shared desktop/browser text protection for the actual backdrop lightness.
    pub fn readable_palette(mut palette: [u32; 6], light: bool, brightness: u32) -> [u32; 6] {
        let rgb = |r: u32, g: u32, b: u32| r | (g << 8) | (b << 16);
        if light {
            for color in &mut palette[..4] {
                let mut adjusted = 0;
                for shift in [0, 8, 16] {
                    adjusted |= (((*color >> shift) & 255) * brightness / 100).min(255) << shift;
                }
                *color = adjusted;
            }
            let text_lum = crate::appearance::colorref_luminance(palette[4]);
            if text_lum > 0.30 {
                palette[4] = rgb(20, 20, 24);
            }
            let muted_lum = crate::appearance::colorref_luminance(palette[5]);
            if muted_lum > 0.52 || muted_lum < 0.15 {
                palette[5] = rgb(94, 102, 118);
            }
        } else {
            let text_lum = crate::appearance::colorref_luminance(palette[4]);
            if text_lum < 0.70 {
                palette[4] = rgb(248, 248, 248);
            }
            let muted_lum = crate::appearance::colorref_luminance(palette[5]);
            if muted_lum < 0.38 || muted_lum > 0.85 {
                palette[5] = rgb(190, 190, 190);
            }
        }
        palette
    }

    pub fn adaptive_color(seed: u32, light: bool, key: &str) -> Option<u32> {
        let mix = |base: u32, percent: u32| {
            let mut color = 0;
            for shift in [0, 8, 16] {
                color |= ((((seed >> shift) & 255) * percent
                    + ((base >> shift) & 255) * (100 - percent))
                    / 100)
                    << shift;
            }
            color
        };
        Some(match key {
            "Background" => mix(
                if light { 0x00ffffff } else { 0x000c0c0c },
                if light { 10 } else { 12 },
            ),
            "Panel" => mix(
                if light { 0x00ffffff } else { 0x00191919 },
                if light { 7 } else { 16 },
            ),
            "Raised" => mix(
                if light { 0x00ffffff } else { 0x002c2c2c },
                if light { 17 } else { 22 },
            ),
            "Border" => mix(if light { 0x00aaaaaa } else { 0x00505050 }, 24),
            "Text" | "Icon" => {
                if light {
                    0x00141418
                } else {
                    0x00f5f5f5
                }
            }
            "Muted" => {
                if light {
                    0x00555555
                } else {
                    0x00bcbcbc
                }
            }
            "Accent" | "GradientStart" => mix(0, 40),
            "GradientEnd" => mix(0, 28),
            "AccentText" => {
                let accent = mix(0, 40);
                if colorref_luminance(accent) > 0.45 {
                    0x00141418
                } else {
                    0x00ffffff
                }
            }
            _ => return None,
        })
    }
    #[cfg(test)]
    mod tests {
        use super::*;
        #[test]
        fn shared_palette_guards_repair_unreadable_text_without_changing_surfaces() {
            let dark = [
                0x00101010, 0x00202020, 0x00303030, 0x00404040, 0x00000000, 0x00ffffff,
            ];
            let guarded = readable_palette(dark, false, 100);
            assert_eq!(&guarded[..4], &dark[..4]);
            assert_eq!(guarded[4], 0x00f8f8f8);
            assert_eq!(guarded[5], 0x00bebebe);
            let light = [
                0x00ffffff, 0x00eeeeee, 0x00dddddd, 0x00cccccc, 0x00ffffff, 0x00000000,
            ];
            let guarded = readable_palette(light, true, 100);
            assert_eq!(&guarded[..4], &light[..4]);
            assert_eq!(guarded[4], 0x00181414);
            assert_eq!(guarded[5], 0x0076665e);
        }

        #[test]
        fn downscale_keeps_bounds_and_extreme_aspect_ratios() {
            let reduced = prepare(&Bitmap::new(1024, 1024, vec![0xff808080; 1024 * 1024]), 40);
            assert_eq!(
                (reduced.width, reduced.height),
                (PREPARE_LIMIT, PREPARE_LIMIT)
            );
            assert!(reduced.pixels.iter().all(|&p| p == 0xff808080));
            for (w, h) in [
                (1, 6000),
                (6000, 1),
                (1, 1),
                (PREPARE_LIMIT, PREPARE_LIMIT),
                (PREPARE_LIMIT + 1, 96),
            ] {
                let source = Bitmap::new(w, h, vec![0xff2878c8; (w * h) as usize]);
                let result = prepare(&source, 100);
                assert!(result.width > 0 && result.height > 0);
                assert!(result.width.max(result.height) <= PREPARE_LIMIT);
                assert_eq!(result.pixels.len(), (result.width * result.height) as usize);
                assert!(result.pixels.iter().all(|&p| p == 0xff2878c8));
            }
        }

        #[test]
        fn running_blur_matches_clamped_spatial_average_on_thin_images() {
            let average = |sum: [u32; 3], count: u32| {
                0xff000000
                    | ((sum[0] + count / 2) / count)
                    | (((sum[1] + count / 2) / count) << 8)
                    | (((sum[2] + count / 2) / count) << 16)
            };
            for count in 1..=71 {
                let reciprocal = Average::new(count);
                for sum in 0..=255 * count {
                    assert_eq!(reciprocal.channel(sum), (sum + count / 2) / count);
                }
            }
            for (w, h) in [(1, 1), (1, 7), (9, 1), (7, 5)] {
                let source: Vec<u32> = (0..w * h)
                    .map(|i| 0xff000000 | (i as u32 * 0x030507))
                    .collect();
                for radius in [1, 3, 12] {
                    let mut horizontal = vec![0; source.len()];
                    let mut actual = vec![0; source.len()];
                    box_blur_horizontal(&source, &mut horizontal, w, radius);
                    box_blur_vertical(&horizontal, &mut actual, w, h, radius, &mut vec![0u64; w]);
                    for y in 0..h {
                        for x in 0..w {
                            let mut horizontal_sum = [0u32; 3];
                            for k in -(radius as isize)..=radius as isize {
                                let sample_x = (x as isize + k).clamp(0, w as isize - 1) as usize;
                                let c = channels(source[y * w + sample_x]);
                                for i in 0..3 {
                                    horizontal_sum[i] += c[i];
                                }
                            }
                            let mut vertical_sum = [0u32; 3];
                            for k in -(radius as isize)..=radius as isize {
                                let sample_y = (y as isize + k).clamp(0, h as isize - 1) as usize;
                                let mut horizontal_sum = [0u32; 3];
                                for j in -(radius as isize)..=radius as isize {
                                    let sample_x =
                                        (x as isize + j).clamp(0, w as isize - 1) as usize;
                                    let c = channels(source[sample_y * w + sample_x]);
                                    for i in 0..3 {
                                        horizontal_sum[i] += c[i];
                                    }
                                }
                                let c = channels(average(horizontal_sum, (radius * 2 + 1) as u32));
                                for i in 0..3 {
                                    vertical_sum[i] += c[i];
                                }
                            }
                            assert_eq!(
                                actual[y * w + x],
                                average(vertical_sum, (radius * 2 + 1) as u32)
                            );
                        }
                    }
                }
            }
        }

        #[test]
        fn gaussian_approximation_preserves_a_smooth_color_gradient() {
            let detail = Bitmap::new(2, 1, vec![0xff000000, 0xffffffff]);
            assert_ne!(prepare(&detail, 1).pixels, detail.pixels);
            let source = Bitmap::new(
                64,
                64,
                (0..64 * 64)
                    .map(|i| 0xff000000 | ((i % 64 * 4) << 16) | ((i / 64 * 4) << 8) | 120)
                    .collect(),
            );
            let rgba = image::RgbaImage::from_fn(64, 64, |x, y| {
                image::Rgba([(x * 4) as u8, (y * 4) as u8, 120, 255])
            });
            for blur in [1, 40, 70, 100] {
                let actual = prepare(&source, blur);
                let gaussian = image::imageops::blur(&rgba, (blur as f32 * 0.35).max(0.5));
                let error: f64 = actual
                    .pixels
                    .iter()
                    .zip(gaussian.pixels())
                    .map(|(&p, q)| {
                        let dr = ((p >> 16) & 255) as f64 - q[0] as f64;
                        let dg = ((p >> 8) & 255) as f64 - q[1] as f64;
                        (dr * dr + dg * dg) / 2.0
                    })
                    .sum::<f64>()
                    / actual.pixels.len() as f64;
                assert!(error.sqrt() < 16.0, "blur={blur}, RMSE={}", error.sqrt());
            }
            assert_eq!(
                prepare(&source, 100).pixels,
                prepare(&source, u32::MAX).pixels
            );
        }
        #[test]
        fn artwork_palettes_preserve_hue_and_readable_text_in_both_modes() {
            let image = Bitmap::new(64, 64, vec![0xffd04020; 4096]);
            assert_eq!(artwork_color(&image), 0x002040d0);
            let luminance = |color: u32| {
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
            for seed in [0, 0xffffff, 0x0000ff, 0x00ff00, 0xff0000, 0x2030d0] {
                for light in [false, true] {
                    for surface in ["Background", "Panel", "Raised"] {
                        let a = luminance(adaptive_color(seed, light, surface).unwrap());
                        let b = luminance(adaptive_color(seed, light, "Text").unwrap());
                        assert!((a.max(b) + 0.05) / (a.min(b) + 0.05) >= 4.5);
                    }
                    let accent = luminance(adaptive_color(seed, light, "Accent").unwrap());
                    assert!((1.0 + 0.05) / (accent + 0.05) >= 4.5);
                }
            }
            assert_eq!(artwork_color(&Bitmap::new(2, 2, vec![0; 4])), 0x00a08060);
        }
        #[test]
        fn artwork_and_colorref_luminance_accurately_detect_brightness() {
            let dark_image = Bitmap::new(4, 4, vec![0xff101010; 16]);
            assert!(artwork_luminance(&dark_image) < 0.15);
            let light_image = Bitmap::new(4, 4, vec![0xfff5f5f5; 16]);
            assert!(artwork_luminance(&light_image) > 0.85);

            assert!(colorref_luminance(0x00ffffff) > 0.95);
            assert!(colorref_luminance(0x00000000) < 0.05);
            assert!(colorref_luminance(0x00141418) < 0.15);
        }

        #[test]
        fn gradient_foreground_checks_both_ends() {
            assert_eq!(
                super::gradient_foreground(0x00141418, 0x0033152c, 0x00200d1c),
                0x00ffffff
            );
            assert_eq!(
                super::gradient_foreground(0x00ffffff, 0x00eeeeee, 0x00ffffff),
                0x00000000
            );
            assert_eq!(
                super::gradient_foreground(0x00eeeeee, 0x00000000, 0x00202020),
                0x00eeeeee
            );
        }
        #[test]
        fn cover_crops_the_center_instead_of_stretching() {
            let image = Bitmap {
                id: 0,
                width: 4,
                height: 2,
                pixels: Arc::new(vec![0, 1, 2, 3, 4, 5, 6, 7]),
            };
            assert_eq!(cover_sample(&image, 0, 0, 2, 2), 1);
            assert_eq!(cover_sample(&image, 1, 1, 2, 2), 6);
        }
        #[test]
        fn blur_preserves_uniform_color_and_zero_preserves_detail() {
            let image = Bitmap {
                id: 1,
                width: 8,
                height: 8,
                pixels: Arc::new(vec![0xff2878c8; 64]),
            };
            assert!(prepare(&image, 100).pixels.iter().all(|p| *p == 0xff2878c8));
            let detail = Bitmap {
                id: 2,
                width: 2,
                height: 1,
                pixels: Arc::new(vec![0xff000000, 0xffffffff]),
            };
            assert_eq!(prepare(&detail, 0).pixels, detail.pixels);
            assert!(Arc::ptr_eq(&prepare(&detail, 0).pixels, &detail.pixels));
        }
    }
}

pub(crate) mod theme {
    //! Portable appearance-only themes. No account, playback or local-path data.
    use crate::ui::Ui;
    use serde_json::{json, Value};
    use std::{collections::HashMap, io::Read, path::Path};

    const LIMIT: usize = 64 * 1024;
    const COLORS: &[&str] = &[
        "Background",
        "Panel",
        "Raised",
        "Border",
        "Text",
        "Muted",
        "Icon",
        "Accent",
        "AccentText",
        "TextHover",
        "GradientStart",
        "GradientEnd",
    ];
    pub struct Theme {
        value: Value,
    }
    fn hex(color: u32) -> String {
        format!(
            "#{:02X}{:02X}{:02X}",
            color & 255,
            (color >> 8) & 255,
            (color >> 16) & 255
        )
    }
    fn color(value: &Value) -> Result<u32, String> {
        let text = value
            .as_str()
            .ok_or("Theme colors must be #RRGGBB strings")?;
        if text.len() != 7
            || !text.starts_with('#')
            || !text[1..].bytes().all(|c| c.is_ascii_hexdigit())
        {
            return Err("Theme colors must use #RRGGBB".into());
        }
        let rgb = u32::from_str_radix(&text[1..], 16).map_err(|_| "Invalid theme color")?;
        Ok(((rgb >> 16) & 255) | (rgb & 0xff00) | ((rgb & 255) << 16))
    }
    fn palette(value: &Value) -> Result<HashMap<String, u32>, String> {
        let map = value.as_object().ok_or("Theme palette must be an object")?;
        if map.len() > COLORS.len() {
            return Err("Too many theme palette entries".into());
        }
        map.iter()
            .map(|(key, value)| {
                if !COLORS.contains(&key.as_str()) {
                    return Err(format!("Unknown theme color: {key}"));
                }
                Ok((key.clone(), color(value)?))
            })
            .collect()
    }
    impl Theme {
        pub fn capture(ui: &Ui) -> Self {
            let encode = |map: &HashMap<String, u32>| {
                map.iter()
                    .filter(|(k, _)| COLORS.contains(&k.as_str()))
                    .map(|(key, value)| (key.clone(), json!(hex(*value))))
                    .collect::<serde_json::Map<_, _>>()
            };
            Self {
                value: json!({"format":"deeznats-theme","version":1,"name":"Custom theme","light":ui.light,"adaptive_colors":ui.adaptive_colors,
            "background":{"mode":(["acrylic","artwork","image","solid"][ui.background_mode.min(3)]),
                "blur":ui.blur,"tint_opacity":ui.light_opacity,"tint":hex(ui.light_tint),
                "color":hex(ui.solid_color),"brightness":ui.brightness},
            "colors":{"dark":encode(&ui.dark_colors),"light":encode(&ui.light_colors)},
            "corners":ui.corner_style,"solid_theme":ui.solid_theme,"window_blur":ui.window_blur,"ui_blur":ui.ui_blur,
            "player":{"layout":ui.player_layout,"transparent":ui.transparent_player,"opacity":ui.surface_opacity}}),
            }
        }
        pub fn parse(bytes: &[u8]) -> Result<Self, String> {
            if bytes.len() > LIMIT {
                return Err("Theme file exceeds 64 KiB".into());
            }
            let value: Value = serde_json::from_slice(bytes).map_err(|_| "Invalid theme JSON")?;
            if value["format"] != "deeznats-theme" || value["version"] != 1 {
                return Err("Unsupported DeezNats theme format/version".into());
            }
            let name = value["name"].as_str().ok_or("Theme name is required")?;
            if name.trim().is_empty()
                || name.chars().count() > 80
                || name.chars().any(char::is_control)
            {
                return Err("Invalid theme name".into());
            }
            let theme = Self { value };
            // Validate the complete document before touching the active appearance.
            theme.apply_fields(&mut Ui::default())?;
            Ok(theme)
        }
        pub fn read(path: &Path) -> Result<Self, String> {
            let file = std::fs::File::open(path).map_err(|e| format!("Cannot open theme: {e}"))?;
            let mut bytes = Vec::new();
            file.take((LIMIT + 1) as u64)
                .read_to_end(&mut bytes)
                .map_err(|e| format!("Cannot read theme: {e}"))?;
            Self::parse(&bytes)
        }
        pub fn write(&self, path: &Path) -> Result<(), String> {
            let bytes =
                serde_json::to_vec_pretty(&self.value).map_err(|_| "Cannot encode theme")?;
            std::fs::write(path, bytes).map_err(|e| format!("Cannot save theme: {e}"))
        }
        pub fn name(&self) -> &str {
            self.value["name"].as_str().unwrap_or("Custom theme")
        }
        pub fn apply(&self, ui: &mut Ui) {
            self.apply_fields(ui).expect("validated theme");
        }
        fn apply_fields(&self, ui: &mut Ui) -> Result<(), String> {
            let v = &self.value;
            let number = |pointer: &str, max: u64| -> Result<u32, String> {
                v.pointer(pointer)
                    .and_then(Value::as_u64)
                    .filter(|n| *n <= max)
                    .map(|n| n as u32)
                    .ok_or_else(|| format!("Invalid theme value: {pointer}"))
            };
            let boolean = |pointer: &str| {
                v.pointer(pointer)
                    .and_then(Value::as_bool)
                    .ok_or_else(|| format!("Invalid theme flag: {pointer}"))
            };
            let mode = match v["background"]["mode"].as_str() {
                Some("acrylic") => 0,
                Some("artwork") => 1,
                Some("image") => 2,
                Some("solid") => 3,
                _ => return Err("Invalid theme background mode".into()),
            };
            let dark = palette(&v["colors"]["dark"])?;
            let light = palette(&v["colors"]["light"])?;
            ui.light = boolean("/light")?;
            ui.blur = number("/background/blur", 100)?;
            ui.light_opacity = number("/background/tint_opacity", 100)?;
            ui.brightness = number("/background/brightness", 110)?;
            ui.light_tint = color(&v["background"]["tint"])?;
            ui.solid_color = color(&v["background"]["color"])?;
            ui.corner_style = number("/corners", 3)? as usize;
            ui.solid_theme =
                number("/solid_theme", (crate::ui::SOLID_THEMES.len() - 1) as u64)? as usize;
            ui.window_blur = boolean("/window_blur")?;
            ui.ui_blur = boolean("/ui_blur")?;
            ui.adaptive_colors = v["adaptive_colors"].as_bool().unwrap_or(false);
            ui.player_layout = number("/player/layout", 1)? as usize;
            ui.transparent_player = boolean("/player/transparent")?;
            ui.surface_opacity = number("/player/opacity", 100)?;
            ui.dark_colors = dark;
            ui.light_colors = light;
            ui.background_mode = mode;
            ui.solid = mode != 0;
            ui.choices.insert("Background", mode);
            Ok(())
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        #[test]
        fn bundled_midnight_theme_is_importable() {
            let theme = Theme::parse(include_bytes!(
                "../../../../themes/Midnight.deeznats-theme.json"
            ))
            .unwrap();
            let mut ui = Ui::default();
            theme.apply(&mut ui);
            assert_eq!(theme.name(), "Midnight");
            assert_eq!(ui.background_mode, 3);
            assert_eq!(ui.color("Accent", 0), 0x00ff9ab5);
        }
        #[test]
        fn portable_theme_roundtrip_keeps_colors_and_leaves_account_and_paths_alone() {
            let mut source = Ui::default();
            source.dark_colors.insert("Accent".into(), 0x00ab3412);
            source.light = true;
            source.adaptive_colors = true;
            source.background_mode = 1;
            source.blur = 37;
            source.background_file = "private-wallpaper.png".into();
            source.account = json!({"private":"account-data"});
            let bytes = serde_json::to_vec(&Theme::capture(&source).value).unwrap();
            let text = String::from_utf8(bytes.clone()).unwrap();
            assert!(!text.contains("private") && !text.contains("account-data"));
            let theme = Theme::parse(&bytes).unwrap();
            let mut target = Ui::default();
            target.background_file = "existing.png".into();
            target.initial_volume = 73;
            theme.apply(&mut target);
            assert_eq!(target.dark_colors["Accent"], 0x00ab3412);
            assert!(target.light);
            assert!(target.adaptive_colors);
            assert_eq!(target.blur, 37);
            assert_eq!(target.background_mode, 1);
            assert_eq!(target.background_file, "existing.png");
            assert_eq!(target.initial_volume, 73);
        }
        #[test]
        fn malformed_or_oversized_themes_are_rejected_before_application() {
            let base = Theme::capture(&Ui::default()).value;
            for (pointer, bad) in [
                ("/version", json!(999)),
                ("/background/blur", json!(101)),
                ("/colors/dark", json!({"Accent":"red"})),
                ("/player/layout", json!(-1)),
                ("/window_blur", json!("yes")),
            ] {
                let mut value = base.clone();
                *value.pointer_mut(pointer).unwrap() = bad;
                assert!(Theme::parse(&serde_json::to_vec(&value).unwrap()).is_err());
            }
            assert!(Theme::parse(&vec![b' '; LIMIT + 1]).is_err());
        }
    }
}

pub(crate) mod visualizer {
    use std::collections::VecDeque;
    use std::ffi::c_void;
    use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::Instant;
    use windows_sys::Win32::System::Com::*;

    #[repr(C)]
    struct Guid {
        data1: u32,
        data2: u16,
        data3: u16,
        data4: [u8; 8],
    }

    const CLSID_MM_DEVICE_ENUMERATOR: Guid = Guid {
        data1: 0xBCDE0395,
        data2: 0xE52F,
        data3: 0x467C,
        data4: [0x8E, 0x3D, 0xC4, 0x57, 0x92, 0x91, 0x69, 0x2E],
    };
    const IID_IMM_DEVICE_ENUMERATOR: Guid = Guid {
        data1: 0xA95664D2,
        data2: 0x9614,
        data3: 0x4F35,
        data4: [0xA7, 0x46, 0xDE, 0x8D, 0xB6, 0x36, 0x17, 0xE6],
    };
    const IID_IAUDIO_CLIENT: Guid = Guid {
        data1: 0x1CB9AD4C,
        data2: 0xDBFA,
        data3: 0x4C32,
        data4: [0xB1, 0x78, 0xC2, 0xF5, 0x68, 0xA7, 0x03, 0xB2],
    };
    const IID_IAUDIO_CAPTURE_CLIENT: Guid = Guid {
        data1: 0xC8ADBD64,
        data2: 0xE71E,
        data3: 0x48A0,
        data4: [0xA4, 0xDE, 0x18, 0x5C, 0x39, 0x5C, 0xD3, 0x17],
    };

    const AUDCLNT_STREAMFLAGS_LOOPBACK: u32 = 0x00020000;
    const AUDCLNT_BUFFERFLAGS_SILENT: u32 = 0x2;

    #[repr(C)]
    struct IMMDeviceEnumeratorVtbl {
        query_interface: usize,
        add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
        release: unsafe extern "system" fn(*mut c_void) -> u32,
        enum_audio_endpoints: usize,
        get_default_audio_endpoint:
            unsafe extern "system" fn(*mut c_void, i32, i32, *mut *mut c_void) -> i32,
    }

    #[repr(C)]
    struct IMMDeviceVtbl {
        query_interface: usize,
        add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
        release: unsafe extern "system" fn(*mut c_void) -> u32,
        activate: unsafe extern "system" fn(
            *mut c_void,
            *const Guid,
            u32,
            *const c_void,
            *mut *mut c_void,
        ) -> i32,
    }

    #[repr(C)]
    struct IAudioClientVtbl {
        query_interface: usize,
        add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
        release: unsafe extern "system" fn(*mut c_void) -> u32,
        initialize: unsafe extern "system" fn(
            *mut c_void,
            i32,
            u32,
            i64,
            i64,
            *const c_void,
            *const Guid,
        ) -> i32,
        get_buffer_size: unsafe extern "system" fn(*mut c_void, *mut u32) -> i32,
        get_stream_latency: usize,
        get_current_padding: usize,
        is_format_supported: usize,
        get_mix_format: unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> i32,
        get_device_period: usize,
        start: unsafe extern "system" fn(*mut c_void) -> i32,
        stop: unsafe extern "system" fn(*mut c_void) -> i32,
        reset: usize,
        set_event_handle: usize,
        get_service: unsafe extern "system" fn(*mut c_void, *const Guid, *mut *mut c_void) -> i32,
    }

    #[repr(C)]
    struct IAudioCaptureClientVtbl {
        query_interface: usize,
        add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
        release: unsafe extern "system" fn(*mut c_void) -> u32,
        get_buffer: unsafe extern "system" fn(
            *mut c_void,
            *mut *mut u8,
            *mut u32,
            *mut u32,
            *mut u64,
            *mut u64,
        ) -> i32,
        release_buffer: unsafe extern "system" fn(*mut c_void, u32) -> i32,
        get_next_packet_size: unsafe extern "system" fn(*mut c_void, *mut u32) -> i32,
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct WaveFormatEx {
        w_format_tag: u16,
        n_channels: u16,
        n_samples_per_sec: u32,
        n_avg_bytes_per_sec: u32,
        n_block_align: u16,
        w_bits_per_sample: u16,
        cb_size: u16,
    }

    pub struct Visualizer {
        active: Arc<AtomicBool>,
        output_volume: Arc<AtomicU32>,
        render_levels: Mutex<([f32; 32], Instant)>,
        levels: Arc<Mutex<[f32; 32]>>,
        scene_levels: Arc<Mutex<[f32; 32]>>,
        trap_state: Mutex<TrapResponse>,
        has_live_signal: Arc<Mutex<bool>>,
        last_update: Arc<Mutex<Instant>>,
    }

    impl Default for Visualizer {
        fn default() -> Self {
            Self::new()
        }
    }

    impl Visualizer {
        pub fn new() -> Self {
            let active = Arc::new(AtomicBool::new(false));
            let active_clone = active.clone();
            let output_volume = Arc::new(AtomicU32::new(1f32.to_bits()));
            let volume_clone = output_volume.clone();
            let levels = Arc::new(Mutex::new([0.0f32; 32]));
            let has_live_signal = Arc::new(Mutex::new(false));
            let last_update = Arc::new(Mutex::new(Instant::now()));

            let scene_levels = Arc::new(Mutex::new([0.; 32]));
            let scene_clone = scene_levels.clone();
            let levels_clone = Arc::clone(&levels);
            let signal_clone = Arc::clone(&has_live_signal);
            let update_clone = Arc::clone(&last_update);

            std::thread::Builder::new()
                .name("deeznats-wasapi-visualizer".into())
                .spawn(move || unsafe {
                    run_wasapi_capture(
                        levels_clone,
                        signal_clone,
                        update_clone,
                        active_clone,
                        volume_clone,
                        scene_clone,
                    );
                })
                .ok();

            Self {
                active,
                output_volume,
                render_levels: Mutex::new(([0.; 32], Instant::now())),
                levels,
                scene_levels,
                trap_state: Mutex::new(TrapResponse::new()),
                has_live_signal,
                last_update,
            }
        }
        pub(crate) fn set_active(&self, active: bool) {
            self.active.store(active, Ordering::Relaxed);
        }

        pub(crate) fn set_output_volume(&self, volume: u32) {
            self.output_volume
                .store((volume.min(100) as f32 / 100.).to_bits(), Ordering::Relaxed);
        }
        #[cfg(test)]
        pub(crate) fn fixture_levels(&self, bands: [f32; 32]) {
            *self.levels.lock().unwrap() = bands;
            *self.scene_levels.lock().unwrap() = bands;
            *self.render_levels.lock().unwrap() = (bands, Instant::now());
            *self.last_update.lock().unwrap() = Instant::now();
            let now = Instant::now();
            let start = now - std::time::Duration::from_secs(1);
            let mut state = self.trap_state.lock().unwrap();
            state.last = start;
            for frame in 1..=60 {
                state.frame(
                    bands,
                    start + std::time::Duration::from_millis(frame * 16),
                    199,
                    80.,
                );
            }
        }
        fn spectrum(&self) -> [f32; 32] {
            let fresh = self
                .last_update
                .lock()
                .map(|at| at.elapsed().as_millis() < 400)
                .unwrap_or(false);
            let raw = crate::connect::visualizer_spectrum().unwrap_or_else(|| {
                if fresh {
                    self.levels.lock().map(|v| *v).unwrap_or([0.; 32])
                } else {
                    [0.; 32]
                }
            });
            let mut state = self.render_levels.lock().unwrap();
            let raw = raw.map(|level| (level * 1.8).clamp(0., 1.));
            let dt = state.1.elapsed().as_secs_f32().min(0.1);
            state.1 = Instant::now();
            follow_levels(&mut state.0, &raw, dt);
            state.0
        }
        pub fn compute_wave(
            &self,
            width: f32,
            height: f32,
            baseline_y: f32,
            playing: bool,
            _position_ms: u64,
            _eq: &[i32; 11],
        ) -> Vec<(f32, f32)> {
            let bands = self.spectrum();
            let max_h = (height * 0.28).clamp(60., 220.);
            let crest = |level: f32| {
                baseline_y
                    - if playing {
                        level.clamp(0.03, 1.0) * max_h
                    } else {
                        0.0
                    }
            };
            // One continuous curve: bass at both edges, treble at the center.
            let count = bands.len() * 2;
            let mut points = Vec::with_capacity(count + 2);
            // Keep curve endpoints and their glow outside the viewport.
            let overscan = (width / count as f32).max(16.);
            points.push((-overscan, crest(bands[0])));
            points.extend((0..count).map(|index| {
                let band = index.min(count - 1 - index);
                (
                    width * (index as f32 + 0.5) / count as f32,
                    crest(bands[band]),
                )
            }));
            points.push((width + overscan, crest(bands[0])));
            points
        }
        pub fn compute_bars(&self, count: usize, _max_h: f32) -> Vec<f32> {
            let bands = self.spectrum();
            (0..count)
                .map(|i| {
                    // Mirror the spectrum around the center: bass in the middle,
                    // treble at both edges.
                    let position = i as f32 / (count.max(2) - 1) as f32;
                    let at = (position * 2. - 1.).abs() * 31.;
                    let low = at.floor() as usize;
                    bands[low] * (1. - at.fract()) + bands[(low + 1).min(31)] * at.fract()
                })
                .collect()
        }
        pub fn compute_trap_nation(&self, count: usize, max_spike: f32) -> (Vec<Vec<f32>>, f32) {
            let raw = crate::connect::visualizer_trap_spectrum().unwrap_or_else(|| {
                if self
                    .last_update
                    .lock()
                    .map(|at| at.elapsed().as_millis() < 400)
                    .unwrap_or(false)
                {
                    *self.scene_levels.lock().unwrap()
                } else {
                    [0.; 32]
                }
            });
            self.trap_state
                .lock()
                .unwrap()
                .frame(raw, Instant::now(), count, max_spike)
        }
    }

    // Translates the supplied scene into time-based smoothing and layer trails.
    // Scene frame delays 0/3/6/9/12 are represented as 0/50/100/150/200 ms.
    struct TrapResponse {
        levels: [f32; 32],
        beat: f32,
        last: Instant,
        history: VecDeque<(Instant, [f32; 32])>,
        kernel_count: Option<usize>,
        kernels: [(Vec<f32>, f32); 5],
        outline_scratch: Vec<f32>,
    }
    impl TrapResponse {
        fn new() -> Self {
            Self {
                levels: [0.; 32],
                beat: 0.,
                last: Instant::now(),
                history: VecDeque::new(),
                kernel_count: None,
                kernels: std::array::from_fn(|_| (Vec::new(), 0.)),
                outline_scratch: Vec::new(),
            }
        }
        fn frame(
            &mut self,
            raw: [f32; 32],
            now: Instant,
            count: usize,
            max_spike: f32,
        ) -> (Vec<Vec<f32>>, f32) {
            let dt = now
                .saturating_duration_since(self.last)
                .as_secs_f32()
                .min(0.1);
            self.last = now;
            let mut raw = raw.map(|level| (level * 1.8).clamp(0., 1.));
            for bass in &mut raw[..5] {
                *bass = (*bass * 1.2).clamp(0., 1.);
            }
            follow_levels(&mut self.levels, &raw, dt);
            let bass = self.levels[..5].iter().copied().fold(0., f32::max);
            self.beat += (bass - self.beat) * (1. - 0.6f32.powf(dt * 60.));
            self.history.push_back((now, self.levels));
            while self.history.len() > 64
                || self
                    .history
                    .front()
                    .is_some_and(|(at, _)| now.saturating_duration_since(*at).as_millis() > 450)
            {
                self.history.pop_front();
            }
            if self.kernel_count != Some(count) {
                for (layer, (weights, sum)) in self.kernels.iter_mut().enumerate() {
                    let radius = (10 + layer * 2).min(count.saturating_sub(1));
                    let sigma = (radius as f32 / 2.4).max(1.);
                    weights.clear();
                    weights.extend(
                        (-(radius as isize)..=radius as isize)
                            .map(|offset| (-(offset as f32).powi(2) / (2. * sigma * sigma)).exp()),
                    );
                    *sum = weights.iter().sum();
                }
                self.kernel_count = Some(count);
            }
            let layers = (0..5)
                .map(|layer| {
                    let target = now
                        .checked_sub(std::time::Duration::from_millis(layer * 50))
                        .unwrap_or(now);
                    let levels = delayed_levels(&self.history, target);
                    let gain = (3.5 + layer as f32 * 0.5) / 3.5;
                    scene_contour(
                        &levels,
                        count,
                        max_spike * gain,
                        &self.kernels[layer as usize],
                        &mut self.outline_scratch,
                    )
                })
                .collect();
            (layers, self.beat)
        }
    }
    fn delayed_levels(history: &VecDeque<(Instant, [f32; 32])>, target: Instant) -> [f32; 32] {
        let Some(first) = history.front() else {
            return [0.; 32];
        };
        if target < first.0 {
            return [0.; 32];
        }
        for index in 1..history.len() {
            let a = &history[index - 1];
            let b = &history[index];
            if target <= b.0 {
                let span = b.0.saturating_duration_since(a.0).as_secs_f32();
                let blend = if span > 0. {
                    target.saturating_duration_since(a.0).as_secs_f32() / span
                } else {
                    1.
                };
                return std::array::from_fn(|i| a.1[i] + (b.1[i] - a.1[i]) * blend);
            }
        }
        history.back().unwrap().1
    }
    fn scene_contour(
        bands: &[f32; 32],
        count: usize,
        height: f32,
        kernel: &(Vec<f32>, f32),
        scratch: &mut Vec<f32>,
    ) -> Vec<f32> {
        let raw = trap_outline_with_scratch(bands, count, height, scratch);
        if count == 0 {
            return raw;
        }
        let (weights, sum) = kernel;
        let radius = weights.len() / 2;
        (0..count)
            .map(|index| {
                weights
                    .iter()
                    .enumerate()
                    .map(|(offset, weight)| raw[(index + count + offset - radius) % count] * weight)
                    .sum::<f32>()
                    / *sum
            })
            .collect()
    }
    fn follow_levels(levels: &mut [f32; 32], targets: &[f32; 32], dt: f32) {
        for (level, target) in levels.iter_mut().zip(targets) {
            let target = target.clamp(0., 1.);
            let tau = if target > *level { 0.018 } else { 0.080 };
            *level += (target - *level) * (1. - (-dt.max(0.) / tau).exp());
        }
    }
    #[cfg(test)]
    fn trap_outline(bands: &[f32; 32], count: usize, max_spike: f32) -> Vec<f32> {
        trap_outline_with_scratch(bands, count, max_spike, &mut Vec::new())
    }
    fn trap_outline_with_scratch(
        bands: &[f32; 32],
        count: usize,
        max_spike: f32,
        raw: &mut Vec<f32>,
    ) -> Vec<f32> {
        if count < 4 {
            return vec![0.; count];
        }
        let sample = |band: f32| {
            let low = band.floor().clamp(0., 31.) as usize;
            bands[low] * (1. - band.fract()) + bands[(low + 1).min(31)] * band.fract()
        };
        raw.clear();
        raw.extend((0..count).map(|k| {
            let angle = k as f32 / count as f32 * std::f32::consts::TAU;
            let band =
                (1. - angle.abs().min(std::f32::consts::TAU - angle) / std::f32::consts::PI) * 31.;
            sample(band.clamp(0., 31.)) * max_spike
        }));
        (0..count)
            .map(|k| {
                (raw[(k + count - 2) % count]
                    + raw[(k + count - 1) % count] * 4.
                    + raw[k] * 6.
                    + raw[(k + 1) % count] * 4.
                    + raw[(k + 2) % count])
                    / 16.
            })
            .collect()
    }
    // Compensate only the known playback volume taper. Never normalize the
    // music's own RMS: doing that turns every quiet phrase into a loud one.
    fn analysis_gain(volume: f32) -> f32 {
        let volume = volume.clamp(0., 1.);
        if volume > 0. {
            1. / volume.powi(3)
        } else {
            0.
        }
    }
    unsafe fn run_wasapi_capture(
        levels: Arc<Mutex<[f32; 32]>>,
        has_live_signal: Arc<Mutex<bool>>,
        last_update: Arc<Mutex<Instant>>,
        active: Arc<AtomicBool>,
        output_volume: Arc<AtomicU32>,
        scene_levels: Arc<Mutex<[f32; 32]>>,
    ) {
        let com_result = CoInitializeEx(std::ptr::null_mut(), 0);
        if com_result < 0 {
            return;
        }
        loop {
            if active.load(Ordering::Relaxed) {
                capture_session(
                    &levels,
                    &has_live_signal,
                    &last_update,
                    &active,
                    &output_volume,
                    &scene_levels,
                );
            }
            if let Ok(mut value) = levels.lock() {
                *value = [0.; 32];
            }
            if let Ok(mut value) = scene_levels.lock() {
                *value = [0.; 32];
            }
            if let Ok(mut value) = has_live_signal.lock() {
                *value = false;
            }
            // Retry failed/invalidated endpoints, and reopen after a window restore.
            std::thread::sleep(std::time::Duration::from_millis(250));
        }
    }

    unsafe fn capture_session(
        levels: &Mutex<[f32; 32]>,
        has_live_signal: &Mutex<bool>,
        last_update: &Mutex<Instant>,
        active: &AtomicBool,
        output_volume: &AtomicU32,
        scene_levels: &Mutex<[f32; 32]>,
    ) {
        let mut enumerator: *mut c_void = std::ptr::null_mut();
        let hr = CoCreateInstance(
            &CLSID_MM_DEVICE_ENUMERATOR as *const _ as _,
            std::ptr::null_mut(),
            CLSCTX_ALL,
            &IID_IMM_DEVICE_ENUMERATOR as *const _ as _,
            &mut enumerator,
        );
        if hr != 0 || enumerator.is_null() {
            return;
        }

        let enum_vtbl = *(enumerator as *mut *mut IMMDeviceEnumeratorVtbl);
        let mut device: *mut c_void = std::ptr::null_mut();
        let hr = ((*enum_vtbl).get_default_audio_endpoint)(enumerator, 0, 0, &mut device);
        if hr != 0 || device.is_null() {
            ((*enum_vtbl).release)(enumerator);
            return;
        }

        let dev_vtbl = *(device as *mut *mut IMMDeviceVtbl);
        let mut audio_client: *mut c_void = std::ptr::null_mut();
        let hr = ((*dev_vtbl).activate)(
            device,
            &IID_IAUDIO_CLIENT,
            CLSCTX_ALL,
            std::ptr::null(),
            &mut audio_client,
        );
        ((*dev_vtbl).release)(device);
        ((*enum_vtbl).release)(enumerator);

        if hr != 0 || audio_client.is_null() {
            return;
        }

        let client_vtbl = *(audio_client as *mut *mut IAudioClientVtbl);
        let mut format_ptr: *mut c_void = std::ptr::null_mut();
        let hr = ((*client_vtbl).get_mix_format)(audio_client, &mut format_ptr);
        if hr != 0 || format_ptr.is_null() {
            ((*client_vtbl).release)(audio_client);
            return;
        }

        let fmt = *(format_ptr as *const WaveFormatEx);
        let channels = fmt.n_channels.max(1) as usize;
        let sample_rate = fmt.n_samples_per_sec.max(8000) as f32;
        let is_32bit = fmt.w_bits_per_sample == 32;
        let is_16bit = fmt.w_bits_per_sample == 16;

        let hr = ((*client_vtbl).initialize)(
            audio_client,
            0, // AUDCLNT_SHAREMODE_SHARED
            AUDCLNT_STREAMFLAGS_LOOPBACK,
            1_000_000i64, // 100 ms buffer
            0i64,
            format_ptr,
            std::ptr::null(),
        );
        CoTaskMemFree(format_ptr);
        if hr != 0 {
            ((*client_vtbl).release)(audio_client);
            return;
        }

        let mut capture_ptr: *mut c_void = std::ptr::null_mut();
        let hr = ((*client_vtbl).get_service)(
            audio_client,
            &IID_IAUDIO_CAPTURE_CLIENT,
            &mut capture_ptr,
        );
        if hr != 0 || capture_ptr.is_null() {
            ((*client_vtbl).release)(audio_client);
            return;
        }

        let capture_vtbl = *(capture_ptr as *mut *mut IAudioCaptureClientVtbl);

        let mut smoothed = [0.0f32; 64];
        let mut mono_ring: Vec<f32> = Vec::with_capacity(8192);

        const N: usize = 4096;
        let pi2 = 2.0 * std::f32::consts::PI;

        // Precalculate Hann window coefficients
        let mut hann = [0.0f32; N];
        for n in 0..N {
            hann[n] = 0.5 * (1.0 - (pi2 * n as f32 / N as f32).cos());
        }

        // Precompute the windowed DFT coefficients: no per-frame trigonometry.
        let coefficients: Vec<_> = (0..64)
            .map(|band| {
                let frequency = if band < 32 {
                    40. * 300f32.powf((band as f32 + 0.5) / 32.)
                } else {
                    crate::connect::scene_frequency(((band - 32) as f32 + 0.5) / 32.)
                };
                let omega = pi2 * frequency / sample_rate;
                (
                    (0..N)
                        .map(|n| (omega * n as f32).cos() * hann[n])
                        .collect::<Vec<_>>(),
                    (0..N)
                        .map(|n| (omega * n as f32).sin() * hann[n])
                        .collect::<Vec<_>>(),
                )
            })
            .collect();
        if ((*client_vtbl).start)(audio_client) < 0 {
            ((*capture_vtbl).release)(capture_ptr);
            ((*client_vtbl).release)(audio_client);
            return;
        }
        'capture: loop {
            if !active.load(Ordering::Relaxed) {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(15));

            let mut packet_length = 0u32;
            let mut got_audio = false;

            loop {
                let hr = ((*capture_vtbl).get_next_packet_size)(capture_ptr, &mut packet_length);
                if hr < 0 {
                    break 'capture;
                }
                if packet_length == 0 {
                    break;
                }
                let mut data_ptr: *mut u8 = std::ptr::null_mut();
                let mut num_frames = 0u32;
                let mut flags = 0u32;
                let mut dev_pos = 0u64;
                let mut qpc_pos = 0u64;

                let hr = ((*capture_vtbl).get_buffer)(
                    capture_ptr,
                    &mut data_ptr,
                    &mut num_frames,
                    &mut flags,
                    &mut dev_pos,
                    &mut qpc_pos,
                );

                if hr == 0 && num_frames > 0 && !data_ptr.is_null() {
                    if (flags & AUDCLNT_BUFFERFLAGS_SILENT) == 0 {
                        let nf = num_frames as usize;
                        if is_32bit {
                            let ptr = data_ptr as *const f32;
                            for i in 0..nf {
                                let mut sum = 0.0f32;
                                for c in 0..channels {
                                    sum += *ptr.add(i * channels + c);
                                }
                                mono_ring.push(sum / channels as f32);
                            }
                        } else if is_16bit {
                            let ptr = data_ptr as *const i16;
                            for i in 0..nf {
                                let mut sum = 0.0f32;
                                for c in 0..channels {
                                    sum += *ptr.add(i * channels + c) as f32 / 32768.0;
                                }
                                mono_ring.push(sum / channels as f32);
                            }
                        }

                        if mono_ring.len() > 8192 {
                            let excess = mono_ring.len() - 4096;
                            mono_ring.drain(0..excess);
                        }

                        got_audio = true;
                    }
                    if ((*capture_vtbl).release_buffer)(capture_ptr, num_frames) < 0 {
                        break 'capture;
                    }
                } else {
                    break 'capture;
                }
            }

            // Release queued packets before analyzing only the newest audio window.
            if got_audio && mono_ring.len() >= N {
                let window_start = mono_ring.len() - N;
                let window = &mono_ring[window_start..];
                let gain = analysis_gain(f32::from_bits(output_volume.load(Ordering::Relaxed)));
                for b in 0..64 {
                    let (cosine, sine) = &coefficients[b];
                    let mut re = 0.;
                    let mut im = 0.;
                    for n in 0..N {
                        let sample = window[n] * gain;
                        re += sample * cosine[n];
                        im += sample * sine[n];
                    }
                    let magnitude = (re * re + im * im).sqrt() / (N as f32 * 0.25);
                    let db = if magnitude > 0. {
                        20. * magnitude.log10()
                    } else {
                        -120.
                    };
                    smoothed[b] = crate::connect::dynamic_level(db);
                }
            }
            if !got_audio {
                for b in smoothed.iter_mut() {
                    *b *= 0.88;
                }
            }

            let total_energy: f32 = smoothed.iter().sum();
            let active = got_audio && total_energy > 0.03;

            if let Ok(mut l) = levels.lock() {
                l.copy_from_slice(&smoothed[..32]);
            }
            if let Ok(mut value) = scene_levels.lock() {
                value.copy_from_slice(&smoothed[32..]);
            }
            if let Ok(mut s) = has_live_signal.lock() {
                *s = active;
            }
            if active {
                if let Ok(mut u) = last_update.lock() {
                    *u = Instant::now();
                }
            }
        }
        let _ = ((*client_vtbl).stop)(audio_client);
        ((*capture_vtbl).release)(capture_ptr);
        ((*client_vtbl).release)(audio_client);
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        #[test]
        fn cached_kernels_match_fresh_smoothing_after_size_changes() {
            let mut state = TrapResponse::new();
            let bands = std::array::from_fn(|i| i as f32 / 32.);
            for count in [0, 1, 3, 17, 199, 17] {
                state.frame(bands, Instant::now(), count, 100.);
                for layer in 0..5 {
                    let radius = (10 + layer * 2).min(count.saturating_sub(1));
                    let sigma = (radius as f32 / 2.4).max(1.);
                    let weights: Vec<_> = (-(radius as isize)..=radius as isize)
                        .map(|offset| (-(offset as f32).powi(2) / (2. * sigma * sigma)).exp())
                        .collect();
                    let sum: f32 = weights.iter().sum();
                    let raw = trap_outline(&bands, count, 100.);
                    let expected: Vec<_> = (0..count)
                        .map(|index| {
                            weights
                                .iter()
                                .enumerate()
                                .map(|(offset, weight)| {
                                    raw[(index + count + offset - radius) % count] * weight
                                })
                                .sum::<f32>()
                                / sum
                        })
                        .collect();
                    let actual = scene_contour(
                        &bands,
                        count,
                        100.,
                        &state.kernels[layer],
                        &mut state.outline_scratch,
                    );
                    assert_eq!(actual, expected);
                }
            }
        }
        #[test]
        fn scene_layers_trail_a_hit_then_return_to_rest() {
            let mut state = TrapResponse::new();
            let start = state.last;
            let mut frame = (Vec::new(), 0.);
            for index in 1..=5 {
                frame = state.frame(
                    [0.7; 32],
                    start + std::time::Duration::from_millis(index * 16),
                    199,
                    100.,
                );
            }
            assert!(frame.0[0].iter().any(|v| *v > 20.));
            assert!(frame.0[4].iter().all(|v| *v == 0.));
            for index in 6..=90 {
                frame = state.frame(
                    [0.; 32],
                    start + std::time::Duration::from_millis(index * 16),
                    199,
                    100.,
                );
            }
            assert!(frame.0.iter().flatten().all(|v| *v < 0.01));
            assert!(frame.1 < 0.001);
            assert!(state.history.len() <= 64);
        }
        #[test]
        fn hits_rise_quickly_and_drop_between_beats_without_lifting_quiet_music() {
            let mut levels = [0.; 32];
            let mut hit = [0.; 32];
            hit[3] = 0.9;
            follow_levels(&mut levels, &hit, 0.036);
            assert!(levels[3] > 0.75);
            assert!(levels.iter().enumerate().all(|(i, v)| i == 3 || *v == 0.));
            let quiet = [0.04; 32];
            follow_levels(&mut levels, &quiet, 0.2);
            assert!(levels[3] < 0.12);
            follow_levels(&mut levels, &quiet, 1.);
            assert!(levels.iter().all(|value| (*value - 0.04).abs() < 0.0001));
        }
        #[test]
        fn trap_outline_is_closed_symmetric_and_preserves_frequency_position() {
            let mut low = [0.; 32];
            low[0] = 1.;
            let mut high = [0.; 32];
            high[31] = 1.;
            let bass = trap_outline(&low, 128, 100.);
            let treble = trap_outline(&high, 128, 100.);
            assert!(bass[64] > bass[0]);
            assert!(treble[0] > treble[64]);
            for k in 1..128 {
                assert!((bass[k] - bass[128 - k]).abs() < 0.001);
            }
            assert!((treble[0] - treble[127]).abs() < 20.);
            assert!(trap_outline(&[0.; 32], 128, 100.).iter().all(|v| *v == 0.));
        }
        #[test]
        fn analysis_level_is_invariant_to_output_gain() {
            for music_amplitude in [0.003, 0.03, 0.3] {
                for volume in [0.01f32, 0.1, 0.5, 1.] {
                    let recorded = music_amplitude * volume.powi(3);
                    assert!((recorded * analysis_gain(volume) - music_amplitude).abs() < 0.00001);
                }
            }
            assert_eq!(analysis_gain(0.), 0.);
            // A quiet phrase must remain ten times quieter than a loud hit.
            assert!((0.03 * analysis_gain(0.5)) / (0.3 * analysis_gain(0.5)) < 0.11);
        }
        #[test]
        fn test_visualizer_creation_and_wave_generation() {
            let v = Visualizer::new();
            let fg = v.compute_wave(1200.0, 800.0, 784.0, true, 45000, &[0; 11]);
            assert_eq!(fg.len(), 66);
            for p in &fg {
                assert!(p.1 <= 784.0);
            }
        }
    }
}
