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
    let mut primary = crate::catalog::entry(artist_data, "artist").or_else(|| track_artist.clone());
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
