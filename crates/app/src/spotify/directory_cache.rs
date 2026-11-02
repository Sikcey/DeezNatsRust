//! Spotify account namespace for the shared browse disk cache.
use serde_json::Value;
pub(super) fn peek(key: &str) -> Option<Value> {
    crate::catalog::disk_cache::peek("spotify", super::web::directory_cache_scope().ok()?, key)
}
pub(super) fn load(
    key: &str,
    fetch: impl FnOnce() -> Result<Value, String>,
) -> Result<Value, String> {
    match super::web::directory_cache_scope() {
        Ok(scope) => crate::catalog::disk_cache::load("spotify", scope, key, fetch),
        Err(_) => fetch(),
    }
}
