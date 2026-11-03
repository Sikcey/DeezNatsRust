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
            let queue_scene = super::super::build(&queue, (width, 900), true, false, "", false, 50);
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
        ui.spotify_devices.extend(
            (0..20).map(
                |i| serde_json::json!({"id":format!("device{i}"),"name":format!("Speaker {i}")}),
            ),
        );
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
                    button.rect.y >= panel.y && button.rect.y + button.rect.h <= panel.y + panel.h
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
        if label.rect.y >= viewport.y && label.rect.y + label.rect.h <= viewport.y + viewport.h {
            s.labels.push(label);
        }
    }
    for mut button in body.buttons {
        button.rect.y += offset;
        if button.rect.y >= viewport.y && button.rect.y + button.rect.h <= viewport.y + viewport.h {
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
