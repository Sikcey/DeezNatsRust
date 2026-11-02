//! Pairing supervisor. Startup/restoration is automatic; first Connect selection remains user-driven.
use super::{
    soloist::{Config, Control, Soloist},
    state::Snapshot,
};
#[path = "output.rs"]
mod output;
use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc, Mutex, Weak,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Phase {
    Starting,
    Connecting,
    Pairing,
    Ready,
    Failed,
    Stopped,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Status {
    pub phase: Phase,
    pub message: String,
}
impl Status {
    fn new(phase: Phase, message: impl Into<String>) -> Self {
        Self {
            phase,
            message: message.into(),
        }
    }
}
pub(crate) struct Service {
    status: Arc<Mutex<Status>>,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
    handle: Arc<Handle>,
}
static ACTIVE: Mutex<Option<Weak<Handle>>> = Mutex::new(None);
pub(crate) fn reopen_output() -> Result<(), String> {
    let handle = ACTIVE
        .lock()
        .ok()
        .and_then(|active| active.as_ref().and_then(Weak::upgrade))
        .ok_or("Start local Spotify playback first")?;
    if matches!(handle.status().phase, Phase::Failed | Phase::Stopped) {
        return Err("Start local Spotify playback first".into());
    }
    handle.reopen_output.store(true, Ordering::Release);
    Ok(())
}
pub(crate) use super::process::AudioTarget;

/// Read-only access: this never starts Soloist or extends the service lease.
pub(crate) fn audio_target() -> Option<Arc<AudioTarget>> {
    let handle = ACTIVE.lock().ok()?.as_ref()?.upgrade()?;
    if handle.stop.load(Ordering::Acquire) {
        return None;
    }
    if !handle
        .snapshot()
        .is_some_and(|state| state.connected && state.logged_in && state.is_active)
    {
        return None;
    }
    let target = handle.audio_target.lock().ok()?.clone()?;
    target.is_alive().then_some(target)
}

struct PublishedTarget(Arc<Mutex<Option<Arc<AudioTarget>>>>);
impl Drop for PublishedTarget {
    fn drop(&mut self) {
        if let Ok(mut target) = self.0.lock() {
            *target = None;
        }
    }
}
struct Request {
    command: Control,
    reply: mpsc::SyncSender<Result<serde_json::Value, String>>,
    deadline: Instant,
}
pub(crate) struct Handle {
    reopen_output: Arc<AtomicBool>,
    audio_target: Arc<Mutex<Option<Arc<AudioTarget>>>>,
    state: Arc<Mutex<Option<Arc<Snapshot>>>>,
    status: Arc<Mutex<Status>>,
    commands: mpsc::SyncSender<Request>,
    stop: Arc<AtomicBool>,
}
impl Drop for Handle {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}
impl Handle {
    pub fn snapshot(&self) -> Option<Arc<Snapshot>> {
        self.state.lock().ok()?.clone()
    }
    pub fn status(&self) -> Status {
        self.status
            .lock()
            .map(|s| s.clone())
            .unwrap_or_else(|_| Status::new(Phase::Failed, "Spotify status unavailable"))
    }
    pub fn control(&self, command: &Control) -> Result<serde_json::Value, String> {
        let (reply, rx) = mpsc::sync_channel(1);
        self.commands
            .try_send(Request {
                command: command.clone(),
                reply,
                deadline: Instant::now() + Duration::from_secs(4),
            })
            .map_err(|_| "Spotify playback worker busy or unavailable")?;
        rx.recv_timeout(Duration::from_secs(5))
            .map_err(|_| "Spotify playback request timed out")?
    }
}
impl Service {
    pub fn start() -> Result<Self, String> {
        let mut active = ACTIVE
            .lock()
            .map_err(|_| "Spotify supervisor unavailable")?;
        if let Some(handle) = active
            .as_ref()
            .and_then(Weak::upgrade)
            .filter(|h| !matches!(h.status().phase, Phase::Failed | Phase::Stopped))
        {
            return Ok(Self {
                status: handle.status.clone(),
                stop: handle.stop.clone(),
                worker: None,
                handle,
            });
        }
        let service = Self::start_with(|stop| Soloist::start_cancellable(Config::load()?, stop))?;
        *active = Some(Arc::downgrade(&service.handle));
        Ok(service)
    }
    pub fn handle(&self) -> Arc<Handle> {
        self.handle.clone()
    }
    fn start_with(
        factory: impl Fn(&AtomicBool) -> Result<Soloist, String> + Send + 'static,
    ) -> Result<Self, String> {
        let status = Arc::new(Mutex::new(Status::new(
            Phase::Starting,
            "Starting Soloist...",
        )));
        let stop = Arc::new(AtomicBool::new(false));
        let output = status.clone();
        let stopped = stop.clone();
        let state = Arc::new(Mutex::new(None));
        let snapshots = state.clone();
        let audio_target = Arc::new(Mutex::new(None));
        let published_target = audio_target.clone();
        let (commands, requests) = mpsc::sync_channel::<Request>(16);
        let reopen_output = Arc::new(AtomicBool::new(false));
        let reopen_requested = reopen_output.clone();
        let handle = Arc::new(Handle {
            reopen_output,
            audio_target,
            state,
            status: status.clone(),
            commands,
            stop: stop.clone(),
        });
        let worker = thread::Builder::new()
            .name("spotify-pairing".into())
            .spawn(move || {
                let publish = |state: Status| {
                    if let Ok(mut current) = output.lock() {
                        *current = state;
                    }
                };
                // Normal startup restores saved pairing. Never use --pair, which clears that session.
                let mut output_watcher = output::Watcher::new();
                let started = factory(&stopped);
                if stopped.load(Ordering::Relaxed) {
                    drop(started);
                    publish(Status::new(Phase::Stopped, "Spotify playback stopped."));
                    return;
                }
                let mut player = match started {
                    Ok(player) => player,
                    Err(error) => {
                        publish(Status::new(Phase::Failed, error));
                        return;
                    }
                };
                // Cleared on every supervisor exit, before the player is destroyed.
                let _published = PublishedTarget(published_target);
                match player.audio_target() {
                    Ok(target) => {
                        if let Ok(mut current) = _published.0.lock() {
                            *current = Some(Arc::new(target));
                        }
                    }
                    Err(error) => {
                        publish(Status::new(Phase::Failed, error));
                        return;
                    }
                }
                let mut awaiting_since = None;
                let mut last_connected = Instant::now();
                let mut restore: Option<OutputRecovery> = None;
                while !stopped.load(Ordering::Relaxed) {
                    let requested = reopen_requested.swap(false, Ordering::AcqRel);
                    let output_changed = output_watcher.changed();
                    if requested || output_changed {
                        let previous = restore.take().map(|recovery| recovery.previous).or_else(||
                            player.snapshot().ok().filter(|s| s.connected && s.logged_in && s.is_active
                                && matches!(s.status.as_str(), "playing" | "paused")));
                        publish(Status::new(Phase::Connecting, "Reopening Spotify audio output..."));
                        if let Ok(mut target) = _published.0.lock() { *target = None; }
                        // Keep the last confirmed track visible while the output is reopened.
                        // Clearing it also makes the audio worker discard its managed queue.
                        // Release the process, monitor and exclusive data lock before restarting.
                        drop(player);
                        player = match factory(&stopped) {
                            Ok(player) => player,
                            Err(error) => { publish(Status::new(Phase::Failed, error)); return; }
                        };
                        if stopped.load(Ordering::Relaxed) { break; }
                        match player.audio_target() {
                            Ok(target) => {
                                if let Ok(mut current) = _published.0.lock() { *current = Some(Arc::new(target)); }
                            }
                            Err(error) => { publish(Status::new(Phase::Failed, error)); return; }
                        }
                        restore = previous.map(OutputRecovery::new);
                        last_connected = Instant::now();
                        awaiting_since = None;
                    }
                    let mut request = requests.recv_timeout(Duration::from_millis(150)).ok();
                    while let Some(req) = request.take() {
                        let result = if Instant::now() < req.deadline {
                            // Keep new sound settings without discarding the interrupted track.
                            match (&req.command, &mut restore) {
                                (Control::Volume(value), Some(recovery)) => recovery.previous.volume = *value,
                                (Control::Shuffle(value), Some(recovery)) => recovery.previous.shuffle = *value,
                                (Control::Repeat(value), Some(recovery)) => recovery.previous.repeat.clone_from(value),
                                (Control::Status | Control::Now | Control::Queue(_), _) => {},
                                _ => restore = None,
                            }
                            player.control(&req.command)
                        } else {
                            Err("Spotify playback request expired before dispatch".into())
                        };
                        let _ = req.reply.send(result);
                        request = requests.try_recv().ok();
                    }
                    match player.snapshot() {
                        Ok(snapshot) => {
                            if let Some(recovery) = &mut restore {
                                match recovery.advance(&snapshot, |command| player.control(command).map(|_| ())) {
                                    Ok(true) => { restore = None; continue; }
                                    Ok(false) => { thread::sleep(Duration::from_millis(150)); continue; }
                                    Err(error) => {
                                        publish(Status::new(Phase::Failed, format!("Could not restore Spotify after output change: {error}")));
                                        return;
                                    }
                                }
                            }
                            let snapshot = Arc::new(snapshot);
                            if let Ok(mut current) = snapshots.lock() {
                                *current = Some(snapshot.clone());
                            }
                            if snapshot.connected {
                                last_connected = Instant::now();
                            } else if last_connected.elapsed() >= Duration::from_secs(15) {
                                publish(Status::new(
                                    Phase::Failed,
                                    "Could not reconnect to Soloist. Try starting Spotify again.",
                                ));
                                return;
                            }
                            let mut status = connection_status(&snapshot);
                            if status.phase == Phase::Pairing {
                                let since = awaiting_since.get_or_insert_with(Instant::now);
                                if since.elapsed() < Duration::from_secs(2) {
                                    status = Status::new(
                                        Phase::Connecting,
                                        "Restoring Spotify session...",
                                    );
                                }
                            } else {
                                awaiting_since = None;
                            }
                            publish(status);
                        }
                        Err(error) => {
                            publish(Status::new(Phase::Failed, error));
                            return;
                        }
                    }
                }
                drop(_published);
                drop(player);
                publish(Status::new(Phase::Stopped, "Spotify playback stopped."));
            })
            .map_err(|_| "Could not start Spotify pairing")?;
        Ok(Self {
            status,
            stop,
            worker: Some(worker),
            handle,
        })
    }
    pub fn status(&self) -> Status {
        self.status
            .lock()
            .map(|status| status.clone())
            .unwrap_or_else(|_| Status::new(Phase::Failed, "Spotify pairing status unavailable."))
    }
}
impl Drop for Service {
    fn drop(&mut self) {
        // Startup may still be waiting for the daemon. Never stall the UI/shutdown by joining it.
        self.worker.take();
    }
}
fn connection_status(snapshot: &Snapshot) -> Status {
    if !snapshot.connected {
        Status::new(Phase::Connecting, "Connecting to Soloist...")
    } else if !snapshot.logged_in {
        Status::new(
            Phase::Pairing,
            "In Spotify on this network, select DeezNats in the device picker.",
        )
    } else {
        Status::new(Phase::Ready, "Spotify paired and ready.")
    }
}
struct OutputRecovery {
    previous: Snapshot,
    started: Instant,
    requested: bool,
}
impl OutputRecovery {
    fn new(previous: Snapshot) -> Self {
        Self {
            previous,
            started: Instant::now(),
            requested: false,
        }
    }
    fn advance(
        &mut self,
        current: &Snapshot,
        mut send: impl FnMut(&Control) -> Result<(), String>,
    ) -> Result<bool, String> {
        if self.started.elapsed() >= Duration::from_secs(15) {
            return Err("Playback restoration timed out; start Spotify again".into());
        }
        if !current.connected || !current.logged_in {
            return Ok(false);
        }
        let previous = &self.previous;
        let Some(uri) = previous.item["uri"].as_str() else {
            return Ok(true);
        };
        if !self.requested {
            send(&Control::Volume(previous.volume))?;
            send(&Control::Activate)?;
            send(&Control::Play(Some(uri.into())))?;
            self.requested = true;
            return Ok(false);
        }
        // Play is asynchronous: never seek into a stale or still-loading item.
        if current.item["uri"].as_str() != Some(uri)
            || !matches!(current.status.as_str(), "playing" | "paused")
        {
            return Ok(false);
        }
        send(&Control::Seek(
            previous.position_ms.min(u32::MAX as u64) as u32
        ))?;
        send(&Control::Shuffle(previous.shuffle))?;
        send(&Control::Repeat(previous.repeat.clone()))?;
        for entry in previous.upcoming.iter().take(80) {
            if let Some(uri) = entry["item"]["uri"]
                .as_str()
                .filter(|uri| uri.starts_with("spotify:track:"))
            {
                send(&Control::AddToQueue(uri.into()))?;
            }
        }
        if previous.status == "paused" {
            send(&Control::Pause)?;
        }
        Ok(true)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn output_recovery_waits_for_auth_and_track_before_restoring_position_and_pause() {
        let previous = Snapshot {
            item: serde_json::json!({"uri":"spotify:track:1234567890123456789012"}),
            position_ms: 3456,
            volume: 42,
            shuffle: true,
            repeat: "track".into(),
            status: "paused".into(),
            ..Snapshot::default()
        };
        let mut recovery = OutputRecovery::new(previous.clone());
        let mut commands = Vec::new();
        let mut send = |command: &Control| {
            commands.push(command.clone());
            Ok(())
        };
        assert!(!recovery.advance(&Snapshot::default(), &mut send).unwrap());
        let mut current = Snapshot {
            connected: true,
            logged_in: true,
            ..Snapshot::default()
        };
        assert!(!recovery.advance(&current, &mut send).unwrap());
        assert!(!recovery.advance(&current, &mut send).unwrap());
        current.item = previous.item.clone();
        current.status = "buffering".into();
        assert!(!recovery.advance(&current, &mut send).unwrap());
        current.status = "playing".into();
        assert!(recovery.advance(&current, &mut send).unwrap());
        assert_eq!(
            commands,
            vec![
                Control::Volume(42),
                Control::Activate,
                Control::Play(Some("spotify:track:1234567890123456789012".into())),
                Control::Seek(3456),
                Control::Shuffle(true),
                Control::Repeat("track".into()),
                Control::Pause,
            ]
        );
    }
    #[test]
    fn output_recovery_has_a_deadline_and_propagates_command_failure() {
        let previous = Snapshot {
            item: serde_json::json!({"uri":"spotify:track:1234567890123456789012"}),
            ..Snapshot::default()
        };
        let current = Snapshot {
            connected: true,
            logged_in: true,
            ..Snapshot::default()
        };
        let mut recovery = OutputRecovery::new(previous);
        assert_eq!(
            recovery.advance(&current, |_| Err("device unavailable".into())),
            Err("device unavailable".into())
        );
        recovery.started = Instant::now() - Duration::from_secs(16);
        assert!(recovery
            .advance(&current, |_| panic!(
                "expired recovery must not send controls"
            ))
            .is_err());
    }
    #[test]
    fn authentication_events_determine_pairing_and_do_not_require_active_playback() {
        let mut snapshot = Snapshot::default();
        assert_eq!(connection_status(&snapshot).phase, Phase::Connecting);
        snapshot.connected = true;
        assert_eq!(connection_status(&snapshot).phase, Phase::Pairing);
        snapshot.logged_in = true;
        snapshot.is_active = false;
        assert_eq!(connection_status(&snapshot).phase, Phase::Ready);
        snapshot.connected = false;
        assert_eq!(connection_status(&snapshot).phase, Phase::Connecting);
    }
    #[test]
    fn failed_startup_is_reported_without_blocking_the_caller() {
        let mut service = Service::start_with(|_| Err("fixture failure".into())).unwrap();
        service.worker.take().unwrap().join().unwrap();
        assert_eq!(
            service.status(),
            Status::new(Phase::Failed, "fixture failure")
        );
    }
    #[test]
    fn cancellation_during_startup_cannot_publish_ready() {
        let mut service = Service::start_with(|_| {
            thread::sleep(Duration::from_millis(50));
            Err("cancelled startup".into())
        })
        .unwrap();
        service.stop.store(true, Ordering::Relaxed);
        service.worker.take().unwrap().join().unwrap();
        assert_eq!(service.status().phase, Phase::Stopped);
    }
    #[test]
    fn detects_first_pairing_and_restores_the_saved_session_on_restart() {
        use std::{
            os::windows::fs::OpenOptionsExt,
            path::PathBuf,
            process::Command,
            time::{Instant, SystemTime, UNIX_EPOCH},
        };
        struct Temporary(PathBuf);
        impl Drop for Temporary {
            fn drop(&mut self) {
                if self.0.parent().and_then(|p| p.canonicalize().ok())
                    == std::env::temp_dir().canonicalize().ok()
                    && self
                        .0
                        .file_name()
                        .unwrap()
                        .to_string_lossy()
                        .starts_with("deeznats-pairing-test-")
                {
                    let _ = std::fs::remove_dir_all(&self.0);
                }
            }
        }
        let temporary = Temporary(std::env::temp_dir().join(format!(
                "deeznats-pairing-test-{}-{}",
                std::process::id(),
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            )));
        std::fs::create_dir(&temporary.0).unwrap();
        let executable = temporary.0.join("fixture.exe");
        let mut compiler = Command::new("rustc");
        compiler
            .arg("--edition=2021")
            .arg(
                std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("tests/fixtures/soloist_mock.rs"),
            )
            .arg("-o")
            .arg(&executable);
        assert!(
            super::super::process::capture(&mut compiler, Duration::from_secs(30))
                .unwrap()
                .0
                .success()
        );
        let data = temporary.0.join("data");
        let start = || {
            let config = Config {
                executable: executable.clone(),
                api_key: "pair-later".into(),
            };
            let data = data.clone();
            Service::start_with(move |_| {
                Soloist::start_at(config.clone(), data.clone(), Duration::from_secs(5))
            })
            .unwrap()
        };
        let mut service = start();
        let began = Instant::now();
        while service.status().phase != Phase::Pairing {
            assert!(
                began.elapsed() < Duration::from_secs(8),
                "{:?}",
                service.status()
            );
            thread::sleep(Duration::from_millis(20));
        }
        std::fs::write(data.join("fixture-pairing"), "paired").unwrap();
        let began = Instant::now();
        while service.status().phase != Phase::Ready {
            assert!(began.elapsed() < Duration::from_secs(3));
            thread::sleep(Duration::from_millis(20));
        }
        let original_pid = service
            .handle
            .audio_target
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .pid;
        service.handle.reopen_output.store(true, Ordering::Release);
        let began = Instant::now();
        loop {
            let pid = service
                .handle
                .audio_target
                .lock()
                .unwrap()
                .as_ref()
                .map(|target| target.pid);
            if pid.is_some_and(|pid| pid != original_pid) && service.status().phase == Phase::Ready
            {
                break;
            }
            assert!(
                began.elapsed() < Duration::from_secs(10),
                "output recovery: {:?}",
                service.status()
            );
            thread::sleep(Duration::from_millis(20));
        }
        service.stop.store(true, Ordering::Relaxed);
        service.worker.take().unwrap().join().unwrap();
        let lock = std::fs::OpenOptions::new()
            .write(true)
            .share_mode(0)
            .open(data.join("deeznats.lock"))
            .unwrap();
        drop(lock);
        let mut restored = start();
        let began = Instant::now();
        while restored.status().phase != Phase::Ready {
            assert_ne!(
                restored.status().phase,
                Phase::Pairing,
                "saved pairing must not prompt for a new pairing"
            );
            assert!(began.elapsed() < Duration::from_secs(5));
            thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(
            std::fs::read_to_string(data.join("fixture-pairing")).unwrap(),
            "paired"
        );
        restored.stop.store(true, Ordering::Relaxed);
        restored.worker.take().unwrap().join().unwrap();
    }
}
