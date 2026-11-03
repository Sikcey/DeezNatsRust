//! YouTube Music adapter (GPL-3.0-or-later).
mod auth;
mod catalog;
mod playback;
mod potoken;
mod potoken_webview;
pub(crate) use catalog::{invoke, process};
pub(crate) use playback::Player;
use serde_json::{json, Value};
use std::sync::{Arc, Mutex, OnceLock};
use ytmusic::YtMusic;
pub(crate) mod routing;
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
    let api = YtMusic::with_cookies(value["cookies"].as_str().ok_or("Missing YouTube cookies")?)
        .as_user(value["authuser"].as_u64().unwrap_or(0) as usize);
    if !api.is_authenticated() {
        return Err("Sign in to YouTube Music to continue.".into());
    }
    let api = Arc::new(api.mint_po_tokens(Arc::new(potoken::Minter)));
    rotate(&api);
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
    let api = Arc::new(YtMusic::with_cookies(&cookies).mint_po_tokens(Arc::new(potoken::Minter)));
    let profile = runtime()
        .block_on(api.profile())
        .map_err(|e| format!("YouTube sign-in: {e}"))?;
    if cancel.load(std::sync::atomic::Ordering::Relaxed) {
        return Err("YouTube sign-in cancelled".into());
    }
    crate::session::provider_save(
        "youtube",
        &json!({"cookies":cookies,"authuser":0}).to_string(),
    )?;
    rotate(&api);
    if let Ok(mut history) = HISTORY.lock() {
        history.clear();
    }
    *API.lock().map_err(|_| "YouTube session lock unavailable")? = Some(api);
    Ok(json!({"id":profile.email.unwrap_or_else(|| profile.name.clone()),"name":profile.name}))
}
pub(crate) fn logout() -> Result<(), String> {
    crate::session::provider_remove("youtube")?;
    *API.lock().map_err(|_| "YouTube session lock unavailable")? = None;
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

fn rotate(api: &Arc<YtMusic>) {
    if !api.is_authenticated() {
        return;
    }
    let weak = Arc::downgrade(api);
    runtime().spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(10 * 60)).await;
            let Some(api) = weak.upgrade() else {
                break;
            };
            REFRESHING.store(true, std::sync::atomic::Ordering::Relaxed);
            let _ = api.rotate_cookies().await;
            REFRESHING.store(false, std::sync::atomic::Ordering::Relaxed);
        }
    });
}
