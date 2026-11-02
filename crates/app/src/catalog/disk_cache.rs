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
