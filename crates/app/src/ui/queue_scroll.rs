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
    let pixels =
        (ui.queue_scroll.min(scene.queue_scroll_max) * 24 + ui.queue_scroll_offset.min(23)) as i32;
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
    scene.queue_row_buttons =
        buttons.start + next.queue_row_buttons.start..buttons.start + next.queue_row_buttons.end;
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
                    let visible = |r: Rect| r.y < viewport.y + viewport.h && r.y + r.h > viewport.y;
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
                    assert!(scene.buttons[scene.queue_buttons.clone()]
                        .iter()
                        .filter(|b| !matches!(b.action, Action::QueueJump(_)) || visible(b.rect))
                        .map(|b| (b.rect, format!("{:?}", b.action)))
                        .eq(
                            fresh.buttons[fresh.queue_buttons.clone()]
                                .iter()
                                .filter(|b| !matches!(b.action, Action::QueueJump(_))
                                    || visible(b.rect))
                                .map(|b| (b.rect, format!("{:?}", b.action)))
                        ));
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
