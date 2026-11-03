//! YouTube Music adapter (GPL-3.0-or-later).
mod auth;
mod catalog;
mod playback;
mod potoken;
mod potoken_webview;
pub(crate) use catalog::{
    collection_tracks, edit as edit_playlist, invoke, menu_playlists, playlist_privacy, process,
    saved_library_entries,
};
pub(crate) use playback::Player;
use serde_json::{json, Value};
use std::sync::{Arc, Mutex, OnceLock};
use ytmusic::YtMusic;
pub(crate) mod routing;
// Serializes publication, cookie writes, and logout so an old request cannot
// recreate credentials after sign-out or overwrite a newer account.
static SESSION: Mutex<u64> = Mutex::new(0);
static NEXT_SESSION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
fn session_id() -> u64 {
    NEXT_SESSION.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}
fn protected_session(api: YtMusic, id: u64, authuser: usize) -> YtMusic {
    api.cookie_sink(Arc::new(move |store| {
        persist_if_current(&SESSION, id, || {
            if crate::session::provider_save(
                "youtube",
                &json!({"cookie_store":store,"authuser":authuser}).to_string(),
            )
            .is_err()
            {
                eprintln!("YouTube: could not persist protected session cookies");
            }
        });
    }))
}
fn persist_if_current(active: &Mutex<u64>, id: u64, save: impl FnOnce()) {
    let Ok(active) = active.lock() else {
        return;
    };
    if *active == id {
        save();
    }
}
fn encoded_session(api: &YtMusic, authuser: usize) -> Result<String, String> {
    let store = api
        .cookie_store()
        .map_err(|_| "Cannot encode YouTube session")?;
    Ok(json!({"cookie_store":store,"authuser":authuser}).to_string())
}
static API: Mutex<Option<Arc<YtMusic>>> = Mutex::new(None);
pub(crate) fn runtime() -> &'static tokio::runtime::Runtime {
    static RT: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RT.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("YouTube runtime")
    })
}
pub(crate) fn api() -> Result<Arc<YtMusic>, String> {
    let mut guard = API.lock().map_err(|_| "YouTube session lock unavailable")?;
    if let Some(api) = guard.as_ref() {
        return Ok(api.clone());
    }
    let saved = crate::session::provider_saved("youtube")
        .map_err(|_| "Sign in to YouTube Music to continue.")?;
    let value: Value = serde_json::from_str(&saved).map_err(|_| "Invalid YouTube session")?;
    let authuser = value["authuser"].as_u64().unwrap_or(0) as usize;
    // Legacy header-only sessions are upgraded on the next response/rotation.
    let mut api = YtMusic::with_cookies(value["cookies"].as_str().unwrap_or("")).as_user(authuser);
    if let Some(store) = value["cookie_store"].as_str() {
        api = api
            .restore_cookie_store(store)
            .map_err(|_| "Invalid YouTube cookie store")?;
    }
    let id = session_id();
    let api = protected_session(api, id, authuser);
    if !api.is_authenticated() {
        return Err("Sign in to YouTube Music to continue.".into());
    }
    let api = Arc::new(api.mint_po_tokens(Arc::new(potoken::Minter)));
    *SESSION
        .lock()
        .map_err(|_| "YouTube session lock unavailable")? = id;
    rotate(&api, true);
    *guard = Some(api.clone());
    Ok(api)
}
pub(crate) fn login(cancel: &std::sync::atomic::AtomicBool) -> Result<Value, String> {
    let cookies = crate::youtube_webview::run(cancel)?;
    if cancel.load(std::sync::atomic::Ordering::Relaxed) {
        return Err("YouTube sign-in cancelled".into());
    }
    login_cookies_with_cancel(&cookies, cancel)
}
pub(crate) fn login_cookies_with_cancel(
    cookies: &str,
    cancel: &std::sync::atomic::AtomicBool,
) -> Result<Value, String> {
    let cookies = auth::header(cookies).map_err(|e| e.to_string())?;
    let id = session_id();
    let api = Arc::new(
        protected_session(YtMusic::with_cookies(&cookies), id, 0)
            .mint_po_tokens(Arc::new(potoken::Minter)),
    );
    let profile = runtime()
        .block_on(api.profile())
        .map_err(|e| format!("YouTube sign-in: {e}"))?;
    if cancel.load(std::sync::atomic::Ordering::Relaxed) {
        return Err("YouTube sign-in cancelled".into());
    }
    let mut guard = API.lock().map_err(|_| "YouTube session lock unavailable")?;
    let mut active = SESSION
        .lock()
        .map_err(|_| "YouTube session lock unavailable")?;
    crate::session::provider_save("youtube", &encoded_session(&api, 0)?)?;
    *active = id;
    drop(active);
    rotate(&api, false);
    if let Ok(mut history) = HISTORY.lock() {
        history.clear();
    }
    *guard = Some(api);
    Ok(json!({"id":profile.email.unwrap_or_else(|| profile.name.clone()),"name":profile.name}))
}
pub(crate) fn logout() -> Result<(), String> {
    let mut guard = API.lock().map_err(|_| "YouTube session lock unavailable")?;
    let mut active = SESSION
        .lock()
        .map_err(|_| "YouTube session lock unavailable")?;
    crate::session::provider_remove("youtube")?;
    *active = 0;
    *guard = None;
    if let Ok(mut h) = HISTORY.lock() {
        h.clear();
    }
    Ok(())
}
pub(crate) fn account() -> Result<Value, String> {
    let api = api()?;
    let p = runtime()
        .block_on(api.profile())
        .map_err(|e| e.to_string())?;
    Ok(json!({"id":p.email.unwrap_or_else(|| p.name.clone()),"name":p.name}))
}

static HISTORY: Mutex<Vec<crate::catalog::Entry>> = Mutex::new(Vec::new());
pub(super) fn remember(entry: &crate::catalog::Entry) {
    if let Ok(mut history) = HISTORY.lock() {
        history.retain(|t| t.id != entry.id);
        history.insert(0, entry.clone());
        history.truncate(100);
    }
}
pub(super) fn history() -> Vec<crate::catalog::Entry> {
    HISTORY.lock().map(|h| h.clone()).unwrap_or_default()
}

static REFRESHING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub(crate) fn refreshing() -> bool {
    REFRESHING.load(std::sync::atomic::Ordering::Relaxed)
}

fn rotate(api: &Arc<YtMusic>, restored: bool) {
    if !api.is_authenticated() {
        return;
    }
    let weak = Arc::downgrade(api);
    runtime().spawn(async move {
        let mut rotated = if restored {
            None
        } else {
            Some(std::time::SystemTime::now())
        };
        loop {
            if weak.strong_count() == 0 {
                break;
            }
            // Short checks use wall time so suspend/resume cannot postpone renewal.
            if rotation_due(rotated, std::time::SystemTime::now()) {
                let Some(api) = weak.upgrade() else {
                    break;
                };
                REFRESHING.store(true, std::sync::atomic::Ordering::Relaxed);
                match tokio::time::timeout(std::time::Duration::from_secs(30), api.rotate_cookies())
                    .await
                {
                    Ok(Ok(())) => rotated = Some(std::time::SystemTime::now()),
                    _ => eprintln!("YouTube: cookie renewal failed; retrying in 30 seconds"),
                }
                REFRESHING.store(false, std::sync::atomic::Ordering::Relaxed);
            }
            tokio::time::sleep(std::time::Duration::from_secs(30)).await;
        }
    });
}

fn rotation_due(last: Option<std::time::SystemTime>, now: std::time::SystemTime) -> bool {
    last.is_none_or(|last| {
        now.duration_since(last).map_or(true, |elapsed| {
            elapsed >= std::time::Duration::from_secs(600)
        })
    })
}
#[cfg(test)]
mod session_tests {
    use super::*;
    #[test]
    #[ignore = "requires a saved YouTube session and network access"]
    fn live_saved_account_and_library() {
        let api = api().expect("saved YouTube session unavailable");
        runtime().block_on(async {
            let library =
                tokio::time::timeout(std::time::Duration::from_secs(30), api.library_playlists())
                    .await;
            let profile =
                tokio::time::timeout(std::time::Duration::from_secs(30), api.profile()).await;
            assert!(
                library.as_ref().is_ok_and(|value| value.is_ok()),
                "library request failed: {library:?}"
            );
            assert!(
                profile.as_ref().is_ok_and(|value| value.is_ok()),
                "account request failed"
            );
        });
    }
    #[test]
    fn stale_requests_cannot_write_after_logout_or_account_switch() {
        let active = Mutex::new(7);
        let writes = std::cell::Cell::new(0);
        persist_if_current(&active, 7, || writes.set(writes.get() + 1));
        *active.lock().unwrap() = 0;
        persist_if_current(&active, 7, || writes.set(writes.get() + 1));
        *active.lock().unwrap() = 8;
        persist_if_current(&active, 7, || writes.set(writes.get() + 1));
        persist_if_current(&active, 8, || writes.set(writes.get() + 1));
        assert_eq!(writes.get(), 2);
    }
    #[test]
    fn renewal_covers_restore_suspend_and_clock_rollback() {
        use std::time::{Duration, UNIX_EPOCH};
        let last = UNIX_EPOCH + Duration::from_secs(1000);
        assert!(rotation_due(None, last));
        assert!(!rotation_due(Some(last), last + Duration::from_secs(599)));
        assert!(rotation_due(Some(last), last + Duration::from_secs(600)));
        assert!(rotation_due(Some(last), last + Duration::from_secs(3600)));
        assert!(rotation_due(Some(last), UNIX_EPOCH));
    }
}
