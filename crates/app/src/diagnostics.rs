//! Optional, bounded diagnostics for the dedicated profiling build.
use std::sync::{
    atomic::{AtomicU64, Ordering},
    mpsc, OnceLock,
};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

struct Event {
    name: &'static str,
    view: &'static str,
    micros: u64,
    time_ms: u128,
}
static DROPPED: AtomicU64 = AtomicU64::new(0);

fn sender() -> Option<&'static mpsc::SyncSender<Event>> {
    static SENDER: OnceLock<Option<mpsc::SyncSender<Event>>> = OnceLock::new();
    SENDER.get_or_init(|| {
        let path = std::env::var_os("DEEZNATS_TIMING_LOG")?;
        let (tx, rx) = mpsc::sync_channel::<Event>(8192);
        std::thread::Builder::new().name("diagnostics-writer".into()).spawn(move || {
            use std::io::Write;
            let Ok(file) = std::fs::OpenOptions::new().create(true).append(true).open(path) else { return };
            let mut file = std::io::BufWriter::new(file);
            while let Ok(event) = rx.recv() {
                let _ = writeln!(file, "{{\"time_unix_ms\":{},\"name\":\"{}\",\"view\":\"{}\",\"duration_us\":{},\"dropped_total\":{}}}", event.time_ms, event.name, event.view, event.micros, DROPPED.load(Ordering::Relaxed));
                let _ = file.flush();
            }
        }).ok()?;
        Some(tx)
    }).as_ref()
}

pub fn record(name: &'static str, view: &'static str, elapsed: Duration) {
    if let Some(sender) = sender() {
        let _ = sender
            .try_send(Event {
                name,
                view,
                micros: elapsed.as_micros().min(u64::MAX as u128) as u64,
                time_ms: SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis(),
            })
            .map_err(|_| DROPPED.fetch_add(1, Ordering::Relaxed));
    }
}

pub struct Span {
    name: &'static str,
    view: &'static str,
    start: Instant,
}
pub fn span(name: &'static str, view: &'static str) -> Span {
    Span {
        name,
        view,
        start: Instant::now(),
    }
}
impl Drop for Span {
    fn drop(&mut self) {
        record(self.name, self.view, self.start.elapsed());
    }
}
