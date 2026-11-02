//! Engine operations run on a bounded worker queue, away from the window thread.
use crate::engine::Engine as Deezer;

#[path = "audio/autoplay.rs"]
mod autoplay;
#[path = "audio/discord.rs"]
mod discord;
use std::{
    ffi::CString,
    sync::{mpsc, Arc},
    time::Duration,
};
#[derive(Clone)]
pub enum Command {
    SwitchProvider(crate::catalog::Provider),
    Login(String),
    LoginSaved,
    Logout,
    Play(String),
    PlayEntry(Box<crate::catalog::Entry>, Vec<crate::catalog::Entry>),
    PlayQueueIndex(usize),
    QueueClear,
    QueueMove {
        from: usize,
        to: usize,
        expected: Vec<String>,
    },
    QueueInsertNext(Box<crate::catalog::Entry>),
    QueueAddLast(Box<crate::catalog::Entry>),
    Next,
    Previous,
    Shuffle(bool),
    Repeat(u8),
    Autoplay(bool),
    Api(String, Vec<serde_json::Value>),
    Discord(serde_json::Value),
    Toggle,
    SetPlaying(bool),
    Stop,
    Seek(u32),
    SeekMs(u64),
    Volume(u32),
    Quit,
}
#[derive(Clone, Default)]
pub struct Snapshot {
    pub provider: crate::catalog::Provider,
    pub playback_connected: bool,
    pub message: String,
    pub unavailable_at: Option<std::time::Instant>,
    pub title: String,
    pub state: String,
    pub position: u64,
    pub duration: u64,
    pub connected: bool,
    pub login_attempt: u64,
    pub engine: Option<Arc<Deezer>>,
    pub account: serde_json::Value,
    pub now: crate::catalog::Entry,
    pub queue: Arc<Vec<crate::catalog::Entry>>,
    pub queue_index: usize,
    pub quality: String,
    pub repeat: u8,
    pub shuffle: bool,
    pub autoplay: bool,
    pub radio_active: bool,
    pub eq: serde_json::Value,
    pub discord_status: String,
    pub discord_settings: serde_json::Value,
}
// Collection tracks are the fallback after explicitly queued items.
pub(crate) const QUEUE_FALLBACK: &str = "_queue_fallback";
pub(crate) fn mark_queue_fallback(items: &mut [crate::catalog::Entry]) {
    for item in items {
        if !item.raw.is_object() {
            item.raw = serde_json::json!({});
        }
        item.raw[QUEUE_FALLBACK] = serde_json::json!(true);
    }
}
pub(crate) fn queue_add_position(items: &[crate::catalog::Entry], index: usize) -> usize {
    let start = index.saturating_add(1).min(items.len());
    items[start..]
        .iter()
        .position(|item| item.raw[QUEUE_FALLBACK] == true)
        .map_or(items.len(), |offset| start + offset)
}
pub(crate) fn explicit_queue_entry(mut item: crate::catalog::Entry) -> crate::catalog::Entry {
    if let Some(raw) = item.raw.as_object_mut() {
        raw.remove(QUEUE_FALLBACK);
    }
    item
}
#[cfg(test)]
mod queue_priority_tests {
    use super::*;
    #[test]
    fn queued_tracks_keep_order_before_collection_fallback() {
        let entry = |id: &str| crate::catalog::Entry {
            id: id.into(),
            kind: "track".into(),
            ..Default::default()
        };
        let mut queue = vec![entry("current"), entry("album1"), entry("album2")];
        mark_queue_fallback(&mut queue);
        for id in ["queued1", "queued2"] {
            let at = queue_add_position(&queue, 0);
            queue.insert(at, explicit_queue_entry(entry(id)));
        }
        assert_eq!(
            queue.iter().map(|e| e.id.as_str()).collect::<Vec<_>>(),
            ["current", "queued1", "queued2", "album1", "album2"]
        );
        assert_eq!(queue_add_position(&queue, 1), 3);
        assert_eq!(
            queue_add_position(&[entry("current"), entry("queued")], 0),
            2
        );
    }
}

pub struct Worker {
    pub commands: mpsc::SyncSender<Command>,
    pub updates: mpsc::Receiver<Snapshot>,
}
// Queue order and repeated tracks are meaningful; catalog::entries deliberately
// deduplicates browse results and must not be used for this wire format.
fn adopt_queue(snapshot: &mut Snapshot, value: &serde_json::Value) -> Result<i64, String> {
    let tracks = value
        .get("tracks")
        .and_then(serde_json::Value::as_array)
        .ok_or("Engine returned an invalid queue")?;
    let mut entries = tracks
        .iter()
        .map(|track| {
            crate::catalog::entry(track, "track")
                .ok_or_else(|| "Engine returned an invalid queue track".to_owned())
        })
        .collect::<Result<Vec<_>, _>>()?;
    let version = value
        .get("version")
        .and_then(serde_json::Value::as_i64)
        .ok_or("Engine returned no queue version")?;
    // The engine owns queue order but does not preserve Connect attribution.
    // Match occurrences separately so repeated tracks retain separate authors.
    let mut previous = snapshot.queue.iter().collect::<Vec<_>>();
    for entry in &mut entries {
        if let Some(index) = previous
            .iter()
            .position(|old| old.id == entry.id && old.kind == entry.kind)
        {
            let old = previous.remove(index);
            if let Some(name) = old.raw["added_by"].as_str() {
                entry.raw["added_by"] = serde_json::json!(name);
            }
            if let Some(fallback) = old.raw.get(QUEUE_FALLBACK) {
                entry.raw[QUEUE_FALLBACK] = fallback.clone();
            }
            if let Some(selection) = old.raw.get(crate::style_mix::MARKER) {
                entry.raw[crate::style_mix::MARKER] = selection.clone();
            }
        }
    }
    snapshot.queue_index = value
        .get("index")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0)
        .min(entries.len().saturating_sub(1) as u64) as usize;
    snapshot.queue = Arc::new(entries);
    Ok(version)
}

fn sync_queue(dz: &Deezer, snapshot: &mut Snapshot, version: &mut i64) -> Result<(), String> {
    let current = dz
        .invoke("DZQueueVersion", &[])?
        .as_i64()
        .ok_or("Invalid queue version")?;
    if current != *version {
        // Commit the version only after a successful read, so transient failures retry.
        *version = adopt_queue(snapshot, &dz.invoke("DZQueueJSON", &[])?)?;
    }
    Ok(())
}

fn set_queue(dz: &Deezer, tracks: &[crate::catalog::Entry], index: usize) -> Result<(), String> {
    let wire: Vec<_> = tracks.iter().map(crate::catalog::Entry::wire).collect();
    let accepted = dz.invoke(
        "DZQueueSet",
        &[serde_json::json!(
            serde_json::to_string(&wire).map_err(|e| e.to_string())?
        )],
    )?;
    if accepted.as_i64() != Some(1) {
        return Err("Engine rejected the queue".into());
    }
    if !tracks.is_empty() {
        dz.invoke("DZQueueSetIndex", &[serde_json::json!(index)])?;
    }
    Ok(())
}
fn reordered_queue(
    tracks: &[crate::catalog::Entry],
    current: usize,
    from: usize,
    to: usize,
    expected: &[String],
) -> Result<Vec<crate::catalog::Entry>, String> {
    if tracks.iter().map(|e| &e.id).ne(expected.iter()) {
        return Err("Queue changed while dragging; try again".into());
    }
    if from <= current || to <= current || from >= tracks.len() || to >= tracks.len() {
        return Err("Only upcoming tracks can be reordered".into());
    }
    let mut tracks = tracks.to_vec();
    let track = tracks.remove(from);
    tracks.insert(to, track);
    Ok(tracks)
}
// The engine retains its selected WASAPI endpoint. Watch Windows on the audio
// thread so changing the system default also moves an already playing stream.
struct DefaultOutput {
    enumerator: Option<windows::Win32::Media::Audio::IMMDeviceEnumerator>,
    endpoint: Option<String>,
    com_initialized: bool,
    follow: bool,
    checked: std::time::Instant,
}
impl DefaultOutput {
    fn new() -> Self {
        use windows::Win32::{
            Media::Audio::{IMMDeviceEnumerator, MMDeviceEnumerator},
            System::Com::{CoCreateInstance, CoInitializeEx, CLSCTX_ALL, COINIT_MULTITHREADED},
        };
        let com_initialized = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED).is_ok() };
        let enumerator = unsafe {
            CoCreateInstance::<_, IMMDeviceEnumerator>(&MMDeviceEnumerator, None, CLSCTX_ALL).ok()
        };
        Self {
            enumerator,
            com_initialized,
            endpoint: None,
            follow: true,
            checked: std::time::Instant::now() - Duration::from_secs(2),
        }
    }
    fn update(&mut self, dz: &Deezer) -> Result<(), String> {
        if !self.follow || self.checked.elapsed() < Duration::from_secs(1) {
            return Ok(());
        }
        self.checked = std::time::Instant::now();
        use windows::Win32::{
            Media::Audio::{eMultimedia, eRender},
            System::Com::CoTaskMemFree,
        };
        let Some(enumerator) = &self.enumerator else {
            return Ok(());
        };
        let endpoint = unsafe {
            let Ok(device) = enumerator.GetDefaultAudioEndpoint(eRender, eMultimedia) else {
                self.endpoint = None;
                return Ok(());
            };
            let id = device.GetId().map_err(|e| e.to_string())?;
            let text = id.to_string().map_err(|e| e.to_string());
            CoTaskMemFree(Some(id.0.cast()));
            text?
        };
        self.route(endpoint, || {
            // Engine IDs are opaque miniaudio IDs, not Windows endpoint IDs.
            // The empty ID reopens the current system default.
            if dz
                .invoke("DZSetAudioDevice", &[serde_json::json!("")])?
                .as_i64()
                != Some(1)
            {
                return Err("Could not switch to the Windows default audio output".into());
            }
            Ok(())
        })
    }
    fn route(
        &mut self,
        endpoint: String,
        switch: impl FnOnce() -> Result<(), String>,
    ) -> Result<(), String> {
        if self.follow && self.endpoint.as_ref() != Some(&endpoint) {
            switch()?;
            self.endpoint = Some(endpoint);
        }
        Ok(())
    }
    fn select(&mut self, dz: &Deezer, args: &[serde_json::Value]) -> Result<(), String> {
        let id = args
            .first()
            .and_then(|v| v.as_str())
            .ok_or("Missing audio device")?;
        if id.is_empty() {
            self.follow = true;
            self.endpoint = None;
            self.checked = std::time::Instant::now() - Duration::from_secs(2);
            self.update(dz)
        } else {
            if dz.invoke("DZSetAudioDevice", args)?.as_i64() != Some(1) {
                return Err("Could not switch audio output".into());
            }
            self.follow = false;
            Ok(())
        }
    }
}
impl Drop for DefaultOutput {
    fn drop(&mut self) {
        self.enumerator.take();
        if self.com_initialized {
            unsafe {
                windows::Win32::System::Com::CoUninitialize();
            }
        }
    }
}
#[cfg(test)]
mod output_tests {
    use super::*;
    #[test]
    fn default_route_changes_once_retries_failures_and_respects_explicit_selection() {
        let mut output = DefaultOutput {
            enumerator: None,
            endpoint: None,
            com_initialized: false,
            follow: true,
            checked: std::time::Instant::now(),
        };
        let mut switches = 0;
        output
            .route("speakers".into(), || {
                switches += 1;
                Ok(())
            })
            .unwrap();
        output
            .route("speakers".into(), || {
                panic!("unchanged endpoint");
            })
            .unwrap();
        assert_eq!(switches, 1);
        assert!(output
            .route("headphones".into(), || Err("unplugged".into()))
            .is_err());
        assert_eq!(output.endpoint.as_deref(), Some("speakers"));
        output
            .route("headphones".into(), || {
                switches += 1;
                Ok(())
            })
            .unwrap();
        assert_eq!(switches, 2);
        output.follow = false;
        output
            .route("speakers".into(), || {
                panic!("explicit selection must stay selected");
            })
            .unwrap();
        assert_eq!(output.endpoint.as_deref(), Some("headphones"));
        output.follow = true;
        output.endpoint = None; // Windows temporarily had no output attached.
        output
            .route("headphones".into(), || {
                switches += 1;
                Ok(())
            })
            .unwrap();
        assert_eq!(switches, 3);
    }
}
impl Worker {
    pub fn start() -> Self {
        let (commands, rx) = mpsc::sync_channel(32);
        let (tx, updates) = mpsc::sync_channel(1);
        std::thread::spawn(move || {
            let discord = discord::Worker::start();
            let mut output = DefaultOutput::new();
            let mut engine: Option<Arc<Deezer>> = None;
            let mut spotify: Option<crate::spotify::service::Service> = None;
            let mut spotify_queue = crate::spotify::playback::Queue::default();
            let mut spotify_remote = crate::spotify::remote::Worker::new();
            let mut spotify_pending: Option<(Command, std::time::Instant)> = None;
            let mut spotify_activation = std::time::Instant::now() - Duration::from_secs(2);
            let (account_tx, account_rx) =
                mpsc::channel::<(u64, Result<serde_json::Value, String>)>();
            let mut account_epoch = 0u64;
            let mut deezer_snapshot: Option<Snapshot> = None;

            let mut failed_index: Option<usize> = None;
            let mut handled_error: Option<usize> = None;
            let mut volume = 100;
            let mut queue_version: i64 = -1;
            let mut autoplay = autoplay::Autoplay::default();
            let mut snapshot = Snapshot {
                autoplay: true,
                discord_settings: discord.settings(),
                message: "Connect your Deezer account to start listening.".into(),
                state: "Stopped".into(),
                ..Default::default()
            };
            let mut deferred_command = None;
            loop {
                let mut command = deferred_command
                    .take()
                    .map(Ok)
                    .unwrap_or_else(|| rx.recv_timeout(Duration::from_millis(200)));
                if matches!(command, Ok(Command::Volume(_))) {
                    while let Ok(next) = rx.try_recv() {
                        if matches!(next, Command::Volume(_)) {
                            command = Ok(next);
                        } else {
                            deferred_command = Some(next);
                            break;
                        }
                    }
                }
                while let Ok((epoch, result)) = account_rx.try_recv() {
                    if epoch == account_epoch
                        && snapshot.provider == crate::catalog::Provider::Spotify
                    {
                        match result {
                            Ok(account) => {
                                snapshot.connected = true;
                                snapshot.account = account;
                            }
                            Err(error) => {
                                snapshot.connected = false;
                                snapshot.account = serde_json::Value::Null;
                                snapshot.message = error;
                            }
                        }
                    }
                }
                if let Ok(Command::SwitchProvider(provider)) = &command {
                    account_epoch = account_epoch.wrapping_add(1);
                    spotify_remote.enable(false, account_epoch);
                    let provider = *provider;
                    if provider != snapshot.provider {
                        crate::spotify::remote::select_device(None);
                        spotify_pending = None;
                        spotify_queue = Default::default();
                        autoplay.cancel(true);
                        if snapshot.provider == crate::catalog::Provider::Deezer {
                            deezer_snapshot = Some(snapshot.clone());
                            if let Some(dz) = &engine {
                                let _ = dz.invoke("DZPause", &[]);
                            }
                        } else if let Some(service) = &spotify {
                            let _ = service
                                .handle()
                                .control(&crate::spotify::soloist::Control::Pause);
                        }
                        snapshot = if provider == crate::catalog::Provider::Deezer {
                            deezer_snapshot.take().unwrap_or_default()
                        } else {
                            Snapshot::default()
                        };
                        snapshot.provider = provider;
                        snapshot.state = "Stopped".into();
                    }
                    provider.set_active();
                    if provider == crate::catalog::Provider::Spotify {
                        snapshot.connected = false;
                        snapshot.account = serde_json::Value::Null;
                        let reply = account_tx.clone();
                        let epoch = account_epoch;
                        std::thread::spawn(move || {
                            let _ = reply.send((epoch, crate::spotify::catalog_account()));
                        });
                        if spotify.as_ref().is_none_or(|s| {
                            matches!(
                                s.status().phase,
                                crate::spotify::service::Phase::Failed
                                    | crate::spotify::service::Phase::Stopped
                            )
                        }) {
                            match crate::spotify::service::Service::start() {
                                Ok(service) => spotify = Some(service),
                                Err(error) => snapshot.message = error,
                            }
                        }
                    }
                }
                if snapshot.provider == crate::catalog::Provider::Spotify {
                    if let Ok(Command::Discord(value)) = &command {
                        match discord.configure(value) {
                            Ok(settings) => snapshot.discord_settings = settings,
                            Err(error) => snapshot.message = error,
                        }
                        discord.update(&snapshot);
                        let _ = tx.try_send(snapshot.clone());
                        continue;
                    }
                    if let Some(status) = discord.status() {
                        snapshot.discord_status = status;
                    }
                    if matches!(
                        &command,
                        Ok(Command::Quit) | Err(mpsc::RecvTimeoutError::Disconnected)
                    ) {
                        spotify_remote.enable(false, account_epoch);
                        break;
                    }
                    let handle = spotify.as_ref().map(|service| service.handle());
                    let local_active = handle.as_ref().is_some_and(|handle| {
                        !matches!(
                            handle.status().phase,
                            crate::spotify::service::Phase::Failed
                                | crate::spotify::service::Phase::Stopped
                        ) && handle
                            .snapshot()
                            .is_some_and(|state| state.connected && state.logged_in)
                    });
                    let explicit_remote = crate::spotify::remote::selected_device().is_some();
                    spotify_remote.enable(explicit_remote, account_epoch);
                    if !explicit_remote && !local_active {
                        spotify_queue = Default::default();
                        snapshot.playback_connected = false;
                        snapshot.state = "Stopped".into();
                        snapshot.now = Default::default();
                        snapshot.title.clear();
                        snapshot.queue = Default::default();
                        snapshot.position = 0;
                        snapshot.duration = 0;
                        if let Some(handle) = &handle {
                            let status = handle.status();
                            snapshot.message = status.message;
                            if let Ok(cmd) = &command {
                                if matches!(
                                    cmd,
                                    Command::Play(_)
                                        | Command::PlayEntry(..)
                                        | Command::PlayQueueIndex(_)
                                        | Command::SetPlaying(true)
                                        | Command::Toggle
                                ) {
                                    spotify_pending =
                                        Some((cmd.clone(), std::time::Instant::now()));
                                } else if matches!(cmd, Command::Stop | Command::SetPlaying(false))
                                {
                                    spotify_pending = None;
                                    let _ =
                                        handle.control(&crate::spotify::soloist::Control::Pause);
                                } else if let Command::Api(name, args) = cmd {
                                    if let Err(error) = crate::spotify::catalog_invoke(name, args) {
                                        snapshot.message = error;
                                    }
                                }
                            }
                            if matches!(
                                status.phase,
                                crate::spotify::service::Phase::Pairing
                                    | crate::spotify::service::Phase::Failed
                                    | crate::spotify::service::Phase::Stopped
                            ) {
                                spotify_pending = None;
                            } else if spotify_pending.as_ref().is_some_and(|(_, started)| {
                                started.elapsed() >= Duration::from_secs(15)
                            }) {
                                spotify_pending = None;
                                snapshot.message = "Soloist did not become the active device; select DeezNats in Spotify and retry".into();
                            } else if spotify_pending.is_some()
                                && handle
                                    .snapshot()
                                    .is_some_and(|s| s.connected && s.logged_in)
                                && spotify_activation.elapsed() >= Duration::from_secs(2)
                            {
                                spotify_activation = std::time::Instant::now();
                                if let Err(error) =
                                    handle.control(&crate::spotify::soloist::Control::Activate)
                                {
                                    spotify_pending = None;
                                    snapshot.message = error;
                                }
                            }
                        } else {
                            spotify_pending = None;
                            snapshot.message = "Set up and pair Soloist for local playback. Remote devices are used only after you select one.".into();
                        }
                    } else if !explicit_remote
                        && handle
                            .as_ref()
                            .is_some_and(|h| h.snapshot().is_some_and(|s| !s.is_active))
                        && !matches!(&command, Ok(Command::Stop | Command::SetPlaying(false)))
                        && (spotify_pending.is_some()
                            || matches!(
                                &command,
                                Ok(Command::Play(_)
                                    | Command::PlayEntry(..)
                                    | Command::PlayQueueIndex(_)
                                    | Command::SetPlaying(true)
                                    | Command::Toggle)
                            ))
                    {
                        let handle = handle.as_ref().unwrap();
                        if let Ok(
                            cmd @ (Command::Play(_)
                            | Command::PlayEntry(..)
                            | Command::PlayQueueIndex(_)
                            | Command::SetPlaying(true)
                            | Command::Toggle),
                        ) = &command
                        {
                            spotify_pending = Some((cmd.clone(), std::time::Instant::now()));
                        }
                        if spotify_pending.as_ref().is_some_and(|(_, started)| {
                            started.elapsed() >= Duration::from_secs(15)
                        }) {
                            spotify_pending = None;
                            snapshot.message = "Soloist did not become active. Check your Spotify connection and retry.".into();
                        } else if spotify_activation.elapsed() >= Duration::from_secs(2) {
                            spotify_activation = std::time::Instant::now();
                            if let Err(error) =
                                handle.control(&crate::spotify::soloist::Control::Activate)
                            {
                                spotify_pending = None;
                                snapshot.message = error;
                            }
                        }
                    } else if !explicit_remote {
                        let handle = handle.unwrap();
                        if matches!(
                            &command,
                            Ok(Command::Play(_)
                                | Command::PlayEntry(..)
                                | Command::PlayQueueIndex(_)
                                | Command::Toggle
                                | Command::Stop
                                | Command::SetPlaying(_))
                        ) {
                            spotify_pending = None;
                        }
                        // A transfer back must use local timing/state for this command.
                        crate::spotify::playback::update(
                            &mut snapshot,
                            &handle,
                            &mut spotify_queue,
                        );
                        if let Some((pending, _)) = spotify_pending.take() {
                            if let Err(error) = crate::spotify::playback::dispatch(
                                &pending,
                                &handle,
                                &mut snapshot,
                                &mut spotify_queue,
                            ) {
                                snapshot.message = error;
                            }
                        }
                        if let Ok(command) = &command {
                            if let Err(error) = crate::spotify::playback::dispatch(
                                command,
                                &handle,
                                &mut snapshot,
                                &mut spotify_queue,
                            ) {
                                crate::spotify_diagnostics::record(
                                    "playback.dispatch.error",
                                    crate::spotify_diagnostics::error_kind(&error),
                                );
                                snapshot.message = error;
                            }
                        }
                        crate::spotify::playback::update(
                            &mut snapshot,
                            &handle,
                            &mut spotify_queue,
                        );
                        spotify_queue.refill(&mut snapshot);
                    } else {
                        spotify_pending = None;
                        // Never refill or mutate a managed local queue after transfer away.
                        spotify_queue = Default::default();
                        spotify_remote.update(&mut snapshot);
                        if let Ok(command) = &command {
                            if let Err(error) = spotify_remote.dispatch(command, &snapshot) {
                                snapshot.message = error;
                            }
                        }
                    }
                    discord.update(&snapshot);
                    let _ = tx.try_send(snapshot.clone());
                    continue;
                }
                // Stale UI/remote requests cannot feed Spotify identities into the native ABI.
                let spotify_identity = match &command {
                    Ok(Command::Play(id)) => id.starts_with("spotify:"),
                    Ok(Command::PlayEntry(item, items)) => {
                        item.id.starts_with("spotify:")
                            || items.iter().any(|i| i.id.starts_with("spotify:"))
                    }
                    Ok(Command::QueueInsertNext(item) | Command::QueueAddLast(item)) => {
                        item.id.starts_with("spotify:")
                    }
                    _ => false,
                };
                if spotify_identity {
                    snapshot.message = "Switch to Spotify before playing this item".into();
                    let _ = tx.try_send(snapshot.clone());
                    continue;
                }
                if matches!(&command, Ok(Command::Login(_)) | Ok(Command::LoginSaved)) {
                    snapshot.login_attempt += 1;
                }
                if matches!(&command, Ok(Command::LoginSaved)) {
                    match crate::session::saved() {
                        Ok(secret) => command = Ok(Command::Login(secret)),
                        Err(error) => {
                            snapshot.message = error;
                            let _ = tx.try_send(snapshot.clone());
                            continue;
                        }
                    }
                }
                if matches!(
                    &command,
                    Ok(Command::Quit) | Err(mpsc::RecvTimeoutError::Disconnected)
                ) {
                    break;
                }
                let queue_insert_next = matches!(&command, Ok(Command::QueueInsertNext(_)));
                let allow_autoplay = matches!(
                    &command,
                    Err(mpsc::RecvTimeoutError::Timeout)
                        | Ok(Command::Volume(_))
                        | Ok(Command::Discord(_))
                );
                if !allow_autoplay {
                    let reset = matches!(
                        &command,
                        Ok(Command::Play(_))
                            | Ok(Command::PlayEntry(..))
                            | Ok(Command::PlayQueueIndex(_))
                            | Ok(Command::QueueClear)
                            | Ok(Command::Login(_))
                            | Ok(Command::Logout)
                    );
                    autoplay.cancel(reset);
                }
                let outcome: Result<(), String> = (|| {
                    match command {
                        Ok(Command::Login(arl)) => {
                            if engine.is_none() {
                                engine = Some(Arc::new(Deezer::load()?));
                            }
                            let dz = engine.as_ref().unwrap();
                            let secret = CString::new(arl.trim()).map_err(|e| e.to_string())?;
                            let authenticated = unsafe { (dz.init)(secret.as_ptr()) != 0 };
                            snapshot.connected = false;
                            if !authenticated {
                                let failure = unsafe { (dz.login_error)() };
                                if failure == 2 && !crate::downloads::load_offline().is_empty() {
                                    dz.init_offline(&secret)?;
                                    snapshot.account = serde_json::Value::Null;
                                    snapshot.connected = true;
                                    snapshot.engine = Some(dz.clone());
                                    snapshot.eq = dz.invoke("DZEQJSON", &[]).unwrap_or_default();
                                    queue_version = -1;
                                    output.endpoint = None;
                                    output.checked =
                                        std::time::Instant::now() - Duration::from_secs(2);
                                    unsafe {
                                        (dz.volume)(volume as f64 / 100.0);
                                    }
                                    snapshot.message =
                                        "Offline — playing downloaded Deezer tracks".into();
                                    return Ok(());
                                }
                                return Err(match failure {
                                    1 => "Session expired or invalid",
                                    2 => "No internet connection",
                                    4 => "Audio output unavailable",
                                    _ => "Sign-in failed",
                                }
                                .into());
                            }
                            let mut account = dz.invoke("DZAccountJSON", &[])?;
                            if account.get("loggedIn").and_then(serde_json::Value::as_bool)
                                != Some(true)
                                || crate::catalog::string(&account, &["userId"]).is_empty()
                            {
                                snapshot.connected = false;
                                return Err(
                                    "Deezer did not return a verified account identity.".into()
                                );
                            }
                            let user_id = crate::catalog::string(&account, &["userId"]);
                            if user_id.bytes().all(|b| b.is_ascii_digit()) {
                                if let Ok(bytes) =
                                    dz.fetch(&format!("https://api.deezer.com/user/{user_id}"))
                                {
                                    if let Ok(profile) =
                                        serde_json::from_slice::<serde_json::Value>(&bytes)
                                    {
                                        if let Some(picture) = profile.get("picture_medium") {
                                            account["picture"] = picture.clone();
                                        }
                                    }
                                }
                            }
                            snapshot.account = account;
                            output.endpoint = None;
                            output.checked = std::time::Instant::now() - Duration::from_secs(2);
                            snapshot.connected = true;
                            queue_version = -1;
                            snapshot.engine = Some(dz.clone());
                            snapshot.eq = dz.invoke("DZEQJSON", &[]).unwrap_or_default();
                            let _ = crate::session::save(arl.trim());
                            unsafe {
                                (dz.volume)(volume as f64 / 100.0);
                            }
                            snapshot.message = format!(
                                "Connected to Deezer - {} ({})",
                                crate::catalog::string(&snapshot.account, &["name"]),
                                crate::catalog::string(&snapshot.account, &["offer"])
                            );
                        }
                        Ok(Command::Logout) => {
                            if let Some(dz) = &engine {
                                unsafe {
                                    (dz.logout)();
                                }
                            }
                            snapshot.connected = false;
                            snapshot.engine = None;
                            snapshot.account = serde_json::Value::Null;
                            snapshot.queue = Arc::new(Vec::new());
                            snapshot.queue_index = 0;
                            snapshot.now = Default::default();
                            queue_version = -1;
                            snapshot.message = match crate::session::logout() {
                                Ok(()) => "Signed out of the Rust app. Your original DeezNats session is unchanged.".into(),
                                Err(error) => format!("Signed out, but could not remove the saved session: {error}"),
                            };
                            {
                                snapshot.title.clear();
                            }
                        }
                        Ok(Command::Play(id)) => {
                            if !snapshot.connected {
                                return Err("Connect to Deezer first".into());
                            }
                            let id = id.trim();
                            if id.is_empty() || !id.bytes().all(|b| b.is_ascii_digit()) {
                                return Err("Enter a numeric Deezer track ID".into());
                            }
                            let dz = engine.as_ref().unwrap();
                            let id = CString::new(id).map_err(|e| e.to_string())?;

                            {}

                            if unsafe { (dz.play)(id.as_ptr(), 0) } == 0 {
                                return Err(dz.error());
                            }
                            snapshot.title = format!("Deezer track {}", id.to_string_lossy());
                            snapshot.message = "Deezer streaming".into();
                        }
                        Ok(Command::PlayEntry(item, items)) => {
                            let dz = engine.as_ref().ok_or("Connect to Deezer first")?;
                            if !snapshot.connected {
                                return Err("Connect to Deezer first".into());
                            }
                            if !item.playable() || item.id.is_empty() {
                                return Err("Only tracks and episodes can be played".into());
                            }

                            let mut tracks: Vec<_> =
                                items.into_iter().filter(|e| e.playable()).collect();
                            let index = tracks
                                .iter()
                                .position(|e| e.id == item.id && e.kind == item.kind)
                                .unwrap_or_else(|| {
                                    tracks.push((*item).clone());
                                    tracks.len() - 1
                                });
                            mark_queue_fallback(&mut tracks);
                            set_queue(dz, &tracks, index)?;
                            snapshot.queue = Arc::new(tracks);
                            snapshot.queue_index = index;
                            failed_index = Some(index);
                            let value = dz.invoke(
                                if item.kind == "episode" {
                                    "DZPlayEpisode"
                                } else {
                                    "DZPlay"
                                },
                                &[serde_json::json!(item.id), serde_json::json!(item.duration)],
                            )?;
                            if value.as_i64() != Some(1) {
                                return Err(dz.error());
                            }
                            snapshot.now = *item;
                            snapshot.title = snapshot.now.title.clone();
                            failed_index = None;
                            queue_version = -1;
                            snapshot.message = "Deezer streaming".into();
                        }
                        Ok(Command::PlayQueueIndex(index)) => {
                            let dz = engine.as_ref().ok_or("Connect to Deezer first")?;
                            sync_queue(dz, &mut snapshot, &mut queue_version)?;
                            let queue = snapshot.queue.as_ref();
                            if index >= queue.len() {
                                return Err("Queue index out of bounds".into());
                            }
                            let item = queue[index].clone();
                            dz.invoke("DZQueueSetIndex", &[serde_json::json!(index)])?;
                            snapshot.queue_index = index;
                            failed_index = Some(index);
                            let value = dz.invoke(
                                if item.kind == "episode" {
                                    "DZPlayEpisode"
                                } else {
                                    "DZPlay"
                                },
                                &[serde_json::json!(item.id), serde_json::json!(item.duration)],
                            )?;
                            if value.as_i64() != Some(1) {
                                return Err(dz.error());
                            }
                            failed_index = None;
                            snapshot.now = item;
                            snapshot.title = snapshot.now.title.clone();
                            snapshot.queue_index = index;
                        }
                        Ok(Command::QueueMove { from, to, expected }) => {
                            let dz = engine.as_ref().ok_or("Connect to Deezer first")?;
                            sync_queue(dz, &mut snapshot, &mut queue_version)?;
                            let tracks = reordered_queue(
                                &snapshot.queue,
                                snapshot.queue_index,
                                from,
                                to,
                                &expected,
                            )?;
                            set_queue(dz, &tracks, snapshot.queue_index)?;
                            snapshot.queue = Arc::new(tracks);
                            queue_version = -1;
                            sync_queue(dz, &mut snapshot, &mut queue_version)?;
                            snapshot.message = "Queue reordered".into();
                        }
                        Ok(Command::QueueClear) => {
                            let dz = engine.as_ref().ok_or("Connect to Deezer first")?;
                            set_queue(dz, &[], 0)?;
                            queue_version = -1;
                            sync_queue(dz, &mut snapshot, &mut queue_version)?;
                            snapshot.message = "Queue cleared".into();
                        }
                        Ok(Command::QueueInsertNext(item)) | Ok(Command::QueueAddLast(item)) => {
                            let dz = engine.as_ref().ok_or("Connect to Deezer first")?;
                            if !snapshot.connected {
                                return Err("Connect to Deezer first".into());
                            }
                            if !item.playable() || item.id.is_empty() {
                                return Err("Only tracks and episodes can be queued".into());
                            }
                            sync_queue(dz, &mut snapshot, &mut queue_version)?;
                            let mut tracks = (*snapshot.queue).clone();
                            if tracks.is_empty()
                                && snapshot.now.playable()
                                && !snapshot.now.id.is_empty()
                            {
                                tracks.push(snapshot.now.clone());
                                snapshot.queue_index = 0;
                            }
                            let insert_next = queue_insert_next;
                            if insert_next && !snapshot.queue.is_empty() {
                                dz.invoke(
                                    "DZQueueInsertNext",
                                    &[serde_json::json!(item.wire().to_string())],
                                )?;
                                let at = (snapshot.queue_index + 1).min(tracks.len());
                                tracks.insert(at, explicit_queue_entry((*item).clone()));
                            } else {
                                let at = if insert_next {
                                    (snapshot.queue_index + 1).min(tracks.len())
                                } else {
                                    queue_add_position(&tracks, snapshot.queue_index)
                                };
                                tracks.insert(at, explicit_queue_entry((*item).clone()));
                                set_queue(dz, &tracks, snapshot.queue_index)?;
                            }
                            snapshot.queue = Arc::new(tracks);
                            queue_version = -1;
                            sync_queue(dz, &mut snapshot, &mut queue_version)?;
                            snapshot.message = format!(
                                "{}: {}",
                                if insert_next {
                                    "Playing next"
                                } else {
                                    "Added to queue"
                                },
                                item.title
                            );
                        }
                        Ok(Command::Next) | Ok(Command::Previous) => {
                            let previous = matches!(command, Ok(Command::Previous));
                            // Standard player rewind: if more than 3 seconds in, previous rewinds to track start.
                            if previous && snapshot.position > 3000 {
                                if let Some(dz) = &engine {
                                    unsafe {
                                        (dz.seek)(0);
                                    }
                                }
                                snapshot.position = 0;
                                return Ok(());
                            }
                            {}
                            let dz = engine.as_ref().ok_or("Connect to Deezer first")?;
                            sync_queue(dz, &mut snapshot, &mut queue_version)?;
                            if !previous
                                && autoplay.skip_radio(dz, &mut snapshot, &mut queue_version)?
                            {
                                return Ok(());
                            }
                            let queue = snapshot.queue.as_ref();
                            if queue.is_empty() {
                                if previous {
                                    unsafe {
                                        (dz.seek)(0);
                                    }
                                    snapshot.position = 0;
                                    return Ok(());
                                }
                                return Err("The playback queue is empty".into());
                            }
                            let current =
                                dz.invoke("DZQueueIndex", &[])?.as_i64().unwrap_or(0).max(0)
                                    as usize;
                            if !previous && current + 1 >= queue.len() && snapshot.repeat == 0 {
                                unsafe {
                                    (dz.stop)();
                                }
                                snapshot.state = "Stopped".into();
                                return Ok(());
                            }
                            if previous && current == 0 {
                                unsafe {
                                    (dz.seek)(0);
                                }
                                snapshot.position = 0;
                                return Ok(());
                            }
                            let index = if previous {
                                current.saturating_sub(1)
                            } else {
                                (current + 1) % queue.len()
                            };
                            let item = &queue[index];
                            dz.invoke("DZQueueSetIndex", &[serde_json::json!(index)])?;
                            failed_index = Some(index);
                            if dz
                                .invoke(
                                    if item.kind == "episode" {
                                        "DZPlayEpisode"
                                    } else {
                                        "DZPlay"
                                    },
                                    &[serde_json::json!(item.id), serde_json::json!(item.duration)],
                                )?
                                .as_i64()
                                != Some(1)
                            {
                                return Err(dz.error());
                            }
                            failed_index = None;
                            snapshot.queue_index = index;
                            snapshot.now = item.clone();
                            snapshot.title = snapshot.now.title.clone();
                        }
                        Ok(Command::Shuffle(value)) => {
                            if let Some(dz) = &engine {
                                dz.invoke("DZSetShuffle", &[serde_json::json!(value as i32)])?;
                            }
                        }
                        Ok(Command::Repeat(value)) => {
                            if let Some(dz) = &engine {
                                dz.invoke("DZSetRepeat", &[serde_json::json!(value)])?;
                            }
                        }
                        Ok(Command::Autoplay(value)) => {
                            snapshot.autoplay = value;
                        }
                        Ok(Command::Discord(value)) => {
                            snapshot.discord_settings = discord.configure(&value)?;
                            snapshot.message = "Discord settings saved".into();
                        }
                        Ok(Command::Api(name, args)) => {
                            let dz = engine.as_ref().ok_or("Connect to Deezer first")?;
                            if name == "DZSetAudioDevice" {
                                output.select(dz, &args)?;
                            } else if name == "DZSetEQJSON" {
                                snapshot.eq = dz.apply_eq(&args)?;
                            } else {
                                dz.invoke(&name, &args)?;
                            }
                            snapshot.message = "Audio setting applied".into();
                        }
                        Ok(Command::Toggle) => {
                            if let Some(dz) = &engine {
                                unsafe {
                                    (dz.toggle)();
                                }
                            }
                        }
                        Ok(Command::SetPlaying(playing)) => {
                            if let Some(dz) = &engine {
                                // SMTC Play/Pause are absolute requests; duplicate events
                                // must not toggle playback back to its previous state.
                                dz.invoke(if playing { "DZResume" } else { "DZPause" }, &[])?;
                            }
                        }
                        Ok(Command::Stop) => {
                            if let Some(dz) = &engine {
                                unsafe {
                                    (dz.stop)();
                                }
                            }
                        }
                        Ok(Command::Seek(percent)) => {
                            let position =
                                snapshot.duration.saturating_mul(percent.min(100) as u64) / 100;
                            if let Some(dz) = &engine {
                                unsafe {
                                    (dz.seek)(position as i64);
                                }
                            }
                            snapshot.position = position;
                        }
                        Ok(Command::SeekMs(position)) => {
                            let position = if snapshot.duration > 0 {
                                position.min(snapshot.duration)
                            } else {
                                position
                            };
                            if let Some(dz) = &engine {
                                unsafe {
                                    (dz.seek)(position as i64);
                                }
                            }
                            snapshot.position = position;
                        }
                        Ok(Command::Volume(value)) => {
                            volume = value.min(100);
                            if let Some(dz) = &engine {
                                unsafe {
                                    (dz.volume)(volume as f64 / 100.0);
                                }
                            }
                        }
                        _ => {}
                    }
                    Ok(())
                })();
                if let Err(error) = outcome {
                    snapshot.message = error;
                }
                if let Some(dz) = &engine {
                    unsafe {
                        snapshot.state = match (dz.state)() {
                            1 => "Loading",
                            2 => "Playing",
                            3 => "Paused",
                            4 => "Error",
                            _ => "Stopped",
                        }
                        .into();
                        snapshot.position = (dz.position)().max(0) as u64;
                        snapshot.duration = (dz.duration)().max(0) as u64;
                    }
                    if snapshot.connected {
                        let failed = failed_index.take().or_else(|| {
                            (snapshot.state == "Error"
                                && handled_error != Some(snapshot.queue_index))
                            .then_some(snapshot.queue_index)
                        });
                        if let Some(index) = failed {
                            snapshot.unavailable_at = Some(std::time::Instant::now());
                            handled_error = Some(index);
                            let mut next = index.saturating_add(1);
                            while let Some(item) = snapshot.queue.get(next).cloned() {
                                let result = dz
                                    .invoke("DZQueueSetIndex", &[serde_json::json!(next)])
                                    .and_then(|_| {
                                        dz.invoke(
                                            if item.kind == "episode" {
                                                "DZPlayEpisode"
                                            } else {
                                                "DZPlay"
                                            },
                                            &[
                                                serde_json::json!(item.id),
                                                serde_json::json!(item.duration),
                                            ],
                                        )
                                    });
                                snapshot.queue_index = next;
                                if result.ok().and_then(|v| v.as_i64()) == Some(1) {
                                    snapshot.now = item;
                                    snapshot.title = snapshot.now.title.clone();
                                    snapshot.state = "Loading".into();
                                    handled_error = None;
                                    break;
                                }
                                next += 1;
                            }
                            if next >= snapshot.queue.len() {
                                unsafe {
                                    (dz.stop)();
                                }
                                snapshot.state = "Stopped".into();
                            }
                        } else if snapshot.state != "Error" && snapshot.state != "Loading" {
                            handled_error = None;
                        }
                        if let Err(error) = output.update(dz) {
                            snapshot.message = error;
                        }
                        let _ = sync_queue(dz, &mut snapshot, &mut queue_version);
                        if let Ok(value) = dz.invoke("DZNowPlayingJSON", &[]) {
                            if let Some(item) = crate::catalog::entry(&value, "track") {
                                snapshot.title = item.title.clone();
                                snapshot.now = item;
                            }
                        }
                        if let Ok(idx) = dz.invoke("DZQueueIndex", &[]) {
                            if let Some(i) = idx.as_i64() {
                                if i >= 0 {
                                    snapshot.queue_index =
                                        (i as usize).min(snapshot.queue.len().saturating_sub(1));
                                }
                            }
                        }
                        if snapshot.now.id.is_empty() && snapshot.queue_index < snapshot.queue.len()
                        {
                            snapshot.now = snapshot.queue[snapshot.queue_index].clone();
                            snapshot.title = snapshot.now.title.clone();
                        }
                        snapshot.quality = dz
                            .invoke("DZFormat", &[])
                            .ok()
                            .and_then(|v| v.as_str().map(str::to_owned))
                            .unwrap_or_default();
                        snapshot.repeat = match dz
                            .invoke("DZGetRepeat", &[])
                            .ok()
                            .and_then(|v| v.as_str().map(str::to_owned))
                            .as_deref()
                        {
                            Some("all") => 1,
                            Some("one") => 2,
                            _ => 0,
                        };
                        snapshot.shuffle = dz
                            .invoke("DZGetShuffle", &[])
                            .ok()
                            .and_then(|v| v.as_i64())
                            .unwrap_or(0)
                            != 0;
                        autoplay.poll(dz, &mut snapshot, &mut queue_version, allow_autoplay);
                    }
                }
                discord.update(&snapshot);
                if let Some(status) = discord.status() {
                    snapshot.discord_status = status;
                }
                let _ = tx.try_send(snapshot.clone());
            }

            if let Some(dz) = &engine {
                unsafe {
                    (dz.stop)();
                }
            }
        });
        Self { commands, updates }
    }
}
#[cfg(test)]
mod reorder_tests {
    use super::*;
    #[test]
    fn moves_upcoming_tracks_without_changing_playing_or_history() {
        let tracks: Vec<_> = ["past", "now", "a", "b", "a"]
            .iter()
            .map(|id| crate::catalog::Entry {
                id: (*id).into(),
                ..Default::default()
            })
            .collect();
        let expected: Vec<_> = tracks.iter().map(|e| e.id.clone()).collect();
        let moved = reordered_queue(&tracks, 1, 4, 2, &expected).unwrap();
        assert_eq!(
            moved.iter().map(|e| e.id.as_str()).collect::<Vec<_>>(),
            vec!["past", "now", "a", "a", "b"]
        );
        assert!(reordered_queue(&tracks, 1, 1, 3, &expected).is_err());
        assert!(reordered_queue(&tracks, 1, 3, 0, &expected).is_err());
        assert!(reordered_queue(&tracks, 1, 2, 4, &[]).is_err());
    }
}

#[cfg(test)]
mod queue_tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn queue_refresh_keeps_style_mix_constraints_when_engine_drops_custom_fields() {
        let mut snapshot = Snapshot::default();
        snapshot.queue = Arc::new(vec![crate::catalog::Entry {
            id: "1".into(),
            kind: "track".into(),
            raw: json!({"style_mix":"1:1"}),
            ..Default::default()
        }]);
        adopt_queue(
            &mut snapshot,
            &json!({"version":1,"tracks":[{"id":"1","name":"Song"}]}),
        )
        .unwrap();
        assert_eq!(snapshot.queue[0].raw[crate::style_mix::MARKER], "1:1");
    }

    #[test]
    fn queue_refresh_preserves_authors_for_duplicate_occurrences_and_reorders() {
        let mut snapshot = Snapshot::default();
        let make = |id: &str, name: &str| crate::catalog::Entry {
            id: id.into(),
            title: id.into(),
            kind: "track".into(),
            raw: json!({"added_by": name}),
            ..Default::default()
        };
        // This is the order staged after an insert or move has succeeded.
        snapshot.queue = Arc::new(vec![
            make("1", "Alice"),
            make("2", "Bob"),
            make("1", "Charlie"),
        ]);
        adopt_queue(
            &mut snapshot,
            &json!({"version":1,"tracks":[
                {"id":"1","name":"A"}, {"id":"2","name":"B"}, {"id":"1","name":"A"}
            ]}),
        )
        .unwrap();
        assert_eq!(
            snapshot
                .queue
                .iter()
                .map(|e| e.raw["added_by"].as_str().unwrap())
                .collect::<Vec<_>>(),
            ["Alice", "Bob", "Charlie"]
        );
        let mut reordered = (*snapshot.queue).clone();
        reordered.swap(1, 2);
        snapshot.queue = Arc::new(reordered);
        adopt_queue(&mut snapshot, &json!({"version":2,"tracks":[
            {"id":"1","name":"A"}, {"id":"1","name":"A"}, {"id":"2","name":"B"}, {"id":"3","name":"C"}
        ]})).unwrap();
        assert_eq!(snapshot.queue[1].raw["added_by"], "Charlie");
        assert_eq!(snapshot.queue[2].raw["added_by"], "Bob");
        assert!(snapshot.queue[3].raw["added_by"].is_null());
    }

    #[test]
    fn engine_queue_keeps_duplicates_order_and_empty_updates() {
        let mut snapshot = Snapshot::default();
        let version = adopt_queue(
            &mut snapshot,
            &json!({"version":7,"index":2,"tracks":[
                {"id":"1","name":"A"}, {"id":"2","name":"B"}, {"id":"1","name":"A"}
            ]}),
        )
        .unwrap();
        assert_eq!(version, 7);
        assert_eq!(
            snapshot
                .queue
                .iter()
                .map(|item| item.id.as_str())
                .collect::<Vec<_>>(),
            ["1", "2", "1"]
        );
        assert_eq!(snapshot.queue_index, 2);
        adopt_queue(&mut snapshot, &json!({"version":8,"index":-1,"tracks":[]})).unwrap();
        assert!(snapshot.queue.is_empty());
        assert_eq!(snapshot.queue_index, 0);
    }

    #[test]
    fn malformed_queue_does_not_replace_a_valid_snapshot() {
        let mut snapshot = Snapshot::default();
        adopt_queue(
            &mut snapshot,
            &json!({"version":1,"index":99,"tracks":[{"id":"1","name":"A"}]}),
        )
        .unwrap();
        assert_eq!(snapshot.queue_index, 0);
        assert!(adopt_queue(&mut snapshot, &json!({"version":2,"tracks":[{}]})).is_err());
        assert!(adopt_queue(&mut snapshot, &json!({"tracks":[]})).is_err());
        assert_eq!(snapshot.queue.len(), 1);
        assert_eq!(snapshot.queue[0].id, "1");
    }
}

/// Validate the original engine ABI without logging in or accessing an account.
pub fn engine_smoke_test() -> Result<(), String> {
    let engine = Deezer::load()?;
    unsafe {
        if !(0..=4).contains(&(engine.state)()) {
            return Err("Invalid engine state".into());
        }
        (engine.position)();
        (engine.duration)();
        (engine.stop)();
    }
    Ok(())
}

/// Explicit opt-in account test. Credentials arrive over stdin, never argv/logs.
pub fn deezer_playback_test() -> Result<(), String> {
    use std::io::Read;
    let mut input = String::new();
    std::io::stdin()
        .read_to_string(&mut input)
        .map_err(|e| e.to_string())?;
    let mut lines = input.lines();
    let secret =
        CString::new(lines.next().unwrap_or_default().trim()).map_err(|_| "Invalid session")?;
    let track =
        CString::new(lines.next().unwrap_or("3135556").trim()).map_err(|_| "Invalid track")?;
    if secret.as_bytes().is_empty() {
        return Err("No session supplied".into());
    }
    let engine = Deezer::load()?;
    let result = (|| {
        unsafe {
            if (engine.init)(secret.as_ptr()) == 0 {
                return Err(format!("Sign-in failed (kind {})", (engine.login_error)()));
            }
            // Test the output pipeline silently.
            (engine.volume)(0.0);
            if (engine.play)(track.as_ptr(), 0) == 0 {
                return Err("Track resolution or playback failed".into());
            }
            let deadline = std::time::Instant::now() + Duration::from_secs(25);
            while (engine.state)() != 2 || (engine.position)() < 250 {
                if (engine.state)() == 4 {
                    return Err("Audio engine reported playback error".into());
                }
                if std::time::Instant::now() > deadline {
                    return Err("Timed out waiting for audio output".into());
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            (engine.toggle)();
            if (engine.state)() != 3 {
                return Err("Pause failed".into());
            }
            if (engine.duration)() > 2000 {
                (engine.seek)(1000);
            }
            (engine.toggle)();
            if (engine.state)() != 2 {
                return Err("Resume failed".into());
            }
            Ok(())
        }
    })();
    unsafe {
        (engine.stop)();
    }
    result
}
