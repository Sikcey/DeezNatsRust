//! Global Connect state/control. Browser readiness never blocks the audio worker.
use crate::audio::{Command, Snapshot};
use crate::catalog::Entry;
use serde_json::{json, Value};
use std::{
    sync::{
        atomic::{AtomicU64, Ordering},
        mpsc, Arc,
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};

type Reply = (u64, Result<Option<Value>, String>, Option<String>);
static SELECTED_DEVICE: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);
pub(crate) fn select_device(id: Option<String>) {
    if let Ok(mut selected) = SELECTED_DEVICE.lock() {
        *selected = id;
    }
}
pub(crate) fn selected_device() -> Option<String> {
    SELECTED_DEVICE.lock().ok()?.clone()
}
fn selected_target_matches(selected: Option<&str>, state: &Value) -> bool {
    selected.is_some_and(|id| state["device"]["id"].as_str() == Some(id))
}
pub(crate) struct Worker {
    epoch: Arc<AtomicU64>,
    enabled: bool,
    generation: u64,
    serial: u64,
    commands: mpsc::SyncSender<(u64, Command, Snapshot)>,
    updates: mpsc::Receiver<Reply>,
    state: Option<Value>,
}

fn current(epoch: u64, live: u64) -> bool {
    epoch != 0 && epoch == live
}

impl Worker {
    pub(crate) fn new() -> Self {
        let epoch = Arc::new(AtomicU64::new(0));
        let live = epoch.clone();
        let (commands, rx) = mpsc::sync_channel::<(u64, Command, Snapshot)>(8);
        let (tx, updates) = mpsc::channel();
        let (metadata_tx, metadata_rx) = mpsc::sync_channel::<(u64, String)>(1);
        let (entry_tx, entry_rx) = mpsc::channel();
        let metadata_live = epoch.clone();
        std::thread::spawn(move || {
            while let Ok((epoch, uri)) = metadata_rx.recv() {
                if !current(epoch, metadata_live.load(Ordering::SeqCst)) {
                    continue;
                }
                let kind = if uri.starts_with("spotify:episode:") {
                    "episode"
                } else {
                    "track"
                };
                let result = if kind == "track" {
                    super::catalog_invoke("DZTrackMetadataJSON", &[json!(uri)])
                } else {
                    super::catalog_invoke("SPCatalogItemJSON", &[json!(kind), json!(uri)])
                };
                if let Ok(value) = result {
                    if let Some(entry) = crate::catalog::entry(&value, kind).filter(|e| e.id == uri)
                    {
                        if current(epoch, metadata_live.load(Ordering::SeqCst)) {
                            let _ = entry_tx.send((epoch, uri, entry));
                        }
                    }
                }
            }
        });
        std::thread::spawn(move || {
            let mut queue = super::playback::Queue::default();
            let mut generation = 0;
            let mut requested_uri = String::new();
            let mut metadata: Option<Entry> = None;
            loop {
                let command = match rx.recv_timeout(Duration::from_secs(1)) {
                    Ok(c) => Some(c),
                    Err(mpsc::RecvTimeoutError::Timeout) => None,
                    Err(mpsc::RecvTimeoutError::Disconnected) => break,
                };
                let epoch = live.load(Ordering::SeqCst);
                if epoch == 0 {
                    continue;
                }
                if generation != epoch {
                    queue = Default::default();
                    metadata = None;
                    requested_uri.clear();
                    generation = epoch;
                }
                queue.bind_generation(live.clone(), epoch);
                if command.as_ref().is_some_and(|c| !current(c.0, epoch)) {
                    continue;
                }
                let mut error = None;
                let result = (|| -> Result<Option<Value>, String> {
                    let mut state = match super::web_devices::snapshot()? {
                        Some(state) => state,
                        None if command.as_ref().is_some_and(|c| {
                            matches!(c.1, Command::Play(_) | Command::PlayEntry(..))
                        }) =>
                        {
                            super::web_devices::playback_target()?
                        }
                        None => {
                            queue = Default::default();
                            return Ok(None);
                        }
                    };
                    if !selected_target_matches(selected_device().as_deref(), &state) {
                        return Err("The selected Spotify Connect device is no longer active; select it again or play on this computer".into());
                    }
                    let mut s = Snapshot::default();
                    apply(&mut s, Some(&state), now_ms());
                    queue.observe_remote(&mut s, &state);
                    s.now.raw["spotify_device_id"] = state["device"]["id"].clone();
                    if let Some((_, cmd, expected)) = command {
                        if !current(epoch, live.load(Ordering::SeqCst)) {
                            return Ok(None);
                        }
                        error = dispatch(&cmd, &expected, &mut s, &mut queue).err();
                        state = super::web_devices::snapshot()?
                            .ok_or("Spotify active device disappeared")?;
                        apply(&mut s, Some(&state), now_ms());
                        queue.observe_remote(&mut s, &state);
                    }
                    if !current(epoch, live.load(Ordering::SeqCst)) {
                        return Ok(None);
                    }
                    queue.refill(&mut s);
                    apply_queue_attribution(&mut s, &state);
                    if s.message.starts_with("Spotify station refill:")
                        || s.message.starts_with("Spotify queue continuation:")
                    {
                        error.get_or_insert(s.message.clone());
                    }
                    let uri = state["uri"].as_str().unwrap_or_default().to_owned();
                    if requested_uri != uri {
                        if metadata_tx.try_send((epoch, uri.clone())).is_ok() {
                            requested_uri = uri.clone();
                        }
                        metadata = None;
                    }
                    while let Ok((e, u, entry)) = entry_rx.try_recv() {
                        if metadata_matches(e, epoch, &u, &state) {
                            metadata = Some(entry);
                        }
                    }
                    if let Some(entry) = metadata.as_ref().filter(|e| e.id == uri) {
                        s.now = entry.clone();
                        if s.duration == 0 {
                            s.duration = entry.duration;
                        }
                        s.now.duration = s.duration;
                    }
                    if s.now.title.is_empty() {
                        if let Some(entry) = s.queue.get(s.queue_index).filter(|e| e.id == uri) {
                            s.now = entry.clone();
                        }
                    }
                    state["entry"] = serde_json::to_value(&s.now).unwrap_or(Value::Null);
                    state["duration_ms"] = json!(s.duration);
                    state["managed_queue"] =
                        serde_json::to_value(s.queue.as_ref()).unwrap_or(Value::Null);
                    state["queue_index"] = json!(s.queue_index);
                    state["radio_active"] = json!(s.radio_active);
                    Ok(Some(state))
                })();
                if current(epoch, live.load(Ordering::SeqCst)) {
                    let _ = tx.send((epoch, result, error));
                }
            }
        });
        Self {
            epoch,
            enabled: false,
            generation: 0,
            serial: 0,
            commands,
            updates,
            state: None,
        }
    }

    /// Disable on local activation/provider change. One worker remains parked;
    /// an in-flight browser request may finish, but its result cannot be adopted.
    pub(crate) fn enable(&mut self, enabled: bool, generation: u64) {
        if enabled != self.enabled || generation != self.generation {
            self.serial = self.serial.wrapping_add(1).max(1);
            self.epoch
                .store(if enabled { self.serial } else { 0 }, Ordering::SeqCst);
            self.state = None;
        }
        self.generation = generation;
        self.enabled = enabled;
    }

    pub(crate) fn dispatch(&self, command: &Command, s: &Snapshot) -> Result<(), String> {
        match command {
            Command::SwitchProvider(_) | Command::Discord(_) => return Ok(()),
            Command::Api(name, args) => return super::catalog_invoke(name, args).map(|_| ()),
            Command::Autoplay(_) => {
                return Err(
                    "Spotify autoplay preferences are not exposed by the verified Connect service"
                        .into(),
                )
            }
            _ => {}
        }
        if !self.enabled
            || (self.state.is_none()
                && !matches!(command, Command::Play(_) | Command::PlayEntry(..)))
        {
            return Err("Waiting for current remote Spotify playback state".into());
        }
        self.commands
            .try_send((
                self.epoch.load(Ordering::SeqCst),
                command.clone(),
                s.clone(),
            ))
            .map_err(|_| "Spotify remote control queue is busy".into())
    }

    pub(crate) fn update(&mut self, s: &mut Snapshot) {
        while let Ok((epoch, result, error)) = self.updates.try_recv() {
            if !current(epoch, self.epoch.load(Ordering::SeqCst)) {
                continue;
            }
            match result {
                Ok(state) => {
                    if state.is_some() && self.state.is_none() {
                        s.message.clear();
                    }
                    self.state = state;
                }
                Err(error) => {
                    self.state = None;
                    s.message = error;
                }
            }
            if let Some(error) = error {
                s.message = error;
            }
        }
        apply(s, self.state.as_ref(), now_ms());
    }
}

fn metadata_matches(epoch: u64, live: u64, uri: &str, state: &Value) -> bool {
    current(epoch, live) && state["uri"].as_str() == Some(uri)
}
fn dispatch(
    command: &Command,
    expected: &Snapshot,
    s: &mut Snapshot,
    queue: &mut super::playback::Queue,
) -> Result<(), String> {
    if expected.now.raw["spotify_device_id"]
        .as_str()
        .is_some_and(|id| s.now.raw["spotify_device_id"].as_str() != Some(id))
    {
        return Err("Spotify active device changed; refresh before controlling playback".into());
    }
    if matches!(
        command,
        Command::PlayQueueIndex(_)
            | Command::QueueClear
            | Command::QueueMove { .. }
            | Command::QueueInsertNext(_)
            | Command::QueueAddLast(_)
    ) && (expected.now.id != s.now.id
        || expected.queue_index != s.queue_index
        || expected
            .queue
            .iter()
            .map(|e| &e.id)
            .ne(s.queue.iter().map(|e| &e.id)))
    {
        return Err("Spotify queue changed; refresh before editing".into());
    }
    if queue.dispatch(command, s)? {
        return Ok(());
    }
    let (action, value) = match command {
        Command::Play(uri) => {
            return super::web_devices::play_context(
                uri,
                s.now.raw["spotify_device_id"]
                    .as_str()
                    .ok_or("Spotify returned no active device")?,
            )
        }
        Command::Toggle => (
            if s.state == "Playing" {
                "pause"
            } else {
                "resume"
            },
            None,
        ),
        Command::SetPlaying(true) => ("resume", None),
        Command::SetPlaying(false) | Command::Stop => ("pause", None),
        Command::Next => ("next", None),
        Command::Previous => ("previous", None),
        Command::Seek(percent) if s.duration > 0 => (
            "seek",
            Some(s.duration.saturating_mul((*percent).min(100) as u64) / 100),
        ),
        Command::SeekMs(ms) => (
            "seek",
            Some(if s.duration > 0 {
                (*ms).min(s.duration)
            } else {
                *ms
            }),
        ),
        Command::Volume(v) => ("volume", Some((*v).min(100) as u64)),
        Command::Shuffle(v) => ("shuffle", Some(u64::from(*v))),
        Command::Repeat(v) => ("repeat", Some(*v as u64)),
        _ => return Err("This command is unavailable on the remote Spotify device".into()),
    };
    super::web_devices::control_on(action, value, s.now.raw["spotify_device_id"].as_str())
}
fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(u64::MAX as u128) as u64
}

// Attribution belongs to a queue occurrence, not to the track URI: the
// same track can be queued more than once by different listeners.
fn apply_queue_attribution(s: &mut Snapshot, state: &Value) {
    let Some(upcoming) = state["next_tracks"].as_array() else {
        return;
    };
    let Some(start) = s.queue_index.checked_add(1) else {
        return;
    };
    let queue = Arc::make_mut(&mut s.queue);
    for (entry, track) in queue.iter_mut().skip(start).zip(upcoming) {
        if track["uri"].as_str() != Some(entry.id.as_str()) {
            break;
        }
        let name = [
            &track["queuedBy"],
            &track["queued_by"],
            &track["added_by"],
            &track["metadata"]["queuedBy"],
            &track["metadata"]["queued_by"],
            &track["metadata"]["added_by"],
        ]
        .into_iter()
        .find_map(|value| {
            value["name"]
                .as_str()
                .or_else(|| value["display_name"].as_str())
                .or_else(|| value.as_str())
        })
        .map(|name| {
            name.chars()
                .filter(|c| !c.is_control())
                .take(64)
                .collect::<String>()
        })
        .filter(|name| !name.trim().is_empty());
        if !entry.raw.is_object() {
            entry.raw = json!({});
        }
        if let Some(name) = name {
            entry.raw["added_by"] = json!(name.trim());
        } else if let Some(raw) = entry.raw.as_object_mut() {
            raw.remove("added_by");
        }
    }
}

fn apply(s: &mut Snapshot, state: Option<&Value>, now: u64) {
    s.playback_connected = state.is_some();
    s.state = "Stopped".into();
    s.now = Default::default();
    s.title.clear();
    s.position = 0;
    s.duration = 0;
    s.queue = Default::default();
    s.queue_index = 0;
    s.shuffle = false;
    s.repeat = 0;
    s.autoplay = false;
    s.radio_active = false;
    s.eq = Value::Null;
    s.quality.clear();
    let Some(state) = state else {
        return;
    };
    let playing = state["is_playing"].as_bool() == Some(true);
    s.state = if playing { "Playing" } else { "Paused" }.into();
    s.duration = state["duration_ms"].as_u64().unwrap_or(0);
    let elapsed = if playing {
        state["timestamp_ms"]
            .as_u64()
            .map(|t| now.saturating_sub(t))
            .unwrap_or(0)
    } else {
        0
    };
    s.position = state["position_ms"]
        .as_u64()
        .unwrap_or(0)
        .saturating_add(elapsed);
    if s.duration > 0 {
        s.position = s.position.min(s.duration);
    }
    let uri = state["uri"].as_str().unwrap_or_default();
    s.now = serde_json::from_value::<Entry>(state["entry"].clone())
        .ok()
        .filter(|e| e.id == uri)
        .unwrap_or_else(|| Entry {
            id: uri.into(),
            kind: if uri.starts_with("spotify:episode:") {
                "episode"
            } else {
                "track"
            }
            .into(),
            ..Default::default()
        });
    if s.duration == 0 {
        s.duration = s.now.duration;
        if s.duration > 0 {
            s.position = s.position.min(s.duration);
        }
    }
    s.now.duration = s.duration;
    s.now.raw["spotify_device_id"] = state["device"]["id"].clone();
    s.title = s.now.title.clone();
    s.shuffle = state["options"]["shuffling_context"]
        .as_bool()
        .unwrap_or(false);
    s.repeat = if state["options"]["repeating_track"].as_bool() == Some(true) {
        2
    } else if state["options"]["repeating_context"].as_bool() == Some(true) {
        1
    } else {
        0
    };
    s.queue = serde_json::from_value::<Vec<Entry>>(state["managed_queue"].clone())
        .map(Arc::new)
        .unwrap_or_else(|_| {
            let mut entries = vec![s.now.clone()];
            entries.extend(
                state["next_tracks"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|t| {
                        let id = t["uri"].as_str()?;
                        if !id.starts_with("spotify:track:") && !id.starts_with("spotify:episode:")
                        {
                            return None;
                        }
                        Some(Entry {
                            id: id.into(),
                            kind: if id.starts_with("spotify:episode:") {
                                "episode"
                            } else {
                                "track"
                            }
                            .into(),
                            ..Default::default()
                        })
                    }),
            );
            Arc::new(entries)
        });
    s.queue_index = state["queue_index"].as_u64().unwrap_or(0) as usize;
    s.radio_active = state["radio_active"].as_bool() == Some(true);
}

#[cfg(test)]
mod tests {
    #[test]
    fn remote_control_requires_the_explicitly_selected_device() {
        let state = serde_json::json!({"device":{"id":"speaker"}});
        assert!(!super::selected_target_matches(None, &state));
        assert!(!super::selected_target_matches(Some("browser"), &state));
        assert!(super::selected_target_matches(Some("speaker"), &state));
        assert!(!super::selected_target_matches(
            Some("speaker"),
            &serde_json::Value::Null
        ));
    }
    use super::*;
    use serde_json::json;
    #[test]
    fn queue_attribution_follows_occurrences_and_clears_missing_names() {
        let mut s = Snapshot::default();
        let state = json!({"uri":"spotify:track:current", "next_tracks":[
            {"uri":"spotify:track:same", "queuedBy":{"name":"Alice"}},
            {"uri":"spotify:track:same", "metadata":{"queued_by":{"display_name":"Bob"}}},
            {"uri":"spotify:track:other"}
        ]});
        apply(&mut s, Some(&state), 0);
        apply_queue_attribution(&mut s, &state);
        assert_eq!(s.queue[1].raw["added_by"], "Alice");
        assert_eq!(s.queue[2].raw["added_by"], "Bob");
        assert!(s.queue[3].raw["added_by"].is_null());
        let mut changed = state.clone();
        changed["next_tracks"][0]["queuedBy"] = Value::Null;
        apply_queue_attribution(&mut s, &changed);
        assert!(s.queue[1].raw["added_by"].is_null());
        assert_eq!(s.queue[2].raw["added_by"], "Bob");
    }
    #[test]
    fn metadata_options_and_uri_epoch_guards() {
        let mut s = Snapshot::default();
        let mut state = json!({"uri":"spotify:track:fixture","device":{"id":"remote"},
            "options":{"shuffling_context":true,"repeating_context":true},
        "entry":Entry{id:"spotify:track:fixture".into(),title:"Song".into(),subtitle:"Artist".into(),artwork:"https://art".into(),duration:210000,..Default::default()}});
        apply(&mut s, Some(&state), 0);
        assert_eq!(
            (
                s.title.as_str(),
                s.now.subtitle.as_str(),
                s.now.artwork.as_str()
            ),
            ("Song", "Artist", "https://art")
        );
        assert!(s.shuffle);
        assert_eq!((s.duration, s.now.duration), (210000, 210000));
        assert_eq!(s.repeat, 1);
        assert!(metadata_matches(3, 3, "spotify:track:fixture", &state));
        assert!(!metadata_matches(2, 3, "spotify:track:fixture", &state));
        assert!(!metadata_matches(3, 3, "spotify:track:old", &state));
        state["uri"] = json!("spotify:track:new");
        state["options"] = Value::Null;
        apply(&mut s, Some(&state), 0);
        assert!(s.title.is_empty() && !s.shuffle && s.repeat == 0);
    }
    #[test]
    fn global_position_time_and_options() {
        let mut s = Snapshot {
            connected: true,
            shuffle: true,
            repeat: 2,
            autoplay: true,
            ..Default::default()
        };
        let mut state = json!({"uri":"spotify:track:fixture","is_playing":true,"position_ms":1000,"timestamp_ms":5000,"duration_ms":10000,"device":{"id":"remote"}});
        apply(&mut s, Some(&state), 7000);
        assert_eq!(
            (s.position, s.duration, s.state.as_str()),
            (3000, 10000, "Playing")
        );
        assert!(s.connected && s.playback_connected);
        assert!(!s.shuffle && !s.autoplay && s.repeat == 0 && s.queue.len() == 1);
        assert_eq!(s.now.id, "spotify:track:fixture");
        state["is_playing"] = json!(false);
        apply(&mut s, Some(&state), 50000);
        assert_eq!((s.position, s.state.as_str()), (1000, "Paused"));
        state["is_playing"] = json!(true);
        apply(&mut s, Some(&state), 50000);
        assert_eq!(s.position, 10000);
        apply(&mut s, Some(&state), 4000);
        assert_eq!(s.position, 1000);
        apply(&mut s, None, 7000);
        assert!(!s.playback_connected && s.connected && s.now.id.is_empty());
    }
    #[test]
    fn stale_provider_and_disabled_results_are_rejected() {
        assert!(!current(1, 3));
        assert!(!current(1, 0));
        assert!(!current(0, 0));
        assert!(current(3, 3));
    }

    #[test]
    fn provider_round_trip_discards_old_reply_and_controls_are_not_optimistic() {
        let (commands, requests) = mpsc::sync_channel(8);
        let (replies, updates) = mpsc::sync_channel(1);
        let mut worker = Worker {
            epoch: Arc::new(AtomicU64::new(0)),
            enabled: false,
            generation: 0,
            serial: 0,
            commands,
            updates,
            state: None,
        };
        worker.enable(true, 1);
        let old = worker.epoch.load(Ordering::SeqCst);
        worker.enable(false, 2);
        worker.enable(true, 3);
        replies
            .send((
                old,
                Ok(Some(json!({"uri":"spotify:track:stale","is_playing":true}))),
                None,
            ))
            .unwrap();
        let mut s = Snapshot {
            message: "Configure Soloist for local playback".into(),
            ..Default::default()
        };
        worker.update(&mut s);
        assert!(!s.playback_connected && s.now.id.is_empty());
        let live = worker.epoch.load(Ordering::SeqCst);
        replies.send((live, Ok(Some(json!({"uri":"spotify:track:live","is_playing":false,"position_ms":20,"duration_ms":1000}))), None)).unwrap();
        worker.update(&mut s);
        worker.dispatch(&Command::Toggle, &s).unwrap();
        assert!(s.message.is_empty());
        assert!(matches!(requests.try_recv().unwrap().1, Command::Toggle));
        assert_eq!(s.state, "Paused");
        worker.dispatch(&Command::Seek(50), &s).unwrap();
        assert!(matches!(requests.try_recv().unwrap().1, Command::Seek(50)));
        worker.dispatch(&Command::Volume(200), &s).unwrap();
        assert!(matches!(
            requests.try_recv().unwrap().1,
            Command::Volume(200)
        ));
        assert!(worker.dispatch(&Command::QueueClear, &s).is_ok());
        assert!(worker
            .dispatch(&Command::Play("spotify:track:new".into()), &s)
            .is_ok());
        // Local activation followed by deactivation in the same provider epoch
        // also invalidates a browser request that was already in flight.
        worker.enable(false, 3);
        worker.enable(true, 3);
        replies
            .send((live, Ok(Some(json!({"uri":"spotify:track:stale"}))), None))
            .unwrap();
        worker.update(&mut s);
        assert!(!s.playback_connected);
    }
}
