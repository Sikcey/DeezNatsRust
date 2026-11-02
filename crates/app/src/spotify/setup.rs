//! User-local Soloist installation automation. No network downloads or shared API keys.
#[path = "setup_archive.rs"]
mod archive;
#[path = "setup_build.rs"]
mod build;
use super::soloist::Config;
pub(crate) use build::{strip_verbatim, Build};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    fs::{File, OpenOptions},
    io::{Read, Write},
    os::windows::fs::OpenOptionsExt,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

pub(crate) const DOWNLOADS: &str =
    "https://developer.spotify.com/documentation/soloist/reference/downloads-and-updates";
pub(crate) const KEY_PAGE: &str = "https://developer.spotify.com/dashboard/soloist";
const RECEIPT: &str = "deeznats-receipt.json";
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct Installation {
    pub id: String,
    pub executable: PathBuf,
    pub build: Build,
    pub imported_at: u64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
struct Receipt {
    id: String,
    relative_executable: PathBuf,
    build: Build,
    imported_at: u64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
struct Active {
    id: String,
    previous: Option<String>,
}
#[derive(Clone, Debug, Serialize)]
pub(crate) struct Candidate {
    pub executable: PathBuf,
    pub source: String,
    pub compatible: bool,
    pub error: Option<String>,
}
#[derive(Serialize)]
pub(crate) struct Doctor {
    pub candidates: Vec<Candidate>,
    pub managed_builds: Vec<Installation>,
    pub active: Option<Installation>,
    pub configuration_error: Option<String>,
    pub installation_error: Option<String>,
    pub api_key_configured: bool,
    pub downloads_url: &'static str,
    pub key_url: &'static str,
    pub notes: Vec<&'static str>,
}
fn root() -> Result<PathBuf, String> {
    Ok(crate::storage::data_dir()?.join("spotify-soloist-install"))
}
fn session_directory() -> Result<PathBuf, String> {
    Ok(crate::storage::data_dir()?.join("spotify-soloist"))
}
fn valid_id(id: &str) -> bool {
    id.len() == 64
        && id
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn bounded_json<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T, String> {
    let mut bytes = Vec::new();
    File::open(path)
        .map_err(|_| "Soloist installation metadata is unavailable")?
        .take(65537)
        .read_to_end(&mut bytes)
        .map_err(|_| "Could not read Soloist installation metadata")?;
    if bytes.len() > 65536 {
        return Err("Soloist installation metadata exceeds its size limit".into());
    }
    serde_json::from_slice(&bytes).map_err(|_| "Invalid Soloist installation metadata".into())
}
fn save_json(path: &Path, value: &impl Serialize) -> Result<(), String> {
    let bytes =
        serde_json::to_vec(value).map_err(|_| "Could not encode Soloist installation metadata")?;
    let temporary = path.with_extension(format!("{}.tmp", nonce()));
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&temporary)
        .map_err(|_| "Could not stage Soloist installation metadata")?;
    let result = (|| {
        file.write_all(&bytes)
            .and_then(|_| file.sync_all())
            .map_err(|_| "Could not write Soloist installation metadata")?;
        drop(file);
        std::fs::rename(&temporary, path)
            .map_err(|_| "Could not activate Soloist installation metadata".to_owned())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(temporary);
    }
    result
}
fn nonce() -> String {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    format!(
        "{}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    )
}
fn lock(directory: &Path, name: &str) -> Result<File, String> {
    std::fs::create_dir_all(directory)
        .map_err(|_| "Could not prepare Soloist management directory")?;
    OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .share_mode(0)
        .open(directory.join(name))
        .map_err(|_| "Soloist installation or playback is busy in another session".into())
}
fn load_at(root: &Path, id: &str) -> Result<Installation, String> {
    if !valid_id(id) {
        return Err("Invalid Soloist installation ID".into());
    }
    let directory = root.join("builds").join(id);
    let receipt: Receipt = bounded_json(&directory.join(RECEIPT))?;
    if receipt.id != id
        || archive::safe_relative(
            receipt
                .relative_executable
                .to_str()
                .ok_or("Invalid Soloist executable path")?,
        )? != receipt.relative_executable
        || !receipt
            .relative_executable
            .file_name()
            .is_some_and(|name| name.eq_ignore_ascii_case("soloist.exe"))
    {
        return Err("Invalid Soloist installation receipt".into());
    }
    let executable = directory.join("payload").join(&receipt.relative_executable);
    let executable = build::strip_verbatim(&executable);
    build::pe_x64(&executable)?;
    if build::digest(&executable)? != receipt.build.sha256 {
        return Err(
            "Installed Soloist executable changed since import; import the official archive again"
                .into(),
        );
    }
    let mut build = receipt.build;
    build.executable = executable.clone();
    build.at(build::now());
    Ok(Installation {
        id: id.into(),
        executable,
        build,
        imported_at: receipt.imported_at,
    })
}
fn active_at(root: &Path) -> Result<Option<Installation>, String> {
    if !root.join("active.json").exists() {
        return Ok(None);
    }
    let active: Active = bounded_json(&root.join("active.json"))?;
    load_at(root, &active.id).map(Some)
}
/// Check the official build at most once per UTC day. A pending build is
/// activated when the playback lock is available, without interrupting audio.
pub(crate) fn background_update() -> Result<(), String> {
    if Config::saved().is_err() || std::env::var_os("DEEZNATS_SOLOIST_PATH").is_some() {
        return Ok(());
    }
    let root = root()?;
    let _guard = lock(&root, "update-check.lock")?;
    let pending = root.join("pending-update.json");
    if let Ok(id) = bounded_json::<String>(&pending) {
        if activate(&id).is_ok() {
            let _ = std::fs::remove_file(&pending);
        }
    }
    let Some(current) = active_at(&root)? else {
        return Ok(());
    };
    // An explicit local executable overrides the managed build.
    if preferred_executable()? != current.executable {
        return Ok(());
    }
    let today = build::now() / 86_400;
    let marker = root.join("update-day.json");
    if bounded_json::<u64>(&marker).ok() == Some(today) {
        return Ok(());
    }
    save_json(&marker, &today)?;
    let path = root.join(format!("download-{}.zip", nonce()));
    let result = (|| {
        let mut file = File::create(&path).map_err(|_| "Could not prepare Soloist update")?;
        crate::updater::http::get(
            "https://soloist-builds.spotifycdn.com/soloist_release_windows_x86_64.zip",
            &mut file,
            512 * 1024 * 1024,
            |_| {},
        )?;
        drop(file);
        let new = import_archive(&path)?;
        if new.id != current.id
            && matches!((new.build.build_timestamp, current.build.build_timestamp), (Some(a), Some(b)) if a > b)
        {
            save_json(&pending, &new.id)?;
            if activate(&new.id).is_ok() {
                let _ = std::fs::remove_file(&pending);
            }
        }
        Ok(())
    })();
    let _ = std::fs::remove_file(path);
    result
}

pub(crate) fn active_executable() -> Result<Option<PathBuf>, String> {
    active_at(&root()?).map(|installation| installation.map(|i| i.executable))
}
/// Candidate discovery reads headers only. It never runs executables from PATH.
pub(crate) fn discover() -> Result<Vec<Candidate>, String> {
    let mut paths = Vec::new();
    if let Some(path) = std::env::var_os("DEEZNATS_SOLOIST_PATH") {
        paths.push((PathBuf::from(path), "environment"));
    }
    if let Some(installation) = active_at(&root()?)? {
        paths.push((installation.executable, "managed"));
    }
    if let Ok(config) = Config::saved() {
        paths.push((config.executable, "saved configuration"));
    }
    if let Ok(root) = root() {
        if let Ok(managed) = installations(&root) {
            for installation in managed {
                paths.push((installation.executable, "managed"));
            }
        }
    }
    if let Ok(current) = std::env::current_dir() {
        let path = current.join("soloist.exe");
        if path.is_file() {
            paths.push((path, "working directory"));
        }
    }
    if let Some(path) = std::env::var_os("PATH") {
        for directory in std::env::split_paths(&path)
            .filter(|p| p.is_absolute())
            .take(128)
        {
            let path = directory.join("soloist.exe");
            if path.is_file() {
                paths.push((path, "PATH"));
            }
        }
    }
    if let Ok(path) = std::env::current_exe() {
        if let Some(parent) = path.parent() {
            let path = parent.join("soloist.exe");
            if path.is_file() {
                paths.push((path, "application directory"));
            }
        }
    }
    let mut seen = HashSet::new();
    let mut result = Vec::new();
    for (path, source) in paths {
        let path = if path.is_absolute() {
            path
        } else {
            std::env::current_dir()
                .map_err(|_| "Could not resolve Soloist path")?
                .join(path)
        };
        let path = build::strip_verbatim(&path);
        let key = path.to_string_lossy().to_ascii_lowercase();
        if !seen.insert(key) {
            continue;
        }
        let error = build::pe_x64(&path).err();
        result.push(Candidate {
            executable: path,
            source: source.into(),
            compatible: error.is_none(),
            error,
        });
    }
    Ok(result)
}
struct Staging {
    path: PathBuf,
    parent: PathBuf,
}
impl Staging {
    fn new(root: &Path) -> Result<Self, String> {
        let parent = root.join("staging");
        std::fs::create_dir_all(&parent)
            .map_err(|_| "Could not prepare Soloist staging directory")?;
        let parent = parent
            .canonicalize()
            .map_err(|_| "Could not resolve Soloist staging directory")?;
        let path = parent.join(nonce());
        std::fs::create_dir(&path).map_err(|_| "Could not create Soloist staging directory")?;
        Ok(Self { path, parent })
    }
}
impl Drop for Staging {
    fn drop(&mut self) {
        // Remove only the unique directory this importer created, inside the checked staging root.
        if self.path.parent() == Some(self.parent.as_path())
            && self.path.canonicalize().ok().as_ref() == Some(&self.path)
            && std::fs::symlink_metadata(&self.path).is_ok_and(|m| !m.file_type().is_symlink())
        {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }
}
fn import_at(root: &Path, path: &Path) -> Result<Installation, String> {
    let _guard = lock(root, "setup.lock")?;
    let archive = archive::archive_file(path)?;
    let id = build::digest(path)?;
    if root.join("builds").join(&id).exists() {
        return load_at(root, &id);
    }
    let stage = Staging::new(root)?;
    let payload = stage.path.join("payload");
    std::fs::create_dir(&payload).map_err(|_| "Could not create Soloist package directory")?;
    let relative_executable = archive::extract(archive, &payload)?;
    let mut version = build::inspect(&payload.join(&relative_executable))?;
    if !version.usable() {
        return Err("Imported Soloist build is expired or has an invalid future build date".into());
    }
    let destination = root.join("builds").join(&id);
    version.executable =
        build::strip_verbatim(&destination.join("payload").join(&relative_executable));
    let receipt = Receipt {
        id: id.clone(),
        relative_executable,
        build: version,
        imported_at: build::now(),
    };
    save_json(&stage.path.join(RECEIPT), &receipt)?;
    std::fs::create_dir_all(root.join("builds"))
        .map_err(|_| "Could not create Soloist build directory")?;
    std::fs::rename(&stage.path, &destination)
        .map_err(|_| "Could not commit staged Soloist installation")?;
    load_at(root, &id)
}
pub(crate) fn import_archive(path: &Path) -> Result<Installation, String> {
    import_at(&root()?, path)
}
fn activate_at(root: &Path, session: &Path, id: &str) -> Result<Installation, String> {
    let _guard = lock(root, "setup.lock")?;
    let _playback_guard = lock(session, "deeznats.lock")?;
    let installation = load_at(root, id)?;
    if !installation.build.usable() {
        return Err("Cannot activate an expired Soloist build or an invalid build date".into());
    }
    let previous = if root.join("active.json").exists() {
        let active: Active = bounded_json(&root.join("active.json"))?;
        if active.id == id {
            return Ok(installation);
        }
        Some(active.id)
    } else {
        None
    };
    save_json(
        &root.join("active.json"),
        &Active {
            id: id.into(),
            previous,
        },
    )?;
    Ok(installation)
}
pub(crate) fn activate(id: &str) -> Result<Installation, String> {
    activate_at(&root()?, &session_directory()?, id)
}
pub(crate) fn rollback() -> Result<Installation, String> {
    rollback_at(&root()?, &session_directory()?)
}
fn rollback_at(root: &Path, session: &Path) -> Result<Installation, String> {
    let active: Active = bounded_json(&root.join("active.json"))?;
    activate_at(
        root,
        session,
        &active
            .previous
            .ok_or("No previous Soloist build is recorded")?,
    )
}
/// Resolve a selected/managed/discovered executable without requiring an API key.
pub(crate) fn preferred_executable() -> Result<PathBuf, String> {
    if let Some(path) = std::env::var_os("DEEZNATS_SOLOIST_PATH") {
        return Ok(build::strip_verbatim(&PathBuf::from(path)));
    }
    let root = root()?;
    let saved = Config::saved().ok().map(|config| config.executable);
    if let Some(path) = selected_at(&root, saved.as_deref())? {
        return Ok(path);
    }
    discover()?
        .into_iter()
        .find(|c| c.compatible)
        .map(|c| build::strip_verbatim(&c.executable))
        .ok_or_else(|| {
            "No Soloist executable found; import and activate the official ZIP or set DEEZNATS_SOLOIST_PATH"
                .into()
        })
}
fn selected_at(root: &Path, saved: Option<&Path>) -> Result<Option<PathBuf>, String> {
    // A manually selected executable is authoritative. Saved managed paths follow
    // active.json instead, so upgrades and rollback retain receipt verification.
    if let Some(path) = saved.map(build::strip_verbatim) {
        if !path.starts_with(build::strip_verbatim(&root.join("builds"))) && path.is_file() {
            return Ok(Some(path));
        }
    }
    active_at(root).map(|installation| installation.map(|i| i.executable))
}
pub(crate) fn check(path: &Path) -> Result<Build, String> {
    build::inspect(path)
}
fn installations(root: &Path) -> Result<Vec<Installation>, String> {
    let directory = root.join("builds");
    if !directory.exists() {
        return Ok(Vec::new());
    }
    let entries =
        std::fs::read_dir(directory).map_err(|_| "Could not list Soloist installations")?;
    let mut result = Vec::new();
    for entry in entries.take(128) {
        let entry = entry.map_err(|_| "Could not list Soloist installation")?;
        if let Some(id) = entry.file_name().to_str().filter(|id| valid_id(id)) {
            result.push(load_at(root, id)?);
        }
    }
    result.sort_by(|a, b| b.imported_at.cmp(&a.imported_at));
    Ok(result)
}
pub(crate) fn doctor() -> Result<Doctor, String> {
    let root = root()?;
    let saved = Config::saved().ok();
    let api_key_configured = std::env::var("DEEZNATS_SOLOIST_API_KEY")
        .ok()
        .is_some_and(|key| !key.trim().is_empty())
        || saved.as_ref().is_some_and(|c| !c.api_key.trim().is_empty());
    let configuration_error = Config::load().err();
    let active_result = active_at(&root);
    let list_result = installations(&root);
    let installation_error = active_result
        .as_ref()
        .err()
        .cloned()
        .or_else(|| list_result.as_ref().err().cloned());
    Ok(Doctor { candidates:discover().unwrap_or_default(),managed_builds:list_result.unwrap_or_default(),active:active_result.unwrap_or_default(),configuration_error,installation_error,api_key_configured,downloads_url:DOWNLOADS,key_url:KEY_PAGE,
        notes:vec!["Generate your own Soloist API key; Premium is required for initial setup.","Download Soloist yourself from Spotify, then import that local ZIP. DeezNats does not distribute it.","Import/check executes the selected local binary with --version. Only use a download you trust from Spotify.","SHA-256 detects changes after import; it does not authenticate the publisher.","Expiry is estimated only from an explicit build timestamp. Unknown formats remain unknown; Soloist exit code 10 is authoritative.","Pair by selecting DeezNats in Spotify Connect. API login and captured cookies do not pair Soloist.","Activate updates only after stopping playback; the paired-session and cache directories are preserved."] })
}
pub(crate) fn open_page(key: bool) -> Result<(), String> {
    use windows_sys::Win32::UI::{Shell::ShellExecuteW, WindowsAndMessaging::SW_SHOWNORMAL};
    let wide = |value: &str| value.encode_utf16().chain(Some(0)).collect::<Vec<_>>();
    let action = wide("open");
    let page = wide(if key { KEY_PAGE } else { DOWNLOADS });
    let result = unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            action.as_ptr(),
            page.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            SW_SHOWNORMAL,
        )
    };
    if result as isize <= 32 {
        Err("Could not open Spotify's setup page in your browser".into())
    } else {
        Ok(())
    }
}

#[cfg(test)]
#[path = "setup_tests.rs"]
mod tests;
