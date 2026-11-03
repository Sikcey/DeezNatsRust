//! Apply confirmed edits to cached pages without discarding unrelated content.
use super::{Entry, Page};

#[derive(Clone)]
pub(crate) enum Change {
    Library(Entry, bool),
    PlaylistDetails(Entry),
    DeletePlaylist(String),
    PlaylistTrack {
        playlist: String,
        track: Entry,
        add: bool,
    },
}

pub(crate) fn route(key: &str) -> Option<serde_json::Value> {
    // Session keys have an epoch prefix; window cache keys do not.
    serde_json::from_str(&key[key.find('[')?..]).ok()
}

impl Change {
    pub(crate) fn affects(&self, key: &str, page: &Page) -> bool {
        let Some(route) = route(key) else {
            return false;
        };
        let view = route[1].as_str().unwrap_or("");
        let contains = |kind: &str, id: &str| {
            page.entity
                .iter()
                .chain(page.rows.iter())
                .chain(
                    page.sections
                        .iter()
                        .flat_map(|section| section.entries.iter()),
                )
                .any(|entry| entry.kind == kind && entry.id == id)
        };
        match self {
            Self::Library(item, _) => {
                contains(&item.kind, &item.id)
                    || matches!(
                        (view, item.kind.as_str()),
                        ("Favorites", "track")
                            | ("Albums", "album")
                            | ("Playlists" | "PlaylistPicker", "playlist")
                            | ("Podcasts" | "Playlists", "podcast")
                    )
            }
            Self::PlaylistDetails(item) => {
                contains("playlist", &item.id) || matches!(view, "Playlists" | "PlaylistPicker")
            }
            Self::DeletePlaylist(id) => contains("playlist", id),
            Self::PlaylistTrack { playlist, .. } => route[2] == "playlist" && route[3] == *playlist,
        }
    }
    pub(crate) fn same_target(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Library(a, _), Self::Library(b, _)) => a.library_key() == b.library_key(),
            (Self::PlaylistDetails(a), Self::PlaylistDetails(b)) => a.id == b.id,
            (Self::DeletePlaylist(a), Self::DeletePlaylist(b)) => a == b,
            (
                Self::PlaylistTrack {
                    playlist: a,
                    track: x,
                    ..
                },
                Self::PlaylistTrack {
                    playlist: b,
                    track: y,
                    ..
                },
            ) => a == b && x.id == y.id,
            _ => false,
        }
    }

    pub(crate) fn apply(&self, key: &str, page: &mut Page) {
        let Some(route) = route(key) else {
            return;
        };
        let view = route[1].as_str().unwrap_or("");
        let same = |a: &Entry, b: &Entry| a.library_key() == b.library_key();
        match self {
            Self::Library(item, add) => {
                let mut saved_item = item.clone();
                saved_item.raw["isFavorite"] = (*add).into();
                let item = &saved_item;
                let update = |entry: &mut Entry| {
                    if same(entry, item) {
                        entry.raw["isFavorite"] = (*add).into();
                    }
                };
                visit(page, update);
                let library = matches!(
                    (view, item.kind.as_str()),
                    ("Favorites", "track")
                        | ("Albums", "album")
                        | ("Playlists" | "PlaylistPicker", "playlist")
                        | ("Podcasts", "podcast")
                );
                if library {
                    if view == "Favorites" {
                        membership(&mut page.rows, item, *add);
                    } else {
                        let index = page.sections.iter().position(|section| {
                            section.entries.iter().any(|entry| entry.kind == item.kind)
                                || section.title == page.title
                                || (view == "Podcasts" && section.title == "Saved podcasts")
                        });
                        if let Some(index) = index {
                            membership(&mut page.sections[index].entries, item, *add);
                        } else if *add {
                            page.sections.insert(
                                0,
                                super::Section {
                                    title: page.title.clone(),
                                    entries: vec![item.clone()],
                                    ..Default::default()
                                },
                            );
                        }
                    }
                }
                // Saved podcasts also appear alongside playlists.
                if view == "Playlists" && item.kind == "podcast" {
                    if let Some(section) = page
                        .sections
                        .iter_mut()
                        .find(|s| s.title == "Saved podcasts")
                    {
                        membership(&mut section.entries, item, *add);
                    } else if *add {
                        page.sections.push(super::Section {
                            title: "Saved podcasts".into(),
                            entries: vec![item.clone()],
                            ..Default::default()
                        });
                    }
                }
            }
            Self::PlaylistDetails(item) => {
                if page.entity.as_ref().is_some_and(|entry| same(entry, item)) {
                    page.title = item.title.clone();
                }
                visit(page, |entry| {
                    if same(entry, item) {
                        entry.title = item.title.clone();
                        if !item.artwork.is_empty() {
                            entry.artwork = item.artwork.clone();
                        }
                        if let Some(raw) = item.raw.as_object() {
                            for (key, value) in raw {
                                entry.raw[key] = value.clone();
                            }
                        }
                    }
                });
                if matches!(view, "Playlists" | "PlaylistPicker") {
                    if page.sections.is_empty() && !page.rows.is_empty() {
                        membership(&mut page.rows, item, true);
                    } else if let Some(section) = page.sections.iter_mut().find(|section| {
                        section.title == page.title
                            || section.entries.iter().any(|entry| entry.kind == "playlist")
                    }) {
                        membership(&mut section.entries, item, true);
                    } else {
                        page.sections.insert(
                            0,
                            super::Section {
                                title: page.title.clone(),
                                entries: vec![item.clone()],
                                ..Default::default()
                            },
                        );
                    }
                }
            }
            Self::DeletePlaylist(id) => {
                page.rows
                    .retain(|entry| entry.kind != "playlist" || entry.id != *id);
                for section in &mut page.sections {
                    section
                        .entries
                        .retain(|entry| entry.kind != "playlist" || entry.id != *id);
                }
            }
            Self::PlaylistTrack {
                playlist,
                track,
                add,
            } => {
                if route[2] == "playlist" && route[3] == *playlist {
                    membership(&mut page.rows, track, *add);
                    if page.next.is_empty() {
                        if let Some(entity) = page.entity.as_mut() {
                            entity.raw["trackCount"] = page.rows.len().into();
                            entity.raw["nb_tracks"] = page.rows.len().into();
                        }
                    }
                }
            }
        }
    }
}

fn membership(entries: &mut Vec<Entry>, item: &Entry, add: bool) {
    if add {
        if !entries
            .iter()
            .any(|entry| entry.library_key() == item.library_key())
        {
            entries.insert(0, item.clone());
        }
    } else {
        entries.retain(|entry| entry.library_key() != item.library_key());
    }
}

fn visit(page: &mut Page, mut update: impl FnMut(&mut Entry)) {
    if let Some(entity) = &mut page.entity {
        update(entity);
    }
    for entry in &mut page.rows {
        update(entry);
    }
    for section in &mut page.sections {
        for entry in &mut section.entries {
            update(entry);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn item(kind: &str, id: &str) -> Entry {
        Entry {
            id: id.into(),
            kind: kind.into(),
            title: id.into(),
            raw: json!({"unrelated":"kept"}),
            ..Default::default()
        }
    }
    fn key(view: &str, kind: &str, id: &str) -> String {
        json!(["Deezer:me", view, kind, id, "", ""]).to_string()
    }

    #[test]
    fn favorites_edit_keeps_pagination_and_other_rows_and_is_idempotent() {
        let removed = item("track", "one");
        let kept = item("track", "two");
        let mut page = Page {
            rows: vec![removed.clone(), kept.clone()],
            next: "next-page".into(),
            raw: vec![("cache:pagination-depth".into(), json!(3))],
            ..Default::default()
        };
        let change = Change::Library(removed.clone(), false);
        change.apply(&key("Favorites", "", ""), &mut page);
        assert_eq!(page.rows, vec![kept]);
        assert_eq!(page.next, "next-page");
        assert_eq!(page.raw[0].1, 3);
        Change::Library(removed.clone(), true).apply(&key("Favorites", "", ""), &mut page);
        Change::Library(removed, true).apply(&key("Favorites", "", ""), &mut page);
        assert_eq!(page.rows.len(), 2);
    }

    #[test]
    fn library_edits_preserve_unrelated_shelves_and_recommendations() {
        let album = item("album", "one");
        let podcasts = super::super::Section {
            title: "Saved podcasts".into(),
            entries: vec![item("podcast", "two")],
            ..Default::default()
        };
        let mut page = Page {
            title: "Albums".into(),
            sections: vec![
                super::super::Section {
                    title: "Albums".into(),
                    entries: vec![album.clone()],
                    ..Default::default()
                },
                podcasts.clone(),
            ],
            ..Default::default()
        };
        Change::Library(album.clone(), false).apply(&key("Albums", "", ""), &mut page);
        assert!(page.sections[0].entries.is_empty());
        assert_eq!(page.sections[1], podcasts);
        let mut home = Page {
            rows: vec![album.clone()],
            ..Default::default()
        };
        Change::Library(album, false).apply(&key("Home", "", ""), &mut home);
        assert_eq!(home.rows.len(), 1);
        assert_eq!(home.rows[0].raw["isFavorite"], false);
    }

    #[test]
    fn empty_podcast_library_uses_saved_shelf() {
        let mut page = super::super::podcast_library_page(vec![], vec![], vec![], vec![]);
        Change::Library(item("podcast", "new"), true).apply(&key("Podcasts", "", ""), &mut page);
        assert_eq!(page.sections.len(), 3);
        assert_eq!(page.sections[0].entries[0].id, "new");
        assert!(page.sections[1].entries.is_empty());
    }

    #[test]
    fn playlist_track_edits_only_change_the_target_playlist() {
        let track = item("track", "one");
        let mut page = Page {
            entity: Some(item("playlist", "target")),
            rows: vec![track.clone(), item("track", "two")],
            ..Default::default()
        };
        let change = Change::PlaylistTrack {
            playlist: "target".into(),
            track,
            add: false,
        };
        let original = page.clone();
        change.apply(&key("Album", "playlist", "other"), &mut page);
        assert_eq!(page, original);
        change.apply(&key("Album", "playlist", "target"), &mut page);
        assert_eq!(page.rows[0].id, "two");
        assert_eq!(page.entity.unwrap().raw["trackCount"], 1);
    }

    #[test]
    fn playlist_details_keep_existing_metadata_and_delete_keeps_other_cards() {
        let mut page = Page {
            rows: vec![item("playlist", "one"), item("playlist", "two")],
            ..Default::default()
        };
        let mut edited = item("playlist", "one");
        edited.title = "Renamed".into();
        edited.raw = json!({"description":"Changed"});
        Change::PlaylistDetails(edited).apply(&key("Playlists", "", ""), &mut page);
        assert_eq!(page.rows[0].title, "Renamed");
        assert_eq!(page.rows[0].raw["unrelated"], "kept");
        Change::DeletePlaylist("one".into()).apply(&key("Playlists", "", ""), &mut page);
        assert_eq!(page.rows, vec![item("playlist", "two")]);
    }
}
