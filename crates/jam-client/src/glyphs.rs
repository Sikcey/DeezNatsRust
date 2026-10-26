//! Bounded background lyric rasterization. Window paints never wait for blur.
use crate::{
    model::{Color, ImageData, Rect},
    render::Renderer,
};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    sync::{
        atomic::{AtomicU64, Ordering},
        mpsc, Arc, Mutex, OnceLock,
    },
    time::{Duration, Instant},
};

#[derive(Clone)]
pub struct Job {
    pub key: String,
    pub text: String,
    pub rect: Rect,
    pub size: f32,
    pub color: Color,
    pub blur: bool,
}
impl Job {
    pub fn new(text: &str, rect: Rect, size: f32, color: Color, blur: bool) -> Self {
        Self {
            key: format!(
                "::lyric:{text}:{size}:{}:{}:{color:?}:{blur}",
                rect.w.round(),
                rect.h.round()
            ),
            text: text.into(),
            rect,
            size,
            color,
            blur,
        }
    }
}
#[derive(Default)]
struct Cache {
    ready: HashMap<String, Arc<ImageData>>,
    order: VecDeque<String>,
    pending: HashSet<String>,
    failed: HashMap<String, Instant>,
    bytes: usize,
}
impl Cache {
    fn touch(&mut self, key: &str) {
        self.order.retain(|k| k != key);
        self.order.push_back(key.into());
    }
    fn get(&mut self, key: &str) -> Option<Arc<ImageData>> {
        let image = self.ready.get(key)?.clone();
        self.touch(key);
        Some(image)
    }
    fn insert(&mut self, key: String, image: Arc<ImageData>) {
        self.pending.remove(&key);
        self.failed.remove(&key);
        if let Some(old) = self.ready.remove(&key) {
            self.bytes = self.bytes.saturating_sub(old.rgba.len());
        }
        self.bytes += image.rgba.len();
        self.ready.insert(key.clone(), image);
        self.touch(&key);
        while self.ready.len() > 32 || self.bytes > 24 * 1024 * 1024 {
            let Some(old) = self.order.pop_front() else {
                break;
            };
            if let Some(image) = self.ready.remove(&old) {
                self.bytes = self.bytes.saturating_sub(image.rgba.len());
            }
        }
    }
}
struct Worker {
    cache: Arc<Mutex<Cache>>,
    jobs: mpsc::SyncSender<Job>,
}
static WORKER: OnceLock<Worker> = OnceLock::new();
static REVISION: AtomicU64 = AtomicU64::new(0);
fn worker() -> &'static Worker {
    WORKER.get_or_init(|| {
        let cache = Arc::new(Mutex::new(Cache::default()));
        let (tx, rx) = mpsc::sync_channel::<Job>(32);
        let target = cache.clone();
        std::thread::spawn(move || {
            unsafe {
                let _ = windows::Win32::System::Com::CoInitializeEx(
                    None,
                    windows::Win32::System::Com::COINIT_MULTITHREADED,
                );
            }
            while let Ok(job) = rx.recv() {
                let image = Renderer::rasterize_lyric(&job);
                let mut cache = target.lock().unwrap();
                match image {
                    Ok(image) => cache.insert(job.key, image),
                    Err(_) => {
                        cache.pending.remove(&job.key);
                        cache
                            .failed
                            .insert(job.key, Instant::now() + Duration::from_secs(3));
                    }
                }
                REVISION.fetch_add(1, Ordering::Relaxed);
            }
        });
        Worker { cache, jobs: tx }
    })
}
pub fn revision() -> u64 {
    REVISION.load(Ordering::Relaxed)
}
pub fn pending() -> usize {
    WORKER
        .get()
        .map(|w| w.cache.lock().unwrap().pending.len())
        .unwrap_or(0)
}
pub fn lookup(job: Job, synchronous: bool) -> Option<Arc<ImageData>> {
    let worker = worker();
    {
        let mut cache = worker.cache.lock().unwrap();
        if let Some(image) = cache.get(&job.key) {
            return Some(image);
        }
        if cache.pending.contains(&job.key)
            || cache
                .failed
                .get(&job.key)
                .is_some_and(|at| Instant::now() < *at)
        {
            return None;
        }
        if !synchronous {
            cache.pending.insert(job.key.clone());
        }
    }
    if synchronous {
        let image = Renderer::rasterize_lyric(&job).ok()?;
        worker.cache.lock().unwrap().insert(job.key, image.clone());
        return Some(image);
    }
    if let Err(error) = worker.jobs.try_send(job) {
        let (mpsc::TrySendError::Full(job) | mpsc::TrySendError::Disconnected(job)) = error;
        worker.cache.lock().unwrap().pending.remove(&job.key);
    }
    None
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn adding_the_next_line_does_not_discard_visible_lyric_masks() {
        let mut c = Cache::default();
        for i in 0..32 {
            c.insert(
                i.to_string(),
                Arc::new(ImageData {
                    width: 1,
                    height: 1,
                    rgba: vec![0; 4],
                }),
            );
        }
        assert!(c.get("0").is_some());
        c.insert(
            "next".into(),
            Arc::new(ImageData {
                width: 1,
                height: 1,
                rgba: vec![0; 4],
            }),
        );
        assert!(c.ready.contains_key("0"));
        assert!(!c.ready.contains_key("1"));
        assert_eq!(c.ready.len(), 32);
    }
}
