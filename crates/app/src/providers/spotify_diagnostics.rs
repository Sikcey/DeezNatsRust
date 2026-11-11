//! Content-free action diagnostics, enabled by default. The environment override
//! DEEZNATS_SPOTIFY_DIAGNOSTICS selects an absolute destination path.
use std::io::Write;
use std::sync::{mpsc, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};
static SENDER: OnceLock<mpsc::SyncSender<String>> = OnceLock::new();
pub(crate) fn record(event: &str, details: &str) {
    let sender = SENDER.get_or_init(|| {
        let (tx, rx) = mpsc::sync_channel::<String>(8192);
        let _ = std::thread::Builder::new().name("action-log".into()).spawn(move || {
            let path = std::env::var_os("DEEZNATS_SPOTIFY_DIAGNOSTICS").map(std::path::PathBuf::from)
                .or_else(|| crate::storage::data_dir().ok().map(|root| root.join("provider-actions.log")));
            let Some(path) = path.filter(|p| p.is_absolute()) else { return };
            for line in rx {
                if std::fs::metadata(&path).is_ok_and(|m| m.len() >= 8 * 1024 * 1024) {
                    let backup = path.with_extension("log.1");
                    let _ = std::fs::remove_file(&backup);
                    let _ = std::fs::rename(&path, backup);
                }
                if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
                    let _ = writeln!(file, "{line}");
                }
            }
        });
        tx
    });
    let timestamp = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis();
    let _ = sender.try_send(format!("{timestamp} pid={} thread={:?} {event} {}", std::process::id(), std::thread::current().id(), details.replace(['\r', '\n'], " ")));
}
pub(crate) fn error_kind(error: &str) -> &'static str {
    if error.contains("Web Player session required") {
        "no_web_session"
    } else if error.contains("request integration is pending") {
        "obsolete_adapter"
    } else if error.contains("malformed device state")
        || error.contains("Invalid Spotify Connect device state")
    {
        "malformed_device_state"
    } else if error.contains("hidden Spotify web session") || error.contains("Pair DeezNats") {
        "web_device_active"
    } else if error.contains("request failed (HTTP 403)") {
        "http_403"
    } else if error.contains("request failed (HTTP 404)") {
        "http_404"
    } else if error.contains("request failed (HTTP 500)") {
        "http_500"
    } else if error.contains("WebView2") {
        "webview_error"
    } else if error.contains("Connect sender") {
        "no_connect_sender"
    } else if error.contains("no active device") || error.contains("No active Spotify") {
        "no_active_device"
    } else if error.contains("active device changed") {
        "device_changed"
    } else if error.contains("queue revision") || error.contains("queue changed") {
        "queue_revision"
    } else if error.contains("authorization") || error.contains("expired") {
        "authorization"
    } else if error.contains("rate limit") {
        "rate_limited"
    } else if error.contains("timed out") || error.contains("timeout") {
        "timeout"
    } else {
        "other"
    }
}

/// A boundary timing event; completion means the call returned, not success.
pub(crate) struct Span { event: &'static str, operation: String, started: std::time::Instant }
impl Span {
    pub(crate) fn new(event: &'static str, operation: &str) -> Self {
        record(event, &format!("begin {operation}"));
        Self { event, operation: operation.into(), started: std::time::Instant::now() }
    }
}
impl Drop for Span {
    fn drop(&mut self) {
        record(self.event, &format!("returned {} elapsed_ms={}", self.operation, self.started.elapsed().as_millis()));
    }
}

/// Write synchronously: a panic in the Windows callback aborts before the log worker can flush.
pub(crate) fn install_panic_log() {
    let path=crate::storage::data_dir().ok().map(|root|root.join("panic.log"));
    let previous=std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        if let Some(path)=&path {
            if let Ok(mut file)=std::fs::OpenOptions::new().create(true).append(true).open(path) {
                let timestamp=SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis();
                let _=writeln!(file,"{timestamp} pid={} thread={:?} {info}\n{}",std::process::id(),std::thread::current().id(),std::backtrace::Backtrace::force_capture());
                let _=file.sync_all();
            }
        }
        previous(info);
    }));
}
