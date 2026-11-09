//! Native update UI worker. Release bytes never run until verified and staged.
mod helper {
    use deeznats_updater::ApplyPlan;
    use std::{
        fs,
        os::windows::process::CommandExt,
        path::{Path, PathBuf},
        process::{Child, Command},
        time::{Duration, Instant},
    };
    use windows_sys::Win32::{
        Foundation::{CloseHandle, WAIT_OBJECT_0},
        System::Threading::{OpenProcess, WaitForSingleObject, PROCESS_SYNCHRONIZE},
    };

    pub fn spawn(stage: &Path) -> Result<Child, String> {
        let helper = stage.join("update-helper.exe");
        fs::copy(std::env::current_exe().map_err(|e| e.to_string())?, &helper)
            .map_err(|e| e.to_string())?;
        let mut child = Command::new(helper)
            .arg("--apply-update")
            .arg(stage)
            .current_dir(stage)
            .creation_flags(0x08000000)
            .spawn()
            .map_err(|e| e.to_string())?;
        let started = Instant::now();
        while started.elapsed() < Duration::from_secs(10) {
            if stage.join("ready").is_file() {
                return Ok(child);
            }
            if child.try_wait().map_err(|e| e.to_string())?.is_some() {
                return Err(fs::read_to_string(stage.join("result.txt"))
                    .unwrap_or_else(|_| "Could not start the update helper.".into()));
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        let _ = child.kill();
        let _ = child.wait();
        Err("Update helper did not become ready. The application was left running.".into())
    }

    pub fn run(stage: &Path) -> Result<(), String> {
        super::validate_stage(stage)?;
        let plan: ApplyPlan =
            serde_json::from_slice(&fs::read(stage.join("plan.json")).map_err(|e| e.to_string())?)
                .map_err(|e| e.to_string())?;
        let mut parent_exited = false;
        let operation = (|| -> Result<(), String> {
            plan.validate(stage)?;
            let parent = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, 0, plan.parent_pid) };
            if parent.is_null() {
                return Err("The original application is no longer running.".into());
            }
            struct Process(windows_sys::Win32::Foundation::HANDLE);
            impl Drop for Process {
                fn drop(&mut self) {
                    unsafe {
                        CloseHandle(self.0);
                    }
                }
            }
            let parent = Process(parent);
            fs::write(stage.join("ready"), b"ready").map_err(|e| e.to_string())?;
            if unsafe { WaitForSingleObject(parent.0, 60_000) } != WAIT_OBJECT_0 {
                return Err("The application did not close. No files were changed.".into());
            }
            parent_exited = true;
            deeznats_updater::commit(&plan, stage)?;
            update_installed_version(&plan);
            Ok(())
        })();
        let message = operation
            .as_ref()
            .map(|_| "Update installed.".to_owned())
            .unwrap_or_else(|error| error.clone());
        let _ = fs::write(stage.join("result.txt"), &message);
        // Only restart after the parent exited, including after a rolled-back install.
        if parent_exited {
            let executable = plan.directory.join("deeznats.exe");
            Command::new(executable)
                .arg("--update-result")
                .arg(stage)
                .current_dir(&plan.directory)
                .creation_flags(0x08000000)
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
                .map_err(|e| e.to_string())?;
        }
        operation
    }

    fn update_installed_version(plan: &ApplyPlan) {
        use windows_sys::Win32::System::Registry::*;
        fn wide(value: &str) -> Vec<u16> {
            value.encode_utf16().chain(Some(0)).collect()
        }
        unsafe {
            let mut value = [0u16; 32768];
            let mut bytes = (value.len() * 2) as u32;
            if RegGetValueW(
                HKEY_CURRENT_USER,
                wide("Software\\DeezNats").as_ptr(),
                wide("InstallDir").as_ptr(),
                RRF_RT_REG_SZ,
                std::ptr::null_mut(),
                value.as_mut_ptr().cast(),
                &mut bytes,
            ) != 0
            {
                return;
            }
            let length = value.iter().position(|ch| *ch == 0).unwrap_or(value.len());
            let registered = PathBuf::from(String::from_utf16_lossy(&value[..length]));
            if registered.canonicalize().ok() != plan.directory.canonicalize().ok() {
                return;
            }
            let version = wide(&plan.version);
            RegSetKeyValueW(
                HKEY_CURRENT_USER,
                wide("Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\DeezNats").as_ptr(),
                wide("DisplayVersion").as_ptr(),
                REG_SZ,
                version.as_ptr().cast(),
                (version.len() * 2) as u32,
            );
        }
    }

    pub fn startup_result() -> Option<String> {
        let args: Vec<_> = std::env::args_os().collect();
        let index = args.iter().position(|arg| arg == "--update-result")?;
        let stage = PathBuf::from(args.get(index + 1)?);
        super::validate_stage(&stage).ok()?;
        let message = fs::read_to_string(stage.join("result.txt")).ok()?;
        // The helper may still be exiting. Cleanup is limited to this exact private
        // staging directory and happens off the UI thread.
        std::thread::spawn(move || {
            for _ in 0..20 {
                if super::clean_stage(&stage).is_ok() {
                    break;
                }
                std::thread::sleep(Duration::from_millis(250));
            }
        });
        Some(message)
    }
}
pub(crate) mod http {
    //! Unauthenticated HTTPS release downloads. Redirects may not downgrade to HTTP.
    use std::{
        ffi::c_void,
        io::Write,
        ptr::{null, null_mut},
        time::{Duration, Instant},
    };
    use windows_sys::Win32::Networking::WinHttp::*;
    struct Handle(*mut c_void);
    pub const NO_RELEASE: &str = "No published update is available yet.";
    impl Drop for Handle {
        fn drop(&mut self) {
            unsafe {
                WinHttpCloseHandle(self.0);
            }
        }
    }
    fn wide(value: &str) -> Vec<u16> {
        value.encode_utf16().chain(Some(0)).collect()
    }

    pub fn get(
        url: &str,
        mut output: impl Write,
        limit: u64,
        mut progress: impl FnMut(u64),
    ) -> Result<u64, String> {
        let rest = url
            .strip_prefix("https://")
            .ok_or("Update URLs must use HTTPS.")?;
        let (host, path) = rest.split_once('/').ok_or("Invalid update URL.")?;
        if !matches!(
            host,
            "api.github.com" | "github.com" | "soloist-builds.spotifycdn.com"
        ) || path.contains(['\r', '\n', '\0'])
        {
            return Err("Invalid update server.".into());
        }
        unsafe {
            let session = Handle(WinHttpOpen(
                wide("DeezNatsRust-Updater/1").as_ptr(),
                WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY,
                null(),
                null(),
                0,
            ));
            if session.0.is_null() {
                return Err("Could not initialize update download.".into());
            }
            WinHttpSetTimeouts(session.0, 5000, 5000, 10000, 15000);
            let connection = Handle(WinHttpConnect(session.0, wide(host).as_ptr(), 443, 0));
            if connection.0.is_null() {
                return Err("Could not connect to update server.".into());
            }
            let request = Handle(WinHttpOpenRequest(
                connection.0,
                wide("GET").as_ptr(),
                wide(&format!("/{path}")).as_ptr(),
                null(),
                null(),
                null(),
                WINHTTP_FLAG_SECURE,
            ));
            if request.0.is_null() {
                return Err("Could not create update request.".into());
            }
            for (option, value) in [
                (WINHTTP_OPTION_DISABLE_FEATURE, WINHTTP_DISABLE_COOKIES),
                (
                    WINHTTP_OPTION_REDIRECT_POLICY,
                    WINHTTP_OPTION_REDIRECT_POLICY_DISALLOW_HTTPS_TO_HTTP,
                ),
                (
                    WINHTTP_OPTION_AUTOLOGON_POLICY,
                    WINHTTP_AUTOLOGON_SECURITY_LEVEL_HIGH,
                ),
            ] {
                if WinHttpSetOption(request.0, option, (&value as *const u32).cast(), 4) == 0 {
                    return Err("Could not configure secure update request.".into());
                }
            }
            if WinHttpSendRequest(request.0, null(), 0, null(), 0, 0, 0) == 0
                || WinHttpReceiveResponse(request.0, null_mut()) == 0
            {
                return Err("Update request failed or timed out.".into());
            }
            let mut status = 0u32;
            let mut length = 4;
            if WinHttpQueryHeaders(
                request.0,
                WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
                null(),
                (&mut status as *mut u32).cast(),
                &mut length,
                null_mut(),
            ) == 0
            {
                return Err("Invalid update response.".into());
            }
            if status == 404 {
                return Err(NO_RELEASE.into());
            }
            if status != 200 {
                return Err(format!("Update server returned HTTP {status}."));
            }
            let start = Instant::now();
            let mut total = 0u64;
            let mut chunk = [0u8; 65536];
            loop {
                if start.elapsed() > Duration::from_secs(300) {
                    return Err("Update download timed out.".into());
                }
                let mut count = 0;
                if WinHttpReadData(
                    request.0,
                    chunk.as_mut_ptr().cast(),
                    chunk.len() as u32,
                    &mut count,
                ) == 0
                {
                    return Err("Update download was interrupted.".into());
                }
                if count == 0 {
                    break;
                }
                total += count as u64;
                if total > limit {
                    return Err("Update download exceeded its expected size.".into());
                }
                output
                    .write_all(&chunk[..count as usize])
                    .map_err(|e| e.to_string())?;
                progress(total);
            }
            Ok(total)
        }
    }

    pub fn bytes(url: &str, limit: u64) -> Result<Vec<u8>, String> {
        let mut data = Vec::new();
        get(url, &mut data, limit, |_| {})?;
        Ok(data)
    }
}
use deeznats_updater::{ApplyFile, ApplyPlan, Manifest, Transfer, MANIFEST_ASSET};
pub use helper::{run as apply_update, startup_result};
use serde::Deserialize;
use std::{
    fs,
    path::{Path, PathBuf},
    sync::mpsc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

const REPO: &str = "Sikcey/DeezNatsRust";
#[derive(Clone, Default, PartialEq)]
pub enum Phase {
    #[default]
    Idle,
    Checking,
    Current,
    Available,
    Downloading,
    Ready,
    Applying,
    Failed,
}
#[derive(Clone, Default)]
pub struct State {
    pub phase: Phase,
    pub version: String,
    pub progress: u8,
    pub error: String,
}
impl State {
    pub fn status(&self) -> &'static str {
        match self.phase {
            Phase::Idle => "Check for application updates.",
            Phase::Checking => "Checking for updates...",
            Phase::Current => "You are up to date.",
            Phase::Available => "Update available.",
            Phase::Downloading => "Downloading update...",
            Phase::Ready => "Update ready. Restart to apply.",
            Phase::Applying => "Restarting to apply update...",
            Phase::Failed => "Update could not be completed.",
        }
    }
    pub fn busy(&self) -> bool {
        matches!(
            self.phase,
            Phase::Checking | Phase::Downloading | Phase::Applying
        )
    }
}
pub enum Request {
    Check,
    Download,
    Apply,
}
pub enum Event {
    State(State),
    Exit,
}
pub struct Worker {
    pub commands: mpsc::SyncSender<Request>,
    pub events: mpsc::Receiver<Event>,
}
#[derive(Deserialize)]
struct Release {
    tag_name: String,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
    assets: Vec<Asset>,
}
#[derive(Deserialize)]
struct Asset {
    name: String,
    size: u64,
    browser_download_url: String,
}
struct Available {
    manifest: Manifest,
    base_url: String,
    assets: Vec<Asset>,
}

pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
fn stage_root() -> PathBuf {
    std::env::temp_dir().join("DeezNatsRustUpdates")
}
pub fn validate_stage(path: &Path) -> Result<(), String> {
    let root = stage_root().canonicalize().map_err(|e| e.to_string())?;
    let resolved = path.canonicalize().map_err(|e| e.to_string())?;
    if resolved.parent() != Some(root.as_path())
        || !resolved
            .file_name()
            .is_some_and(|n| n.to_string_lossy().starts_with("update-"))
    {
        return Err("Invalid update staging directory.".into());
    }
    Ok(())
}
fn clean_stage(path: &Path) -> Result<(), String> {
    validate_stage(path)?;
    fs::remove_dir_all(path).map_err(|e| e.to_string())
}
fn new_stage() -> Result<PathBuf, String> {
    let root = stage_root();
    fs::create_dir_all(&root).map_err(|e| e.to_string())?;
    let path = root.join(format!(
        "update-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    fs::create_dir(&path).map_err(|e| e.to_string())?;
    Ok(path)
}

fn check() -> Result<Option<Available>, String> {
    let data = match http::bytes(
        &format!("https://api.github.com/repos/{REPO}/releases/latest"),
        2 * 1024 * 1024,
    ) {
        Ok(data) => data,
        Err(error) if error == http::NO_RELEASE => return Ok(None),
        Err(error) => return Err(error),
    };
    let release: Release =
        serde_json::from_slice(&data).map_err(|_| "Invalid update release response.")?;
    if release.draft || release.prerelease {
        return Ok(None);
    }
    let version = release
        .tag_name
        .strip_prefix('v')
        .unwrap_or(&release.tag_name);
    // Manifest validation also validates the version before it becomes a URL path.
    if !version.bytes().all(|b| b.is_ascii_digit() || b == b'.') {
        return Err("Unsupported update version.".into());
    }
    if !deeznats_updater::newer_version(version, env!("CARGO_PKG_VERSION"))? {
        return Ok(None);
    }
    let base_url = format!(
        "https://github.com/{REPO}/releases/download/{}/",
        release.tag_name
    );
    let asset = release
        .assets
        .iter()
        .find(|a| a.name == MANIFEST_ASSET)
        .ok_or("No compatible update package is published for this release.")?;
    if asset.size > 128 * 1024
        || asset.browser_download_url != format!("{base_url}{MANIFEST_ASSET}")
    {
        return Err("Invalid update manifest asset.".into());
    }
    let bytes = http::bytes(&asset.browser_download_url, asset.size)?;
    if bytes.len() as u64 != asset.size {
        return Err("Incomplete update manifest.".into());
    }
    let manifest = Manifest::parse(&bytes)?;
    if manifest.version != version {
        return Err("Update release version mismatch.".into());
    }
    if !manifest.newer_than(env!("CARGO_PKG_VERSION"))? {
        return Ok(None);
    }
    Ok(Some(Available {
        manifest,
        base_url,
        assets: release.assets,
    }))
}

fn download_asset(
    release: &Available,
    name: &str,
    size: u64,
    digest: &str,
    target: &Path,
    progress: impl FnMut(u64),
) -> Result<(), String> {
    let url = format!("{}{name}", release.base_url);
    let asset = release
        .assets
        .iter()
        .find(|asset| asset.name == name)
        .ok_or("Update asset is missing.")?;
    if asset.size != size || asset.browser_download_url != url {
        return Err("Update asset does not match its manifest.".into());
    }
    let file = fs::File::create(target).map_err(|e| e.to_string())?;
    http::get(&url, file, size, progress)?;
    deeznats_updater::verify(target, size, digest)
}

fn download(
    release: &Available,
    directory: &Path,
    stage: &Path,
    send: &mpsc::Sender<Event>,
) -> Result<(), String> {
    let changes = release.manifest.changes(directory)?;
    let mut files = Vec::new();
    let total = changes.len().max(1);
    for (index, change) in changes.into_iter().enumerate() {
        let target = stage.join(&change.file.name);
        let mut last_progress = std::time::Instant::now() - Duration::from_secs(1);
        let mut progress = |done: u64, size: u64| {
            if last_progress.elapsed() >= Duration::from_millis(100) {
                let progress = (((index as u64 * 100)
                    + (done.saturating_mul(100) / size.max(1)).min(100))
                    / total as u64)
                    .min(99) as u8;
                let _ = send.send(Event::State(State {
                    phase: Phase::Downloading,
                    version: release.manifest.version.clone(),
                    progress,
                    ..Default::default()
                }));
                last_progress = std::time::Instant::now();
            }
        };
        let patched = if let Transfer::Delta(patch) = &change.transfer {
            let patch_path = stage.join("delta.bsdiff");
            let result = download_asset(
                release,
                &patch.asset,
                patch.size,
                &patch.sha256,
                &patch_path,
                |done| progress(done, patch.size),
            )
            .and_then(|_| {
                deeznats_updater::apply_delta(
                    &directory.join(&change.file.name),
                    &patch_path,
                    &target,
                    &change.file,
                    patch,
                )
            });
            let _ = fs::remove_file(&patch_path);
            result.is_ok()
        } else {
            false
        };
        if !patched {
            download_asset(
                release,
                &change.file.asset,
                change.file.size,
                &change.file.sha256,
                &target,
                |done| progress(done, change.file.size),
            )?;
        }
        files.push(ApplyFile {
            name: change.file.name,
            size: change.file.size,
            sha256: change.file.sha256,
        });
    }
    if files.is_empty() {
        return Err("Installed files already match this update.".into());
    }
    let plan = ApplyPlan {
        directory: directory.to_owned(),
        parent_pid: std::process::id(),
        version: release.manifest.version.clone(),
        files,
    };
    plan.validate(stage)?;
    fs::write(
        stage.join("plan.json"),
        serde_json::to_vec(&plan).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())
}

impl Worker {
    pub fn start() -> Self {
        let (commands, requests) = mpsc::sync_channel(2);
        let (send, events) = mpsc::channel();
        std::thread::spawn(move || {
            let mut available = None;
            let mut stage: Option<PathBuf> = None;
            let mut applying = false;
            while let Ok(request) = requests.recv() {
                let result: Result<(), String> = (|| {
                    match request {
                        Request::Check => {
                            let _ = send.send(Event::State(State {
                                phase: Phase::Checking,
                                ..Default::default()
                            }));
                            let release = check()?;
                            let state = if let Some(release) = &release {
                                State {
                                    phase: Phase::Available,
                                    version: release.manifest.version.clone(),
                                    ..Default::default()
                                }
                            } else {
                                State {
                                    phase: Phase::Current,
                                    ..Default::default()
                                }
                            };
                            if let Some(old) = stage.take() {
                                let _ = clean_stage(&old);
                            }
                            available = release;
                            let _ = send.send(Event::State(state));
                        }
                        Request::Download => {
                            let release = available.as_ref().ok_or("Check for updates first.")?;
                            let _ = send.send(Event::State(State {
                                phase: Phase::Downloading,
                                version: release.manifest.version.clone(),
                                ..Default::default()
                            }));
                            if let Some(old) = stage.take() {
                                let _ = clean_stage(&old);
                            }
                            let path = new_stage()?;
                            stage = Some(path.clone());
                            let executable = std::env::current_exe().map_err(|e| e.to_string())?;
                            let directory = executable
                                .parent()
                                .ok_or("Application directory is unavailable.")?;
                            download(release, directory, &path, &send)?;
                            let _ = send.send(Event::State(State {
                                phase: Phase::Ready,
                                version: release.manifest.version.clone(),
                                progress: 100,
                                ..Default::default()
                            }));
                        }
                        Request::Apply => {
                            let path = stage.as_ref().ok_or("Download the update first.")?;
                            let _ = send.send(Event::State(State {
                                phase: Phase::Applying,
                                ..Default::default()
                            }));
                            helper::spawn(path)?;
                            applying = true;
                            let _ = send.send(Event::Exit);
                        }
                    }
                    Ok(())
                })();
                if let Err(error) = result {
                    let _ = send.send(Event::State(State {
                        phase: Phase::Failed,
                        error,
                        ..Default::default()
                    }));
                }
                if applying {
                    break;
                }
            }
            if !applying {
                if let Some(path) = stage {
                    let _ = clean_stage(&path);
                }
            }
        });
        Self { commands, events }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn release_transport_rejects_other_origins_without_network_access() {
        for url in [
            "http://github.com/anything",
            "https://evil.example/app",
            "https://github.com@evil.example/app",
            "https://github.com/file\r\nInjected: 1",
        ] {
            assert!(http::bytes(url, 1024).is_err());
        }
    }
    #[test]
    #[ignore = "reads a public GitHub license to exercise HTTPS release redirects"]
    fn github_https_redirect_download() {
        let data = http::bytes(
            "https://github.com/rust-lang/rust/raw/refs/heads/main/LICENSE-MIT",
            64 * 1024,
        )
        .unwrap();
        assert!(String::from_utf8(data)
            .unwrap()
            .contains("Permission is hereby granted"));
    }

    #[test]
    #[ignore = "runs an isolated Windows restart-helper smoke test with temporary files"]
    fn apply_helper_waits_for_parent_and_preserves_settings() {
        use std::{os::windows::process::CommandExt, process::Command};
        let stage = new_stage().unwrap();
        let installed = std::env::temp_dir().join(format!(
            "deeznats-helper-test-{}-{}",
            std::process::id(),
            now()
        ));
        fs::create_dir(&installed).unwrap();
        let system = PathBuf::from(std::env::var_os("SystemRoot").unwrap()).join("System32");
        // whoami.exe is an inert stand-in here: the unsupported application
        // argument makes it exit without running an interactive shell.
        let bytes = fs::read(system.join("whoami.exe")).unwrap();
        fs::write(stage.join("deeznats.exe"), &bytes).unwrap();
        fs::write(installed.join("deeznats.exe"), b"old executable").unwrap();
        fs::write(installed.join("ui.json"), b"preserved preferences").unwrap();
        let mut parent = Command::new(system.join("WindowsPowerShell/v1.0/powershell.exe"))
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                "Start-Sleep -Milliseconds 750",
            ])
            .creation_flags(0x08000000)
            .spawn()
            .unwrap();
        let plan = ApplyPlan {
            directory: installed.clone(),
            parent_pid: parent.id(),
            version: "0.2.0".into(),
            files: vec![ApplyFile {
                name: "deeznats.exe".into(),
                size: bytes.len() as u64,
                sha256: deeznats_updater::hash(&bytes),
            }],
        };
        fs::write(stage.join("plan.json"), serde_json::to_vec(&plan).unwrap()).unwrap();
        helper::run(&stage).unwrap();
        assert!(parent.try_wait().unwrap().is_some());
        assert_eq!(fs::read(installed.join("deeznats.exe")).unwrap(), bytes);
        assert_eq!(
            fs::read(installed.join("ui.json")).unwrap(),
            b"preserved preferences"
        );
        assert_eq!(
            fs::read_to_string(stage.join("result.txt")).unwrap(),
            "Update installed."
        );
        clean_stage(&stage).unwrap();
        let target = installed.canonicalize().unwrap();
        assert_eq!(
            target.parent(),
            std::env::temp_dir().canonicalize().ok().as_deref()
        );
        for _ in 0..20 {
            if fs::remove_dir_all(&target).is_ok() {
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        assert!(!target.exists());
    }
}
