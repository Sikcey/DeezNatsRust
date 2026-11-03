//! Native audio playback using the ytmusic resolver and ranged stream loader.
use crate::audio::{Command, Snapshot};
use crate::catalog::Entry;
use rodio::{OutputStream, Sink};
use std::{io::Cursor, sync::mpsc, time::Duration};
type Loaded = Result<(ytmusic::AudioFormat, Vec<u8>, Option<Entry>), String>;

fn matching_song(video: &Entry, candidates: Vec<ytmusic::Track>) -> Option<Entry> {
    fn normalized(value: &str) -> String {
        value
            .chars()
            .filter(|c| c.is_alphanumeric())
            .flat_map(char::to_lowercase)
            .collect()
    }
    let title = video
        .title
        .split(" (Official")
        .next()
        .unwrap_or(&video.title);
    let title = normalized(title);
    let artist = normalized(video.subtitle.split(", ").next().unwrap_or(&video.subtitle));
    if title.is_empty() || artist.is_empty() {
        return None;
    }
    candidates
        .into_iter()
        .filter(|t| {
            t.available && !t.is_video() && t.video_id.as_ref().is_some_and(|id| !id.is_empty())
        })
        .find(|t| {
            normalized(&t.title) == title && t.artists.iter().any(|a| normalized(&a.name) == artist)
        })
        .map(super::catalog::track)
}
pub(crate) struct Player {
    eq: std::sync::Arc<std::sync::Mutex<super::routing::Eq>>,
    output: Option<OutputStream>,
    sink: Option<Sink>,
    audio_bytes: Option<std::sync::Arc<[u8]>>,
    endpoint: Option<String>,
    endpoint_checked: std::time::Instant,
    pending: Option<mpsc::Receiver<Loaded>>,
    load_cancel: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
    volume: f32,
    paused: bool,
    auth: Option<(
        std::sync::Arc<std::sync::atomic::AtomicBool>,
        mpsc::Receiver<Result<serde_json::Value, String>>,
    )>,
    radio: Option<mpsc::Receiver<Result<Vec<Entry>, String>>>,
    last_seed: String,
}
impl Default for Player {
    fn default() -> Self {
        Self {
            eq: Default::default(),
            output: None,
            sink: None,
            audio_bytes: None,
            endpoint: None,
            endpoint_checked: std::time::Instant::now() - Duration::from_secs(2),
            pending: None,
            load_cancel: None,
            volume: 1.0,
            paused: false,
            auth: None,
            radio: None,
            last_seed: String::new(),
        }
    }
}
impl Player {
    pub(crate) fn stop(&mut self) {
        super::routing::reset();
        self.pending = None;
        if let Some(cancel) = self.load_cancel.take() {
            cancel.store(true, std::sync::atomic::Ordering::Relaxed);
        }
        if let Some(s) = self.sink.take() {
            s.stop();
        }
    }
    pub(crate) fn suspend(&mut self) {
        self.stop();
        self.radio = None;
        self.last_seed.clear();
        if let Some((cancel, _)) = self.auth.take() {
            cancel.store(true, std::sync::atomic::Ordering::Relaxed);
        }
    }
    fn auth(&mut self, value: Option<String>, snapshot: &mut Snapshot) {
        if self.auth.is_some() {
            if value.is_none() {
                return;
            }
            if let Some((cancel, _)) = self.auth.take() {
                cancel.store(true, std::sync::atomic::Ordering::Relaxed);
            }
        }
        let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let flag = cancel.clone();
        let (tx, rx) = mpsc::sync_channel(1);
        self.auth = Some((cancel, rx));
        snapshot.message = "Connecting to YouTube Music...".into();
        std::thread::spawn(move || {
            let result = match value {
                Some(s) if s.is_empty() => super::login(&flag),
                Some(s) => super::login_cookies_with_cancel(&s, &flag),
                None => super::account(),
            };
            let _ = tx.send(result);
        });
    }
    fn start(&mut self, entry: Entry, snapshot: &mut Snapshot) {
        self.stop();
        self.paused = false;
        snapshot.now = entry.clone();
        snapshot.title = entry.title.clone();
        snapshot.duration = entry.duration;
        snapshot.position = 0;
        snapshot.state = "Loading".into();
        snapshot.message.clear();
        let (tx, rx) = mpsc::sync_channel(1);
        self.pending = Some(rx);
        let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        self.load_cancel = Some(cancel.clone());
        std::thread::spawn(move || {
            let result = super::api().and_then(|api| {
                super::runtime()
                    .block_on(async {
                        let load = async {
                            let song = if entry.raw.pointer("/youtube/kind").and_then(|v| v.as_str()) == Some("Video") {
                                let title = entry.title.split(" (Official").next().unwrap_or(&entry.title);
                                let query = format!("{} {}", title, entry.subtitle);
                                match tokio::time::timeout(Duration::from_secs(5), api.search_songs(&query)).await {
                                    Ok(Ok(candidates)) => matching_song(&entry, candidates),
                                    _ => None,
                                }
                            } else { None };
                            let mut song = song;
                            let format = if let Some(selected) = song.as_ref() {
                                match api.best_audio(super::catalog::bare(&selected.id)).await {
                                    Ok(format) => format,
                                    Err(_) => { song = None; api.best_audio(super::catalog::bare(&entry.id)).await? }
                                }
                            } else { api.best_audio(super::catalog::bare(&entry.id)).await? };
                            anyhow::ensure!(format.content_length.is_some_and(|n| n <= 128 * 1024 * 1024),
                                "YouTube track has an unknown size or exceeds the 128 MiB playback buffer");
                            if cancel.load(std::sync::atomic::Ordering::Relaxed) {
                                anyhow::bail!("YouTube playback cancelled");
                            }
                            let bytes = api.download(&format).await?;
                            anyhow::ensure!(bytes.len() <= 128 * 1024 * 1024,
                                "YouTube track exceeds the 128 MiB playback buffer");
                            Ok::<_, anyhow::Error>((format, bytes, song))
                        };
                        tokio::time::timeout(Duration::from_secs(60), async {
                            tokio::select! {
                                result = load => result,
                                _ = async {
                                    while !cancel.load(std::sync::atomic::Ordering::Relaxed) {
                                        tokio::time::sleep(Duration::from_millis(100)).await;
                                    }
                                } => Err(anyhow::anyhow!("YouTube playback cancelled")),
                            }
                        }).await
                    })
                    .map_err(|_| "YouTube audio load timed out".to_string())?
                    .map_err(|e| e.to_string())
            });
            let _ = tx.send(result);
        });
    }
    fn advance(&mut self, snapshot: &mut Snapshot, backwards: bool) {
        if snapshot.queue.is_empty() {
            self.stop();
            snapshot.state = "Stopped".into();
            return;
        }
        let len = snapshot.queue.len();
        let index = if backwards {
            snapshot.queue_index.saturating_sub(1)
        } else if snapshot.repeat == 2 {
            snapshot.queue_index
        } else if snapshot.shuffle {
            (snapshot.queue_index
                + 1
                + (std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .subsec_nanos() as usize
                    % len.saturating_sub(1).max(1)))
                % len
        } else {
            snapshot.queue_index + 1
        };
        if index >= len {
            if snapshot.repeat == 1 {
                snapshot.queue_index = 0;
            } else {
                self.stop();
                snapshot.state = "Stopped".into();
                return;
            }
        } else {
            snapshot.queue_index = index;
        }
        self.start(snapshot.queue[snapshot.queue_index].clone(), snapshot);
    }
    pub(crate) fn handle(&mut self, command: &Command, snapshot: &mut Snapshot) {
        match command {
            Command::Login(value) => self.auth(Some(value.clone()), snapshot),
            Command::LoginSaved => self.auth(None, snapshot),
            Command::Logout => {
                self.suspend();
                if let Err(e) = super::logout() {
                    snapshot.message = e;
                }
                snapshot.connected = false;
                snapshot.account = serde_json::Value::Null;
                snapshot.now = Default::default();
                snapshot.queue = Default::default();
                snapshot.queue_index = 0;
                snapshot.title.clear();
                snapshot.position = 0;
                snapshot.duration = 0;
                snapshot.radio_active = false;
                snapshot.state = "Stopped".into();
            }
            Command::PlayEntry(e, queue) => {
                self.radio = None;
                self.last_seed.clear();
                snapshot.radio_active = e.raw["_youtube_radio"] == true;
                let mut q = queue.clone();
                if !q.iter().any(|t| t.id == e.id) {
                    q.insert(0, (**e).clone());
                }
                snapshot.queue_index = q.iter().position(|t| t.id == e.id).unwrap_or(0);
                snapshot.queue = q.into();
                self.start((**e).clone(), snapshot);
            }
            Command::Play(id) => {
                if let Some(i) = snapshot.queue.iter().position(|t| t.id == *id) {
                    snapshot.queue_index = i;
                    self.start(snapshot.queue[i].clone(), snapshot);
                }
            }
            Command::PlayQueueIndex(i) if *i < snapshot.queue.len() => {
                snapshot.queue_index = *i;
                self.start(snapshot.queue[*i].clone(), snapshot);
            }
            Command::Next => self.advance(snapshot, false),
            Command::Previous => self.advance(snapshot, true),
            Command::Toggle => {
                self.paused = !self.paused;
                if let Some(s) = &self.sink {
                    if self.paused {
                        s.pause()
                    } else {
                        s.play()
                    }
                }
                snapshot.state = if self.paused { "Paused" } else { "Playing" }.into();
            }
            Command::SetPlaying(p) => {
                self.paused = !*p;
                if let Some(s) = &self.sink {
                    if *p {
                        s.play()
                    } else {
                        s.pause()
                    }
                }
                snapshot.state = if *p { "Playing" } else { "Paused" }.into();
            }
            Command::Stop => {
                self.stop();
                snapshot.state = "Stopped".into();
                snapshot.position = 0;
            }
            Command::Seek(p) => self.seek(*p as u64 * 1000, snapshot),
            Command::SeekMs(p) => self.seek(*p, snapshot),
            Command::Volume(v) => {
                self.volume = (*v).min(100) as f32 / 100.;
                if let Some(s) = &self.sink {
                    s.set_volume(self.volume);
                }
            }
            Command::Repeat(v) => snapshot.repeat = *v,
            Command::Shuffle(v) => snapshot.shuffle = *v,
            Command::Autoplay(v) => snapshot.autoplay = *v,
            Command::QueueClear => {
                self.stop();
                self.radio = None;
                self.last_seed.clear();
                snapshot.queue = Default::default();
                snapshot.queue_index = 0;
                snapshot.state = "Stopped".into();
            }
            Command::QueueInsertNext(t) => {
                let index = (snapshot.queue_index + 1).min(snapshot.queue.len());
                ArcQueue::insert(snapshot, index, (**t).clone());
            }
            Command::QueueAddLast(t) => {
                let index = snapshot.queue.len();
                ArcQueue::insert(snapshot, index, (**t).clone());
            }
            Command::QueueMove { from, to, expected } => {
                match crate::audio::reordered_queue(
                    &snapshot.queue,
                    snapshot.queue_index,
                    *from,
                    *to,
                    expected,
                ) {
                    Ok(queue) => {
                        snapshot.queue = std::sync::Arc::new(queue);
                        snapshot.message = "Queue reordered".into();
                    }
                    Err(error) => snapshot.message = error,
                }
            }
            Command::Api(name, args) => {
                if name == "DZSetEQJSON" {
                    let result = crate::engine::normalize_eq(args);
                    match result {
                        Ok(value) => {
                            if let Ok(mut eq) = self.eq.lock() {
                                eq.update(&value);
                            }
                        }
                        Err(error) => snapshot.message = error.into(),
                    }
                    return;
                }
                if let Err(e) = super::invoke(name, args) {
                    snapshot.message = e;
                }
            }
            _ => {}
        }
    }
    fn seek(&mut self, ms: u64, snapshot: &mut Snapshot) {
        if let Some(s) = &self.sink {
            match s.try_seek(Duration::from_millis(ms)) {
                Ok(()) => snapshot.position = ms,
                Err(e) => snapshot.message = format!("YouTube seek: {e}"),
            }
        }
    }
    fn reopen_output(
        &mut self,
        bytes: std::sync::Arc<[u8]>,
        position: Duration,
    ) -> Result<(), String> {
        let output =
            rodio::OutputStreamBuilder::open_default_stream().map_err(|e| e.to_string())?;
        let sink = Sink::connect_new(output.mixer());
        let source = rodio::Decoder::try_from(Cursor::new(bytes)).map_err(|e| e.to_string())?;
        sink.set_volume(self.volume);
        sink.pause();
        let source = rodio::source::UniformSourceIterator::new(source, 2, 44_100);
        sink.append(super::routing::Routed::new(source, self.eq.clone()));
        if !position.is_zero() {
            sink.try_seek(position).map_err(|e| e.to_string())?;
        }
        if !self.paused {
            sink.play();
        }
        if let Some(old) = self.sink.take() {
            old.stop();
        }
        self.output = Some(output);
        self.sink = Some(sink);
        Ok(())
    }
    pub(crate) fn poll(&mut self, snapshot: &mut Snapshot) {
        if self.endpoint_checked.elapsed() >= Duration::from_secs(1) {
            self.endpoint_checked = std::time::Instant::now();
            if let Ok(endpoint) = crate::audio::windows_default_output_id() {
                if self.endpoint.as_ref() != Some(&endpoint) {
                    if let (Some(bytes), Some(sink)) = (&self.audio_bytes, &self.sink) {
                        let position = sink.get_pos();
                        match self.reopen_output(bytes.clone(), position) {
                            Ok(()) => self.endpoint = Some(endpoint),
                            Err(e) => snapshot.message = format!("YouTube output: {e}"),
                        }
                    } else {
                        self.output = None;
                        self.endpoint = Some(endpoint);
                    }
                }
            }
        }
        if let Some(result) = self.auth.as_ref().and_then(|(_, rx)| rx.try_recv().ok()) {
            self.auth = None;
            match result {
                Ok(account) => {
                    snapshot.account = account;
                    snapshot.connected = true;
                    snapshot.message.clear();
                }
                Err(e) => snapshot.message = e,
            }
        }
        if let Some(result) = self.radio.as_ref().and_then(|rx| rx.try_recv().ok()) {
            self.radio = None;
            match result {
                Ok(rows) => {
                    let known = snapshot
                        .queue
                        .iter()
                        .map(|t| t.id.clone())
                        .collect::<std::collections::HashSet<_>>();
                    std::sync::Arc::make_mut(&mut snapshot.queue)
                        .extend(rows.into_iter().filter(|t| !known.contains(&t.id)));
                }
                Err(e) => snapshot.message = format!("YouTube radio: {e}"),
            }
        }
        if (snapshot.radio_active || snapshot.autoplay)
            && snapshot.queue_index.saturating_add(3) >= snapshot.queue.len()
            && !snapshot.queue.is_empty()
            && self.radio.is_none()
            && (self.sink.is_some() || self.pending.is_some())
        {
            let tail = snapshot.queue.last().unwrap();
            let flow = tail.raw["_youtube_flow"].as_str().map(str::to_owned);
            let offset = tail.raw["_youtube_flow_offset"].as_u64().unwrap_or(0) as usize;
            let seed = tail.id.clone();
            let known = snapshot.queue.iter().map(|t| t.id.clone()).collect();
            if seed != self.last_seed {
                self.last_seed = seed.clone();
                let (tx, rx) = mpsc::sync_channel(1);
                self.radio = Some(rx);
                std::thread::spawn(move || {
                    let result = super::api().and_then(|api| {
                        super::runtime()
                            .block_on(async {
                                if let Some(key) = flow {
                                    super::catalog::continue_flow(&api, &key, offset, known).await
                                } else {
                                    super::catalog::station(&api, super::catalog::bare(&seed), None)
                                        .await
                                }
                            })
                            .map(|(rows, _)| rows)
                            .map_err(|e| e.to_string())
                    });
                    let _ = tx.send(result);
                });
            }
        }
        if let Some(result) = self.pending.as_ref().and_then(|rx| rx.try_recv().ok()) {
            self.pending = None;
            let result = result.and_then(|(format, bytes, song)| {
                let bytes: std::sync::Arc<[u8]> = bytes.into();
                self.reopen_output(bytes.clone(), Duration::ZERO)?;
                self.audio_bytes = Some(bytes);
                if let Some(song) = song {
                    snapshot.title = song.title.clone();
                    snapshot.now = song;
                }
                snapshot.duration = format
                    .duration
                    .map(|d| d.as_millis() as u64)
                    .unwrap_or(snapshot.duration);
                snapshot.quality = format!("{} kbps {}", format.bitrate / 1000, format.codec);
                snapshot.state = if self.paused { "Paused" } else { "Playing" }.into();
                snapshot.playback_connected = true;
                super::remember(&snapshot.now);
                Ok(())
            });
            if let Err(e) = result {
                snapshot.message = format!("YouTube playback: {e}");
                snapshot.state = "Stopped".into();
                snapshot.playback_connected = false;
            }
        }
        if let Some(s) = &self.sink {
            snapshot.position = s.get_pos().as_millis() as u64;
            if s.empty()
                && !self.paused
                && (self.radio.is_none() || snapshot.queue_index + 1 < snapshot.queue.len())
            {
                self.advance(snapshot, false);
            }
        }
    }
}
struct ArcQueue;
impl ArcQueue {
    fn insert(s: &mut Snapshot, index: usize, t: Entry) {
        std::sync::Arc::make_mut(&mut s.queue).insert(index, t);
    }
}

impl Drop for Player {
    fn drop(&mut self) {
        self.suspend();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn video_matching_requires_same_song_and_artist() {
        let video = Entry {
            title: "KGB (Official Music Video)".into(),
            subtitle: "ADÉLA".into(),
            ..Default::default()
        };
        let candidate = |title: &str, artist: &str, kind| ytmusic::Track {
            video_id: Some("id".into()),
            title: title.into(),
            available: true,
            kind,
            artists: vec![ytmusic::ArtistRef {
                name: artist.into(),
                id: None,
            }],
            ..Default::default()
        };
        assert!(matching_song(
            &video,
            vec![
                candidate("KGB", "Other", ytmusic::TrackKind::Song),
                candidate("KGB", "ADÉLA", ytmusic::TrackKind::Video)
            ]
        )
        .is_none());
        assert!(matching_song(
            &video,
            vec![candidate("KGB", "ADÉLA", ytmusic::TrackKind::Song)]
        )
        .is_some());
    }
    #[test]
    fn reorder_preserves_current_duplicate_and_rejects_stale_queue() {
        let mut player = Player::default();
        let mut snapshot = Snapshot::default();
        snapshot.queue = ["same", "same", "a", "b"]
            .into_iter()
            .map(|id| Entry {
                id: id.into(),
                ..Default::default()
            })
            .collect::<Vec<_>>()
            .into();
        snapshot.queue_index = 1;
        let expected = snapshot
            .queue
            .iter()
            .map(|entry| entry.id.clone())
            .collect();
        player.handle(
            &Command::QueueMove {
                from: 3,
                to: 2,
                expected,
            },
            &mut snapshot,
        );
        assert_eq!(
            snapshot
                .queue
                .iter()
                .map(|e| e.id.as_str())
                .collect::<Vec<_>>(),
            ["same", "same", "b", "a"]
        );
        assert_eq!(snapshot.queue_index, 1);
        player.handle(
            &Command::QueueMove {
                from: 2,
                to: 3,
                expected: vec!["stale".into()],
            },
            &mut snapshot,
        );
        assert!(snapshot.message.contains("changed"));
        assert_eq!(snapshot.queue[2].id, "b");
    }

    #[test]
    #[ignore = "requires a signed-in YouTube Music session and network access"]
    fn live_audio_load() {
        let api = super::super::api().expect("Sign in to YouTube Music first");
        super::super::runtime().block_on(async {
            let tracks = api
                .search_songs("Daft Punk Get Lucky")
                .await
                .expect("song search failed");
            let id = tracks
                .iter()
                .find_map(|track| track.video_id.as_deref())
                .expect("no playable song");
            let result = tokio::time::timeout(Duration::from_secs(60), api.load_audio(id))
                .await
                .expect("YouTube audio load timed out")
                .expect("YouTube audio load failed");
            let mut decoded =
                rodio::Decoder::try_from(Cursor::new(result.1)).expect("audio decode failed");
            assert!(decoded.next().is_some());
        });
    }
    #[test]
    #[ignore = "requires a signed-in YouTube Music session, network and an audio device"]
    fn live_player_loads_paused() {
        let api = super::super::api().expect("Sign in to YouTube Music first");
        let track = super::super::runtime()
            .block_on(api.search_songs("Daft Punk Get Lucky"))
            .expect("song search failed")
            .into_iter()
            .find(|track| track.video_id.is_some())
            .expect("no playable song");
        let mut player = Player::default();
        let mut snapshot = Snapshot::default();
        player.start(super::super::catalog::track(track), &mut snapshot);
        player.paused = true;
        let deadline = std::time::Instant::now() + Duration::from_secs(65);
        while player.pending.is_some() && std::time::Instant::now() < deadline {
            player.poll(&mut snapshot);
            std::thread::sleep(Duration::from_millis(50));
        }
        assert_eq!(snapshot.state, "Paused", "{}", snapshot.message);
        assert!(player.sink.is_some());
    }
    #[test]
    fn leaving_provider_cancels_login_and_discards_pending_audio() {
        let mut player = Player::default();
        let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let (_tx, rx) = mpsc::channel();
        player.auth = Some((cancel.clone(), rx));
        let (_audio, rx) = mpsc::channel();
        player.pending = Some(rx);
        player.suspend();
        assert!(cancel.load(std::sync::atomic::Ordering::Relaxed));
        assert!(player.auth.is_none());
        assert!(player.pending.is_none());
    }
    #[test]
    fn cleared_queue_does_not_receive_old_radio_results() {
        let mut player = Player::default();
        let (tx, rx) = mpsc::channel();
        player.radio = Some(rx);
        let mut snapshot = Snapshot::default();
        player.handle(&Command::QueueClear, &mut snapshot);
        assert!(tx
            .send(Ok(vec![Entry {
                id: "youtube:track:old".into(),
                ..Default::default()
            }]))
            .is_err());
        player.poll(&mut snapshot);
        assert!(snapshot.queue.is_empty());
    }
}
