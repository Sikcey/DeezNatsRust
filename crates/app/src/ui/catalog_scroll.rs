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
    scene.content_buttons.end = scene.content_buttons.end - old.buttons.len() + next.buttons.len();
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
                        if let Some(b) = patch.scene.buttons.iter().find(|b| b.accessible == text) {
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
