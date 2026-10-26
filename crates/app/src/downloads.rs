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
    Track(u64, usize, usize, Box<Entry>, Result<Value, String>),
    Finished(u64, bool),
}
pub struct Worker {
    pub requests: mpsc::SyncSender<Request>,
    pub updates: mpsc::Receiver<Event>,
}
impl Worker {
    pub fn start() -> Self {
        let (requests, rx) = mpsc::sync_channel::<Request>(8);
        let (tx, updates) = mpsc::sync_channel(8);
        std::thread::spawn(move || {
            while let Ok(request) = rx.recv() {
                let total = request.items.len();
                for (index, item) in request.items.into_iter().enumerate() {
                    if request.cancelled.load(Ordering::Relaxed) {
                        break;
                    }
                    let result = if item.kind == "episode" {
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
                    };
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
                let _ = tx.send(Event::Finished(
                    request.id,
                    request.cancelled.load(Ordering::Relaxed),
                ));
            }
        });
        Self { requests, updates }
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
pub fn load_offline() -> Vec<Entry> {
    let Ok(directory) = crate::storage::data_dir() else {
        return Vec::new();
    };
    std::fs::read(directory.join("offline.json"))
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
        .map(|value| crate::catalog::entries(&value, "track"))
        .unwrap_or_default()
}
pub fn save_offline(items: &[Entry]) {
    if let Ok(directory) = crate::storage::data_dir() {
        let _ = std::fs::create_dir_all(&directory);
        let data: Vec<_> = items.iter().map(Entry::wire).collect();
        if let Ok(bytes) = serde_json::to_vec(&data) {
            let temporary = directory.join("offline.json.tmp");
            if std::fs::write(&temporary, bytes).is_ok() {
                // Rename commits a complete document; a failed write keeps the old library.
                let _ = std::fs::rename(&temporary, directory.join("offline.json"));
            }
        }
    }
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
