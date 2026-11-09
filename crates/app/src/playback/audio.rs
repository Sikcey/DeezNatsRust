//! Engine operations run on a bounded worker queue, away from the window thread.
use crate::engine::Engine as Deezer;

mod autoplay {
    //! One host-owned radio station; recommendations stay private until needed.
    use super::{sync_queue, Snapshot};
    use crate::{catalog::Entry, engine::Engine};
    use std::{
        collections::VecDeque,
        sync::{mpsc, Arc},
    };

    #[derive(Clone)]
    struct Stamp {
        manual: bool,
        generation: u64,
        version: i64,
        index: usize,
        finished_id: String,
    }

    struct Pending {
        stamp: Stamp,
        result: mpsc::Receiver<Result<Vec<Entry>, String>>,
    }

    #[derive(Default)]
    pub(super) struct Autoplay {
        generation: u64,
        finished_count: Option<i64>,
        seed: Option<Entry>,
        pool: VecDeque<Entry>,
        recent: VecDeque<String>,
        pending: Option<Pending>,
        radio_id: String,
        radio_version: i64,
    }

    fn manual_radio(s: &Snapshot, in_radio: bool) -> bool {
        s.autoplay
            && s.connected
            && s.now.kind == "track"
            && !s.now.id.is_empty()
            && s.queue
                .get(s.queue_index)
                .is_some_and(|item| item.id == s.now.id)
            && (in_radio || (s.repeat == 0 && s.queue_index + 1 == s.queue.len()))
    }

    fn eligible(s: &Snapshot, manual: bool) -> bool {
        s.autoplay
            && s.connected
            && (s.repeat == 0 || manual)
            && s.state == "Stopped"
            && s.now.kind == "track"
            && !s.now.id.is_empty()
            && s.queue
                .get(s.queue_index)
                .is_some_and(|e| e.id == s.now.id && e.kind == "track")
    }

    impl Autoplay {
        pub(super) fn cancel(&mut self, reset_station: bool) {
            self.generation = self.generation.wrapping_add(1);
            self.pending = None;
            if reset_station {
                self.seed = None;
                self.pool.clear();
                self.radio_id.clear();
            }
        }

        fn remember(&mut self, item: &Entry) {
            if item.kind != "track" || item.id.is_empty() {
                return;
            }
            if self.seed.is_none() {
                self.seed = Some(item.clone());
            }
            if self.recent.back() != Some(&item.id) {
                self.recent.push_back(item.id.clone());
                if self.recent.len() > 100 {
                    self.recent.pop_front();
                }
            }
        }

        fn take_candidate(&mut self, s: &Snapshot) -> Option<Entry> {
            while let Some(item) = self.pool.pop_front() {
                if item.kind == "track"
                    && !item.id.is_empty()
                    && !self.recent.contains(&item.id)
                    && !s.queue.iter().any(|queued| queued.id == item.id)
                {
                    return Some(item);
                }
            }
            None
        }

        fn valid(&self, stamp: &Stamp, s: &Snapshot, version: i64) -> bool {
            stamp.generation == self.generation
                && stamp.version == version
                && stamp.index == s.queue_index
                && stamp.finished_id == s.now.id
                && eligible(s, stamp.manual)
        }

        fn natural_finish(&mut self, count: i64) -> bool {
            self.finished_count
                .replace(count)
                .is_some_and(|previous| count > previous)
        }

        fn play_one(
            &mut self,
            dz: &Engine,
            s: &mut Snapshot,
            version: &mut i64,
            stamp: &Stamp,
            item: Entry,
        ) -> Result<(), String> {
            if !self.valid(stamp, s, *version) {
                return Ok(());
            }
            if item.id.starts_with("spotify:") {
                return Err("Spotify radio must use the Spotify playback queue".into());
            }
            // Insert exactly one row, preserving the native shuffle cycle/history.
            let index = s.queue_index + 1;
            dz.invoke(
                "DZQueueInsertNext",
                &[serde_json::json!(item.wire().to_string())],
            )?;
            dz.invoke("DZQueueSetIndex", &[serde_json::json!(index)])?;
            let mut queue = (*s.queue).clone();
            queue.insert(index.min(queue.len()), item.clone());
            s.queue = Arc::new(queue);
            sync_queue(dz, s, version)?;
            if dz
                .invoke(
                    "DZPlay",
                    &[serde_json::json!(item.id), serde_json::json!(item.duration)],
                )?
                .as_i64()
                != Some(1)
            {
                return Err(dz.error());
            }
            s.now = item;
            s.title = s.now.title.clone();
            s.state = "Playing".into();
            s.position = 0;
            s.duration = s.now.duration;
            self.remember(&s.now);
            self.radio_id = s.now.id.clone();
            self.radio_version = *version;
            s.radio_active = true;
            Ok(())
        }

        pub(super) fn skip_radio(
            &mut self,
            dz: &Arc<Engine>,
            s: &mut Snapshot,
            version: &mut i64,
        ) -> Result<bool, String> {
            let in_radio = self.radio_id == s.now.id
                && !self.radio_id.is_empty()
                && self.radio_version == *version;
            if !manual_radio(s, in_radio) {
                return Ok(false);
            }
            unsafe {
                (dz.stop)();
            }
            s.state = "Stopped".into();
            self.remember(&s.now);
            self.begin_next(dz, s, version, true);
            Ok(true)
        }

        pub(super) fn poll(
            &mut self,
            dz: &Arc<Engine>,
            s: &mut Snapshot,
            version: &mut i64,
            allow_start: bool,
        ) {
            self.remember(&s.now);
            s.radio_active = self.radio_id == s.now.id
                && !self.radio_id.is_empty()
                && self.radio_version == *version;
            let Ok(count) = dz
                .invoke("DZFinishedCount", &[])
                .and_then(|v| v.as_i64().ok_or("Invalid finish count".into()))
            else {
                return;
            };
            let ended = self.natural_finish(count);
            if let Some(pending) = self.pending.take() {
                if self.valid(&pending.stamp, s, *version) {
                    match pending.result.try_recv() {
                        Ok(Ok(tracks)) => {
                            self.pool = tracks.into();
                            if let Some(item) = self.take_candidate(s) {
                                if let Err(error) =
                                    self.play_one(dz, s, version, &pending.stamp, item)
                                {
                                    s.message = error;
                                }
                            } else {
                                s.message = "Autoplay found no new related songs".into();
                            }
                        }
                        Ok(Err(error)) => s.message = format!("Autoplay: {error}"),
                        Err(mpsc::TryRecvError::Empty) => self.pending = Some(pending),
                        Err(mpsc::TryRecvError::Disconnected) => {}
                    }
                }
            }
            if !ended || !allow_start || !eligible(s, false) || self.pending.is_some() {
                return;
            }
            self.begin_next(dz, s, version, false);
        }

        fn begin_next(
            &mut self,
            dz: &Arc<Engine>,
            s: &mut Snapshot,
            version: &mut i64,
            manual: bool,
        ) {
            let stamp = Stamp {
                manual,
                generation: self.generation,
                version: *version,
                index: s.queue_index,
                finished_id: s.now.id.clone(),
            };
            if let Some(item) = self.take_candidate(s) {
                if let Err(error) = self.play_one(dz, s, version, &stamp, item) {
                    s.message = error;
                }
                return;
            }
            let Some(seed) = self.seed.clone() else {
                return;
            };
            let engine = Arc::clone(dz);
            let excluded: Vec<_> = s
                .queue
                .iter()
                .map(|e| e.id.clone())
                .chain(self.recent.iter().cloned())
                .collect();
            let (tx, result) = mpsc::channel();
            self.pending = Some(Pending { stamp, result });
            std::thread::spawn(move || {
                let fetch = |name, id| {
                    let value = if seed.id.starts_with("spotify:") {
                        crate::spotify::catalog_invoke(name, &[serde_json::json!(id)])
                    } else {
                        engine.invoke(name, &[serde_json::json!(id)])
                    };
                    value.map(|v| crate::catalog::entries(&v, "track"))
                };
                // A country/genre station must never widen into generic track radio.
                if let Some(key) = seed.raw[crate::style_mix::MARKER].as_str() {
                    let tracks = if seed.id.starts_with("spotify:") {
                        crate::catalog::spotify_style_mix(key)
                            .map(|value| crate::catalog::entries(&value, "track"))
                    } else {
                        crate::style_mix::Selection::parse(key)
                            .and_then(|selection| crate::style_mix::tracks(&engine, selection))
                    };
                    let _ = tx.send(tracks);
                    return;
                }
                let mut tracks = fetch("DZTrackMixJSON", &seed.id);
                if !seed.artist_id.is_empty()
                    && tracks.as_ref().map_or(true, |items| {
                        !items
                            .iter()
                            .any(|e| e.kind == "track" && !excluded.contains(&e.id))
                    })
                {
                    tracks = fetch("DZArtistMixJSON", &seed.artist_id);
                }
                let _ = tx.send(tracks);
            });
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        fn track(id: &str) -> Entry {
            Entry {
                id: id.into(),
                kind: "track".into(),
                ..Entry::default()
            }
        }
        fn stopped() -> Snapshot {
            Snapshot {
                autoplay: true,
                connected: true,
                state: "Stopped".into(),
                now: track("seed"),
                queue: Arc::new(vec![track("seed")]),
                ..Snapshot::default()
            }
        }

        #[test]
        fn manual_skip_starts_radio_only_at_non_looping_queue_end() {
            let mut s = stopped();
            s.state = "Playing".into();
            assert!(manual_radio(&s, false));
            for repeat in [1, 2] {
                s.repeat = repeat;
                assert!(!manual_radio(&s, false));
                assert!(manual_radio(&s, true));
            }
            s.repeat = 0;
            Arc::make_mut(&mut s.queue).push(track("next"));
            assert!(!manual_radio(&s, false));
            s.queue_index = 1;
            s.now = track("next");
            assert!(manual_radio(&s, false));
            s.autoplay = false;
            assert!(!manual_radio(&s, false));
            s.autoplay = true;
            s.now.kind = "episode".into();
            assert!(!manual_radio(&s, false));
        }

        #[test]
        fn radio_is_consumed_one_by_one_without_reseeding_or_queue_pollution() {
            let mut a = Autoplay::default();
            let mut s = stopped();
            a.remember(&s.now);
            a.pool = vec![track("seed"), track("one"), track("one"), track("two")].into();
            let next = a.take_candidate(&s).unwrap();
            assert_eq!(next.id, "one");
            assert_eq!(s.queue.len(), 1); // Selection never adds the radio batch.
            s.now = next;
            a.remember(&s.now);
            assert_eq!(a.take_candidate(&s).unwrap().id, "two");
            assert_eq!(a.seed.as_ref().unwrap().id, "seed");
            assert!(a.take_candidate(&s).is_none());
        }

        #[test]
        fn late_radio_results_cannot_override_user_intent() {
            let mut a = Autoplay::default();
            let mut s = stopped();
            let stamp = Stamp {
                manual: false,
                generation: 0,
                version: 10,
                index: 0,
                finished_id: "seed".into(),
            };
            assert!(a.valid(&stamp, &s, 10));
            assert!(!a.valid(&stamp, &s, 11)); // Queue changed while fetching.
            for mode in [1, 2] {
                s.repeat = mode;
                assert!(!a.valid(&stamp, &s, 10));
                let manual = Stamp {
                    manual: true,
                    ..stamp.clone()
                };
                assert!(a.valid(&manual, &s, 10));
            }
            s.repeat = 0;
            s.autoplay = false;
            assert!(!a.valid(&stamp, &s, 10));
            s.autoplay = true;
            s.state = "Playing".into();
            assert!(!a.valid(&stamp, &s, 10));
            s.state = "Stopped".into();
            s.now.kind = "episode".into();
            assert!(!a.valid(&stamp, &s, 10));
            s.now.kind = "track".into();
            a.cancel(false);
            assert!(!a.valid(&stamp, &s, 10));
        }

        #[test]
        fn only_a_new_natural_finish_can_consume_another_recommendation() {
            let mut a = Autoplay::default();
            assert!(!a.natural_finish(4)); // Old finishes at startup are not new events.
            assert!(!a.natural_finish(4));
            assert!(a.natural_finish(5));
            assert!(!a.natural_finish(5));
            assert!(!a.natural_finish(0)); // Engine restart resets the baseline.
            assert!(a.natural_finish(1));
        }

        #[test]
        fn skipping_preserves_the_original_station_and_remaining_order() {
            let mut a = Autoplay::default();
            let s = stopped();
            a.remember(&s.now);
            a.pool = vec![track("one"), track("two")].into();
            assert_eq!(a.take_candidate(&s).unwrap().id, "one");
            a.cancel(false); // Next cancels the outstanding fetch, not the station.
            assert_eq!(a.seed.as_ref().unwrap().id, "seed");
            assert_eq!(a.take_candidate(&s).unwrap().id, "two");
        }
    }
}
mod discord {
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
            buttons
                .push(json!({"label":"DeezNats","url":"https://github.com/Sikcey/DeezNatsRust"}));
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
                            let result = client.poll().and_then(|_| {
                                client.publish(activity(&config, &playback, now), false)
                            });
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
            let handle = unsafe {
                CreateNamedPipeW(wide.as_ptr(), 3, 1, 1, 8192, 8192, 0, std::ptr::null())
            };
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
}
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

// Materialize shuffle in the engine queue so automatic advancement, preloading,
// manual navigation and the visible upcoming list all agree on the next row.
#[derive(Default)]
pub(crate) struct PreparedShuffle {
    original: Vec<crate::catalog::Entry>,
}

impl PreparedShuffle {
    pub(crate) fn prepare(&mut self, tracks: &mut [crate::catalog::Entry], current: usize) {
        self.original = tracks.to_vec();
        let start = current.saturating_add(1).min(tracks.len());
        let mut random = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos() as u64
            | 1;
        for i in (start + 1..tracks.len()).rev() {
            random ^= random << 13;
            random ^= random >> 7;
            random ^= random << 17;
            tracks.swap(i, start + random as usize % (i - start + 1));
        }
        Self::spread_artists(tracks, start, &mut random);
    }

    fn artist_key(row: &crate::catalog::Entry) -> String {
        if !row.artist_id.trim().is_empty() {
            format!("artist:{}", row.artist_id.trim())
        } else if !row.subtitle.trim().is_empty() {
            format!("name:{}", row.subtitle.trim().to_lowercase())
        } else {
            // Missing credits must not turn unrelated tracks into one artist.
            format!("track:{}:{}", row.kind, row.id)
        }
    }

    fn spread_artists(tracks: &mut [crate::catalog::Entry], start: usize, random: &mut u64) {
        use std::cmp::Reverse;
        use std::collections::{BTreeMap, BinaryHeap, VecDeque};

        // Spotify's classic perceptual shuffle idea: randomize within each
        // artist, then distribute that artist's songs at roughly equal intervals.
        // Random phase and jitter keep the interleaving from becoming periodic.
        let mut grouped = BTreeMap::<String, VecDeque<crate::catalog::Entry>>::new();
        for row in &tracks[start..] {
            grouped
                .entry(Self::artist_key(row))
                .or_default()
                .push_back(row.clone());
        }
        let mut groups: Vec<_> = grouped.into_iter().collect();
        let mut next = BinaryHeap::new();
        let mut steps = Vec::with_capacity(groups.len());
        let mut positions = Vec::with_capacity(groups.len());
        for (i, (_, rows)) in groups.iter().enumerate() {
            let step = (1u64 << 48) / rows.len() as u64;
            *random ^= *random << 13;
            *random ^= *random >> 7;
            *random ^= *random << 17;
            let phase = *random % step;
            steps.push(step);
            positions.push(phase);
            next.push(Reverse((phase, i)));
        }
        let mut previous = start.checked_sub(1).map(|i| Self::artist_key(&tracks[i]));
        for row in &mut tracks[start..] {
            let Some(Reverse(mut candidate)) = next.pop() else {
                break;
            };
            if previous.as_ref() == Some(&groups[candidate.1].0) && !next.is_empty() {
                let alternative = next.pop().unwrap().0;
                next.push(Reverse(candidate));
                candidate = alternative;
            }
            let i = candidate.1;
            *row = groups[i].1.pop_front().unwrap();
            previous = Some(groups[i].0.clone());
            if !groups[i].1.is_empty() {
                *random ^= *random << 13;
                *random ^= *random >> 7;
                *random ^= *random << 17;
                let jitter = steps[i] / 5;
                positions[i] += steps[i] - jitter / 2 + *random % jitter.max(1);
                next.push(Reverse((positions[i], i)));
            }
        }
    }

    pub(crate) fn restore(
        &mut self,
        tracks: &mut Vec<crate::catalog::Entry>,
        current: usize,
    ) -> usize {
        // Track row identities, including duplicate occurrences, across queue edits.
        let mut remaining: Vec<_> = std::mem::take(tracks).into_iter().enumerate().collect();
        let mut selected = 0;
        for original in self.original.drain(..) {
            if let Some(at) = remaining
                .iter()
                .position(|(_, row)| row.id == original.id && row.kind == original.kind)
            {
                let (index, row) = remaining.remove(at);
                if index == current {
                    selected = tracks.len();
                }
                tracks.push(row);
            }
        }
        for (index, row) in remaining {
            if index == current {
                selected = tracks.len();
            }
            tracks.push(row);
        }
        selected
    }
}
#[cfg(test)]
mod prepared_shuffle_tests {
    use super::*;

    fn rows(ids: &[&str]) -> Vec<crate::catalog::Entry> {
        ids.iter()
            .map(|id| crate::catalog::Entry {
                id: (*id).into(),
                kind: "track".into(),
                ..Default::default()
            })
            .collect()
    }

    #[test]
    fn prepared_shuffle_preserves_current_and_restores_order_after_navigation() {
        let mut order = PreparedShuffle::default();
        let mut tracks = rows(&["a", "b", "c", "d", "e", "f"]);
        order.prepare(&mut tracks, 1);
        assert_eq!(tracks[0].id, "a");
        assert_eq!(tracks[1].id, "b");
        let selected = tracks[4].id.clone();
        let index = order.restore(&mut tracks, 4);
        assert_eq!(tracks[index].id, selected);
        assert_eq!(
            tracks.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(),
            ["a", "b", "c", "d", "e", "f"]
        );
    }

    #[test]
    fn prepared_shuffle_restore_keeps_queue_edits_and_duplicates() {
        let mut order = PreparedShuffle::default();
        let mut tracks = rows(&["a", "b", "b", "c"]);
        order.prepare(&mut tracks, 0);
        tracks.retain(|r| r.id != "c");
        tracks.insert(1, rows(&["new"]).remove(0));
        let index = order.restore(&mut tracks, 1);
        assert_eq!(tracks[index].id, "new");
        assert_eq!(
            tracks.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(),
            ["a", "b", "b", "new"]
        );
        order.prepare(&mut [], 0);
        order.prepare(&mut rows(&["only"]), 0);
    }

    #[test]
    fn artist_shuffle_spreads_balanced_artists_including_current_boundary() {
        for seed in 1..100 {
            let mut tracks = rows(&[
                "playing", "a1", "a2", "a3", "b1", "b2", "b3", "c1", "c2", "c3",
            ]);
            tracks[0].artist_id = "a".into();
            for row in &mut tracks[1..] {
                row.artist_id = row.id[..1].into();
            }
            PreparedShuffle::spread_artists(&mut tracks, 1, &mut (seed as u64));
            assert_eq!(tracks[0].id, "playing");
            assert!(tracks
                .windows(2)
                .all(|pair| pair[0].artist_id != pair[1].artist_id));
            let mut ids: Vec<_> = tracks.iter().map(|r| r.id.as_str()).collect();
            ids.sort_unstable();
            assert_eq!(
                ids,
                ["a1", "a2", "a3", "b1", "b2", "b3", "c1", "c2", "c3", "playing"]
            );
        }
    }

    #[test]
    fn artist_shuffle_handles_dominant_artist_and_missing_credits() {
        let mut tracks = rows(&["playing", "a1", "a2", "a3", "a4", "b1"]);
        for row in &mut tracks {
            row.artist_id = row.id[..1].into();
        }
        PreparedShuffle::spread_artists(&mut tracks, 1, &mut 7);
        assert_eq!(tracks.len(), 6);
        assert_eq!(tracks.iter().filter(|r| r.artist_id == "a").count(), 4);
        assert_eq!(tracks.iter().filter(|r| r.artist_id == "b").count(), 1);
        let mut first = rows(&["x"]).remove(0);
        let mut second = rows(&["y"]).remove(0);
        assert_ne!(
            PreparedShuffle::artist_key(&first),
            PreparedShuffle::artist_key(&second)
        );
        first.subtitle = " Artist ".into();
        second.subtitle = "artist".into();
        assert_eq!(
            PreparedShuffle::artist_key(&first),
            PreparedShuffle::artist_key(&second)
        );
    }
}

pub(crate) fn reordered_queue(
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
pub(crate) fn windows_default_output_id() -> Result<String, String> {
    use windows::Win32::{
        Media::Audio::{eMultimedia, eRender, IMMDeviceEnumerator, MMDeviceEnumerator},
        System::Com::{CoCreateInstance, CoTaskMemFree, CLSCTX_ALL},
    };
    unsafe {
        let enumerator: IMMDeviceEnumerator =
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).map_err(|e| e.to_string())?;
        let device = enumerator
            .GetDefaultAudioEndpoint(eRender, eMultimedia)
            .map_err(|e| e.to_string())?;
        let id = device.GetId().map_err(|e| e.to_string())?;
        let result = id.to_string().map_err(|e| e.to_string());
        CoTaskMemFree(Some(id.0.cast()));
        result
    }
}

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
            let mut youtube = crate::youtube::Player::default();
            let mut deezer_snapshot: Option<Snapshot> = None;

            let mut failed_index: Option<usize> = None;
            let mut handled_error: Option<usize> = None;
            let mut volume = 100;
            let mut spotify_volume_applied = false;
            let mut queue_version: i64 = -1;
            let mut prepared_shuffle = PreparedShuffle::default();
            let mut autoplay = autoplay::Autoplay::default();
            let mut snapshot = Snapshot {
                autoplay: true,
                discord_settings: discord.settings(),
                message: "Connect your Deezer account to start listening.".into(),
                state: "Stopped".into(),
                ..Default::default()
            };
            let mut deferred_command = None;
            #[cfg(feature = "diagnostics")]
            let mut playback_started: Option<std::time::Instant> = None;
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
                if let Ok(Command::Volume(value)) = &command {
                    volume = (*value).min(100);
                    spotify_volume_applied = false;
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
                        spotify_volume_applied = false;
                        youtube.suspend();
                        crate::spotify::remote::select_device(None);
                        spotify_pending = None;
                        spotify_queue = Default::default();
                        autoplay.cancel(true);
                        if snapshot.provider == crate::catalog::Provider::Deezer {
                            deezer_snapshot = Some(snapshot.clone());
                            if let Some(dz) = &engine {
                                let _ = dz.invoke("DZPause", &[]);
                            }
                        } else if snapshot.provider == crate::catalog::Provider::Spotify {
                            drop(spotify.take());
                        }
                        let autoplay_enabled = snapshot.autoplay;
                        snapshot = if provider == crate::catalog::Provider::Deezer {
                            deezer_snapshot.take().unwrap_or_default()
                        } else {
                            Snapshot::default()
                        };
                        snapshot.autoplay = autoplay_enabled;
                        snapshot.provider = provider;
                        snapshot.state = "Stopped".into();
                    }
                    provider.set_active();
                    if provider == crate::catalog::Provider::YouTube {
                        youtube.handle(&Command::LoginSaved, &mut snapshot);
                        youtube.handle(&Command::Volume(volume), &mut snapshot);
                    } else if provider == crate::catalog::Provider::Deezer {
                        if let Some(dz) = &engine {
                            unsafe { (dz.volume)(volume as f64 / 100.0) };
                        }
                    }
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
                if snapshot.provider == crate::catalog::Provider::YouTube {
                    if matches!(&command, Ok(Command::Quit)) {
                        youtube.stop();
                        break;
                    }
                    if let Ok(cmd) = &command {
                        if let Command::Discord(value) = cmd {
                            match discord.configure(value) {
                                Ok(settings) => snapshot.discord_settings = settings,
                                Err(e) => snapshot.message = e,
                            }
                        } else {
                            youtube.handle(cmd, &mut snapshot);
                        }
                    }
                    youtube.poll(&mut snapshot);
                    discord.update(&snapshot);
                    let _ = tx.try_send(snapshot.clone());
                    continue;
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
                    if !local_active || explicit_remote {
                        spotify_volume_applied = false;
                    } else if !spotify_volume_applied {
                        if let Some(handle) = &handle {
                            spotify_volume_applied = handle
                                .control(&crate::spotify::soloist::Control::Volume(volume as u8))
                                .is_ok();
                        }
                    }
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
                #[cfg(feature = "diagnostics")]
                let play_timing = matches!(
                    &command,
                    Ok(Command::Play(_)
                        | Command::PlayEntry(..)
                        | Command::PlayQueueIndex(_)
                        | Command::Next
                        | Command::Previous)
                )
                .then(|| {
                    playback_started = Some(std::time::Instant::now());
                    crate::diagnostics::record("deezer.play.dequeue", "audio", Duration::ZERO);
                    crate::diagnostics::span("deezer.play.handle", "audio")
                });
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
                            dz.invoke("DZSetShuffle", &[serde_json::json!(0)])?;
                            snapshot.shuffle = false;
                            prepared_shuffle.original.clear();
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
                            if snapshot.shuffle {
                                prepared_shuffle.prepare(&mut tracks, index);
                            }
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
                                sync_queue(dz, &mut snapshot, &mut queue_version)?;
                                if value != snapshot.shuffle {
                                    let mut tracks = snapshot.queue.as_ref().clone();
                                    let index = if value {
                                        prepared_shuffle.prepare(&mut tracks, snapshot.queue_index);
                                        snapshot.queue_index
                                    } else {
                                        prepared_shuffle.restore(&mut tracks, snapshot.queue_index)
                                    };
                                    dz.invoke("DZSetShuffle", &[serde_json::json!(0)])?;
                                    set_queue(dz, &tracks, index)?;
                                    snapshot.queue = Arc::new(tracks);
                                    snapshot.queue_index = index;
                                    snapshot.shuffle = value;
                                    queue_version = -1;
                                }
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
                #[cfg(feature = "diagnostics")]
                drop(play_timing);
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
                        autoplay.poll(dz, &mut snapshot, &mut queue_version, allow_autoplay);
                    }
                }
                #[cfg(feature = "diagnostics")]
                if playback_started.is_some()
                    && matches!(snapshot.state.as_str(), "Playing" | "Error")
                {
                    let event = if snapshot.state == "Playing" {
                        "deezer.play.ready"
                    } else {
                        "deezer.play.error"
                    };
                    crate::diagnostics::record(
                        event,
                        "audio",
                        playback_started.take().unwrap().elapsed(),
                    );
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
