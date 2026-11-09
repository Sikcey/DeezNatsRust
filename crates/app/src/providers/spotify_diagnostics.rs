//! Opt-in, content-free Spotify diagnostics. Set DEEZNATS_SPOTIFY_DIAGNOSTICS
//! to an absolute log file path before starting the application.
use std::io::Write;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

static LOCK: Mutex<()> = Mutex::new(());

pub(crate) fn record(event: &str, details: &str) {
    let Some(path) = std::env::var_os("DEEZNATS_SPOTIFY_DIAGNOSTICS") else {
        return;
    };
    if !std::path::Path::new(&path).is_absolute() {
        return;
    }
    let Ok(_guard) = LOCK.lock() else { return };
    let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    else {
        return;
    };
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let _ = writeln!(file, "{timestamp} {event} {details}");
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
