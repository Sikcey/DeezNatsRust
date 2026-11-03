//! Browser token protocol adapted to the app's WebView2 host.
use std::{
    collections::HashMap,
    sync::{Mutex, OnceLock},
    time::{Duration, Instant},
};
pub(super) struct Minter;
#[async_trait::async_trait]
impl ytmusic::Minter for Minter {
    async fn mint(&self, binding: &str) -> Option<String> {
        static CACHE: OnceLock<Mutex<HashMap<String, (Instant, String)>>> = OnceLock::new();
        let cache = CACHE.get_or_init(Mutex::default);
        if let Some((at, token)) = cache.lock().ok()?.get(binding) {
            if at.elapsed() < Duration::from_secs(6 * 3600) {
                return Some(token.clone());
            }
        }
        let binding = binding.to_owned();
        let requested = binding.clone();
        let value = tokio::task::spawn_blocking(move || {
            super::potoken_webview::run(&requested, &std::sync::atomic::AtomicBool::new(false))
        })
        .await
        .ok()?
        .ok()?;
        let (returned, token) = value.split_once(':')?;
        let returned = url::form_urlencoded::parse(format!("v={returned}").as_bytes())
            .next()?
            .1
            .into_owned();
        if returned != binding || token.starts_with('!') || token.is_empty() {
            return None;
        }
        cache
            .lock()
            .ok()?
            .insert(binding, (Instant::now(), token.into()));
        Some(token.into())
    }
}
