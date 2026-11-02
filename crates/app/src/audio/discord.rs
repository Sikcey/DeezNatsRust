//! Discord IPC runs independently of playback. No Discord token is required.
//! Protocol: https://discord.com/developers/docs/topics/rpc
use super::Snapshot;
use serde_json::{json, Value};
use std::{
    io::{Read, Write},
    sync::{mpsc, Arc, Mutex},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

#[derive(Clone, Debug)]
struct Config {
    application_id: String,
    enabled: bool,
    show_paused: bool,
    badge: bool,
    status_name: u8,
    hide_details: bool,
    service_button: bool,
    app_button: bool,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            application_id: String::new(),
            enabled: true,
            show_paused: false,
            badge: true,
            status_name: 0,
            hide_details: false,
            service_button: true,
            app_button: true,
        }
    }
}
impl Config {
    fn value(&self) -> Value {
        json!({"application_id":self.application_id,"enabled":self.enabled,"show_paused":self.show_paused,"badge":self.badge,"status_name":self.status_name,"hide_details":self.hide_details,"service_button":self.service_button,"app_button":self.app_button})
    }
    fn update(&self, value: &Value) -> Result<Self, String> {
        let object = value
            .as_object()
            .ok_or("Discord settings must be an object")?;
        let mut next = self.clone();
        for (key, value) in object {
            if key == "application_id" {
                let id = value
                    .as_str()
                    .ok_or("Discord application ID must be text")?
                    .trim();
                if !id.is_empty()
                    && (!id.bytes().all(|b| b.is_ascii_digit())
                        || id.parse::<u64>().ok().filter(|id| *id > 0).is_none())
                {
                    return Err("Enter a numeric Discord application ID, not a token".into());
                }
                next.application_id = id.into();
                continue;
            }
            if key == "status_name" {
                next.status_name = value
                    .as_u64()
                    .filter(|v| *v <= 3)
                    .ok_or("Invalid status name")? as u8;
                continue;
            }
            let flag = value.as_bool().ok_or("Discord options must be booleans")?;
            match key.as_str() {
                "enabled" => next.enabled = flag,
                "show_paused" => next.show_paused = flag,
                "badge" => next.badge = flag,
                "hide_details" => next.hide_details = flag,
                "service_button" => next.service_button = flag,
                "app_button" => next.app_button = flag,
                _ => return Err(format!("Unknown Discord option: {key}")),
            }
        }
        Ok(next)
    }
    fn load() -> Self {
        let mut config = Self::default();
        if let Some(path) = config_path() {
            if let Ok(bytes) = std::fs::read(path) {
                if let Ok(value) = serde_json::from_slice(&bytes) {
                    if let Ok(saved) = config.update(&value) {
                        config = saved;
                    }
                }
            }
        }
        if let Ok(id) = std::env::var("DEEZNATS_DISCORD_APP_ID") {
            if let Ok(value) = config.update(&json!({"application_id":id})) {
                config = value;
            }
        }
        config.badge = false;
        config.status_name = 1;
        config
    }
    fn save(&self) -> Result<(), String> {
        let path = config_path().ok_or("LOCALAPPDATA is unavailable")?;
        std::fs::create_dir_all(path.parent().ok_or("Invalid Discord settings path")?)
            .map_err(|e| e.to_string())?;
        let temporary = path.with_extension("json.tmp");
        std::fs::write(
            &temporary,
            serde_json::to_vec(&self.value()).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        std::fs::rename(temporary, path).map_err(|e| e.to_string())
    }
}
fn config_path() -> Option<std::path::PathBuf> {
    Some(crate::storage::data_dir().ok()?.join("discord-rpc.json"))
}

#[derive(Clone, Default)]
struct Playback {
    state: String,
    provider: String,
    title: String,
    artist: String,
    album: String,
    artwork: String,
    id: String,
    kind: String,
    position: u64,
    duration: u64,
}
impl Playback {
    fn from_snapshot(snapshot: &Snapshot) -> Self {
        Self {
            state: snapshot.state.clone(),
            provider: snapshot.provider.label().to_string(),
            title: { snapshot.now.title.clone() },
            artist: { snapshot.now.subtitle.clone() },
            album: { snapshot.now.album.clone() },
            artwork: { snapshot.now.artwork.clone() },
            id: { snapshot.now.id.clone() },
            kind: { snapshot.now.kind.clone() },
            position: snapshot.position,
            duration: snapshot.duration,
        }
    }
}
fn clipped(text: &str) -> String {
    let mut result = String::new();
    for ch in text.chars().filter(|ch| !ch.is_control()) {
        if result.len() + ch.len_utf8() > 128 {
            break;
        }
        result.push(ch);
    }
    result
}
fn activity(config: &Config, playback: &Playback, now: u64) -> Value {
    let playing = playback.state == "Playing";
    if !config.enabled || playback.title.is_empty() || !playing {
        return Value::Null;
    }
    let mut result = json!({"type":2,"details":clipped(if config.hide_details {"Listening to music"}else{&playback.title}),"state":clipped(if !playing {"Paused"}else if config.hide_details {"DeezNats"}else{&playback.artist})});
    result["name"] = json!(clipped(match config.status_name {
        0 => &playback.provider,
        2 => &playback.title,
        3 => &playback.artist,
        _ => "DeezNats",
    }));
    result["status_display_type"] = json!(match config.status_name {
        2 => 2,
        3 => 1,
        _ => 0,
    });
    if playing && playback.duration > 0 {
        let start = now.saturating_sub(playback.position.min(playback.duration) / 1000);
        result["timestamps"] =
            json!({"start":start,"end":start.saturating_add(playback.duration/1000)});
    }
    if !config.hide_details && playback.artwork.starts_with("https://") {
        result["assets"] =
            json!({"large_image":playback.artwork,"large_text":clipped(&playback.album)});
    }
    if config.badge {
        result["assets"]["small_image"] = json!(playback.provider.to_ascii_lowercase());
        result["assets"]["small_text"] = json!(&playback.provider);
    }
    let mut buttons = Vec::new();
    if config.service_button
        && !config.hide_details
        && !playback.id.is_empty()
        && playback.id.bytes().all(|b| b.is_ascii_digit())
    {
        buttons.push(json!({"label":"Listen on Deezer","url":format!("https://www.deezer.com/{}/{}",if playback.kind=="episode" {"episode"} else {"track"},playback.id)}));
    }
    if config.app_button {
        buttons.push(json!({"label":"DeezNats","url":"https://github.com/Sikcey/DeezNats"}));
    }
    if !buttons.is_empty() {
        result["buttons"] = json!(buttons);
    }
    result
}

#[derive(Default)]
struct Frames {
    bytes: Vec<u8>,
}
impl Frames {
    fn next(&mut self) -> Result<Option<(u32, Vec<u8>)>, String> {
        if self.bytes.len() < 8 {
            return Ok(None);
        }
        let op = u32::from_le_bytes(self.bytes[..4].try_into().unwrap());
        let len = u32::from_le_bytes(self.bytes[4..8].try_into().unwrap()) as usize;
        if len > 1024 * 1024 {
            return Err("Discord IPC frame exceeds limit".into());
        }
        if self.bytes.len() < len + 8 {
            return Ok(None);
        }
        let data = self.bytes[8..len + 8].to_vec();
        self.bytes.drain(..len + 8);
        Ok(Some((op, data)))
    }
}
fn frame(op: u32, data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len() + 8);
    out.extend_from_slice(&op.to_le_bytes());
    out.extend_from_slice(&(data.len() as u32).to_le_bytes());
    out.extend_from_slice(data);
    out
}

struct Connection {
    file: std::fs::File,
    frames: Frames,
    ready: bool,
    opened: Instant,
    nonce: u64,
    sent: Option<Value>,
    last_send: Instant,
}
impl Connection {
    fn connect(id: &str) -> Result<Self, String> {
        for i in 0..10 {
            if let Ok(connection) = Self::open(&format!(r"\\?\pipe\discord-ipc-{i}"), id) {
                return Ok(connection);
            }
        }
        Err("Discord desktop is not available".into())
    }
    fn open(path: &str, id: &str) -> Result<Self, String> {
        use std::os::windows::io::AsRawHandle;
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn SetNamedPipeHandleState(
                pipe: *mut std::ffi::c_void,
                mode: *const u32,
                max: *const u32,
                timeout: *const u32,
            ) -> i32;
        }
        let mut file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(path)
            .map_err(|e| e.to_string())?;
        // PIPE_NOWAIT bounds every read/write, including shutdown with a wedged client.
        let mode = 1u32;
        if unsafe {
            SetNamedPipeHandleState(
                file.as_raw_handle(),
                &mode,
                std::ptr::null(),
                std::ptr::null(),
            )
        } == 0
        {
            return Err(std::io::Error::last_os_error().to_string());
        }
        file.write_all(&frame(
            0,
            &serde_json::to_vec(&json!({"v":1,"client_id":id})).unwrap(),
        ))
        .map_err(|e| e.to_string())?;
        Ok(Self {
            file,
            frames: Frames::default(),
            ready: false,
            opened: Instant::now(),
            nonce: 0,
            sent: None,
            last_send: Instant::now() - Duration::from_secs(2),
        })
    }
    fn poll(&mut self) -> Result<(), String> {
        let mut bytes = [0u8; 8192];
        for _ in 0..16 {
            match self.file.read(&mut bytes) {
                // Byte-mode PIPE_NOWAIT may report a successful zero-byte read
                // while the connected pipe has no data. A closed pipe returns
                // ERROR_BROKEN_PIPE, handled below as an actual disconnect.
                Ok(0) => break,
                Ok(count) => self.frames.bytes.extend_from_slice(&bytes[..count]),
                Err(error)
                    if matches!(error.raw_os_error(), Some(232 | 536))
                        || error.kind() == std::io::ErrorKind::WouldBlock =>
                {
                    break
                }
                Err(error) => return Err(error.to_string()),
            }
            while let Some((op, data)) = self.frames.next()? {
                match op {
                    1 => {
                        let value: Value =
                            serde_json::from_slice(&data).map_err(|e| e.to_string())?;
                        if value["evt"] == "READY" {
                            self.ready = true;
                        }
                        if value["evt"] == "ERROR" {
                            return Err(format!(
                                "Discord: {}",
                                value["data"]["message"].as_str().unwrap_or("RPC rejected")
                            ));
                        }
                    }
                    2 => return Err("Discord closed the connection".into()),
                    3 => self
                        .file
                        .write_all(&frame(4, &data))
                        .map_err(|e| e.to_string())?,
                    4 => {}
                    _ => return Err("Invalid Discord IPC opcode".into()),
                }
            }
        }
        if !self.ready && self.opened.elapsed() > Duration::from_secs(3) {
            return Err("Discord handshake timed out".into());
        }
        Ok(())
    }
    fn publish(&mut self, value: Value, force: bool) -> Result<(), String> {
        if !self.ready {
            return Ok(());
        }
        let changed = self
            .sent
            .as_ref()
            .is_none_or(|sent| !equivalent(sent, &value));
        if !force && (!changed || self.last_send.elapsed() < Duration::from_secs(1)) {
            return Ok(());
        }
        self.nonce += 1;
        let command = json!({"cmd":"SET_ACTIVITY","args":{"pid":std::process::id(),"activity":value},"nonce":self.nonce.to_string()});
        self.file
            .write_all(&frame(1, &serde_json::to_vec(&command).unwrap()))
            .map_err(|e| e.to_string())?;
        self.sent = Some(value);
        self.last_send = Instant::now();
        Ok(())
    }
}
fn equivalent(a: &Value, b: &Value) -> bool {
    let mut a = a.clone();
    let mut b = b.clone();
    let timestamps = a["timestamps"]["start"]
        .as_u64()
        .zip(b["timestamps"]["start"].as_u64());
    if a.get("timestamps").is_some() != b.get("timestamps").is_some() {
        return false;
    }
    if let Some((left, right)) = timestamps {
        if left.abs_diff(right) > 2 {
            return false;
        }
        let left_duration = a["timestamps"]["end"]
            .as_u64()
            .unwrap_or(left)
            .saturating_sub(left);
        let right_duration = b["timestamps"]["end"]
            .as_u64()
            .unwrap_or(right)
            .saturating_sub(right);
        if left_duration.abs_diff(right_duration) > 1 {
            return false;
        }
    }
    if let Some(object) = a.as_object_mut() {
        object.remove("timestamps");
    }
    if let Some(object) = b.as_object_mut() {
        object.remove("timestamps");
    }
    a == b
}

struct Latest {
    config: Config,
    playback: Playback,
    closed: bool,
}
pub(super) struct Worker {
    latest: Arc<Mutex<Latest>>,
    wake: mpsc::SyncSender<()>,
    updates: mpsc::Receiver<String>,
    join: Option<std::thread::JoinHandle<()>>,
}
impl Worker {
    pub(super) fn start() -> Self {
        let latest = Arc::new(Mutex::new(Latest {
            config: Config::load(),
            playback: Playback::default(),
            closed: false,
        }));
        let state = latest.clone();
        let (wake, rx) = mpsc::sync_channel(1);
        let (tx, updates) = mpsc::sync_channel(1);
        let join = std::thread::spawn(move || {
            let mut connection: Option<Connection> = None;
            let mut id = String::new();
            let mut retry = Instant::now();
            let mut status = String::new();
            loop {
                let _ = rx.recv_timeout(Duration::from_millis(100));
                let (config, playback, closed) = {
                    let state = state.lock().unwrap();
                    (state.config.clone(), state.playback.clone(), state.closed)
                };
                if closed || !config.enabled || config.application_id != id {
                    if let Some(client) = connection.as_mut() {
                        let _ = client.publish(Value::Null, true);
                    }
                    connection = None;
                    id = config.application_id.clone();
                    retry = Instant::now();
                    if closed {
                        break;
                    }
                }
                let mut message = if !config.enabled {
                    "Disabled".into()
                } else if id.is_empty() {
                    "Discord application ID required".into()
                } else {
                    "Waiting for Discord".into()
                };
                if config.enabled && !id.is_empty() {
                    if connection.is_none() && Instant::now() >= retry {
                        match Connection::connect(&id) {
                            Ok(client) => connection = Some(client),
                            Err(error) => message = error,
                        }
                        retry = Instant::now() + Duration::from_secs(10);
                    }
                    if let Some(client) = connection.as_mut() {
                        let now = SystemTime::now()
                            .duration_since(UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_secs();
                        let result = client
                            .poll()
                            .and_then(|_| client.publish(activity(&config, &playback, now), false));
                        match result {
                            Ok(()) => {
                                message = if client.ready {
                                    "Connected"
                                } else {
                                    "Connecting"
                                }
                                .into()
                            }
                            Err(error) => {
                                message = error;
                                connection = None;
                                retry = Instant::now() + Duration::from_secs(10);
                            }
                        }
                    }
                }
                if message != status && tx.try_send(message.clone()).is_ok() {
                    status = message;
                }
            }
        });
        Self {
            latest,
            wake,
            updates,
            join: Some(join),
        }
    }
    pub(super) fn configure(&self, value: &Value) -> Result<Value, String> {
        let next = self.latest.lock().unwrap().config.update(value)?;
        next.save()?;
        let result = next.value();
        self.latest.lock().unwrap().config = next;
        let _ = self.wake.try_send(());
        Ok(result)
    }
    pub(super) fn update(&self, snapshot: &Snapshot) {
        self.latest.lock().unwrap().playback = Playback::from_snapshot(snapshot);
        let _ = self.wake.try_send(());
    }
    pub(super) fn settings(&self) -> Value {
        self.latest.lock().unwrap().config.value()
    }
    pub(super) fn status(&self) -> Option<String> {
        let mut last = None;
        while let Ok(value) = self.updates.try_recv() {
            last = Some(value);
        }
        last
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        self.latest.lock().unwrap().closed = true;
        let _ = self.wake.try_send(());
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "Opt-in Discord desktop handshake; does not publish activity"]
    fn live_discord_handshake() {
        let config = Config::load();
        assert!(
            !config.application_id.is_empty(),
            "Configure a Discord application ID first"
        );
        let mut connection = Connection::connect(&config.application_id).unwrap();
        let deadline = Instant::now() + Duration::from_secs(4);
        while !connection.ready && Instant::now() < deadline {
            connection.poll().unwrap();
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(
            connection.ready,
            "Discord did not acknowledge the application ID"
        );
        println!("Discord READY acknowledged; no activity published");
    }
    #[test]
    fn windows_pipe_roundtrip_handshake_ping_activity_and_clear() {
        use std::os::windows::io::FromRawHandle;
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn CreateNamedPipeW(
                name: *const u16,
                open: u32,
                mode: u32,
                instances: u32,
                out_size: u32,
                in_size: u32,
                timeout: u32,
                security: *const std::ffi::c_void,
            ) -> *mut std::ffi::c_void;
            fn ConnectNamedPipe(
                pipe: *mut std::ffi::c_void,
                overlapped: *mut std::ffi::c_void,
            ) -> i32;
        }
        let path = format!(r"\\?\pipe\deeznats-rpc-test-{}", std::process::id());
        let wide: Vec<_> = path.encode_utf16().chain(Some(0)).collect();
        let handle =
            unsafe { CreateNamedPipeW(wide.as_ptr(), 3, 1, 1, 8192, 8192, 0, std::ptr::null()) };
        assert_ne!(handle as isize, -1, "{}", std::io::Error::last_os_error());
        let mut server = unsafe { std::fs::File::from_raw_handle(handle) };
        unsafe {
            ConnectNamedPipe(handle, std::ptr::null_mut());
        }
        let mut client = Connection::open(&path, "123456789012345678").unwrap();
        let mut buffer = [0u8; 8192];
        let count = server.read(&mut buffer).unwrap();
        let mut frames = Frames {
            bytes: buffer[..count].to_vec(),
        };
        let (op, body) = frames.next().unwrap().unwrap();
        assert_eq!(op, 0);
        assert_eq!(serde_json::from_slice::<Value>(&body).unwrap()["v"], 1);
        server.write_all(&frame(1, br#"{"evt":"READY"}"#)).unwrap();
        server.write_all(&frame(3, b"ping")).unwrap();
        client.poll().unwrap();
        assert!(client.ready);
        let count = server.read(&mut buffer).unwrap();
        let mut frames = Frames {
            bytes: buffer[..count].to_vec(),
        };
        assert_eq!(frames.next().unwrap().unwrap(), (4, b"ping".to_vec()));
        client
            .publish(activity(&Config::default(), &playing(), 1000), true)
            .unwrap();
        let count = server.read(&mut buffer).unwrap();
        let mut frames = Frames {
            bytes: buffer[..count].to_vec(),
        };
        let (_, body) = frames.next().unwrap().unwrap();
        let command: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(command["cmd"], "SET_ACTIVITY");
        assert_eq!(command["args"]["activity"]["details"], "Song");
        client.publish(Value::Null, true).unwrap();
        let count = server.read(&mut buffer).unwrap();
        let mut frames = Frames {
            bytes: buffer[..count].to_vec(),
        };
        let (_, body) = frames.next().unwrap().unwrap();
        assert!(serde_json::from_slice::<Value>(&body).unwrap()["args"]["activity"].is_null());
        server.write_all(&frame(2, b"{}")).unwrap();
        assert!(client.poll().is_err());
    }
    fn playing() -> Playback {
        Playback {
            state: "Playing".into(),
            provider: "Deezer".into(),
            title: "Song".into(),
            artist: "Artist".into(),
            album: "Album".into(),
            id: "42".into(),
            kind: "track".into(),
            artwork: "https://example.test/cover.jpg".into(),
            position: 30000,
            duration: 180000,
        }
    }
    #[test]
    fn presence_tracks_playback_pause_privacy_and_stop() {
        let config = Config::default();
        let mut playback = playing();
        let value = activity(&config, &playback, 1000);
        assert_eq!(value["type"], 2);
        assert_eq!(value["timestamps"], json!({"start":970,"end":1150}));
        assert_eq!(
            value["buttons"][0]["url"],
            "https://www.deezer.com/track/42"
        );
        playback.state = "Paused".into();
        assert!(activity(&config, &playback, 1000).is_null());
        let mut config = config;
        config.show_paused = true;
        assert!(activity(&config, &playback, 1000).is_null());
        playback.state = "Playing".into();
        config.hide_details = true;
        let value = activity(&config, &playback, 1000);
        assert_ne!(value["details"], "Song");
        assert!(value["assets"].get("large_image").is_none());
        assert_eq!(value["buttons"].as_array().unwrap().len(), 1);
        playback.state = "Stopped".into();
        assert!(activity(&config, &playback, 1000).is_null());
        config.enabled = false;
        playback.state = "Playing".into();
        assert!(activity(&config, &playback, 1000).is_null());
    }
    #[test]
    fn clock_jitter_is_deduplicated_but_seeks_and_duration_changes_are_not() {
        let config = Config::default();
        let mut playback = playing();
        let a = activity(&config, &playback, 1000);
        assert!(equivalent(&a, &activity(&config, &playback, 1001)));
        playback.position = 70000;
        assert!(!equivalent(&a, &activity(&config, &playback, 1000)));
        playback = playing();
        playback.duration = 200000;
        assert!(!equivalent(&a, &activity(&config, &playback, 1000)));
    }
    #[test]
    fn ipc_handles_partial_frames_and_rejects_unbounded_payloads() {
        let data = frame(1, br#"{"evt":"READY"}"#);
        let mut frames = Frames::default();
        frames.bytes.extend_from_slice(&data[..6]);
        assert!(frames.next().unwrap().is_none());
        frames.bytes.extend_from_slice(&data[6..]);
        assert_eq!(
            frames.next().unwrap().unwrap(),
            (1, br#"{"evt":"READY"}"#.to_vec())
        );
        frames
            .bytes
            .extend_from_slice(&[1, 0, 0, 0, 255, 255, 255, 127]);
        assert!(frames.next().is_err());
    }
    #[test]
    fn config_accepts_public_ids_and_rejects_secrets_or_invalid_types() {
        let config = Config::default();
        assert!(config
            .update(&json!({"application_id":"123456789012345678"}))
            .is_ok());
        for value in [
            json!({"application_id":"token.secret"}),
            json!({"enabled":1}),
            json!({"unknown":true}),
        ] {
            assert!(config.update(&value).is_err());
        }
        let clipped = clipped(&"ş".repeat(100));
        assert!(clipped.len() <= 128);
        assert!(clipped.chars().all(|c| c == 'ş'));
    }
}
