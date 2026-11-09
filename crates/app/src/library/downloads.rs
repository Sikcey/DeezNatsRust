use crate::{catalog::Entry, engine::Engine};
use serde_json::Value;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc, Arc,
};
pub struct Request {
    pub id: u64,
    pub engine: Arc<Engine>,
    pub items: Vec<Entry>,
    pub quality: usize,
    pub cancelled: Arc<AtomicBool>,
}
pub enum Event {
    Maintenance(Result<String, String>),
    Started(u64, String),
    Removed(Result<(), String>),
    RemovedSelection(Result<(), String>),
    Track(u64, usize, usize, Box<Entry>, Result<Value, String>),
    Finished(u64, bool),
}
#[derive(serde::Serialize, serde::Deserialize)]
struct Job {
    id: u64,
    items: Vec<Entry>,
    quality: usize,
}
enum Message {
    Maintenance(bool),
    Download(Request),
    Remove(Option<Arc<Engine>>),
    RemoveSelection(Option<Arc<Engine>>, Entry, Option<Entry>),
}

pub struct Worker {
    requests: mpsc::Sender<Message>,
    removing: Arc<AtomicBool>,
    resumed: bool,
    pub updates: mpsc::Receiver<Event>,
}
impl Worker {
    pub fn maintenance(&self, clean: bool) -> Result<(), String> {
        self.requests
            .send(Message::Maintenance(clean))
            .map_err(|_| "Download worker unavailable".into())
    }

    pub fn start() -> Self {
        let (requests, rx) = mpsc::channel::<Message>();
        let removing = Arc::new(AtomicBool::new(false));
        let (tx, updates) = mpsc::sync_channel(8);
        std::thread::spawn(move || {
            let mut last_engine = None;
            while let Ok(message) = rx.recv() {
                let request = match message {
                    Message::Maintenance(clean) => {
                        let result =
                            crate::storage::data_dir().and_then(|dir| maintenance_at(&dir, clean));
                        if tx.send(Event::Maintenance(result)).is_err() {
                            return;
                        }
                        continue;
                    }

                    Message::Download(request) => request,
                    Message::RemoveSelection(engine, selected, page) => {
                        let engine = engine
                            .or_else(|| last_engine.clone())
                            .or_else(|| Engine::load().ok().map(Arc::new));
                        let result = crate::storage::data_dir().and_then(|directory| {
                            remove_selected_at(&directory, &selected, page.as_ref(), |id| {
                                if let Some(engine) = &engine {
                                    let result = engine
                                        .invoke("DZRemoveOfflineTrack", &[serde_json::json!(id)])?;
                                    if result["removed"] != true {
                                        return Err(
                                            "Engine did not confirm download removal".into()
                                        );
                                    }
                                }
                                Ok(())
                            })
                        });
                        if tx.send(Event::RemovedSelection(result)).is_err() {
                            return;
                        }
                        continue;
                    }
                    Message::Remove(engine) => {
                        let engine = engine
                            .or_else(|| last_engine.clone())
                            .or_else(|| Engine::load().ok().map(Arc::new));
                        let result = crate::storage::data_dir().and_then(|directory| {
                            remove_all_at(&directory, |id| {
                                if let Some(engine) = &engine {
                                    let result = engine
                                        .invoke("DZRemoveOfflineTrack", &[serde_json::json!(id)])?;
                                    if result["removed"] != true {
                                        return Err(
                                            "Engine did not confirm download removal".into()
                                        );
                                    }
                                }
                                Ok(())
                            })
                        });
                        if tx.send(Event::Removed(result)).is_err() {
                            return;
                        }
                        continue;
                    }
                };
                last_engine = Some(request.engine.clone());
                let total = request.items.len();
                let mut pending = request.items.clone();
                for (index, item) in request.items.into_iter().enumerate() {
                    if request.cancelled.load(Ordering::Relaxed) {
                        break;
                    }
                    let mut item = item;
                    if tx
                        .send(Event::Started(request.id, item.title.clone()))
                        .is_err()
                    {
                        return;
                    }
                    let result = retry(&request.cancelled, || {
                        let value = if item.kind == "episode" {
                            request
                                .engine
                                .invoke("DZDownloadEpisodeJSON", &[serde_json::json!(item.id)])
                        } else {
                            download_at_quality(request.quality, &request.cancelled, |quality| {
                                request.engine.invoke(
                                    "DZDownloadForOfflineAtQuality",
                                    &[serde_json::json!(item.id), serde_json::json!(quality)],
                                )
                            })
                        }?;
                        let directory = crate::storage::data_dir()?;
                        let path = verified_path(&directory, &item, Some(&value))?;
                        let mut receipt = receipt(&path)?;
                        if let Some(key) = value.get("key") {
                            receipt["key"] = key.clone();
                        }
                        Ok(receipt)
                    });
                    let result = result.and_then(|receipt| {
                        item.raw["_offline"] = receipt.clone();
                        let mut offline = load_offline();
                        if let Some(previous) = offline
                            .iter()
                            .find(|e| e.id == item.id && e.kind == item.kind)
                        {
                            let mut groups = previous.raw["_download_groups"]
                                .as_array()
                                .cloned()
                                .unwrap_or_default();
                            for group in item.raw["_download_groups"]
                                .as_array()
                                .into_iter()
                                .flatten()
                            {
                                if !groups
                                    .iter()
                                    .any(|g| g["id"] == group["id"] && g["kind"] == group["kind"])
                                {
                                    groups.push(group.clone());
                                }
                            }
                            item.raw["_download_groups"] = groups.into();
                            if previous.raw["_download_single"] == true
                                || previous.raw.get("_download_groups").is_none()
                            {
                                item.raw["_download_single"] = true.into();
                            }
                        }
                        offline.retain(|e| e.id != item.id || e.kind != item.kind);
                        offline.push(item.clone());
                        persist_offline(&offline)?;
                        pending.retain(|e| e.id != item.id || e.kind != item.kind);
                        save_job(&Job {
                            id: request.id,
                            items: pending.clone(),
                            quality: request.quality,
                        })?;
                        Ok(receipt)
                    });
                    if tx
                        .send(Event::Track(
                            request.id,
                            index + 1,
                            total,
                            Box::new(item),
                            result,
                        ))
                        .is_err()
                    {
                        return;
                    }
                }
                if request.cancelled.load(Ordering::Relaxed) {
                    let _ = remove_job(request.id);
                }
                let _ = tx.send(Event::Finished(
                    request.id,
                    request.cancelled.load(Ordering::Relaxed),
                ));
            }
        });
        Self {
            requests,
            removing,
            updates,
            resumed: false,
        }
    }
    /// Persist before acknowledging queue acceptance, including jobs not yet started.
    pub fn remove_all(&self, engine: Option<Arc<Engine>>) -> Result<(), String> {
        if self.removing.swap(true, Ordering::AcqRel) {
            return Err("Download removal is already in progress".into());
        }
        if self.requests.send(Message::Remove(engine)).is_err() {
            self.removing.store(false, Ordering::Release);
            return Err("Download worker is unavailable".into());
        }
        Ok(())
    }

    pub fn remove_selection(
        &self,
        engine: Option<Arc<Engine>>,
        selected: Entry,
        page: Option<Entry>,
    ) -> Result<(), String> {
        if self.removing.swap(true, Ordering::AcqRel) {
            return Err("Download removal is already in progress".into());
        }
        if self
            .requests
            .send(Message::RemoveSelection(engine, selected, page))
            .is_err()
        {
            self.removing.store(false, Ordering::Release);
            return Err("Download worker is unavailable".into());
        }
        Ok(())
    }
    pub fn finish_removal(&self) {
        self.removing.store(false, Ordering::Release);
    }

    pub fn enqueue(&self, request: Request) -> Result<(), String> {
        if self.removing.load(Ordering::Acquire) {
            return Err("Wait for download removal to finish".into());
        }
        save_job(&Job {
            id: request.id,
            items: request.items.clone(),
            quality: request.quality,
        })?;
        let id = request.id;
        if self.requests.send(Message::Download(request)).is_err() {
            remove_job(id)?;
            return Err("Download queue is full".into());
        }
        Ok(())
    }
    pub fn resume(&mut self, engine: Arc<Engine>) -> Vec<(Status, Arc<AtomicBool>)> {
        if self.resumed || self.removing.load(Ordering::Acquire) {
            return Vec::new();
        }
        self.resumed = true;
        let Ok(directory) = crate::storage::data_dir() else {
            return Vec::new();
        };
        let Ok(files) = std::fs::read_dir(directory.join("download-jobs")) else {
            return Vec::new();
        };
        let mut restored = Vec::new();
        for file in files.flatten() {
            if file.path().extension().and_then(|s| s.to_str()) != Some("json") {
                continue;
            }
            let Ok(bytes) = std::fs::read(file.path()) else {
                continue;
            };
            let Ok(job) = serde_json::from_slice::<Job>(&bytes) else {
                continue;
            };
            let cancel = Arc::new(AtomicBool::new(false));
            let status = Status {
                id: job.id,
                title: "Recovered download".into(),
                total: job.items.len(),
                message: "Resuming pending tracks".into(),
                ..Default::default()
            };
            // Recovered work is already durable; do not rewrite it while dispatching.
            if self
                .requests
                .send(Message::Download(Request {
                    id: job.id,
                    engine: engine.clone(),
                    items: job.items,
                    quality: job.quality,
                    cancelled: cancel.clone(),
                }))
                .is_err()
            {
                break;
            }
            restored.push((status, cancel));
        }
        restored
    }
}
#[derive(Clone, Default)]
pub struct Status {
    pub id: u64,
    pub title: String,
    pub done: usize,
    pub total: usize,
    pub failed: usize,
    pub message: String,
    pub finished: bool,
}
/// Inspect only the engine-owned media directory. A grace period protects files
/// being written by a concurrent streaming request before its index is committed.
fn maintenance_at(directory: &std::path::Path, clean: bool) -> Result<String, String> {
    let root = directory.join("mediacache");
    if root
        .symlink_metadata()
        .is_ok_and(|info| info.file_type().is_symlink())
    {
        return Err("Cache directory is a symlink; cleanup stopped".into());
    }
    let offline = load_offline_at(directory)?;
    let index: Value = match std::fs::read(root.join("index.json")) {
        Ok(bytes) => {
            serde_json::from_slice(&bytes).map_err(|e| format!("Invalid cache index: {e}"))?
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound && !root.exists() => {
            serde_json::json!({"entries":{}})
        }
        Err(e) => return Err(format!("Cannot inspect cache index: {e}")),
    };
    let entries = index["entries"]
        .as_object()
        .ok_or("Invalid cache entries; cleanup stopped")?;
    let mut referenced = std::collections::HashSet::new();
    let mut downloads = 0u64;
    let mut cache = 0u64;
    for entry in entries.values() {
        let name = entry["file"]
            .as_str()
            .ok_or("Missing cache filename; cleanup stopped")?;
        if name.is_empty() || name.contains(['/', '\\', ':']) || name == "." || name == ".." {
            return Err("Unsafe cache filename; cleanup stopped".into());
        }
        referenced.insert(name.to_owned());
    }
    let mut orphan_bytes = 0u64;
    let mut removed = 0usize;
    if root.exists() {
        for file in std::fs::read_dir(&root).map_err(|e| e.to_string())? {
            let file = file.map_err(|e| e.to_string())?;
            let info = file.path().symlink_metadata().map_err(|e| e.to_string())?;
            if !info.is_file() || info.file_type().is_symlink() {
                continue;
            }
            let name = file.file_name().to_string_lossy().into_owned();
            if name == "index.json" || name.ends_with(".tmp") {
                continue;
            }
            if referenced.contains(&name) {
                if entries
                    .values()
                    .any(|entry| entry["file"].as_str() == Some(&name) && entry["pinned"] == true)
                {
                    downloads += info.len();
                } else {
                    cache += info.len();
                }
            } else {
                orphan_bytes += info.len();
                if clean
                    && info
                        .modified()
                        .ok()
                        .and_then(|time| time.elapsed().ok())
                        .is_some_and(|age| age.as_secs() >= 86400)
                {
                    std::fs::remove_file(file.path()).map_err(|e| e.to_string())?;
                    removed += 1;
                }
            }
        }
    }
    let mut missing = 0;
    for item in &offline {
        match verified_path(directory, item, None) {
            Ok(path) if item.kind == "episode" => {
                downloads += path.metadata().map_err(|e| e.to_string())?.len();
            }
            Ok(_) => {}
            Err(_) => missing += 1,
        }
    }
    Ok(format!("Cache: {:.1} MiB; downloads: {:.1} MiB; unreferenced: {:.1} MiB; missing: {missing}. Removed {removed} files (older than 24h).",
        cache as f64 / 1048576.0, downloads as f64 / 1048576.0, orphan_bytes as f64 / 1048576.0))
}

fn load_offline_at(directory: &std::path::Path) -> Result<Vec<Entry>, String> {
    match std::fs::read(directory.join("offline.json")) {
        Ok(bytes) => {
            let rows: Value = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
            let rows = rows.as_array().ok_or("Invalid offline library")?;
            rows.iter()
                .map(|row| {
                    crate::catalog::entry(row, "track")
                        .ok_or_else(|| "Invalid offline entry".into())
                })
                .collect()
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(e) => Err(e.to_string()),
    }
}

pub fn missing_downloads() -> Result<Vec<Entry>, String> {
    let directory = crate::storage::data_dir()?;
    Ok(load_offline_at(&directory)?
        .into_iter()
        .filter(|item| verified_path(&directory, item, None).is_err())
        .collect())
}

fn remove_all_at(
    directory: &std::path::Path,
    remove_track: impl FnMut(&str) -> Result<(), String>,
) -> Result<(), String> {
    remove_at(directory, None, remove_track)
}
fn remove_selected_at(
    directory: &std::path::Path,
    selected: &Entry,
    page: Option<&Entry>,
    remove_track: impl FnMut(&str) -> Result<(), String>,
) -> Result<(), String> {
    remove_at(directory, Some((selected, page)), remove_track)
}
fn remove_at(
    directory: &std::path::Path,
    selection: Option<(&Entry, Option<&Entry>)>,
    mut remove_track: impl FnMut(&str) -> Result<(), String>,
) -> Result<(), String> {
    fn read_json(path: &std::path::Path, default: Value) -> Result<Value, String> {
        match std::fs::read(path) {
            Ok(bytes) => serde_json::from_slice(&bytes).map_err(|e| e.to_string()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(default),
            Err(e) => Err(e.to_string()),
        }
    }
    fn remove_file(path: &std::path::Path) -> Result<(), String> {
        match std::fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(format!("Could not remove {}: {e}", path.display())),
        }
    }
    let document = read_json(&directory.join("offline.json"), serde_json::json!([]))?;
    if !document.is_array() {
        return Err("Invalid offline library; downloaded files were not removed".into());
    }
    // Process every persisted row, including damaged rows and libraries larger
    // than the catalog parser's display limit.
    let mut remaining: Vec<Entry> = document
        .as_array()
        .unwrap()
        .iter()
        .map(|row| {
            crate::catalog::entry(row, "track").unwrap_or_else(|| Entry {
                id: crate::catalog::string(row, &["id"]),
                kind: row["kind"].as_str().unwrap_or("track").to_owned(),
                raw: row.clone(),
                ..Default::default()
            })
        })
        .collect();
    let original = remaining.clone();
    let mut delete = std::collections::HashSet::new();
    if let Some((selected, page)) = selection {
        let group = if selected.kind == "offline_collection" {
            Some(selected)
        } else {
            page.filter(|p| p.kind == "offline_collection" && !p.id.starts_with("__"))
        };
        for item in &mut remaining {
            if selected.playable() && (selected.id != item.id || selected.kind != item.kind) {
                continue;
            }
            if let Some(group) = group {
                let Some(groups) = item.raw["_download_groups"].as_array_mut() else {
                    continue;
                };
                let before = groups.len();
                groups.retain(|g| {
                    g["id"] != group.raw["group_id"] || g["kind"] != group.raw["group_kind"]
                });
                if before == groups.len() {
                    continue;
                }
                if groups.is_empty() && item.raw["_download_single"] != true {
                    delete.insert((item.kind.clone(), item.id.clone()));
                }
            } else if selected.playable() {
                item.raw["_download_single"] = false.into();
                if item.raw["_download_groups"]
                    .as_array()
                    .is_none_or(|g| g.is_empty())
                {
                    delete.insert((item.kind.clone(), item.id.clone()));
                }
            }
        }
    }
    let cache = directory.join("mediacache");
    let index_path = cache.join("index.json");
    let index = read_json(&index_path, serde_json::json!({}))?;
    if !index.is_object()
        || index
            .get("entries")
            .is_some_and(|entries| !entries.is_object())
        || index.get("metas").is_some_and(|metas| !metas.is_object())
    {
        return Err("Invalid cache index; downloaded files were not removed".into());
    }
    let mut ids = std::collections::BTreeSet::new();
    for item in &remaining {
        if item.kind != "episode"
            && (selection.is_none() || delete.contains(&(item.kind.clone(), item.id.clone())))
        {
            ids.insert(item.id.clone());
        }
    }
    if selection.is_none() {
        if let Some(entries) = index["entries"].as_object() {
            for (key, entry) in entries {
                if entry["pinned"] == true {
                    if let Some((id, _)) = key.split_once('.') {
                        ids.insert(id.to_owned());
                    }
                }
            }
        }
    }
    let mut errors = Vec::new();
    for id in ids {
        let result = (|| {
            if id.is_empty() || !id.bytes().all(|c| c.is_ascii_digit()) {
                return Err("Invalid downloaded track ID".to_owned());
            }
            let prefix = format!("{id}.");
            let mut paths = Vec::new();
            if let Some(entries) = index["entries"].as_object() {
                for (key, entry) in entries.iter().filter(|(key, _)| key.starts_with(&prefix)) {
                    let name = entry["file"]
                        .as_str()
                        .ok_or("Missing downloaded filename")?;
                    if name.is_empty()
                        || name.contains(['/', '\\', ':'])
                        || name == "."
                        || name == ".."
                    {
                        return Err(format!("Unsafe downloaded filename for {key}"));
                    }
                    let path = cache.join(name);
                    if path.exists()
                        && !path
                            .canonicalize()
                            .map_err(|e| e.to_string())?
                            .starts_with(cache.canonicalize().map_err(|e| e.to_string())?)
                    {
                        return Err("Downloaded file is outside the cache".into());
                    }
                    paths.push(path);
                }
            }
            remove_track(&id)?;
            for path in paths {
                remove_file(&path)?;
            }
            // A live engine updates its own index. Handle signed-out deletion as well,
            // when no cache is attached and the engine has no in-memory entries.
            let mut current = read_json(&index_path, serde_json::json!({}))?;
            let mut changed = false;
            for field in ["entries", "metas"] {
                if let Some(values) = current[field].as_object_mut() {
                    let before = values.len();
                    values.retain(|key, _| !key.starts_with(&prefix));
                    changed |= before != values.len();
                }
            }
            if changed {
                write_json(&index_path, &current)?;
            }
            Ok(())
        })();
        match result {
            Ok(()) => remaining.retain(|item| item.kind == "episode" || item.id != id),
            Err(error) => {
                for item in remaining
                    .iter_mut()
                    .filter(|item| item.kind != "episode" && item.id == id)
                {
                    if let Some(old) = original
                        .iter()
                        .find(|old| old.kind == item.kind && old.id == item.id)
                    {
                        item.raw = old.raw.clone();
                    }
                }
                errors.push(error);
            }
        }
    }
    let mut removed_episodes = std::collections::HashSet::new();
    for item in remaining.iter().filter(|item| {
        item.kind == "episode"
            && (selection.is_none() || delete.contains(&(item.kind.clone(), item.id.clone())))
    }) {
        let result = if !item.id.is_empty() && item.id.bytes().all(|c| c.is_ascii_digit()) {
            episode_path(directory, &item.id).and_then(|path| remove_file(&path))
        } else {
            Err("Invalid downloaded episode ID".into())
        };
        match result {
            Ok(()) => {
                removed_episodes.insert(item.id.clone());
            }
            Err(error) => errors.push(error),
        }
    }
    for item in remaining.iter_mut().filter(|item| {
        item.kind == "episode"
            && delete.contains(&(item.kind.clone(), item.id.clone()))
            && !removed_episodes.contains(&item.id)
    }) {
        if let Some(old) = original
            .iter()
            .find(|old| old.kind == item.kind && old.id == item.id)
        {
            item.raw = old.raw.clone();
        }
    }
    remaining.retain(|item| item.kind != "episode" || !removed_episodes.contains(&item.id));
    persist_offline_at(directory, &remaining)?;
    // These belong to the canceled batch and must not resurrect on next launch.
    if selection.is_none() {
        match std::fs::read_dir(directory.join("download-jobs")) {
            Ok(files) => {
                for file in files {
                    let file = file.map_err(|e| e.to_string())?;
                    let path = file.path();
                    if path.extension().is_some_and(|ext| ext == "json")
                        && path
                            .file_stem()
                            .and_then(|id| id.to_str())
                            .is_some_and(|id| id.parse::<u64>().is_ok())
                    {
                        remove_file(&path)?;
                    }
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.to_string()),
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("; "))
    }
}

fn write_json(path: &std::path::Path, value: &impl serde::Serialize) -> Result<(), String> {
    let temporary = path.with_extension("json.tmp");
    let bytes = serde_json::to_vec(value).map_err(|e| e.to_string())?;
    let mut file = std::fs::File::create(&temporary).map_err(|e| e.to_string())?;
    use std::io::Write;
    file.write_all(&bytes)
        .and_then(|_| file.sync_all())
        .map_err(|e| e.to_string())?;
    drop(file);
    std::fs::rename(temporary, path).map_err(|e| e.to_string())
}
fn job_path(id: u64) -> Result<std::path::PathBuf, String> {
    Ok(crate::storage::data_dir()?
        .join("download-jobs")
        .join(format!("{id}.json")))
}
fn remove_job(id: u64) -> Result<(), String> {
    match std::fs::remove_file(job_path(id)?) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.to_string()),
    }
}
fn save_job(job: &Job) -> Result<(), String> {
    if job.items.is_empty() {
        return remove_job(job.id);
    }
    let path = job_path(job.id)?;
    std::fs::create_dir_all(path.parent().unwrap()).map_err(|e| e.to_string())?;
    write_json(&path, job)
}
fn retry(
    cancelled: &AtomicBool,
    mut operation: impl FnMut() -> Result<Value, String>,
) -> Result<Value, String> {
    let mut error = "Download cancelled".to_owned();
    for attempt in 0..3 {
        if cancelled.load(Ordering::Relaxed) {
            return Err("Download cancelled".into());
        }
        match operation() {
            Ok(value) => return Ok(value),
            Err(e) => error = e,
        }
        if attempt < 2 {
            for _ in 0..(10 << attempt) {
                if cancelled.load(Ordering::Relaxed) {
                    return Err("Download cancelled".into());
                }
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
        }
    }
    Err(error)
}
fn receipt(path: &std::path::Path) -> Result<Value, String> {
    use sha2::{Digest, Sha256};
    let mut file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let size = file.metadata().map_err(|e| e.to_string())?.len();
    if size == 0 {
        return Err("Offline file is empty".into());
    }
    let mut hash = Sha256::new();
    std::io::copy(&mut file, &mut hash).map_err(|e| e.to_string())?;
    Ok(serde_json::json!({"size":size,"sha256":format!("{:x}",hash.finalize())}))
}
fn episode_path(directory: &std::path::Path, id: &str) -> Result<std::path::PathBuf, String> {
    let root = std::env::var_os("OPENDEEZER_DOWNLOAD_DIR")
        .map(std::path::PathBuf::from)
        .or_else(|| {
            std::fs::read_to_string(directory.join("download-dir.txt"))
                .ok()
                .filter(|s| !s.trim().is_empty())
                .map(|s| s.trim().into())
        })
        .or_else(|| {
            std::env::var_os("USERPROFILE").map(|home| {
                let home = std::path::PathBuf::from(home);
                home.join(if home.join("Music").is_dir() {
                    "Music"
                } else {
                    "Downloads"
                })
                .join("OpenDeezer")
            })
        })
        .ok_or("Episode download folder unavailable")?;
    Ok(root.join("Podcasts").join(format!("{id}.mp3")))
}

fn verified_path(
    directory: &std::path::Path,
    item: &Entry,
    response: Option<&Value>,
) -> Result<std::path::PathBuf, String> {
    if item.id.is_empty() || !item.id.bytes().all(|c| c.is_ascii_digit()) {
        return Err("Invalid offline track ID".into());
    }
    if item.kind == "episode" {
        let path = episode_path(directory, &item.id)?;
        if response.is_some_and(|v| v["path"].as_str().is_none()) {
            return Err("Engine did not confirm episode file".into());
        }
        if !path
            .metadata()
            .is_ok_and(|info| info.is_file() && info.len() > 0)
        {
            return Err("Offline episode file is missing or empty".into());
        }
        return Ok(path);
    }
    let root = directory.join("mediacache");
    let index: Value =
        serde_json::from_slice(&std::fs::read(root.join("index.json")).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
    let entries = index["entries"]
        .as_object()
        .ok_or("Offline cache index is missing")?;
    for (key, value) in entries {
        if !key.starts_with(&format!("{}.", item.id))
            || response.is_some_and(|v| v["key"].as_str() != Some(key.as_str()))
            || item.raw["_offline"]["key"]
                .as_str()
                .is_some_and(|saved| saved != key)
        {
            continue;
        }
        let meta = &index["metas"][key];
        if meta["Encrypted"] != true
            || meta["Preview"] == true
            || meta["Format"].as_str().unwrap_or("").is_empty()
        {
            continue;
        }
        let Some(name) = value["file"].as_str() else {
            continue;
        };
        if name.contains(['/', '\\', ':']) || name == "." || name == ".." {
            continue;
        }
        let path = root.join(name);
        if let Ok(info) = path.metadata() {
            if info.is_file() && info.len() > 0 && Some(info.len()) == value["size"].as_u64() {
                return Ok(path);
            }
        }
    }
    Err("Offline audio or playback metadata is missing or incomplete".into())
}
pub fn library_page(items: &[Entry], selected: Option<&Entry>) -> crate::catalog::Page {
    use crate::catalog::{Page, Section};
    if let Some(selected) = selected.filter(|e| e.kind == "offline_collection") {
        let rows = items
            .iter()
            .filter(|item| match selected.id.as_str() {
                "__songs" | "__episodes" => {
                    let kind = if selected.id == "__songs" {
                        "track"
                    } else {
                        "episode"
                    };
                    item.kind == kind
                        && (item.raw["_download_single"] == true
                            || item.raw.get("_download_groups").is_none())
                }
                _ => item.raw["_download_groups"]
                    .as_array()
                    .is_some_and(|groups| {
                        groups.iter().any(|group| {
                            group["id"] == selected.raw["group_id"]
                                && group["kind"] == selected.raw["group_kind"]
                        })
                    }),
            })
            .cloned()
            .collect();
        return Page {
            title: selected.title.clone(),
            rows,
            ..Default::default()
        };
    }
    let mut cards = vec![
        Entry {
            id: "__songs".into(),
            title: "Songs".into(),
            kind: "offline_collection".into(),
            ..Default::default()
        },
        Entry {
            id: "__episodes".into(),
            title: "Episodes".into(),
            kind: "offline_collection".into(),
            ..Default::default()
        },
    ];
    for item in items {
        for group in item.raw["_download_groups"]
            .as_array()
            .into_iter()
            .flatten()
        {
            let id = group["id"].as_str().unwrap_or_default();
            let kind = group["kind"].as_str().unwrap_or_default();
            if id.is_empty()
                || cards
                    .iter()
                    .any(|card| card.raw["group_id"] == id && card.raw["group_kind"] == kind)
            {
                continue;
            }
            let mut entry = crate::catalog::entries(&serde_json::json!([group]), kind)
                .into_iter()
                .next()
                .unwrap_or_default();
            entry.id = format!("{kind}:{id}");
            entry.kind = "offline_collection".into();
            entry.raw = serde_json::json!({"group_id": id, "group_kind": kind});
            cards.push(entry);
        }
    }
    Page {
        title: "Downloads".into(),
        sections: vec![Section {
            title: "Library".into(),
            entries: cards,
            ..Default::default()
        }],
        ..Default::default()
    }
}

pub fn load_offline() -> Vec<Entry> {
    let Ok(directory) = crate::storage::data_dir() else {
        return Vec::new();
    };
    let items = std::fs::read(directory.join("offline.json"))
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
        .map(|value| crate::catalog::entries(&value, "track"))
        .unwrap_or_default();
    items
        .into_iter()
        .filter(|item| {
            let Ok(path) = verified_path(&directory, item, None) else {
                return false;
            };
            // Legacy rows must pass the engine index/file checks; new rows also detect corruption.
            item.raw.get("_offline").is_none_or(|expected| {
                receipt(&path).is_ok_and(|actual| {
                    actual["size"] == expected["size"] && actual["sha256"] == expected["sha256"]
                })
            })
        })
        .collect()
}
fn persist_offline(items: &[Entry]) -> Result<(), String> {
    persist_offline_at(&crate::storage::data_dir()?, items)
}

fn persist_offline_at(directory: &std::path::Path, items: &[Entry]) -> Result<(), String> {
    std::fs::create_dir_all(&directory).map_err(|e| e.to_string())?;
    let data: Vec<_> = items
        .iter()
        .map(|item| {
            let mut wire = item.wire();
            if let Some(receipt) = item.raw.get("_offline") {
                wire["_offline"] = receipt.clone();
            }
            for key in ["_download_groups", "_download_single"] {
                if let Some(value) = item.raw.get(key) {
                    wire[key] = value.clone();
                }
            }
            wire
        })
        .collect();
    write_json(&directory.join("offline.json"), &data)
}

// Retry only lower explicit qualities. The default-quality API may silently
// select a higher quality than the user requested and hides the original error.
fn download_at_quality(
    quality: usize,
    cancelled: &AtomicBool,
    mut invoke: impl FnMut(usize) -> Result<Value, String>,
) -> Result<Value, String> {
    let mut errors = Vec::new();
    for quality in (0..=quality.min(2)).rev() {
        if cancelled.load(Ordering::Relaxed) {
            return Err("Download cancelled".into());
        }
        match invoke(quality) {
            Ok(value)
                if matches!(value["status"].as_str(), Some("downloaded" | "cached"))
                    && value["key"].as_str().is_some_and(|key| !key.is_empty()) =>
            {
                return Ok(value)
            }
            Ok(_) => errors.push(format!(
                "quality {quality}: engine did not confirm an offline file"
            )),
            Err(error) => errors.push(format!("quality {quality}: {error}")),
        }
    }
    Err(errors.join("; "))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn temporary_directory() -> std::path::PathBuf {
        let directory = std::env::temp_dir().join(format!(
            "deeznats-download-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&directory).unwrap();
        directory
    }

    #[test]
    fn maintenance_preserves_referenced_and_recent_files() {
        let directory = temporary_directory();
        let root = directory.join("mediacache");
        std::fs::create_dir_all(&root).unwrap();
        for name in [
            "saved",
            "stream",
            "old-orphan",
            "recent-orphan",
            "index.json.tmp",
        ] {
            std::fs::write(root.join(name), b"audio").unwrap();
        }
        let old = std::time::SystemTime::now() - std::time::Duration::from_secs(172800);
        for name in ["saved", "stream", "old-orphan", "index.json.tmp"] {
            std::fs::File::options()
                .write(true)
                .open(root.join(name))
                .unwrap()
                .set_modified(old)
                .unwrap();
        }
        write_json(
            &root.join("index.json"),
            &json!({"entries": {
                "1.FLAC": {"file":"saved", "pinned":true},
                "2.FLAC": {"file":"stream"}
            }}),
        )
        .unwrap();
        std::fs::write(root.join("index.json.tmp"), b"pending").unwrap();
        std::fs::File::options()
            .write(true)
            .open(root.join("index.json.tmp"))
            .unwrap()
            .set_modified(old)
            .unwrap();
        let report = maintenance_at(&directory, false).unwrap();
        assert!(report.contains("Removed 0 files"));
        assert!(root.join("old-orphan").exists());
        let report = maintenance_at(&directory, true).unwrap();
        assert!(report.contains("Removed 1 files"));
        assert!(!root.join("old-orphan").exists());
        for name in ["saved", "stream", "recent-orphan", "index.json.tmp"] {
            assert!(root.join(name).exists());
        }
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn maintenance_reports_missing_saved_audio() {
        let directory = temporary_directory();
        let root = directory.join("mediacache");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("saved"), b"audio").unwrap();
        write_json(
            &root.join("index.json"),
            &json!({"entries": {
            "1.FLAC": {"file":"saved", "size":5, "pinned":true}
        }, "metas":{"1.FLAC":{"Encrypted":true,"Preview":false,"Format":"FLAC"}}}),
        )
        .unwrap();
        write_json(&directory.join("offline.json"), &json!([
            {"id":"1","kind":"track","title":"Present"},
            {"id":"2","kind":"track","title":"Missing", "_download_groups":[{"id":"9","kind":"album"}]}
        ])).unwrap();
        assert!(maintenance_at(&directory, false)
            .unwrap()
            .contains("missing: 1"));
        let rows = load_offline_at(&directory).unwrap();
        assert!(verified_path(&directory, &rows[0], None).is_ok());
        assert!(verified_path(&directory, &rows[1], None).is_err());
        assert_eq!(rows[1].raw["_download_groups"][0]["id"], "9");
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn maintenance_rejects_invalid_index_before_deleting() {
        let directory = temporary_directory();
        let root = directory.join("mediacache");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("orphan"), b"audio").unwrap();
        write_json(
            &root.join("index.json"),
            &json!({"entries":{"1":{"file":"../outside"}}}),
        )
        .unwrap();
        assert!(maintenance_at(&directory, true).is_err());
        assert!(root.join("orphan").exists());
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn remove_all_deletes_downloads_and_jobs_but_preserves_streaming_cache() {
        let directory = temporary_directory();
        let cache = directory.join("mediacache");
        std::fs::create_dir_all(&cache).unwrap();
        for file in ["download", "interrupted", "stream"] {
            std::fs::write(cache.join(file), b"audio").unwrap();
        }
        let index = json!({"entries": {
            "42.FLAC": {"file":"download","size":5,"pinned":true},
            "43.FLAC": {"file":"interrupted","size":5,"pinned":true},
            "99.FLAC": {"file":"stream","size":5}
        }, "metas":{"42.FLAC":{},"43.FLAC":{},"99.FLAC":{}}});
        write_json(&cache.join("index.json"), &index).unwrap();
        // Invalid/incomplete offline rows must still be deletable.
        write_json(
            &directory.join("offline.json"),
            &json!([{"id":"42","kind":"track"}]),
        )
        .unwrap();
        let jobs = directory.join("download-jobs");
        std::fs::create_dir_all(&jobs).unwrap();
        std::fs::write(jobs.join("7.json"), b"{}").unwrap();
        std::fs::write(jobs.join("notes.txt"), b"keep").unwrap();
        let mut removed = Vec::new();
        remove_all_at(&directory, |id| {
            removed.push(id.to_owned());
            Ok(())
        })
        .unwrap();
        assert_eq!(removed, ["42", "43"]);
        assert!(!cache.join("download").exists());
        assert!(!cache.join("interrupted").exists());
        assert!(cache.join("stream").exists());
        assert!(!jobs.join("7.json").exists());
        assert!(jobs.join("notes.txt").exists());
        let index: Value =
            serde_json::from_slice(&std::fs::read(cache.join("index.json")).unwrap()).unwrap();
        assert_eq!(index["entries"].as_object().unwrap().len(), 1);
        assert_eq!(index["metas"].as_object().unwrap().len(), 1);
        let offline: Value =
            serde_json::from_slice(&std::fs::read(directory.join("offline.json")).unwrap())
                .unwrap();
        assert_eq!(offline, json!([]));
        remove_all_at(&directory, |_| panic!("nothing left to delete")).unwrap();
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn failed_removal_retains_record_and_rejects_paths_outside_cache() {
        let directory = temporary_directory();
        let cache = directory.join("mediacache");
        std::fs::create_dir_all(&cache).unwrap();
        std::fs::write(cache.join("body"), b"audio").unwrap();
        let mut index = json!({"entries":{"42.FLAC":{"file":"body","pinned":true}},"metas":{}});
        write_json(&cache.join("index.json"), &index).unwrap();
        write_json(
            &directory.join("offline.json"),
            &json!([{"id":"42","kind":"track"}]),
        )
        .unwrap();
        assert!(remove_all_at(&directory, |_| Err("file busy".into()))
            .unwrap_err()
            .contains("file busy"));
        assert!(cache.join("body").exists());
        let offline: Value =
            serde_json::from_slice(&std::fs::read(directory.join("offline.json")).unwrap())
                .unwrap();
        assert_eq!(offline.as_array().unwrap().len(), 1);
        std::fs::write(directory.join("outside"), b"keep").unwrap();
        index["entries"]["42.FLAC"]["file"] = json!("../outside");
        write_json(&cache.join("index.json"), &index).unwrap();
        assert!(remove_all_at(&directory, |_| panic!("unsafe path reached engine")).is_err());
        assert_eq!(std::fs::read(directory.join("outside")).unwrap(), b"keep");
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn remove_all_deletes_downloaded_episodes_from_configured_directory() {
        let directory = temporary_directory();
        let destination = directory.join("downloads");
        let podcasts = destination.join("Podcasts");
        std::fs::create_dir_all(&podcasts).unwrap();
        std::fs::write(podcasts.join("42.mp3"), b"audio").unwrap();
        std::fs::write(podcasts.join("99.mp3"), b"keep").unwrap();
        std::fs::write(
            directory.join("download-dir.txt"),
            destination.to_string_lossy().as_bytes(),
        )
        .unwrap();
        write_json(
            &directory.join("offline.json"),
            &json!([{"id":"42","kind":"episode"}]),
        )
        .unwrap();
        remove_all_at(&directory, |_| panic!("episodes do not use media cache")).unwrap();
        assert!(!podcasts.join("42.mp3").exists());
        assert!(podcasts.join("99.mp3").exists());
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn cache_requires_complete_body_and_playback_metadata() {
        let directory = temporary_directory();
        let cache = directory.join("mediacache");
        std::fs::create_dir_all(&cache).unwrap();
        let item = Entry {
            id: "42".into(),
            kind: "track".into(),
            ..Default::default()
        };
        let body = cache.join("body-1");
        std::fs::write(&body, b"complete audio").unwrap();
        let mut index = json!({"entries":{"42.MP3_128":{"file":"body-1","size":14}},
            "metas":{"42.MP3_128":{"Encrypted":true,"Format":"MP3_128","MediaTrackID":"42"}}});
        write_json(&cache.join("index.json"), &index).unwrap();
        assert_eq!(verified_path(&directory, &item, None).unwrap(), body);
        // The engine only sets MediaTrackID when Deezer substitutes another recording.
        // Ordinary tracks decrypt using the requested track ID.
        index["metas"]["42.MP3_128"]["MediaTrackID"] = json!("");
        write_json(&cache.join("index.json"), &index).unwrap();
        assert_eq!(verified_path(&directory, &item, None).unwrap(), body);
        let mut selected = item.clone();
        selected.raw = json!({"_offline":{"key":"42.FLAC"}});
        assert!(verified_path(&directory, &selected, None).is_err());
        assert!(verified_path(&directory, &item, Some(&json!({"key":"99.MP3_128"}))).is_err());
        std::fs::write(&body, b"partial").unwrap();
        assert!(verified_path(&directory, &item, None).is_err());
        std::fs::write(&body, b"complete audio").unwrap();
        index["metas"] = json!({});
        write_json(&cache.join("index.json"), &index).unwrap();
        assert!(verified_path(&directory, &item, None).is_err());
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn receipt_detects_same_length_corruption_and_deleted_files() {
        let directory = temporary_directory();
        let path = directory.join("audio");
        std::fs::write(&path, b"audio").unwrap();
        let saved = receipt(&path).unwrap();
        std::fs::write(&path, b"xxxxx").unwrap();
        assert_ne!(saved, receipt(&path).unwrap());
        std::fs::remove_file(&path).unwrap();
        assert!(receipt(&path).is_err());
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn durable_job_replacement_retains_only_unfinished_tracks() {
        let directory = temporary_directory();
        let path = directory.join("job.json");
        let mut job = Job {
            id: 7,
            quality: 2,
            items: vec![
                Entry {
                    id: "42".into(),
                    ..Default::default()
                },
                Entry {
                    id: "43".into(),
                    ..Default::default()
                },
            ],
        };
        write_json(&path, &job).unwrap();
        // A restart before completion restores both tracks.
        let recovered: Job = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(recovered.items.len(), 2);
        job.items.remove(0);
        write_json(&path, &job).unwrap();
        let recovered: Job = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(recovered.items[0].id, "43");
        assert_eq!(recovered.quality, 2);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn cancellation_interrupts_retry_backoff() {
        let cancelled = AtomicBool::new(false);
        let mut calls = 0;
        let result = retry(&cancelled, || {
            calls += 1;
            cancelled.store(true, Ordering::Relaxed);
            Err("network failure".into())
        });
        assert!(result.unwrap_err().contains("cancelled"));
        assert_eq!(calls, 1);
    }

    #[test]
    fn fallback_is_descending_and_reports_only_confirmed_downloads() {
        let cancelled = AtomicBool::new(false);
        let mut tried = Vec::new();
        let value = download_at_quality(2, &cancelled, |quality| {
            tried.push(quality);
            match quality {
                2 => Err("Unavailable".into()),
                1 => Ok(json!({})),
                _ => Ok(json!({"status":"downloaded", "key":"42.MP3_128"})),
            }
        })
        .unwrap();
        assert_eq!(tried, [2, 1, 0]);
        assert_eq!(value["key"], "42.MP3_128");
        let error = download_at_quality(0, &cancelled, |_| Err("No network".into())).unwrap_err();
        assert!(error.contains("No network"));
    }

    #[test]
    fn cancellation_prevents_fallback_requests() {
        let cancelled = AtomicBool::new(false);
        let mut count = 0;
        let result = download_at_quality(2, &cancelled, |_| {
            count += 1;
            cancelled.store(true, Ordering::Relaxed);
            Err("Unavailable".into())
        });
        assert!(result.unwrap_err().contains("cancelled"));
        assert_eq!(count, 1);
    }
}

#[cfg(test)]
mod library_tests {
    use super::*;
    #[test]
    fn removing_collection_preserves_shared_downloads() {
        let dir = std::env::temp_dir().join(format!("deeznats-selection-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let first = serde_json::json!({"id":"42", "kind":"album", "name":"Album"});
        let second = serde_json::json!({"id":"43", "kind":"playlist", "name":"Playlist"});
        let item = Entry {
            id: "1".into(),
            title: "Song".into(),
            kind: "track".into(),
            raw: serde_json::json!({"_download_groups":[first, second]}),
            ..Default::default()
        };
        persist_offline_at(&dir, &[item]).unwrap();
        let selected = Entry {
            id: "album:42".into(),
            kind: "offline_collection".into(),
            raw: serde_json::json!({"group_id":"42", "group_kind":"album"}),
            ..Default::default()
        };
        remove_selected_at(&dir, &selected, None, |_| panic!("shared file must remain")).unwrap();
        let document: Value =
            serde_json::from_slice(&std::fs::read(dir.join("offline.json")).unwrap()).unwrap();
        assert_eq!(document[0]["_download_groups"].as_array().unwrap().len(), 1);
        assert_eq!(document[0]["_download_groups"][0]["id"], "43");
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn library_separates_individual_items_and_collection_members() {
        let group = serde_json::json!({"id":"42", "kind":"album", "name":"Album"});
        let items = vec![
            Entry {
                id: "1".into(),
                kind: "track".into(),
                raw: serde_json::json!({"_download_groups":[group]}),
                ..Default::default()
            },
            Entry {
                id: "2".into(),
                kind: "episode".into(),
                raw: serde_json::json!({"_download_single":true}),
                ..Default::default()
            },
            Entry {
                id: "3".into(),
                kind: "track".into(),
                raw: serde_json::json!({}),
                ..Default::default()
            },
        ];
        let library = library_page(&items, None);
        let cards = &library.sections[0].entries;
        assert_eq!(
            cards.iter().map(|e| e.title.as_str()).collect::<Vec<_>>(),
            ["Songs", "Episodes", "Album"]
        );
        assert_eq!(library_page(&items, Some(&cards[0])).rows[0].id, "3");
        assert_eq!(library_page(&items, Some(&cards[1])).rows[0].id, "2");
        assert_eq!(library_page(&items, Some(&cards[2])).rows[0].id, "1");
    }
}
