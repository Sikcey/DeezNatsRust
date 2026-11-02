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
        let Some(page) = self.read(&key) else {
            self.state.lock().unwrap().saved.remove(&key);
            return None;
        };
        let mut state = self.state.lock().unwrap();
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
            let merged = match route.page.as_deref() {
                Some(previous) => Arc::new(merge(Some(previous), &incoming)),
                None => incoming,
            };
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
        expected
            .raw
            .retain(|(key, _)| !key.starts_with("home:") && !key.starts_with("discover-source:"));
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
        let base = std::env::temp_dir().join(format!("deeznats-flow-test-{}", std::process::id()));
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
