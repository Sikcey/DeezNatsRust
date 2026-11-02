use super::{disk_cache, entries, picture, Entry, Page, Section};
use crate::engine::Engine;
use serde_json::{json, Value};
use std::collections::HashSet;

pub(super) fn public(engine: &Engine, path: &str) -> Result<Value, String> {
    disk_cache::load("deezer", 0, &format!("public:{path}"), || {
        super::public(engine, path)
    })
}
pub(super) fn web(engine: &Engine, url: &str) -> Result<Value, String> {
    disk_cache::load("deezer", 0, &format!("web:{url}"), || {
        super::web_catalog(engine, url).map(|v| (*v).clone())
    })
}
pub(super) fn api(engine: &Engine, name: &str, args: &[Value]) -> Result<Value, String> {
    let user = engine
        .invoke("DZUserID", &[])
        .ok()
        .and_then(|value| value.as_u64().or_else(|| value.as_str()?.parse().ok()));
    match user {
        Some(user) => disk_cache::load(
            "deezer",
            user,
            &format!("api:{}", json!([name, args])),
            || engine.invoke(name, args),
        ),
        None => engine.invoke(name, args),
    }
}
fn put(page: &mut Page, title: &str, state: Value) {
    let key = format!("deezer:shelf:{title}");
    if let Some((_, old)) = page.raw.iter_mut().find(|(k, _)| k == &key) {
        *old = state;
    } else {
        page.raw.push((key, state));
    }
}
pub(super) fn remember_public(page: &mut Page, title: &str, value: &Value, kind: &str) {
    put(
        page,
        title,
        json!({"source":"public","next":value["next"],"kind":kind,"editorial":kind=="channel"}),
    );
}
fn page_info(value: &Value, depth: usize) -> Option<&Value> {
    if depth > 24 {
        return None;
    }
    if value["pageInfo"].get("hasNextPage").is_some() {
        return Some(&value["pageInfo"]);
    }
    if let Some(object) = value.as_object() {
        object.values().find_map(|v| page_info(v, depth + 1))
    } else {
        value
            .as_array()?
            .iter()
            .find_map(|v| page_info(v, depth + 1))
    }
}
pub(super) fn remember_dynamic(page: &mut Page, title: &str, value: &Value, id: &str, kind: &str) {
    let info = page_info(value, 0).cloned().unwrap_or(Value::Null);
    put(
        page,
        title,
        json!({"source":"dynamic","id":id,"type":kind,
        "next":if info["hasNextPage"]==true {info["endCursor"].clone()} else {Value::Null}}),
    );
}
pub(super) fn has_more(page: &Page, title: &str) -> bool {
    page.raw
        .iter()
        .find(|(key, _)| key == &format!("deezer:shelf:{title}"))
        .is_some_and(|(_, value)| value["next"].as_str().is_some_and(|next| !next.is_empty()))
}
pub(super) fn cover_directories(sections: &mut [Section], parent: &str) {
    let fallback = sections
        .iter()
        .flat_map(|s| &s.entries)
        .find(|entry| {
            matches!(entry.kind.as_str(), "playlist" | "album") && !entry.artwork.is_empty()
        })
        .map(|entry| entry.artwork.clone())
        .unwrap_or_else(|| parent.into());
    for entry in sections
        .iter_mut()
        .flat_map(|s| &mut s.entries)
        .filter(|entry| entry.kind == "channel")
    {
        if entry.artwork.contains("/images/misc//") {
            entry.artwork.clear();
        }
        if entry.artwork.is_empty() {
            entry.artwork = picture(&entry.raw);
        }
        if entry.artwork.contains("/images/misc//") {
            entry.artwork.clear();
        }
        if entry.artwork.is_empty() {
            entry.artwork = fallback.clone();
        }
    }
}
pub(super) fn more(
    engine: &Engine,
    page: &mut Page,
    title: &str,
    target: usize,
) -> Result<(), String> {
    for _ in 0..3 {
        let loaded = page
            .sections
            .iter()
            .find(|s| s.title == title)
            .map_or(0, |s| s.entries.len());
        if loaded >= target || !has_more(page, title) {
            break;
        }
        let key = format!("deezer:shelf:{title}");
        let state = page.raw.iter().find(|(k, _)| k == &key).unwrap().1.clone();
        let next = state["next"].as_str().unwrap_or("");
        let value = if state["source"] == "public" {
            let mut url = url::Url::parse(next).map_err(|_| "Invalid Deezer catalogue cursor")?;
            let pairs: Vec<_> = url
                .query_pairs()
                .filter(|(key, _)| key != "limit")
                .map(|(k, v)| (k.into_owned(), v.into_owned()))
                .collect();
            url.query_pairs_mut()
                .clear()
                .extend_pairs(pairs)
                .append_pair("limit", &(target - loaded).clamp(1, 100).to_string());
            let value = public(engine, url.as_str())?;
            remember_public(
                page,
                title,
                &value,
                state["kind"].as_str().unwrap_or("playlist"),
            );
            value
        } else {
            let id = state["id"].as_str().unwrap_or("");
            let kind = state["type"].as_str().unwrap_or("");
            let value = api(
                engine,
                "DZDiscoverySectionJSON",
                &[json!(id), json!(kind), json!(next)],
            )?;
            remember_dynamic(page, title, &value, id, kind);
            value
        };
        if page
            .raw
            .iter()
            .find(|(k, _)| k == &key)
            .is_some_and(|(_, v)| v["next"] == state["next"])
        {
            if let Some((_, v)) = page.raw.iter_mut().find(|(k, _)| k == &key) {
                v["next"] = Value::Null;
            }
        }
        let mut more = entries(&value, state["kind"].as_str().unwrap_or("playlist"));
        if state["editorial"] == true {
            more.retain(|e| e.id != "0");
            for entry in &mut more {
                entry.id = format!("editorial:{}", entry.id);
            }
        }
        if let Some(section) = page.sections.iter_mut().find(|s| s.title == title) {
            append_unique(&mut section.entries, more);
        }
    }
    let cover = page
        .entity
        .as_ref()
        .map(|e| e.artwork.as_str())
        .unwrap_or("");
    cover_directories(&mut page.sections, cover);
    Ok(())
}
fn append_unique(existing: &mut Vec<Entry>, more: Vec<Entry>) {
    let mut ids: HashSet<_> = existing
        .iter()
        .map(|e| (e.kind.clone(), e.id.clone()))
        .collect();
    existing.extend(
        more.into_iter()
            .filter(|e| ids.insert((e.kind.clone(), e.id.clone()))),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn deezer_genres_keep_provider_images_and_fill_missing_nested_covers() {
        let mut sections = vec![Section {
            entries: vec![
                Entry {
                    kind: "channel".into(),
                    raw: json!({"picture_xl":"https://cdn-images.dzcdn.net/images/misc/pop/1000x1000.jpg"}),
                    ..Default::default()
                },
                Entry {
                    kind: "channel".into(),
                    ..Default::default()
                },
            ],
            ..Default::default()
        }];
        cover_directories(
            &mut sections,
            "https://cdn-images.dzcdn.net/images/cover/parent.jpg",
        );
        assert!(sections[0].entries[0].artwork.contains("/misc/pop/"));
        assert!(sections[0].entries[1].artwork.contains("/cover/parent"));
    }
    #[test]
    fn public_and_dynamic_shelves_retain_real_pagination_cursors() {
        let mut page = Page::default();
        remember_public(
            &mut page,
            "Genres",
            &json!({"next":"https://api.deezer.com/editorial?index=8"}),
            "channel",
        );
        remember_dynamic(
            &mut page,
            "Rock",
            &json!({"data":{"section":{"pageInfo":{"hasNextPage":true,"endCursor":"next8"}}}}),
            "id",
            "Grid",
        );
        assert!(has_more(&page, "Genres"));
        assert!(has_more(&page, "Rock"));
        remember_dynamic(
            &mut page,
            "Rock",
            &json!({"pageInfo":{"hasNextPage":false}}),
            "id",
            "Grid",
        );
        assert!(!has_more(&page, "Rock"));
    }
    #[test]
    fn discovery_more_opens_only_the_selected_catalogue_and_preserves_its_cursor() {
        let mut page = Page {
            sections: vec![
                Section {
                    title: "Genres".into(),
                    entries: vec![Entry {
                        id: "editorial:132".into(),
                        kind: "channel".into(),
                        artwork: "https://cdn-images.dzcdn.net/pop.jpg".into(),
                        ..Default::default()
                    }],
                    ..Default::default()
                },
                Section {
                    title: "Categories".into(),
                    ..Default::default()
                },
            ],
            raw: vec![("discover".into(), json!(true))],
            ..Default::default()
        };
        remember_public(
            &mut page,
            "Genres",
            &json!({"next":"https://api.deezer.com/editorial?index=8"}),
            "channel",
        );
        let related = super::super::catalogue_page(&page, "Genres").unwrap();
        assert_eq!(related.sections.len(), 1);
        assert_eq!(related.title, "Genres");
        assert!(!related.is_discover());
        assert!(related.is_catalogue());
        assert!(has_more(&related, "Genres"));
        assert_eq!(
            related.sections[0].entries[0].artwork,
            page.sections[0].entries[0].artwork
        );
    }
}
