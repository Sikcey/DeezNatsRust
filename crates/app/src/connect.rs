//! DeezNats Connect: Embedded HTTP server supporting the full application interface in Blood Moon theme.
use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc::SyncSender,
        Arc, RwLock,
    },
    thread,
    time::Duration,
};

#[path = "connect/spectrum.rs"]
mod spectrum;
#[path = "connect/spotify_audio.rs"]
mod spotify_audio;
pub(crate) use spectrum::{dynamic_level, scene_frequency};

use crate::audio::{Command, Snapshot};
use crate::catalog::{Entry, Provider};
use crate::engine::Engine as Deezer;
use std::sync::{Mutex, OnceLock};

static VISUALIZER_SPECTRUM: OnceLock<RwLock<spectrum::Snapshot>> = OnceLock::new();
pub(crate) fn visualizer_spectrum() -> Option<[f32; 32]> {
    let value = VISUALIZER_SPECTRUM.get()?.read().ok()?;
    value.fresh().then_some(value.levels)
}
pub(crate) fn visualizer_trap_spectrum() -> Option<[f32; 32]> {
    let value = VISUALIZER_SPECTRUM.get()?.read().ok()?;
    value.fresh().then_some(value.trap_levels)
}
pub struct AudioBroadcaster {
    clients: Mutex<Vec<AudioClient>>,
}
struct AudioClient {
    sender: std::sync::mpsc::SyncSender<Arc<Vec<u8>>>,
    listener: std::sync::Weak<()>,
}
pub struct AudioSubscription {
    receiver: std::sync::mpsc::Receiver<Arc<Vec<u8>>>,
    _listener: Arc<()>,
}
impl std::ops::Deref for AudioSubscription {
    type Target = std::sync::mpsc::Receiver<Arc<Vec<u8>>>;
    fn deref(&self) -> &Self::Target {
        &self.receiver
    }
}

static BROADCASTER: OnceLock<AudioBroadcaster> = OnceLock::new();

pub fn broadcaster() -> &'static AudioBroadcaster {
    BROADCASTER.get_or_init(|| AudioBroadcaster {
        clients: Mutex::new(Vec::new()),
    })
}

impl AudioBroadcaster {
    pub fn subscribe(&self) -> AudioSubscription {
        let (tx, rx) = std::sync::mpsc::sync_channel(128);
        let listener = Arc::new(());
        if let Ok(mut clients) = self.clients.lock() {
            clients.push(AudioClient {
                sender: tx,
                listener: Arc::downgrade(&listener),
            });
        }
        AudioSubscription {
            receiver: rx,
            _listener: listener,
        }
    }
    fn has_listeners(&self) -> bool {
        self.clients
            .lock()
            .map(|mut clients| {
                clients.retain(|c| c.listener.strong_count() > 0);
                !clients.is_empty()
            })
            .unwrap_or(false)
    }
    fn publish(&self, pcm: Arc<Vec<u8>>) {
        if let Ok(mut clients) = self.clients.lock() {
            // Disconnect slow clients rather than dropping timeline samples.
            clients.retain(|c| {
                c.listener.strong_count() > 0 && c.sender.try_send(pcm.clone()).is_ok()
            });
        }
    }
}

fn publish_pcm(
    state: &RwLock<HubState>,
    active: &AtomicBool,
    spectrum_tx: &std::sync::mpsc::SyncSender<(Provider, u64, Arc<Vec<u8>>)>,
    generation: u64,
    pcm: &[u8],
    provider: Provider,
) {
    if let Ok(st) = state.read() {
        if !active.load(Ordering::Relaxed) || st.snapshot.provider != provider {
            return;
        }
        let pcm = Arc::new(pcm.to_vec());
        broadcaster().publish(pcm.clone());
        // Visualisation is best effort and must never block the audio stream.
        let _ = spectrum_tx.try_send((provider, generation, pcm));
    }
}

fn wav_header(sample_rate: u32, channels: u16) -> [u8; 44] {
    let mut header = [0u8; 44];
    header[0..4].copy_from_slice(b"RIFF");
    header[4..8].copy_from_slice(&0x7FFF_FFFFu32.to_le_bytes());
    header[8..12].copy_from_slice(b"WAVE");
    header[12..16].copy_from_slice(b"fmt ");
    header[16..20].copy_from_slice(&16u32.to_le_bytes());
    header[20..22].copy_from_slice(&1u16.to_le_bytes());
    header[22..24].copy_from_slice(&channels.to_le_bytes());
    header[24..28].copy_from_slice(&sample_rate.to_le_bytes());
    let byte_rate = sample_rate * (channels as u32) * 2;
    header[28..32].copy_from_slice(&byte_rate.to_le_bytes());
    let block_align = channels * 2;
    header[32..34].copy_from_slice(&block_align.to_le_bytes());
    header[34..36].copy_from_slice(&16u16.to_le_bytes());
    header[36..40].copy_from_slice(b"data");
    header[40..44].copy_from_slice(&0x7FFF_FFFFu32.to_le_bytes());
    header
}

pub const DEFAULT_PORT: u16 = 3131;

fn attribute_entry(mut entry: Entry, payload: &serde_json::Value) -> Entry {
    if let Some(key) = payload[crate::style_mix::MARKER]
        .as_str()
        .filter(|key| crate::style_mix::Selection::parse(key).is_ok())
    {
        entry.raw[crate::style_mix::MARKER] = serde_json::json!(key);
    }
    let nickname: String = payload["added_by"]
        .as_str()
        .unwrap_or("")
        .trim()
        .chars()
        .filter(|ch| !ch.is_control())
        .take(64)
        .collect();
    entry.raw["added_by"] = serde_json::json!(nickname);
    entry
}

#[derive(Clone, Default)]
pub struct ConnectTrack {
    pub kind: String,
    pub id: String,
    pub title: String,
    pub artist: String,
    #[allow(dead_code)]
    pub album: String,
    pub artwork: String,
    pub duration: u64,
}

impl ConnectTrack {
    #[allow(dead_code)]
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "id": self.id, "kind": self.kind,
            "title": self.title,
            "artist": self.artist,
            "album": self.album,
            "artwork": self.artwork,
            "duration": self.duration / 1000, "duration_ms": self.duration,
        })
    }

    pub fn from_value(val: &serde_json::Value) -> Option<Self> {
        let id = val.get("id").and_then(|v| {
            v.as_str()
                .map(ToString::to_string)
                .or_else(|| v.as_i64().map(|n| n.to_string()))
        })?;
        let title = val
            .get("title")
            .and_then(serde_json::Value::as_str)?
            .to_string();
        let artist = val
            .get("artist")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("")
            .to_string();
        let album = val
            .get("album")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("")
            .to_string();
        let artwork = val
            .get("artwork")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("")
            .to_string();
        let duration = val
            .get("duration_ms")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or_else(|| {
                val.get("duration")
                    .and_then(serde_json::Value::as_u64)
                    .unwrap_or(0)
                    .saturating_mul(1000)
            });

        let kind = match val.get("kind").and_then(serde_json::Value::as_str) {
            Some("episode") => "episode",
            _ => "track",
        }
        .to_string();
        Some(Self {
            kind,
            id,
            title,
            artist,
            album,
            artwork,
            duration,
        })
    }

    pub fn to_entry(&self) -> Entry {
        Entry {
            id: self.id.clone(),
            title: self.title.clone(),
            subtitle: self.artist.clone(),
            artwork: self.artwork.clone(),
            kind: self.kind.clone(),
            duration: self.duration,
            album: self.album.clone(),
            ..Default::default()
        }
    }
}

pub struct HubState {
    spotify_audio_error: Option<String>,
    pub spectrum: spectrum::Snapshot,
    pub snapshot: Snapshot,
    pub engine: Option<Arc<Deezer>>,
}

pub struct ConnectHub {
    audio_thread: Option<thread::JoinHandle<()>>,
    server_thread: Option<thread::JoinHandle<()>>,
    state: Arc<RwLock<HubState>>,
    #[allow(dead_code)]
    command_tx: SyncSender<Command>,
    active: Arc<AtomicBool>,
    #[allow(dead_code)]
    port: u16,
}

impl ConnectHub {
    pub fn start(port: u16, command_tx: SyncSender<Command>) -> Option<Self> {
        let bind_host = if cfg!(test) { "127.0.0.1" } else { "0.0.0.0" };
        let listener = TcpListener::bind((bind_host, port))
            .or_else(|error| {
                if port == DEFAULT_PORT {
                    TcpListener::bind((bind_host, DEFAULT_PORT + 1))
                } else {
                    Err(error)
                }
            })
            .ok()?;
        let actual_port = listener.local_addr().map(|a| a.port()).unwrap_or(port);
        let _ = listener.set_nonblocking(true);
        let active = Arc::new(AtomicBool::new(true));
        let state = Arc::new(RwLock::new(HubState {
            spotify_audio_error: None,
            spectrum: spectrum::Snapshot::default(),
            snapshot: Snapshot::default(),
            engine: None,
        }));

        let audio_active = active.clone();
        let audio_state = state.clone();
        let audio_thread = thread::spawn(move || {
            let mut buffer = vec![0u8; 262144];
            let (spectrum_tx, spectrum_rx) =
                std::sync::mpsc::sync_channel::<(Provider, u64, Arc<Vec<u8>>)>(16);
            let spectrum_state = audio_state.clone();
            let spectrum_active = audio_active.clone();
            let spectrum_epoch = Arc::new(AtomicU64::new(0));
            let worker_epoch = spectrum_epoch.clone();
            let spectrum_thread = thread::spawn(move || {
                let mut analyzer = spectrum::Analyzer::new();
                let mut generation = None;
                while let Ok((provider, current, pcm)) = spectrum_rx.recv() {
                    if !spectrum_active.load(Ordering::Relaxed) {
                        break;
                    }
                    if current != worker_epoch.load(Ordering::Acquire) {
                        continue;
                    }
                    if generation != Some(current) {
                        analyzer = spectrum::Analyzer::new();
                        generation = Some(current);
                    }
                    if let Some(levels) = analyzer.push(&pcm) {
                        if let Ok(mut st) = spectrum_state.write() {
                            if st.snapshot.provider != provider
                                || current != worker_epoch.load(Ordering::Acquire)
                            {
                                continue;
                            }
                            let snapshot = spectrum::Snapshot {
                                levels,
                                trap_levels: analyzer.scene_levels(),
                                updated: Some(std::time::Instant::now()),
                            };
                            if let Ok(mut latest) = VISUALIZER_SPECTRUM
                                .get_or_init(|| RwLock::new(spectrum::Snapshot::default()))
                                .write()
                            {
                                *latest = snapshot.clone();
                            }
                            st.spectrum = snapshot;
                        }
                    }
                }
            });
            let mut spectrum_generation = 0u64;
            let mut enabled_engine: Option<Arc<Deezer>> = None;
            let mut spotify_capture: Option<spotify_audio::Capture> = None;
            let mut previous_target = None;
            let mut previous_provider = Provider::Deezer;
            while audio_active.load(Ordering::Relaxed) {
                let (provider, engine) = audio_state
                    .read()
                    .map(|s| (s.snapshot.provider, s.engine.clone()))
                    .unwrap_or((Provider::Deezer, None));
                let target = if provider == Provider::Spotify {
                    crate::spotify::service::audio_target()
                } else {
                    None
                };
                if provider != previous_provider
                    || !spotify_audio::same_target(previous_target.as_ref(), target.as_ref())
                {
                    spotify_capture = None;
                    spectrum_generation = spectrum_generation.wrapping_add(1);
                    spectrum_epoch.store(spectrum_generation, Ordering::Release);
                    if let Ok(mut st) = audio_state.write() {
                        st.spectrum = spectrum::Snapshot::default();
                        st.spotify_audio_error = None;
                        if let Ok(mut latest) = VISUALIZER_SPECTRUM
                            .get_or_init(|| RwLock::new(spectrum::Snapshot::default()))
                            .write()
                        {
                            *latest = spectrum::Snapshot::default();
                        }
                        if previous_target.is_some() {
                            if let Ok(mut clients) = broadcaster().clients.lock() {
                                clients.clear();
                            }
                        }
                    }
                    previous_provider = provider;
                    previous_target = target.clone();
                }
                if engine.is_none() {
                    if let Some(previous) = enabled_engine.take() {
                        unsafe {
                            if let Ok(disable) = previous
                                .library
                                .get::<unsafe extern "C" fn(i32)>(b"DZEnableBroadcast\0")
                            {
                                disable(0);
                            }
                        }
                    }
                }
                if let Some(engine) = engine {
                    unsafe {
                        if let (Ok(enable), Ok(read)) = (
                            engine
                                .library
                                .get::<unsafe extern "C" fn(i32)>(b"DZEnableBroadcast\0"),
                            engine
                                .library
                                .get::<unsafe extern "C" fn(*mut u8, i32) -> i32>(
                                    b"DZReadBroadcast\0",
                                ),
                        ) {
                            if !enabled_engine
                                .as_ref()
                                .is_some_and(|previous| Arc::ptr_eq(previous, &engine))
                            {
                                if let Some(previous) = enabled_engine.take() {
                                    if let Ok(disable) = previous
                                        .library
                                        .get::<unsafe extern "C" fn(i32)>(b"DZEnableBroadcast\0")
                                    {
                                        disable(0);
                                    }
                                }
                                enable(1);
                                enabled_engine = Some(engine.clone());
                            }
                            loop {
                                if !audio_active.load(Ordering::Relaxed)
                                    || !audio_state
                                        .read()
                                        .is_ok_and(|st| st.snapshot.provider == Provider::Deezer)
                                {
                                    break;
                                }
                                let n = read(buffer.as_mut_ptr(), buffer.len() as i32);
                                if n <= 0 {
                                    break;
                                }
                                publish_pcm(
                                    &audio_state,
                                    &audio_active,
                                    &spectrum_tx,
                                    spectrum_generation,
                                    &buffer[..n as usize],
                                    Provider::Deezer,
                                );
                            }
                        }
                    }
                }
                if provider == Provider::Spotify && broadcaster().has_listeners() {
                    let result = (|| {
                        let target = target.ok_or_else(|| "Spotify audio unavailable: start the host's supervised Soloist player first".to_string())?;
                        if spotify_capture.is_none() {
                            spotify_capture = Some(spotify_audio::Capture::start(target)?);
                            if let Ok(mut st) = audio_state.write() {
                                st.spotify_audio_error = None;
                            }
                        }
                        buffer.clear();
                        spotify_capture.as_mut().unwrap().read(&mut buffer)?;
                        if !buffer.is_empty() {
                            // Revalidate generation after asynchronous activation/capture.
                            let current = crate::spotify::service::audio_target();
                            if spotify_audio::same_target(
                                current.as_ref(),
                                spotify_capture.as_ref().map(|c| &c.target),
                            ) {
                                publish_pcm(
                                    &audio_state,
                                    &audio_active,
                                    &spectrum_tx,
                                    spectrum_generation,
                                    &buffer,
                                    Provider::Spotify,
                                );
                            }
                        }
                        Ok::<(), String>(())
                    })();
                    if let Err(error) = result {
                        spotify_capture = None;
                        spectrum_generation = spectrum_generation.wrapping_add(1);
                        spectrum_epoch.store(spectrum_generation, Ordering::Release);
                        if let Ok(mut st) = audio_state.write() {
                            st.spotify_audio_error = Some(error);
                            st.spectrum = spectrum::Snapshot::default();
                            if let Ok(mut latest) = VISUALIZER_SPECTRUM
                                .get_or_init(|| RwLock::new(spectrum::Snapshot::default()))
                                .write()
                            {
                                *latest = spectrum::Snapshot::default();
                            }
                            if let Ok(mut clients) = broadcaster().clients.lock() {
                                clients.clear();
                            }
                        }
                    }
                } else {
                    if spotify_capture.take().is_some() {
                        spectrum_generation = spectrum_generation.wrapping_add(1);
                        spectrum_epoch.store(spectrum_generation, Ordering::Release);
                        if let Ok(mut st) = audio_state.write() {
                            st.spectrum = spectrum::Snapshot::default();
                        }
                        if let Ok(mut latest) = VISUALIZER_SPECTRUM
                            .get_or_init(|| RwLock::new(spectrum::Snapshot::default()))
                            .write()
                        {
                            *latest = spectrum::Snapshot::default();
                        }
                    }
                }
                // The Deezer ABI reads into initialized storage; Spotify appends.
                buffer.resize(262144, 0);
                thread::sleep(Duration::from_millis(5));
            }
            drop(spectrum_tx);
            let _ = spectrum_thread.join();
            if let Some(engine) = enabled_engine {
                unsafe {
                    if let Ok(enable) = engine
                        .library
                        .get::<unsafe extern "C" fn(i32)>(b"DZEnableBroadcast\0")
                    {
                        enable(0);
                    }
                }
            }
        });
        let active_clone = active.clone();
        let state_clone = state.clone();
        let cmd_tx_clone = command_tx.clone();

        let server_thread = thread::Builder::new()
            .name("deeznats-connect".into())
            .spawn(move || {
                while active_clone.load(Ordering::Relaxed) {
                    let stream: std::io::Result<TcpStream> = match listener.accept() {
                        Ok((stream, _)) => Ok(stream),
                        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                            thread::sleep(Duration::from_millis(25));
                            continue;
                        }
                        Err(_) => break,
                    };
                    if !active_clone.load(Ordering::Relaxed) {
                        break;
                    }
                    if let Ok(mut stream) = stream {
                        let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
                        let _ = stream.set_write_timeout(Some(Duration::from_secs(5)));
                        let st = state_clone.clone();
                        let cmd = cmd_tx_clone.clone();
                        let active = active_clone.clone();
                        thread::spawn(move || {
                            handle_client(&mut stream, &st, &cmd, &active);
                        });
                    }
                }
            })
            .ok()?;

        Some(Self {
            audio_thread: Some(audio_thread),
            server_thread: Some(server_thread),
            state,
            command_tx,
            active,
            port: actual_port,
        })
    }

    #[allow(dead_code)]
    pub fn port(&self) -> u16 {
        self.port
    }

    pub fn share_url(&self) -> String {
        if cfg!(test) {
            return format!("http://127.0.0.1:{}/", self.port);
        }
        // UDP connect selects the LAN route without sending a packet.
        let ip = std::net::UdpSocket::bind("0.0.0.0:0")
            .and_then(|socket| {
                socket.connect("192.0.2.1:80")?;
                socket.local_addr()
            })
            .map(|address| address.ip())
            .unwrap_or(std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST));
        format!("http://{ip}:{}/", self.port)
    }

    pub fn update(&self, snapshot: &Snapshot) {
        if let Ok(mut st) = self.state.write() {
            if st.snapshot.provider != snapshot.provider {
                if let Ok(mut clients) = broadcaster().clients.lock() {
                    clients.clear();
                }
            }
            st.engine = if snapshot.provider == Provider::Deezer {
                snapshot.engine.clone()
            } else {
                None
            };
            st.snapshot = snapshot.clone();
        }
    }
}

impl Drop for ConnectHub {
    fn drop(&mut self) {
        self.active.store(false, Ordering::Relaxed);
        if let Ok(mut clients) = broadcaster().clients.lock() {
            clients.clear();
        }
        if let Some(handle) = self.audio_thread.take() {
            let _ = handle.join();
        }
        if let Some(handle) = self.server_thread.take() {
            let _ = handle.join();
        }
    }
}

fn handle_client(
    stream: &mut TcpStream,
    state: &Arc<RwLock<HubState>>,
    command_tx: &SyncSender<Command>,
    active: &Arc<AtomicBool>,
) {
    if !active.load(Ordering::Relaxed) {
        return;
    }
    let mut buffer = vec![0u8; 16384];
    let mut bytes_read = match stream.read(&mut buffer) {
        Ok(n) if n > 0 => n,
        _ => return,
    };
    // TCP may split headers and JSON across reads.
    let header_end = loop {
        if let Some(pos) = buffer[..bytes_read]
            .windows(4)
            .position(|w| w == b"\r\n\r\n")
        {
            break pos + 4;
        }
        if bytes_read == buffer.len() {
            return;
        }
        match stream.read(&mut buffer[bytes_read..]) {
            Ok(n) if n > 0 => bytes_read += n,
            _ => return,
        }
    };
    let headers = String::from_utf8_lossy(&buffer[..header_end]);
    let content_length = headers
        .lines()
        .filter_map(|l| l.split_once(':'))
        .find(|(k, _)| k.eq_ignore_ascii_case("content-length"))
        .and_then(|(_, v)| v.trim().parse::<usize>().ok())
        .unwrap_or(0);
    if content_length > 1024 * 1024 {
        return;
    }
    let total = header_end + content_length;
    if buffer.len() < total {
        buffer.resize(total, 0);
    }
    while bytes_read < total {
        match stream.read(&mut buffer[bytes_read..total]) {
            Ok(n) if n > 0 => bytes_read += n,
            _ => return,
        }
    }
    let request_str = String::from_utf8_lossy(&buffer[..bytes_read]);
    let mut lines = request_str.lines();
    let request_line = match lines.next() {
        Some(line) => line,
        None => return,
    };

    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or("GET");
    let full_path = parts.next().unwrap_or("/");

    let (path, query) = full_path.split_once('?').unwrap_or((full_path, ""));

    let body_start = request_str
        .find("\r\n\r\n")
        .map(|p| p + 4)
        .unwrap_or(bytes_read);
    let body_slice = &request_str[body_start..];

    // A captured host provider prevents requests falling through to Deezer
    // catalog calls or public fallbacks while Spotify is selected.
    let host = state.read().unwrap().snapshot.clone();
    let provider_name = if host.provider == Provider::Spotify {
        "spotify"
    } else {
        "deezer"
    };
    if get_query_param(query, "host_provider").is_some_and(|p| p != provider_name) {
        send_error(stream, "Host provider changed. Refresh the catalog.");
        return;
    }
    if method == "POST" && path.starts_with("/api/") {
        let payload = serde_json::from_str::<serde_json::Value>(body_slice).unwrap_or_default();
        if !valid_host_payload(host.provider, &payload) {
            send_error(
                stream,
                "Host provider changed. Refresh the catalog and try again.",
            );
            return;
        }
    }
    let command_tx = HostCommands {
        tx: command_tx,
        state,
        provider: host.provider,
    };
    if host.provider == Provider::Spotify {
        if let Some(result) =
            spotify_station_request(method, path, query, body_slice, &host, &command_tx)
        {
            match result {
                Ok(value) => send_json(stream, &value),
                Err(error) => send_error(stream, &error),
            }
            return;
        }
        if let Some(result) = spotify_guest_request(method, path, query, &host) {
            match result {
                Ok(value) => send_json(stream, &value),
                Err(error) => send_error(stream, &error),
            }
            return;
        }
    }
    match (method, path) {
        ("GET", "/") | ("GET", "/index.html") => {
            let html = include_str!("connect/index.html");
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\nAccess-Control-Allow-Origin: *\r\n\r\n{}",
                html.len(),
                html
            );
            let _ = stream.write_all(resp.as_bytes());
        }
        ("GET", "/ui.js") | ("GET", "/ui.css") | ("GET", "/translations.json") => {
            let (content, mime) = if path == "/ui.js" {
                (include_str!("connect/ui.js"), "text/javascript")
            } else if path == "/ui.css" {
                (include_str!("connect/ui.css"), "text/css")
            } else {
                (
                    include_str!("connect/translations.json"),
                    "application/json",
                )
            };
            let head = format!("HTTP/1.1 200 OK\r\nContent-Type: {mime}; charset=utf-8\r\nContent-Length: {}\r\nCache-Control: no-cache\r\nConnection: close\r\n\r\n", content.len());
            let _ = stream.write_all(head.as_bytes());
            let _ = stream.write_all(content.as_bytes());
        }
        ("GET", "/native-icons.ttf") => {
            let font = std::env::var_os("WINDIR")
                .map(std::path::PathBuf::from)
                .unwrap_or_else(|| "C:/Windows".into())
                .join("Fonts/segmdl2.ttf");
            if let Ok(bytes) = std::fs::read(font) {
                let head = format!("HTTP/1.1 200 OK\r\nContent-Type: font/ttf\r\nContent-Length: {}\r\nCache-Control: public, max-age=86400\r\nConnection: close\r\n\r\n", bytes.len());
                let _ = stream.write_all(head.as_bytes());
                let _ = stream.write_all(&bytes);
            }
        }
        ("GET", "/api/appearance") => send_json(stream, &browser_appearance()),
        ("GET", "/api/stream") => {
            let stream_target = if host.provider == Provider::Spotify {
                crate::spotify::service::audio_target()
            } else {
                None
            };
            let source_current = || {
                state
                    .read()
                    .is_ok_and(|s| s.snapshot.provider == host.provider)
                    && (host.provider != Provider::Spotify
                        || spotify_audio::same_target(
                            stream_target.as_ref(),
                            crate::spotify::service::audio_target().as_ref(),
                        ))
            };
            let available = if host.provider == Provider::Spotify {
                stream_target.is_some()
            } else {
                state
                    .read()
                    .ok()
                    .and_then(|s| s.engine.clone())
                    .is_some_and(|engine| unsafe {
                        engine
                            .library
                            .get::<unsafe extern "C" fn(i32)>(b"DZEnableBroadcast\0")
                            .is_ok()
                            && engine
                                .library
                                .get::<unsafe extern "C" fn(*mut u8, i32) -> i32>(
                                    b"DZReadBroadcast\0",
                                )
                                .is_ok()
                    })
            };
            if !available {
                send_error(
                    stream,
                    if host.provider == Provider::Spotify {
                        "Spotify audio unavailable: start the host's supervised Soloist player first"
                    } else {
                        "Jam requires the updated playback engine and a Deezer connection"
                    },
                );
                return;
            }
            let _ = stream.set_nodelay(true);
            let _ = stream.set_read_timeout(None);
            let _ = stream.set_write_timeout(Some(Duration::from_secs(30)));
            let rx = broadcaster().subscribe();
            let (sample_rate, channels) = (spotify_audio::RATE, spotify_audio::CHANNELS);
            let header = wav_header(sample_rate, channels);
            let http_head = "HTTP/1.1 200 OK\r\nContent-Type: audio/wav\r\nAccept-Ranges: none\r\nConnection: keep-alive\r\nCache-Control: no-cache, no-store, must-revalidate\r\nPragma: no-cache\r\nExpires: 0\r\nAccess-Control-Allow-Origin: *\r\n\r\n";
            // Reserve bounded jitter headroom. Native clients retain the 500 ms
            // default; browsers can request more for mobile links and recovery.
            let buffer_bytes = stream_buffer_bytes(query);
            let mut prebuffer = Vec::with_capacity(buffer_bytes);
            let started = std::time::Instant::now();
            while prebuffer.len() < buffer_bytes && active.load(Ordering::Relaxed) {
                if !source_current() {
                    send_error(stream, "Host audio source changed. Reconnect to listen.");
                    return;
                }
                match rx.recv_timeout(Duration::from_millis(200)) {
                    Ok(chunk) => prebuffer.extend_from_slice(&chunk),
                    Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                        let error = state
                            .read()
                            .ok()
                            .and_then(|s| s.spotify_audio_error.clone())
                            .unwrap_or_else(|| {
                                "Host audio source changed. Reconnect to listen.".into()
                            });
                        send_error(stream, &error);
                        return;
                    }
                    Err(_) if started.elapsed() > Duration::from_secs(10) => return,
                    Err(_) => continue,
                }
            }
            if !active.load(Ordering::Relaxed) || !source_current() {
                return;
            }
            if stream.write_all(http_head.as_bytes()).is_err()
                || stream.write_all(&header).is_err()
                || stream.write_all(&prebuffer).is_err()
            {
                return;
            }
            while active.load(Ordering::Relaxed) && source_current() {
                match rx.recv_timeout(Duration::from_millis(100)) {
                    Ok(chunk) => {
                        if !source_current() {
                            break;
                        }
                        if stream.write_all(&chunk).is_err() {
                            break;
                        }
                    }
                    // Only the engine produces PCM (including paused silence).
                    // A wall-clock timeout must never alter the audio timeline.
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
                    Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                        break;
                    }
                }
            }
            return;
        }
        ("GET", "/api/lyrics") => {
            let (engine, track, duration) = (host.engine.clone(), host.now.clone(), host.duration);
            let id = track.id.clone();
            let ui = crate::ui::Ui::load_preferences();
            let result = engine
                .ok_or_else(|| "Connect the selected provider on the host first".to_string())
                .and_then(|engine| {
                    crate::lyrics::fetch_best_lyrics(&engine, &track, &id, ui.lyrics_sources)
                });
            match result {
                Ok((provider, lines)) => {
                    let gaps = crate::lyrics::instrumental_gaps(&lines, duration);
                    let timed = crate::lyrics::has_timing(&lines);
                    let lines: Vec<_> = lines.iter().map(|l| {
                        let syllables: Vec<_> = l.syllables.iter().map(|w| serde_json::json!({"start_ms":w.start_ms,"end_ms":w.end_ms,"text":w.text})).collect();
                        serde_json::json!({"start_ms":l.start_ms,"end_ms":l.end_ms,"text":l.text,"translation":l.translation,"syllables":syllables})
                    }).collect();
                    send_json(
                        stream,
                        &serde_json::json!({"id": id, "provider": provider, "lines": lines, "karaoke": ui.karaoke_lyrics, "blur": ui.blur_inactive_lyrics, "gaps": gaps, "timed": timed}),
                    );
                }
                Err(error) => send_json(
                    stream,
                    &serde_json::json!({"id": id, "lines": [], "error": error}),
                ),
            }
        }
        ("GET", "/api/spectrum") => {
            let (levels, available) = state
                .read()
                .map(|st| {
                    let fresh = st.spectrum.fresh();
                    (if fresh { st.spectrum.levels } else { [0.0; 32] }, fresh)
                })
                .unwrap_or(([0.0; 32], false));
            send_json(
                stream,
                &serde_json::json!({"levels":levels,"available":available}),
            );
        }
        ("GET", "/api/state") => {
            let json = {
                let st = state.read().unwrap();
                let snap = &st.snapshot;
                let queue_json: Vec<_> = snap
                    .queue
                    .iter()
                    .enumerate()
                    .map(|(idx, e)| {
                        serde_json::json!({
                                    "index": idx,
                                    "id": e.id,
                                    "kind": e.kind,
                                    "title": e.title,
                                    "artist": e.subtitle,
                                    "artwork": e.artwork,
                                    "duration": e.duration / 1000, "duration_ms": e.duration,
                        "added_by": e.raw["added_by"].as_str().unwrap_or(""),
                                })
                    })
                    .collect();

                let album = snap
                    .now
                    .raw
                    .pointer("/album/title")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("")
                    .to_string();

                // Sample the engine now, rather than forwarding a UI snapshot
                // that may already be several hundred milliseconds old.
                let (position, playback_state) = if let Some(engine) = &st.engine {
                    unsafe {
                        let playback_state = match (engine.state)() {
                            1 => "loading",
                            2 => "playing",
                            3 => "paused",
                            4 => "error",
                            _ => "stopped",
                        };
                        (
                            (engine.position)().max(0) as u64,
                            playback_state.to_string(),
                        )
                    }
                } else {
                    (snap.position, snap.state.to_ascii_lowercase())
                };
                serde_json::json!({
                "provider": if snap.provider == Provider::Spotify { "spotify" } else { "deezer" },
                "stream_available": if snap.provider == Provider::Spotify { crate::spotify::service::audio_target().is_some() } else { st.engine.is_some() },
                "stream_error": if snap.provider == Provider::Spotify { st.spotify_audio_error.clone() } else { None },
                "connected": snap.connected,
                    "state": playback_state,
                    "title": snap.now.title,
                    "artist": snap.now.subtitle,
                    "album": album,
                    "artwork": snap.now.artwork,
                    "id": snap.now.id,
                    "position_ms": position,
                    "duration_ms": snap.duration,
                    "queue_index": snap.queue_index,
                    "repeat": snap.repeat,
                    "shuffle": snap.shuffle,
                    "autoplay": snap.autoplay,
                    "queue": queue_json,
                })
            };
            send_json(stream, &json);
        }
        ("GET", "/api/discover") => {
            let engine = host.engine.clone();
            let json = fetch_discover(engine.as_deref());
            send_json(stream, &json);
        }
        ("GET", "/api/style-mix/genres") => {
            let region =
                get_query_param(query, "region").and_then(|region| region.parse::<usize>().ok());
            let result = region
                .ok_or_else(|| "Choose a valid language".to_string())
                .and_then(crate::style_mix::genre_options);
            match result {
                Ok(value) => send_json(stream, &value),
                Err(error) => send_json(
                    stream,
                    &serde_json::json!({"region":region,"error":error,"genres":[]}),
                ),
            }
        }
        ("GET", "/api/style-mix") => {
            let mut response = crate::style_mix::options();
            if let Some(key) = get_query_param(query, "selection") {
                response["selection"] = serde_json::json!(key);
                let engine = host.engine.clone();
                let result = crate::style_mix::Selection::parse(&key).and_then(|selection| {
                    let engine = engine
                        .as_deref()
                        .ok_or("Connect Deezer on the host first")?;
                    crate::style_mix::tracks(engine, selection).map(|tracks| (selection, tracks))
                });
                match result {
                    Ok((selection, tracks)) => {
                        response["selection"] = serde_json::json!(selection.key());
                        response["title"] = serde_json::json!(selection.title());
                        response["tracks"] = serde_json::json!(tracks
                            .iter()
                            .map(style_mix_track)
                            .collect::<Vec<_>>());
                    }
                    Err(error) => response["error"] = serde_json::json!(error),
                }
            }
            send_json(stream, &response);
        }
        ("POST", "/api/style-mix/start") => {
            let payload = serde_json::from_str::<serde_json::Value>(body_slice).unwrap_or_default();
            let engine = host.engine.clone();
            let result =
                crate::style_mix::Selection::parse(payload["selection"].as_str().unwrap_or(""))
                    .and_then(|selection| {
                        crate::style_mix::tracks(
                            engine
                                .as_deref()
                                .ok_or("Connect Deezer on the host first")?,
                            selection,
                        )
                    });
            match result {
                Ok(tracks) => {
                    let tracks: Vec<_> = tracks
                        .into_iter()
                        .map(|e| attribute_entry(e, &payload))
                        .collect();
                    let first = if let Some(id) = payload["track_id"].as_str() {
                        tracks.iter().find(|t| t.id == id)
                    } else {
                        tracks.first()
                    };
                    if let Some(first) = first {
                        if command_tx
                            .send(Command::PlayEntry(Box::new(first.clone()), tracks))
                            .is_ok()
                        {
                            send_ok(stream);
                        } else {
                            send_error(stream, "Playback is unavailable");
                        }
                    } else {
                        send_error(stream, "Track is no longer available in this mix");
                    }
                }
                Err(error) => send_error(stream, &error),
            }
        }
        ("GET", "/api/flow") => {
            let config_id = get_query_param(query, "config").unwrap_or_default();
            let engine = host.engine.clone();
            let json = fetch_flow(engine.as_deref(), &config_id);
            send_json(stream, &json);
        }
        ("POST", "/api/flow/start") => {
            let engine = host.engine.clone();
            let config_id = if let Ok(val) = serde_json::from_str::<serde_json::Value>(body_slice) {
                val.get("config")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("")
                    .to_string()
            } else {
                String::new()
            };
            if let Some(dz) = engine {
                let call = if config_id.is_empty() || config_id == "default" {
                    dz.invoke("DZFlowJSON", &[])
                } else {
                    dz.invoke("DZFlowForConfigJSON", &[serde_json::json!(config_id)])
                };
                if let Ok(val) = call {
                    let payload =
                        serde_json::from_str::<serde_json::Value>(body_slice).unwrap_or_default();
                    let tracks: Vec<_> = crate::catalog::entries(&val, "track")
                        .into_iter()
                        .map(|entry| attribute_entry(entry, &payload))
                        .collect();
                    if let Some(first) = tracks.first() {
                        let _ =
                            command_tx.send(Command::PlayEntry(Box::new(first.clone()), tracks));
                        send_ok(stream);
                        return;
                    }
                }
            }
            send_error(stream, "Flow could not be started");
        }
        ("GET", "/api/flow/tuner") => {
            let flow_id = get_query_param(query, "id").unwrap_or_else(|| "default".to_string());
            let engine = host.engine.clone();
            if let Some(dz) = engine {
                if let Ok(tuner) = dz.invoke("DZFlowTunerJSON", &[serde_json::json!(flow_id)]) {
                    send_json(stream, &tuner);
                    return;
                }
            }
            let fallback_genres = [
                ("132", "Pop", true),
                ("152", "Rock", true),
                ("116", "Rap/Hip Hop", true),
                ("113", "Dance/Electro", true),
                ("165", "R&B", true),
                ("153", "Metal", false),
                ("129", "Jazz", false),
                ("98", "Classical", false),
                ("85", "Alternative", true),
                ("464", "Folk/Acoustic", false),
                ("144", "Reggae", false),
                ("84", "Country", false),
                ("52", "Blues", false),
                ("71", "Indie", true),
                ("173", "Soundtrack", false),
            ];
            let genres: Vec<_> = fallback_genres
                .iter()
                .map(|(id, title, enabled)| {
                    serde_json::json!({
                        "id": id,
                        "title": title,
                        "isEnabled": enabled,
                    })
                })
                .collect();
            send_json(
                stream,
                &serde_json::json!({
                    "id": flow_id,
                    "customizable": true,
                    "genres": genres,
                }),
            );
        }
        ("POST", "/api/flow/tuner/save") => {
            if let Ok(val) = serde_json::from_str::<serde_json::Value>(body_slice) {
                let flow_id = val
                    .get("flow_id")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("default")
                    .to_string();

                let edits: Vec<serde_json::Value> = if let Some(edits_arr) =
                    val.get("edits").and_then(serde_json::Value::as_array)
                {
                    edits_arr.clone()
                } else if let Some(genres_arr) =
                    val.get("genres").and_then(serde_json::Value::as_array)
                {
                    genres_arr
                        .iter()
                        .filter_map(|g| {
                            let id = g.get("id").or_else(|| g.get("clusterId"))?.as_str()?;
                            let enabled = g
                                .get("isEnabled")
                                .and_then(serde_json::Value::as_bool)
                                .unwrap_or(false);
                            Some(serde_json::json!({
                                "clusterId": id,
                                "isEnabled": enabled,
                                "isEditedByUser": true,
                            }))
                        })
                        .collect()
                } else {
                    Vec::new()
                };

                if flow_selection_is_empty(&val) {
                    send_json(
                        stream,
                        &serde_json::json!({"error":"Select at least one genre", "ok":false}),
                    );
                    return;
                }
                let engine = host.engine.clone();
                if let Some(dz) = engine {
                    let edits_str = serde_json::to_string(&edits).unwrap_or_default();
                    let invoke_res = dz.invoke(
                        "DZSaveFlowTunerJSON",
                        &[serde_json::json!(flow_id), serde_json::json!(edits_str)],
                    );
                    // A successful read does not prove a failed write succeeded.
                    match invoke_res {
                        Err(error) => {
                            send_json(stream, &serde_json::json!({"ok":false,"error":error}))
                        }
                        Ok(result) => {
                            if result.get("saved").and_then(serde_json::Value::as_bool)
                                == Some(false)
                            {
                                send_json(
                                    stream,
                                    &serde_json::json!({"ok":false,"error":"Deezer did not save the selection"}),
                                );
                            } else {
                                match dz.invoke("DZFlowTunerJSON", &[serde_json::json!(flow_id)]) {
                                    Ok(tuner) => send_json(
                                        stream,
                                        &serde_json::json!({"ok":true,"tuner":tuner}),
                                    ),
                                    Err(_) => send_json(
                                        stream,
                                        &serde_json::json!({"ok":true,"saved":true,"verified":false}),
                                    ),
                                }
                            }
                        }
                    }
                    return;
                } else {
                    send_json(
                        stream,
                        &serde_json::json!({"ok":false,"error":"Connect Deezer on the host before saving Flow"}),
                    );
                    return;
                }
            }
            send_error(stream, "Flow tercihleri kaydedilemedi");
        }

        ("GET", "/api/collection") => {
            let kind = get_query_param(query, "kind").unwrap_or_default();
            let id = get_query_param(query, "id").unwrap_or_default();
            let engine = host.engine.clone();
            let json = fetch_collection(engine.as_deref(), &kind, &id);
            send_json(stream, &json);
        }
        ("GET", "/api/search") => {
            let q = get_query_param(query, "q").unwrap_or_default();
            let engine = host.engine.clone();
            let results = search_deezer(engine.as_deref(), &q);
            send_json(stream, &results);
        }
        // Queue editing: Add Last ("Sıranın sonuna ekle")
        ("POST", "/api/queue/add-last") | ("POST", "/api/queue") => {
            if let Ok(val) = serde_json::from_str::<serde_json::Value>(body_slice) {
                if let Some(track) = ConnectTrack::from_value(&val) {
                    let _ = command_tx.send(Command::QueueAddLast(Box::new(attribute_entry(
                        track.to_entry(),
                        &val,
                    ))));
                    send_ok(stream);
                    return;
                }
            }
            send_error(stream, "Invalid track payload");
        }
        // Queue editing: Insert Next ("Sıradaki şarkı olarak ekle")
        ("POST", "/api/queue/insert-next") => {
            if let Ok(val) = serde_json::from_str::<serde_json::Value>(body_slice) {
                if let Some(track) = ConnectTrack::from_value(&val) {
                    let _ = command_tx.send(Command::QueueInsertNext(Box::new(attribute_entry(
                        track.to_entry(),
                        &val,
                    ))));
                    send_ok(stream);
                    return;
                }
            }
            send_error(stream, "Invalid track payload");
        }
        // Queue editing: Drag to move queue around
        ("POST", "/api/queue/move") => {
            if let Ok(val) = serde_json::from_str::<serde_json::Value>(body_slice) {
                if let (Some(from), Some(to)) = (
                    val.get("from").and_then(serde_json::Value::as_u64),
                    val.get("to").and_then(serde_json::Value::as_u64),
                ) {
                    let expected: Vec<String> = {
                        let st = state.read().unwrap();
                        st.snapshot.queue.iter().map(|t| t.id.clone()).collect()
                    };
                    let _ = command_tx.send(Command::QueueMove {
                        from: from as usize,
                        to: to as usize,
                        expected,
                    });
                    send_ok(stream);
                    return;
                }
            }
            send_error(stream, "Invalid move parameters");
        }
        // Queue editing: Jump/Play directly by queue index
        ("POST", "/api/queue/play-index") => {
            if let Ok(val) = serde_json::from_str::<serde_json::Value>(body_slice) {
                if let Some(idx) = val.get("index").and_then(serde_json::Value::as_u64) {
                    let _ = command_tx.send(Command::PlayQueueIndex(idx as usize));
                    send_ok(stream);
                    return;
                }
            }
            send_error(stream, "Invalid index");
        }
        // Queue editing: Clear queue
        ("POST", "/api/queue/clear") => {
            let _ = command_tx.send(Command::QueueClear);
            send_ok(stream);
        }
        // Direct Play Track
        ("POST", "/api/play") => {
            if let Ok(val) = serde_json::from_str::<serde_json::Value>(body_slice) {
                if let Some(track) = ConnectTrack::from_value(&val) {
                    let entry = attribute_entry(track.to_entry(), &val);
                    let _ =
                        command_tx.send(Command::PlayEntry(Box::new(entry.clone()), vec![entry]));
                    send_ok(stream);
                    return;
                }
            }
            send_error(stream, "Invalid track");
        }
        // Play entire album/playlist collection
        ("POST", "/api/play-collection") => {
            if let Ok(val) = serde_json::from_str::<serde_json::Value>(body_slice) {
                if let Some(items) = val.get("tracks").and_then(serde_json::Value::as_array) {
                    let entries: Vec<_> = items
                        .iter()
                        .filter_map(ConnectTrack::from_value)
                        .map(|t| attribute_entry(t.to_entry(), &val))
                        .collect();
                    if let Some(first) = entries.first() {
                        let _ =
                            command_tx.send(Command::PlayEntry(Box::new(first.clone()), entries));
                        send_ok(stream);
                        return;
                    }
                }
            }
            send_error(stream, "Invalid collection");
        }
        ("POST", "/api/toggle") => {
            let _ = command_tx.send(Command::Toggle);
            send_ok(stream);
        }
        ("POST", "/api/next") => {
            let _ = command_tx.send(Command::Next);
            send_ok(stream);
        }
        ("POST", "/api/previous") => {
            let _ = command_tx.send(Command::Previous);
            send_ok(stream);
        }
        ("POST", "/api/shuffle") | ("POST", "/api/repeat") | ("POST", "/api/autoplay") => {
            if !state.read().map(|s| s.snapshot.connected).unwrap_or(false) {
                send_error(stream, "Connect the selected provider on the host first");
                return;
            }
            let value = serde_json::from_str::<serde_json::Value>(body_slice).unwrap_or_default();
            let command = if path == "/api/shuffle" {
                value["enabled"].as_bool().map(Command::Shuffle)
            } else if path == "/api/autoplay" {
                value["enabled"].as_bool().map(Command::Autoplay)
            } else {
                value["mode"]
                    .as_u64()
                    .filter(|mode| *mode <= 2)
                    .map(|mode| Command::Repeat(mode as u8))
            };
            match command {
                Some(command) => {
                    if command_tx.try_send(command).is_ok() {
                        send_ok(stream)
                    } else {
                        send_error(stream, "Playback is busy. Please retry.")
                    }
                }
                None => send_error(stream, "Invalid playback mode"),
            }
        }
        ("POST", "/api/seek") => {
            if let Ok(val) = serde_json::from_str::<serde_json::Value>(body_slice) {
                if let Some(ms) = val.get("position_ms").and_then(serde_json::Value::as_u64) {
                    let _ = command_tx.send(Command::SeekMs(ms));
                    send_ok(stream);
                    return;
                }
            }
            send_error(stream, "Invalid seek");
        }
        ("OPTIONS", _) => {
            let resp = "HTTP/1.1 204 No Content\r\nAccess-Control-Allow-Origin: *\r\nAccess-Control-Allow-Methods: GET, POST, OPTIONS\r\nAccess-Control-Allow-Headers: Content-Type\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
            let _ = stream.write_all(resp.as_bytes());
        }
        _ => {
            let resp = "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
            let _ = stream.write_all(resp.as_bytes());
        }
    }
}

struct HostCommands<'a> {
    tx: &'a SyncSender<Command>,
    state: &'a Arc<RwLock<HubState>>,
    provider: Provider,
}
impl HostCommands<'_> {
    fn send(&self, command: Command) -> Result<(), String> {
        self.try_send(command)
    }
    fn try_send(&self, command: Command) -> Result<(), String> {
        let state = self.state.read().map_err(|_| "Host unavailable")?;
        if state.snapshot.provider != self.provider || Provider::active() != self.provider {
            return Err("Host provider changed".into());
        }
        self.tx
            .try_send(command)
            .map_err(|_| "Playback is busy. Please retry.".into())
    }
}

fn valid_host_payload(provider: Provider, value: &serde_json::Value) -> bool {
    let name = if provider == Provider::Spotify {
        "spotify"
    } else {
        "deezer"
    };
    if value["provider"].as_str().is_some_and(|p| p != name) {
        return false;
    }
    if let Some(id) = value["id"].as_str() {
        if !id.is_empty() && (id.starts_with("spotify:") != (provider == Provider::Spotify)) {
            return false;
        }
    }
    value["tracks"].as_array().is_none_or(|tracks| {
        tracks.iter().all(|track| {
            if !track.is_object() {
                return false;
            }
            let mut track = track.clone();
            track["provider"] = serde_json::json!(name);
            valid_host_payload(provider, &track)
        })
    })
}

fn spotify_station_request(
    method: &str,
    path: &str,
    query: &str,
    body: &str,
    _host: &Snapshot,
    commands: &HostCommands<'_>,
) -> Option<Result<serde_json::Value, String>> {
    use serde_json::{json, Value};
    if !path.starts_with("/api/flow") && !path.starts_with("/api/style-mix") {
        return None;
    }
    let invoke = |name: &str, args: &[Value]| crate::spotify::catalog_invoke(name, args);
    let payload: Value = serde_json::from_str(body).unwrap_or_default();
    Some((|| match (method, path) {
        ("GET", "/api/flow") => {
            let config = get_query_param(query, "config").unwrap_or_else(|| "default".into());
            let options = invoke("DZFlowConfigsJSON", &[])?;
            let tracks = invoke("DZFlowForConfigJSON", &[json!(config)])?;
            let tuner = invoke("DZFlowTunerJSON", &[json!(config)])?;
            Ok(
                json!({"options":options["options"],"tracks":crate::catalog::entries(&tracks,"track").iter().map(style_mix_track).collect::<Vec<_>>(),"config":config,"customizable":tuner["customizable"]}),
            )
        }
        ("GET", "/api/flow/tuner") => invoke(
            "DZFlowTunerJSON",
            &[json!(
                get_query_param(query, "id").unwrap_or_else(|| "default".into())
            )],
        ),
        ("POST", "/api/flow/tuner/save") => {
            if flow_selection_is_empty(&payload) {
                return Err("Select at least one genre".into());
            }
            let id = payload["flow_id"].as_str().unwrap_or("default");
            let edits: Vec<_> = payload["genres"].as_array().into_iter().flatten().map(|genre| json!({"clusterId":genre["id"],"isEnabled":genre["enabled"].as_bool().unwrap_or(false),"isEditedByUser":true})).collect();
            invoke(
                "DZSaveFlowTunerJSON",
                &[json!(id), json!(json!(edits).to_string())],
            )
        }
        ("GET", "/api/style-mix/genres") => {
            let region = get_query_param(query, "region")
                .and_then(|v| v.parse::<usize>().ok())
                .ok_or("Choose a valid language")?;
            crate::style_mix::genre_options(region)
        }
        ("GET", "/api/style-mix") if get_query_param(query, "selection").is_none() => {
            Ok(crate::style_mix::options())
        }
        ("GET", "/api/style-mix")
        | ("POST", "/api/style-mix/start")
        | ("POST", "/api/flow/start") => {
            let style = path.starts_with("/api/style-mix");
            let (name, args, selection) = if style {
                let key = if method == "GET" {
                    get_query_param(query, "selection").unwrap_or_default()
                } else {
                    payload["selection"].as_str().unwrap_or_default().to_owned()
                };
                let selection = crate::style_mix::Selection::parse(&key)?;
                (
                    "SPStyleMixJSON",
                    vec![json!(selection.key())],
                    Some(selection),
                )
            } else {
                (
                    "DZFlowForConfigJSON",
                    vec![json!(payload["config"].as_str().unwrap_or("default"))],
                    None,
                )
            };
            let value = if style {
                crate::catalog::spotify_style_mix(&selection.as_ref().unwrap().key())?
            } else {
                invoke(name, &args)?
            };
            let tracks: Vec<_> = crate::catalog::entries(&value, "track")
                .into_iter()
                .map(|entry| {
                    let mut entry = entry;
                    if let Some(selection) = &selection {
                        entry.raw[crate::style_mix::MARKER] = json!(selection.key());
                        entry.raw["style_mix_title"] = json!(selection.title());
                    }
                    entry.raw["spotify_station"] = json!({"name":name,"args":args});
                    attribute_entry(entry, &payload)
                })
                .collect();
            if method == "POST" {
                let first = tracks
                    .iter()
                    .find(|entry| payload["track_id"].as_str() == Some(entry.id.as_str()))
                    .or_else(|| tracks.first())
                    .ok_or("This mix returned no playable tracks")?;
                commands.try_send(Command::PlayEntry(Box::new(first.clone()), tracks))?;
                Ok(json!({"ok":true}))
            } else {
                Ok(
                    json!({"selection":selection.as_ref().map(|s|s.key()),"title":selection.as_ref().map(|s|s.title()),"tracks":tracks.iter().map(style_mix_track).collect::<Vec<_>>()}),
                )
            }
        }
        _ => Err("Unknown Spotify station action".into()),
    })())
}
// These calls use the host's saved Spotify session, never guest credentials.
fn spotify_guest_request(
    method: &str,
    path: &str,
    query: &str,
    host: &Snapshot,
) -> Option<Result<serde_json::Value, String>> {
    use serde_json::{json, Value};
    let invoke = |name: &str, args: &[Value]| crate::spotify::catalog_invoke(name, args);
    let result = match (method, path) {
        ("GET", "/api/search") => {
            let q = get_query_param(query, "q").unwrap_or_default();
            if q.trim().is_empty() { Ok(normalize_search(&json!({}))) }
            else { invoke("DZSearchJSON", &[json!(q)]).map(|value| {
                let mut result = normalize_search(&value);
                result["podcasts"] = json!(search_items(&value["shows"], "podcast"));
                result
            }) }
        }
        ("GET", "/api/discover") => (|| {
            let value = invoke("SPCatalogDiscoveryJSON", &[])?;
            let sections: Vec<_> = value["sections"].as_array().into_iter().flatten().map(|section| json!({
                "title":section["title"],"items":search_items(&section["items"],"playlist"),"error":section["error"]
            })).collect();
            Ok(json!({"sections":sections,"provider":"spotify"}))
        })(),
        ("GET", "/api/collection") => (|| {
            let kind = get_query_param(query, "kind").unwrap_or_default();
            let id = get_query_param(query, "id").unwrap_or_default();
            if !id.starts_with("spotify:") { return Err("Refresh the host catalog before opening this item".into()); }
            let name = match kind.as_str() {
                "album" => "DZAlbumTracksJSON", "playlist" => "DZPlaylistTracksJSON",
                "podcast" | "show" => "DZPodcastEpisodesJSON", "artist" => "DZArtistSectionJSON",
                _ => return Err("Unsupported Spotify collection".into()),
            };
            let value = invoke(name, &[json!(id), json!(0)])?;
            let tracks = search_items(&value, if kind == "artist" { "album" } else if kind == "show" || kind == "podcast" { "episode" } else { "track" });
            Ok(json!({"id":id,"kind":kind,"tracks":tracks,"provider":"spotify"}))
        })(),
        ("GET", "/api/lyrics") => {
            let ui = crate::ui::Ui::load_preferences();
            crate::lyrics::fetch_spotify_lyrics(&host.now, &host.now.id, ui.lyrics_sources).map(|(provider, lines)| {
                let lines: Vec<_> = lines.iter().map(|l| json!({"start_ms":l.start_ms,"end_ms":l.end_ms,"text":l.text,"translation":l.translation,"syllables":l.syllables.iter().map(|w|json!({"start_ms":w.start_ms,"end_ms":w.end_ms,"text":w.text})).collect::<Vec<_>>()})).collect();
                json!({"id":host.now.id,"duration_ms":host.duration,"provider":provider,"lines":lines,"karaoke":ui.karaoke_lyrics,"blur":ui.blur_inactive_lyrics})
            })
        }
        (_, p) if p.starts_with("/api/flow") || p.starts_with("/api/style-mix") => Err("Spotify does not expose this mix or tuner through its supported API. Use host search or your Spotify library.".into()),
        _ => return None,
    };
    Some(result)
}

fn send_json(stream: &mut TcpStream, json: &serde_json::Value) {
    let body = json.to_string();
    let resp = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\nAccess-Control-Allow-Origin: *\r\n\r\n{}",
        body.len(),
        body
    );
    let _ = stream.write_all(resp.as_bytes());
}

fn send_ok(stream: &mut TcpStream) {
    let body = "{\"ok\":true}";
    let resp = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\nAccess-Control-Allow-Origin: *\r\n\r\n{}",
        body.len(),
        body
    );
    let _ = stream.write_all(resp.as_bytes());
}

fn send_error(stream: &mut TcpStream, msg: &str) {
    let body = serde_json::json!({ "ok": false, "error": msg }).to_string();
    let resp = format!(
        "HTTP/1.1 400 Bad Request\r\nContent-Type: application/json; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\nAccess-Control-Allow-Origin: *\r\n\r\n{}",
        body.len(),
        body
    );
    let _ = stream.write_all(resp.as_bytes());
}

fn stream_buffer_bytes(query: &str) -> usize {
    let milliseconds = get_query_param(query, "buffer_ms")
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(500)
        .clamp(250, 2000);
    (44100 * milliseconds / 1000) * 4
}
fn get_query_param(query: &str, param: &str) -> Option<String> {
    query.split('&').find_map(|pair| {
        let (k, v) = pair.split_once('=')?;
        (k == param).then(|| url_decode(v))
    })
}

fn fetch_discover(engine: Option<&Deezer>) -> serde_json::Value {
    let personal = [
        (3, "\u{00d6}nerilen \u{00c7}alma Listeleri / Recommended"),
        (
            1,
            "Sizin \u{0130}\u{00e7}in Se\u{00e7}ilenler / Made for you",
        ),
        (6, "Sanat\u{00e7}\u{0131}lar\u{0131}n\u{0131}zdan Listeler"),
    ];
    let public = [
        (
            "chart/0/playlists?limit=25",
            "En \u{00c7}ok Dinlenen \u{00c7}alma Listeleri",
            "playlist",
        ),
        (
            "editorial/0/releases?limit=25",
            "Yeni \u{00c7}\u{0131}kan Alb\u{00fc}mler",
            "album",
        ),
        (
            "editorial/0/selection?limit=25",
            "Edit\u{00f6}r\u{00fc}n Se\u{00e7}tikleri",
            "album",
        ),
        (
            "editorial?limit=30",
            "M\u{00fc}zik T\u{00fc}rleri ve Kategoriler",
            "channel",
        ),
    ];
    let personal_count = if engine.is_some() { personal.len() } else { 0 };
    let mut results = vec![None; personal_count + public.len()];
    let _ = crate::catalog::parallel::load(
        results.len(),
        |index| {
            let (title, items) = if index < personal_count {
                let (module, title) = personal[index];
                let value = engine?
                    .invoke("DZHomeModuleJSON", &[serde_json::json!(module)])
                    .ok()?;
                let items = crate::catalog::home_module_entries(&value,module,"playlist").into_iter()
                .map(|entry| serde_json::json!({"id":entry.id,"title":entry.title,"subtitle":entry.subtitle,
                    "artwork":entry.artwork,"kind":entry.kind})).collect::<Vec<_>>();
                (title, items)
            } else {
                let (endpoint, title, kind) = public[index - personal_count];
                let url = format!("https://api.deezer.com/{endpoint}");
                let value = crate::catalog::public_json(&url, || {
                    crate::lyrics::http::request("GET", &url, &[], &[])
                })
                .ok()?;
                let rows = value
                    .get("data")
                    .or_else(|| value.get("playlists"))?
                    .as_array()?;
                let items = rows.iter().filter_map(|item| {
                let id = item.get("id")?.to_string();
                let title = item.get("title").or_else(||item.get("name"))?.as_str()?;
                let subtitle = item.pointer("/user/name").or_else(||item.pointer("/artist/name"))
                    .and_then(serde_json::Value::as_str).unwrap_or("");
                let artwork = item.get("picture_medium").or_else(||item.get("cover_medium"))
                    .or_else(||item.get("picture_big")).or_else(||item.get("cover_big"))
                    .and_then(serde_json::Value::as_str).unwrap_or("");
                Some(serde_json::json!({"id":id,"title":title,"subtitle":subtitle,"artwork":artwork,"kind":kind}))
            }).collect::<Vec<_>>();
                (title, items)
            };
            (!items.is_empty()).then(|| serde_json::json!({"title":title,"items":items}))
        },
        |index, section| {
            results[index] = section;
            Ok(())
        },
    );
    serde_json::json!({"sections":results.into_iter().flatten().collect::<Vec<_>>()})
}

fn style_mix_track(entry: &Entry) -> serde_json::Value {
    serde_json::json!({"id":entry.id,"title":entry.title,"artist":entry.subtitle,
        "album":entry.album,"artwork":entry.artwork,"duration_ms":entry.duration,
        "duration":entry.duration/1000,"kind":"track", "style_mix":entry.raw[crate::style_mix::MARKER]})
}

fn fetch_flow(engine: Option<&Deezer>, config_id: &str) -> serde_json::Value {
    let mut options = Vec::new();
    let mut tracks = Vec::new();
    let mut customizable = false;

    if let Some(dz) = engine {
        if let Ok(cfg) = dz.invoke("DZFlowConfigsJSON", &[]) {
            if let Some(arr) = cfg.get("options").and_then(serde_json::Value::as_array) {
                for item in arr.iter().filter(|v| v["kind"].as_str() != Some("genre")) {
                    let id = crate::catalog::string(item, &["id"]);
                    let title = crate::catalog::string(item, &["title"]);
                    if !id.is_empty() {
                        let icon = match id.as_str() {
                            "default" => "🌊",
                            "chill" => "☕",
                            "motivation" => "⚡",
                            "focus" => "🎯",
                            "party" => "🎉",
                            "melancholy" => "🌧️",
                            "workout" => "🔥",
                            _ => "🎵",
                        };
                        options.push(serde_json::json!({
                            "id": id,
                            "title": title,
                            "description": "",
                            "icon": icon,
                            "active": id == config_id || (config_id.is_empty() && id == "default"),
                        }));
                    }
                }
            }
        }
    }

    if options.is_empty() {
        let default_options = [
            (
                "default",
                "Kişisel Flow",
                "Tamamen size özel kesintisiz müzik",
                "🌊",
            ),
            ("chill", "Chill", "Rahatlatıcı, huzurlu parçalar", "☕"),
            (
                "motivation",
                "Motivasyon",
                "Enerjinizi yükseltecek şarkılar",
                "⚡",
            ),
            (
                "focus",
                "Odaklanma",
                "Çalışırken ve üretirken eşlikçi",
                "🎯",
            ),
            ("party", "Parti", "Eğlenceli ve hareketli ritimler", "🎉"),
            ("melancholy", "Hüzün", "Duygusal, içsel anlar için", "🌧️"),
            ("workout", "Antrenman", "Spor ve koşu temposu", "🔥"),
        ];

        for (id, title, desc, icon) in default_options {
            options.push(serde_json::json!({
                "id": id,
                "title": title,
                "description": desc,
                "icon": icon,
                "active": id == config_id || (config_id.is_empty() && id == "default"),
            }));
        }
    }

    if let Some(dz) = engine {
        let target_id = if config_id.is_empty() {
            "default"
        } else {
            config_id
        };
        if let Ok(tuner) = dz.invoke("DZFlowTunerJSON", &[serde_json::json!(target_id)]) {
            customizable = tuner["customizable"].as_bool().unwrap_or(false);
        } else if target_id == "default" {
            customizable = true;
        }

        let call = if target_id == "default" {
            dz.invoke("DZFlowJSON", &[])
        } else {
            dz.invoke("DZFlowForConfigJSON", &[serde_json::json!(target_id)])
        };
        if let Ok(val) = call {
            tracks = crate::catalog::entries(&val, "track")
                .into_iter()
                .map(|e| {
                    serde_json::json!({
                        "id": e.id,
                        "title": e.title,
                        "artist": e.subtitle,
                        "artwork": e.artwork,
                        "duration": e.duration / 1000, "duration_ms": e.duration,
                    })
                })
                .collect();
        }
    } else {
        customizable = true;
    }

    serde_json::json!({
        "options": options,
        "tracks": tracks,
        "customizable": customizable,
    })
}

fn fetch_collection(engine: Option<&Deezer>, kind: &str, id: &str) -> serde_json::Value {
    let mut title = String::new();
    let mut artwork = String::new();
    let mut tracks = Vec::new();

    if kind == "podcast" {
        let result = engine
            .ok_or_else(|| "Connect Deezer on the host".to_string())
            .and_then(|engine| engine.invoke("DZPodcastEpisodesJSON", &[serde_json::json!(id)]));
        return match result {
            Ok(value) => {
                serde_json::json!({"id":id, "kind":kind, "tracks":search_items(&value, "episode")})
            }
            Err(error) => serde_json::json!({"error":error,"tracks":[]}),
        };
    }
    if kind == "artist" {
        if let Some(engine) = engine {
            match engine.invoke("DZArtistProfileJSON", &[serde_json::json!(id)]) {
                Ok(value) => {
                    let artist =
                        crate::catalog::entry(&value["artist"], "artist").unwrap_or_default();
                    return serde_json::json!({"id":id, "kind":kind, "title":artist.title, "artwork":artist.artwork,
                        "tracks":search_items(&value["top"], "track"), "albums":search_items(&value["albums"], "album"),
                        "artists":search_items(&value["related"], "artist")});
                }
                Err(error) => return serde_json::json!({"error":error, "tracks":[]}),
            }
        }
        return serde_json::json!({"error":"Connect Deezer on the host", "tracks":[]});
    }
    // Use the same native collection methods as the desktop.
    if let Some(dz) = engine {
        let attempts: &[&str] = match kind {
            "mix" => &["DZSmartMixJSON", "DZPlaylistTracksJSON"],
            "album" => &["DZAlbumTracksJSON"],
            _ => &[
                "DZPlaylistTracksJSON",
                "DZSmartMixJSON",
                "DZAlbumTracksJSON",
            ],
        };

        for &method in attempts {
            if let Ok(val) = dz.invoke(method, &[serde_json::json!(id)]) {
                let entries = crate::catalog::table_entries(&val, "track");
                if !entries.is_empty() {
                    title = val
                        .pointer("/data/smartTracklist/title")
                        .or_else(|| val.pointer("/data/smartTracklist/displayTitle"))
                        .or_else(|| val.pointer("/smartTracklist/title"))
                        .or_else(|| val.pointer("/smartTracklist/displayTitle"))
                        .or_else(|| val.pointer("/data/playlist/title"))
                        .or_else(|| val.pointer("/playlist/title"))
                        .or_else(|| val.pointer("/data/album/title"))
                        .or_else(|| val.pointer("/album/title"))
                        .or_else(|| val.get("title"))
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("")
                        .to_string();

                    artwork = val
                        .pointer("/data/smartTracklist/pictures/medium")
                        .or_else(|| val.pointer("/smartTracklist/pictures/medium"))
                        .or_else(|| val.pointer("/data/playlist/pictures/medium"))
                        .or_else(|| val.pointer("/playlist/pictures/medium"))
                        .or_else(|| val.pointer("/data/album/pictures/medium"))
                        .or_else(|| val.pointer("/album/pictures/medium"))
                        .or_else(|| val.pointer("/picture_medium"))
                        .or_else(|| val.pointer("/cover_medium"))
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("")
                        .to_string();

                    if artwork.is_empty() {
                        if let Some(first) = entries.first() {
                            artwork = first.artwork.clone();
                        }
                    }

                    tracks = entries
                        .into_iter()
                        .map(|e| {
                            serde_json::json!({
                                "id": e.id,
                                "kind": e.kind,
                                "title": e.title,
                                "artist": e.subtitle,
                                "artwork": e.artwork,
                                "duration": e.duration / 1000, "duration_ms": e.duration,
                            })
                        })
                        .collect();
                    break;
                }
            }
        }
    }

    // 2. Fall back to public Deezer API (for editorial playlists, charts, etc.)
    if tracks.is_empty() {
        let endpoint = if kind == "album" {
            format!("https://api.deezer.com/album/{id}")
        } else {
            format!("https://api.deezer.com/playlist/{id}")
        };

        if let Ok(bytes) = crate::lyrics::http::request("GET", &endpoint, &[], &[]) {
            if let Ok(value) = serde_json::from_slice::<serde_json::Value>(&bytes) {
                if title.is_empty() {
                    title = value
                        .get("title")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("")
                        .to_string();
                }
                if artwork.is_empty() {
                    artwork = value
                        .pointer("/cover_big")
                        .or_else(|| value.pointer("/picture_big"))
                        .or_else(|| value.pointer("/cover_medium"))
                        .or_else(|| value.pointer("/picture_medium"))
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("")
                        .to_string();
                }

                let track_array = value
                    .pointer("/tracks/data")
                    .or_else(|| value.get("data"))
                    .and_then(serde_json::Value::as_array);

                if let Some(arr) = track_array {
                    tracks = arr
                        .iter()
                        .filter_map(|item| {
                            let id = item.get("id")?.to_string();
                            let t_title = item.get("title")?.as_str()?.to_string();
                            let artist = item
                                .pointer("/artist/name")
                                .and_then(serde_json::Value::as_str)
                                .unwrap_or("")
                                .to_string();
                            let t_artwork = item
                                .pointer("/album/cover_medium")
                                .and_then(serde_json::Value::as_str)
                                .unwrap_or(&artwork)
                                .to_string();
                            let duration = item
                                .get("duration")
                                .and_then(serde_json::Value::as_u64)
                                .unwrap_or(0);
                            Some(serde_json::json!({
                                "id": id,
                                "title": t_title,
                                "artist": artist,
                                "artwork": t_artwork,
                                "duration": duration, "duration_ms": duration.saturating_mul(1000),
                            }))
                        })
                        .collect();
                }
            }
        }
    }

    serde_json::json!({
        "id": id,
        "kind": kind,
        "title": title,
        "artwork": artwork,
        "tracks": tracks,
    })
}

fn browser_hex(color: u32) -> String {
    format!(
        "#{:02x}{:02x}{:02x}",
        color & 255,
        (color >> 8) & 255,
        (color >> 16) & 255
    )
}

fn browser_palette(ui: &crate::ui::Ui) -> serde_json::Value {
    let keys = ["Background", "Panel", "Raised", "Border", "Text", "Muted"];
    let protected =
        crate::appearance::readable_palette(keys.map(|key| ui.color(key, 0)), ui.light, 100);
    let mut palette = serde_json::json!({});
    for key in [
        "Background",
        "Panel",
        "Raised",
        "Border",
        "Text",
        "Muted",
        "Accent",
        "AccentText",
        "GradientStart",
        "GradientEnd",
    ] {
        palette[key] = serde_json::json!(browser_hex(ui.color(key, 0)));
    }
    palette["Text"] = serde_json::json!(browser_hex(protected[4]));
    palette["Muted"] = serde_json::json!(browser_hex(protected[5]));
    palette
}

fn browser_appearance() -> serde_json::Value {
    let themes: Vec<_> = crate::ui::SOLID_THEMES.iter().enumerate().map(|(index, theme)| {
        let mut ui = crate::ui::Ui::default(); ui.apply_solid_theme(index);
        serde_json::json!({"id":index,"name":theme.0,"light":theme.1,"palette":browser_palette(&ui)})
    }).collect();
    serde_json::json!({"themes":themes,"defaults":{"theme":8,"blur":20,"opacity":40}})
}

fn flow_selection_is_empty(value: &serde_json::Value) -> bool {
    // Edits are a delta, so an all-disabled delta can still leave an unchanged
    // genre enabled. Only a complete selection can establish that none remain.
    value
        .get("genres")
        .and_then(serde_json::Value::as_array)
        .is_some_and(|genres| {
            !genres
                .iter()
                .any(|genre| genre["isEnabled"].as_bool().unwrap_or(false))
        })
}

fn search_items(value: &serde_json::Value, kind: &str) -> Vec<serde_json::Value> {
    crate::catalog::entries(value, kind).into_iter().map(|e| serde_json::json!({
        "id": e.id, "kind": e.kind, "title": e.title, "artist": e.subtitle,
        "subtitle": e.subtitle, "album": e.album, "artwork": e.artwork, "duration": e.duration / 1000, "duration_ms": e.duration,
    })).collect()
}

fn normalize_search(value: &serde_json::Value) -> serde_json::Value {
    let mut result = serde_json::json!({});
    for (key, kind) in [
        ("tracks", "track"),
        ("albums", "album"),
        ("playlists", "playlist"),
        ("artists", "artist"),
        ("podcasts", "podcast"),
    ] {
        result[key] = serde_json::json!(search_items(&value[key], kind));
    }
    result
}

fn search_deezer(engine: Option<&Deezer>, query: &str) -> serde_json::Value {
    let mut empty = normalize_search(&serde_json::json!({}));
    if query.trim().is_empty() {
        return empty;
    }
    let Some(engine) = engine else {
        empty["error"] = serde_json::json!("Connect Deezer on the host to search");
        return empty;
    };
    match engine.invoke("DZSearchJSON", &[serde_json::json!(query)]) {
        Ok(value) => {
            let mut result = normalize_search(&value);
            if result["podcasts"].as_array().is_some_and(|p| p.is_empty()) {
                if let Ok(podcasts) =
                    engine.invoke("DZSearchPodcastsJSON", &[serde_json::json!(query)])
                {
                    result["podcasts"] = serde_json::json!(search_items(&podcasts, "podcast"));
                }
            }
            result
        }
        Err(error) => {
            empty["error"] = serde_json::json!(error);
            empty
        }
    }
}

fn url_decode(input: &str) -> String {
    let mut bytes = Vec::with_capacity(input.len());
    let mut iter = input.bytes();
    while let Some(b) = iter.next() {
        if b == b'%' {
            let hex1 = iter.next();
            let hex2 = iter.next();
            if let (Some(h1), Some(h2)) = (hex1, hex2) {
                if let Ok(val) = u8::from_str_radix(&format!("{}{}", h1 as char, h2 as char), 16) {
                    bytes.push(val);
                    continue;
                }
            }
        } else if b == b'+' {
            bytes.push(b' ');
        } else {
            bytes.push(b);
        }
    }
    String::from_utf8_lossy(&bytes).into_owned()
}

fn url_encode(input: &str) -> String {
    let mut out = String::new();
    for b in input.bytes() {
        match b {
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char);
            }
            b' ' => out.push('+'),
            _ => {
                out.push_str(&format!("%{:02X}", b));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    #[test]
    fn audio_delivery_continues_when_spectrum_worker_is_backlogged() {
        let state = RwLock::new(HubState {
            spotify_audio_error: None,
            spectrum: spectrum::Snapshot::default(),
            snapshot: Snapshot::default(),
            engine: None,
        });
        let active = AtomicBool::new(true);
        let (tx, _rx) = std::sync::mpsc::sync_channel(1);
        let listener = broadcaster().subscribe();
        publish_pcm(&state, &active, &tx, 0, &[1, 2, 3, 4], Provider::Deezer);
        publish_pcm(&state, &active, &tx, 0, &[5, 6, 7, 8], Provider::Deezer);
        assert_eq!(listener.try_recv().unwrap().as_slice(), &[1, 2, 3, 4]);
        assert_eq!(listener.try_recv().unwrap().as_slice(), &[5, 6, 7, 8]);
    }

    #[test]
    fn pcm_subscriptions_release_idle_capture_demand_and_disconnect_slow_clients() {
        use super::*;
        let broadcaster = AudioBroadcaster {
            clients: Mutex::new(Vec::new()),
        };
        let listener = broadcaster.subscribe();
        assert!(broadcaster.has_listeners());
        drop(listener);
        assert!(!broadcaster.has_listeners());
        let listener = broadcaster.subscribe();
        for sample in 0..128u8 {
            broadcaster.publish(Arc::new(vec![sample; 4]));
        }
        broadcaster.publish(Arc::new(vec![255; 4]));
        assert!(!broadcaster.has_listeners());
        for sample in 0..128u8 {
            assert_eq!(&**listener.try_recv().unwrap(), &[sample; 4]);
        }
        assert!(matches!(
            listener.try_recv(),
            Err(std::sync::mpsc::TryRecvError::Disconnected)
        ));
    }
    #[test]
    fn host_payloads_reject_cross_provider_ids_and_stale_collections() {
        use super::*;
        use serde_json::json;
        assert!(valid_host_payload(
            Provider::Spotify,
            &json!({"provider":"spotify","id":"spotify:track:abc"})
        ));
        assert!(!valid_host_payload(
            Provider::Spotify,
            &json!({"provider":"spotify","id":"123"})
        ));
        assert!(valid_host_payload(
            Provider::Spotify,
            &json!({"id":"spotify:track:abc"})
        ));
        assert!(!valid_host_payload(
            Provider::Deezer,
            &json!({"provider":"spotify"})
        ));
        assert!(!valid_host_payload(
            Provider::Deezer,
            &json!({"id":"spotify:track:abc"})
        ));
        assert!(!valid_host_payload(
            Provider::Spotify,
            &json!({"provider":"spotify","tracks":[{"id":"spotify:track:abc"},{"id":"123"}]})
        ));
        assert!(valid_host_payload(
            Provider::Deezer,
            &json!({"tracks":[{"id":"123"}]})
        ));
    }
    #[test]
    fn spotify_guest_routes_never_enter_deezer_fallbacks() {
        use super::*;
        let host = Snapshot {
            provider: Provider::Spotify,
            ..Default::default()
        };
        assert!(spotify_guest_request("GET", "/api/stream", "", &host).is_none());
        for (method, route, query) in [
            ("GET", "/api/flow", ""),
            ("POST", "/api/flow/start", ""),
            ("GET", "/api/style-mix", ""),
            ("GET", "/api/collection", "id=123&kind=album"),
        ] {
            assert!(spotify_guest_request(method, route, query, &host)
                .unwrap()
                .is_err());
        }
        let empty = spotify_guest_request("GET", "/api/search", "q=", &host)
            .unwrap()
            .unwrap();
        assert!(empty["tracks"].as_array().unwrap().is_empty());
    }
    #[test]
    fn queue_attribution_is_bounded_and_removes_control_characters() {
        let entry = super::attribute_entry(
            Default::default(),
            &serde_json::json!({"added_by":" Alice\n "}),
        );
        assert_eq!(entry.raw["added_by"], "Alice");
        let entry = super::attribute_entry(
            Default::default(),
            &serde_json::json!({"added_by":"x".repeat(100)}),
        );
        assert_eq!(entry.raw["added_by"].as_str().unwrap().len(), 64);
    }
    #[test]
    fn style_mix_options_and_errors_are_separate_from_flow() {
        let (tx, _rx) = std::sync::mpsc::sync_channel(4);
        let hub = ConnectHub::start(0, tx).unwrap();
        for (path, expected) in [
            ("/api/style-mix", "Japanese"),
            ("/api/style-mix/genres?region=1", "genres"),
            (
                "/api/style-mix/genres?region=invalid",
                "Choose a valid language",
            ),
            (
                "/api/style-mix?selection=0%3A0",
                "Connect Deezer on the host first",
            ),
            (
                "/api/style-mix?selection=999%3A0",
                "Choose a valid country and genre",
            ),
        ] {
            let mut client = TcpStream::connect(("127.0.0.1", hub.port())).unwrap();
            client
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            client
                .write_all(format!("GET {path} HTTP/1.1\r\nHost: localhost\r\n\r\n").as_bytes())
                .unwrap();
            let mut response = String::new();
            client.read_to_string(&mut response).unwrap();
            assert!(response.contains(expected), "{response}");
        }
        let entry = attribute_entry(
            Entry {
                raw: serde_json::json!({}),
                ..Default::default()
            },
            &serde_json::json!({"style_mix":"1:1", "added_by":"Guest"}),
        );
        assert_eq!(entry.raw[crate::style_mix::MARKER], "1:1");
        assert_eq!(entry.raw["added_by"], "Guest");

        // Verify genre loading across Deezer and Spotify host providers
        for provider in [Provider::Deezer, Provider::Spotify] {
            hub.update(&Snapshot {
                provider,
                ..Default::default()
            });
            let mut client = TcpStream::connect(("127.0.0.1", hub.port())).unwrap();
            client
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            client
                .write_all(
                    b"GET /api/style-mix/genres?region=0 HTTP/1.1\r\nHost: localhost\r\n\r\n",
                )
                .unwrap();
            let mut response = String::new();
            client.read_to_string(&mut response).unwrap();
            assert!(
                response.starts_with("HTTP/1.1 200"),
                "Provider {:?} failed: {}",
                provider,
                response
            );
            let body = response.split_once("\r\n\r\n").unwrap().1;
            let data: serde_json::Value = serde_json::from_str(body).unwrap();
            let genres = data["genres"].as_array().expect("genres array");
            assert!(
                !genres.is_empty(),
                "Genres must not be empty for {:?}",
                provider
            );
            assert_eq!(genres[0]["title"], "Pop");
        }
    }

    #[test]
    fn phone_stream_buffer_is_bounded_and_native_default_is_unchanged() {
        assert_eq!(stream_buffer_bytes(""), 88200);
        assert_eq!(stream_buffer_bytes("buffer_ms=1200"), 211680);
        assert_eq!(stream_buffer_bytes("buffer_ms=1500"), 264600);
        assert_eq!(stream_buffer_bytes("buffer_ms=999999"), 352800);
        assert_eq!(stream_buffer_bytes("buffer_ms=1"), 44100);
        assert_eq!(stream_buffer_bytes("buffer_ms=invalid"), 88200);
    }
    use super::*;
    #[test]
    fn named_queue_additions_are_shared_with_other_clients() {
        let (tx, rx) = std::sync::mpsc::sync_channel(4);
        let hub = ConnectHub::start(0, tx).unwrap();
        let mut queued = Vec::new();
        for nickname in ["Alice", "Bob"] {
            let body =
                serde_json::json!({"id":"1","title":"Same song","added_by":nickname}).to_string();
            let mut client = TcpStream::connect(("127.0.0.1", hub.port())).unwrap();
            client
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            client
                .write_all(
                    format!(
                        "POST /api/queue/add-last HTTP/1.1\r\nContent-Length: {}\r\n\r\n{body}",
                        body.len()
                    )
                    .as_bytes(),
                )
                .unwrap();
            let mut response = String::new();
            client.read_to_string(&mut response).unwrap();
            assert!(response.starts_with("HTTP/1.1 200"));
            match rx.recv_timeout(Duration::from_secs(2)).unwrap() {
                Command::QueueAddLast(entry) => queued.push(*entry),
                _ => panic!("Expected a queue addition"),
            }
        }
        hub.update(&Snapshot {
            queue: Arc::new(queued),
            ..Snapshot::default()
        });
        let mut client = TcpStream::connect(("127.0.0.1", hub.port())).unwrap();
        client
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        client
            .write_all(b"GET /api/state HTTP/1.1\r\n\r\n")
            .unwrap();
        let mut response = String::new();
        client.read_to_string(&mut response).unwrap();
        let state: serde_json::Value =
            serde_json::from_str(response.split_once("\r\n\r\n").unwrap().1).unwrap();
        assert_eq!(state["queue"][0]["added_by"], "Alice");
        assert_eq!(state["queue"][1]["added_by"], "Bob");
    }

    #[test]
    fn browser_shuffle_and_repeat_dispatch_explicit_modes() {
        let (tx, rx) = std::sync::mpsc::sync_channel(4);
        let hub = ConnectHub::start(0, tx).unwrap();
        hub.update(&Snapshot {
            connected: true,
            ..Snapshot::default()
        });
        for (path, body, ok) in [
            ("shuffle", r#"{"enabled":true}"#, true),
            ("repeat", r#"{"mode":2}"#, true),
            ("repeat", r#"{"mode":3}"#, false),
            ("autoplay", r#"{"enabled":true}"#, true),
            ("autoplay", r#"{"enabled":false}"#, true),
            ("autoplay", r#"{"enabled":"true"}"#, false),
        ] {
            let mut client = TcpStream::connect(("127.0.0.1", hub.port())).unwrap();
            client
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            client
                .write_all(
                    format!(
                        "POST /api/{path} HTTP/1.1\r\nContent-Length: {}\r\n\r\n{body}",
                        body.len()
                    )
                    .as_bytes(),
                )
                .unwrap();
            let mut response = String::new();
            client.read_to_string(&mut response).unwrap();
            assert_eq!(response.starts_with("HTTP/1.1 200"), ok);
        }
        assert!(matches!(rx.try_recv().unwrap(), Command::Shuffle(true)));
        assert!(matches!(rx.try_recv().unwrap(), Command::Repeat(2)));
        assert!(matches!(rx.try_recv().unwrap(), Command::Autoplay(true)));
        assert!(matches!(rx.try_recv().unwrap(), Command::Autoplay(false)));
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn browser_assets_and_appearance_are_served_by_the_jam_host() {
        let (tx, _rx) = std::sync::mpsc::sync_channel(4);
        let hub = ConnectHub::start(0, tx).unwrap();
        for (path, mime, expected) in [
            ("/ui.js", "text/javascript", "JamUI"),
            ("/ui.css", "text/css", "DeezNatsIcons"),
            ("/translations.json", "application/json", "Appearance"),
            ("/api/appearance", "application/json", "Blood Moon"),
        ] {
            let mut client = TcpStream::connect(("127.0.0.1", hub.port())).unwrap();
            client
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            client
                .write_all(format!("GET {path} HTTP/1.1\r\nHost: localhost\r\n\r\n").as_bytes())
                .unwrap();
            let mut response = String::new();
            client.read_to_string(&mut response).unwrap();
            assert!(response.starts_with("HTTP/1.1 200"), "{path}");
            assert!(response.contains(mime), "{path}");
            assert!(response.contains(expected), "{path}");
        }
    }

    #[test]
    fn browser_appearance_exposes_only_themes_opacity_and_blur() {
        let appearance = browser_appearance();
        assert_eq!(
            appearance["themes"].as_array().unwrap().len(),
            crate::ui::SOLID_THEMES.len()
        );
        assert_eq!(appearance["defaults"].as_object().unwrap().len(), 3);
        assert_eq!(
            appearance["defaults"],
            serde_json::json!({"theme":8,"opacity":40,"blur":20})
        );
        assert!(appearance.get("palettes").is_none());
        assert_eq!(appearance["themes"][11]["name"], "Blood Moon");
        assert_eq!(
            appearance["themes"][11]["palette"]["Background"],
            browser_hex(crate::ui::SOLID_THEMES[11].2)
        );
        assert_eq!(browser_hex(0x00332211), "#112233");
        let translations: serde_json::Value =
            serde_json::from_str(include_str!("connect/translations.json")).unwrap();
        assert!(translations["Appearance"].is_string());
        assert!(translations["Loading lyrics..."].is_string());
    }

    #[test]
    fn flow_accepts_one_unchanged_genre_when_other_genres_are_disabled() {
        let selection = serde_json::json!({"genres":[{"id":"rock","isEnabled":true},{"id":"pop","isEnabled":false}],
            "edits":[{"clusterId":"pop","isEnabled":false}]});
        assert!(!flow_selection_is_empty(&selection));
        assert!(flow_selection_is_empty(
            &serde_json::json!({"genres":[{"id":"rock","isEnabled":false}]})
        ));
        assert!(!flow_selection_is_empty(
            &serde_json::json!({"edits":[{"clusterId":"pop","isEnabled":false}]})
        ));
    }

    #[test]
    fn jam_durations_preserve_milliseconds_through_search_and_queue() {
        let results = search_items(
            &serde_json::json!([{"id":"1","title":"Levels","durationMs":199000,"kind":"track"}]),
            "track",
        );
        assert_eq!(results[0]["duration_ms"], 199000);
        assert_eq!(results[0]["duration"], 199);
        let queued = ConnectTrack::from_value(&results[0]).unwrap();
        assert_eq!(queued.to_entry().duration, 199000);
        assert_eq!(queued.to_json()["duration_ms"], 199000);
        let legacy = ConnectTrack::from_value(
            &serde_json::json!({"id":"1","title":"Levels","duration":199}),
        )
        .unwrap();
        assert_eq!(legacy.to_entry().duration, 199000);
    }

    #[test]
    fn search_preserves_categories_and_requires_host_session() {
        let raw = serde_json::json!({
            "tracks":[{"id":"1","title":"Song","kind":"track"}],
            "albums":[{"id":"2","title":"Album","kind":"album"}],
            "artists":[{"id":"3","name":"Artist","kind":"artist"}],
            "playlists":[{"id":"4","title":"Playlist","kind":"playlist"}]
        });
        let results = normalize_search(&raw);
        for key in ["tracks", "albums", "artists", "playlists"] {
            assert_eq!(results[key].as_array().unwrap().len(), 1, "{key}");
        }
        assert_eq!(results["albums"][0]["kind"], "album");
        assert!(search_deezer(None, "Song")["error"].is_string());
        assert_eq!(
            get_query_param("q=AC%2BDC%20%2525", "q").unwrap(),
            "AC+DC %25"
        );
        let episode = ConnectTrack::from_value(
            &serde_json::json!({"id":"8","title":"Episode","kind":"episode"}),
        )
        .unwrap();
        assert_eq!(episode.to_entry().kind, "episode");
    }

    #[test]
    fn state_api_normalizes_engine_playback_state() {
        let (tx, _rx) = std::sync::mpsc::sync_channel(4);
        let hub = ConnectHub::start(0, tx).unwrap();
        let snapshot = Snapshot {
            state: "Playing".into(),
            ..Snapshot::default()
        };
        hub.update(&snapshot);
        let mut client = TcpStream::connect(("127.0.0.1", hub.port())).unwrap();
        client
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        client
            .write_all(b"GET /api/state HTTP/1.1\r\nHost: localhost\r\n\r\n")
            .unwrap();
        let mut response = String::new();
        client.read_to_string(&mut response).unwrap();
        let body = response.split_once("\r\n\r\n").unwrap().1;
        let state: serde_json::Value = serde_json::from_str(body).unwrap();
        assert_eq!(state["state"], "playing");
    }

    #[test]
    fn fragmented_request_and_shutdown() {
        let (tx, rx) = std::sync::mpsc::sync_channel(4);
        let hub = ConnectHub::start(0, tx).unwrap();
        let port = hub.port();
        let mut client = TcpStream::connect(("127.0.0.1", port)).unwrap();
        client
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let body = br#"{"position_ms":42}"#;
        let request = format!(
            "POST /api/seek HTTP/1.1\r\nContent-Length: {}\r\n\r\n",
            body.len()
        );
        for byte in request.bytes().chain(body.iter().copied()) {
            client.write_all(&[byte]).unwrap();
        }
        let mut response = String::new();
        client.read_to_string(&mut response).unwrap();
        assert!(response.starts_with("HTTP/1.1 200"));
        assert!(matches!(
            rx.recv_timeout(Duration::from_secs(2)).unwrap(),
            Command::SeekMs(42)
        ));
        drop(hub);
        let rebound = TcpListener::bind(("127.0.0.1", port)).unwrap();
        drop(rebound);
    }

    #[test]
    fn test_fetch_flow_structure() {
        let flow = fetch_flow(None, "default");
        assert!(flow
            .get("options")
            .and_then(serde_json::Value::as_array)
            .is_some());
        assert!(flow
            .get("customizable")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false));
    }

    #[test]
    fn test_flow_tuner_endpoints() {
        let (tx, _rx) = std::sync::mpsc::sync_channel(32);
        let hub = ConnectHub::start(3195, tx).expect("failed to start test hub");
        let port = hub.port();

        // Test GET /api/flow/tuner
        let url = format!("http://127.0.0.1:{port}/api/flow/tuner?id=default");
        if let Ok(bytes) = crate::lyrics::http::request("GET", &url, &[], &[]) {
            let val: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(val["id"], "default");
            assert!(val["genres"].as_array().unwrap().len() > 0);
        }

        // Test POST /api/flow/tuner/save empty validation check
        let save_url = format!("http://127.0.0.1:{port}/api/flow/tuner/save");
        let empty_payload = serde_json::json!({
            "flow_id": "default",
            "genres": [
                { "id": "132", "isEnabled": false }
            ]
        });
        let body = serde_json::to_vec(&empty_payload).unwrap();
        if let Ok(bytes) = crate::lyrics::http::request("POST", &save_url, &[], &body) {
            let val: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
            assert!(val.get("error").is_some());
        }

        // Test POST /api/flow/tuner/save valid save
        let valid_payload = serde_json::json!({
            "flow_id": "default",
            "genres": [
                { "id": "132", "isEnabled": true }
            ]
        });
        let body = serde_json::to_vec(&valid_payload).unwrap();
        if let Ok(bytes) = crate::lyrics::http::request("POST", &save_url, &[], &body) {
            let val: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(val["ok"], false);
            assert!(val["error"].as_str().unwrap().contains("Connect Deezer"));
        }
    }
}
