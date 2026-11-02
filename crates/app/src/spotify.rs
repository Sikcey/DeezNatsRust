//! Experimental Spotify backend. OAuth/cookie credentials never enter Soloist arguments.
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    io::{self, BufRead},
    sync::{
        atomic::{AtomicBool, Ordering},
        Mutex,
    },
};
use url::Url;

const REDIRECT: &str = "http://127.0.0.1:43821/spotify/callback";
pub(crate) mod api;
pub(crate) mod capabilities;
mod catalog;
mod directory_cache;
pub(crate) mod flow;
pub(crate) mod jam;
mod jam_auth;
pub(crate) mod playback;
pub(crate) mod remote;
pub(crate) mod style_mix;
pub(crate) mod web;
mod web_catalog;
pub(crate) mod web_devices;
pub(crate) use catalog::invoke as catalog_invoke;
mod process;
pub(crate) mod service;
pub(crate) mod setup;
pub(crate) mod soloist;
pub(crate) mod state;
pub(crate) mod wizard;
static AUTH: Mutex<()> = Mutex::new(());
#[derive(Clone, Debug)]
pub(crate) enum AuthAction {
    Login,
    Logout,
    Refresh,
}
/// Window callers poll this receiver; browser/network work stays off the UI thread.
pub(crate) fn auth_async(
    action: AuthAction,
) -> Result<std::sync::mpsc::Receiver<Result<(), String>>, String> {
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    std::thread::Builder::new()
        .name("spotify-auth".into())
        .spawn(move || {
            let result = match action {
                AuthAction::Login => login_web(&AtomicBool::new(false)),
                AuthAction::Logout => logout(),
                AuthAction::Refresh => refresh(),
            };
            let _ = tx.send(result);
        })
        .map_err(|_| "Could not start Spotify authorization worker")?;
    Ok(rx)
}
/// Configuration is not proof that a saved grant is still authorized.
pub(crate) fn api_configured() -> bool {
    crate::session::provider_exists("spotify") || web::has_session()
}
pub(crate) const WARNING: &str = "Experimental Spotify playback uses a separately installed Spotify Soloist. Generate your own Soloist API key; never share it. Premium is required for initial setup. Pair once using the Spotify app on the same network. API authorization and web-session capture do not pair Soloist. Soloist builds expire after 90 days; install updates from Spotify. Soloist plays through Windows' default audio output, outside the Deezer EQ, crossfade and audio pipeline. Do not redistribute Soloist binaries.";

#[derive(Serialize, Deserialize)]
struct Credentials {
    client_id: String,
    access_token: String,
    refresh_token: String,
    expires_at: u64,
    sp_dc: String,
}
fn random() -> Result<String, String> {
    let mut bytes = [0u8; 32];
    let status = unsafe {
        windows_sys::Win32::Security::Cryptography::BCryptGenRandom(
            std::ptr::null_mut(),
            bytes.as_mut_ptr(),
            bytes.len() as u32,
            windows_sys::Win32::Security::Cryptography::BCRYPT_USE_SYSTEM_PREFERRED_RNG,
        )
    };
    if status < 0 {
        return Err("Could not generate Spotify authorization challenge".into());
    }
    Ok(URL_SAFE_NO_PAD.encode(bytes))
}
pub(crate) fn is_callback(uri: &str, redirect: &str) -> bool {
    let (Ok(actual), Ok(expected)) = (Url::parse(uri), Url::parse(redirect)) else {
        return false;
    };
    actual.scheme() == expected.scheme()
        && actual.host_str() == expected.host_str()
        && actual.port_or_known_default() == expected.port_or_known_default()
        && actual.path() == expected.path()
        && actual.username().is_empty()
        && actual.password().is_none()
        && actual.fragment().is_none()
}
fn authorization_code(callback: &str, state: &str) -> Result<String, String> {
    if !is_callback(callback, REDIRECT) {
        return Err("Unexpected Spotify callback".into());
    }
    let url = Url::parse(callback).map_err(|_| "Invalid Spotify callback")?;
    let values: Vec<_> = url.query_pairs().collect();
    let single = |key: &str| -> Result<String, String> {
        let found: Vec<_> = values.iter().filter(|(k, _)| k == key).collect();
        if found.len() != 1 {
            return Err("Missing or ambiguous Spotify callback parameter".into());
        }
        Ok(found[0].1.to_string())
    };
    if single("state")? != state {
        return Err("Spotify authorization state did not match".into());
    }
    if values.iter().any(|(k, _)| k == "error") {
        return Err("Spotify authorization was declined".into());
    }
    let code = single("code")?;
    if code.is_empty() {
        return Err("Spotify returned an empty authorization code".into());
    }
    Ok(code)
}
fn token_request(fields: &[(&str, &str)]) -> Result<serde_json::Value, String> {
    let body = url::form_urlencoded::Serializer::new(String::new())
        .extend_pairs(fields.iter().copied())
        .finish();
    let response = crate::lyrics::http::response(
        "POST",
        "https://accounts.spotify.com/api/token",
        &[("Content-Type", "application/x-www-form-urlencoded")],
        body.as_bytes(),
    )
    .map_err(|_| "Spotify API token request failed or timed out")?;
    match response.status {
        200 => {}
        400 | 401 => {
            return Err("Spotify authorization expired or was revoked; sign in again".into())
        }
        429 => {
            return Err(format!(
                "Spotify token requests are rate limited; retry after {} seconds",
                response.retry_after.unwrap_or(5)
            ))
        }
        _ => return Err("Spotify token service is unavailable; retry later".into()),
    }
    let bytes = response.body;
    let value: serde_json::Value =
        serde_json::from_slice(&bytes).map_err(|_| "Invalid Spotify token response")?;
    if value["access_token"]
        .as_str()
        .filter(|s| !s.is_empty())
        .is_none()
    {
        return Err("Spotify did not grant an API token".into());
    }
    Ok(value)
}
fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
fn save(credentials: &Credentials) -> Result<(), String> {
    crate::session::provider_save(
        "spotify",
        &serde_json::to_string(credentials).map_err(|_| "Could not encode Spotify session")?,
    )
}
fn login() -> Result<(), String> {
    let client =
        configured_client_id().ok_or("Enter your Spotify Web API application client ID")?;
    if client.is_empty() {
        return Err("Spotify client ID is empty".into());
    }
    login_with(&client, &AtomicBool::new(false))
}
/// Run on a worker thread. The cancellation flag is also checked before storing either grant.
pub(crate) fn login_with(client: &str, cancel: &AtomicBool) -> Result<(), String> {
    if client.is_empty() {
        return Err("Spotify client ID is empty".into());
    }
    let state = random()?;
    let verifier = random()?;
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    let mut url = Url::parse("https://accounts.spotify.com/authorize").unwrap();
    url.query_pairs_mut().extend_pairs([
        ("client_id", client),
        ("response_type", "code"),
        ("redirect_uri", REDIRECT),
        ("code_challenge_method", "S256"),
        ("code_challenge", challenge.as_str()),
        ("state", state.as_str()),
        (
            "scope",
            "user-read-private user-top-read user-read-playback-state user-modify-playback-state user-read-recently-played user-library-read user-library-modify user-follow-read user-follow-modify playlist-read-private playlist-read-collaborative playlist-modify-private playlist-modify-public",
        ),
    ]);
    let result = crate::spotify_webview::run(url.as_str(), REDIRECT, cancel)?;
    let captured: serde_json::Value =
        serde_json::from_str(&result).map_err(|_| "Invalid Spotify sign-in result")?;
    let code = authorization_code(
        captured["callback"]
            .as_str()
            .ok_or("Missing API authorization")?,
        &state,
    )?;
    let cookie = captured["sp_dc"]
        .as_str()
        .filter(|s| !s.is_empty())
        .ok_or("Missing Spotify web session")?;
    let token = token_request(&[
        ("grant_type", "authorization_code"),
        ("code", &code),
        ("redirect_uri", REDIRECT),
        ("client_id", &client),
        ("code_verifier", &verifier),
    ])?;
    let credentials = Credentials {
        client_id: client.to_owned(),
        access_token: token["access_token"].as_str().unwrap().into(),
        refresh_token: token["refresh_token"]
            .as_str()
            .filter(|s| !s.is_empty())
            .ok_or("Spotify did not grant a refresh token")?
            .into(),
        expires_at: token_expiry(&token, now())?,
        sp_dc: cookie.into(),
    };
    let _guard = AUTH
        .lock()
        .map_err(|_| "Spotify credential storage unavailable")?;
    if cancel.load(Ordering::Relaxed) {
        return Err("Spotify sign-in cancelled".into());
    }
    let result = save(&credentials);
    drop(_guard);
    if result.is_ok() {
        catalog::reset();
    }
    result
}
pub(crate) fn refresh() -> Result<(), String> {
    if web::has_session() {
        web::refresh()
    } else {
        access_token(true).map(|_| ())
    }
}
/// Cookie-only authorization; no developer application ID required.
pub(crate) fn login_web(cancel: &AtomicBool) -> Result<(), String> {
    let result = crate::spotify_webview::login_web(cancel)?;
    let value: serde_json::Value =
        serde_json::from_str(&result).map_err(|_| "Invalid Spotify sign-in result")?;
    let cookie = value["sp_dc"]
        .as_str()
        .ok_or("Spotify did not provide a web session")?;
    if cancel.load(Ordering::Relaxed) {
        return Err("Spotify sign-in cancelled".into());
    }
    save_web_cookie(cookie)?;
    web::refresh()?;
    // Session capture is independent from catalog request integration.
    let result = Ok(());
    catalog::reset();
    result
}
pub(crate) fn configured_client_id() -> Option<String> {
    std::env::var("DEEZNATS_SPOTIFY_CLIENT_ID")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .or_else(|| {
            crate::session::provider_saved("spotify")
                .ok()
                .and_then(|s| serde_json::from_str::<Credentials>(&s).ok())
                .map(|c| c.client_id)
        })
}
pub(crate) fn catalog_account() -> Result<serde_json::Value, String> {
    if web::has_session() {
        return web::account();
    }
    api::Client::saved().profile().map_err(|e| e.to_string())
}
/// Reload under one lock so concurrent clients cannot overwrite rotated refresh tokens.
fn access_token(force: bool) -> Result<String, String> {
    let _guard = AUTH
        .lock()
        .map_err(|_| "Spotify credential storage unavailable")?;
    let mut credentials: Credentials =
        serde_json::from_str(&crate::session::provider_saved("spotify")?)
            .map_err(|_| "Saved Spotify session is invalid")?;
    if force || credentials.expires_at <= now().saturating_add(60) {
        let _refresh = web::begin_refresh();
        let token = token_request(&[
            ("grant_type", "refresh_token"),
            ("client_id", &credentials.client_id),
            ("refresh_token", &credentials.refresh_token),
        ])?;
        update_token(&mut credentials, &token, now())?;
        save(&credentials)?;
    }
    Ok(credentials.access_token)
}
fn token_expiry(token: &serde_json::Value, now: u64) -> Result<u64, String> {
    let lifetime = token["expires_in"]
        .as_u64()
        .filter(|n| (1..=86400).contains(n))
        .ok_or("Invalid Spotify token expiry")?;
    now.checked_add(lifetime)
        .ok_or_else(|| "Invalid Spotify token expiry".into())
}
fn update_token(
    credentials: &mut Credentials,
    token: &serde_json::Value,
    now: u64,
) -> Result<(), String> {
    let expiry = token_expiry(token, now)?;
    let access = token["access_token"]
        .as_str()
        .filter(|s| !s.is_empty())
        .ok_or("Spotify did not grant an API token")?;
    credentials.access_token = access.into();
    credentials.expires_at = expiry;
    if let Some(value) = token["refresh_token"].as_str().filter(|s| !s.is_empty()) {
        credentials.refresh_token = value.into();
    }
    Ok(())
}
/// Preserve the same-window web grant without claiming API authorization or Soloist pairing.
pub(crate) fn save_web_cookie(cookie: &str) -> Result<(), String> {
    if !crate::spotify_webview::valid_spotify_cookie("sp_dc", ".spotify.com", true, cookie) {
        return Err("Invalid Spotify web session".into());
    }
    let _guard = AUTH
        .lock()
        .map_err(|_| "Spotify credential storage unavailable")?;
    crate::session::provider_save("spotify-web", cookie)?;
    if let Ok(saved) = crate::session::provider_saved("spotify") {
        if let Ok(mut credentials) = serde_json::from_str::<Credentials>(&saved) {
            credentials.sp_dc = cookie.into();
            save(&credentials)?;
        }
    }
    drop(_guard);
    web::reset();
    Ok(())
}
pub(crate) fn logout() -> Result<(), String> {
    web::reset();
    let _guard = AUTH
        .lock()
        .map_err(|_| "Spotify credential storage unavailable")?;
    let api = crate::session::provider_remove("spotify");
    let web = crate::session::provider_remove("spotify-web");
    let web_api = crate::session::provider_remove("spotify-web-api");
    drop(_guard);
    catalog::reset();
    api.and(web).and(web_api)
}
pub(crate) fn run_cli() -> Result<(), String> {
    let args: Vec<_> = std::env::args().collect();
    let action = args
        .iter()
        .position(|s| s == "--spotify")
        .and_then(|i| args.get(i + 1))
        .map(String::as_str)
        .unwrap_or("help");
    match action {
        "jam-doctor" => {
            let report = match jam::run(jam::Operation::Refresh) {
                Ok(session) => {
                    serde_json::json!({"ok":true,"active":session.as_ref().is_some_and(|s|s.active),
                    "owner":session.as_ref().is_some_and(|s|s.owner),"inviteAvailable":session.as_ref().is_some_and(|s|s.invite.is_some()),
                    "participants":session.as_ref().map_or(0,|s|s.members.len())})
                }
                Err(error) => serde_json::json!({"ok":false,"error":error}),
            };
            println!("{report}");
            return Ok(());
        }
        "jam-lifecycle-check" => {
            let result = (|| -> Result<serde_json::Value, String> {
                if jam::run(jam::Operation::Refresh)?
                    .as_ref()
                    .is_some_and(|s| s.active)
                {
                    return Err(
                        "An existing Jam is present; lifecycle check refused to change it.".into(),
                    );
                }
                let session = jam::run(jam::Operation::Start)
                    .map_err(|e| format!("Create: {e}"))?
                    .ok_or("No Jam was created")?;
                let invite = jam::run(jam::Operation::Invite(session.id.clone()));
                let ended = jam::run(jam::Operation::End(session.id.clone()))
                    .map_err(|e| format!("End: {e}"))?;
                Ok(
                    serde_json::json!({"created":session.active && session.owner,"inviteAvailable":invite.as_ref().ok().and_then(|s|s.as_ref()).is_some_and(|s|s.invite.is_some()),
                    "inviteError":invite.err(),"participants":session.members.len(),"ended":!ended.as_ref().is_some_and(|s|s.active)}),
                )
            })();
            println!(
                "{}",
                result.unwrap_or_else(|error| serde_json::json!({"error":error}))
            );
            return Ok(());
        }
        "catalog-layout-doctor" => {
            let mut report = serde_json::Map::new();
            if args.iter().any(|arg| arg == "--refresh-session") {
                report.insert(
                    "sessionRefresh".into(),
                    match web::refresh() {
                        Ok(()) => serde_json::json!({"ok":true}),
                        Err(error) => serde_json::json!({"error":error}),
                    },
                );
            }
            report.insert(
                "profileQuery".into(),
                match web::account() {
                    Ok(_) => serde_json::json!({"ok":true}),
                    Err(error) => serde_json::json!({"error":error}),
                },
            );
            if args.iter().any(|arg| arg == "--browse-shape") {
                report.insert(
                    "browseShape".into(),
                    web::browse().unwrap_or_else(|error| serde_json::json!({"error":error})),
                );
            }
            if args.iter().any(|arg| arg == "--directory-probe") {
                report.insert(
                    "browseOperations".into(),
                    serde_json::json!(web::browse_operations().unwrap_or_default()),
                );
                for (label, uri, operation) in [
                    (
                        "allMusicSection",
                        "spotify:section:0JQ5DAwD41iRZZRVB8eoeO",
                        "browseSection",
                    ),
                    (
                        "rockDirectory",
                        "spotify:page:0JQ5DAqbMKFDXXwE9BDJAr",
                        "browsePage",
                    ),
                ] {
                    report.insert(label.into(), web::query(operation, serde_json::json!({
                "uri":uri,"pagination":{"offset":0,"limit":100},
                "pagePagination":{"offset":0,"limit":100},"sectionPagination":{"offset":0,"limit":100},
                "browseEndUserIntegration":"INTEGRATION_WEB_PLAYER","includeEpisodeContentRatingsV2":false
            })).unwrap_or_else(|error| serde_json::json!({"error":error})));
                }
                report.insert("musicDirectory".into(), web::query("browsePage", serde_json::json!({
            "uri":"spotify:page:0JQ5DAqbMKFSi39LMRT0Cy", "pagePagination":{"offset":0,"limit":100},
            "sectionPagination":{"offset":0,"limit":100},"browseEndUserIntegration":"INTEGRATION_WEB_PLAYER",
            "includeEpisodeContentRatingsV2":false
        })).unwrap_or_else(|error| serde_json::json!({"error":error})));
                report.insert(
                    "publicCategories".into(),
                    web::rest_get("https://api.spotify.com/v1/browse/categories?limit=50&offset=0")
                        .unwrap_or_else(|error| serde_json::json!({"error":error})),
                );
            }
            if args.iter().any(|arg| arg == "--genre-probe") {
                for (label, uri) in [
                    ("dance", "spotify:page:0JQ5DAqbMKFHOzuVTgTizF"),
                    ("metal", "spotify:page:0JQ5DAqbMKFDkd668ypn6O"),
                    ("jazz", "spotify:page:0JQ5DAqbMKFAJ5xb0fwo9m"),
                    ("hiphop", "spotify:page:0JQ5DAqbMKFQ00XGBls6ym"),
                    ("pop", "spotify:page:0JQ5DAqbMKFEC4WFtoNRpw"),
                    ("latin", "spotify:page:0JQ5DAqbMKFxXaXKP7zcDp"),
                ] {
                    report.insert(label.into(), web::query("browsePage", serde_json::json!({
                "uri":uri,"pagePagination":{"offset":0,"limit":100},"sectionPagination":{"offset":0,"limit":100},
                "browseEndUserIntegration":"INTEGRATION_WEB_PLAYER","includeEpisodeContentRatingsV2":false
            })).unwrap_or_else(|error| serde_json::json!({"error":error})));
                }
            }
            if args.iter().any(|arg| arg == "--expanded-directory") {
                report.insert(
                    "expandedDirectory".into(),
                    crate::catalog::spotify_discovery_report()
                        .unwrap_or_else(|error| serde_json::json!({"error":error})),
                );
            }
            if args.iter().any(|arg| arg == "--lazy-probe") {
                let result = (|| -> Result<serde_json::Value, String> {
                    let value = catalog_invoke(
                        "SPCatalogChannelJSON",
                        &[serde_json::json!("spotify:page:0JQ5DAqbMKFDXXwE9BDJAr")],
                    )?;
                    let section = value["sections"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .find(|s| {
                            s["total"].as_u64().unwrap_or(0) > s["offset"].as_u64().unwrap_or(0)
                        })
                        .ok_or("No paginated section")?;
                    let more = catalog_invoke(
                        "SPCatalogSectionJSON",
                        &[
                            section["uri"].clone(),
                            section["offset"].clone(),
                            serde_json::json!(8),
                        ],
                    )?;
                    Ok(
                        serde_json::json!({"title":section["title"],"initial":section["items"].as_array().map_or(0,Vec::len),
                "nextBatch":more["items"].as_array().map_or(0,Vec::len),"offset":more["offset"],"total":more["total"]}),
                    )
                })();
                report.insert(
                    "lazySection".into(),
                    result.unwrap_or_else(|error| serde_json::json!({"error":error})),
                );
            }
            for name in ["SPCatalogHomeJSON", "SPCatalogDiscoveryJSON"] {
                let value = match web_catalog::invoke(name, &[]).unwrap() {
                    Ok(value) => {
                        serde_json::json!({"sections":value["sections"].as_array().into_iter().flatten().map(|s|
                serde_json::json!({"title":s["title"],"items":s["items"].as_array().into_iter().flatten().map(|item|
                    serde_json::json!({"type":item["type"],"name":item["name"],"uri":item["uri"],"images":item["images"]})).collect::<Vec<_>>()})).collect::<Vec<_>>()})
                    }
                    Err(error) => serde_json::json!({"error":error}),
                };
                report.insert(name.into(), value);
            }
            if args.iter().any(|arg| arg == "--library") {
                for module in 0..5 {
                    report.insert(format!("library:{module}"), match catalog_invoke("DZHomeModuleJSON", &[serde_json::json!(module)]) {
                Ok(value) => serde_json::json!({"items":value["items"].as_array().map_or(0,Vec::len)}),
                Err(error) => serde_json::json!({"error":error}),
            });
                }
                report.insert("genrePage".into(), match catalog_invoke("SPCatalogChannelJSON", &[serde_json::json!("spotify:page:0JQ5DAqbMKFEC4WFtoNRpw")]) {
            Ok(value) => serde_json::json!({"sections":value["sections"].as_array().map_or(0,Vec::len)}),
            Err(error) => serde_json::json!({"error":error}),
        });
                for (name, uri) in [
                    ("editorial", "spotify:page:0JQ5DAqbMKFSi39LMRT0Cy"),
                    ("newReleases", "spotify:page:0JQ5DAqbMKFz6FAsUtgAab"),
                    ("charts", "spotify:page:0JQ5DAudkNjCgYMM0TZXDw"),
                ] {
                    report.insert(name.into(), match catalog_invoke("SPCatalogChannelJSON", &[serde_json::json!(uri)]) {
                Ok(value) => serde_json::json!({"shelves":value["sections"].as_array().into_iter().flatten()
                    .map(|s| serde_json::json!({"title":s["title"],"types":s["items"].as_array().into_iter().flatten()
                        .map(|item| item["type"].clone()).collect::<Vec<_>>()})).collect::<Vec<_>>()}),
                Err(error) => serde_json::json!({"error":error}),
            });
                }
            }
            let path = args
                .iter()
                .position(|s| s == "--spotify")
                .and_then(|i| args.get(i + 2))
                .ok_or("Provide a local report path")?;
            std::fs::write(
                path,
                serde_json::to_vec_pretty(&report).map_err(|_| "Could not encode report")?,
            )
            .map_err(|_| "Could not write report")?;
            Ok(())
        }
        "login-web" => login_web(&AtomicBool::new(false)),
        "devices-doctor" => {
            let devices = web_devices::devices()?;
            println!(
                "Spotify web Connect devices: {}",
                devices.as_array().map_or(0, Vec::len)
            );
            println!(
                "Spotify web active playback state available: {}",
                web_devices::snapshot()?.is_some()
            );
            Ok(())
        }
        "lyrics-doctor" => {
            let saved = catalog_invoke("DZFavoritesJSON", &[])?;
            let tracks = crate::catalog::entries(&saved, "track");
            for track in tracks.into_iter().take(3) {
                match crate::lyrics::fetch_spotify_lyrics(&track, &track.id, 1) {
                    Ok((provider, lines)) if !lines.is_empty() => {
                        println!(
                            "Spotify native lyrics: provider={provider}, lines={}",
                            lines.len()
                        );
                        return Ok(());
                    }
                    Ok(_) => println!("Spotify native lyrics: no lines for sample"),
                    Err(error) => println!("Spotify native lyrics: {error}"),
                }
            }
            Err("No native lyrics available for the sampled tracks".into())
        }
        "web-doctor" => {
            println!("Spotify web saved session: {}", web::has_session());
            web::account()?;
            println!("Spotify web authenticated token/client-token ready; profile query succeeded");
            let result = web::query(
                "home",
                serde_json::json!({"homeEndUserIntegration":"INTEGRATION_WEB_PLAYER","timeZone":"Europe/Istanbul","sp_t":"","facet":"","sectionItemsLimit":10,"includeEpisodeContentRatingsV2":false}),
            )?;
            println!(
                "Spotify web home query succeeded; sections: {}",
                result
                    .pointer("/data/home/sectionContainer/sections/items")
                    .and_then(|v| v.as_array())
                    .map_or(0, |v| v.len())
            );
            let mut failures = 0;
            let mut sample_track = None;
            let mut sample_playlist = None;
            for name in [
                "SPCatalogDiscoveryJSON",
                "DZFavoritesJSON",
                "DZFavoriteAlbumsJSON",
                "DZPlaylistsJSON",
                "DZHistoryRecentJSON",
                "DZFlowConfigsJSON",
            ] {
                match web_catalog::invoke(name, &[]).unwrap_or_else(|| Err("No web adapter".into()))
                {
                    Ok(value) => {
                        let items = crate::catalog::entries(
                            &value,
                            if name == "DZPlaylistsJSON" {
                                "playlist"
                            } else {
                                "track"
                            },
                        );
                        if name == "DZFavoritesJSON" {
                            sample_track = items.into_iter().next();
                        } else if name == "DZPlaylistsJSON" {
                            sample_playlist = items.into_iter().next();
                        }
                        println!("Spotify web {name}: OK");
                    }
                    Err(error) => {
                        failures += 1;
                        println!("Spotify web {name}: {error}");
                    }
                }
            }
            let mut checks = Vec::new();
            if let Some(track) = sample_track {
                checks.push(("DZTrackMetadataJSON", vec![serde_json::json!(track.id)]));
                if !track.artist_id.is_empty() {
                    checks.push((
                        "DZArtistProfileJSON",
                        vec![serde_json::json!(track.artist_id)],
                    ));
                }
                if !track.album_id.is_empty() {
                    checks.push(("DZAlbumTracksJSON", vec![serde_json::json!(track.album_id)]));
                }
            }
            if let Some(playlist) = sample_playlist {
                checks.push(("DZPlaylistTracksJSON", vec![serde_json::json!(playlist.id)]));
            }
            for (name, args) in checks {
                match web_catalog::invoke(name, &args)
                    .unwrap_or_else(|| Err("No web adapter".into()))
                {
                    Ok(_) => println!("Spotify web {name}: OK"),
                    Err(error) => {
                        failures += 1;
                        println!("Spotify web {name}: {error}");
                    }
                }
            }
            if failures > 0 {
                return Err(format!("Spotify read-only web checks failed: {failures}"));
            }
            Ok(())
        }
        "login" => {
            login()?;
            println!("Spotify API authorization and web session saved. Soloist pairing remains separate.");
            Ok(())
        }
        "refresh" => refresh(),
        "logout" => logout(),
        "configure" => {
            let index = args.iter().position(|arg| arg == "--spotify").unwrap() + 2;
            let api_key = args
                .get(index)
                .cloned()
                .or_else(|| std::env::var("DEEZNATS_SOLOIST_API_KEY").ok())
                .or_else(|| soloist::Config::saved().ok().map(|c| c.api_key))
                .filter(|key| !key.trim().is_empty())
                .ok_or("Provide an API key: --spotify configure [API_KEY] [PATH], or set DEEZNATS_SOLOIST_API_KEY")?;
            let executable = match args.get(index + 1) {
                Some(p) => {
                    let path = std::path::PathBuf::from(p);
                    if !path.is_file() {
                        return Err(format!(
                            "Soloist executable not found at {}",
                            path.display()
                        ));
                    }
                    path
                }
                None => setup::preferred_executable()?,
            };
            soloist::Config {
                executable,
                api_key,
            }
            .save()?;
            println!("Soloist configuration saved with Windows user protection");
            Ok(())
        }
        "soloist-discover"
        | "soloist-import"
        | "soloist-check"
        | "soloist-activate"
        | "soloist-rollback"
        | "soloist-doctor"
        | "soloist-open-downloads"
        | "soloist-open-key" => setup_cli(action, &args),
        "forget-soloist-config" => crate::session::provider_remove("spotify-soloist"),
        "profile" | "search" | "playlists" | "saved-tracks" | "saved-albums" | "playlist"
        | "album" | "item" => catalog_cli(action, &args),
        "run" => {
            println!("{WARNING}\nSelect DeezNats in Spotify Connect to pair. Commands: play [spotify:URI], pause, next, prev, seek MS, volume 0-100, shuffle on/off, repeat off/context/track, status, now, queue [1-80], snapshot, add-to-queue URI, activate, deactivate, quit.");
            let mut player = soloist::Soloist::start(soloist::Config::load()?)?;
            for line in io::stdin().lock().lines() {
                let line = line.map_err(|_| "Could not read Spotify command")?;
                let args: Vec<_> = line.split_whitespace().collect();
                if args.as_slice() == ["quit"] {
                    break;
                }
                if args.as_slice() == ["snapshot"] {
                    match player.snapshot() {
                        Ok(snapshot) => println!(
                            "{}",
                            serde_json::to_string(&snapshot)
                                .map_err(|_| "Could not serialize Spotify state")?
                        ),
                        Err(error) => println!("{error}"),
                    }
                    continue;
                }
                match soloist::Control::parse(&args).and_then(|control| player.control(&control)) {
                    Ok(value) if !value.is_null() => println!("{value}"),
                    Ok(_) => {}
                    Err(error) => println!("{error}"),
                }
            }
            Ok(())
        }
        _ => {
            println!("{WARNING}\nUsage: --spotify login|refresh|logout|configure [API_KEY] [PATH]|forget-soloist-config|run|profile|search|playlists|saved-tracks|saved-albums|playlist|album|item|soloist-discover|soloist-import ZIP|soloist-check [EXE]|soloist-activate ID|soloist-rollback|soloist-doctor|soloist-open-downloads|soloist-open-key. See docs/SPOTIFY.md.");
            Ok(())
        }
    }
}
fn setup_cli(action: &str, args: &[String]) -> Result<(), String> {
    let index = args.iter().position(|arg| arg == "--spotify").unwrap() + 2;
    let argument = || {
        args.get(index)
            .map(String::as_str)
            .ok_or_else(|| "Missing Soloist setup argument".to_owned())
    };
    let value = match action {
        "soloist-discover" => serde_json::to_value(setup::discover()?),
        "soloist-doctor" => serde_json::to_value(setup::doctor()?),
        "soloist-import" => {
            serde_json::to_value(setup::import_archive(std::path::Path::new(argument()?))?)
        }
        "soloist-check" => {
            let path = match args.get(index) {
                Some(path) => std::path::PathBuf::from(path),
                None => setup::preferred_executable()?,
            };
            serde_json::to_value(setup::check(&path)?)
        }
        "soloist-activate" => serde_json::to_value(setup::activate(argument()?)?),
        "soloist-rollback" => serde_json::to_value(setup::rollback()?),
        "soloist-open-downloads" => return setup::open_page(false),
        "soloist-open-key" => return setup::open_page(true),
        _ => unreachable!(),
    }
    .map_err(|_| "Could not encode Soloist setup report")?;
    println!("{value}");
    Ok(())
}
fn catalog_cli(action: &str, args: &[String]) -> Result<(), String> {
    let index = args.iter().position(|arg| arg == "--spotify").unwrap() + 2;
    let value = |offset: usize| {
        args.get(index + offset)
            .map(String::as_str)
            .ok_or_else(|| "Missing Spotify catalog argument".to_owned())
    };
    let mut client = api::Client::saved();
    let result = match action {
        "profile" => client.profile(),
        "search" => client
            .search(
                value(0)?,
                args.get(index + 1).map(String::as_str).unwrap_or("track"),
                0,
            )
            .and_then(|v| serde_json::to_value(v).map_err(|_| api::Error::InvalidResponse)),
        "item" => client
            .item(value(0)?, value(1)?)
            .and_then(|v| serde_json::to_value(v).map_err(|_| api::Error::InvalidResponse)),
        _ => {
            let mut page = match action {
                "playlists" => client.playlists(0),
                "saved-tracks" => client.saved_tracks(0),
                "saved-albums" => client.saved_albums(0),
                "playlist" => client.playlist_items(value(0)?, 0),
                "album" => client.album_tracks(value(0)?, 0),
                _ => unreachable!(),
            }
            .map_err(|e| e.to_string())?;
            println!(
                "{}",
                serde_json::to_string(&page).map_err(|_| "Could not encode Spotify page")?
            );
            // Explicit paging avoids fetching an unbounded library or hiding rate limits.
            for line in io::stdin().lock().lines() {
                if line.map_err(|_| "Could not read paging command")?.trim() != "next" {
                    break;
                }
                match client.next(&page).map_err(|e| e.to_string())? {
                    Some(next) => {
                        page = next;
                        println!(
                            "{}",
                            serde_json::to_string(&page)
                                .map_err(|_| "Could not encode Spotify page")?
                        );
                    }
                    None => break,
                }
            }
            return Ok(());
        }
    }
    .map_err(|e| e.to_string())?;
    println!("{result}");
    Ok(())
}
#[cfg(test)]
fn valid_control(args: &[&str]) -> bool {
    soloist::Control::parse(args).is_ok()
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn callback_requires_exact_origin_path_and_state() {
        assert_eq!(
            authorization_code(&format!("{REDIRECT}?state=expected&code=abc"), "expected").unwrap(),
            "abc"
        );
        for suffix in [
            "?state=wrong&code=abc",
            "?state=expected&code=a&code=b",
            "?state=expected&error=access_denied",
            "?state=expected&code=",
            "?state=expected&state=expected&code=a",
        ] {
            assert!(authorization_code(&format!("{REDIRECT}{suffix}"), "expected").is_err());
        }
        assert!(!is_callback(
            "http://127.0.0.1:43821/spotify/callback/evil?code=a",
            REDIRECT
        ));
        assert!(!is_callback(
            "http://127.0.0.1:43822/spotify/callback?code=a",
            REDIRECT
        ));
        assert!(!is_callback(
            "http://127.0.0.1.evil/spotify/callback",
            REDIRECT
        ));
    }
    #[test]
    fn playback_commands_are_bounded_and_do_not_accept_cli_flags() {
        assert!(valid_control(&[
            "play",
            "spotify:track:0123456789012345678901"
        ]));
        assert!(valid_control(&["volume", "100"]));
        assert!(!valid_control(&["volume", "101"]));
        assert!(!valid_control(&["play", "--api-key"]));
        assert!(!valid_control(&["seek", "-1"]));
        assert!(!valid_control(&[
            "play",
            "https://open.spotify.com/track/foo"
        ]));
    }
    #[test]
    fn refresh_rotation_preserves_cookie_and_validates_before_mutating() {
        let mut credentials = Credentials {
            client_id: "client".into(),
            access_token: "old-access".into(),
            refresh_token: "old-refresh".into(),
            expires_at: 1,
            sp_dc: "cookie".into(),
        };
        update_token(
            &mut credentials,
            &serde_json::json!({"access_token":"new-access","expires_in":3600}),
            100,
        )
        .unwrap();
        assert_eq!(credentials.refresh_token, "old-refresh");
        assert_eq!(credentials.sp_dc, "cookie");
        assert_eq!(credentials.expires_at, 3700);
        update_token(&mut credentials, &serde_json::json!({"access_token":"next-access","refresh_token":"rotated","expires_in":3600}),200).unwrap();
        assert_eq!(credentials.refresh_token, "rotated");
        assert!(update_token(
            &mut credentials,
            &serde_json::json!({"access_token":"bad","expires_in":u64::MAX}),
            200
        )
        .is_err());
        assert_eq!(credentials.access_token, "next-access");
    }
}
