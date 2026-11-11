#[path = "genre_photos.rs"]
pub(crate) mod genre_photos;

mod artwork_queue {
    //! Bounded visible/prefetch lanes and cancellation before expensive decoding.
    use super::ArtRequest;
    use std::{
        collections::{HashSet, VecDeque},
        sync::{Condvar, Mutex},
    };
    #[derive(Default)]
    struct State {
        visible: VecDeque<ArtRequest>,
        prefetch: VecDeque<ArtRequest>,
        wanted: Option<HashSet<String>>,
        closed: bool,
    }
    #[derive(Default)]
    pub(super) struct ArtworkQueue {
        state: Mutex<State>,
        ready: Condvar,
    }
    fn wanted(state: &State, request: &ArtRequest) -> bool {
        request.background
            || (request.max_edge > 256 && !request.url.starts_with(super::genre_photos::PREFIX))
            || state
                .wanted
                .as_ref()
                .is_none_or(|keys| keys.contains(&request.key))
    }
    impl ArtworkQueue {
        pub(super) fn push(&self, request: ArtRequest, visible: bool) -> Result<(), ArtRequest> {
            let mut state = self.state.lock().unwrap();
            if state.closed {
                return Err(request);
            }
            let lane = if visible {
                &mut state.visible
            } else {
                &mut state.prefetch
            };
            if lane.len() >= if visible { 16 } else { 8 } {
                return Err(request);
            }
            lane.push_back(request);
            self.ready.notify_one();
            Ok(())
        }
        pub(super) fn pop(&self) -> Option<ArtRequest> {
            let mut state = self.state.lock().unwrap();
            loop {
                if state.closed {
                    return None;
                }
                if let Some(request) = state
                    .visible
                    .pop_front()
                    .or_else(|| state.prefetch.pop_front())
                {
                    return Some(request);
                }
                state = self.ready.wait(state).unwrap();
            }
        }
        pub(super) fn wanted(&self, request: &ArtRequest) -> bool {
            wanted(&self.state.lock().unwrap(), request)
        }
        pub(super) fn update(
            &self,
            keys: HashSet<String>,
            visible: HashSet<String>,
        ) -> Vec<String> {
            let mut state = self.state.lock().unwrap();
            state.wanted = Some(keys);
            let mut cancelled = Vec::new();
            let old_visible = std::mem::take(&mut state.visible);
            let old_prefetch = std::mem::take(&mut state.prefetch);
            for request in old_visible.into_iter().chain(old_prefetch) {
                if !wanted(&state, &request) {
                    cancelled.push(request.key);
                    continue;
                }
                let foreground = visible.contains(&request.key)
                    || request.background
                    || (request.max_edge > 256
                        && !request.url.starts_with(super::genre_photos::PREFIX));
                if foreground && state.visible.len() < 16 {
                    state.visible.push_back(request);
                } else if state.prefetch.len() < 8 {
                    state.prefetch.push_back(request);
                } else {
                    cancelled.push(request.key);
                }
            }
            cancelled
        }
        pub(super) fn close(&self) {
            self.state.lock().unwrap().closed = true;
            self.ready.notify_all();
        }
    }
    #[cfg(test)]
    mod tests {
        use super::*;
        fn job(key: &str) -> ArtRequest {
            ArtRequest {
                #[cfg(feature = "diagnostics")]
                view_tag: 0,
                engine: None,
                url: key.into(),
                key: key.into(),
                background: false,
                max_edge: 192,
            }
        }
        #[test]
        fn visible_work_bypasses_prefetch_and_stale_jobs_are_cancelled() {
            let queue = ArtworkQueue::default();
            for i in 0..8 {
                assert!(queue.push(job(&format!("ahead-{i}")), false).is_ok());
            }
            assert!(queue.push(job("overflow"), false).is_err());
            assert!(queue.push(job("visible"), true).is_ok());
            assert_eq!(queue.pop().unwrap().key, "visible");
            let cancelled = queue.update(HashSet::from(["ahead-7".into()]), HashSet::new());
            assert_eq!(cancelled.len(), 7);
            assert!(!queue.wanted(&job("ahead-0")));
            assert_eq!(queue.pop().unwrap().key, "ahead-7");
            queue.close();
            assert!(queue.pop().is_none());
        }
        #[test]
        fn newly_visible_prefetch_is_promoted() {
            let queue = ArtworkQueue::default();
            let keys: HashSet<String> = (0..8).map(|i| format!("cover-{i}")).collect();
            for i in 0..8 {
                assert!(queue.push(job(&format!("cover-{i}")), false).is_ok());
            }
            assert!(queue
                .update(keys, HashSet::from(["cover-7".into()]))
                .is_empty());
            assert_eq!(queue.pop().unwrap().key, "cover-7");
            queue.close();
        }
    }
}
mod artwork_resize {
    // SPDX-License-Identifier: GPL-3.0-or-later
    // Changes: return DynamicImage; borrow large crops; crop small covers too.
    use image::{imageops, DynamicImage};

    pub(super) fn cover(image: DynamicImage, edge: u32) -> DynamicImage {
        let (width, height) = (image.width(), image.height());
        let side = width.min(height);
        if edge != 0 && side > edge {
            if width == height {
                return if side > edge.saturating_mul(2) {
                    image.thumbnail_exact(edge, edge)
                } else {
                    image.resize_exact(edge, edge, imageops::FilterType::Triangle)
                };
            }
            // Borrow the source crop: copying it first can allocate tens of MiB
            // just to produce a small thumbnail. Only allocate the resized output.
            let square =
                imageops::crop_imm(&image, (width - side) / 2, (height - side) / 2, side, side);
            return DynamicImage::ImageRgba8(if side > edge.saturating_mul(2) {
                imageops::thumbnail(&*square, edge, edge)
            } else {
                imageops::resize(&*square, edge, edge, imageops::FilterType::Triangle)
            });
        }
        let square = match width == height {
            true => image,
            false => image.crop_imm((width - side) / 2, (height - side) / 2, side, side),
        };
        square
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        #[test]
        fn covers_only_downscale_and_small_sources_still_crop_to_square() {
            assert_eq!(cover(DynamicImage::new_rgb8(64, 32), 192).width(), 32);
            assert_eq!(cover(DynamicImage::new_rgb8(1000, 800), 192).height(), 192);
            assert_eq!(cover(DynamicImage::new_rgb8(256, 256), 192).width(), 192);
            assert_eq!(cover(DynamicImage::new_rgb8(128, 64), 0).width(), 64);
        }

        #[test]
        fn downscaled_cover_keeps_the_center_crop() {
            let image = image::RgbImage::from_fn(80, 40, |x, _| {
                if (20..60).contains(&x) {
                    image::Rgb([20, 100, 200])
                } else {
                    image::Rgb([255, 0, 0])
                }
            });
            for edge in [8, 32] {
                let result = cover(DynamicImage::ImageRgb8(image.clone()), edge).into_rgba8();
                assert_eq!(result.dimensions(), (edge, edge));
                assert!(result.pixels().all(|p| p.0 == [20, 100, 200, 255]));
            }
        }
    }
}
mod cache {
    //! Short-lived, bounded cache for unauthenticated public catalog responses.
    use serde_json::Value;
    use std::{
        collections::VecDeque,
        sync::{Arc, Mutex},
        time::{Duration, Instant},
    };

    pub(super) struct Cache {
        state: Mutex<State>,
        budget: usize,
    }
    #[derive(Default)]
    struct State {
        entries: VecDeque<Entry>,
        bytes: usize,
    }
    struct Entry {
        key: String,
        expires: Instant,
        weight: usize,
        value: Arc<Value>,
    }
    impl Cache {
        pub(super) fn new(budget: usize) -> Self {
            Self {
                state: Mutex::new(State::default()),
                budget,
            }
        }
        pub(super) fn expire(&self) {
            let now = Instant::now();
            let mut state = self.state.lock().unwrap();
            state.entries.retain(|entry| entry.expires > now);
            state.bytes = state.entries.iter().map(|entry| entry.weight).sum();
        }
        pub(super) fn load(
            &self,
            key: &str,
            ttl: Duration,
            fetch: impl FnOnce() -> Result<Vec<u8>, String>,
            decode: impl FnOnce(&[u8]) -> Result<Value, String>,
        ) -> Result<Arc<Value>, String> {
            self.load_inner(key, ttl, fetch, decode, false)
        }
        pub(super) fn refresh(
            &self,
            key: &str,
            ttl: Duration,
            fetch: impl FnOnce() -> Result<Vec<u8>, String>,
            decode: impl FnOnce(&[u8]) -> Result<Value, String>,
        ) -> Result<Arc<Value>, String> {
            self.load_inner(key, ttl, fetch, decode, true)
        }
        fn load_inner(
            &self,
            key: &str,
            ttl: Duration,
            fetch: impl FnOnce() -> Result<Vec<u8>, String>,
            decode: impl FnOnce(&[u8]) -> Result<Value, String>,
            refresh: bool,
        ) -> Result<Arc<Value>, String> {
            let now = Instant::now();
            if !refresh {
                let mut state = self.state.lock().unwrap();
                state.entries.retain(|entry| entry.expires > now);
                state.bytes = state.entries.iter().map(|entry| entry.weight).sum();
                if let Some(index) = state.entries.iter().position(|entry| entry.key == key) {
                    let entry = state.entries.remove(index).unwrap();
                    let value = entry.value.clone();
                    state.entries.push_back(entry);
                    return Ok(value);
                }
            }
            // Neither a slow download nor JSON parsing holds the shared cache lock.
            let bytes = fetch()?;
            let value = Arc::new(decode(&bytes)?);
            // JSON's in-memory representation can be substantially larger than its
            // wire form. Charge conservatively and don't retain oversized pages.
            let weight = bytes.len().saturating_mul(8).saturating_add(1024);
            if weight <= self.budget {
                let mut state = self.state.lock().unwrap();
                if let Some(index) = state.entries.iter().position(|entry| entry.key == key) {
                    state.bytes -= state.entries.remove(index).unwrap().weight;
                }
                while !state.entries.is_empty()
                    && (state.bytes + weight > self.budget || state.entries.len() >= 256)
                {
                    state.bytes -= state.entries.pop_front().unwrap().weight;
                }
                state.bytes += weight;
                state.entries.push_back(Entry {
                    key: key.into(),
                    expires: Instant::now() + ttl,
                    weight,
                    value: value.clone(),
                });
            }
            Ok(value)
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        fn decode(bytes: &[u8]) -> Result<Value, String> {
            serde_json::from_slice(bytes).map_err(|e| e.to_string())
        }
        #[test]
        fn expire_releases_only_expired_entries_and_updates_accounting() {
            let cache = Cache::new(4096);
            for key in ["old", "current"] {
                cache
                    .load(key, Duration::from_secs(30), || Ok(b"{}".to_vec()), decode)
                    .unwrap();
            }
            cache.state.lock().unwrap().entries[0].expires = Instant::now();
            cache.expire();
            let state = cache.state.lock().unwrap();
            assert_eq!(state.entries.len(), 1);
            assert_eq!(state.entries[0].key, "current");
            assert_eq!(state.bytes, state.entries[0].weight);
        }
        #[test]
        fn revisits_skip_download_and_parsing_but_expired_entries_refresh() {
            let cache = Cache::new(4096);
            let first = cache
                .load(
                    "a",
                    Duration::from_secs(30),
                    || Ok(b"{\"items\":[1]}".to_vec()),
                    decode,
                )
                .unwrap();
            let hit = cache
                .load(
                    "a",
                    Duration::from_secs(30),
                    || panic!("repeated download"),
                    |_| panic!("repeated parse"),
                )
                .unwrap();
            assert!(Arc::ptr_eq(&first, &hit));
            cache.state.lock().unwrap().entries[0].expires = Instant::now();
            let fresh = cache
                .load(
                    "a",
                    Duration::from_secs(30),
                    || Ok(b"{\"items\":[2]}".to_vec()),
                    decode,
                )
                .unwrap();
            assert_eq!(fresh["items"][0], 2);
        }
        #[test]
        fn background_revalidation_fetches_updates_without_waiting_for_ttl() {
            let cache = Cache::new(4096);
            let ttl = Duration::from_secs(120);
            cache
                .load("page", ttl, || Ok(b"{\"version\":1}".to_vec()), decode)
                .unwrap();
            assert!(cache
                .refresh("page", ttl, || Err("network".into()), decode)
                .is_err());
            assert_eq!(
                cache
                    .load("page", ttl, || panic!("lost valid cached data"), decode)
                    .unwrap()["version"],
                1
            );
            assert_eq!(
                cache
                    .refresh("page", ttl, || Ok(b"{\"version\":2}".to_vec()), decode)
                    .unwrap()["version"],
                2
            );
            assert_eq!(
                cache
                    .load("page", ttl, || panic!("refresh not saved"), decode)
                    .unwrap()["version"],
                2
            );
        }
        #[test]
        fn failed_responses_are_not_cached_and_the_budget_is_enforced() {
            let cache = Cache::new(2100);
            assert!(cache
                .load(
                    "a",
                    Duration::from_secs(30),
                    || Err("network".into()),
                    decode
                )
                .is_err());
            assert!(cache
                .load(
                    "a",
                    Duration::from_secs(30),
                    || Ok(b"invalid".to_vec()),
                    decode
                )
                .is_err());
            for key in ["a", "b", "c"] {
                cache
                    .load(key, Duration::from_secs(30), || Ok(b"{}".to_vec()), decode)
                    .unwrap();
            }
            let state = cache.state.lock().unwrap();
            assert!(state.bytes <= 2100);
            assert_eq!(state.entries.len(), 2);
            assert_eq!(state.entries.front().unwrap().key, "b");
        }
    }
}
mod deezer_shelves {
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
    pub(super) fn remember_dynamic(
        page: &mut Page,
        title: &str,
        value: &Value,
        id: &str,
        kind: &str,
    ) {
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
                let mut url =
                    url::Url::parse(next).map_err(|_| "Invalid Deezer catalogue cursor")?;
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
}
pub(crate) mod disk_cache {
    //! Account-scoped disk cache for provider browse metadata only.
    use serde_json::{json, Value};
    use std::{
        hash::{Hash, Hasher},
        path::{Path, PathBuf},
        time::{SystemTime, UNIX_EPOCH},
    };

    const TTL: u64 = 24 * 60 * 60;
    const MAX_BYTES: u64 = 8 * 1024 * 1024;
    fn now() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
    }
    fn hash(value: &str) -> u64 {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        value.hash(&mut hasher);
        hasher.finish()
    }
    fn root(provider: &str, scope: u64) -> Option<PathBuf> {
        Some(
            crate::storage::data_dir()
                .ok()?
                .join(format!("{provider}-directory-cache"))
                .join(format!("{scope:016x}")),
        )
    }
    fn read(root: &Path, key: &str, fresh: bool) -> Option<Value> {
        let path = root.join(format!("{:016x}.json", hash(key)));
        if std::fs::metadata(&path).ok()?.len() > MAX_BYTES {
            return None;
        }
        let doc: Value = serde_json::from_slice(&std::fs::read(path).ok()?).ok()?;
        let age = now().checked_sub(doc["savedAt"].as_u64()?)?;
        if doc["version"] != 1 || doc["key"] != key || age > if fresh { TTL } else { 7 * TTL } {
            return None;
        }
        doc.get("value").cloned()
    }
    fn write(root: &Path, key: &str, value: &Value) {
        let Ok(bytes) =
            serde_json::to_vec(&json!({"version":1,"key":key,"savedAt":now(),"value":value}))
        else {
            return;
        };
        if bytes.len() as u64 > MAX_BYTES || std::fs::create_dir_all(root).is_err() {
            return;
        }
        let path = root.join(format!("{:016x}.json", hash(key)));
        let temp = root.join(format!(
            "{:016x}-{}-{:?}.tmp",
            hash(key),
            std::process::id(),
            std::thread::current().id()
        ));
        if std::fs::write(&temp, bytes).is_ok() {
            let _ = std::fs::rename(&temp, &path);
        }
        let _ = std::fs::remove_file(&temp);
        let mut files: Vec<_> = std::fs::read_dir(root)
            .into_iter()
            .flatten()
            .flatten()
            .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "json"))
            .filter_map(|entry| Some((entry.path(), entry.metadata().ok()?)))
            .collect();
        files.sort_by_key(|(_, metadata)| metadata.modified().ok());
        let mut total: u64 = files.iter().map(|(_, metadata)| metadata.len()).sum();
        for (path, metadata) in files {
            if total <= 64 * 1024 * 1024 {
                break;
            }
            if std::fs::remove_file(path).is_ok() {
                total = total.saturating_sub(metadata.len());
            }
        }
    }
    pub(crate) fn peek(provider: &str, scope: u64, key: &str) -> Option<Value> {
        read(&root(provider, scope)?, key, true)
    }
    pub(crate) fn load(
        provider: &str,
        scope: u64,
        key: &str,
        fetch: impl FnOnce() -> Result<Value, String>,
    ) -> Result<Value, String> {
        let root = root(provider, scope);
        if let Some(value) = root.as_deref().and_then(|root| read(root, key, true)) {
            return Ok(value);
        }
        refresh(provider, scope, key, fetch)
    }

    pub(crate) fn refresh(
        provider: &str,
        scope: u64,
        key: &str,
        fetch: impl FnOnce() -> Result<Value, String>,
    ) -> Result<Value, String> {
        let root = root(provider, scope);
        match fetch() {
            Ok(value) => {
                if let Some(root) = root {
                    write(&root, key, &value);
                }
                Ok(value)
            }
            Err(error) => root
                .as_deref()
                .and_then(|root| read(root, key, false))
                .ok_or(error),
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        #[test]
        fn provider_and_account_cache_directories_are_separate() {
            assert_ne!(root("deezer", 42), root("spotify", 42));
            assert_ne!(root("deezer", 42), root("deezer", 43));
        }
        #[test]
        fn browse_metadata_survives_reopening_and_rejects_old_or_corrupt_records() {
            let root = std::env::temp_dir().join(format!(
                "spotify-directory-test-{}-{}",
                std::process::id(),
                now()
            ));
            write(&root, "page", &json!({"items":["House"]}));
            assert_eq!(read(&root, "page", true).unwrap()["items"][0], "House");
            assert!(read(&root, "other", true).is_none());
            let path = root.join(format!("{:016x}.json", hash("page")));
            let mut doc: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
            doc["savedAt"] = json!(now() - TTL - 1);
            std::fs::write(&path, serde_json::to_vec(&doc).unwrap()).unwrap();
            assert!(read(&root, "page", true).is_none());
            assert!(read(&root, "page", false).is_some());
            std::fs::write(&path, b"broken").unwrap();
            assert!(read(&root, "page", false).is_none());
            let _ = std::fs::remove_file(path);
            let _ = std::fs::remove_dir(root);
        }
    }
}
pub(crate) mod musicbrainz {
    //! Optional public metadata for sparse provider track responses.
    use super::Entry;
    use serde_json::Value;
    use std::sync::{Mutex, OnceLock};
    use std::time::{Duration, Instant};

    fn normalized(s: &str) -> String {
        s.chars()
            .flat_map(char::to_lowercase)
            .filter(|c| c.is_alphanumeric())
            .collect()
    }

    pub(crate) fn enrich(track: &Entry, meta: Value) -> Value {
        let known = |paths: &[&str]| {
            paths.iter().any(|path| {
                meta.pointer(path).is_some_and(|value| {
                    !value.is_null()
                        && value.as_str() != Some("")
                        && value.as_array().is_none_or(|values| !values.is_empty())
                })
            })
        };
        if known(&["/genres", "/genre", "/album/genres"])
            && known(&["/label", "/album/label"])
            && known(&[
                "/release_date",
                "/releaseDate",
                "/album/release_date",
                "/album/releaseDate",
            ])
            && known(&["/track_number", "/track_position"])
        {
            return meta;
        }
        use std::hash::{Hash, Hasher};
        let mut hash = std::collections::hash_map::DefaultHasher::new();
        (
            &track.id,
            &track.title,
            &track.subtitle,
            &track.album,
            track.duration,
            meta.to_string(),
        )
            .hash(&mut hash);
        let key = hash.finish();
        static CACHE: OnceLock<Mutex<std::collections::VecDeque<(u64, Instant, Value)>>> =
            OnceLock::new();
        let cache = CACHE.get_or_init(|| Mutex::new(std::collections::VecDeque::new()));
        if let Ok(cache) = cache.lock() {
            if let Some((_, _, value)) = cache
                .iter()
                .find(|(id, time, _)| *id == key && time.elapsed() < Duration::from_secs(3600))
            {
                return value.clone();
            }
        }
        let result = enrich_uncached(track, meta);
        if let Ok(mut cache) = cache.lock() {
            cache.retain(|(id, _, _)| *id != key);
            cache.push_front((key, Instant::now(), result.clone()));
            cache.truncate(32);
        }
        result
    }

    fn fetch(url: &str) -> Option<Value> {
        static LAST: OnceLock<Mutex<Option<Instant>>> = OnceLock::new();
        let mut last = LAST.get_or_init(|| Mutex::new(None)).lock().ok()?;
        if let Some(previous) = *last {
            std::thread::sleep(Duration::from_millis(1100).saturating_sub(previous.elapsed()));
        }
        *last = Some(Instant::now());
        crate::lyrics::http::json(
            url,
            &[(
                "User-Agent",
                "DeezNatsRust/0.1 (https://github.com/Sikcey/DeezNatsRust)",
            )],
        )
        .ok()
    }

    fn lookup(kind: &str, id: &Value, include: &str) -> Option<Value> {
        let id = id.as_str().filter(|id| {
            id.len() == 36 && id.bytes().all(|b| b.is_ascii_hexdigit() || b == b'-')
        })?;
        let mut url = url::Url::parse(&format!("https://musicbrainz.org/ws/2/{kind}/{id}")).ok()?;
        url.query_pairs_mut()
            .append_pair("inc", include)
            .append_pair("fmt", "json");
        fetch(url.as_str())
    }

    fn enrich_uncached(track: &Entry, mut meta: Value) -> Value {
        let artists = track.artist_entries();
        let artist = artists
            .first()
            .map(|artist| artist.title.as_str())
            .filter(|name| !name.is_empty())
            .unwrap_or(track.subtitle.trim());
        if track.title.trim().is_empty()
            || artist.is_empty()
            || (track.duration == 0 && track.album.is_empty())
        {
            return meta;
        }
        let clean = |s: &str| {
            s.chars()
                .map(|c| {
                    if c.is_alphanumeric() || c.is_whitespace() {
                        c
                    } else {
                        ' '
                    }
                })
                .collect::<String>()
        };
        let query = format!(
            "recording:\"{}\" AND artist:\"{}\"",
            clean(&track.title),
            clean(artist)
        );
        let mut url = url::Url::parse("https://musicbrainz.org/ws/2/recording/").unwrap();
        url.query_pairs_mut()
            .append_pair("query", &query)
            .append_pair("fmt", "json")
            .append_pair("limit", "10");
        let Some(result) = fetch(url.as_str()) else {
            return meta;
        };
        let Some(rows) = result["recordings"].as_array() else {
            return meta;
        };
        let title = normalized(&track.title);
        let artist = normalized(artist);
        let Some(row) = rows
            .iter()
            .filter(|row| {
                normalized(row["title"].as_str().unwrap_or("")) == title
                    && row["artist-credit"].as_array().is_some_and(|credits| {
                        credits.iter().any(|c| {
                            normalized(
                                c["artist"]["name"]
                                    .as_str()
                                    .or_else(|| c["name"].as_str())
                                    .unwrap_or(""),
                            ) == artist
                        })
                    })
                    && (track.duration == 0
                        || row["length"]
                            .as_u64()
                            .is_some_and(|length| length.abs_diff(track.duration as u64) <= 4000))
            })
            .max_by_key(|row| {
                !track.album.is_empty()
                    && row["releases"].as_array().is_some_and(|releases| {
                        releases.iter().any(|r| {
                            normalized(r["title"].as_str().unwrap_or(""))
                                == normalized(&track.album)
                        })
                    })
            })
        else {
            return meta;
        };
        let detail = lookup(
            "recording",
            &row["id"],
            "releases+artist-credits+isrcs+genres",
        )
        .unwrap_or_else(|| row.clone());
        if !meta.is_object() {
            meta = serde_json::json!({});
        }
        let release = row["releases"].as_array().and_then(|releases| {
            releases.iter().find(|release| {
                !track.album.is_empty()
                    && normalized(release["title"].as_str().unwrap_or(""))
                        == normalized(&track.album)
            })
        });
        let release = release.map(|release| {
            lookup(
                "release",
                &release["id"],
                "labels+recordings+artist-credits",
            )
            .unwrap_or_else(|| release.clone())
        });
        apply_details(&mut meta, &detail, release.as_ref());
        meta
    }

    fn apply_details(meta: &mut Value, recording: &Value, release: Option<&Value>) {
        let empty = |value: &Value| {
            value.is_null()
                || value.as_str() == Some("")
                || value.as_array().is_some_and(Vec::is_empty)
        };
        if empty(&meta["genres"]) && empty(&meta["genre"]) && empty(&meta["album"]["genres"]) {
            let genres: Vec<_> = recording["genres"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|genre| genre["count"].as_i64().unwrap_or(1) > 0)
                .take(5)
                .filter_map(|genre| genre["name"].as_str())
                .collect();
            if !genres.is_empty() {
                meta["genres"] = serde_json::json!(genres);
            }
        }
        if empty(&meta["isrc"]) && empty(&meta["external_ids"]["isrc"]) {
            if let Some(isrc) = recording["isrcs"]
                .as_array()
                .and_then(|values| values.first())
                .and_then(Value::as_str)
            {
                meta["isrc"] = isrc.into();
            }
        }
        if let Some(release) = release {
            if ["release_date", "releaseDate", "physical_release_date"]
                .iter()
                .all(|key| empty(&meta[*key]))
                && empty(&meta["album"]["release_date"])
                && empty(&meta["album"]["releaseDate"])
            {
                if let Some(date) = release["date"].as_str().filter(|date| !date.is_empty()) {
                    meta["release_date"] = date.into();
                }
            }
            if empty(&meta["label"]) && empty(&meta["label_name"]) && empty(&meta["album"]["label"])
            {
                let labels: Vec<_> = release["label-info"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|info| info["label"]["name"].as_str())
                    .collect();
                if !labels.is_empty() {
                    meta["label"] = labels.join(" / ").into();
                }
            }
            if empty(&meta["track_number"]) && empty(&meta["track_position"]) {
                let track = release["media"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .flat_map(|medium| medium["tracks"].as_array().into_iter().flatten())
                    .find(|track| {
                        !recording["id"].is_null() && track["recording"]["id"] == recording["id"]
                    });
                if let Some(number) = track
                    .and_then(|track| track.get("number"))
                    .filter(|number| !empty(number))
                {
                    meta["track_number"] = number.clone();
                }
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use serde_json::json;
        #[test]
        fn third_party_details_fill_gaps_without_replacing_provider_values_or_statistics() {
            let mut meta = json!({"label":"Provider label","popularity":0,"explicit":true});
            apply_details(
                &mut meta,
                &json!({"id":"matched","genres":[{"name":"R&B","count":5}]}),
                Some(
                    &json!({"label-info":[{"label":{"name":"Other label"}}],"media":[{"tracks":[{"number":"1","recording":{"id":"other"}},{"number":"4","recording":{"id":"matched"}}]}]}),
                ),
            );
            assert_eq!(meta["label"], "Provider label");
            assert_eq!(meta["track_number"], "4");
            assert_eq!(meta["genres"][0], "R&B");
            assert_eq!(meta["popularity"], 0);
            assert!(meta.get("playCount").is_none());
            assert!(meta.get("language").is_none());
        }
    }
}
mod spotify {
    //! Spotify's API shapes adapted to the shared native page model.
    use super::*;
    pub(super) fn style_mix(selection: &str) -> Result<Value, String> {
        invoke("SPStyleMixJSON", &[json!(selection)])
    }

    fn invoke(name: &str, args: &[Value]) -> Result<Value, String> {
        if name == "SPStyleMixJSON" {
            return crate::spotify::catalog_invoke(name, args);
        }
        if name == "StyleMixGenres" {
            if let Ok(value) = crate::spotify::catalog_invoke(name, args) {
                return Ok(value);
            }
        }
        if name == "StyleMixGenres" {
            let region = args
                .first()
                .and_then(Value::as_u64)
                .ok_or("Choose a valid language")? as usize;
            crate::style_mix::Selection::new(region, 0)?;
            let playlists = library_playlists()?;
            let mut genres = Vec::new();
            for genre in 0..crate::style_mix::GENRES.len() {
                let selection = crate::style_mix::Selection::new(region, genre)?;
                if !matching_mix(&playlists, selection)?.is_empty() {
                    genres.push(json!({"id":genre,"title":crate::style_mix::GENRES[genre].0}));
                }
            }
            return Ok(json!({"region":region,"genres":genres,"partial":false}));
        }
        crate::spotify::catalog_invoke(name, args)
    }
    fn library_playlists() -> Result<Vec<Entry>, String> {
        let mut value = crate::spotify::catalog_invoke("DZPlaylistsJSON", &[])?;
        let mut result = rows(&value, "playlist");
        let mut seen = HashSet::new();
        for _ in 0..3 {
            let cursor = string(&value, &["next"]);
            if cursor.is_empty() || !seen.insert(cursor.clone()) {
                break;
            }
            value = crate::spotify::catalog_invoke("SPCatalogNextJSON", &[json!(cursor)])?;
            result.extend(rows(&value, "playlist"));
        }
        Ok(result)
    }
    fn words(text: &str) -> String {
        format!(
            " {} ",
            text.to_lowercase()
                .chars()
                .map(|c| match c {
                    'ü' | 'ú' | 'ù' => 'u',
                    'ö' | 'ó' | 'ô' => 'o',
                    'ç' => 'c',
                    'ñ' => 'n',
                    'ä' | 'å' | 'ã' | 'á' | 'à' | 'â' => 'a',
                    'ş' => 's',
                    'ğ' => 'g',
                    'ı' | 'ï' => 'i',
                    'é' | 'è' | 'ê' => 'e',
                    c if c.is_alphanumeric() => c,
                    _ => ' ',
                })
                .collect::<String>()
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
        )
    }
    fn matches_selection(title: &str, selection: crate::style_mix::Selection) -> bool {
        let title = words(title);
        let contains = |aliases: &[&str]| aliases.iter().any(|alias| title.contains(&words(alias)));
        contains(crate::style_mix::REGIONS[selection.region].1)
            && contains(crate::style_mix::GENRES[selection.genre].1)
    }
    fn matching_mix(
        playlists: &[Entry],
        selection: crate::style_mix::Selection,
    ) -> Result<Vec<Entry>, String> {
        let mut result = Vec::new();
        let mut seen = HashSet::new();
        let mut error = None;
        for playlist in playlists
            .iter()
            .filter(|playlist| matches_selection(&playlist.title, selection))
            .take(3)
        {
            match crate::spotify::catalog_invoke("DZPlaylistTracksJSON", &[json!(playlist.id)]) {
                Ok(value) => {
                    for mut track in tracks(&value) {
                        if track.raw["is_playable"] == false || !seen.insert(track.id.clone()) {
                            continue;
                        }
                        track.raw[crate::style_mix::MARKER] = json!(selection.key());
                        result.push(track);
                    }
                }
                Err(failure) => error = Some(failure),
            }
        }
        crate::spotify::style_mix::retain_language(&mut result, selection, |track| &track.raw);
        if result.is_empty() {
            if let Some(error) = error {
                return Err(error);
            }
        }
        Ok(result)
    }
    pub(super) fn is_item(value: &Value) -> bool {
        ["uri", "id"].iter().any(|key| {
            value[*key]
                .as_str()
                .is_some_and(|s| s.starts_with("spotify:"))
        }) || value["provider"] == "spotify"
    }
    fn identity(value: &Value, kind: &str) -> String {
        let uri = string(value, &["uri", "id"]);
        if uri.starts_with("spotify:") {
            return uri;
        }
        let id = string(value, &["id"]);
        if id.is_empty() {
            String::new()
        } else {
            format!("spotify:{kind}:{id}")
        }
    }
    fn artwork(value: &Value) -> String {
        super::largest_artwork(
            value["images"]
                .as_array()
                .into_iter()
                .flatten()
                .chain(value["album"]["images"].as_array().into_iter().flatten()),
        )
        .map(str::to_owned)
        .unwrap_or_else(|| picture(value))
    }
    pub(super) fn entry(value: &Value, fallback: &str) -> Option<Entry> {
        if value.is_null() || value["is_local"] == true {
            return None;
        }
        let uri = string(value, &["uri", "id"]);
        let inferred = uri.split(':').nth(1).unwrap_or(fallback);
        let typ = string(value, &["type", "kind"]);
        let typ = if typ.is_empty() { inferred } else { &typ };
        let kind = match typ {
            "show" => "podcast",
            "category" => "channel",
            other => other,
        };
        if !matches!(
            kind,
            "track"
                | "episode"
                | "album"
                | "artist"
                | "playlist"
                | "podcast"
                | "channel"
                | "mix"
                | "flow"
        ) {
            return None;
        }
        let id = identity(value, if typ == "podcast" { "show" } else { typ });
        let title = string(value, &["name", "title"]);
        if id.is_empty() || title.is_empty() {
            return None;
        }
        let mut raw = compact_entry_metadata(value);
        raw["provider"] = json!("spotify");
        raw["id"] = json!(id);
        raw["kind"] = json!(kind);
        // Nested credit IDs must never enter a Deezer detail page or queue.
        if let Some(artists) = raw["artists"].as_array_mut() {
            for artist in artists {
                artist["id"] = json!(identity(artist, "artist"));
                artist["provider"] = json!("spotify");
            }
        }
        if raw["album"].is_object() {
            raw["album"]["id"] = json!(identity(&value["album"], "album"));
            raw["album"]["provider"] = json!("spotify");
        }
        if let Some(count) = value.pointer("/followers/total").and_then(Value::as_u64) {
            raw["followers"] = json!(count);
        }
        if let Some(count) = value["total_episodes"].as_u64() {
            raw["episodeCount"] = json!(count);
        }
        if let Some(count) = value["total_tracks"]
            .as_u64()
            .or_else(|| value.pointer("/tracks/total").and_then(Value::as_u64))
        {
            raw["nb_tracks"] = json!(count);
        }
        if let Some(date) = value["release_date"].as_str() {
            raw["releaseDate"] = json!(date);
        }
        let artists = raw["artists"].as_array();
        let artist_id = artists
            .and_then(|artists| artists.first())
            .map(|a| string(a, &["id"]))
            .unwrap_or_default();
        let artist_line = artists
            .map(|artists| {
                artists
                    .iter()
                    .map(|a| string(a, &["name", "title"]))
                    .filter(|s| !s.is_empty())
                    .collect::<Vec<_>>()
                    .join(", ")
            })
            .unwrap_or_default();
        let subtitle = match kind {
            "playlist" => string(&value["owner"], &["display_name", "name", "id"]),
            "podcast" => string(value, &["publisher", "authorName"]),
            "episode" => string(&value["show"], &["name"]),
            "artist" => raw["followers"]
                .as_u64()
                .map(|n| format!("{} followers", format_count(n)))
                .unwrap_or_default(),
            _ => artist_line,
        };
        let subtitle = if subtitle.is_empty() {
            string(value, &["artistLine", "subtitle"])
        } else {
            subtitle
        };
        Some(Entry {
            id,
            title,
            subtitle,
            kind: kind.into(),
            artwork: artwork(value),
            duration: value["duration_ms"]
                .as_u64()
                .or_else(|| value["durationMs"].as_u64())
                .unwrap_or(0),
            album: string(&value["album"], &["name", "title"]),
            artist_id,
            album_id: if value["album"].is_object() {
                identity(&value["album"], "album")
            } else {
                string(value, &["albumId"])
            },
            raw,
        })
    }

    fn spotify_value(value: &Value) -> Value {
        let mut value = value.clone();
        // API objects omit URI in a few browse/category responses.
        if value.is_object()
            && !string(&value, &["id"]).is_empty()
            && !string(&value, &["name", "title"]).is_empty()
        {
            value["provider"] = json!("spotify");
        }
        match &mut value {
            Value::Array(items) => {
                for item in items {
                    *item = spotify_value(item);
                }
            }
            Value::Object(object) => {
                for child in object.values_mut() {
                    if child.is_object() || child.is_array() {
                        *child = spotify_value(child);
                    }
                }
            }
            _ => {}
        }
        value
    }

    fn track_info(
        track: &Entry,
        invoke: &impl Fn(&str, &[Value]) -> Result<Value, String>,
    ) -> (Value, Value) {
        let meta = invoke("DZTrackMetadataJSON", &[json!(track.id)])
            .map(|v| spotify_value(&v))
            .unwrap_or_else(|error| {
                crate::spotify_diagnostics::record(
                    "catalog.track_info",
                    crate::spotify_diagnostics::error_kind(&error),
                );
                track.raw.clone()
            });
        let artist_id = entry(&meta, "track")
            .map(|full| full.artist_id)
            .filter(|id| !id.is_empty())
            .unwrap_or_else(|| track.artist_id.clone());
        let artist = if artist_id.is_empty() {
            Value::Null
        } else {
            invoke("DZArtistProfileJSON", &[json!(artist_id)])
                .map(|v| spotify_value(v.get("artist").unwrap_or(&v)))
                .unwrap_or_else(|error| {
                    crate::spotify_diagnostics::record(
                        "catalog.artist_info",
                        crate::spotify_diagnostics::error_kind(&error),
                    );
                    Value::Null
                })
        };
        (meta, artist)
    }
    fn rows(value: &Value, kind: &str) -> Vec<Entry> {
        let mut result = entries(&spotify_value(value), kind);
        if let Some(items) = value.get("items").unwrap_or(value).as_array() {
            for wrapper in items {
                let item = wrapper
                    .get("track")
                    .or_else(|| wrapper.get("item"))
                    .or_else(|| wrapper.get("episode"));
                if let Some(item) = item {
                    let uri = identity(item, kind);
                    if let Some(row) = result.iter_mut().find(|row| row.id == uri) {
                        for key in ["played_at", "added_at"] {
                            if let Some(date) = wrapper.get(key) {
                                row.raw[key] = date.clone();
                            }
                        }
                    }
                }
            }
        }
        result
    }
    fn tracks(value: &Value) -> Vec<Entry> {
        rows(value, "track")
            .into_iter()
            .filter(Entry::playable)
            .collect()
    }
    fn sections(value: &Value) -> Vec<Section> {
        value["sections"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|section| Section {
                title: string(section, &["title", "name"]),
                entries: rows(
                    section
                        .get("items")
                        .or_else(|| section.get("entries"))
                        .unwrap_or(&Value::Null),
                    "playlist",
                ),
                error: string(section, &["error"]),
            })
            .collect()
    }
    fn home_sections(value: &Value) -> Vec<Section> {
        let mut grouped: Vec<Section> = Vec::new();
        for mut section in sections(value) {
            if section.entries.is_empty() && section.error.is_empty() {
                continue;
            }
            if !section.title.is_empty() {
                if let Some(existing) = grouped.iter_mut().find(|s| s.title == section.title) {
                    for entry in section.entries.drain(..) {
                        if entry.id.is_empty()
                            || !existing.entries.iter().any(|item| item.id == entry.id)
                        {
                            existing.entries.push(entry);
                        }
                    }
                    if existing.error.is_empty() {
                        existing.error = section.error;
                    }
                    continue;
                }
            }
            grouped.push(section);
        }
        grouped
    }

    fn presentable_home_shelf(section: &Section) -> bool {
        section.entries.len() >= 2
            && !section.title.trim().is_empty()
            && !section.title.contains("{0}")
    }
    fn discovery_sections(value: &Value) -> Vec<Section> {
        let mut result: Vec<Section> = Vec::new();
        for mut section in sections(value) {
            let mut channels = Vec::new();
            section.entries.retain(|item| {
                if item.kind == "channel" {
                    channels.push(item.clone());
                    false
                } else {
                    true
                }
            });
            for item in channels {
                let title = section.title.to_lowercase();
                let id = item.id.rsplit(':').next().unwrap_or("");
                // Spotify's browse directory mixes genres, moods and podcasts in
                // one localized shelf. Stable page IDs keep our grouping language independent.
                let inherited = item.raw["directory_group"].as_str().unwrap_or("");
                let title = if matches!(inherited, "Genres" | "Categories" | "Podcasts by category")
                {
                    inherited
                } else if title.contains("podcast")
                    || matches!(
                        id,
                        "0JQ5DArNBzkmxXHCqFLx2J"
                            | "0JQ5IMCbQBLl5gxKLgufp8"
                            | "0JQ5IMCbQBLjfX9OdDrA5X"
                            | "0JQ5IMCbQBLyUJhSxhaPc6"
                    )
                {
                    "Podcasts by category"
                } else if item.id.starts_with("spotify:genre:")
                    || matches!(
                        id,
                        "0JQ5DAqbMKFCsfq2JeFgB0"
                            | "0JQ5DAqbMKFEC4WFtoNRpw"
                            | "0JQ5DAqbMKFHOzuVTgTizF"
                            | "0JQ5DAqbMKFDXXwE9BDJAr"
                            | "0JQ5DAqbMKFQ00XGBls6ym"
                            | "0JQ5DAqbMKFCWjUTdzaG0e"
                            | "0JQ5DAqbMKFKLfwjuJMoNC"
                            | "0JQ5DAqbMKFEZPnFQSFB1T"
                            | "0JQ5DAqbMKFGvOw3O4nLAf"
                            | "0JQ5DAqbMKFDkd668ypn6O"
                            | "0JQ5DAqbMKFAJ5xb0fwo9m"
                            | "0JQ5DAqbMKFziKOShCi009"
                            | "0JQ5DAqbMKFxXaXKP7zcDp"
                            | "0JQ5DAqbMKFPrEiAOxgac3"
                            | "0JQ5DAqbMKFy78wprEpAjl"
                            | "0JQ5DAqbMKFIpEuaCnimBj"
                            | "0JQ5DAqbMKFAjfauKLOZiv"
                            | "0JQ5DAqbMKFLjmiZRss79w"
                            | "0JQ5DAqbMKFQiK2EHwyjcU"
                            | "0JQ5DAqbMKFFtlLYUHv8bT"
                            | "0JQ5DAqbMKFObNLOHydSW8"
                            | "0JQ5DAqbMKFNQ0fGp4byGU"
                            | "0JQ5DAqbMKFFsW9N8maB6z"
                            | "0JQ5DAqbMKFHCxg5H5PtqW"
                            | "0JQ5DAqbMKFQFQN0rnK48G"
                            | "0JQ5DAqbMKFJKoGyUMo2hE"
                            | "0JQ5DAqbMKFQkIFo7vWoOd"
                            | "0JQ5DAqbMKFARvdmw6EIef"
                    )
                {
                    "Genres"
                } else {
                    "Categories"
                };
                if let Some(existing) = result.iter_mut().find(|s| s.title == title) {
                    if !existing.entries.iter().any(|entry| entry.id == item.id) {
                        existing.entries.push(item);
                    }
                } else {
                    result.push(Section {
                        title: title.into(),
                        entries: vec![item],
                        ..Default::default()
                    });
                }
            }
            if !section.entries.is_empty() || !section.error.is_empty() {
                result.push(section);
            }
        }
        result
    }
    fn curate_shelves(sections: &mut Vec<Section>) {
        let dedicated_mixes: HashSet<_> = sections
            .iter()
            .filter(|s| s.title == "Made for you")
            .flat_map(|s| &s.entries)
            .filter(|item| {
                item.kind == "playlist" && item.title.to_lowercase().starts_with("daily mix ")
            })
            .map(|item| (item.kind.clone(), item.id.clone()))
            .collect();
        let mut seen = HashSet::new();
        let mut artists = std::collections::HashMap::<String, usize>::new();
        for section in sections.iter_mut() {
            // History is useful even when the same item is saved or recommended.
            let history = section.title == "Recently played";
            let mut local = HashSet::new();
            let mut themes = HashSet::new();
            section.entries.retain(|item| {
                let key = (item.kind.clone(), item.id.clone());
                if !history && section.title != "Made for you" && dedicated_mixes.contains(&key) {
                    return false;
                }
                if !item.id.is_empty()
                    && (!local.insert(key.clone()) || (!history && seen.contains(&key)))
                {
                    return false;
                }
                if !history && section.title != "Your playlists" {
                    let name = item.title.to_lowercase();
                    let artist = if item.kind == "playlist" {
                        name.strip_prefix("this is ")
                            .or_else(|| name.strip_suffix(" radio"))
                    } else {
                        None
                    };
                    if let Some(artist) = artist {
                        let artist = artist.trim().to_owned();
                        let count = artists.entry(artist.clone()).or_default();
                        if !themes.insert(artist) || *count >= 2 {
                            return false;
                        }
                        *count += 1;
                    }
                }
                if !history && !item.id.is_empty() {
                    seen.insert(key);
                }
                true
            });
        }
        sections.retain(|section| !section.entries.is_empty() || !section.error.is_empty());
    }
    fn publish_curated(
        page: &Page,
        publish: &mut dyn FnMut(&Page) -> Result<(), String>,
    ) -> Result<(), String> {
        let mut visible = page.clone();
        curate_shelves(&mut visible.sections);
        publish(&visible)
    }
    fn remember_shelf(page: &mut Page, title: &str, value: &Value, kind: &str) {
        page.raw.push((
            format!("spotify:shelf:{title}"),
            json!({"kind":kind,"sections":value["sections"]}),
        ));
    }
    fn directory_covers(
        sections: &mut [Section],
        parent_cover: &str,
        fetch: &(impl Fn(&str, &[Value]) -> Result<Value, String> + Sync),
    ) {
        let fallback = sections
            .iter()
            .flat_map(|section| &section.entries)
            .find(|entry| {
                matches!(entry.kind.as_str(), "playlist" | "album") && !entry.artwork.is_empty()
            })
            .map(|entry| entry.artwork.clone())
            .unwrap_or_else(|| parent_cover.to_owned());
        for entry in sections
            .iter_mut()
            .flat_map(|section| &mut section.entries)
            .filter(|entry| {
                entry.kind == "channel"
                    && (entry.artwork.is_empty() || entry.raw["directory_artwork_fallback"] == true)
            })
        {
            // Use already downloaded genre metadata; displaying a tile must not
            // trigger another catalogue download just to find its cover.
            let cached = fetch("SPCatalogDirectoryCachedJSON", &[json!(entry.id)]).ok();
            let own_cover = cached
                .as_ref()
                .into_iter()
                .flat_map(|value| sections_from_cache(value))
                .find(|entry| {
                    matches!(entry.kind.as_str(), "playlist" | "album") && !entry.artwork.is_empty()
                })
                .map(|entry| entry.artwork);
            if let Some(cover) = own_cover {
                entry.artwork = cover;
                entry.raw["directory_artwork_fallback"] = json!(false);
            } else if entry.artwork.is_empty() && !fallback.is_empty() {
                entry.artwork = fallback.clone();
                entry.raw["directory_artwork_fallback"] = json!(true);
            }
        }
    }
    fn sections_from_cache(value: &Value) -> impl Iterator<Item = Entry> {
        sections(value)
            .into_iter()
            .flat_map(|section| section.entries)
    }
    fn is_genre_directory_category(title: &str) -> bool {
        let name = title.to_lowercase();
        ["mood", "workout", "new releases", "discover", "classics"]
            .iter()
            .any(|word| name.contains(word))
    }

    fn load_shelf_more(
        page: &mut Page,
        title: &str,
        target: usize,
        publish: &mut dyn FnMut(&Page) -> Result<(), String>,
        fetch: &(impl Fn(&str, &[Value]) -> Result<Value, String> + Sync),
    ) -> Result<(), String> {
        let key = format!("spotify:shelf:{title}");
        let Some(state) = page
            .raw
            .iter()
            .find(|(k, _)| k == &key)
            .map(|(_, v)| v.clone())
        else {
            return Ok(());
        };
        if state["directory"] == true {
            expand_discovery_directories(page, publish, fetch, 3, Some((title, target)))?;
            let pending_key = format!("spotify:directory-sections:{title}");
            let mut pending = page
                .raw
                .iter()
                .find(|(k, _)| k == &pending_key)
                .map(|(_, v)| v.clone())
                .unwrap_or(json!([]));
            for _ in 0..3 {
                let loaded = page
                    .sections
                    .iter()
                    .find(|s| s.title == title)
                    .map_or(0, |s| s.entries.len());
                if loaded >= target {
                    break;
                }
                let Some(index) = pending.as_array().and_then(|ss| {
                    ss.iter().position(|s| {
                        s["offset"].as_u64().unwrap_or(0) < s["total"].as_u64().unwrap_or(0)
                    })
                }) else {
                    break;
                };
                let offset = pending[index]["offset"].as_u64().unwrap_or(0);
                let response = fetch(
                    "SPCatalogSectionJSON",
                    &[
                        pending[index]["uri"].clone(),
                        json!(offset),
                        json!((target - loaded).clamp(1, 100)),
                    ],
                )?;
                let new_offset = response["offset"].as_u64().unwrap_or(offset);
                pending[index]["offset"] = json!(new_offset);
                if new_offset <= offset {
                    pending[index]["total"] = json!(offset);
                }
                let mut known: HashSet<_> = page
                    .sections
                    .iter()
                    .flat_map(|s| &s.entries)
                    .map(|e| e.id.clone())
                    .collect();
                for mut child in rows(&response["items"], "channel")
                    .into_iter()
                    .filter(|e| e.kind == "channel")
                {
                    if title == "Genres" && is_genre_directory_category(&child.title) {
                        continue;
                    }
                    if !known.insert(child.id.clone()) {
                        continue;
                    }
                    child.raw["directory_group"] = json!(title);
                    if child.artwork.is_empty() {
                        child.artwork = pending[index]["cover"].as_str().unwrap_or("").into();
                        child.raw["directory_artwork_fallback"] = json!(true);
                    }
                    if let Some(section) = page.sections.iter_mut().find(|s| s.title == title) {
                        section.entries.push(child);
                    }
                }
            }
            let remaining = pending
                .as_array()
                .into_iter()
                .flatten()
                .any(|s| s["offset"].as_u64().unwrap_or(0) < s["total"].as_u64().unwrap_or(0));
            if let Some((_, v)) = page.raw.iter_mut().find(|(k, _)| k == &pending_key) {
                *v = pending;
            }
            if remaining {
                if let Some((_, v)) = page.raw.iter_mut().find(|(k, _)| k == &key) {
                    v["complete"] = json!(false);
                }
            }
            return Ok(());
        }
        let mut state = state;
        let kind = state["kind"].as_str().unwrap_or("").to_owned();
        for _ in 0..3 {
            if page
                .sections
                .iter()
                .find(|s| s.title == title)
                .map_or(0, |s| s.entries.len())
                >= target
            {
                break;
            }
            let Some(index) = state["sections"].as_array().and_then(|ss| {
                ss.iter().position(|s| {
                    s["total"].as_u64().unwrap_or(0) > s["offset"].as_u64().unwrap_or(0)
                })
            }) else {
                break;
            };
            let section = &state["sections"][index];
            let offset = section["offset"].as_u64().unwrap_or(0);
            let value = fetch(
                "SPCatalogSectionJSON",
                &[
                    section["uri"].clone(),
                    json!(offset),
                    json!(target
                        .saturating_sub(
                            page.sections
                                .iter()
                                .find(|s| s.title == title)
                                .map_or(0, |s| s.entries.len())
                        )
                        .clamp(1, 100)),
                ],
            )?;
            let new_offset = value["offset"].as_u64().unwrap_or(offset);
            state["sections"][index]["offset"] = json!(new_offset);
            state["sections"][index]["total"] = if new_offset <= offset {
                json!(offset)
            } else {
                value["total"].clone()
            };
            let more = rows(&value["items"], "playlist")
                .into_iter()
                .filter(|item| {
                    kind.is_empty()
                        || item.kind == kind
                        || (kind == "album" && item.kind == "playlist")
                });
            if let Some(section) = page.sections.iter_mut().find(|s| s.title == title) {
                let mut ids: HashSet<_> = section
                    .entries
                    .iter()
                    .map(|e| (e.kind.clone(), e.id.clone()))
                    .collect();
                section
                    .entries
                    .extend(more.filter(|e| ids.insert((e.kind.clone(), e.id.clone()))));
            }
            publish(page)?;
        }
        if let Some((_, v)) = page.raw.iter_mut().find(|(k, _)| k == &key) {
            *v = state;
        }
        Ok(())
    }
    fn expand_discovery_directories(
        page: &mut Page,
        publish: &mut dyn FnMut(&Page) -> Result<(), String>,
        fetch: &(impl Fn(&str, &[Value]) -> Result<Value, String> + Sync),
        budget: usize,
        target: Option<(&str, usize)>,
    ) -> Result<(), String> {
        let mut queue = std::collections::VecDeque::new();
        for group in ["Genres", "Categories", "Podcasts by category"] {
            if target.is_some_and(|(title, _)| title != group) {
                continue;
            }
            for item in page
                .sections
                .iter()
                .filter(|s| s.title == group)
                .flat_map(|s| &s.entries)
            {
                if item.kind == "channel" {
                    queue.push_back((item.clone(), group.to_owned(), 0usize));
                }
            }
        }
        let mut visited = HashSet::new();
        let mut known: HashSet<_> = page
            .sections
            .iter()
            .flat_map(|s| &s.entries)
            .map(|item| item.id.clone())
            .collect();
        let mut downloads = 0;
        let mut skipped = false;
        // Grow the requested directory on demand, reusing cached pages and keeping
        // new downloads within the click's budget on three worker threads.
        while !queue.is_empty() && visited.len() < 256 {
            if target.is_some_and(|(title, count)| {
                page.sections
                    .iter()
                    .find(|s| s.title == title)
                    .is_some_and(|s| s.entries.len() >= count)
            }) {
                break;
            }
            publish_curated(page, publish)?;
            let mut jobs = Vec::new();
            while jobs.len() < 3 {
                let Some((item, group, depth)) = queue.pop_front() else {
                    break;
                };
                if !visited.insert(item.id.clone()) {
                    continue;
                }
                let cached = fetch("SPCatalogDirectoryCachedJSON", &[json!(item.id)])
                    .ok()
                    .filter(|v| !v.is_null());
                if cached.is_none() {
                    if downloads >= budget {
                        skipped = true;
                        continue;
                    }
                    downloads += 1;
                }
                jobs.push((item, group, depth, cached));
            }
            if jobs.is_empty() {
                break;
            }
            parallel::load(
                jobs.len(),
                |index| {
                    let (item, _, _, cached) = &jobs[index];
                    cached
                        .clone()
                        .map(Ok)
                        .unwrap_or_else(|| fetch("SPCatalogChannelJSON", &[json!(item.id)]))
                },
                |index, result| {
                    let (parent, group, depth, _) = &jobs[index];
                    if let Ok(value) = result {
                        for source in
                            value["sections"]
                                .as_array()
                                .into_iter()
                                .flatten()
                                .filter(|source| {
                                    source["total"].as_u64().unwrap_or(0)
                                        > source["offset"].as_u64().unwrap_or(0)
                                        && rows(&source["items"], "playlist")
                                            .iter()
                                            .any(|item| item.kind == "channel")
                                })
                        {
                            let key = format!("spotify:directory-sections:{group}");
                            if !page.raw.iter().any(|(k, _)| k == &key) {
                                page.raw.push((key.clone(), json!([])));
                            }
                            let pending = page
                                .raw
                                .iter_mut()
                                .find(|(k, _)| k == &key)
                                .unwrap()
                                .1
                                .as_array_mut()
                                .unwrap();
                            if !pending.iter().any(|s| s["uri"] == source["uri"]) {
                                pending.push(json!({"uri":source["uri"],"offset":source["offset"],"total":source["total"],"cover":parent.artwork}));
                            }
                        }
                        let shelves = sections(&value);
                        let cover = shelves
                            .iter()
                            .flat_map(|s| &s.entries)
                            .find(|item| !item.artwork.is_empty())
                            .map(|item| item.artwork.clone());
                        if let Some(cover) = cover {
                            for entry in page
                                .sections
                                .iter_mut()
                                .flat_map(|s| &mut s.entries)
                                .filter(|e| e.id == parent.id)
                            {
                                if entry.artwork.is_empty()
                                    || entry.raw["directory_artwork_fallback"] == true
                                {
                                    entry.artwork = cover.clone();
                                    entry.raw["directory_artwork_fallback"] = json!(false);
                                }
                            }
                        }
                        if *depth < 2 {
                            for mut child in shelves
                                .into_iter()
                                .flat_map(|s| s.entries)
                                .filter(|item| item.kind == "channel")
                            {
                                let child_group = if group == "Genres"
                                    && is_genre_directory_category(&child.title)
                                {
                                    "Categories"
                                } else {
                                    group.as_str()
                                };
                                if target.is_some_and(|(title, _)| title != child_group)
                                    || !known.insert(child.id.clone())
                                {
                                    continue;
                                }
                                child.raw["directory_group"] = json!(child_group);
                                if child.artwork.is_empty() {
                                    child.artwork = parent.artwork.clone();
                                    child.raw["directory_artwork_fallback"] = json!(true);
                                }
                                queue.push_front((
                                    child.clone(),
                                    child_group.to_owned(),
                                    depth + 1,
                                ));
                                if let Some(section) =
                                    page.sections.iter_mut().find(|s| s.title == child_group)
                                {
                                    section.entries.push(child);
                                } else {
                                    page.sections.push(Section {
                                        title: child_group.into(),
                                        entries: vec![child],
                                        ..Default::default()
                                    });
                                }
                            }
                        }
                    }
                    publish_curated(page, publish)
                },
            )?;
        }
        if let Some((title, _)) = target {
            if let Some((_, state)) = page
                .raw
                .iter_mut()
                .find(|(k, _)| k == &format!("spotify:shelf:{title}"))
            {
                state["complete"] = json!(!skipped && queue.is_empty());
            }
        }
        Ok(())
    }
    fn metadata(page: &mut Page, value: &Value) {
        let source = page
            .entity
            .as_ref()
            .map(|e| e.kind.as_str())
            .unwrap_or("track");
        if let Some(item) = entry(&spotify_value(value), source) {
            page.entity = Some(item);
        }
        apply_entity_metadata(page);
    }

    fn apply_entity_metadata(page: &mut Page) {
        if let Some(entity) = &page.entity {
            for row in &mut page.rows {
                if entity.kind == "podcast" && row.kind == "episode" {
                    if row.artwork.is_empty() {
                        row.artwork = entity.artwork.clone();
                    }
                    if row.subtitle.is_empty() {
                        row.subtitle = entity.title.clone();
                    }
                    if row.raw["show"].is_null() {
                        row.raw["show"] = entity.raw.clone();
                    }
                }
                if entity.kind == "album" && row.kind == "track" {
                    if row.artwork.is_empty() {
                        row.artwork = entity.artwork.clone();
                    }
                    if row.album.is_empty() {
                        row.album = entity.title.clone();
                    }
                    if row.album_id.is_empty() {
                        row.album_id = entity.id.clone();
                    }
                    if row.artist_id.is_empty() {
                        row.artist_id = entity.artist_id.clone();
                    }
                    if row.subtitle.is_empty() {
                        row.subtitle = entity.subtitle.clone();
                    }
                }
            }
        }
    }
    fn append(page: &mut Page, cursor: &str, value: Value) {
        if let Some(index) = page
            .raw
            .iter()
            .find(|(key, _)| key == "spotify:pagination-section")
            .and_then(|(_, value)| value.as_u64())
            .map(|value| value as usize)
        {
            if let Some(section) = page.sections.get_mut(index) {
                let kind = section
                    .entries
                    .first()
                    .map(|entry| entry.kind.as_str())
                    .unwrap_or("track");
                let mut seen: HashSet<_> = section
                    .entries
                    .iter()
                    .map(|entry| (entry.kind.clone(), entry.id.clone()))
                    .collect();
                section.entries.extend(
                    rows(&value, kind)
                        .into_iter()
                        .filter(|entry| seen.insert((entry.kind.clone(), entry.id.clone()))),
                );
            }
            let next = string(&value, &["next"]);
            page.next = if next == cursor
                || page
                    .raw
                    .iter()
                    .any(|(key, _)| key == &format!("next:{next}"))
            {
                String::new()
            } else {
                next
            };
            page.raw.push((format!("next:{cursor}"), Value::Null));
            page.error.clear();
            return;
        }
        let start = page.rows.len();
        let kind = page
            .rows
            .first()
            .map(|row| row.kind.as_str())
            .unwrap_or("track");
        let more = rows(&value, kind);
        let mut seen: HashSet<_> = page
            .rows
            .iter()
            .map(|row| (row.kind.clone(), row.id.clone()))
            .collect();
        page.rows.extend(
            more.into_iter()
                .filter(|row| seen.insert((row.kind.clone(), row.id.clone()))),
        );
        let next = string(&value, &["next"]);
        page.next = if next == cursor
            || page
                .raw
                .iter()
                .any(|(key, _)| key == &format!("next:{next}"))
        {
            String::new()
        } else {
            next
        };
        page.error.clear();
        page.raw.push((format!("next:{cursor}"), Value::Null));
        page.raw.push((
            "cache:chunk".into(),
            json!({"cursor":cursor,"start":start,"end":page.rows.len()}),
        ));
        apply_entity_metadata(page);
    }

    pub(super) fn load(
        _engine: &Engine,
        view: View,
        entity: Option<Entry>,
        query: String,
        publish: &mut dyn FnMut(&Page) -> Result<(), String>,
    ) -> Result<Page, String> {
        load_page(view, entity, query, publish)
    }
    fn recent_history(page: &mut Page, value: &Value) {
        let entries = rows(value, "track");
        // Web history contains listening contexts; OAuth history contains tracks.
        if entries
            .iter()
            .any(|entry| !matches!(entry.kind.as_str(), "track" | "episode"))
        {
            page.sections.push(Section {
                title: page.title.clone(),
                entries,
                ..Default::default()
            });
        } else {
            page.rows = entries;
        }
    }

    fn load_page(
        view: View,
        entity: Option<Entry>,
        query: String,
        publish: &mut dyn FnMut(&Page) -> Result<(), String>,
    ) -> Result<Page, String> {
        let mut page = Page {
            title: entity
                .as_ref()
                .map(|e| e.title.clone())
                .unwrap_or_else(|| view.title().into()),
            entity: entity.clone(),
            loading: true,
            ..Default::default()
        };
        publish(&page)?;
        if view == View::StyleMix && query.is_empty() {
            page.loading = false;
            return Ok(page);
        }
        if view == View::Home {
            page.title = "For you".into();
            page.raw.push(("provider".into(), json!("spotify")));
            let shelves = [
                ("Your playlists", 2, "playlist"),
                ("Recently played", 0, "track"),
                ("Liked songs", 1, "track"),
                ("Saved albums", 3, "album"),
                ("Followed artists", 4, "artist"),
                ("Saved podcasts", 7, "podcast"),
                ("Saved episodes", 9, "episode"),
            ];
            page.title = "For you".into();
            page.subtitle = "Your Spotify library and listening history".into();
            page.sections = shelves
                .iter()
                .map(|(title, _, _)| Section {
                    title: (*title).into(),
                    ..Default::default()
                })
                .collect();
            let mut personal_count = 0;
            publish(&page)?;
            parallel::load(
                shelves.len() + 1,
                |index| {
                    if index == 0 {
                        return Err(invoke("SPCatalogHomeJSON", &[])
                            .map(|value| home_sections(&value))
                            .unwrap_or_default());
                    }
                    let (title, module, kind) = shelves[index - 1];
                    Ok(match invoke("DZHomeModuleJSON", &[json!(module)]) {
                        Ok(value) => Section {
                            title: title.into(),
                            entries: rows(&value, kind),
                            ..Default::default()
                        },
                        Err(error) => Section {
                            title: title.into(),
                            error,
                            ..Default::default()
                        },
                    })
                },
                |index, result| {
                    let section = match result {
                        Ok(section) => section,
                        Err(mut personal) => {
                            personal.retain(presentable_home_shelf);
                            personal_count = personal.len();
                            page.sections.splice(2..2, personal);
                            return publish_curated(&page, publish);
                        }
                    };
                    let index = index - 1;
                    let section_index = if index < 2 {
                        index
                    } else {
                        index + personal_count
                    };
                    page.sections[section_index] = section;
                    page.raw.push((format!("home:{index}"), Value::Null));
                    publish_curated(&page, publish)
                },
            )?;
            curate_shelves(&mut page.sections);
        } else if view == View::Explore && query.is_empty() {
            page.raw.push(("discover".into(), json!(true)));
            publish(&page)?;
            let value = invoke("SPCatalogDiscoveryJSON", &[])?;
            page.sections = discovery_sections(&value);
            for title in ["Genres", "Categories", "Podcasts by category"] {
                page.raw.push((
                    format!("spotify:shelf:{title}"),
                    json!({"directory":true,"complete":false}),
                ));
            }
            publish(&page)?;
            let sources = [
                (
                    "spotify:page:0JQ5DAqbMKFSi39LMRT0Cy",
                    "Editorial playlists",
                    "playlist",
                ),
                (
                    "spotify:page:0JQ5DAqbMKFz6FAsUtgAab",
                    "New releases",
                    "album",
                ),
                ("spotify:page:0JQ5DAudkNjCgYMM0TZXDw", "Charts", "playlist"),
            ];
            let available: HashSet<_> = page
                .sections
                .iter()
                .flat_map(|s| s.entries.iter())
                .map(|entry| entry.id.clone())
                .collect();
            let sources: Vec<_> = sources
                .into_iter()
                .filter(|(uri, _, _)| available.contains(*uri))
                .collect();
            let directories = std::mem::take(&mut page.sections);
            page.sections = sources
                .iter()
                .map(|(_, title, _)| Section {
                    title: (*title).into(),
                    ..Default::default()
                })
                .collect();
            page.sections.extend(directories);
            parallel::load(
                sources.len(),
                |index| {
                    let (uri, title, kind) = sources[index];
                    match invoke("SPCatalogChannelJSON", &[json!(uri)]) {
                        Ok(value) => (
                            Section {
                                title: title.into(),
                                entries: sections(&value)
                                    .into_iter()
                                    .flat_map(|s| s.entries)
                                    .filter(|item| {
                                        item.kind == kind
                                            || (kind == "album" && item.kind == "playlist")
                                    })
                                    .collect(),
                                ..Default::default()
                            },
                            value,
                        ),
                        Err(error) => (
                            Section {
                                title: title.into(),
                                error,
                                ..Default::default()
                            },
                            Value::Null,
                        ),
                    }
                },
                |index, (section, value)| {
                    remember_shelf(&mut page, &section.title, &value, sources[index].2);
                    page.sections[index] = section;
                    publish(&page)
                },
            )?;
            page.sections
                .retain(|section| !section.entries.is_empty() || !section.error.is_empty());
            page.loading = false;
        } else if view == View::Explore {
            let value = invoke("DZSearchJSON", &[json!(query)])?;
            for (key, title, kind) in [
                ("tracks", "Tracks", "track"),
                ("albums", "Albums", "album"),
                ("artists", "Artists", "artist"),
                ("playlists", "Playlists", "playlist"),
                ("shows", "Podcasts", "podcast"),
                ("episodes", "Episodes", "episode"),
            ] {
                page.sections.push(Section {
                    title: title.into(),
                    entries: rows(&value[key], kind),
                    ..Default::default()
                });
            }
            if page.sections.iter().all(|s| s.entries.is_empty()) {
                page.rows = rows(&value, "track");
            }
        } else if matches!(view, View::Album | View::Artist | View::Show) {
            let item = entity.as_ref().ok_or("Select a Spotify item first")?;
            if !item.id.starts_with("spotify:") {
                return Err("Select a Spotify item for this provider".into());
            }
            if item.kind == "channel" {
                let value = invoke("SPCatalogChannelJSON", &[json!(item.id)])?;
                page.sections = sections(&value);
                directory_covers(&mut page.sections, &item.artwork, &invoke);
                for title in page
                    .sections
                    .iter()
                    .map(|s| s.title.clone())
                    .collect::<Vec<_>>()
                {
                    let matching: Vec<_> = value["sections"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter(|s| s["title"] == title)
                        .cloned()
                        .collect();
                    remember_shelf(&mut page, &title, &json!({"sections":matching}), "");
                }
            } else if item.kind == "artist" {
                let value = invoke("DZArtistProfileJSON", &[json!(item.id)])?;
                metadata(&mut page, value.get("artist").unwrap_or(&value));
                let top = value
                    .get("top")
                    .or_else(|| value.get("topTracks"))
                    .cloned()
                    .unwrap_or(Value::Null);
                page.sections.push(Section {
                    title: "Popular tracks".into(),
                    entries: tracks(&top),
                    ..Default::default()
                });
                for (index, title, kind) in [
                    (0, "Albums", "album"),
                    (2, "Singles and EPs", "album"),
                    (3, "Appears on", "album"),
                    (4, "Similar artists", "artist"),
                    (5, "Related playlists", "playlist"),
                ] {
                    publish(&page)?;
                    let section = match invoke(
                        "DZArtistSectionJSON",
                        &[json!(item.id), json!(index), json!("")],
                    ) {
                        Ok(value) => Section {
                            title: title.into(),
                            entries: rows(&value, kind),
                            ..Default::default()
                        },
                        Err(error) => Section {
                            title: title.into(),
                            error,
                            ..Default::default()
                        },
                    };
                    page.sections.push(section);
                }
            } else {
                let name = match item.kind.as_str() {
                    "playlist" => "DZPlaylistTracksJSON",
                    "podcast" => "DZPodcastEpisodesJSON",
                    "track" => "DZTrackMetadataJSON",
                    "episode" => "SPCatalogItemJSON",
                    _ => "DZAlbumTracksJSON",
                };
                let args = if item.kind == "episode" {
                    vec![json!("episode"), json!(item.id)]
                } else {
                    vec![json!(item.id)]
                };
                let value = invoke(name, &args)?;
                page.rows = tracks(&value);
                page.next = string(&value, &["next"]);
                match invoke(
                    "SPCatalogItemJSON",
                    &[
                        json!(if item.kind == "podcast" {
                            "show"
                        } else {
                            &item.kind
                        }),
                        json!(item.id),
                    ],
                ) {
                    Ok(value) => metadata(&mut page, &value),
                    Err(error) => page.error = error,
                }
                // Album tracks omit their parent album; playlists must retain each track's album.
                let retained = page.entity.clone();
                if let Some(entity) = retained {
                    metadata(&mut page, &entity.raw);
                }
            }
        } else {
            let (name, args, kind) = match view {
                View::Favorites => ("DZFavoritesJSON", vec![], "track"),
                View::Albums => ("DZFavoriteAlbumsJSON", vec![], "album"),
                View::Playlists | View::PlaylistPicker => ("DZPlaylistsJSON", vec![], "playlist"),
                View::Recent => ("DZHistoryRecentJSON", vec![json!(200)], "track"),
                View::Podcasts => ("DZPodcastShelfJSON", vec![json!(1)], "podcast"),
                View::Queue => ("DZQueueJSON", vec![], "track"),
                View::Flow => {
                    let (name, args) = super::flow_request(entity.as_ref(), &query);
                    (name, args, "track")
                }
                View::StyleMix => ("SPStyleMixJSON", vec![json!(query)], "track"),
                _ => return Err("This page is unavailable for Spotify".into()),
            };
            let value = invoke(name, &args)?;
            if matches!(view, View::Albums | View::Playlists | View::PlaylistPicker) {
                page.sections.push(Section {
                    title: page.title.clone(),
                    entries: rows(&value, kind),
                    ..Default::default()
                });
                page.raw
                    .push(("spotify:pagination-section".into(), json!(0)));
            } else if view == View::Recent {
                recent_history(&mut page, &value);
            } else if view == View::Podcasts {
                let saved = rows(&value, "podcast");
                let bookmarks = invoke("DZPodcastShelfJSON", &[json!(3)])
                    .map(|value| rows(&value, "episode"))
                    .unwrap_or_default();
                page = super::podcast_library_page(saved, Vec::new(), bookmarks, Vec::new());
                page.raw
                    .push(("spotify:pagination-section".into(), json!(0)));
            } else {
                page.rows = rows(&value, kind);
            }
            if view == View::StyleMix {
                if let Some(selection) = args.first().and_then(Value::as_str) {
                    for row in &mut page.rows {
                        row.raw[crate::style_mix::MARKER] = json!(selection);
                    }
                }
            }
            page.next = string(&value, &["next"]);
        }
        if view == View::Flow && entity.as_ref().is_some_and(|entry| entry.kind == "track") {
            page.subtitle = "Song radio".into();
        }
        page.raw.push(("provider".into(), json!("spotify")));
        page.loading = false;
        Ok(page)
    }

    pub(super) fn discovery_report() -> Result<Value, String> {
        let page = load_page(View::Explore, None, String::new(), &mut |_| Ok(()))?;
        Ok(json!({"sections":page.sections.iter().map(|section| json!({
        "title":section.title,"count":section.entries.len(),"error":section.error,
        "entries":section.entries.iter().filter(|e|e.kind=="channel").map(|entry|
            json!({"name":entry.title,"uri":entry.id,"cover":entry.artwork})).collect::<Vec<_>>()
    })).collect::<Vec<_>>()}))
    }
    pub(super) fn process(generation: u64, job: Job, tx: &EventSender, current: &AtomicU64) {
        let active = || current.load(Ordering::Relaxed) == generation;
        match job {
            Job::LibraryState | Job::Collection(..) => {
                unreachable!("library operations are handled before provider dispatch")
            }
            Job::Route(view, entity, query) => {
                let cached = tx.resume(generation);
                if view == View::Flow {
                    if let Some(cached) = cached.as_ref().filter(|p| !p.rows.is_empty()) {
                        let mut page = (**cached).clone();
                        page.loading = false;
                        let _ = tx.send(Event::Page(generation, Arc::new(page)));
                        return;
                    }
                }
                let result = load_page(view, entity, query, &mut |page| {
                    if !active() {
                        return Err("Superseded load".into());
                    }
                    tx.send(Event::Page(generation, Arc::new(page.clone())))
                        .map_err(|_| "Window closed".into())
                });
                if !active() {
                    return;
                }
                let mut page = result.unwrap_or_else(|error| Page {
                    title: view.title().into(),
                    error,
                    ..Default::default()
                });
                for _ in 0..cached
                    .as_deref()
                    .map(session::pagination_depth)
                    .unwrap_or(0)
                {
                    if !page.error.is_empty()
                        || page.next.is_empty()
                        || page.next.starts_with("flow:")
                        || !active()
                    {
                        break;
                    }
                    let cursor = page.next.clone();
                    match invoke("SPCatalogNextJSON", &[json!(cursor)]) {
                        Ok(value) => append(&mut page, &cursor, value),
                        Err(error) => {
                            page.error = error;
                            break;
                        }
                    }
                }
                if active() {
                    let _ = tx.send(Event::Page(generation, Arc::new(page)));
                }
            }
            Job::More(mut page) => {
                if let Some(index) = page.raw.iter().position(|(k, _)| k == "spotify:more-shelf") {
                    let (_, request) = page.raw.remove(index);
                    let title = request["title"].as_str().unwrap_or("");
                    let target = request["target"].as_u64().unwrap_or(0) as usize;
                    if let Err(error) = load_shelf_more(
                        &mut page,
                        title,
                        target,
                        &mut |partial| {
                            if !active() {
                                return Err("Superseded load".into());
                            }
                            tx.send(Event::Page(generation, Arc::new(partial.clone())))
                                .map_err(|_| "Window closed".into())
                        },
                        &invoke,
                    ) {
                        page.error = error;
                    }
                    let cover = page
                        .entity
                        .as_ref()
                        .map(|item| item.artwork.as_str())
                        .unwrap_or("");
                    directory_covers(&mut page.sections, cover, &invoke);
                    page.loading = false;
                    if active() {
                        let _ = tx.send(Event::Page(generation, Arc::new(page)));
                    }
                    return;
                }
                let cursor = page.next.clone();
                let result = if let Some(config) = cursor.strip_prefix("flow:") {
                    invoke(
                        if config.is_empty() || config == "default" {
                            "DZFlowJSON"
                        } else {
                            "DZFlowForConfigJSON"
                        },
                        &if config.is_empty() || config == "default" {
                            vec![]
                        } else {
                            vec![json!(config)]
                        },
                    )
                } else {
                    invoke("SPCatalogNextJSON", &[json!(cursor)])
                };
                match result {
                    Ok(value) if cursor.starts_with("flow:") => {
                        append_flow_rows(&mut page, tracks(&value));
                        page.error.clear();
                    }
                    Ok(value) => append(&mut page, &cursor, value),
                    Err(error) => page.error = error,
                }
                page.loading = false;
                if active() {
                    let _ = tx.send(Event::Page(generation, Arc::new(page)));
                }
            }
            Job::Api(name, args) => {
                let result = if name == "SpotifyStyleMixGenres" {
                    invoke("StyleMixGenres", &args)
                } else {
                    invoke(&name, &args).map(|value| spotify_value(&value))
                };
                let _ = tx.send(Event::Api(generation, Provider::Spotify, name, result));
            }
            Job::TrackInfo(track) => {
                let (meta, artist) = track_info(&track, &invoke);
                let meta = super::musicbrainz::enrich(&track, meta);
                if active() {
                    let _ = tx.send(Event::TrackInfo(generation, Box::new(track), meta, artist));
                }
            }
            Job::Playlist(mut mutation) => {
                let result = crate::playlist::execute_spotify(&mut mutation);
                let _ = tx.send(Event::Playlist(
                    generation,
                    Provider::Spotify,
                    mutation,
                    result,
                ));
            }
            Job::Lyrics(..) | Job::LyricsWindow(..) => {} // Forwarded to the bounded lyrics worker.
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn track_info_resolves_artist_from_full_credits_and_unwraps_profile() {
            let track = Entry {
                id: "spotify:track:song".into(),
                ..Default::default()
            };
            let (meta, artist) = track_info(&track, &|name, args| match name {
                "DZTrackMetadataJSON" => Ok(json!({"id":"spotify:track:song","name":"Song",
                "artists":[{"id":"spotify:artist:primary","name":"Artist"}]})),
                "DZArtistProfileJSON" => {
                    assert_eq!(args, &[json!("spotify:artist:primary")]);
                    Ok(
                        json!({"artist":{"id":"spotify:artist:primary","name":"Artist",
                    "monthlyListeners":900,"followers":100},"top":{"items":[]}}),
                    )
                }
                _ => panic!("Unexpected request: {name}"),
            });
            assert_eq!(meta["name"], "Song");
            assert_eq!(artist["monthlyListeners"], 900);
            assert_eq!(artist["followers"], 100);
            assert!(entry(&artist, "artist").is_some());
        }

        #[test]
        fn recent_contexts_use_cards_and_track_history_keeps_track_rows() {
            let mut page = Page {
                title: "Recently played".into(),
                ..Default::default()
            };
            recent_history(
                &mut page,
                &json!({"items":[
                    {"uri":"spotify:playlist:mix", "type":"playlist", "name":"Daily Mix 5"},
                    {"uri":"spotify:album:album", "type":"album", "name":"Album"},
                    {"uri":"spotify:track:song", "type":"track", "name":"Song", "duration_ms":180000}
                ]}),
            );
            assert!(page.rows.is_empty());
            assert_eq!(page.sections.len(), 1);
            assert_eq!(page.sections[0].title, "Recently played");
            let entries = &page.sections[0].entries;
            assert_eq!(
                entries.iter().map(|e| e.kind.as_str()).collect::<Vec<_>>(),
                vec!["playlist", "album", "track"]
            );
            assert_eq!(entries[0].id, "spotify:playlist:mix");

            let mut tracks = Page::default();
            recent_history(
                &mut tracks,
                &json!({"items":[
                    {"track":{"uri":"spotify:track:song", "type":"track", "name":"Song", "duration_ms":180000},
                     "played_at":"2026-10-28T10:00:00Z"}
                ]}),
            );
            assert!(tracks.sections.is_empty());
            assert_eq!(tracks.rows.len(), 1);
            assert_eq!(tracks.rows[0].duration, 180000);
        }
        #[test]
        fn nested_genre_covers_use_cached_art_then_parent_without_downloading() {
            let genre = |id: &str, art: &str| Entry {
                id: id.into(),
                kind: "channel".into(),
                artwork: art.into(),
                ..Default::default()
            };
            let mut shelves = vec![Section {
                title: "Related genres".into(),
                entries: vec![
                    genre("spotify:page:house", ""),
                    genre("spotify:page:techno", ""),
                    genre("spotify:page:rock", "https://i.scdn.co/image/official"),
                ],
                ..Default::default()
            }];
            let fetch = |name: &str, args: &[Value]| {
                assert_eq!(name, "SPCatalogDirectoryCachedJSON");
                if args[0] == "spotify:page:house" {
                    Ok(json!({"sections":[{"title":"House essentials","items":[{
                        "type":"playlist","uri":"spotify:playlist:house","name":"House",
                        "images":[{"url":"https://i.scdn.co/image/house"}]
                    }]}]}))
                } else {
                    Ok(Value::Null)
                }
            };
            directory_covers(&mut shelves, "https://i.scdn.co/image/parent", &fetch);
            assert_eq!(
                shelves[0].entries[0].artwork,
                "https://i.scdn.co/image/house"
            );
            assert_eq!(
                shelves[0].entries[1].artwork,
                "https://i.scdn.co/image/parent"
            );
            assert_eq!(
                shelves[0].entries[2].artwork,
                "https://i.scdn.co/image/official"
            );
            assert_eq!(
                shelves[0].entries[1].raw["directory_artwork_fallback"],
                true
            );
        }
        #[test]
        fn shelf_more_fetches_only_the_next_two_rows_and_advances_the_cursor() {
            let mut page = Page {
                sections: vec![Section {
                    title: "Rock Classics".into(),
                    entries: (0..8)
                        .map(|i| Entry {
                            kind: "playlist".into(),
                            id: format!("spotify:playlist:{i}"),
                            ..Default::default()
                        })
                        .collect(),
                    ..Default::default()
                }],
                ..Default::default()
            };
            remember_shelf(
                &mut page,
                "Rock Classics",
                &json!({"sections":[{
                    "uri":"spotify:section:classics","offset":8,"total":30
                }]}),
                "",
            );
            let calls = std::sync::atomic::AtomicUsize::new(0);
            let fetch = |name: &str, args: &[Value]| {
                assert_eq!(name, "SPCatalogSectionJSON");
                assert_eq!(args[2], 8);
                calls.fetch_add(1, Ordering::Relaxed);
                let offset = args[1].as_u64().unwrap();
                Ok(
                    json!({"offset":offset+8,"total":30,"items":(offset..offset+8)
                .map(|i|json!({"type":"playlist","uri":format!("spotify:playlist:{i}"),"name":format!("Playlist {i}")})).collect::<Vec<_>>()}),
                )
            };
            for target in [16, 24] {
                load_shelf_more(&mut page, "Rock Classics", target, &mut |_| Ok(()), &fetch)
                    .unwrap();
                assert_eq!(page.sections[0].entries.len(), target);
            }
            assert_eq!(calls.load(Ordering::Relaxed), 2);
            assert_eq!(page.raw[0].1["sections"][0]["offset"], 24);
            load_shelf_more(&mut page, "Rock Classics", 24, &mut |_| Ok(()), &|_, _| {
                panic!("already loaded")
            })
            .unwrap();
        }
        #[test]
        fn nested_directories_expand_progressively_with_a_bounded_download_budget() {
            let mut page = Page {
                sections: vec![Section {
                    title: "Genres".into(),
                    entries: (0..20)
                        .map(|i| Entry {
                            id: format!("spotify:page:root{i}"),
                            title: format!("Genre {i}"),
                            kind: "channel".into(),
                            artwork: "https://i.scdn.co/image/parent".into(),
                            ..Default::default()
                        })
                        .collect(),
                    ..Default::default()
                }],
                ..Default::default()
            };
            let requests = std::sync::atomic::AtomicUsize::new(0);
            let mut updates = 0;
            expand_discovery_directories(
                &mut page,
                &mut |_| {
                    updates += 1;
                    Ok(())
                },
                &|name, args| {
                    if name == "SPCatalogDirectoryCachedJSON" {
                        return Ok(Value::Null);
                    }
                    requests.fetch_add(1, Ordering::Relaxed);
                    if args[0] == "spotify:page:root0" {
                        Ok(json!({"sections":[{"title":"Related content","items":[
                            {"type":"category","uri":"spotify:page:house","name":"House"},
                            {"type":"category","uri":"spotify:page:techno","name":"Techno"},
                        ]}]}))
                    } else {
                        Ok(Value::Null)
                    }
                },
                3,
                None,
            )
            .unwrap();
            assert!(updates > 1);
        }

        #[test]
        fn expanding_genres_does_not_add_or_fetch_categories() {
            let mut page = Page {
                sections: vec![Section {
                    title: "Genres".into(),
                    entries: vec![Entry {
                        id: "spotify:page:root".into(),
                        kind: "channel".into(),
                        ..Default::default()
                    }],
                    ..Default::default()
                }],
                ..Default::default()
            };
            expand_discovery_directories(
            &mut page,
            &mut |_| Ok(()),
            &|name, args| {
                if name == "SPCatalogDirectoryCachedJSON" {
                    return Ok(Value::Null);
                }
                assert_ne!(args[0], "spotify:page:discover");
                Ok(if args[0] == "spotify:page:root" {
                    json!({"sections":[{"title":"Related","items":[
                        {"type":"category","uri":"spotify:page:discover","name":"Discover Jazz"},
                        {"type":"category","uri":"spotify:page:house","name":"House"}
                    ]}]})
                } else {
                    Value::Null
                })
            },
            3,
            Some(("Genres", 10)),
        )
        .unwrap();
            assert_eq!(page.sections.len(), 1);
            assert_eq!(page.sections[0].title, "Genres");
            assert!(page.sections[0]
                .entries
                .iter()
                .any(|e| e.id == "spotify:page:house"));
            assert!(!page.sections[0]
                .entries
                .iter()
                .any(|e| e.id == "spotify:page:discover"));
        }

        #[test]
        fn cached_directory_expansion_does_not_request_the_network() {
            let mut page = Page {
                sections: vec![Section {
                    title: "Genres".into(),
                    entries: vec![Entry {
                        id: "spotify:page:root0".into(),
                        title: "Genre 0".into(),
                        kind: "channel".into(),
                        artwork: "https://i.scdn.co/image/parent".into(),
                        ..Default::default()
                    }],
                    ..Default::default()
                }],
                ..Default::default()
            };
            let requests = std::sync::atomic::AtomicUsize::new(0);
            let mut updates = 0;
            expand_discovery_directories(
                &mut page,
                &mut |_| {
                    updates += 1;
                    Ok(())
                },
                &|name, _args| {
                    if name == "SPCatalogDirectoryCachedJSON" {
                        return Ok(json!({"sections":[{"title":"Related content","items":[
                            {"type":"category","uri":"spotify:page:house","name":"House"}
                        ]}]}));
                    }
                    requests.fetch_add(1, Ordering::Relaxed);
                    Ok(Value::Null)
                },
                3,
                None,
            )
            .unwrap();
            assert_eq!(requests.load(Ordering::Relaxed), 0);
            assert!(updates > 0);
        }

        #[test]
        fn localized_browse_all_is_split_into_real_genres_moods_and_podcasts() {
            let payload = json!({"sections": [
                {"title": "Genres", "items": [{"type": "category", "id": "spotify:genre:rock", "name": "Rock"}]},
                {"title": "Browse all", "items": [{"type": "category", "uri": "spotify:page:focus", "name": "Focus"}]},
                {"title": "Podcasts", "items": [{"type": "category", "uri": "spotify:page:podcasts", "name": "Podcasts"}]},
                {"title": "Recommended playlists", "items": [{"type": "playlist", "uri": "spotify:playlist:mix", "name": "Mix"}]}
            ]});
            let shelves = discovery_sections(&payload);
            assert!(shelves[..3].iter().all(|s| s.entries[0].kind == "channel"));
            for section in &shelves {
                assert!(super::super::discover_section_matches(section, 0));
            }
        }

        #[test]
        fn genre_named_shelf_does_not_turn_mood_categories_into_genres() {
            let payload = json!({"sections": [
                {"title": "Hepsine göz at", "items": [
                    {"type": "category", "uri": "spotify:page:0JQ5DAqbMKFDXXwE9BDJAr", "name": "Odaklan"},
                    {"type": "category", "uri": "spotify:page:0JQ5IMCbQBLl5gxKLgufp8", "name": "Eğitim"}
                ]}
            ]});
            let shelves = discovery_sections(&payload);
            assert!(shelves
                .iter()
                .any(|s| s.title == "Categories" || s.title == "Podcasts by category"));
        }

        #[test]
        fn home_keeps_repeated_cards_in_their_original_shelves() {
            let shelves = home_sections(&json!({
                "sections": [
                    {"title": "Your playlists", "items": [{"type": "playlist", "uri": "spotify:playlist:m1", "name": "Daily Mix 1"}]},
                    {"title": "Daily Mix 1", "items": [{"type": "playlist", "uri": "spotify:playlist:m1", "name": "Daily Mix 1"}]}
                ]
            }));
            assert_eq!(shelves.len(), 2);
        }

        #[test]
        fn personalized_home_hides_sparse_and_unresolved_shelves() {
            let shelf = |title: &str, count: usize| Section {
                title: title.into(),
                entries: vec![Entry::default(); count],
                ..Default::default()
            };
            assert!(!presentable_home_shelf(&shelf(
                "Based on your recent listening",
                1
            )));
            assert!(!presentable_home_shelf(&shelf("New episode from {0}", 2)));
            assert!(!presentable_home_shelf(&shelf(" ", 2)));
            assert!(presentable_home_shelf(&shelf("Made for you", 2)));
        }

        #[test]
        fn home_combines_matching_context_shelves() {
            let shelves = home_sections(&json!({"sections": [
                {"title":"Made for you","items":[{"type":"playlist","uri":"spotify:playlist:a","name":"A"}]},
                {"title":"Other","items":[{"type":"playlist","uri":"spotify:playlist:a","name":"A"}]},
                {"title":"Made for you","items":[{"type":"playlist","uri":"spotify:playlist:b","name":"B"},{"type":"playlist","uri":"spotify:playlist:a","name":"A"}]}
            ]}));
            assert_eq!(shelves.len(), 2);
            assert_eq!(
                shelves[0]
                    .entries
                    .iter()
                    .map(|e| e.title.as_str())
                    .collect::<Vec<_>>(),
                ["A", "B"]
            );
            assert_eq!(shelves[1].entries.len(), 1);
        }

        #[test]
        fn shelf_variety_limits_artist_repeats_but_keeps_all_daily_mixes() {
            let shelves = home_sections(&json!({
                "sections": [
                    {"title": "Made for you", "items": [
                        {"type": "playlist", "uri": "spotify:playlist:m1", "name": "Daily Mix 1"},
                        {"type": "playlist", "uri": "spotify:playlist:m2", "name": "Daily Mix 2"}
                    ]}
                ]
            }));
            assert_eq!(shelves[0].entries.len(), 2);
        }

        #[test]
        fn home_groups_real_radios_artist_playlists_and_artists_without_duplicates() {
            let shelves = home_sections(&json!({
                "sections": [
                    {"title": "Radios for you", "items": [{"type": "playlist", "uri": "spotify:playlist:r1", "name": "Artist Radio"}, {"type": "playlist", "uri": "spotify:playlist:r2", "name": "Other Radio"}]},
                    {"title": "This Is…", "items": [{"type": "playlist", "uri": "spotify:playlist:t1", "name": "This Is Artist"}]},
                    {"title": "Recommended artists", "items": [{"type": "artist", "uri": "spotify:artist:a1", "name": "Artist"}]},
                    {"title": "Made for you", "items": [{"type": "playlist", "uri": "spotify:playlist:m1", "name": "Daily Mix 1"}]}
                ]
            }));
            assert_eq!(
                shelves.iter().map(|s| s.title.as_str()).collect::<Vec<_>>(),
                [
                    "Radios for you",
                    "This Is…",
                    "Recommended artists",
                    "Made for you"
                ]
            );
            assert_eq!(
                shelves.iter().map(|s| s.entries.len()).collect::<Vec<_>>(),
                vec![2, 1, 1, 1]
            );
            assert_eq!(shelves[3].entries[0].title, "Daily Mix 1");
            assert!(home_sections(&json!({"sections":[]})).is_empty());
        }
        #[test]
        fn language_genre_matches_require_both_whole_word_aliases() {
            let selection = crate::style_mix::Selection::new(
                0,
                crate::style_mix::GENRES
                    .iter()
                    .position(|genre| genre.0 == "Rock")
                    .unwrap(),
            )
            .unwrap();
            assert!(matches_selection("Türkçe Rock", selection));
            assert!(!matches_selection("Turkish Pop", selection));
            assert!(!matches_selection("Japanese Rock", selection));
            assert!(!matches_selection("Turkish Rocket", selection));
        }
        #[test]
        fn spotify_identity_credits_art_and_duration_survive_shared_wire() {
            let value = json!({"type":"track","id":"song","uri":"spotify:track:song","name":"Song","duration_ms":1234,
            "artists":[{"id":"artist","name":"Artist"}],"album":{"id":"album","name":"Album","images":[{"url":"https://i.scdn.co/cover"}]}});
            let track = entry(&value, "track").unwrap();
            assert_eq!(track.duration, 1234);
            assert_eq!(track.id, "spotify:track:song");
            assert_eq!(track.artist_id, "spotify:artist:artist");
            assert_eq!(track.album_id, "spotify:album:album");
            assert_eq!(track.artwork, "https://i.scdn.co/cover");
            assert_eq!(track.artist_entries()[0].id, "spotify:artist:artist");
            assert_eq!(track.album_entry().unwrap().id, "spotify:album:album");
            assert_eq!(track.wire()["id"], "spotify:track:song");
        }
        #[test]
        fn wrappers_keep_history_and_show_metadata_without_inventing_metrics() {
            let result = rows(
                &json!({"items":[{"played_at":"2026-10-27","track":{"id":"a","type":"track","name":"Song"}}, {"track":null}]}),
                "track",
            );
            assert_eq!(result.len(), 1);
            assert_eq!(result[0].raw["played_at"], "2026-10-27");
            let show=entry(&json!({"uri":"spotify:show:a","name":"Show","total_episodes":5,"publisher":"Publisher"}),"podcast").unwrap();
            assert_eq!(show.kind, "podcast");
            assert_eq!(show.episode_count(), Some(5));
            let artist = entry(
                &json!({"uri":"spotify:artist:a","name":"Artist","followers":{"total":45}}),
                "artist",
            )
            .unwrap();
            assert_eq!(artist.raw["followers"], 45);
            assert!(artist.raw["monthlyListeners"].is_null());
        }
        #[test]
        fn library_pagination_keeps_collections_and_stops_cyclic_cursors() {
            let mut page = Page {
                rows: rows(
                    &json!([{"id":"a","name":"Album A","type":"album"}]),
                    "album",
                ),
                ..Default::default()
            };
            append(
                &mut page,
                "https://api.spotify.com/v1/me/albums?page=2",
                json!({"items":[{"album":{"id":"b","name":"Album B","type":"album"}}],"next":"https://api.spotify.com/v1/me/albums?page=2"}),
            );
            assert_eq!(page.rows.len(), 2);
            assert_eq!(page.rows[1].kind, "album");
            assert!(page.next.is_empty());
        }
        #[test]
        fn podcast_show_page_and_episode_pagination_preserve_show_metadata_and_playable_episodes() {
            let episodes_payload = json!({
                "items": [
                    {
                        "uri": "spotify:episode:ep1",
                        "id": "spotify:episode:ep1",
                        "name": "Episode 1",
                        "type": "episode",
                        "duration_ms": 1800000,
                        "release_date": "2024-03-01",
                        "images": [{"url": "https://i.scdn.co/image/ep1", "width": 300, "height": 300}],
                        "show": {
                            "uri": "spotify:show:show1",
                            "id": "spotify:show:show1",
                            "name": "My Show",
                            "type": "show",
                            "images": [{"url": "https://i.scdn.co/image/show1", "width": 640, "height": 640}]
                        }
                    }
                ],
                "next": "spotify-web:cursor2"
            });
            let initial_rows = tracks(&episodes_payload);
            assert_eq!(initial_rows.len(), 1);
            assert_eq!(initial_rows[0].kind, "episode");
            assert_eq!(initial_rows[0].title, "Episode 1");
            assert_eq!(initial_rows[0].subtitle, "My Show");

            let mut page = Page {
                rows: initial_rows,
                next: "spotify-web:cursor2".into(),
                ..Default::default()
            };

            let show_meta = json!({
                "uri": "spotify:show:show1",
                "name": "My Show",
                "type": "show",
                "publisher": "Host Name",
                "images": [{"url": "https://i.scdn.co/image/show1", "width": 640, "height": 640}]
            });
            metadata(&mut page, &show_meta);
            assert!(page.entity.is_some());
            assert_eq!(page.entity.as_ref().unwrap().kind, "podcast");
            assert_eq!(
                page.entity.as_ref().unwrap().artwork,
                "https://i.scdn.co/image/show1"
            );

            let more_payload = json!({
                "items": [
                    {
                        "uri": "spotify:episode:ep2",
                        "id": "spotify:episode:ep2",
                        "name": "Episode 2",
                        "type": "episode",
                        "duration_ms": 2000000,
                        "release_date": "2024-02-20"
                    }
                ],
                "next": ""
            });
            append(&mut page, "spotify-web:cursor2", more_payload);
            assert_eq!(page.rows.len(), 2);
            assert_eq!(page.rows[1].kind, "episode");
            assert_eq!(page.rows[1].title, "Episode 2");
            assert_eq!(page.rows[1].artwork, "https://i.scdn.co/image/show1");
            assert_eq!(page.rows[1].subtitle, "My Show");
            assert!(page.next.is_empty());
        }
    }
}
pub(crate) fn spotify_discovery_report() -> Result<serde_json::Value, String> {
    spotify::discovery_report()
}
pub(crate) fn shelf_has_more(page: &Page, title: &str) -> bool {
    if deezer_shelves::has_more(page, title) {
        return true;
    }
    page.raw
        .iter()
        .find(|(key, _)| key == &format!("spotify:shelf:{title}"))
        .is_some_and(|(_, state)| {
            if state["directory"] == true {
                state["complete"] != true
            } else {
                state["sections"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .any(|s| s["total"].as_u64().unwrap_or(0) > s["offset"].as_u64().unwrap_or(0))
            }
        })
}
pub(crate) fn catalogue_page(page: &Page, title: &str) -> Option<Page> {
    let section = page.sections.iter().find(|s| s.title == title)?.clone();
    let mut related = page.clone();
    related.title = title.into();
    related.subtitle.clear();
    related.entity = None;
    related.sections = vec![section];
    related.rows.clear();
    related.next.clear();
    related.error.clear();
    related.loading = false;
    related
        .raw
        .retain(|(key, _)| key != "discover" && key != "spotify:more-shelf");
    related.raw.push(("catalogue".into(), json!(true)));
    Some(related)
}
pub(crate) mod page_updates {
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
                Self::PlaylistTrack { playlist, .. } => {
                    route[2] == "playlist" && route[3] == *playlist
                }
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
            Change::Library(item("podcast", "new"), true)
                .apply(&key("Podcasts", "", ""), &mut page);
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
}
pub(crate) mod parallel {
    use std::sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc,
    };

    /// Fetch independent shelves with bounded concurrency and publish each result as
    /// soon as it is available. A superseded page stops starting additional work.
    pub(crate) fn load<T: Send>(
        count: usize,
        fetch: impl Fn(usize) -> T + Sync,
        ready: impl FnMut(usize, T) -> Result<(), String>,
    ) -> Result<(), String> {
        load_with(3, count, fetch, ready)
    }

    pub(crate) fn load_with<T: Send>(
        workers: usize,
        count: usize,
        fetch: impl Fn(usize) -> T + Sync,
        mut ready: impl FnMut(usize, T) -> Result<(), String>,
    ) -> Result<(), String> {
        let workers = workers.max(1);
        let next = AtomicUsize::new(0);
        let cancelled = AtomicBool::new(false);
        let (tx, rx) = mpsc::sync_channel(workers);
        std::thread::scope(|scope| {
            for _ in 0..count.min(workers) {
                let tx = tx.clone();
                let fetch = &fetch;
                let next = &next;
                let cancelled = &cancelled;
                scope.spawn(move || loop {
                    if cancelled.load(Ordering::Relaxed) {
                        break;
                    }
                    let index = next.fetch_add(1, Ordering::Relaxed);
                    if index >= count {
                        break;
                    }
                    if tx.send((index, fetch(index))).is_err() {
                        break;
                    }
                });
            }
            drop(tx);
            for (index, value) in rx {
                if let Err(error) = ready(index, value) {
                    cancelled.store(true, Ordering::Relaxed);
                    return Err(error);
                }
            }
            Ok(())
        })
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use std::{sync::Mutex, time::Duration};
        #[test]
        fn a_slow_first_shelf_does_not_delay_ready_shelves() {
            let (release, blocked) = mpsc::channel();
            let blocked = Mutex::new(blocked);
            let mut order = Vec::new();
            load(
                6,
                |index| {
                    if index == 0 {
                        blocked
                            .lock()
                            .unwrap()
                            .recv_timeout(Duration::from_secs(2))
                            .unwrap();
                    }
                    index
                },
                |index, value| {
                    assert_eq!(index, value);
                    if order.is_empty() {
                        assert_ne!(index, 0);
                        release.send(()).unwrap();
                    }
                    order.push(index);
                    Ok(())
                },
            )
            .unwrap();
            order.sort();
            assert_eq!(order, [0, 1, 2, 3, 4, 5]);
        }
        #[test]
        fn superseded_loads_stop_scheduling_the_rest() {
            let calls = AtomicUsize::new(0);
            let result = load(
                1000,
                |_| {
                    calls.fetch_add(1, Ordering::Relaxed);
                },
                |_, _| Err("superseded".into()),
            );
            assert_eq!(result.unwrap_err(), "superseded");
            assert!(calls.load(Ordering::Relaxed) < 1000);
        }
    }
}
pub(crate) mod session {
    //! Per-session page snapshots. Disk reads/writes run on catalog workers, never
    //! the window thread. Account identity is part of the caller's cache key.
    use super::{Entry, Page};
    use serde::{Deserialize, Serialize};
    use std::{
        collections::HashMap,
        fs::{self, File, OpenOptions},
        hash::{Hash, Hasher},
        path::{Path, PathBuf},
        sync::{Arc, Mutex},
        time::{SystemTime, UNIX_EPOCH},
    };

    pub(crate) struct Store {
        root: PathBuf,
        state: Mutex<State>,
        writer: Mutex<()>,
        lease: Option<File>,
    }
    #[derive(Default)]
    struct State {
        routes: HashMap<u64, Route>,
        newest: HashMap<String, u64>,
        saved: HashMap<String, (u64, usize)>,
        epoch: u64,
        edits: Vec<(String, super::page_updates::Change)>,
    }
    struct Route {
        key: String,
        page: Option<Arc<Page>>,
    }
    #[derive(Serialize, Deserialize)]
    struct Document {
        version: u32,
        key: String,
        page: Page,
    }
    // Serialize the persistent view without cloning every entry and its raw JSON.
    #[derive(Serialize)]
    struct StoredDocument<'a> {
        version: u32,
        key: &'a str,
        page: StoredPage<'a>,
    }
    #[derive(Serialize)]
    struct StoredPage<'a> {
        title: &'a str,
        subtitle: &'a str,
        sections: Vec<StoredSection<'a>>,
        rows: &'a [Entry],
        entity: &'a Option<Entry>,
        raw: Vec<&'a (String, serde_json::Value)>,
        next: &'a str,
        loading: bool,
        error: &'static str,
    }
    #[derive(Serialize)]
    struct StoredSection<'a> {
        title: &'a str,
        entries: &'a [Entry],
        error: &'static str,
    }
    impl<'a> StoredDocument<'a> {
        fn new(key: &'a str, page: &'a Page) -> Self {
            Self {
                version: 1,
                key,
                page: StoredPage {
                    title: &page.title,
                    subtitle: &page.subtitle,
                    sections: page
                        .sections
                        .iter()
                        .map(|section| StoredSection {
                            title: &section.title,
                            entries: &section.entries,
                            error: "",
                        })
                        .collect(),
                    rows: &page.rows,
                    entity: &page.entity,
                    raw: page
                        .raw
                        .iter()
                        .filter(|(key, _)| {
                            !key.starts_with("home:") && !key.starts_with("discover-source:")
                        })
                        .collect(),
                    next: &page.next,
                    loading: false,
                    error: "",
                },
            }
        }
    }

    fn is_flow_key(key: &str) -> bool {
        serde_json::from_str::<serde_json::Value>(key)
            .ok()
            .and_then(|value| value.get(1).and_then(|v| v.as_str()).map(|v| v == "Flow"))
            .unwrap_or(false)
    }

    impl Store {
        /// Keep confirmed local edits over older cached/in-flight snapshots.
        pub(crate) fn record_change(&self, account: &str, mut change: super::page_updates::Change) {
            let mut state = self.state.lock().unwrap();
            if let super::page_updates::Change::PlaylistDetails(new) = &mut change {
                for (owner, old) in &state.edits {
                    if owner != account {
                        continue;
                    }
                    if let super::page_updates::Change::PlaylistDetails(old) = old {
                        if old.id == new.id {
                            if new.artwork.is_empty() {
                                new.artwork = old.artwork.clone();
                            }
                            if let Some(fields) = old.raw.as_object() {
                                for (key, value) in fields {
                                    if new.raw.get(key).is_none() {
                                        new.raw[key] = value.clone();
                                    }
                                }
                            }
                        }
                    }
                }
            }
            state
                .edits
                .retain(|(owner, old)| owner != account || !old.same_target(&change));
            for route in state.routes.values_mut() {
                if super::page_updates::route(&route.key).is_some_and(|key| key[0] == account) {
                    if let Some(page) = &mut route.page {
                        if change.affects(&route.key, page) {
                            change.apply(&route.key, Arc::make_mut(page));
                        }
                    }
                }
            }
            state.edits.push((account.into(), change));
        }

        pub(crate) fn patch_page(&self, key: &str, page: &mut Arc<Page>) {
            let state = self.state.lock().unwrap();
            patch(&state, key, page);
        }

        pub(crate) fn patch_library(
            &self,
            account: &str,
            saved: &mut std::collections::HashSet<String>,
        ) {
            let state = self.state.lock().unwrap();
            for (owner, change) in &state.edits {
                if owner != account {
                    continue;
                }
                match change {
                    super::page_updates::Change::Library(item, add) => {
                        if *add {
                            saved.insert(item.library_key());
                        } else {
                            saved.remove(&item.library_key());
                        }
                    }
                    super::page_updates::Change::DeletePlaylist(id) => {
                        saved.remove(&format!("playlist:{id}"));
                    }
                    _ => {}
                }
            }
        }

        pub(crate) fn refresh_flow(&self, key: &str) {
            if is_flow_key(key) {
                let _ = fs::remove_file(self.filename(key));
                let mut state = self.state.lock().unwrap();
                state.routes.retain(|_, route| route.key != key);
                state.saved.remove(key);
            }
        }

        pub(crate) fn for_session() -> Self {
            #[cfg(test)]
            let base =
                std::env::temp_dir().join(format!("deeznats-pages-tests-{}", std::process::id()));
            #[cfg(not(test))]
            let base = crate::storage::data_dir()
                .unwrap_or_else(|_| std::env::temp_dir())
                .join("page-cache");
            let _ = fs::create_dir_all(&base);
            let cleanup_base = base.clone();
            std::thread::spawn(move || cleanup_inactive(&cleanup_base));
            let stamp = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos();
            let root = base.join(format!("session-{}-{stamp}", std::process::id()));
            let _ = fs::create_dir_all(&root);
            let lease = exclusive(&root.join(".lock")).ok();
            Self {
                root,
                state: Mutex::new(State::default()),
                writer: Mutex::new(()),
                lease,
            }
        }
        #[cfg(test)]
        fn at(root: PathBuf) -> Self {
            Self {
                root,
                state: Mutex::new(State::default()),
                writer: Mutex::new(()),
                lease: None,
            }
        }
        pub(crate) fn begin(&self, generation: u64, key: String, seed: Option<Arc<Page>>) {
            let mut state = self.state.lock().unwrap();
            let key = if is_flow_key(&key) {
                key
            } else {
                format!("{}:{key}", state.epoch)
            };
            state.newest.insert(key.clone(), generation);
            state
                .routes
                .retain(|old, _| generation.saturating_sub(*old) < 16);
            state.routes.insert(generation, Route { key, page: seed });
        }
        pub(crate) fn release_inactive_pages(&self, current_generation: u64) {
            let mut state = self.state.lock().unwrap();
            for (&generation, route) in &mut state.routes {
                if generation != current_generation {
                    route.page = None;
                }
            }
        }
        pub(crate) fn resume(&self, generation: u64) -> Option<Arc<Page>> {
            let key = {
                let state = self.state.lock().unwrap();
                let route = state.routes.get(&generation)?;
                if let Some(page) = &route.page {
                    return Some(page.clone());
                }
                route.key.clone()
            };
            let Some(mut page) = self.read(&key) else {
                self.state.lock().unwrap().saved.remove(&key);
                return None;
            };
            let mut state = self.state.lock().unwrap();
            patch(&state, &key, &mut page);
            let route = state.routes.get_mut(&generation)?;
            route.page = Some(page.clone());
            Some(page)
        }
        fn filename(&self, key: &str) -> PathBuf {
            let mut a = std::collections::hash_map::DefaultHasher::new();
            let mut b = std::collections::hash_map::DefaultHasher::new();
            key.hash(&mut a);
            ("page-snapshot", key).hash(&mut b);
            let directory = if is_flow_key(key) {
                self.root.parent().unwrap_or(&self.root).join("flow")
            } else {
                self.root.clone()
            };
            let _ = fs::create_dir_all(&directory);
            directory.join(format!("{:016x}{:016x}.json", a.finish(), b.finish()))
        }
        fn read(&self, key: &str) -> Option<Arc<Page>> {
            let path = self.filename(key);
            if fs::metadata(&path).ok()?.len() > 64 * 1024 * 1024 {
                return None;
            }
            let document: Document = serde_json::from_slice(&fs::read(path).ok()?).ok()?;
            (document.version == 1 && document.key == key).then(|| Arc::new(document.page))
        }
        pub(crate) fn apply(&self, generation: u64, incoming: Arc<Page>) -> Arc<Page> {
            let (key, merged) = {
                let mut state = self.state.lock().unwrap();
                let Some(route) = state.routes.get(&generation) else {
                    return incoming;
                };
                let key = route.key.clone();
                if state.newest.get(&key) != Some(&generation) {
                    return incoming;
                }
                let mut merged = match route.page.as_deref() {
                    Some(previous) => Arc::new(merge(Some(previous), &incoming)),
                    None => incoming,
                };
                patch(&state, &key, &mut merged);
                state.routes.get_mut(&generation).unwrap().page = Some(merged.clone());
                (key, merged)
            };
            let Ok(bytes) = serde_json::to_vec(&StoredDocument::new(&key, &merged)) else {
                return merged;
            };
            let mut hash = std::collections::hash_map::DefaultHasher::new();
            bytes.hash(&mut hash);
            let fingerprint = (hash.finish(), bytes.len());
            let _writer = self.writer.lock().unwrap();
            {
                let state = self.state.lock().unwrap();
                if state.newest.get(&key) != Some(&generation)
                    || state.saved.get(&key) == Some(&fingerprint)
                {
                    return merged;
                }
            }
            let path = self.filename(&key);
            let temporary = path.with_extension("json.tmp");
            if fs::create_dir_all(&self.root).is_ok()
                && fs::write(&temporary, &bytes).is_ok()
                && fs::rename(temporary, path).is_ok()
            {
                self.state.lock().unwrap().saved.insert(key, fingerprint);
            }
            merged
        }
        pub(crate) fn invalidate(&self) {
            let mut state = self.state.lock().unwrap();
            state.epoch = state.epoch.wrapping_add(1);
            state.routes.clear();
            state.newest.clear();
            state.saved.clear();
            state.edits.clear();
        }
    }

    fn patch(state: &State, key: &str, page: &mut Arc<Page>) {
        let Some(route) = super::page_updates::route(key) else {
            return;
        };
        for (account, change) in &state.edits {
            if route[0] == *account && change.affects(key, page) {
                change.apply(key, Arc::make_mut(page));
            }
        }
    }

    fn exclusive(path: &Path) -> std::io::Result<File> {
        use std::os::windows::fs::OpenOptionsExt;
        OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .share_mode(0)
            .open(path)
    }
    fn cleanup_inactive(base: &Path) {
        let Ok(base) = base.canonicalize() else {
            return;
        };
        let Ok(entries) = fs::read_dir(&base) else {
            return;
        };
        for entry in entries.flatten() {
            if !entry.file_name().to_string_lossy().starts_with("session-") {
                continue;
            }
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            if !kind.is_dir() || kind.is_symlink() {
                continue;
            }
            let Ok(resolved) = entry.path().canonicalize() else {
                continue;
            };
            if resolved.parent() != Some(base.as_path()) {
                continue;
            }
            if !resolved.join(".lock").is_file() {
                continue;
            }
            if let Ok(lease) = exclusive(&resolved.join(".lock")) {
                drop(lease);
                let _ = fs::remove_dir_all(resolved);
            }
        }
    }
    impl Drop for Store {
        fn drop(&mut self) {
            let lease = self.lease.take();
            if lease.is_some() {
                drop(lease);
                // root was created as a direct child of this session cache directory.
                if let (Ok(root), Some(parent)) = (self.root.canonicalize(), self.root.parent()) {
                    if parent
                        .canonicalize()
                        .is_ok_and(|base| root.parent() == Some(base.as_path()))
                    {
                        let _ = fs::remove_dir_all(root);
                    }
                }
            }
        }
    }

    fn has_content(page: &Page) -> bool {
        !page.rows.is_empty()
            || page
                .sections
                .iter()
                .any(|section| !section.entries.is_empty())
    }
    pub(super) fn pagination_depth(page: &Page) -> usize {
        page.raw
            .iter()
            .find(|(key, _)| key == "cache:pagination-depth")
            .and_then(|(_, value)| value.as_u64())
            .map(|depth| depth as usize)
            .unwrap_or_else(|| {
                page.raw
                    .iter()
                    .filter(|(key, _)| key.starts_with("next:"))
                    .count()
            })
    }
    fn first_chunk_size(page: &Page) -> Option<usize> {
        page.raw
            .iter()
            .find(|(key, _)| key == "cache:first-chunk-size")
            .and_then(|(_, value)| value.as_u64())
            .or_else(|| {
                page.raw
                    .iter()
                    .find(|(key, _)| key == "cache:chunk")
                    .and_then(|(_, value)| value["start"].as_u64())
            })
            .map(|size| size as usize)
    }

    fn merge_rows(old: &Page, incoming: &Page) -> Vec<Entry> {
        if incoming.rows.is_empty() {
            return old.rows.clone();
        }
        let mut rows = incoming.rows.clone();
        let old_first_end = first_chunk_size(old);
        if let Some(start) = old_first_end {
            let mut seen: std::collections::HashSet<_> = rows
                .iter()
                .map(|row| (row.kind.clone(), row.id.clone()))
                .collect();
            rows.extend(
                old.rows
                    .iter()
                    .skip(start as usize)
                    .filter(|row| seen.insert((row.kind.clone(), row.id.clone())))
                    .cloned(),
            );
        }
        rows
    }
    pub(super) fn merge(cached: Option<&Page>, incoming: &Page) -> Page {
        let Some(old) = cached else {
            return incoming.clone();
        };
        if !incoming.error.is_empty() && has_content(old) {
            let mut page = old.clone();
            page.loading = false;
            page.error = incoming.error.clone();
            return page;
        }
        let mut page = incoming.clone();
        let home = incoming.raw.iter().any(|(key, _)| key.starts_with("home:"));
        for (index, section) in page.sections.iter_mut().enumerate() {
            let pending = home
                && incoming.loading
                && !incoming
                    .raw
                    .iter()
                    .any(|(key, _)| key == &format!("home:{index}"));
            if incoming.loading && incoming.is_discover() {
                if let Some(cached) = old.sections.iter().find(|old| old.title == section.title) {
                    let mut seen: std::collections::HashSet<_> = section
                        .entries
                        .iter()
                        .map(|entry| (entry.kind.clone(), entry.id.clone()))
                        .collect();
                    section.entries.extend(
                        cached
                            .entries
                            .iter()
                            .filter(|entry| seen.insert((entry.kind.clone(), entry.id.clone())))
                            .cloned(),
                    );
                }
            }
            if pending || !section.error.is_empty() {
                if let Some(cached) = old.sections.iter().find(|old| old.title == section.title) {
                    section.entries = cached.entries.clone();
                }
            }
        }
        if incoming.loading {
            for section in &old.sections {
                if !page
                    .sections
                    .iter()
                    .any(|fresh| fresh.title == section.title)
                {
                    page.sections.push(section.clone());
                }
            }
            let mut ordered = Vec::new();
            for previous in &old.sections {
                if ordered
                    .iter()
                    .any(|section: &super::Section| section.title == previous.title)
                {
                    continue;
                }
                ordered.push(
                    page.sections
                        .iter()
                        .find(|fresh| fresh.title == previous.title)
                        .cloned()
                        .unwrap_or_else(|| previous.clone()),
                );
            }
            for fresh in &page.sections {
                if !ordered
                    .iter()
                    .any(|previous: &super::Section| previous.title == fresh.title)
                {
                    ordered.push(fresh.clone());
                }
            }
            page.sections = ordered;
            page.rows = merge_rows(old, incoming);
            let depth = pagination_depth(old).max(pagination_depth(incoming));
            if depth > 0 {
                page.raw.retain(|(key, _)| {
                    key != "cache:pagination-depth" && key != "cache:first-chunk-size"
                });
                page.raw
                    .push(("cache:pagination-depth".into(), serde_json::json!(depth)));
                let size = if incoming.rows.is_empty() {
                    first_chunk_size(old).unwrap_or(old.rows.len())
                } else {
                    first_chunk_size(incoming).unwrap_or(incoming.rows.len())
                };
                page.raw
                    .push(("cache:first-chunk-size".into(), serde_json::json!(size)));
            }
            if page.entity.is_none() {
                page.entity = old.entity.clone();
            }
            if page.title.is_empty() {
                page.title = old.title.clone();
            }
            if page.subtitle.is_empty() {
                page.subtitle = old.subtitle.clone();
            }
        }
        page
    }

    pub(crate) fn same_render(a: &Page, b: &Page) -> bool {
        a.title == b.title
            && a.subtitle == b.subtitle
            && a.sections == b.sections
            && a.rows == b.rows
            && a.entity == b.entity
            && a.next == b.next
            && a.loading == b.loading
            && a.error == b.error
            && a.is_discover() == b.is_discover()
    }

    #[cfg(test)]
    mod tests {
        use super::super::Section;
        use super::*;
        #[test]
        fn stored_document_matches_sanitized_page_format() {
            let page = Page {
                title: "Library".into(),
                subtitle: "Saved music".into(),
                sections: vec![Section {
                    title: "Section".into(),
                    entries: vec![item("section-track")],
                    error: "temporary".into(),
                }],
                rows: vec![item("track")],
                entity: Some(item("entity")),
                raw: vec![
                    ("home:0".into(), serde_json::json!({"pending": true})),
                    ("discover-source:one".into(), serde_json::json!([1, 2])),
                    ("metadata".into(), serde_json::json!({"count": 42})),
                ],
                next: "cursor".into(),
                loading: true,
                error: "temporary".into(),
            };
            let mut expected = page.clone();
            expected.loading = false;
            expected.error.clear();
            expected.sections[0].error.clear();
            expected.raw.retain(|(key, _)| {
                !key.starts_with("home:") && !key.starts_with("discover-source:")
            });
            let old = serde_json::to_vec(&Document {
                version: 1,
                key: "key".into(),
                page: expected,
            })
            .unwrap();
            let new = serde_json::to_vec(&StoredDocument::new("key", &page)).unwrap();
            assert_eq!(new, old);
        }
        fn fixture() -> (PathBuf, Store) {
            let base = std::env::temp_dir().join(format!(
                "deeznats-session-test-{}-{:?}",
                std::process::id(),
                std::thread::current().id()
            ));
            if base.exists() {
                std::fs::remove_dir_all(&base).unwrap();
            }
            (base.clone(), Store::at(base))
        }
        fn item(id: &str) -> Entry {
            Entry {
                id: id.into(),
                title: id.into(),
                kind: "track".into(),
                raw: serde_json::json!({"explicit":true,"contributors":[{"id":"artist","name":"Artist"}]}),
                ..Default::default()
            }
        }
        fn page(id: &str) -> Arc<Page> {
            Arc::new(Page {
                title: "Library".into(),
                rows: vec![item(id)],
                raw: vec![("metadata".into(), serde_json::json!({"count":42}))],
                ..Default::default()
            })
        }
        #[test]
        fn inactive_routes_release_pages_and_reload_from_disk() {
            let (root, store) = fixture();
            store.begin(1, "old".into(), None);
            let old = store.apply(1, page("old"));
            let weak = Arc::downgrade(&old);
            drop(old);
            store.begin(2, "current".into(), None);
            let current = store.apply(2, page("current"));
            store.release_inactive_pages(2);
            assert!(weak.upgrade().is_none());
            assert!(Arc::ptr_eq(&store.resume(2).unwrap(), &current));
            assert_eq!(store.resume(1).unwrap().rows[0].id, "old");
            drop(current);
            drop(store);
            fs::remove_dir_all(root).unwrap();
        }

        #[test]
        fn confirmed_edits_survive_disk_resume_and_late_responses_without_evicting_other_pages() {
            let (root, store) = fixture();
            let key = serde_json::json!(["Deezer:me", "Favorites", "", "", "", ""]).to_string();
            let home = serde_json::json!(["Deezer:me", "Home", "", "", "", ""]).to_string();
            store.begin(1, key.clone(), None);
            store.apply(1, page("removed"));
            store.begin(2, home, None);
            let unrelated = store.apply(2, page("unrelated"));
            store.record_change(
                "Deezer:me",
                super::super::page_updates::Change::Library(item("removed"), false),
            );
            assert!(Arc::ptr_eq(&unrelated, &store.resume(2).unwrap()));
            store.release_inactive_pages(2);
            assert!(store.resume(1).unwrap().rows.is_empty());
            let late = store.apply(1, page("removed"));
            assert!(late.rows.is_empty());
            let mut saved = std::collections::HashSet::from(["removed".into(), "unrelated".into()]);
            store.patch_library("Deezer:me", &mut saved);
            assert!(!saved.contains("removed"));
            assert!(saved.contains("unrelated"));
            store.record_change(
                "Deezer:me",
                super::super::page_updates::Change::Library(item("removed"), true),
            );
            let mut restored = Arc::new(Page::default());
            store.patch_page(&key, &mut restored);
            assert_eq!(restored.rows.len(), 1);
            assert_eq!(store.state.lock().unwrap().edits.len(), 1);
            let other_key =
                serde_json::json!(["Deezer:other", "Favorites", "", "", "", ""]).to_string();
            let mut other = Arc::new(Page::default());
            store.patch_page(&other_key, &mut other);
            assert!(other.rows.is_empty());
            drop(store);
            std::fs::remove_dir_all(root).unwrap();
        }
        #[test]
        fn snapshots_survive_memory_eviction_for_the_whole_session_and_keep_metadata() {
            let (root, store) = fixture();
            for index in 0..50 {
                store.begin(index, format!("user-a/page-{index}"), None);
                store.apply(index, page(&index.to_string()));
            }
            store.begin(100, "user-a/page-0".into(), None);
            let restored = store.resume(100).unwrap();
            assert_eq!(*restored, *page("0"));
            assert_eq!(restored.rows[0].raw["contributors"][0]["id"], "artist");
            store.begin(101, "user-b/page-0".into(), None);
            assert!(store.resume(101).is_none());
            store.invalidate();
            store.begin(102, "user-a/page-0".into(), None);
            assert!(store.resume(102).is_none());
            fs::remove_dir_all(root).unwrap();
        }

        #[test]
        fn successive_playlist_edits_preserve_previous_field_changes_on_old_snapshots() {
            let (root, store) = fixture();
            let key = serde_json::json!(["Deezer:me", "Playlists", "", "", "", ""]).to_string();
            let mut playlist = item("playlist");
            playlist.kind = "playlist".into();
            playlist.raw = serde_json::json!({"description":"Changed"});
            store.record_change(
                "Deezer:me",
                super::super::page_updates::Change::PlaylistDetails(playlist.clone()),
            );
            playlist.title = "Renamed".into();
            playlist.raw = serde_json::json!({});
            store.record_change(
                "Deezer:me",
                super::super::page_updates::Change::PlaylistDetails(playlist),
            );
            let mut old = Arc::new(Page::default());
            store.patch_page(&key, &mut old);
            assert_eq!(old.sections[0].entries[0].title, "Renamed");
            assert_eq!(old.sections[0].entries[0].raw["description"], "Changed");
            drop(store);
            if root.exists() {
                std::fs::remove_dir_all(root).unwrap();
            }
        }
        #[test]
        fn newer_requests_win_and_corrupt_snapshots_are_repaired() {
            let (root, store) = fixture();
            store.begin(1, "page".into(), None);
            store.apply(1, page("old"));
            store.begin(2, "page".into(), None);
            store.apply(2, page("new"));
            store.apply(1, page("obsolete"));
            store.begin(3, "page".into(), None);
            assert_eq!(store.resume(3).unwrap().rows[0].id, "new");
            let path = store.filename("0:page");
            fs::write(&path, b"broken json").unwrap();
            store.begin(4, "page".into(), None);
            assert!(store.resume(4).is_none());
            store.apply(4, page("new"));
            store.begin(5, "page".into(), None);
            assert_eq!(store.resume(5).unwrap().rows[0].id, "new");
            fs::remove_dir_all(root).unwrap();
        }
        #[test]
        fn unchanged_snapshots_do_not_rewrite_the_file() {
            let (root, store) = fixture();
            store.begin(1, "page".into(), None);
            store.apply(1, page("track"));
            let path = store.filename("0:page");
            let file = OpenOptions::new().write(true).open(&path).unwrap();
            let stamp = UNIX_EPOCH + std::time::Duration::from_secs(1000000);
            file.set_modified(stamp).unwrap();
            drop(file);
            let before = fs::metadata(&path).unwrap().modified().unwrap();
            store.apply(1, page("track"));
            assert_eq!(before, fs::metadata(path).unwrap().modified().unwrap());
            fs::remove_dir_all(root).unwrap();
        }
        #[test]
        fn refreshing_chunks_preserves_pending_content_and_removes_deleted_items_at_completion() {
            let old = Page {
                sections: vec![
                    Section {
                        title: "First".into(),
                        entries: vec![item("old")],
                        ..Default::default()
                    },
                    Section {
                        title: "Second".into(),
                        entries: vec![item("keep")],
                        ..Default::default()
                    },
                ],
                ..Default::default()
            };
            let incoming = Page {
                loading: true,
                sections: vec![
                    Section {
                        title: "First".into(),
                        ..Default::default()
                    },
                    Section {
                        title: "Second".into(),
                        ..Default::default()
                    },
                ],
                raw: vec![("home:0".into(), serde_json::Value::Null)],
                ..Default::default()
            };
            let partial = merge(Some(&old), &incoming);
            assert!(partial.sections[0].entries.is_empty());
            assert_eq!(partial.sections[1].entries[0].id, "keep");
            let complete = Page {
                loading: false,
                ..incoming
            };
            assert!(merge(Some(&partial), &complete)
                .sections
                .iter()
                .all(|s| s.entries.is_empty()));
            let failed = Page {
                error: "network unavailable".into(),
                ..Default::default()
            };
            let retained = merge(Some(&old), &failed);
            assert_eq!(retained.sections, old.sections);
            assert_eq!(retained.error, "network unavailable");
        }
        #[test]
        fn ready_sections_do_not_reorder_the_cached_page_during_refresh() {
            let old = Page {
                sections: vec![
                    Section {
                        title: "First".into(),
                        entries: vec![item("a")],
                        ..Default::default()
                    },
                    Section {
                        title: "Second".into(),
                        entries: vec![item("b")],
                        ..Default::default()
                    },
                ],
                ..Default::default()
            };
            let incoming = Page {
                loading: true,
                sections: vec![Section {
                    title: "Second".into(),
                    entries: vec![item("new-b")],
                    ..Default::default()
                }],
                ..Default::default()
            };
            let partial = merge(Some(&old), &incoming);
            assert_eq!(partial.sections[0].title, "First");
            assert_eq!(partial.sections[1].entries[0].id, "new-b");
        }
        #[test]
        fn pagination_refresh_keeps_the_cached_tail_until_updated_chunks_finish() {
            let old = Page {
                rows: vec![item("a"), item("b"), item("c"), item("d")],
                raw: vec![
                    ("next:cursor".into(), serde_json::Value::Null),
                    (
                        "cache:chunk".into(),
                        serde_json::json!({"start":2,"end":4,"cursor":"cursor"}),
                    ),
                ],
                ..Default::default()
            };
            let first = Page {
                rows: vec![item("a"), item("new")],
                loading: true,
                ..Default::default()
            };
            assert_eq!(pagination_depth(&old), 1);
            let partial = merge(Some(&old), &first);
            assert_eq!(pagination_depth(&partial), 1);
            let resumed_refresh = merge(Some(&partial), &first);
            assert_eq!(pagination_depth(&resumed_refresh), 1);
            assert_eq!(resumed_refresh.rows.len(), 4);
            assert_eq!(
                partial
                    .rows
                    .iter()
                    .map(|row| row.id.as_str())
                    .collect::<Vec<_>>(),
                ["a", "new", "c", "d"]
            );
            let complete = Page {
                rows: vec![item("a"), item("new"), item("c")],
                ..Default::default()
            };
            assert_eq!(merge(Some(&partial), &complete).rows.len(), 3);
        }
        #[test]
        fn cleanup_preserves_active_sessions_and_sessions_are_isolated() {
            let (base, _) = fixture();
            fs::create_dir_all(&base).unwrap();
            let root = base.join("session-live");
            fs::create_dir_all(&root).unwrap();
            let lease = exclusive(&root.join(".lock")).unwrap();
            let mut store = Store::at(root.clone());
            store.lease = Some(lease);
            store.begin(1, "user/page".into(), None);
            store.apply(1, page("track"));
            cleanup_inactive(&base);
            assert!(root.is_dir());
            let other = Store::at(base.join("session-other"));
            other.begin(1, "user/page".into(), None);
            assert!(other.resume(1).is_none());
            drop(store);
            assert!(!root.exists());
            fs::remove_dir_all(base).unwrap();
        }
    }

    #[cfg(test)]
    mod flow_persistence_tests {
        use super::*;

        #[test]
        fn flow_survives_store_restart_and_refresh_is_scoped() {
            let base =
                std::env::temp_dir().join(format!("deeznats-flow-test-{}", std::process::id()));
            let make_store = |name: &str| {
                let root = base.join(name);
                fs::create_dir_all(&root).unwrap();
                Store {
                    root,
                    state: Mutex::new(State::default()),
                    writer: Mutex::new(()),
                    lease: None,
                }
            };
            let key = r#"["user-a","Flow","","","","default"]"#;
            let other = r#"["user-b","Flow","","","","default"]"#;
            let first = make_store("first");
            first.begin(1, key.into(), None);
            first.apply(
                1,
                Arc::new(Page {
                    rows: vec![Entry {
                        id: "one".into(),
                        kind: "track".into(),
                        ..Default::default()
                    }],
                    next: "flow:default".into(),
                    ..Default::default()
                }),
            );
            let second = make_store("second");
            second.begin(2, key.into(), None);
            assert_eq!(second.resume(2).unwrap().rows[0].id, "one");
            second.begin(3, other.into(), None);
            assert!(second.resume(3).is_none());
            second.refresh_flow(key);
            let third = make_store("third");
            third.begin(4, key.into(), None);
            assert!(third.resume(4).is_none());
            fs::remove_dir_all(base).unwrap();
        }
    }
}
use crate::{engine::Engine, ui::View};
use serde_json::{json, Value};
use std::{
    collections::{HashMap, HashSet},
    sync::{
        atomic::{AtomicU64, Ordering},
        mpsc, Arc,
    },
};
/// Shared navigation capability; catalog IDs remain provider namespaced.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Provider {
    #[default]
    Deezer,
    Spotify,
    YouTube,
}
static ACTIVE_PROVIDER: AtomicU64 = AtomicU64::new(0);
pub(crate) fn spotify_style_mix(selection: &str) -> Result<Value, String> {
    spotify::style_mix(selection)
}
impl Provider {
    pub fn label(self) -> &'static str {
        match self {
            Self::Deezer => "Deezer",
            Self::Spotify => "Spotify",
            Self::YouTube => "YouTube Music",
        }
    }
    pub fn supports_downloads(self) -> bool {
        self == Self::Deezer
    }
    pub fn active() -> Self {
        match ACTIVE_PROVIDER.load(Ordering::Acquire) {
            1 => Self::Spotify,
            2 => Self::YouTube,
            _ => Self::Deezer,
        }
    }
    pub fn set_active(self) {
        ACTIVE_PROVIDER.store(
            match self {
                Self::Deezer => 0,
                Self::Spotify => 1,
                Self::YouTube => 2,
            },
            Ordering::Release,
        );
    }

    fn invoke(self, engine: &Engine, name: &str, args: &[Value]) -> Result<Value, String> {
        match self {
            Self::Deezer => engine.invoke(name, args),
            Self::Spotify => crate::spotify::catalog_invoke(name, args),
            Self::YouTube => crate::youtube::invoke(name, args),
        }
    }
    fn next(self, engine: &Engine, cursor: &str) -> Result<Value, String> {
        match self {
            Self::Deezer => public(engine, cursor),
            Self::Spotify => self.invoke(engine, "SPCatalogNextJSON", &[json!(cursor)]),
            Self::YouTube => crate::youtube::invoke("Next", &[json!(cursor)]),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Entry {
    pub id: String,
    pub title: String,
    pub subtitle: String,
    pub artwork: String,
    pub kind: String,
    pub duration: u64,
    pub album: String,
    pub artist_id: String,
    pub album_id: String,
    pub raw: Value,
}
impl Entry {
    pub fn library_key(&self) -> String {
        if matches!(self.kind.as_str(), "track" | "episode") {
            self.id.clone()
        } else {
            format!("{}:{}", self.kind, self.id)
        }
    }
    /// Metadata targets shared by clickable credits and native context menus.
    pub fn artist_entries(&self) -> Vec<Self> {
        let mut result = Vec::new();
        let mut seen = HashSet::new();
        for key in ["artists", "contributors", "artist"] {
            for artist in entries(&self.raw[key], "artist") {
                if !artist.id.is_empty()
                    && !artist.title.is_empty()
                    && seen.insert(artist.id.clone())
                {
                    result.push(artist);
                }
            }
        }
        if !self.artist_id.is_empty() && !seen.contains(&self.artist_id) {
            let title = if result.is_empty() {
                self.subtitle.clone()
            } else {
                string(&self.raw["artist"], &["name", "title"])
            };
            if !title.is_empty() {
                result.insert(
                    0,
                    Self {
                        id: self.artist_id.clone(),
                        title,
                        kind: "artist".into(),
                        ..Default::default()
                    },
                );
            }
        }
        result
    }
    pub fn artist_entry(&self) -> Option<Self> {
        self.artist_entries().into_iter().next()
    }
    pub fn album_entry(&self) -> Option<Self> {
        if self.album_id.is_empty() {
            return None;
        }
        let mut raw = self.raw.get("album").cloned().unwrap_or_else(|| json!({}));
        if !raw.is_object() {
            raw = json!({});
        }
        raw["id"] = json!(self.album_id);
        raw["kind"] = json!("album");
        if string(&raw, &["name", "title"]).is_empty() {
            raw["name"] = json!(self.album);
        }
        if picture(&raw).is_empty() && !self.artwork.is_empty() {
            raw["artworkUrl"] = json!(self.artwork);
        }
        raw["artistId"] = json!(self.artist_id);
        raw["artistLine"] = json!(self.subtitle);
        entry(&raw, "album")
    }
    pub fn episode_count(&self) -> Option<u64> {
        [
            "episodeCount",
            "nb_episodes",
            "numberOfEpisodes",
            "episodesCount",
        ]
        .iter()
        .find_map(|key| {
            self.raw
                .get(*key)
                .and_then(|v| v.as_u64().or_else(|| v.as_str()?.parse().ok()))
        })
        .or_else(|| {
            self.raw
                .pointer("/episodes/totalCount")
                .and_then(Value::as_u64)
        })
    }
    pub fn playable(&self) -> bool {
        matches!(self.kind.as_str(), "track" | "episode")
    }
    pub fn wire(&self) -> Value {
        let mut value = self.base_wire();
        if let Some(selection) = self.raw.get(crate::style_mix::MARKER) {
            value[crate::style_mix::MARKER] = selection.clone();
        }
        value
    }
    fn base_wire(&self) -> Value {
        json!({"id":self.id,"name":self.title,"artistLine":self.subtitle,"artistId":self.artist_id,"albumId":self.album_id,"albumName":self.album,"artworkUrl":self.artwork,"durationMs":self.duration,"kind":self.kind,"episodeCount":self.episode_count(),"description":self.raw.get("description").cloned().unwrap_or(Value::Null),"explicit":self.raw["explicit"].as_bool().or(self.raw["isExplicit"].as_bool()).unwrap_or(false),"artists":self.artist_entries().iter().map(|a|json!({"id":a.id,"name":a.title,"kind":"artist"})).collect::<Vec<_>>()})
    }
}
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Section {
    pub title: String,
    pub entries: Vec<Entry>,
    pub error: String,
}
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Page {
    pub title: String,
    pub subtitle: String,
    pub sections: Vec<Section>,
    pub rows: Vec<Entry>,
    pub entity: Option<Entry>,
    pub raw: Vec<(String, Value)>,
    pub next: String,
    pub loading: bool,
    pub error: String,
}
pub fn string(value: &Value, keys: &[&str]) -> String {
    for key in keys {
        if let Some(v) = value.get(*key) {
            if let Some(s) = v.as_str() {
                if !s.is_empty() {
                    return s.into();
                }
            } else if v.is_number() {
                return v.to_string();
            }
        }
    }
    String::new()
}
fn picture(value: &Value) -> String {
    let direct = string(
        value,
        &[
            "artworkUrl",
            "cover_xl",
            "picture_xl",
            "cover_big",
            "picture_big",
            "cover_medium",
            "picture_medium",
            "image",
            "picture",
            "cover",
        ],
    );
    if direct.starts_with("https://") {
        return direct;
    }
    for name in ["mosaicCover", "picture", "cover", "image"] {
        if let Some(urls) = value
            .get(name)
            .and_then(|v| v.get("urls"))
            .and_then(Value::as_array)
        {
            for url in urls {
                if let Some(url) = url.as_str() {
                    if url.starts_with("https://") {
                        return url.into();
                    }
                }
            }
        }
    }
    for item in value.get("image_linked_item").into_iter().chain(
        value
            .get("pictures")
            .and_then(Value::as_array)
            .into_iter()
            .flatten(),
    ) {
        let hash = string(item, &["md5"]);
        let kind = string(item, &["type"]);
        if hash.len() == 32
            && hash.bytes().all(|b| b.is_ascii_hexdigit())
            && matches!(
                kind.as_str(),
                "playlist" | "cover" | "artist" | "talk" | "misc"
            )
        {
            return format!(
                "https://cdn-images.dzcdn.net/images/{kind}/{hash}/320x320-000000-80-0-0.jpg"
            );
        }
    }
    for name in ["album", "podcast"] {
        if let Some(v) = value.get(name) {
            let image = picture(v);
            if !image.is_empty() {
                return image;
            }
        }
    }
    String::new()
}
pub fn entry(value: &Value, fallback: &str) -> Option<Entry> {
    if spotify::is_item(value) {
        return spotify::entry(value, fallback);
    }
    let mut typename = string(value, &["__typename", "type", "kind"]).to_lowercase();
    if typename.is_empty() {
        typename = string(&value["data"], &["__TYPE__"]).to_lowercase();
    }
    if value.get("__typename").is_some()
        && !matches!(
            typename.as_str(),
            "track"
                | "album"
                | "artist"
                | "playlist"
                | "podcast"
                | "episode"
                | "smarttracklist"
                | "flow"
                | "channel"
        )
    {
        return None;
    }
    let mut id = string(value, &["id", "trackId"]);
    if typename == "channel" {
        let target = string(value, &["target"]);
        if target.starts_with("/channels/") {
            id = format!("web:{target}");
        } else {
            let url = string(&value["url"], &["deepLink", "webUrl"]);
            if !url.is_empty() {
                id = url;
            } else if !string(value, &["slug"]).is_empty() {
                id = format!(
                    "deezer://www.deezer.com/channels/{}",
                    string(value, &["slug"])
                );
            }
        }
    }
    if id.is_empty() {
        return None;
    }
    let mut title = string(value, &["title", "displayTitle", "name"]);
    if title.is_empty() {
        title = string(&value["data"], &["title", "name"]);
    }
    if title.is_empty() {
        return None;
    }
    let kind = match typename.as_str() {
        "smarttracklist" | "mix" => "mix",
        "podcast" | "show" => "podcast",
        "episode" => "episode",
        "track" | "song" => "track",
        "album" => "album",
        "artist" => "artist",
        "playlist" => "playlist",
        "flow" => "flow",
        "channel" => "channel",
        _ => fallback,
    }
    .to_string();
    let mut subtitle = match kind.as_str() {
        "playlist" => {
            let mut owner = String::new();
            for key in ["owner", "creator", "user"] {
                owner = value
                    .get(key)
                    .and_then(Value::as_str)
                    .map(str::to_owned)
                    .unwrap_or_else(|| string(&value[key], &["name", "displayName", "title"]));
                if !owner.trim().is_empty() {
                    break;
                }
            }
            if owner.trim().is_empty() {
                string(value, &["artistLine", "ownerName", "creatorName"])
            } else {
                owner
            }
        }
        "podcast" => string(value, &["authorName", "author", "publisher", "artistLine"]),
        "track" | "episode" | "album" => string(value, &["artistLine", "podcastName", "artist"]),
        _ => string(
            value,
            &[
                "artistLine",
                "subTitle",
                "subtitle",
                "owner",
                "podcastName",
                "artist",
            ],
        ),
    };
    if kind == "artist" && subtitle.is_empty() {
        let fans = string(value, &["nb_fan", "fans", "followers"]);
        if !fans.is_empty() && fans != "0" {
            if let Ok(n) = fans.parse::<u64>() {
                subtitle = format!("{} followers", format_count(n));
            } else {
                subtitle = format!("{fans} followers");
            }
        }
    }
    if subtitle.is_empty() {
        for key in ["artist", "owner", "user", "creator"] {
            if let Some(artist) = value.get(key) {
                subtitle = string(artist, &["name", "title"]);
                if !subtitle.is_empty() {
                    break;
                }
            }
        }
    }
    if subtitle.is_empty() {
        let mut artists = Vec::new();
        if let Some(contributors) = value.get("contributors") {
            for artist in entries(contributors, "artist") {
                artists.push(artist.title);
            }
        }
        if let Some(items) = value.get("artists").and_then(Value::as_array) {
            artists.extend(items.iter().map(|v| string(v, &["name"])));
        }
        subtitle = artists.join(", ");
    }
    title = title.split_whitespace().collect::<Vec<_>>().join(" ");
    subtitle = subtitle.split_whitespace().collect::<Vec<_>>().join(" ");
    if kind == "episode" && subtitle.is_empty() {
        subtitle = string(&value["podcast"], &["displayTitle", "title", "name"]);
    }
    let duration = value
        .get("durationMs")
        .and_then(Value::as_u64)
        .unwrap_or_else(|| {
            let duration = value.get("duration").and_then(Value::as_u64).unwrap_or(0);
            if duration >= 10000
                && (value.get("contributors").is_some() || value.get("__typename").is_some())
            {
                duration
            } else {
                duration * 1000
            }
        });
    let album = value.get("album").cloned().unwrap_or(Value::Null);
    Some(Entry {
        id,
        title,
        subtitle,
        artwork: picture(value),
        kind,
        duration,
        album: {
            let a = string(value, &["albumName"]);
            if a.is_empty() {
                album
                    .as_str()
                    .map(str::to_owned)
                    .unwrap_or_else(|| string(&album, &["title", "displayTitle", "name"]))
            } else {
                a
            }
        },
        artist_id: {
            let a = string(value, &["artistId"]);
            if a.is_empty() {
                let id = string(&value["artist"], &["id"]);
                if id.is_empty() {
                    value
                        .get("contributors")
                        .and_then(|v| entries(v, "artist").into_iter().next())
                        .map(|e| e.id)
                        .unwrap_or_default()
                } else {
                    id
                }
            } else {
                a
            }
        },
        album_id: {
            let a = string(value, &["albumId"]);
            if a.is_empty() {
                string(&album, &["id"])
            } else {
                a
            }
        },
        raw: compact_entry_metadata(value),
    })
}
// Bulk collection rows already have their own Entry records. Retain all display
// and artist metadata, but not a second copy of those entire collections.
fn compact_entry_metadata(value: &Value) -> Value {
    let Some(object) = value.as_object() else {
        return value.clone();
    };
    let mut result = serde_json::Map::new();
    for (key, value) in object {
        if matches!(
            key.as_str(),
            "tracks" | "episodes" | "albums" | "playlists" | "sections"
        ) {
            if let Some(metadata) = value.as_object() {
                let kept: serde_json::Map<String, Value> = metadata
                    .iter()
                    .filter(|(key, _)| {
                        !matches!(key.as_str(), "data" | "items" | "edges" | "nodes")
                    })
                    .map(|(key, value)| (key.clone(), value.clone()))
                    .collect();
                if !kept.is_empty() {
                    result.insert(key.clone(), Value::Object(kept));
                }
            }
        } else {
            result.insert(key.clone(), value.clone());
        }
    }
    Value::Object(result)
}
pub fn format_count(n: u64) -> String {
    let s = n.to_string();
    let mut result = String::new();
    let len = s.len();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (len - i) % 3 == 0 {
            result.push('.');
        }
        result.push(c);
    }
    result
}

pub fn entries(value: &Value, fallback: &str) -> Vec<Entry> {
    fn visit(
        v: &Value,
        kind: &str,
        out: &mut Vec<Entry>,
        seen: &mut HashSet<String>,
        depth: usize,
    ) {
        if depth > 32 || out.len() >= 2000 {
            return;
        }
        if let Some(items) = v.as_array() {
            for item in items {
                visit(item, kind, out, seen, depth + 1);
            }
            return;
        }
        if let Some(object) = v.as_object() {
            if let Some(item) = entry(v, kind) {
                if seen.insert(format!("{}:{}", item.kind, item.id)) {
                    out.push(item);
                }
                return;
            }
            for (key, child) in object {
                let inferred = match key.as_str() {
                    "tracks" | "topTracks" | "hotTracks" => "track",
                    "albums" | "topAlbums" | "newReleases" => "album",
                    "artists" | "recommendedArtists" => "artist",
                    "playlists" | "recentlyPlayed" | "madeForMe" | "artistPlaylists" => "playlist",
                    "episodes" => "episode",
                    "podcasts" | "suggestedPodcasts" => "podcast",
                    _ => kind,
                };
                if !matches!(
                    key.as_str(),
                    "pageInfo" | "errors" | "extensions" | "viewer" | "user"
                ) {
                    visit(child, inferred, out, seen, depth + 1);
                }
            }
        }
    }
    let mut result = Vec::new();
    visit(value, fallback, &mut result, &mut HashSet::new(), 0);
    result
}
// Collection metadata also has an id/title. Resolve its track connection before
// passing anything to the generic catalog walker, which otherwise stops there.
fn table_payload<'a>(value: &'a Value) -> Option<&'a Value> {
    for key in ["tracks", "episodes"] {
        if let Some(rows) = value.get(key) {
            return Some(rows);
        }
    }
    for key in ["smartTracklist", "playlist", "album"] {
        if let Some(collection) = value.get(key) {
            return Some(collection.get("tracks").unwrap_or(&Value::Null));
        }
    }
    value
        .get("data")
        .filter(|v| v.is_object())
        .and_then(table_payload)
}
pub(crate) fn table_entries(value: &Value, kind: &str) -> Vec<Entry> {
    entries(table_payload(value).unwrap_or(value), kind)
        .into_iter()
        .filter(Entry::playable)
        .collect()
}

// Collection responses may wrap metadata separately from their track connection.
fn inherit_collection_metadata(page: &mut Page, value: &Value) {
    let Some(entity) = page.entity.as_mut() else {
        return;
    };
    if !matches!(entity.kind.as_str(), "album" | "playlist") {
        return;
    }
    let metadata = value
        .get(&entity.kind)
        .or_else(|| value.get("data").and_then(|data| data.get(&entity.kind)))
        .unwrap_or(value);
    if let Some(description) = metadata
        .get("description")
        .or_else(|| metadata.get("DESCRIPTION"))
        .and_then(Value::as_str)
    {
        if !entity.raw.is_object() {
            entity.raw = json!({});
        }
        entity.raw["description"] = json!(description);
    }
    for key in [
        "owner",
        "creator",
        "user",
        "creator_id",
        "owner_id",
        "USER_ID",
    ] {
        if let Some(value) = metadata.get(key) {
            if !entity.raw.is_object() {
                entity.raw = json!({});
            }
            entity.raw[key] = value.clone();
        }
    }
    let artwork = picture(metadata);
    if !artwork.is_empty() {
        entity.artwork = artwork;
    }
    for row in &mut page.rows {
        if row.kind != "track" {
            continue;
        }
        if row.artwork.is_empty() {
            row.artwork = entity.artwork.clone();
        }
        if row.album.is_empty() {
            row.album = entity.title.clone();
        }
        if row.album_id.is_empty() {
            row.album_id = entity.id.clone();
        }
    }
}

fn curated_entries(value: &Value, fallback: &str) -> Vec<Entry> {
    entries(value, fallback)
        .into_iter()
        .filter(|item| matches!(item.kind.as_str(), "playlist" | "album" | "mix" | "podcast"))
        .collect()
}

fn web_sections(value: &Value, directory: bool) -> Vec<Section> {
    value["sections"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|row| {
            let items = if directory {
                entries(&row["items"], "channel")
                    .into_iter()
                    .filter(|item| item.kind == "channel")
                    .collect()
            } else {
                curated_entries(&row["items"], "playlist")
            };
            (!items.is_empty()).then(|| Section {
                title: string(row, &["title"]),
                entries: items,
                ..Default::default()
            })
        })
        .collect()
}

impl Page {
    pub fn is_catalogue(&self) -> bool {
        self.is_discover()
            || self
                .entity
                .as_ref()
                .is_some_and(|item| item.kind == "channel")
            || self.raw.iter().any(|(key, _)| key == "catalogue")
    }
    pub fn is_discover(&self) -> bool {
        self.raw.iter().any(|(key, _)| key == "discover")
    }
}

pub fn discover_section_matches(section: &Section, filter: usize) -> bool {
    match filter {
        1 => section
            .entries
            .iter()
            .any(|item| matches!(item.kind.as_str(), "playlist" | "mix")),
        2 => section.title == "Genres",
        3 => section.title == "Categories",
        4 => section.title == "Podcasts by category",
        _ => true,
    }
}

fn discover(
    engine: &Engine,
    mut page: Page,
    publish: &mut dyn FnMut(&Page) -> Result<(), String>,
) -> Result<Page, String> {
    page.raw.push(("discover".into(), json!(true)));
    publish(&page)?;
    let user = engine.invoke("DZUserID", &[]).unwrap_or(Value::Null);
    let logged_in = user.as_u64().is_some_and(|id| id > 0)
        || user
            .as_str()
            .is_some_and(|id| id.parse::<u64>().is_ok_and(|id| id > 0));
    // Each source is independent. Slow personalized modules must not delay the
    // public genres and web categories that are already available.
    let mut jobs = Vec::new();
    if logged_in {
        for (module, title) in [
            (3, "Recommended playlists"),
            (1, "Made for you"),
            (6, "100% playlists from your artists"),
            (9, "Recommended for offline listening"),
        ] {
            jobs.push((0, module, "", title, "playlist"));
        }
    }
    for (path, title, kind) in [
        (
            "chart/0/playlists?limit=8",
            "Editorial playlists",
            "playlist",
        ),
        ("editorial/0/releases?limit=8", "New releases", "album"),
        (
            "editorial/0/selection?limit=8",
            "Editors' album picks",
            "album",
        ),
        ("editorial?limit=8", "Genres", "channel"),
    ] {
        jobs.push((1, 0, path, title, kind));
    }
    for (path, title) in [
        ("/channels/explore", "Categories"),
        ("/channels/podcasts", "Podcasts by category"),
    ] {
        jobs.push((2, 0, path, title, "channel"));
    }
    let personal_count = if logged_in { 4 } else { 0 };
    let mut order = Vec::new();
    for index in 0..4 {
        if index < personal_count {
            order.push(index);
        }
        order.push(personal_count + index);
        if index < 2 {
            order.push(personal_count + 4 + index);
        }
    }
    let mut results = vec![None; jobs.len()];
    let cursors = std::sync::Mutex::new(HashMap::new());
    parallel::load(
        jobs.len(),
        |slot| {
            let index = order[slot];
            let (source, module, path, title, kind) = jobs[index];
            let response = match source {
                0 => {
                    deezer_shelves::api(engine, "DZHomeModuleJSON", &[json!(module)]).map(|value| {
                        vec![Section {
                            title: title.into(),
                            entries: home_module_entries(&value, module as usize, kind)
                                .into_iter()
                                .filter(|item| matches!(item.kind.as_str(), "playlist" | "mix"))
                                .collect(),
                            ..Default::default()
                        }]
                    })
                }
                1 => deezer_shelves::public(engine, path).map(|value| {
                    cursors.lock().unwrap().insert(index, value.clone());
                    let mut rows = entries(&value, kind);
                    if kind == "channel" {
                        rows.retain(|item| item.id != "0");
                        for item in &mut rows {
                            item.id = format!("editorial:{}", item.id);
                        }
                    }
                    vec![Section {
                        title: title.into(),
                        entries: rows,
                        ..Default::default()
                    }]
                }),
                _ => deezer_shelves::web(engine, &format!("https://www.deezer.com/en{path}")).map(
                    |value| {
                        web_sections(&value, true)
                            .into_iter()
                            .map(|mut section| {
                                let genre = title == "Categories"
                                    && ["genre", "t\u{fc}r", "tarz"]
                                        .iter()
                                        .any(|word| section.title.to_lowercase().contains(word));
                                section.title = if genre { "Genres" } else { title }.into();
                                section
                            })
                            .collect()
                    },
                ),
            };
            (
                index,
                response.unwrap_or_else(|error| {
                    vec![Section {
                        title: title.into(),
                        error,
                        ..Default::default()
                    }]
                }),
            )
        },
        |_, (index, sections)| {
            if let Some(value) = cursors.lock().unwrap().remove(&index) {
                deezer_shelves::remember_public(&mut page, jobs[index].3, &value, jobs[index].4);
            }
            results[index] = Some(sections);
            page.sections.clear();
            for sections in results.iter().flatten() {
                for section in sections {
                    if let Some(existing) =
                        page.sections.iter_mut().find(|s| s.title == section.title)
                    {
                        for item in &section.entries {
                            if let Some(old) = existing.entries.iter_mut().find(|e| {
                                e.id == item.id || e.title.eq_ignore_ascii_case(&item.title)
                            }) {
                                if old.artwork.is_empty() {
                                    old.artwork = item.artwork.clone();
                                }
                            } else {
                                existing.entries.push(item.clone());
                            }
                        }
                    } else {
                        page.sections.push(section.clone());
                    }
                }
            }
            page.raw
                .push((format!("discover-source:{index}"), Value::Null));
            deezer_shelves::cover_directories(&mut page.sections, "");
            publish(&page)
        },
    )?;
    Ok(page)
}
fn artist_section_entries(value: &Value, index: usize) -> Vec<Entry> {
    let artist = value
        .pointer("/data/artist")
        .or_else(|| value.get("artist"));
    let connection = artist.and_then(|artist| {
        if index == 5 {
            artist.pointer("/playlists/items")
        } else {
            artist.get("items")
        }
    });
    let kind = match index {
        0 => "track",
        4 => "artist",
        5 => "playlist",
        _ => "album",
    };
    entries(connection.unwrap_or(&Value::Null), kind)
        .into_iter()
        .filter(|item| item.kind == kind)
        .collect()
}
fn update_artist_metadata(entity: &mut Entry, metadata: &Value) {
    let artwork = picture(metadata);
    if !artwork.is_empty() {
        entity.artwork = artwork;
    }
    let name = string(metadata, &["name", "displayTitle", "title"]);
    if !name.is_empty() {
        entity.title = name;
    }
    let fans = string(
        metadata,
        &["fansCount", "nbFans", "nb_fan", "fans", "followers"],
    );
    entity.kind = "artist".into();
    entity.raw = compact_entry_metadata(&metadata);
    if !fans.is_empty() {
        entity.raw["fans"] = json!(fans);
        entity.subtitle = if fans == "0" {
            String::new()
        } else {
            format!(
                "{} followers",
                fans.parse::<u64>().map(format_count).unwrap_or(fans)
            )
        };
    }
}
pub(crate) fn home_module_entries(value: &Value, index: usize, kind: &str) -> Vec<Entry> {
    let mut items: Vec<_> = entries(value, kind)
        .into_iter()
        .filter(|entry| !is_liked_songs(entry) && (index != 1 || entry.kind != "flow"))
        .collect();
    if kind == "album" {
        for item in &mut items {
            if item.kind == "track"
                && item.duration == 0
                && item.album_id.is_empty()
                && string(&item.raw, &["__typename", "type", "kind"]).is_empty()
            {
                item.kind = "album".into();
            }
        }
    }
    normalize_daily_mixes(&mut items);
    items
}
fn normalize_daily_mixes(items: &mut [Entry]) {
    let mut daily = 0;
    for item in items {
        if item.title.trim().eq_ignore_ascii_case("daily") {
            daily += 1;
            item.title = format!("Daily Mix {daily}");
            item.kind = "mix".into();
            item.subtitle.clear();
        } else if item.kind == "mix" {
            item.subtitle.clear();
        }
    }
}

fn is_liked_songs(entry: &Entry) -> bool {
    entry.kind == "playlist"
        && [
            "sevdi\u{011f}in \u{015f}ark\u{0131}lar",
            "sevdi\u{011f}in par\u{00e7}alar",
            "sevilen \u{015f}ark\u{0131}lar",
            "favori par\u{00e7}alar",
            "liked songs",
            "favorite tracks",
            "favourite tracks",
            "coups de c\u{0153}ur",
            "coups de coeur",
        ]
        .contains(&entry.title.trim().to_lowercase().as_str())
}
pub fn web_state(bytes: &[u8]) -> Result<Value, String> {
    let html = std::str::from_utf8(bytes).map_err(|e| e.to_string())?;
    let marker = "window.__DZR_APP_STATE__";
    let start = html
        .find(marker)
        .ok_or("Deezer web page contains no catalog state")?
        + marker.len();
    let remaining = &html[start..];
    let start = remaining.find('=').ok_or("Invalid Deezer web state")? + 1;
    let mut stream =
        serde_json::Deserializer::from_str(remaining[start..].trim_start()).into_iter::<Value>();
    stream
        .next()
        .ok_or("Empty Deezer web state")?
        .map_err(|e| e.to_string())
}
fn public_cache() -> &'static cache::Cache {
    static CACHE: std::sync::OnceLock<cache::Cache> = std::sync::OnceLock::new();
    CACHE.get_or_init(|| cache::Cache::new(32 * 1024 * 1024))
}
pub(crate) fn expire_public_cache() {
    public_cache().expire();
}
fn web_catalog(engine: &Engine, url: &str) -> Result<Arc<Value>, String> {
    public_cache().refresh(
        url,
        std::time::Duration::from_secs(120),
        || engine.fetch(url),
        web_state,
    )
}
pub(crate) fn public(engine: &Engine, path: &str) -> Result<Value, String> {
    let url = if path.starts_with("https://api.deezer.com/") {
        path.to_string()
    } else {
        format!("https://api.deezer.com/{}", path.trim_start_matches('/'))
    };
    let value = public_cache().refresh(
        &url,
        std::time::Duration::from_secs(120),
        || engine.fetch(&url),
        decode_public_json,
    )?;
    Ok((*value).clone())
}
pub(crate) fn public_json(
    url: &str,
    fetch: impl FnOnce() -> Result<Vec<u8>, String>,
) -> Result<Arc<Value>, String> {
    public_cache().load(
        url,
        std::time::Duration::from_secs(120),
        fetch,
        decode_public_json,
    )
}
fn decode_public_json(bytes: &[u8]) -> Result<Value, String> {
    let value: Value = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
    if let Some(error) = value.get("error") {
        return Err(format!("Deezer: {}", string(error, &["message", "type"])));
    }
    Ok(value)
}

pub(super) fn flow_request(entity: Option<&Entry>, query: &str) -> (&'static str, Vec<Value>) {
    if let Some(track) = entity.filter(|entry| entry.kind == "track") {
        ("DZTrackMixJSON", vec![json!(track.id)])
    } else if query.is_empty() || query == "default" {
        ("DZFlowJSON", Vec::new())
    } else {
        ("DZFlowForConfigJSON", vec![json!(query)])
    }
}

/// Load the full provider library for the inline playlist submenu, independent
/// of route/page-cache residency. Repeated cursors terminate malformed paging.
fn playlist_access_cache() -> &'static std::sync::Mutex<HashMap<String, Value>> {
    static CACHE: std::sync::OnceLock<std::sync::Mutex<HashMap<String, Value>>> =
        std::sync::OnceLock::new();
    CACHE.get_or_init(Default::default)
}

pub(crate) fn restore_playlist_owner(item: &mut Entry, account: &Value) {
    if item.kind != "playlist" {
        return;
    }
    let key = format!("{}:{}", string(account, &["userId"]), item.id);
    if let Some(access) = playlist_access_cache().lock().unwrap().get(&key) {
        item.raw["owner"] = access["owner"].clone();
    }
}

fn hydrate_deezer_playlists(engine: &Engine, value: Value) -> Result<Value, String> {
    let account = engine.invoke("DZAccountJSON", &[]).unwrap_or(Value::Null);
    let user = string(&account, &["userId"]);
    hydrate_deezer_playlists_with(value, &user, |id| {
        engine.invoke("DZPlaylistAccessJSON", &[json!(id)])
    })
}

fn hydrate_deezer_playlists_with(
    value: Value,
    user: &str,
    fetch: impl Fn(&str) -> Result<Value, String> + Sync,
) -> Result<Value, String> {
    let mut lists = entries(&value, "playlist");
    let mut updates = Vec::new();
    parallel::load(
        lists.len(),
        |index| fetch(&lists[index].id),
        |index, result| {
            let result = result?;
            let access = result
                .pointer("/data/playlist")
                .or_else(|| result.get("playlist"))
                .filter(|v| v.is_object())
                .ok_or("Deezer did not return playlist ownership and library status")?;
            updates.push((index, access.clone()));
            Ok(())
        },
    )?;
    for (index, access) in updates {
        let item = &mut lists[index];
        item.raw["owner"] = access["owner"].clone();
        item.raw["isFavorite"] = access["isFavorite"].clone();
        playlist_access_cache()
            .lock()
            .unwrap()
            .insert(format!("{user}:{}", item.id), access);
    }
    Ok(
        json!({"playlists": lists.into_iter().map(|item| item.raw).collect::<Vec<_>>(), "next": value["next"]}),
    )
}

pub fn editable_menu_playlists(
    engine: Option<&Engine>,
    provider: Provider,
    account: &Value,
) -> Result<Vec<Entry>, String> {
    if provider == Provider::YouTube {
        return crate::youtube::menu_playlists();
    }
    collect_editable_menu_playlists(account, |cursor| {
        let (name, args) = match cursor {
            None => ("DZPlaylistsJSON", Vec::new()),
            Some(cursor) => ("SPCatalogNextJSON", vec![json!(cursor)]),
        };
        match provider {
            Provider::Deezer => {
                let engine = engine.ok_or("Connect to Deezer to load playlists")?;
                match cursor {
                    None => engine
                        .invoke(name, &args)
                        .and_then(|value| hydrate_deezer_playlists(engine, value)),
                    Some(cursor) => public(engine, cursor),
                }
            }
            Provider::Spotify => crate::spotify::catalog_invoke(name, &args),
            Provider::YouTube => {
                crate::youtube::invoke(if cursor.is_some() { "Next" } else { name }, &args)
            }
        }
    })
}

fn collect_editable_menu_playlists(
    account: &Value,
    mut fetch: impl FnMut(Option<&str>) -> Result<Value, String>,
) -> Result<Vec<Entry>, String> {
    let mut value = fetch(None)?;
    let mut result = Vec::new();
    let mut ids = HashSet::new();
    let mut cursors = HashSet::new();
    loop {
        for entry in entries(&value, "playlist") {
            if crate::playlist::can_edit(&entry, account) && ids.insert(entry.id.clone()) {
                result.push(entry);
            }
        }
        let next = string(&value, &["next"]);
        if next.is_empty() || !cursors.insert(next.clone()) {
            break;
        }
        value = fetch(Some(&next))?;
    }
    Ok(result)
}

fn saved_library_entries(
    engine: Option<&Engine>,
    provider: Provider,
) -> Result<Vec<Entry>, String> {
    if provider == Provider::YouTube {
        return crate::youtube::saved_library_entries();
    }
    let mut saved = Vec::new();
    let mut last_error = None;
    for (name, kind) in [
        ("DZFavoritesJSON", "track"),
        ("DZFavoriteAlbumsJSON", "album"),
        ("DZPlaylistsJSON", "playlist"),
    ] {
        let mut cursor = None;
        let mut cursors = HashSet::new();
        loop {
            let result = match (provider, cursor.as_deref()) {
                (Provider::Deezer, None) => engine
                    .ok_or_else(|| "Connect to Deezer first".to_string())
                    .and_then(|engine| engine.invoke(name, &[])),
                (Provider::Deezer, Some(cursor)) => engine
                    .ok_or_else(|| "Connect to Deezer first".to_string())
                    .and_then(|engine| public(engine, cursor)),
                (Provider::Spotify, None) => crate::spotify::catalog_invoke(name, &[]),
                (Provider::Spotify, Some(cursor)) => {
                    crate::spotify::catalog_invoke("SPCatalogNextJSON", &[json!(cursor)])
                }
                _ => unreachable!(),
            };
            match result {
                Ok(value) => {
                    let value = if provider == Provider::Deezer && kind == "playlist" {
                        hydrate_deezer_playlists(engine.unwrap(), value)?
                    } else {
                        value
                    };
                    saved.extend(
                        entries(&value, kind)
                            .into_iter()
                            .filter(|item| item.raw["isFavorite"] != false),
                    );
                    let next = string(&value, &["next"]);
                    if next.is_empty() || !cursors.insert(next.clone()) {
                        break;
                    }
                    cursor = Some(next);
                }
                Err(error) => {
                    last_error = Some(error);
                    break;
                }
            }
        }
    }
    if saved.is_empty() && last_error.is_some() {
        Err(last_error.unwrap())
    } else {
        Ok(saved)
    }
}

#[cfg(test)]
mod menu_playlist_tests {
    use super::*;
    #[test]
    fn playlist_copy_updates_only_verified_tracks_and_rejects_incomplete_success() {
        let tracks = entries(
            &json!({"tracks":[{"id":"1","title":"One"},{"id":"2","title":"Two"}]}),
            "track",
        );
        let mut applied = Vec::new();
        let result =
            confirm_playlist_copy(&tracks, &json!({"confirmed":["1"],"failure":""}), |track| {
                applied.push(track.id.clone())
            });
        assert_eq!(applied, vec!["1"]);
        assert!(result.unwrap_err().contains("1/2"));
        applied.clear();
        let result = confirm_playlist_copy(
            &tracks,
            &json!({"confirmed":["2"],"failure":"Rate limited"}),
            |track| applied.push(track.id.clone()),
        );
        assert_eq!(applied, vec!["2"]);
        assert!(result.unwrap_err().contains("Rate limited"));
        assert!(confirm_playlist_copy(
            &tracks,
            &json!({"confirmed":["1","2"],"failure":""}),
            |_| {}
        )
        .is_ok());
    }
    #[test]
    fn collection_copy_follows_nested_track_pages_past_sixty_tracks() {
        let tracks = |start: usize, end: usize| {
            (start..end)
                .map(|id| json!({"id":id.to_string(),"title":format!("Track {id}"),"type":"track"}))
                .collect::<Vec<_>>()
        };
        let first = json!({"playlist":{"id":"source","title":"Source","tracks":{"data":tracks(0,60),"next":"page2"}}});
        let mut calls = Vec::new();
        let copied = collect_collection_tracks(first, |cursor| {
            calls.push(cursor.to_owned());
            Ok(match cursor {
                "page2" => {
                    json!({"data":{"playlist":{"tracks":{"data":tracks(60,120),"next":"page3"}}}})
                }
                "page3" => json!({"data":tracks(120,127),"next":null}),
                _ => panic!("unexpected page"),
            })
        })
        .unwrap();
        assert_eq!(calls, vec!["page2", "page3"]);
        assert_eq!(copied.len(), 127);
        assert_eq!(
            copied
                .iter()
                .map(|track| track.id.clone())
                .collect::<Vec<_>>(),
            (0..127).map(|id| id.to_string()).collect::<Vec<_>>()
        );
    }

    #[test]
    fn collection_copy_does_not_accept_failed_or_repeated_pages_as_a_complete_list() {
        let first = json!({"tracks":{"data":[{"id":"1","title":"One"}],"next":"page2"}});
        let failed = collect_collection_tracks(first.clone(), |_| Err("Page failed".into()));
        assert_eq!(failed.unwrap_err(), "Page failed");
        let repeated = collect_collection_tracks(first.clone(), |_| Ok(first.clone()));
        assert!(repeated.unwrap_err().contains("repeated a playlist page"));
    }

    #[test]
    fn collection_copy_preserves_top_level_pagination_and_track_order() {
        let first = json!({"tracks":[{"id":"1","title":"One"}],"next":"page2"});
        let copied = collect_collection_tracks(first, |cursor| {
            assert_eq!(cursor, "page2");
            Ok(json!({"tracks":[{"id":"2","title":"Two"}],"next":null}))
        })
        .unwrap();
        assert_eq!(
            copied
                .iter()
                .map(|track| track.id.as_str())
                .collect::<Vec<_>>(),
            vec!["1", "2"]
        );
    }
    #[test]
    fn native_playlist_access_restores_ownership_and_membership() {
        let value = json!({"playlists":[
            {"id":"owned-menu-fixture", "name":"Mine", "owner":""},
            {"id":"foreign-menu-fixture", "name":"Saved", "owner":""}
        ]});
        let value = hydrate_deezer_playlists_with(value, "menu-fixture-user", |id| {
            Ok(json!({"data":{"playlist":{
                "id":id, "owner":{"id":if id == "owned-menu-fixture" {"menu-fixture-user"} else {"someone-else"}},
                "isFavorite":id == "foreign-menu-fixture"
            }}}))
        }).unwrap();
        let account = json!({"userId":"menu-fixture-user"});
        let lists = collect_editable_menu_playlists(&account, |_| Ok(value.clone())).unwrap();
        assert_eq!(lists.len(), 1);
        assert_eq!(lists[0].id, "owned-menu-fixture");
        let saved: Vec<_> = entries(&value, "playlist")
            .into_iter()
            .filter(|item| item.raw["isFavorite"] != false)
            .collect();
        assert_eq!(saved.len(), 1);
        assert_eq!(saved[0].id, "foreign-menu-fixture");
        let mut card = entry(
            &json!({"id":"owned-menu-fixture", "title":"Mine"}),
            "playlist",
        )
        .unwrap();
        restore_playlist_owner(&mut card, &account);
        assert!(crate::playlist::can_edit(&card, &account));
        let mut other_account_card = entry(
            &json!({"id":"owned-menu-fixture", "title":"Mine"}),
            "playlist",
        )
        .unwrap();
        restore_playlist_owner(&mut other_account_card, &json!({"userId":"other-account"}));
        assert!(!crate::playlist::can_edit(
            &other_account_card,
            &json!({"userId":"other-account"})
        ));
    }
    #[test]
    fn playlist_access_failure_is_reported_instead_of_an_empty_editable_list() {
        let result = hydrate_deezer_playlists_with(
            json!({"playlists":[{"id":"1","name":"Mine"}]}),
            "me",
            |_| Err("Access request failed".into()),
        );
        assert_eq!(result.unwrap_err(), "Access request failed");
    }

    #[test]
    fn song_radio_uses_the_track_seed_instead_of_personal_flow() {
        for id in ["42", "spotify:track:seed", "youtube:track:seed"] {
            let track = Entry {
                id: id.into(),
                kind: "track".into(),
                ..Default::default()
            };
            let (name, args) = flow_request(Some(&track), "default");
            assert_eq!(name, "DZTrackMixJSON");
            assert_eq!(args, vec![json!(id)]);
        }
        assert_eq!(flow_request(None, "default"), ("DZFlowJSON", Vec::new()));
        assert_eq!(
            flow_request(None, "focus"),
            ("DZFlowForConfigJSON", vec![json!("focus")])
        );
    }

    #[test]
    fn saved_collection_membership_does_not_collide_with_track_ids() {
        let item = |kind: &str| Entry {
            id: "42".into(),
            kind: kind.into(),
            ..Default::default()
        };
        let mut saved = HashSet::new();
        saved.insert(item("album").library_key());
        assert!(saved.contains(&item("album").library_key()));
        assert!(!saved.contains(&item("track").library_key()));
        assert!(!saved.contains(&item("playlist").library_key()));
    }

    #[test]
    fn selected_tracks_are_loaded_without_a_network_collection_lookup() {
        let entries: Vec<Entry> = ["42", "43", "44"]
            .iter()
            .map(|id| Entry {
                id: (*id).into(),
                kind: "track".into(),
                title: (*id).into(),
                ..Default::default()
            })
            .collect();
        let selection = Entry {
            kind: "selection".into(),
            raw: json!({"entries":entries}),
            ..Default::default()
        };
        for provider in [Provider::Deezer, Provider::Spotify, Provider::YouTube] {
            assert_eq!(
                collection_tracks(None, provider, &selection)
                    .unwrap()
                    .iter()
                    .map(|entry| entry.id.as_str())
                    .collect::<Vec<_>>(),
                vec!["42", "43", "44"]
            );
        }
    }

    #[test]
    fn loads_all_pages_deduplicates_and_excludes_other_owners() {
        let mut calls = 0;
        let lists = collect_editable_menu_playlists(&json!({"id":"me"}), |cursor| {
            calls += 1;
            let (start, end, next) = match cursor {
                None => (0, 75, "page2"),
                Some("page2") => (75, 150, "page3"),
                Some("page3") => (150, 153, "page3"),
                _ => panic!("unexpected cursor"),
            };
            let mut data: Vec<Value> = (start..end)
                .map(|id| {
                    json!({
                        "id":id.to_string(), "title":format!("List {id}"), "creator":{"id":"me"}
                    })
                })
                .collect();
            data.push(json!({"id":"foreign", "title":"Someone else's", "creator":{"id":"other"}}));
            data.push(json!({"id":"0", "title":"Duplicate", "creator":{"id":"me"}}));
            Ok(json!({"data":data,"next":next}))
        })
        .unwrap();
        assert_eq!(calls, 3);
        assert_eq!(lists.len(), 153);
        assert_eq!(lists.last().unwrap().id, "152");
    }

    #[test]
    fn reports_pagination_failure_instead_of_claiming_a_complete_library() {
        let result = collect_editable_menu_playlists(&json!({"id":"me"}), |cursor| {
            if cursor.is_none() {
                Ok(json!({"data":[],"next":"page2"}))
            } else {
                Err("Library request failed".into())
            }
        });
        assert_eq!(result.unwrap_err(), "Library request failed");
    }
}

#[derive(Clone)]
pub enum CollectionAction {
    Queue,
    Play,
    Download,
    Add(String),
    CreateDetails(String, String, Option<Vec<u8>>, Option<bool>),
    Remove(String),
    Favorites,
}

fn collection_tracks(
    engine: Option<&Engine>,
    provider: Provider,
    item: &Entry,
) -> Result<Vec<Entry>, String> {
    if item.kind == "selection" {
        return serde_json::from_value(item.raw["entries"].clone())
            .map_err(|error| error.to_string());
    }
    if provider == Provider::YouTube {
        return crate::youtube::collection_tracks(item);
    }
    let name = if item.kind == "mix" && provider == Provider::Deezer {
        "DZSmartMixJSON"
    } else if item.kind == "artist" {
        "DZArtistTopJSON"
    } else if item.kind == "album" {
        "DZAlbumTracksJSON"
    } else if provider == Provider::Deezer && item.kind == "playlist" {
        "DZPlaylistTracksStrictJSON"
    } else {
        "DZPlaylistTracksJSON"
    };
    let invoke = |name: &str, args: &[Value]| match provider {
        Provider::Deezer => engine
            .ok_or_else(|| "Connect to Deezer first".to_string())?
            .invoke(name, args),
        Provider::Spotify => crate::spotify::catalog_invoke(name, args),
        Provider::YouTube => crate::youtube::invoke(name, args),
    };
    let value = invoke(name, &[json!(item.id)])?;
    collect_collection_tracks(value, |next| match provider {
        Provider::Deezer => public(engine.ok_or("Connect to Deezer first")?, next),
        Provider::Spotify => invoke("SPCatalogNextJSON", &[json!(next)]),
        Provider::YouTube => invoke("Next", &[json!(next)]),
    })
}

fn collect_collection_tracks(
    mut value: Value,
    mut fetch_next: impl FnMut(&str) -> Result<Value, String>,
) -> Result<Vec<Entry>, String> {
    let mut tracks = Vec::new();
    let mut cursors = HashSet::new();
    loop {
        tracks.extend(
            table_entries(&value, "track")
                .into_iter()
                .filter(|e| e.playable()),
        );
        let mut next = string(&value, &["next"]);
        if next.is_empty() {
            next = table_payload(&value)
                .map(|tracks| string(tracks, &["next"]))
                .unwrap_or_default();
        }
        if next.is_empty() {
            break;
        }
        if !cursors.insert(next.clone()) {
            return Err(
                "The service repeated a playlist page; the collection was not copied".into(),
            );
        }
        value = fetch_next(&next)?;
    }
    if tracks.is_empty() {
        return Err("This collection has no playable tracks".into());
    }
    Ok(tracks)
}

fn process_collection(
    generation: u64,
    provider: Provider,
    engine: Option<&Engine>,
    item: Entry,
    action: CollectionAction,
    tx: &EventSender,
) {
    let tracks = collection_tracks(engine, provider, &item).map(|mut tracks| {
        if !item.title.trim().is_empty() { for track in &mut tracks { track.raw["queue_context_name"] = json!(item.title); } }
        tracks
    });
    if matches!(action, CollectionAction::Favorites) {
        let mut completed = Vec::new();
        let mut error = None;
        match tracks {
            Ok(tracks) => {
                for track in tracks {
                    let args = [json!(track.id)];
                    let result = match provider {
                        Provider::Deezer => engine
                            .ok_or_else(|| "Connect to Deezer first".to_string())
                            .and_then(|engine| engine.invoke("DZAddFavorite", &args)),
                        Provider::Spotify => crate::spotify::catalog_invoke("DZAddFavorite", &args),
                        Provider::YouTube => crate::youtube::invoke("DZAddFavorite", &args),
                    };
                    match result {
                        Ok(value) if value != false && value != 0 && !value.is_null() => {
                            completed.push(track)
                        }
                        Ok(_) => {
                            error = Some("The service rejected the favorites update".into());
                            break;
                        }
                        Err(message) => {
                            error = Some(message);
                            break;
                        }
                    }
                }
            }
            Err(message) => error = Some(message),
        }
        let _ = tx.send(Event::BatchFavorites(provider, completed, error));
        return;
    }
    if matches!(action, CollectionAction::Play) {
        let _ = tx.send(Event::CollectionPlay(provider, tracks));
        return;
    }
    if matches!(action, CollectionAction::Queue) {
        let _ = tx.send(Event::CollectionQueued(provider, tracks));
        return;
    }
    if matches!(action, CollectionAction::Download) {
        let _ = tx.send(Event::CollectionDownload(provider, item, tracks));
        return;
    }
    let mut mutation = match action {
        CollectionAction::Remove(id) => crate::playlist::Mutation::Remove(id, String::new()),
        CollectionAction::CreateDetails(title, description, cover, public) => {
            crate::playlist::Mutation::Save {
                id: String::new(),
                title,
                description: Some(description),
                cover,
                seed: None,
                public,
            }
        }
        CollectionAction::Add(id) => crate::playlist::Mutation::Add(id, String::new()),
        CollectionAction::Queue
        | CollectionAction::Play
        | CollectionAction::Download
        | CollectionAction::Favorites => unreachable!(),
    };
    let execute = |mutation: &mut crate::playlist::Mutation| match provider {
        Provider::Deezer => {
            crate::playlist::execute(engine.ok_or("Connect to Deezer first")?, mutation)
        }
        Provider::Spotify => crate::playlist::execute_spotify(mutation),
        Provider::YouTube => crate::youtube::edit_playlist(mutation),
    };
    let result = (|| {
        let tracks = tracks?;
        let id = match &mutation {
            crate::playlist::Mutation::Add(id, _) | crate::playlist::Mutation::Remove(id, _) => {
                id.clone()
            }
            _ => execute(&mut mutation)?.id,
        };
        if provider == Provider::Deezer
            && !matches!(mutation, crate::playlist::Mutation::Remove(..))
        {
            let ids: Vec<_> = tracks.iter().map(|track| track.id.as_str()).collect();
            let result = engine.ok_or("Connect to Deezer first")?.invoke(
                "DZCopyPlaylistTracksJSON",
                &[
                    json!(id),
                    json!(serde_json::to_string(&ids).map_err(|error| error.to_string())?),
                ],
            )?;
            confirm_playlist_copy(&tracks, &result, |track| {
                let _ = tx.send(Event::PlaylistTrackChanged(
                    provider,
                    id.clone(),
                    track.clone(),
                    true,
                ));
            })?;
            return Ok(crate::playlist::Outcome { id, deleted: false });
        }
        for (index, track) in tracks.iter().enumerate() {
            let removing = matches!(mutation, crate::playlist::Mutation::Remove(..));
            let mut change = if removing {
                crate::playlist::Mutation::Remove(id.clone(), track.id.clone())
            } else {
                crate::playlist::Mutation::Add(id.clone(), track.id.clone())
            };
            execute(&mut change)
                .map_err(|error| format!("Updated {index}/{} tracks: {error}", tracks.len()))?;
            let _ = tx.send(Event::PlaylistTrackChanged(
                provider,
                id.clone(),
                track.clone(),
                !removing,
            ));
        }
        Ok(crate::playlist::Outcome { id, deleted: false })
    })();
    let _ = tx.send(Event::Playlist(generation, provider, mutation, result));
}

fn confirm_playlist_copy(
    tracks: &[Entry],
    result: &Value,
    mut confirmed: impl FnMut(&Entry),
) -> Result<(), String> {
    let ids: HashSet<_> = result["confirmed"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect();
    let mut copied = 0;
    for track in tracks {
        if ids.contains(track.id.as_str()) {
            confirmed(track);
            copied += 1;
        }
    }
    let failure = string(result, &["failure"]);
    if !failure.is_empty() {
        return Err(format!(
            "Confirmed {copied}/{} tracks in destination: {failure}",
            tracks.len()
        ));
    }
    if copied != tracks.len() {
        return Err(format!(
            "Confirmed {copied}/{} tracks in destination; copy is incomplete",
            tracks.len()
        ));
    }
    Ok(())
}

#[derive(Clone)]
pub enum Job {
    LibraryState,
    Collection(Entry, CollectionAction),
    Route(View, Option<Entry>, String),
    More(Page),
    Api(String, Vec<Value>),
    Playlist(crate::playlist::Mutation),
    Lyrics(Entry, String, u32),
    LyricsWindow(Entry, Vec<Entry>, u32),
    TrackInfo(Entry),
}
pub struct Request {
    #[cfg(feature = "diagnostics")]
    pub view_tag: usize,
    pub generation: u64,
    pub provider: Provider,
    pub engine: Option<Arc<Engine>>,
    pub job: Job,
}
pub enum Event {
    PlaylistTrackChanged(Provider, String, Entry, bool),
    LibraryState(Provider, Result<Vec<Entry>, String>),
    BatchFavorites(Provider, Vec<Entry>, Option<String>),
    CollectionPlay(Provider, Result<Vec<Entry>, String>),
    CollectionDownload(Provider, Entry, Result<Vec<Entry>, String>),
    CollectionQueued(Provider, Result<Vec<Entry>, String>),
    Playlist(
        u64,
        Provider,
        crate::playlist::Mutation,
        Result<crate::playlist::Outcome, String>,
    ),
    Page(u64, Arc<Page>),
    Api(u64, Provider, String, Result<Value, String>),
    Lyrics(
        bool, // All enabled providers have completed.
        Provider,
        String,
        u32,
        Result<(String, Vec<crate::lyrics::LyricLine>), String>,
    ),
    TrackInfo(u64, Box<Entry>, Value, Value),
}
pub struct Worker {
    pub requests: mpsc::Sender<Request>,
    pub lyrics_requests: mpsc::SyncSender<Request>,
    pub updates: mpsc::Receiver<Event>,
    route_generation: Arc<AtomicU64>,
    pub session: Arc<session::Store>,
}
#[derive(Clone)]
pub(crate) struct EventSender {
    tx: mpsc::SyncSender<Event>,
    session: Arc<session::Store>,
}
impl EventSender {
    pub(crate) fn send(&self, event: Event) -> Result<(), mpsc::SendError<Event>> {
        let event = match event {
            Event::Page(generation, page) => {
                Event::Page(generation, self.session.apply(generation, page))
            }
            other => other,
        };
        self.tx.send(event)
    }
    fn resume(&self, generation: u64) -> Option<Arc<Page>> {
        let page = self.session.resume(generation)?;
        let mut displayed = (*page).clone();
        displayed.loading = true;
        displayed.error.clear();
        let _ = self.tx.send(Event::Page(generation, Arc::new(displayed)));
        Some(page)
    }
}
impl Worker {
    /// API browsing does not depend on a native Deezer engine or Soloist playback.
    pub fn request_spotify(&self, generation: u64, job: Job) -> Result<(), String> {
        #[cfg(feature = "diagnostics")]
        let view_tag = match &job {
            Job::Route(view, ..) => *view as usize + 1,
            _ => 0,
        };
        self.requests
            .send(Request {
                #[cfg(feature = "diagnostics")]
                view_tag,
                generation,
                provider: Provider::Spotify,
                engine: None,
                job,
            })
            .map_err(|_| "Catalog worker stopped".into())
    }
    pub fn set_generation(&self, generation: u64) {
        self.route_generation.store(generation, Ordering::Relaxed);
    }
    pub fn start() -> Self {
        // Navigation must never be dropped because the catalog worker is busy.
        // Obsolete routes are discarded before doing any network work.
        let (requests, rx) = mpsc::channel::<Request>();
        let (tx, updates) = mpsc::sync_channel(4);
        let session = Arc::new(session::Store::for_session());
        let tx = EventSender {
            tx,
            session: session.clone(),
        };
        let (lyrics_tx, lyrics_rx) = mpsc::sync_channel::<Request>(16);
        let lyrics_events = tx.clone();
        std::thread::spawn(move || {
            let mut pending = None;
            loop {
                let mut request = match pending.take().or_else(|| lyrics_rx.recv().ok()) {
                    Some(r) => r,
                    None => break,
                };
                while let Ok(newer) = lyrics_rx.try_recv() {
                    request = newer;
                }
                #[cfg(feature = "diagnostics")]
                let _page_scope = crate::memory_profile::page_scope(if request.view_tag == 0 {
                    0
                } else {
                    request.view_tag + 64
                });
                let (track, id, sources, neighbors) = match request.job {
                    Job::Lyrics(track, id, sources) => (track, id, sources, Vec::new()),
                    Job::LyricsWindow(track, neighbors, sources) => {
                        let id = track.id.clone();
                        (track, id, sources, neighbors)
                    }
                    _ => continue,
                };
                let result = crate::lyrics::fetch_progressive(
                    if request.provider == Provider::Deezer {
                        request.engine.as_deref()
                    } else {
                        None
                    },
                    &track,
                    &id,
                    sources,
                    |answer| {
                        let _ = lyrics_events.send(Event::Lyrics(
                            false,
                            request.provider,
                            id.clone(),
                            sources,
                            Ok(answer),
                        ));
                    },
                );
                if lyrics_events
                    .send(Event::Lyrics(true, request.provider, id, sources, result))
                    .is_err()
                {
                    break;
                }
                // One adjacent lookup at a time, only while no current-song request is waiting.
                for neighbor in neighbors {
                    if let Ok(newer) = lyrics_rx.try_recv() {
                        pending = Some(newer);
                        break;
                    }
                    if neighbor.kind == "track"
                        && !neighbor.id.is_empty()
                        && neighbor.id != track.id
                    {
                        if request.provider != Provider::Deezer {
                            let _ = crate::lyrics::fetch_spotify_lyrics(
                                &neighbor,
                                &neighbor.id,
                                sources,
                            );
                        } else if let Some(engine) = request.engine.as_deref() {
                            let _ = crate::lyrics::fetch_best_lyrics(
                                engine,
                                &neighbor,
                                &neighbor.id,
                                sources,
                            );
                        }
                    }
                }
            }
        });
        let lyrics_forward = lyrics_tx.clone();
        let route_generation = Arc::new(AtomicU64::new(0));
        let history_generation = route_generation.clone();
        let worker_generation = route_generation.clone();
        let (history_tx, history_rx) = mpsc::sync_channel::<(u64, Arc<Engine>, Page, usize)>(1);
        let history_events = tx.clone();
        std::thread::spawn(move || {
            let mut recent_metadata = HashMap::<(String, String), Entry>::new();
            while let Ok(mut request) = history_rx.recv() {
                while let Ok(newer) = history_rx.try_recv() {
                    request = newer;
                }
                let (generation, engine, mut page, _view_tag) = request;
                #[cfg(feature = "diagnostics")]
                let _page_scope = crate::memory_profile::page_scope(_view_tag + 64);
                if history_generation.load(Ordering::Relaxed) != generation {
                    continue;
                }
                for row in &mut page.rows {
                    if let Some(metadata) = recent_metadata.get(&(row.kind.clone(), row.id.clone()))
                    {
                        enrich_history_entry(row, metadata);
                    }
                }
                let _ = history_events.send(Event::Page(generation, Arc::new(page.clone())));
                let mut dirty = false;
                for index in 0..page.rows.len() {
                    if history_generation.load(Ordering::Relaxed) != generation {
                        break;
                    }
                    let row = &mut page.rows[index];
                    if !row.artwork.is_empty() {
                        continue;
                    }
                    let key = (row.kind.clone(), row.id.clone());
                    let metadata = if row.kind == "episode" {
                        public(&engine, &format!("episode/{}", row.id))
                    } else {
                        engine.invoke("DZTrackMetadataJSON", &[json!(row.id)])
                    };
                    if let Ok(value) = metadata {
                        if let Some(metadata) = entry(&value, &row.kind) {
                            if metadata.id == row.id && !metadata.artwork.is_empty() {
                                enrich_history_entry(row, &metadata);
                                if recent_metadata.len() >= 256 {
                                    if let Some(old) = recent_metadata.keys().next().cloned() {
                                        recent_metadata.remove(&old);
                                    }
                                }
                                recent_metadata.insert(key, metadata);
                                dirty = true;
                            }
                        }
                    }
                    if dirty
                        && (index == 0 || index % 8 == 7)
                        && history_generation.load(Ordering::Relaxed) == generation
                    {
                        if history_events
                            .send(Event::Page(generation, Arc::new(page.clone())))
                            .is_err()
                        {
                            return;
                        }
                        dirty = false;
                    }
                }
                if dirty && history_generation.load(Ordering::Relaxed) == generation {
                    let _ = history_events.send(Event::Page(generation, Arc::new(page)));
                }
            }
        });
        let (navigation_tx, navigation_rx) = mpsc::channel();
        let (action_tx, action_rx) = mpsc::channel();
        std::thread::spawn(move || {
            while let Ok(request) = rx.recv() {
                let target = if matches!(
                    request.job,
                    Job::Route(..) | Job::More(..) | Job::TrackInfo(..)
                ) || matches!(&request.job, Job::Api(name,_) if name == "DeezerStyleMixGenres" || name == "SpotifyStyleMixGenres")
                {
                    &navigation_tx
                } else {
                    &action_tx
                };
                if target.send(request).is_err() {
                    break;
                }
            }
        });
        let navigation_rx = Arc::new(std::sync::Mutex::new(navigation_rx));
        let action_rx = Arc::new(std::sync::Mutex::new(action_rx));
        for rx in [action_rx, navigation_rx.clone(), navigation_rx] {
            let tx = tx.clone();
            let worker_generation = worker_generation.clone();
            let history_tx = history_tx.clone();
            let lyrics_forward = lyrics_forward.clone();
            std::thread::spawn(move || {
                loop {
                    // Drop the receiver lock before doing network work.
                    let request = { rx.lock().unwrap().recv() };
                    let Ok(request) = request else {
                        break;
                    };

                    let Request {
                        #[cfg(feature = "diagnostics")]
                        view_tag,
                        generation,
                        engine,
                        provider,
                        job,
                    } = request;
                    let operation = match &job {
                        Job::LibraryState => "LibraryState", Job::Collection(..) => "Collection",
                        Job::Route(..) => "Route", Job::More(..) => "More", Job::Api(..) => "Api",
                        Job::Playlist(..) => "Playlist", Job::Lyrics(..) => "Lyrics",
                        Job::LyricsWindow(..) => "LyricsWindow", Job::TrackInfo(..) => "TrackInfo",
                    };
                    let _request_log = crate::spotify_diagnostics::Span::new("catalog.job", &format!("{:?} {operation} generation={generation}", provider));

                    #[cfg(feature = "diagnostics")]
                    let _page_scope = crate::memory_profile::page_scope(if view_tag == 0 {
                        0
                    } else {
                        view_tag + 64
                    });
                    if matches!(job, Job::Route(..) | Job::More(..) | Job::TrackInfo(..))
                        && worker_generation.load(Ordering::Relaxed) != generation
                    {
                        continue;
                    }
                    if matches!(job, Job::Lyrics(..) | Job::LyricsWindow(..)) {
                        let _ = lyrics_forward.try_send(Request {
                            #[cfg(feature = "diagnostics")]
                            view_tag,
                            generation,
                            provider,
                            engine,
                            job,
                        });
                        continue;
                    }
                    if matches!(job, Job::LibraryState) {
                        let result = saved_library_entries(engine.as_deref(), provider);
                        let _ = tx.send(Event::LibraryState(provider, result));
                        continue;
                    }
                    if let Job::Collection(item, action) = job {
                        process_collection(
                            generation,
                            provider,
                            engine.as_deref(),
                            item,
                            action,
                            &tx,
                        );
                        continue;
                    }
                    if provider == Provider::YouTube {
                        crate::youtube::process(generation, job, &tx, &worker_generation);
                        continue;
                    }
                    if provider == Provider::Spotify {
                        spotify::process(generation, job, &tx, &worker_generation);
                        continue;
                    }
                    let Some(engine) = engine else {
                        continue;
                    };
                    let cached = if matches!(job, Job::Route(..)) {
                        tx.resume(generation)
                    } else {
                        None
                    };
                    match job {
                        Job::LibraryState | Job::Collection(..) => {
                            unreachable!("collections are handled before provider dispatch")
                        }
                        Job::Route(View::Home, _, _) => {
                            let shelves = [
                                ("Recently played", "DZHomeModuleJSON", 0, "playlist"),
                                ("Made for you", "DZHomeModuleJSON", 1, "playlist"),
                                ("Playlists you love", "DZHomeModuleJSON", 2, "playlist"),
                                ("Recommended playlists", "DZHomeModuleJSON", 3, "playlist"),
                                ("Recommended artists", "DZHomeModuleJSON", 4, "artist"),
                                ("Recommended tracks", "DZHomeModuleJSON", 5, "track"),
                                (
                                    "100% playlists from your artists",
                                    "DZHomeModuleJSON",
                                    6,
                                    "playlist",
                                ),
                                ("New releases for you", "DZHomeModuleJSON", 7, "album"),
                                (
                                    "Rediscover your favorites",
                                    "DZFavoriteAlbumsJSON",
                                    -1,
                                    "album",
                                ),
                                ("More albums for you", "DZHomeModuleJSON", 8, "album"),
                                ("More for you", "DZHomeModuleJSON", 9, "playlist"),
                            ];
                            let mut page = Page {
                                title: "For you".into(),
                                subtitle: format!(
                                    "Personalized recommendations from {}",
                                    provider.label()
                                ),
                                loading: true,
                                sections: shelves
                                    .iter()
                                    .map(|(title, _, _, _)| Section {
                                        title: (*title).into(),
                                        ..Default::default()
                                    })
                                    .collect(),
                                ..Default::default()
                            };
                            let mut completed = 0;
                            let _ = parallel::load(
                                shelves.len(),
                                |index| {
                                    let (title, name, module, kind) = shelves[index];
                                    let mut section = Section {
                                        title: title.into(),
                                        ..Default::default()
                                    };
                                    if worker_generation.load(Ordering::Relaxed) != generation {
                                        return section;
                                    }
                                    let args = if module < 0 {
                                        vec![]
                                    } else {
                                        vec![json!(module)]
                                    };
                                    match provider.invoke(&engine, name, &args) {
                                        Ok(value) => {
                                            section.entries = if module >= 0
                                                && provider == Provider::Deezer
                                            {
                                                home_module_entries(&value, module as usize, kind)
                                            } else {
                                                let mut rows = entries(&value, kind);
                                                normalize_daily_mixes(&mut rows);
                                                rows
                                            };
                                        }
                                        Err(error) => section.error = error,
                                    }
                                    section
                                },
                                |index, section| {
                                    if worker_generation.load(Ordering::Relaxed) != generation {
                                        return Err("Superseded load".into());
                                    }
                                    page.sections[index] = section;
                                    page.raw.push((format!("home:{index}"), Value::Null));
                                    completed += 1;
                                    page.loading = completed < shelves.len();
                                    tx.send(Event::Page(generation, Arc::new(page.clone())))
                                        .map_err(|_| "Window closed".into())
                                },
                            );
                        }
                        Job::Playlist(mut mutation) => {
                            let result = if provider == Provider::Spotify {
                                crate::playlist::execute_spotify(&mut mutation)
                            } else {
                                crate::playlist::execute(&engine, &mut mutation)
                            };
                            if tx
                                .send(Event::Playlist(generation, provider, mutation, result))
                                .is_err()
                            {
                                return;
                            }
                        }
                        Job::Api(name, args) => {
                            let result = if matches!(
                                name.as_str(),
                                "DeezerStyleMixGenres" | "SpotifyStyleMixGenres"
                            ) {
                                args.first()
                                    .and_then(Value::as_u64)
                                    .ok_or_else(|| "Choose a valid language".to_string())
                                    .and_then(|region| {
                                        if provider == Provider::Spotify {
                                            provider.invoke(&engine, "StyleMixGenres", &args)
                                        } else {
                                            crate::style_mix::available_genres_while(
                                                &engine,
                                                region as usize,
                                                || {
                                                    worker_generation.load(Ordering::Relaxed)
                                                        == generation
                                                },
                                            )
                                        }
                                    })
                            } else if provider == Provider::Spotify {
                                provider.invoke(&engine, &name, &args)
                            } else if name == "RustPodcastLibraryJSON" {
                                podcast_library(&engine, &args)
                            } else if name == "StyleMixGenres" {
                                args.first()
                                    .and_then(Value::as_u64)
                                    .ok_or_else(|| "Choose a valid language".to_string())
                                    .and_then(|region| {
                                        crate::style_mix::available_genres_while(
                                            &engine,
                                            region as usize,
                                            || {
                                                worker_generation.load(Ordering::Relaxed)
                                                    == generation
                                            },
                                        )
                                    })
                            } else {
                                engine.invoke(&name, &args)
                            };
                            if tx
                                .send(Event::Api(generation, provider, name, result))
                                .is_err()
                            {
                                return;
                            }
                        }
                        job @ (Job::Lyrics(..) | Job::LyricsWindow(..)) => {
                            let _ = lyrics_forward.try_send(Request {
                                #[cfg(feature = "diagnostics")]
                                view_tag,
                                generation,
                                provider,
                                engine: Some(engine),
                                job,
                            });
                        }
                        Job::TrackInfo(track) => {
                            let meta = provider
                                .invoke(&engine, "DZTrackMetadataJSON", &[json!(track.id)])
                                .or_else(|error| {
                                    if provider == Provider::Deezer {
                                        public(&engine, &format!("track/{}", track.id))
                                    } else {
                                        Err(error)
                                    }
                                })
                                .unwrap_or_else(|_| track.raw.clone());
                            let artist_id = entry(&meta, "track")
                                .map(|full| full.artist_id)
                                .filter(|id| !id.is_empty())
                                .unwrap_or_else(|| {
                                    if track.artist_id.is_empty() {
                                        string(&meta["artist"], &["id"])
                                    } else {
                                        track.artist_id.clone()
                                    }
                                });
                            let artist_meta = if !artist_id.is_empty() {
                                if provider == Provider::Spotify {
                                    provider
                                        .invoke(
                                            &engine,
                                            "SPCatalogItemJSON",
                                            &[json!("artist"), json!(artist_id)],
                                        )
                                        .unwrap_or(Value::Null)
                                } else {
                                    public(&engine, &format!("artist/{artist_id}"))
                                        .unwrap_or(Value::Null)
                                }
                            } else {
                                Value::Null
                            };
                            let meta = musicbrainz::enrich(&track, meta);
                            if tx
                                .send(Event::TrackInfo(
                                    generation,
                                    Box::new(track),
                                    meta,
                                    artist_meta,
                                ))
                                .is_err()
                            {
                                return;
                            }
                        }
                        Job::More(mut page) if page.next.starts_with("flow:") => {
                            let config = page.next.trim_start_matches("flow:").to_string();
                            let result = if config.is_empty() || config == "default" {
                                provider.invoke(&engine, "DZFlowJSON", &[])
                            } else {
                                provider.invoke(&engine, "DZFlowForConfigJSON", &[json!(config)])
                            };
                            match result {
                                Ok(value) => {
                                    append_flow_rows(&mut page, table_entries(&value, "track"));
                                    page.error.clear();
                                }
                                Err(error) => page.error = error,
                            }
                            page.loading = false;
                            let _ = tx.send(Event::Page(generation, Arc::new(page)));
                        }
                        Job::More(mut page) => {
                            if let Some(index) = page
                                .raw
                                .iter()
                                .position(|(key, _)| key == "spotify:more-shelf")
                            {
                                let (_, request) = page.raw.remove(index);
                                let title = request["title"].as_str().unwrap_or("");
                                let target = request["target"].as_u64().unwrap_or(0) as usize;
                                if let Err(error) =
                                    deezer_shelves::more(&engine, &mut page, title, target)
                                {
                                    page.error = error;
                                }
                                page.loading = false;
                                let _ = tx.send(Event::Page(generation, Arc::new(page)));
                                continue;
                            }
                            let cursor = page.next.clone();
                            let result = if cursor.is_empty() {
                                Err("No more results are available".into())
                            } else {
                                provider.next(&engine, &cursor)
                            };
                            match result {
                                Ok(value) => {
                                    append_page(&mut page, &cursor, value);
                                }
                                Err(error) => page.error = error,
                            }
                            page.loading = false;
                            let _ = tx.send(Event::Page(generation, Arc::new(page)));
                        }
                        Job::Route(view, entity, query) => {
                            if (view == View::Album || view == View::Artist || view == View::Show)
                                && entity.is_none()
                            {
                                if let Some(page) = cached.clone() {
                                    let mut page = (*page).clone();
                                    page.loading = false;
                                    let _ = tx.send(Event::Page(generation, Arc::new(page)));
                                    continue;
                                }
                            }
                            if view == View::Flow {
                                if let Some(page) =
                                    cached.clone().filter(|page| !page.rows.is_empty())
                                {
                                    let mut page = (*page).clone();
                                    page.loading = false;
                                    let _ = tx.send(Event::Page(generation, Arc::new(page)));
                                    continue;
                                }
                            }
                            let mut publish = |page: &Page| {
                                if worker_generation.load(Ordering::Relaxed) != generation {
                                    return Err("Superseded load".into());
                                }
                                let mut partial = page.clone();
                                partial.loading = true;
                                tx.send(Event::Page(generation, Arc::new(partial)))
                                    .map_err(|_| "Window closed".to_string())
                            };
                            let result = if provider == Provider::Spotify {
                                spotify::load(&engine, view, entity, query, &mut publish)
                            } else {
                                load(&engine, view, entity, query, &mut publish)
                            };
                            if worker_generation.load(Ordering::Relaxed) != generation {
                                continue;
                            }
                            let page = match result {
                                Ok(mut page) => {
                                    let depth = cached
                                        .as_deref()
                                        .map(session::pagination_depth)
                                        .unwrap_or(0);
                                    for _ in 0..depth {
                                        if page.next.is_empty()
                                            || worker_generation.load(Ordering::Relaxed)
                                                != generation
                                        {
                                            break;
                                        }
                                        let cursor = page.next.clone();
                                        match provider.next(&engine, &cursor) {
                                            Ok(value) => append_page(&mut page, &cursor, value),
                                            Err(error) => {
                                                page.error = error;
                                                break;
                                            }
                                        }
                                        let mut partial = page.clone();
                                        partial.loading = true;
                                        let _ = tx.send(Event::Page(generation, Arc::new(partial)));
                                    }
                                    page
                                }
                                Err(error) => Page {
                                    title: view.title().into(),
                                    error,
                                    ..Default::default()
                                },
                            };
                            if tx
                                .send(Event::Page(generation, Arc::new(page.clone())))
                                .is_err()
                            {
                                return;
                            }
                            if view == View::Recent && provider == Provider::Deezer {
                                // Cover metadata must never occupy the page/API worker.
                                let _ = history_tx.try_send((
                                    generation,
                                    engine,
                                    page,
                                    view as usize + 1,
                                ));
                            }
                        }
                    }
                }
            });
        }
        Self {
            requests,
            updates,
            lyrics_requests: lyrics_tx,
            route_generation,
            session,
        }
    }
}

// Retain the recorded title/kind and history fields while filling replay metadata.
fn enrich_history_entry(row: &mut Entry, metadata: &Entry) {
    if row.artwork.is_empty() {
        row.artwork = metadata.artwork.clone();
    }
    if row.subtitle.is_empty() {
        row.subtitle = metadata.subtitle.clone();
    }
    if row.album.is_empty() {
        row.album = metadata.album.clone();
    }
    if row.duration == 0 {
        row.duration = metadata.duration;
    }
    if row.artist_id.is_empty() {
        row.artist_id = metadata.artist_id.clone();
    }
    if row.album_id.is_empty() {
        row.album_id = metadata.album_id.clone();
    }
    if let (Some(recorded), Some(details)) = (row.raw.as_object_mut(), metadata.raw.as_object()) {
        for (key, value) in details {
            recorded.entry(key.clone()).or_insert_with(|| value.clone());
        }
    }
}

fn append_page(page: &mut Page, cursor: &str, value: Value) {
    let start = page.rows.len();
    let depth = session::pagination_depth(page);
    let mut seen: HashSet<_> = page
        .rows
        .iter()
        .map(|item| (item.kind.clone(), item.id.clone()))
        .collect();
    page.rows.extend(
        table_entries(&value, "track")
            .into_iter()
            .filter(|item| seen.insert((item.kind.clone(), item.id.clone()))),
    );
    inherit_collection_metadata(page, &value);
    let next = string(&value, &["next"]);
    // Stop cyclic cursors instead of fetching the same page indefinitely.
    page.next = if next == cursor
        || page
            .raw
            .iter()
            .any(|(key, _)| key == &format!("next:{next}"))
    {
        String::new()
    } else {
        next
    };
    page.error.clear();
    page.raw.push((format!("next:{cursor}"), Value::Null));
    page.raw.push((
        "cache:chunk".into(),
        json!({"cursor":cursor,"start":start,"end":page.rows.len()}),
    ));
    if let Some((_, value)) = page
        .raw
        .iter_mut()
        .find(|(key, _)| key == "cache:pagination-depth")
    {
        *value = json!(depth + 1);
    }
}
fn dynamic_sections(
    engine: &Engine,
    uri: &str,
    page: &mut Page,
    publish: &mut dyn FnMut(&Page) -> Result<(), String>,
) -> Result<(), String> {
    publish(page)?;
    let value = deezer_shelves::api(engine, "DZDiscoveryPageJSON", &[json!(uri), json!("")])?;
    let sections = value
        .pointer("/data/dynamicPage/sections/edges")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    for batch in sections.iter().take(24).collect::<Vec<_>>().chunks(3) {
        let results = std::thread::scope(|scope| {
            let jobs: Vec<_> = batch
                .iter()
                .map(|edge| {
                    let node = &edge["node"];
                    let id = string(node, &["id"]);
                    let kind = string(node, &["__typename"]);
                    let title = string(node, &["title"]);
                    scope.spawn(move || {
                        let result = if id.is_empty() {
                            Ok(Value::Null)
                        } else {
                            deezer_shelves::api(
                                engine,
                                "DZDiscoverySectionJSON",
                                &[json!(id), json!(kind), json!("")],
                            )
                        };
                        (id, title, kind, result)
                    })
                })
                .collect();
            jobs.into_iter()
                .filter_map(|job| job.join().ok())
                .collect::<Vec<_>>()
        });
        for (id, title, kind, result) in results {
            if id.is_empty() {
                continue;
            }
            match result {
                Ok(content) => {
                    deezer_shelves::remember_dynamic(page, &title, &content, &id, &kind);
                    page.sections.push(Section {
                        title,
                        entries: entries(&content, "playlist"),
                        ..Default::default()
                    });
                    page.raw.push((format!("section:{id}"), Value::Null));
                }
                Err(error) => page.sections.push(Section {
                    title,
                    error,
                    ..Default::default()
                }),
            }
            publish(page)?;
        }
    }
    let cover = page
        .entity
        .as_ref()
        .map(|item| item.artwork.as_str())
        .unwrap_or("");
    deezer_shelves::cover_directories(&mut page.sections, cover);
    page.raw.push((uri.into(), Value::Null));
    Ok(())
}

fn podcast_library_path(engine: &Engine) -> Result<std::path::PathBuf, String> {
    let user = engine.invoke("DZUserID", &[])?;
    let user = user
        .as_str()
        .map(str::to_owned)
        .or_else(|| user.as_u64().map(|id| id.to_string()))
        .ok_or("No podcast library account")?;
    if user.is_empty() || !user.bytes().all(|b| b.is_ascii_digit()) {
        return Err("No verified podcast library account".into());
    }
    Ok(crate::storage::data_dir()?.join(format!("podcasts-{user}.json")))
}

fn read_podcast_library(path: &std::path::Path) -> Result<Vec<Entry>, String> {
    match std::fs::read(path) {
        Ok(bytes) => {
            let value: Value = serde_json::from_slice(&bytes)
                .map_err(|e| format!("Invalid saved podcast library: {e}"))?;
            if !value.is_array() {
                return Err("Invalid saved podcast library".into());
            }
            Ok(entries(&value, "podcast"))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(error) => Err(error.to_string()),
    }
}

fn edit_podcast_library(items: &mut Vec<Entry>, item: Entry, add: bool) -> Result<(), String> {
    if item.kind != "podcast" || item.id.is_empty() {
        return Err("Select a podcast to save".into());
    }
    items.retain(|old| old.id != item.id);
    if add {
        items.push(item);
    }
    Ok(())
}

// App-local subscription support. The native ABI has no Deezer podcast-follow
// mutation; do not misroute a podcast ID into the track-favorites API.
fn podcast_library(engine: &Engine, args: &[Value]) -> Result<Value, String> {
    let path = podcast_library_path(engine)?;
    let mut items = read_podcast_library(&path)?;
    if !args.is_empty() {
        if args.len() != 2 {
            return Err("Podcast library expects a podcast object and add/remove boolean".into());
        }
        let add = args[1]
            .as_bool()
            .ok_or("Podcast library add/remove must be boolean")?;
        let item = entry(&args[0], "podcast").ok_or("Invalid podcast")?;
        edit_podcast_library(&mut items, item, add)?;
        let bytes = serde_json::to_vec(&items.iter().map(Entry::wire).collect::<Vec<_>>())
            .map_err(|e| e.to_string())?;
        let temporary = path.with_extension("json.tmp");
        std::fs::create_dir_all(path.parent().ok_or("Invalid podcast library path")?)
            .map_err(|e| e.to_string())?;
        std::fs::write(&temporary, bytes).map_err(|e| e.to_string())?;
        std::fs::rename(&temporary, &path).map_err(|e| e.to_string())?;
    }
    Ok(
        json!({"local":true,"changed":!args.is_empty(),"podcasts":items.iter().map(Entry::wire).collect::<Vec<_>>()}),
    )
}

pub fn podcast_library_page(
    account: Vec<Entry>,
    local: Vec<Entry>,
    bookmarks: Vec<Entry>,
    offline: Vec<Entry>,
) -> Page {
    let mut seen = HashSet::new();
    let saved = account
        .into_iter()
        .chain(local)
        .filter(|item| item.kind == "podcast" && seen.insert(item.id.clone()))
        .collect();
    let episodes = |items: Vec<Entry>| items.into_iter().filter(|e| e.kind == "episode").collect();
    Page {
        title: "Podcasts".into(),
        subtitle: "Your podcasts".into(),
        sections: vec![
            Section {
                title: "Saved podcasts".into(),
                entries: saved,
                ..Default::default()
            },
            Section {
                title: "Favorites".into(),
                entries: episodes(bookmarks),
                ..Default::default()
            },
            Section {
                title: "Downloads".into(),
                entries: episodes(offline),
                ..Default::default()
            },
        ],
        ..Default::default()
    }
}
fn load(
    engine: &Engine,
    view: View,
    entity: Option<Entry>,
    query: String,
    publish: &mut dyn FnMut(&Page) -> Result<(), String>,
) -> Result<Page, String> {
    let mut page = Page {
        title: entity
            .as_ref()
            .map(|e| e.title.clone())
            .unwrap_or_else(|| view.title().into()),
        entity: entity.clone(),
        ..Default::default()
    };
    if view == View::Explore && query.is_empty() {
        return discover(engine, page, publish);
    }
    if view == View::Podcasts {
        let mut errors = Vec::new();
        let mut raw = Vec::new();
        let offline = crate::downloads::load_offline();
        let local = match podcast_library(engine, &[]) {
            Ok(value) => entries(&value["podcasts"], "podcast"),
            Err(error) => {
                errors.push(error);
                Vec::new()
            }
        };
        let mut page = podcast_library_page(Vec::new(), local.clone(), Vec::new(), offline.clone());
        publish(&page)?;
        let account = match engine.invoke("DZPodcastShelfJSON", &[json!(1)]) {
            Ok(value) => {
                let items = entries(&value, "podcast");
                raw.push(("podcasts:account".into(), Value::Null));
                items
            }
            Err(error) => {
                errors.push(error);
                Vec::new()
            }
        };
        page = podcast_library_page(account.clone(), local.clone(), Vec::new(), offline.clone());
        publish(&page)?;
        let bookmarks = match engine.invoke("DZPodcastShelfJSON", &[json!(3)]) {
            Ok(value) => {
                let items = entries(&value, "episode");
                raw.push(("podcasts:bookmarks".into(), Value::Null));
                items
            }
            Err(error) => {
                errors.push(error);
                Vec::new()
            }
        };
        page = podcast_library_page(account, local, bookmarks, offline);
        page.error = errors.join("; ");
        page.raw = raw;
        return Ok(page);
    }
    if view == View::StyleMix {
        if !query.is_empty() {
            let selection = crate::style_mix::Selection::parse(&query)?;
            page.subtitle = format!("{} · From matching Deezer playlists", selection.title());
            page.rows = crate::style_mix::tracks(engine, selection)?;
        }
        return Ok(page);
    }
    let (name, args, kind, table) = match view {
        View::Favorites => ("DZFavoritesJSON", vec![], "track", true),
        View::Albums => ("DZFavoriteAlbumsJSON", vec![], "album", false),
        View::Playlists | View::PlaylistPicker => ("DZPlaylistsJSON", vec![], "playlist", false),
        View::Recent => ("DZHistoryRecentJSON", vec![json!(200)], "track", true),
        View::Flow => {
            let (name, args) = flow_request(entity.as_ref(), &query);
            if entity.as_ref().is_some_and(|entry| entry.kind == "track") {
                page.subtitle = "Song radio".into();
            }
            (name, args, "track", true)
        }
        View::Queue => ("DZQueueJSON", vec![], "track", true),
        View::Explore => ("DZSearchJSON", vec![json!(query)], "track", false),
        View::Album | View::Artist | View::Show => {
            let item = entity
                .as_ref()
                .ok_or("Select an album, artist, playlist or podcast first")?;
            match item.kind.as_str() {
                "playlist" => ("DZPlaylistTracksJSON", vec![json!(item.id)], "track", true),
                "artist" => ("DZArtistProfileJSON", vec![json!(item.id)], "album", false),
                "podcast" => (
                    "DZPodcastEpisodesJSON",
                    vec![json!(item.id)],
                    "episode",
                    true,
                ),
                "mix" => ("DZSmartMixJSON", vec![json!(item.id)], "track", true),
                "flow" => ("DZFlowJSON", vec![], "track", true),
                "channel" => {
                    if item.id.starts_with("deezer://") {
                        dynamic_sections(engine, &item.id, &mut page, publish)?;
                        for section in &mut page.sections {
                            section.entries.retain(|item| {
                                matches!(
                                    item.kind.as_str(),
                                    "playlist" | "album" | "mix" | "podcast"
                                )
                            });
                        }
                        return Ok(page);
                    }
                    if item.id.starts_with("editorial:") {
                        publish(&page)?;
                        let id = item.id.trim_start_matches("editorial:");
                        for (part, label, kind) in [
                            ("charts", "Top playlists", "playlist"),
                            ("releases", "New releases", "album"),
                            ("selection", "Editors' album picks", "album"),
                        ] {
                            match deezer_shelves::public(
                                engine,
                                &format!("editorial/{id}/{part}?limit=8"),
                            ) {
                                Ok(value) => {
                                    let data = if part == "charts" {
                                        &value["playlists"]
                                    } else {
                                        &value
                                    };
                                    deezer_shelves::remember_public(&mut page, label, data, kind);
                                    page.sections.push(Section {
                                        title: label.into(),
                                        entries: curated_entries(data, kind),
                                        ..Default::default()
                                    });
                                    page.raw.push((part.into(), Value::Null));
                                }
                                Err(error) => page.sections.push(Section {
                                    title: label.into(),
                                    error,
                                    ..Default::default()
                                }),
                            }
                            publish(&page)?;
                        }
                        return Ok(page);
                    }
                    let value = deezer_shelves::web(
                        engine,
                        &format!(
                            "https://www.deezer.com/en{}",
                            item.id.trim_start_matches("web:")
                        ),
                    )?;
                    page.sections = web_sections(&value, false);
                    page.raw.push(("web".into(), Value::Null));
                    return Ok(page);
                }
                _ => ("DZAlbumTracksJSON", vec![json!(item.id)], "track", true),
            }
        }
        _ => return Ok(page),
    };
    let value = engine.invoke(name, &args)?;
    if table {
        page.rows = table_entries(&value, kind);
        inherit_collection_metadata(&mut page, &value);
        if let Some(playlist) = page.entity.as_ref().filter(|item| item.kind == "playlist") {
            if let Ok(metadata) = public(engine, &format!("playlist/{}", playlist.id)) {
                let preview = playlist.artwork.clone();
                inherit_collection_metadata(&mut page, &metadata);
                if preview.starts_with("playlist-cover:") {
                    if let Some(entity) = page.entity.as_mut() {
                        entity.artwork = preview;
                    }
                }
            }
        }
        // The native album API often returns only tracks, without a header.
        if let Some(album) = page
            .entity
            .as_ref()
            .filter(|item| item.kind == "album" && item.artwork.is_empty())
        {
            if let Ok(metadata) = public(engine, &format!("album/{}", album.id)) {
                inherit_collection_metadata(&mut page, &metadata);
                page.raw.push(("album-metadata".into(), Value::Null));
            }
        }
        if let Some(mix) = value.pointer("/data/smartTracklist") {
            if let Some(entity) = page.entity.as_mut() {
                let artwork = picture(mix);
                if !artwork.is_empty() {
                    entity.artwork = artwork;
                }
                entity.subtitle.clear();
                entity.raw = compact_entry_metadata(&mix);
            }
        }
    } else if view == View::Explore {
        for (key, label, typ) in [
            ("tracks", "Tracks", "track"),
            ("albums", "Albums", "album"),
            ("artists", "Artists", "artist"),
            ("playlists", "Playlists", "playlist"),
            ("podcasts", "Podcasts", "podcast"),
        ] {
            if let Some(data) = value.get(key) {
                page.sections.push(Section {
                    title: label.into(),
                    entries: entries(data, typ),
                    ..Default::default()
                });
            }
        }
        publish(&page)?;
        if !query.is_empty()
            && !page
                .sections
                .iter()
                .any(|section| section.title == "Podcasts" && !section.entries.is_empty())
        {
            if let Ok(podcasts) = engine.invoke("DZSearchPodcastsJSON", &[json!(query)]) {
                page.sections.push(Section {
                    title: "Podcasts".into(),
                    entries: entries(&podcasts, "podcast"),
                    ..Default::default()
                });
                page.raw.push(("podcast-search".into(), Value::Null));
            }
        }
    } else {
        page.sections.push(Section {
            title: page.title.clone(),
            entries: entries(&value, kind),
            ..Default::default()
        });
    }
    if view == View::Show {
        if let Some(podcast) = page.entity.as_mut().filter(|item| item.kind == "podcast") {
            // The native episodes API returns the complete paginated collection.
            podcast.raw["episodeCount"] = json!(page.rows.len());
        }
    }
    if view == View::Artist {
        if let Some(item) = entity.as_ref() {
            // Profile is an envelope, not the artist object itself.
            if let (Some(metadata), Some(artist)) = (value.get("artist"), page.entity.as_mut()) {
                update_artist_metadata(artist, metadata);
            }
            page.sections.clear();
            let mut top = table_entries(&value["top"], "track");
            if top.is_empty() {
                if let Ok(response) = engine.invoke("DZArtistTopJSON", &[json!(item.id)]) {
                    top = table_entries(&response, "track");
                }
            }
            if !top.is_empty() {
                page.sections.push(Section {
                    title: "Popular tracks".into(),
                    entries: top,
                    ..Default::default()
                });
            }
            for (index, label) in [
                (1, "Albums"),
                (2, "Singles and EPs"),
                (3, "Appears on"),
                (4, "Similar artists"),
                (5, "Related playlists"),
            ] {
                publish(&page)?;
                let fallback = match index {
                    1 => entries(&value["albums"], "album"),
                    4 => entries(&value["related"], "artist"),
                    _ => Vec::new(),
                };
                match engine.invoke(
                    "DZArtistSectionJSON",
                    &[json!(item.id), json!(index), json!("")],
                ) {
                    Ok(section) => {
                        if let (Some(metadata), Some(artist)) =
                            (section.pointer("/data/artist"), page.entity.as_mut())
                        {
                            update_artist_metadata(artist, metadata);
                        }
                        let items = artist_section_entries(&section, index);
                        page.sections.push(Section {
                            title: label.into(),
                            entries: items,
                            ..Default::default()
                        });
                        page.raw.push((format!("artist:{index}"), Value::Null));
                    }
                    Err(error) => page.sections.push(Section {
                        title: label.into(),
                        error: if fallback.is_empty() {
                            error
                        } else {
                            String::new()
                        },
                        entries: fallback,
                        ..Default::default()
                    }),
                }
            }
            page.rows.clear();
        }
    }
    page.next = if view == View::Flow && !entity.as_ref().is_some_and(|entry| entry.kind == "track")
    {
        format!("flow:{query}")
    } else {
        string(&value, &["next"])
    };
    page.raw.push((name.into(), Value::Null));
    Ok(page)
}
#[derive(Clone)]
pub struct Bitmap {
    pub id: u64,
    pub width: u32,
    pub height: u32,
    pub pixels: Arc<Vec<u32>>,
}
impl Bitmap {
    pub fn new(width: u32, height: u32, pixels: Vec<u32>) -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        Self {
            id: NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            width,
            height,
            pixels: Arc::new(pixels),
        }
    }
}
pub fn decode(bytes: &[u8], size: u32) -> Result<Bitmap, String> {
    decode_image(bytes, size, true)
}
pub fn decode_background(bytes: &[u8], size: u32) -> Result<Bitmap, String> {
    decode_image(bytes, size, false)
}
fn decode_image(bytes: &[u8], size: u32, cover: bool) -> Result<Bitmap, String> {
    decode_image_target(bytes, size, cover, None)
}

pub(crate) fn style_mix_artwork_size(key: &str) -> Option<(u32, u32)> {
    let (width, height) = key.rsplit_once('@')?.1.split_once('x')?;
    Some((
        width.parse::<u32>().ok()?.clamp(1, 1024),
        height.parse::<u32>().ok()?.clamp(1, 1024),
    ))
}

fn decode_image_target(
    bytes: &[u8],
    size: u32,
    cover: bool,
    target: Option<(u32, u32)>,
) -> Result<Bitmap, String> {
    let mut reader = image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|e| e.to_string())?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(6000);
    limits.max_image_height = Some(6000);
    limits.max_alloc = Some(96 * 1024 * 1024);
    reader.limits(limits);
    let image = reader.decode().map_err(|e| e.to_string())?;
    let image = if let Some((width, height)) = target {
        image.resize_to_fill(width, height, image::imageops::FilterType::Triangle)
    } else if cover {
        artwork_resize::cover(image, size)
    } else {
        image.resize(size, size, image::imageops::FilterType::Triangle)
    }
    .into_rgba8();
    let pixels = image
        .pixels()
        .map(|p| 0xff000000 | ((p[0] as u32) << 16) | ((p[1] as u32) << 8) | p[2] as u32)
        .collect();
    Ok(Bitmap::new(image.width(), image.height(), pixels))
}
#[test]
fn style_mix_artwork_fills_target_with_center_crop() {
    for (width, height) in [(200, 100), (100, 200)] {
        let source = image::RgbaImage::from_fn(width, height, |x, y| {
            let position = if width > height { x } else { y };
            if (40..160).contains(&position) {
                image::Rgba([0, 255, 0, 255])
            } else {
                image::Rgba([255, 0, 0, 255])
            }
        });
        let mut encoded = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(source)
            .write_to(&mut encoded, image::ImageFormat::Png)
            .unwrap();
        let decoded = decode_image_target(encoded.get_ref(), 100, false, Some((100, 100))).unwrap();
        assert_eq!((decoded.width, decoded.height), (100, 100));
        assert!(decoded.pixels.iter().all(|&pixel| pixel == 0xff00ff00));
    }
    assert_eq!(
        style_mix_artwork_size("style-mix:region:0@274x200"),
        Some((274, 200))
    );
}

#[test]
fn flow_photographs_are_prepared_at_card_resolution_instead_of_source_resolution() {
    let key = genre_photos::key("Jazz");
    let bytes = genre_photos::bytes(&key).unwrap();
    for (width, height) in [(137, 100), (274, 200), (411, 300)] {
        let requested = format!("{key}@{width}x{height}");
        let image = decode_image_target(
            bytes,
            width.max(height),
            false,
            style_mix_artwork_size(&requested),
        )
        .unwrap();
        let image = crate::appearance::prepare_genre_photo(&image);
        assert_eq!((image.width, image.height), (width, height));
        assert_eq!(image.pixels.len(), (width * height) as usize);
    }
}

pub struct ArtRequest {
    #[cfg(feature = "diagnostics")]
    pub view_tag: usize,
    pub engine: Option<Arc<Engine>>,
    pub url: String,
    pub key: String,
    pub background: bool,
    pub max_edge: u32,
}

// Cards render at at most 160 physical pixels; leave sampling headroom without
// retaining a full 256px decoded copy for every thumbnail in a large queue.
pub const CARD_ART_EDGE: u32 = 192;

pub(crate) fn is_default_cover(url: &str) -> bool {
    url.contains("/images/cover/d41d8cd98f00b204e9800998ecf8427e/")
}

pub(crate) fn cover_artwork_key(url: &str) -> String {
    format!("cover:{}", display_artwork_url(url, 1024, true))
}

/// Spotify image arrays are not guaranteed to be ordered by resolution.
pub(crate) fn largest_artwork<'a>(images: impl Iterator<Item = &'a Value>) -> Option<&'a str> {
    images
        .filter_map(|image| {
            let url = image["url"]
                .as_str()
                .filter(|url| url.starts_with("https://"))?;
            let edge = image["width"]
                .as_u64()
                .unwrap_or(0)
                .max(image["height"].as_u64().unwrap_or(0));
            Some((edge, url))
        })
        .max_by_key(|(edge, _)| *edge)
        .map(|(_, url)| url)
}

pub struct Artwork {
    cache: Arc<std::sync::Mutex<ArtworkCache>>,
    queue: Arc<artwork_queue::ArtworkQueue>,
    cancelled_tx: mpsc::Sender<String>,
    pub cancelled: mpsc::Receiver<String>,
    #[cfg(test)]
    fixture_requests: Option<mpsc::SyncSender<ArtRequest>>,
    pub updates: mpsc::Receiver<(String, Result<Arc<Bitmap>, String>)>,
}

// Deezer URLs often request 1000px covers even for 160px cards. Keep the
// original UI key, but fetch the size we actually decode. Other hosts are unchanged.
#[cfg(test)]
fn artwork_url(url: &str, background: bool) -> String {
    display_artwork_url(url, if background { 768 } else { 256 }, false)
}
pub fn display_artwork_url(url: &str, size: u32, upscale: bool) -> String {
    let host = url
        .strip_prefix("https://")
        .and_then(|rest| rest.split('/').next());
    if host.is_some_and(|host| {
        matches!(
            host,
            "lh3.googleusercontent.com" | "yt3.googleusercontent.com" | "yt3.ggpht.com"
        )
    }) {
        let (path, query) = url.split_once('?').map_or((url, ""), |(a, b)| (a, b));
        if let Some((base, options)) = path.rsplit_once('=') {
            let size = size.clamp(64, 1024);
            let mut resized = false;
            let parts: Vec<String> = options
                .split('-')
                .map(|part| {
                    if let Some((prefix, digits)) = ["w", "h", "s"].iter().find_map(|prefix| {
                        part.strip_prefix(prefix).map(|digits| (*prefix, digits))
                    }) {
                        if !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()) {
                            if let Ok(original) = digits.parse::<u32>() {
                                resized = true;
                                return format!(
                                    "{prefix}{}",
                                    if upscale { size } else { original.min(size) }
                                );
                            }
                        }
                    }
                    part.to_owned()
                })
                .collect();
            if resized {
                return format!(
                    "{base}={}{}",
                    parts.join("-"),
                    if query.is_empty() {
                        String::new()
                    } else {
                        format!("?{query}")
                    }
                );
            }
        }
        return url.into();
    }
    if !url.starts_with("https://")
        || !url
            .split('/')
            .nth(2)
            .is_some_and(|host| host == "dzcdn.net" || host.ends_with(".dzcdn.net"))
    {
        return url.into();
    }
    let Some((base, file)) = url.rsplit_once('/') else {
        return url.into();
    };
    let Some((dimensions, suffix)) = file.split_once('-') else {
        return url.into();
    };
    let Some((width, height)) = dimensions.split_once('x') else {
        return url.into();
    };
    let (Ok(width), Ok(height)) = (width.parse::<u32>(), height.parse::<u32>()) else {
        return url.into();
    };
    let size = size.clamp(64, 1024);
    if !upscale && (width <= size || height <= size) {
        return url.into();
    }
    format!("{base}/{size}x{size}-{suffix}")
}

#[derive(Default)]
struct ArtworkCache {
    // One owning cache shares allocations with resident UI images. Recent
    // offscreen covers survive navigation instead of immediately expiring.
    items: std::collections::VecDeque<(String, bool, Arc<Bitmap>, usize, std::time::Instant)>,
    bytes: usize,
    pins: std::collections::HashSet<u64>,
}
impl ArtworkCache {
    fn get(&mut self, url: &str, background: bool) -> Option<Arc<Bitmap>> {
        let index = self
            .items
            .iter()
            .position(|(key, bg, ..)| key == url && *bg == background)?;
        let mut item = self.items.remove(index)?;
        let image = item.2.clone();
        item.4 = std::time::Instant::now();
        self.items.push_back(item);
        Some(image)
    }
    fn insert(&mut self, url: String, background: bool, image: Arc<Bitmap>) {
        const LIMIT: usize = 64 * 1024 * 1024;
        let bytes = image.pixels.len() * 4;
        if bytes > LIMIT {
            return;
        }
        if let Some(index) = self
            .items
            .iter()
            .position(|(key, bg, ..)| key == &url && *bg == background)
        {
            if let Some((_, _, _, bytes, _)) = self.items.remove(index) {
                self.bytes -= bytes;
            }
        }
        self.bytes += bytes;
        self.items
            .push_back((url, background, image, bytes, std::time::Instant::now()));
        self.trim_to(LIMIT);
    }
    fn trim_to(&mut self, limit: usize) {
        while self.bytes > limit || self.items.len() > 1024 {
            let Some(index) = self
                .items
                .iter()
                .position(|entry| !self.pins.contains(&entry.2.id))
            else {
                break;
            };
            let entry = self.items.remove(index).unwrap();
            self.bytes = self.bytes.saturating_sub(entry.3);
        }
    }
}
fn artwork_identity(image: &Bitmap, key: &str, background: bool) -> u64 {
    use sha2::{Digest, Sha256};
    let mut hash = Sha256::new();
    hash.update(key.as_bytes());
    hash.update([background as u8]);
    hash.update(image.width.to_le_bytes());
    hash.update(image.height.to_le_bytes());
    // Bitmap pixels are an initialized contiguous u32 buffer, read only here.
    hash.update(unsafe {
        std::slice::from_raw_parts(image.pixels.as_ptr().cast::<u8>(), image.pixels.len() * 4)
    });
    let digest = hash.finalize();
    (1u64 << 62) | (u64::from_le_bytes(digest[..8].try_into().unwrap()) & ((1u64 << 62) - 1))
}
impl Artwork {
    pub fn recent_ids(&self) -> std::collections::HashSet<u64> {
        self.cache
            .lock()
            .unwrap()
            .items
            .iter()
            .filter(|e| e.4.elapsed() < std::time::Duration::from_secs(5))
            .map(|e| e.2.id)
            .collect()
    }
    pub fn residency(
        &self,
        pins: std::collections::HashSet<u64>,
        visible: &std::collections::HashSet<u64>,
    ) {
        let mut cache = self.cache.lock().unwrap();
        cache.pins = pins;
        for entry in &mut cache.items {
            if visible.contains(&entry.2.id) {
                entry.4 = std::time::Instant::now();
            }
        }
        cache.trim_to(64 * 1024 * 1024);
    }
    pub fn shrink(&self, pins: std::collections::HashSet<u64>) {
        let mut cache = self.cache.lock().unwrap();
        cache.pins = pins;
        cache.trim_to(2 * 1024 * 1024);
    }
    pub fn request(&self, request: ArtRequest, visible: bool) -> Result<(), ArtRequest> {
        #[cfg(test)]
        if let Some(sender) = &self.fixture_requests {
            return sender.try_send(request).map_err(|e| match e {
                mpsc::TrySendError::Full(r) | mpsc::TrySendError::Disconnected(r) => r,
            });
        }
        self.queue.push(request, visible)
    }
    #[cfg(test)]
    pub(crate) fn fixture(
        requests: mpsc::SyncSender<ArtRequest>,
        updates: mpsc::Receiver<(String, Result<Arc<Bitmap>, String>)>,
    ) -> Self {
        let (cancelled_tx, cancelled) = mpsc::channel();
        Self {
            cache: Arc::new(std::sync::Mutex::new(ArtworkCache::default())),
            queue: Arc::new(artwork_queue::ArtworkQueue::default()),
            cancelled_tx,
            cancelled,
            updates,
            fixture_requests: Some(requests),
        }
    }
    pub fn set_wanted(
        &self,
        keys: std::collections::HashSet<String>,
        visible: std::collections::HashSet<String>,
    ) {
        for key in self.queue.update(keys, visible) {
            let _ = self.cancelled_tx.send(key);
        }
    }
    pub fn start() -> Self {
        let queue = Arc::new(artwork_queue::ArtworkQueue::default());
        let (cancelled_tx, cancelled) = mpsc::channel();
        let (tx, updates) = mpsc::sync_channel(16);
        let cache = Arc::new(std::sync::Mutex::new(ArtworkCache::default()));
        let cpus = std::thread::available_parallelism().map_or(4, usize::from);
        let (decode_tx, decode_rx) = mpsc::sync_channel::<(ArtRequest, Vec<u8>, String, u32)>(4);
        let decode_rx = Arc::new(std::sync::Mutex::new(decode_rx));
        // More network overlap without letting simultaneous decoders consume
        // every core when the foreground UI is CPU constrained.
        for index in 0..(cpus / 4).clamp(1, 2) {
            let work = decode_rx.clone();
            let tx = tx.clone();
            let cache = cache.clone();
            let queue = queue.clone();
            let cancelled = cancelled_tx.clone();
            std::thread::Builder::new()
                .name(format!("artwork-decode-{index}"))
                .spawn(move || {
                    artwork_background_priority();
                    loop {
                        let job = work.lock().unwrap().recv();
                        let Ok((request, bytes, cache_key, edge)) = job else {
                            return;
                        };
                        if !queue.wanted(&request) {
                            let _ = cancelled.send(request.key);
                            continue;
                        }
                        #[cfg(feature = "diagnostics")]
                        let _page_scope =
                            crate::memory_profile::page_scope(if request.view_tag == 0 {
                                0
                            } else {
                                request.view_tag + 128
                            });
                        let result = (if request.url.starts_with("style-mix:") {
                            decode_image_target(
                                &bytes,
                                edge,
                                false,
                                style_mix_artwork_size(&request.key),
                            )
                        } else if request.url.starts_with(genre_photos::PREFIX) {
                            decode_image_target(
                                &bytes,
                                edge,
                                false,
                                style_mix_artwork_size(&request.key),
                            )
                        } else if request.background {
                            decode_background(&bytes, edge)
                        } else {
                            decode(&bytes, edge)
                        })
                        .map(|image| {
                            let mut image = if request.url.starts_with("style-mix:") {
                                crate::appearance::prepare_style_mix_photo(&image)
                            } else if request.url.starts_with(genre_photos::PREFIX) {
                                crate::appearance::prepare_genre_photo(&image)
                            } else if request.url.starts_with("flow-genre:") {
                                crate::appearance::prepare(&image, 5)
                            } else {
                                image
                            };
                            image.id = artwork_identity(&image, &cache_key, request.background);
                            Arc::new(image)
                        });
                        if !queue.wanted(&request) {
                            let _ = cancelled.send(request.key);
                            continue;
                        }
                        if let Ok(image) = &result {
                            cache.lock().unwrap().insert(
                                cache_key,
                                request.background,
                                image.clone(),
                            );
                        }
                        let hover = if edge > 256
                            && !request.background
                            && !request.url.starts_with(genre_photos::PREFIX)
                            && !request.url.starts_with("style-mix:")
                        {
                            result
                                .as_ref()
                                .ok()
                                .map(|image| (format!("blur:cover:{}", request.key), image.clone()))
                        } else {
                            None
                        };
                        if tx.send((request.key, result)).is_err() {
                            return;
                        }
                        if let Some((key, image)) = hover {
                            let blurred = Arc::new(crate::appearance::prepare(&image, 24));
                            if tx.send((key, Ok(blurred))).is_err() {
                                return;
                            }
                        }
                    }
                })
                .expect("start artwork decoder");
        }
        for index in 0..cpus.clamp(4, 8) {
            let queue = queue.clone();
            let cancelled = cancelled_tx.clone();
            let tx = tx.clone();
            let cache = cache.clone();
            let decode_tx = decode_tx.clone();
            std::thread::Builder::new()
                .name(format!("artwork-fetch-{index}"))
                .spawn(move || {
                    artwork_background_priority();
                    loop {
                        let Some(request) = queue.pop() else {
                            return;
                        };
                        if !queue.wanted(&request) {
                            let _ = cancelled.send(request.key);
                            continue;
                        }
                        #[cfg(feature = "diagnostics")]
                        let _page_scope =
                            crate::memory_profile::page_scope(if request.view_tag == 0 {
                                0
                            } else {
                                request.view_tag + 128
                            });
                        let edge = request.max_edge.clamp(64, 1024);
                        let cache_key = if request.url.starts_with("style-mix:")
                            || request.url.starts_with(genre_photos::PREFIX)
                        {
                            format!("bundled-cover:{}", request.key)
                        } else {
                            format!("{}@{edge}", request.url)
                        };
                        let cached = cache.lock().unwrap().get(&cache_key, request.background);
                        if let Some(image) = cached {
                            if tx.send((request.key, Ok(image))).is_err() {
                                return;
                            }
                            continue;
                        }
                        if request.url.starts_with(genre_photos::PREFIX) {
                            let Some(bytes) = genre_photos::bytes(&request.url) else {
                                if tx
                                    .send((request.key, Err("Unknown genre photograph".into())))
                                    .is_err()
                                {
                                    return;
                                }
                                continue;
                            };
                            if decode_tx
                                .send((request, bytes.to_vec(), cache_key, edge))
                                .is_err()
                            {
                                return;
                            }
                            continue;
                        }
                        if request.url.starts_with("style-mix:") {
                            match crate::style_mix::artwork::data(&request.url) {
                                Some(bytes) => {
                                    if decode_tx
                                        .send((request, bytes.to_vec(), cache_key, edge))
                                        .is_err()
                                    {
                                        return;
                                    }
                                }
                                None => {
                                    let _ = tx
                                        .send((request.key, Err("Unknown bundled artwork".into())));
                                }
                            }
                            continue;
                        }
                        let resolved = if request.url.starts_with("flow-genre:") {
                            match crate::spotify::flow::resolve_artwork(
                                &request.url,
                                request.engine.as_deref(),
                            ) {
                                Ok(url) => url,
                                Err(error) => {
                                    if tx.send((request.key, Err(error))).is_err() {
                                        return;
                                    }
                                    continue;
                                }
                            }
                        } else {
                            request.url.clone()
                        };
                        if !queue.wanted(&request) {
                            let _ = cancelled.send(request.key);
                            continue;
                        }
                        let url = display_artwork_url(&resolved, edge, edge > 256);
                        let public_cdn = url
                            .strip_prefix("https://")
                            .and_then(|rest| rest.split('/').next())
                            .is_some_and(|host| {
                                host == "dzcdn.net" || host.ends_with(".dzcdn.net")
                            });
                        let bytes = if public_cdn {
                            crate::lyrics::http::artwork(&url)
                        } else if let Some(engine) = &request.engine {
                            engine.fetch(&url)
                        } else {
                            crate::lyrics::http::request("GET", &url, &[], &[])
                        };
                        match bytes {
                            Ok(bytes) => {
                                if !queue.wanted(&request) {
                                    let _ = cancelled.send(request.key);
                                    continue;
                                }
                                if decode_tx.send((request, bytes, cache_key, edge)).is_err() {
                                    return;
                                }
                            }
                            Err(error) => {
                                if tx.send((request.key, Err(error))).is_err() {
                                    return;
                                }
                            }
                        }
                    }
                })
                .expect("start artwork fetcher");
        }
        Self {
            cache,
            queue,
            cancelled_tx,
            cancelled,
            updates,
            #[cfg(test)]
            fixture_requests: None,
        }
    }
}
impl Drop for Artwork {
    fn drop(&mut self) {
        self.queue.close();
    }
}
pub(crate) fn artwork_background_priority() {
    unsafe {
        use windows_sys::Win32::System::Threading::{
            GetCurrentThread, SetThreadPriority, THREAD_MODE_BACKGROUND_BEGIN,
        };
        SetThreadPriority(GetCurrentThread(), THREAD_MODE_BACKGROUND_BEGIN);
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn collection_metadata_keeps_credits_and_counts_without_duplicate_track_arrays() {
        let input = serde_json::json!({"id":"1","title":"Collection","description":"Keep this","artists":[{"id":"2","name":"Artist"}],"tracks":{"data":vec![serde_json::json!({"id":"3","title":"Track"});5000],"total":5000},"episodes":{"data":[],"totalCount":42}});
        let compact = compact_entry_metadata(&input);
        assert_eq!(compact["description"], input["description"]);
        assert_eq!(compact["artists"], input["artists"]);
        assert_eq!(compact["tracks"]["total"], 5000);
        assert_eq!(compact["episodes"]["totalCount"], 42);
        assert!(compact["tracks"].get("data").is_none());
        assert!(compact.to_string().len() * 100 < input.to_string().len());
    }
    use super::*;
    #[test]
    fn youtube_artwork_requests_larger_images_for_playback() {
        let small = "https://lh3.googleusercontent.com/album=w60-h60-l90-rj";
        assert_eq!(
            display_artwork_url(small, 1024, true),
            "https://lh3.googleusercontent.com/album=w1024-h1024-l90-rj"
        );
        assert_eq!(display_artwork_url(small, 192, false), small);
        assert_eq!(
            display_artwork_url(
                "https://yt3.googleusercontent.com/album=w120-h120-l90-rj",
                1024,
                true
            ),
            "https://yt3.googleusercontent.com/album=w1024-h1024-l90-rj"
        );
        assert_eq!(
            display_artwork_url("https://yt3.ggpht.com/image=s500-c-k?x=1", 192, false),
            "https://yt3.ggpht.com/image=s192-c-k?x=1"
        );
        assert_eq!(
            display_artwork_url("https://example.com/image=w60-h60", 1024, true),
            "https://example.com/image=w60-h60"
        );
        assert_eq!(
            display_artwork_url("https://lh3.googleusercontent.com/image=other", 1024, true),
            "https://lh3.googleusercontent.com/image=other"
        );
    }
    #[test]
    fn spotify_artwork_selects_largest_valid_image_in_any_order() {
        let images = json!([
            {"url":"https://example.test/small","width":64},
            {"url":"https://example.test/large","width":640},
            {"url":"http://example.test/invalid","width":2000},
            {"url":"https://example.test/medium","width":300}
        ]);
        assert_eq!(
            largest_artwork(images.as_array().unwrap().iter()),
            Some("https://example.test/large")
        );
        assert_ne!(
            cover_artwork_key("https://example.test/large"),
            "https://example.test/large"
        );
    }
    #[test]
    #[ignore = "requires the playback DLL and internet; writes only public timing results"]
    fn profile_public_web_cache() {
        let engine = Engine::load().unwrap();
        let mut report = Vec::new();
        for (name, url) in [
            ("explore", "https://www.deezer.com/en/channels/explore"),
            ("podcasts", "https://www.deezer.com/en/channels/podcasts"),
        ] {
            let cold_started = std::time::Instant::now();
            let cold = web_catalog(&engine, url);
            let cold_ms = cold_started.elapsed().as_secs_f64() * 1000.;
            match cold {
                Ok(first) => {
                    let store = session::Store::for_session();
                    let page = Arc::new(Page {
                        title: name.into(),
                        sections: web_sections(&first, false),
                        ..Default::default()
                    });
                    store.begin(1, name.into(), None);
                    store.apply(1, page.clone());
                    store.begin(2, name.into(), None);
                    let warm_started = std::time::Instant::now();
                    let second = store.resume(2).unwrap();
                    let warm_ms = warm_started.elapsed().as_secs_f64() * 1000.;
                    assert_eq!(page.sections, second.sections);
                    report.push(json!({"page":name,"cold_ms":cold_ms,"disk_ms":warm_ms}));
                }
                Err(error) => report.push(json!({"page":name,"cold_ms":cold_ms,"error":error})),
            }
        }
        let path = std::env::var("DEEZNATS_FETCH_PROFILE")
            .expect("Set DEEZNATS_FETCH_PROFILE to an output JSON path");
        std::fs::write(path, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
    }

    #[test]
    fn album_tracks_inherit_cover_and_independent_album_fields() {
        let value = json!({"data":{"album":{"id":"42", "title":"Album", "cover_xl":"https://example.test/album.jpg", "tracks":[
            {"id":"1", "title":"First"},
            {"id":"2", "title":"Second", "albumName":"Known title"},
            {"id":"3", "title":"Third", "artworkUrl":"https://example.test/track.jpg", "albumId":"other"}
        ]}}});
        let mut page = Page {
            entity: entry(&json!({"id":"42","title":"Album"}), "album"),
            rows: table_entries(&value, "track"),
            ..Default::default()
        };
        inherit_collection_metadata(&mut page, &value);
        assert_eq!(page.rows.len(), 3);
        assert_eq!(
            page.entity.as_ref().unwrap().artwork,
            "https://example.test/album.jpg"
        );
        assert_eq!(page.rows[0].artwork, page.rows[1].artwork);
        assert_eq!(
            page.rows[0].wire()["artworkUrl"],
            "https://example.test/album.jpg"
        );
        assert_eq!(page.rows[0].album, "Album");
        assert_eq!(page.rows[1].album, "Known title");
        assert_eq!(page.rows[1].album_id, "42");
        assert_eq!(page.rows[2].album_id, "other");
        assert_eq!(page.rows[2].artwork, "https://example.test/track.jpg");
    }

    #[test]
    fn playlist_tracks_inherit_missing_metadata_without_changing_episodes() {
        let mut page = Page {
            entity: entry(
                &json!({"id":"42","title":"Playlist","cover":"https://example.test/list.jpg"}),
                "playlist",
            ),
            rows: entries(
                &json!([
                    {"id":"1","title":"Track","kind":"track"},
                    {"id":"2","title":"Episode","kind":"episode"}
                ]),
                "track",
            ),
            ..Default::default()
        };
        inherit_collection_metadata(&mut page, &json!({}));
        assert_eq!(page.rows[0].artwork, "https://example.test/list.jpg");
        assert_eq!(page.rows[0].album_id, "42");
        assert!(page.rows[1].artwork.is_empty());
    }

    #[test]
    fn genre_charts_and_channel_shelves_keep_curated_collections() {
        let charts = json!({"tracks":{"data":[{"id":"1","title":"Track"}]},
            "artists":{"data":[{"id":"2","name":"Artist"}]},
            "playlists":{"data":[{"id":"3","title":"Top playlist"}]}});
        let playlists = curated_entries(&charts["playlists"], "playlist");
        assert_eq!(playlists.len(), 1);
        assert_eq!(playlists[0].kind, "playlist");
        let state = json!({"sections":[{"title":"Curated", "items":[
            {"id":"1","title":"Artist","data":{"__TYPE__":"artist"}},
            {"id":"2","title":"100% artist","type":"playlist"},
            {"id":"3","title":"Timeless","type":"album"},
            {"id":"4","title":"Mix","type":"mix"},
            {"id":"6","title":"Show","type":"show","target":"/show/6"}
        ]},{"title":"Artists", "items":[{"id":"5","title":"Raw artist","type":"artist"}]}]});
        let shelves = web_sections(&state, false);
        assert_eq!(shelves.len(), 1);
        assert_eq!(
            shelves[0]
                .entries
                .iter()
                .map(|e| e.kind.as_str())
                .collect::<Vec<_>>(),
            ["playlist", "album", "mix", "podcast"]
        );
    }

    #[test]
    fn discover_filters_distinguish_channel_directories() {
        for (title, filter) in [
            ("Genres", 2),
            ("Categories", 3),
            ("Podcasts by category", 4),
        ] {
            let section = Section {
                title: title.into(),
                entries: entries(
                    &json!([
                        {"type":"channel", "title":"Tile", "target":"/channels/chill"}
                    ]),
                    "channel",
                ),
                ..Default::default()
            };
            assert_eq!(section.entries[0].id, "web:/channels/chill");
            for index in 1..=4 {
                assert_eq!(discover_section_matches(&section, index), index == filter);
            }
            assert!(discover_section_matches(&section, 0));
        }
    }

    #[test]
    fn history_metadata_fills_covers_without_replacing_recorded_fields() {
        let mut row = entry(&json!({"trackId":"123", "title":"Recorded title", "artist":"Recorded artist", "album":"Recorded album", "startedAt":"today"}), "track").unwrap();
        assert!(row.artwork.is_empty());
        assert_eq!(row.subtitle, "Recorded artist");
        assert_eq!(row.album, "Recorded album");
        let metadata = entry(&json!({"id":"123", "name":"Catalog title", "artistLine":"Catalog artist", "artworkUrl":"https://example.test/cover.jpg", "durationMs":120000, "artistId":"456", "albumId":"789"}), "track").unwrap();
        enrich_history_entry(&mut row, &metadata);
        assert_eq!(row.artwork, metadata.artwork);
        assert_eq!(row.title, "Recorded title");
        assert_eq!(row.subtitle, "Recorded artist");
        assert_eq!(row.duration, 120000);
        assert_eq!(row.artist_id, "456");
        assert_eq!(row.album_id, "789");
        assert_eq!(row.raw["startedAt"], "today");
        assert_eq!(row.raw["title"], "Recorded title");
        let mut episode = entry(
            &json!({"trackId":"123", "title":"Episode", "kind":"episode"}),
            "track",
        )
        .unwrap();
        enrich_history_entry(&mut episode, &metadata);
        assert_eq!(episode.kind, "episode");
        assert_eq!(episode.artwork, metadata.artwork);
    }

    #[test]
    fn podcast_library_merges_saved_shows_likes_and_downloads_without_recommendations() {
        let show = Entry {
            id: "show".into(),
            title: "Saved show".into(),
            kind: "podcast".into(),
            ..Default::default()
        };
        let episode = Entry {
            id: "episode".into(),
            title: "Saved episode".into(),
            kind: "episode".into(),
            ..Default::default()
        };
        let music = Entry {
            id: "song".into(),
            kind: "track".into(),
            ..Default::default()
        };
        let page = podcast_library_page(
            vec![show.clone()],
            vec![show, music.clone()],
            vec![episode.clone(), music.clone()],
            vec![episode, music],
        );
        assert_eq!(
            page.sections
                .iter()
                .map(|s| s.title.as_str())
                .collect::<Vec<_>>(),
            vec!["Saved podcasts", "Favorites", "Downloads"]
        );
        assert!(page.sections.iter().all(|s| s.entries.len() == 1));
        assert_eq!(page.sections[0].entries[0].kind, "podcast");
        assert!(page.sections[1..]
            .iter()
            .all(|s| s.entries[0].kind == "episode"));
        let bookmarks = json!({"data":{"me":{"podcastEpisodeBookmarks":{"edges":[{"node":{"episode":{"id":"ep","title":"Liked episode","duration":120,"podcast":{"id":"show","displayTitle":"Show title"}}}}]}}}});
        let parsed = entries(&bookmarks, "episode");
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].subtitle, "Show title");
    }
    #[test]
    fn artist_sections_extract_connections_without_repeating_artist_metadata() {
        for (index, kind, node) in [
            (
                0,
                "track",
                json!({"id":"song","title":"Popular song","duration":200}),
            ),
            (
                1,
                "album",
                json!({"id":"album","displayTitle":"Actual album"}),
            ),
            (
                2,
                "album",
                json!({"id":"single","displayTitle":"Actual single"}),
            ),
            (
                3,
                "album",
                json!({"id":"featured","displayTitle":"Featured album"}),
            ),
            (4, "artist", json!({"id":"related","name":"Another artist"})),
            (
                5,
                "playlist",
                json!({"id":"playlist","title":"Actual playlist","owner":{"name":"Deezer"}}),
            ),
        ] {
            let mut artist = json!({"id":"lamb","name":"Lamb Of God","fansCount":123456,"picture":{"urls":["https://example.test/artist.jpg"]}});
            let connection = json!({"edges":[{"node":node}],"pageInfo":{"hasNextPage":false}});
            if index == 5 {
                artist["playlists"] = json!({"items":connection});
            } else {
                artist["items"] = connection;
            }
            let response = json!({"data":{"artist":artist}});
            let rows = artist_section_entries(&response, index);
            assert_eq!(rows.len(), 1);
            assert_eq!(rows[0].kind, kind);
            assert_ne!(rows[0].id, "lamb");
            assert!(artist_section_entries(
                &json!({"data":{"artist":{"id":"lamb","name":"Lamb Of God","items":{"edges":[]}}}}),
                index
            )
            .is_empty());
        }
        let mut artist = Entry {
            id: "lamb".into(),
            title: "Lamb Of God".into(),
            ..Default::default()
        };
        update_artist_metadata(
            &mut artist,
            &json!({"id":"lamb","name":"Lamb Of God","nbFans":123456,"artworkUrl":"https://example.test/profile.jpg"}),
        );
        assert_eq!(artist.artwork, "https://example.test/profile.jpg");
        assert_eq!(artist.raw["fans"], "123456");
        update_artist_metadata(
            &mut artist,
            &json!({"id":"lamb","name":"Lamb Of God","fansCount":654321}),
        );
        assert_eq!(artist.raw["fans"], "654321");
        assert_eq!(artist.artwork, "https://example.test/profile.jpg");
    }
    #[test]
    fn descriptions_and_marketing_subtitles_never_replace_titles_or_credits() {
        let playlist=entry(&json!({"__typename":"Playlist","id":"p","title":"Hits Remixed","subTitle":"Dance & EDM Editor description","description":"A long playlist description","owner":{"name":"Laeti\n- Deezer"}}),"playlist").unwrap();
        assert_eq!(playlist.title, "Hits Remixed");
        assert_eq!(playlist.subtitle, "Laeti - Deezer");
        assert_eq!(playlist.raw["description"], "A long playlist description");
        let missing=entry(&json!({"id":"p","title":"Techno Tunnel","subTitle":"Description masquerading as a subtitle","description":"About this playlist"}),"playlist").unwrap();
        assert!(missing.subtitle.is_empty());
        assert_eq!(missing.title, "Techno Tunnel");
        let podcast=entry(&json!({"id":"show","name":"Podcast","subtitle":"A very long description","description":"Full details"}),"podcast").unwrap();
        assert!(podcast.subtitle.is_empty());
        assert_eq!(podcast.title, "Podcast");
        let track=entry(&json!({"id":"song","title":"Song","subTitle":"Promotional blurb","artist":{"id":"a","name":"Singer"}}),"track").unwrap();
        assert_eq!(track.subtitle, "Singer");
        let dto = entry(
            &json!({"id":"native","name":"Playlist","owner":"Deezer"}),
            "playlist",
        )
        .unwrap();
        assert_eq!(dto.subtitle, "Deezer");
    }
    #[test]
    fn personalized_mix_shelf_excludes_flow_and_names_daily_mixes() {
        let value = json!({"data":{"me":{"madeForMe":{"edges":[
            {"node":{"__typename":"Flow","id":"flow","title":"Flow"}},
            {"node":{"__typename":"SmartTracklist","id":"mix-a","title":"daily","subTitle":"Featuring Borne, Ivy Lab"}},
            {"node":{"__typename":"SmartTracklist","id":"mix-b","title":"daily","subTitle":"Featuring Eminem, 50 Cent"}},
            {"node":{"__typename":"SmartTracklist","id":"mix-c","title":"Discoveries"}}
        ]}}}});
        let items = home_module_entries(&value, 1, "playlist");
        assert_eq!(
            items.iter().map(|e| e.title.as_str()).collect::<Vec<_>>(),
            vec!["Daily Mix 1", "Daily Mix 2", "Discoveries"]
        );
        assert_eq!(items[0].id, "mix-a");
        assert!(items[1].subtitle.is_empty());
        assert!(home_module_entries(&value, 0, "playlist")
            .iter()
            .any(|e| e.kind == "flow"));
    }
    #[test]
    fn home_album_shelf_keeps_album_identity_under_tracks_wrapper() {
        let value = json!({"tracks": [
            {"id":"764165091", "displayTitle":"Metalicious", "cover":{"urls":["cover"]}},
            {"__typename":"Track", "id":"123", "title":"A track", "duration":180}
        ]});
        let items = home_module_entries(&value, 8, "album");
        assert_eq!(items[0].kind, "album");
        assert_eq!(items[0].id, "764165091");
        assert_eq!(items[1].kind, "track");
    }
    #[test]
    fn fallback_home_mix_dtos_get_numbered_without_marketing_credits() {
        let value = json!([{"id":"flow","kind":"flow","name":"Flow"},
            {"id":"m1","kind":"mix","name":"daily","subtitle":"?n planda Artist"},
            {"id":"m2","kind":"playlist","name":"daily","subtitle":"?n planda Other"}]);
        let mut items = entries(&value, "playlist");
        normalize_daily_mixes(&mut items);
        assert_eq!(
            items.iter().map(|e| e.title.as_str()).collect::<Vec<_>>(),
            vec!["Flow", "Daily Mix 1", "Daily Mix 2"]
        );
        assert!(items[1..]
            .iter()
            .all(|e| e.kind == "mix" && e.subtitle.is_empty()));
    }
    #[test]
    fn smart_mix_rows_come_from_tracks_not_the_collection_header() {
        let response = json!({"data":{"smartTracklist":{"id":"daily","title":"daily","subTitle":"Featuring artists","tracks":{"edges":[
            {"node":{"id":"1","title":"First song","duration":187000,"album":{"id":"a","displayTitle":"Album"},"contributors":{"edges":[{"node":{"id":"singer","name":"Singer"}}]}}},
            {"node":{"id":"2","title":"Second song","duration":180,"contributors":{"edges":[]}}}
        ]}}}});
        let rows = table_entries(&response, "track");
        assert_eq!(
            rows.iter().map(|e| e.id.as_str()).collect::<Vec<_>>(),
            vec!["1", "2"]
        );
        assert_eq!((rows[0].duration, rows[1].duration), (187000, 180000));
        assert_eq!(rows[0].artist_id, "singer");
        assert_eq!(rows[0].subtitle, "Singer");
        assert_eq!(rows[0].album, "Album");
        assert!(rows.iter().all(Entry::playable));
        assert!(table_entries(&json!({"data":{"smartTracklist":{"id":"daily","title":"daily","tracks":{"edges":[]}}}}),"track").is_empty());
        assert_eq!(table_entries(&json!({"id":"playlist","title":"Metadata","tracks":{"data":[{"id":"3","title":"REST track","duration":200}]}}),"track")[0].id,"3");
        assert_eq!(
            table_entries(
                &json!({"episodes":[{"id":"4","title":"Episode","durationMs":1000}]}),
                "episode"
            )[0]
            .kind,
            "episode"
        );
    }
    #[test]
    fn multiple_artist_targets_survive_playback_wire_roundtrip() {
        let track = entry(
            &json!({"id":"song","title":"Together","artistId":"1",
            "artistLine":"First, Second", "contributors":{"edges":[
                {"node":{"id":"1","name":"First"}},
                {"node":{"id":"2","name":"Second"}},
                {"node":{"id":"1","name":"First"}}
            ]}}),
            "track",
        )
        .unwrap();
        let artists = track.artist_entries();
        assert_eq!(
            artists.iter().map(|a| a.id.as_str()).collect::<Vec<_>>(),
            vec!["1", "2"]
        );
        let restored = entry(&track.wire(), "track").unwrap();
        assert_eq!(
            restored
                .artist_entries()
                .iter()
                .map(|a| a.title.as_str())
                .collect::<Vec<_>>(),
            vec!["First", "Second"]
        );
    }

    #[test]
    fn navigation_targets_use_related_ids_and_preserve_metadata() {
        let track=entry(&json!({"id":1,"title":"Song","artist":{"id":7,"name":"Singer","picture_medium":"https://example.test/artist.jpg"},"album":{"id":9,"title":"Album","cover_medium":"https://example.test/album.jpg"}}),"track").unwrap();
        let artist = track.artist_entry().unwrap();
        let album = track.album_entry().unwrap();
        assert_eq!(
            (
                artist.id.as_str(),
                artist.kind.as_str(),
                artist.title.as_str()
            ),
            ("7", "artist", "Singer")
        );
        assert_eq!(
            (album.id.as_str(), album.kind.as_str(), album.title.as_str()),
            ("9", "album", "Album")
        );
        assert!(artist.artwork.ends_with("artist.jpg"));
        assert!(album.artwork.ends_with("album.jpg"));
        assert!(Entry::default().artist_entry().is_none());
        assert!(Entry::default().album_entry().is_none());
    }

    #[test]
    fn artwork_fetch_uses_bounded_cdn_sizes_and_lru_reuses_bitmaps() {
        assert_eq!(
            artwork_url(
                "https://cdn-images.dzcdn.net/images/cover/abc/1000x1000-000000-80-0-0.jpg",
                false
            ),
            "https://cdn-images.dzcdn.net/images/cover/abc/256x256-000000-80-0-0.jpg"
        );
        for url in [
            "https://other.test/1000x1000-a.jpg",
            "https://dzcdn.net.evil.test/1000x1000-a.jpg",
            "https://cdn.dzcdn.net/100x100-a.jpg",
        ] {
            assert_eq!(artwork_url(url, false), url);
        }
        let mut cache = ArtworkCache::default();
        let image = Arc::new(Bitmap::new(1, 1, vec![0]));
        cache.insert("first".into(), false, image.clone());
        assert!(Arc::ptr_eq(&cache.get("first", false).unwrap(), &image));
        assert!(cache.get("first", true).is_none());
        for i in 0..1024 {
            cache.insert(i.to_string(), false, image.clone());
        }
        assert_eq!(cache.items.len(), 1024);
        assert_eq!(cache.bytes, 1024 * 4);
        assert!(cache.get("first", false).is_none());
    }

    #[test]
    fn artwork_cache_retains_pixels_until_explicit_eviction() {
        let mut cache = ArtworkCache::default();
        let image = Arc::new(Bitmap::new(1, 1, vec![0]));
        let allocation = Arc::downgrade(&image);
        cache.insert("cover".into(), false, image.clone());
        let consumer = cache.get("cover", false).unwrap();
        drop(image);
        assert!(allocation.upgrade().is_some());
        drop(consumer);
        assert!(allocation.upgrade().is_some());
        assert!(cache.get("cover", false).is_some());
        cache.trim_to(0);
        assert!(allocation.upgrade().is_none());
        assert_eq!(cache.bytes, 0);
        assert!(cache.items.is_empty());
    }

    #[test]
    fn artwork_resource_identity_survives_redecode_but_changes_with_pixels() {
        let a = Bitmap::new(1, 1, vec![0xff123456]);
        let b = Bitmap::new(1, 1, vec![0xff123456]);
        assert_ne!(a.id, b.id);
        assert_eq!(
            artwork_identity(&a, "cover@192", false),
            artwork_identity(&b, "cover@192", false)
        );
        assert_ne!(
            artwork_identity(&a, "cover@192", false),
            artwork_identity(&b, "other@192", false)
        );
        assert_ne!(
            artwork_identity(&a, "cover@192", false),
            artwork_identity(&Bitmap::new(1, 1, vec![0xff000000]), "cover@192", false)
        );
        let mut cache = ArtworkCache::default();
        cache.insert("pinned".into(), false, Arc::new(a));
        let id = cache.get("pinned", false).unwrap().id;
        cache.pins.insert(id);
        cache.trim_to(0);
        assert_eq!(cache.bytes, 4);
        cache.pins.clear();
        cache.trim_to(0);
        assert_eq!(cache.bytes, 0);
    }

    #[test]
    fn large_cover_requests_upgrade_cdn_resolution_without_affecting_other_hosts() {
        let small = "https://cdn-images.dzcdn.net/images/cover/id/250x250-000000-80-0-0.jpg";
        assert!(display_artwork_url(small, 1024, true).contains("/1024x1024-"));
        assert!(display_artwork_url(small, 4096, true).contains("/1024x1024-"));
        assert_eq!(display_artwork_url(small, 256, false), small);
        let other = "https://example.test/250x250-cover.jpg";
        assert_eq!(display_artwork_url(other, 1024, true), other);
    }

    #[test]
    fn podcast_saves_are_idempotent_and_preserve_counts() {
        let podcast = entry(&json!({"id":"42","kind":"podcast","name":"Show","nb_episodes":"12","description":"About"}), "podcast").unwrap();
        let mut saved = Vec::new();
        edit_podcast_library(&mut saved, podcast.clone(), true).unwrap();
        edit_podcast_library(&mut saved, podcast.clone(), true).unwrap();
        assert_eq!(saved.len(), 1);
        let restored = entry(&saved[0].wire(), "podcast").unwrap();
        assert_eq!(restored.episode_count(), Some(12));
        assert_eq!(restored.raw["description"], "About");
        edit_podcast_library(&mut saved, podcast, false).unwrap();
        assert!(saved.is_empty());
        assert!(edit_podcast_library(
            &mut saved,
            Entry {
                id: "1".into(),
                kind: "track".into(),
                ..Default::default()
            },
            true
        )
        .is_err());
    }

    #[test]
    fn pagination_preserves_order_deduplicates_boundaries_and_stops_cycles() {
        let mut page = Page {
            error: "Transient failure".into(),
            ..Default::default()
        };
        append_page(
            &mut page,
            "page1",
            json!({"data":[{"id":1,"title":"A"},{"id":2,"title":"B"}],"next":"page2"}),
        );
        append_page(
            &mut page,
            "page2",
            json!({"data":[{"id":2,"title":"B"},{"id":3,"title":"C"}],"next":"page1"}),
        );
        assert_eq!(
            page.rows
                .iter()
                .map(|item| item.id.as_str())
                .collect::<Vec<_>>(),
            ["1", "2", "3"]
        );
        assert!(page.next.is_empty());
        assert!(page.error.is_empty());
        append_page(&mut page, "same", json!({"data":[],"next":"same"}));
        assert!(page.next.is_empty());
    }

    #[test]
    fn rest_graphql_and_saved_tracks_keep_the_same_identity() {
        let rest = json!({"id":42,"type":"track","title":"Example","duration":183,
            "artist":{"id":7,"name":"Artist"},
            "album":{"id":9,"title":"Album","cover_medium":"https://example.test/cover.jpg"}});
        let track = entry(&rest, "track").unwrap();
        assert_eq!(track.duration, 183_000);
        assert_eq!(track.subtitle, "Artist");
        let saved = entry(&track.wire(), "track").unwrap();
        assert_eq!(
            (saved.id, saved.artist_id, saved.album_id),
            ("42".into(), "7".into(), "9".into())
        );
        assert_eq!(saved.artwork, track.artwork);
        assert_eq!(saved.duration, track.duration);
        let graphql = json!({"__typename":"Section","id":"section","title":"Shelf",
            "edges":[{"node":{"__typename":"Track","id":"42","title":"Example"}},
                     {"node":{"__typename":"Track","id":"42","title":"Example"}}]});
        let parsed = entries(&graphql, "track");
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].id, "42");
    }

    #[test]
    fn catalog_state_parser_rejects_invalid_input() {
        let html = br#"<script>window.__DZR_APP_STATE__ = {"tracks":[{"id":1,"title":"A; B"}]}; dangerous();</script>"#;
        let state = web_state(html).unwrap();
        assert_eq!(entries(&state, "track")[0].title, "A; B");
        assert!(web_state(b"<html>No catalog</html>").is_err());
        assert!(web_state(b"window.__DZR_APP_STATE__ = broken;").is_err());
    }
}

fn append_flow_rows(page: &mut Page, rows: Vec<Entry>) {
    let mut seen: HashSet<_> = page
        .rows
        .iter()
        .map(|e| (e.kind.clone(), e.id.clone()))
        .collect();
    page.rows.extend(
        rows.into_iter()
            .filter(|e| seen.insert((e.kind.clone(), e.id.clone()))),
    );
}

#[cfg(test)]
mod flow_batch_tests {
    use super::*;
    #[test]
    fn additional_batches_keep_order_and_skip_duplicates() {
        let track = |id: &str| Entry {
            id: id.into(),
            kind: "track".into(),
            ..Default::default()
        };
        let mut page = Page {
            rows: vec![track("1"), track("2")],
            next: "flow:default".into(),
            ..Default::default()
        };
        append_flow_rows(
            &mut page,
            vec![track("2"), track("3"), track("3"), track("4")],
        );
        assert_eq!(
            page.rows.iter().map(|e| e.id.as_str()).collect::<Vec<_>>(),
            ["1", "2", "3", "4"]
        );
        assert_eq!(page.next, "flow:default");
    }
}
