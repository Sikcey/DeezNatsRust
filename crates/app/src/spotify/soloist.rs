//! Reusable Soloist lifecycle and command API. Owns no Deezer/UI state.
use super::{
    process::{self, Process, MAX_OUTPUT},
    state::{Snapshot, State},
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    fs::{File, OpenOptions},
    io::{BufRead, BufReader, Read},
    os::windows::fs::OpenOptionsExt,
    path::PathBuf,
    process::{Command as ProcessCommand, Stdio},
    sync::{Arc, Mutex},
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct Config {
    pub executable: PathBuf,
    pub api_key: String,
}
impl Config {
    pub(super) fn saved() -> Result<Self, String> {
        serde_json::from_str(&crate::session::provider_saved("spotify-soloist")?)
            .map_err(|_| "Saved Soloist configuration is invalid".into())
    }
    pub fn load() -> Result<Self, String> {
        let saved = Self::saved().ok();
        let api_key = personal_key(
            std::env::var("DEEZNATS_SOLOIST_API_KEY").ok().as_deref(),
            saved.as_ref().map(|config| config.api_key.as_str()),
        )?;
        let executable = super::setup::preferred_executable()?;
        let config = Self {
            executable,
            api_key,
        };
        config.validate()?;
        Ok(config)
    }
    fn validate(&self) -> Result<(), String> {
        let clean = super::setup::strip_verbatim(&self.executable);
        if !clean.is_absolute() || !clean.is_file() {
            return Err("Soloist must be an existing absolute executable path".into());
        }
        if self.api_key.trim().is_empty() || self.api_key.chars().any(char::is_control) {
            return Err("Soloist API key is invalid".into());
        }
        Ok(())
    }
    pub fn save(&self) -> Result<(), String> {
        let config = Self {
            executable: self.executable.clone(),
            api_key: self.api_key.trim().to_owned(),
        };
        config.validate()?;
        crate::session::provider_save(
            "spotify-soloist",
            &serde_json::to_string(&config)
                .map_err(|_| "Could not encode Soloist configuration")?,
        )
    }
}
fn personal_key(environment: Option<&str>, saved: Option<&str>) -> Result<String, String> {
    environment.into_iter().chain(saved).map(str::trim).find(|key| !key.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| "Generate your own Soloist API key and enter it in Spotify setup, or set DEEZNATS_SOLOIST_API_KEY locally".into())
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Control {
    Play(Option<String>),
    Pause,
    Next,
    Previous,
    Seek(u32),
    Volume(u8),
    Shuffle(bool),
    Repeat(String),
    AddToQueue(String),
    Activate,
    Deactivate,
    Status,
    Now,
    Queue(u8),
}
impl Control {
    pub fn parse(args: &[&str]) -> Result<Self, String> {
        let command = match args {
            ["play"] => Self::Play(None),
            ["play", uri] if valid_uri(uri, false) => Self::Play(Some((*uri).into())),
            ["pause"] => Self::Pause,
            ["next"] => Self::Next,
            ["prev"] => Self::Previous,
            ["seek", value] => {
                Self::Seek(value.parse().map_err(|_| "Invalid Spotify seek position")?)
            }
            ["volume", value] => Self::Volume(
                value
                    .parse::<u8>()
                    .ok()
                    .filter(|v| *v <= 100)
                    .ok_or("Spotify volume must be 0-100")?,
            ),
            ["shuffle", "on"] => Self::Shuffle(true),
            ["shuffle", "off"] => Self::Shuffle(false),
            ["repeat", mode @ ("off" | "context" | "track")] => Self::Repeat((*mode).into()),
            ["add-to-queue", uri] if valid_uri(uri, true) => Self::AddToQueue((*uri).into()),
            ["activate"] => Self::Activate,
            ["deactivate"] => Self::Deactivate,
            ["status"] => Self::Status,
            ["now"] => Self::Now,
            ["queue"] => Self::Queue(10),
            ["queue", limit] => Self::Queue(
                limit
                    .parse::<u8>()
                    .ok()
                    .filter(|n| (1..=80).contains(n))
                    .ok_or("Spotify queue limit must be 1-80")?,
            ),
            _ => return Err("Invalid Spotify command".into()),
        };
        Ok(command)
    }
    fn args(&self) -> Result<Vec<String>, String> {
        // Validate typed callers too; enum construction cannot bypass command bounds.
        let args: Vec<String> = match self {
            Self::Play(None) => vec!["play".into()],
            Self::Play(Some(uri)) => vec!["play".into(), uri.clone()],
            Self::Pause => vec!["pause".into()],
            Self::Next => vec!["next".into()],
            Self::Previous => vec!["prev".into()],
            Self::Seek(ms) => vec!["seek".into(), ms.to_string()],
            Self::Volume(v) => vec!["volume".into(), v.to_string()],
            Self::Shuffle(value) => {
                vec!["shuffle".into(), if *value { "on" } else { "off" }.into()]
            }
            Self::Repeat(mode) => vec!["repeat".into(), mode.clone()],
            Self::AddToQueue(uri) => vec!["add-to-queue".into(), uri.clone()],
            Self::Activate => vec!["activate".into()],
            Self::Deactivate => vec!["deactivate".into()],
            Self::Status => vec!["status".into()],
            Self::Now => vec!["now".into()],
            Self::Queue(limit) => vec!["queue".into(), limit.to_string()],
        };
        Self::parse(&args.iter().map(String::as_str).collect::<Vec<_>>())?;
        Ok(args)
    }
}
fn valid_uri(uri: &str, track_only: bool) -> bool {
    let parts: Vec<_> = uri.split(':').collect();
    parts.len() == 3
        && parts[0] == "spotify"
        && (if track_only {
            parts[1] == "track"
        } else {
            matches!(parts[1], "track" | "episode" | "album" | "playlist")
        })
        && parts[2].len() == 22
        && parts[2].bytes().all(|b| b.is_ascii_alphanumeric())
}
struct Monitor {
    process: Process,
    reader: Option<JoinHandle<()>>,
}
impl Drop for Monitor {
    fn drop(&mut self) {
        self.process.stop();
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
    }
}
pub(crate) struct Soloist {
    process: Process,
    executable: PathBuf,
    data: PathBuf,
    state: Arc<Mutex<State>>,
    monitor: Option<Monitor>,
    last_reconnect: Instant,
    _lock: File,
}
impl Drop for Soloist {
    fn drop(&mut self) {
        self.monitor.take();
        self.process.stop();
    }
}
impl Soloist {
    pub(super) fn audio_target(&self) -> Result<super::process::AudioTarget, String> {
        self.process.audio_target()
    }
    pub fn start(config: Config) -> Result<Self, String> {
        Self::start_at(
            config,
            crate::storage::data_dir()?.join("spotify-soloist"),
            Duration::from_secs(15),
        )
    }
    pub(super) fn start_cancellable(
        config: Config,
        stop: &std::sync::atomic::AtomicBool,
    ) -> Result<Self, String> {
        Self::start_inner(
            config,
            crate::storage::data_dir()?.join("spotify-soloist"),
            Duration::from_secs(15),
            Some(stop),
        )
    }
    pub(super) fn start_at(
        config: Config,
        data: PathBuf,
        timeout: Duration,
    ) -> Result<Self, String> {
        Self::start_inner(config, data, timeout, None)
    }
    fn start_inner(
        config: Config,
        data: PathBuf,
        timeout: Duration,
        stop: Option<&std::sync::atomic::AtomicBool>,
    ) -> Result<Self, String> {
        config.validate()?;
        let data = super::setup::strip_verbatim(&data);
        std::fs::create_dir_all(&data).map_err(|_| "Could not create Soloist data directory")?;
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .share_mode(0)
            .open(data.join("deeznats.lock"))
            .map_err(|_| "Soloist is already managed by another DeezNats session")?;
        let mut command = clean_command(&config.executable);
        command
            .args([
                "--device-name",
                "DeezNats",
                "--api-key",
                &config.api_key,
                "--ws",
                "127.0.0.1:0",
                "--cache-size",
                "512",
            ])
            .arg("--data-dir")
            .arg(&data)
            .arg("--cache-dir")
            .arg(data.join("cache"))
            .stdout(Stdio::null());
        let process = Process::spawn(&mut command)?;
        let mut player = Self {
            process,
            executable: config.executable,
            data,
            state: Arc::new(Mutex::new(State::default())),
            monitor: None,
            last_reconnect: Instant::now(),
            _lock: lock,
        };
        let started = Instant::now();
        loop {
            if stop.is_some_and(|stop| stop.load(std::sync::atomic::Ordering::Relaxed)) {
                return Err("Spotify startup cancelled".into());
            }
            player.check_running()?;
            if player.runtime_ready() && player.control(&Control::Status).is_ok() {
                break;
            }
            if started.elapsed() >= timeout {
                player.process.stop();
                return Err(startup_error(None, &player.process.diagnostics()));
            }
            thread::sleep(Duration::from_millis(100));
        }
        player.connect_monitor()?;
        Ok(player)
    }
    fn check_running(&mut self) -> Result<(), String> {
        if let Some(status) = self
            .process
            .child
            .try_wait()
            .map_err(|_| "Could not inspect Soloist")?
        {
            self.process.stop();
            let error = startup_error(status.code(), &self.process.diagnostics());
            if let Ok(mut state) = self.state.lock() {
                state.disconnected(&error);
            }
            return Err(error);
        }
        // Key validation may finish after the local API has already started.
        let diagnostics = self.process.diagnostics();
        if api_key_rejected(&diagnostics.to_ascii_lowercase()) {
            return Err(startup_error(None, &diagnostics));
        }
        Ok(())
    }
    fn runtime_ready(&self) -> bool {
        let read = |name: &str| std::fs::read_to_string(self.data.join(name)).unwrap_or_default();
        read("soloist.pid").trim().parse::<u32>().ok() == Some(self.process.child.id())
            && read("ws.addr").trim() == "127.0.0.1"
            && read("ws.port").trim().parse::<u16>().is_ok_and(|p| p > 0)
    }
    fn ctl(&self) -> ProcessCommand {
        let mut command = clean_command(&self.executable);
        let clean_data = super::setup::strip_verbatim(&self.data);
        command.arg("ctl").arg("--data-dir").arg(clean_data);
        command
    }
    pub fn control(&mut self, control: &Control) -> Result<Value, String> {
        let args = control.args()?;
        self.check_running()?;
        if !self.runtime_ready() {
            return Err(
                "Soloist local API discovery is missing, stale, or not loopback-only".into(),
            );
        }
        let query = matches!(control, Control::Status | Control::Now | Control::Queue(_));
        let mut command = self.ctl();
        command.args(args);
        if matches!(control, Control::Now) {
            command.arg("--json");
        }
        let (status, bytes) = process::capture(&mut command, Duration::from_secs(3))?;
        if !status.success() {
            return Err(match status.code() {
                Some(2) => "Soloist local API is unavailable",
                Some(3) => {
                    "Soloist is not paired or did not respond; choose DeezNats in Spotify Connect"
                }
                _ => "Soloist rejected the command; check pairing and available actions",
            }
            .into());
        }
        if query {
            if let Ok(value) = serde_json::from_slice::<Value>(&bytes) {
                Ok(value)
            } else {
                let text = String::from_utf8_lossy(&bytes).trim().to_string();
                Ok(serde_json::json!({ "output": text }))
            }
        } else {
            Ok(Value::Null)
        }
    }
    fn connect_monitor(&mut self) -> Result<(), String> {
        self.monitor.take();
        if !self.runtime_ready() {
            return Err(
                "Soloist local API discovery is missing, stale, or not loopback-only".into(),
            );
        }
        let mut command = self.ctl();
        command.arg("trace").stdout(Stdio::piped());
        let mut process = Process::spawn(&mut command)?;
        let stdout = process
            .child
            .stdout
            .take()
            .ok_or("Soloist event stream unavailable")?;
        let state = self.state.clone();
        let reader = thread::spawn(move || {
            let mut input = BufReader::new(stdout);
            loop {
                let mut line = Vec::new();
                match (&mut input)
                    .take(MAX_OUTPUT as u64 + 1)
                    .read_until(b'\n', &mut line)
                {
                    Ok(0) | Err(_) => break,
                    Ok(_) if line.len() > MAX_OUTPUT => break,
                    Ok(_) => {
                        let text = String::from_utf8_lossy(&line);
                        let trimmed = text.trim();
                        if trimmed.is_empty() || trimmed.starts_with("connected to ") {
                            continue;
                        }
                        let result = parse_trace(&line).and_then(|event| {
                            state
                                .lock()
                                .map_err(|_| "Soloist state unavailable".to_owned())?
                                .apply(&event)
                        });
                        if result.is_err() {
                            break;
                        }
                    }
                }
            }
            if let Ok(mut state) = state.lock() {
                state.disconnected("Soloist event stream disconnected");
            }
        });
        self.monitor = Some(Monitor {
            process,
            reader: Some(reader),
        });
        self.last_reconnect = Instant::now();
        Ok(())
    }
    /// Polling reads a cached snapshot; only reconnects a failed event stream, never restarts audio.
    pub fn snapshot(&mut self) -> Result<Snapshot, String> {
        self.check_running()?;
        let disconnected = self
            .monitor
            .as_ref()
            .is_none_or(|m| m.reader.as_ref().is_none_or(JoinHandle::is_finished));
        if disconnected && self.last_reconnect.elapsed() >= Duration::from_secs(1) {
            self.connect_monitor()?;
        }
        self.state
            .lock()
            .map(|state| state.snapshot())
            .map_err(|_| "Soloist state unavailable".into())
    }
}
fn clean_command(executable: &std::path::Path) -> ProcessCommand {
    let clean = super::setup::strip_verbatim(executable);
    let mut command = ProcessCommand::new(clean);
    command
        .env_remove("DEEZNATS_SOLOIST_API_KEY")
        .env_remove("DEEZNATS_SPOTIFY_CLIENT_ID");
    command
}
fn startup_error(code: Option<i32>, diagnostics: &str) -> String {
    // Classify locally: never display raw daemon output, which can contain keys,
    // session tokens, or account information.
    let text = diagnostics.to_ascii_lowercase();
    let reason = if code == Some(10) {
        "Soloist build expired; download a new build from Spotify"
    } else if api_key_rejected(&text) {
        "Soloist rejected the personal API key. Generate a Soloist key in Spotify's developer dashboard and replace it in setup; a Web API client ID or client secret will not work"
    } else if text.contains("permission denied")
        || text.contains("access is denied")
        || text.contains("access denied")
    {
        "Soloist cannot access its data, cache, or executable directory. Check folder permissions"
    } else if text.contains("already running")
        || text.contains("already in use")
        || text.contains("lock file")
    {
        "Another Soloist process is using the data directory. Close it and start Spotify again"
    } else if text.contains("unknown option")
        || text.contains("unrecognized option")
        || text.contains("unexpected argument")
        || text.contains("invalid argument")
    {
        "This Soloist build rejected the startup options. Install the latest official Windows x64 build"
    } else if text.contains("websocket")
        && ["bind", "listen", "address in use"]
            .iter()
            .any(|word| text.contains(word))
    {
        "Soloist could not open its local WebSocket API. Check local firewall or endpoint security settings"
    } else if text.contains("premium") {
        "Soloist requires Spotify Premium for initial setup. Check the account used to generate the personal key"
    } else if [
        "resolve host",
        "dns",
        "connection refused",
        "network unreachable",
        "certificate",
        "tls",
    ]
    .iter()
    .any(|word| text.contains(word))
    {
        "Soloist could not connect to Spotify. Check the network, proxy, certificate, and firewall settings"
    } else if code.is_none() {
        "Soloist did not expose its local API before startup timed out. Check the installed build and data-directory access"
    } else {
        "Soloist failed to start. Check the installed build and data-directory access"
    };
    let override_note = if reason.contains("personal API key")
        && std::env::var("DEEZNATS_SOLOIST_API_KEY").is_ok_and(|key| !key.trim().is_empty())
    {
        " DEEZNATS_SOLOIST_API_KEY overrides the saved key; update or remove that environment variable."
    } else {
        ""
    };
    match code {
        Some(code) => format!("{reason} (exit code {code}).{override_note}"),
        None => format!("{reason}{override_note}"),
    }
}
fn api_key_rejected(text: &str) -> bool {
    text.lines().any(|line| {
        (line.contains("api key") || line.contains("api-key") || line.contains("api_key"))
            && [
                "invalid", "reject", "unauthor", "revoked", "expired", "missing",
            ]
            .iter()
            .any(|word| line.contains(word))
    })
}
fn parse_trace(line: &[u8]) -> Result<Value, String> {
    let text = std::str::from_utf8(line)
        .map_err(|_| "Invalid Soloist event encoding")?
        .trim();
    let json = if text.starts_with('{') {
        text
    } else {
        let (timestamp, json) = text
            .split_once(' ')
            .ok_or("Invalid Soloist event envelope")?;
        timestamp
            .parse::<u64>()
            .map_err(|_| "Invalid Soloist event timestamp")?;
        json
    };
    serde_json::from_str(json).map_err(|_| "Invalid Soloist event JSON".into())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn typed_controls_cannot_bypass_validation() {
        assert!(Control::Volume(101).args().is_err());
        assert!(Control::Repeat("--api-key".into()).args().is_err());
        assert!(
            Control::AddToQueue("spotify:album:0123456789012345678901".into())
                .args()
                .is_err()
        );
        assert!(
            Control::Play(Some("spotify:playlist:0123456789012345678901".into()))
                .args()
                .is_ok()
        );
        assert!(Control::Queue(81).args().is_err());
        assert!(Control::parse(&["seek", "-1"]).is_err());
    }
    #[test]
    fn trace_accepts_documented_timestamp_envelope_and_rejects_garbage() {
        assert_eq!(
            parse_trace(b"123 {\"type\":\"auth_state\",\"logged_in\":true}\n").unwrap()
                ["logged_in"],
            true
        );
        assert!(parse_trace(b"secret value").is_err());
        assert!(parse_trace(b"123 not-json").is_err());
    }
    #[test]
    fn personal_keys_ignore_paste_whitespace_and_empty_environment_overrides() {
        assert_eq!(
            personal_key(Some("  \r\n"), Some(" saved-key\n")).unwrap(),
            "saved-key"
        );
        assert_eq!(
            personal_key(Some(" environment-key\r\n"), Some("saved-key")).unwrap(),
            "environment-key"
        );
        assert!(personal_key(None, Some(" ")).is_err());
    }
    #[test]
    fn startup_diagnostics_are_actionable_without_exposing_raw_secrets() {
        assert!(!api_key_rejected(
            "api key accepted\ninvalid playback request"
        ));
        for (diagnostic, expected) in [
            ("invalid API key private-secret", "personal API key"),
            ("permission denied private-secret", "folder permissions"),
            ("unrecognized option private-secret", "startup options"),
            ("TLS handshake failed private-secret", "network"),
            ("unknown problem private-secret", "exit code 1"),
        ] {
            let error = startup_error(Some(1), diagnostic);
            assert!(error.contains(expected), "{error}");
            assert!(!error.contains("private-secret"));
        }
    }
    #[test]
    fn offline_daemon_lifecycle_state_reconnect_and_expiry() {
        struct Temporary(PathBuf);
        impl Drop for Temporary {
            fn drop(&mut self) {
                let root = std::env::temp_dir().canonicalize().unwrap();
                if self.0.parent().and_then(|p| p.canonicalize().ok()).as_ref() == Some(&root)
                    && self
                        .0
                        .file_name()
                        .unwrap()
                        .to_string_lossy()
                        .starts_with("deeznats-soloist-test-")
                {
                    let _ = std::fs::remove_dir_all(&self.0);
                }
            }
        }
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory = Temporary(std::env::temp_dir().join(format!(
            "deeznats-soloist-test-{}-{nonce}",
            std::process::id()
        )));
        std::fs::create_dir(&directory.0).unwrap();
        let executable = directory.0.join("soloist-fixture.exe");
        let mut compiler = ProcessCommand::new("rustc");
        compiler
            .arg("--edition=2021")
            .arg(
                std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("tests/fixtures/soloist_mock.rs"),
            )
            .arg("-o")
            .arg(&executable);
        assert!(process::capture(&mut compiler, Duration::from_secs(30))
            .unwrap()
            .0
            .success());
        let config = |key: &str| Config {
            executable: executable.clone(),
            api_key: key.into(),
        };
        let data = directory.0.join("data");
        let mut player =
            Soloist::start_at(config("test"), data.clone(), Duration::from_secs(5)).unwrap();
        assert!(
            Soloist::start_at(config("test"), data.clone(), Duration::from_secs(1))
                .err()
                .unwrap()
                .contains("another DeezNats")
        );
        let started = Instant::now();
        loop {
            let snapshot = player.snapshot().unwrap();
            if snapshot.logged_in && snapshot.volume == 42 {
                assert_eq!(snapshot.position_ms, 1234);
                break;
            }
            assert!(started.elapsed() < Duration::from_secs(3));
            thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(player.control(&Control::Now).unwrap()["fixture"], true);
        assert!(player
            .control(&Control::Volume(50))
            .unwrap_err()
            .contains("not paired"));
        player.control(&Control::Pause).unwrap();
        // A broken event stream can reconnect while the daemon remains untouched.
        player.monitor.as_mut().unwrap().process.stop();
        let started = Instant::now();
        while !player
            .monitor
            .as_ref()
            .unwrap()
            .reader
            .as_ref()
            .unwrap()
            .is_finished()
        {
            assert!(started.elapsed() < Duration::from_secs(3));
            thread::sleep(Duration::from_millis(10));
        }
        let pid = player.process.child.id();
        player.last_reconnect = Instant::now() - Duration::from_secs(2);
        player.snapshot().unwrap();
        assert_eq!(player.process.child.id(), pid);
        // Simulate the OS closing our job handles on app exit.
        player.process.job_for_test_close();
        let started = Instant::now();
        while player.process.child.try_wait().unwrap().is_none() {
            assert!(started.elapsed() < Duration::from_secs(3));
            thread::sleep(Duration::from_millis(10));
        }
        assert!(player.snapshot().is_err());
        drop(player);
        assert!(Soloist::start_at(
            config("expired"),
            directory.0.join("expired"),
            Duration::from_secs(2)
        )
        .err()
        .unwrap()
        .contains("expired"));
        let invalid_key = Soloist::start_at(
            config("invalid-key-fixture-secret"),
            directory.0.join("invalid-key"),
            Duration::from_secs(2),
        )
        .err()
        .unwrap();
        assert!(invalid_key.contains("personal API key"), "{invalid_key}");
        assert!(!invalid_key.contains("invalid-key-fixture-secret"));
        let late_data = directory.0.join("late-key-rejection");
        let mut late = Soloist::start_at(
            config("late-invalid-key-fixture-secret"),
            late_data.clone(),
            Duration::from_secs(2),
        )
        .unwrap();
        std::fs::write(late_data.join("reject-key"), b"").unwrap();
        let started = Instant::now();
        loop {
            if let Err(error) = late.snapshot() {
                assert!(error.contains("personal API key"), "{error}");
                assert!(!error.contains("late-invalid-key-fixture-secret"));
                break;
            }
            assert!(started.elapsed() < Duration::from_secs(2));
            thread::sleep(Duration::from_millis(10));
        }
        drop(late);
        assert!(Soloist::start_at(
            config("no-api"),
            directory.0.join("missing-api"),
            Duration::from_millis(200)
        )
        .err()
        .unwrap()
        .contains("local API"));
    }
}
