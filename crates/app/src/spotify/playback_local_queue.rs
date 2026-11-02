//! App-owned queue for Soloist's single-URI playback API. Queue edits do not
//! restart the current track or depend on a hidden Web Player Connect sender.
use super::{
    metadata, playable, selected, station, station_tracks, Command, Entry, Snapshot, STATION,
};
use crate::spotify::{soloist::Control, state};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};

#[derive(Default)]
pub(super) struct LocalQueue {
    items: Arc<Vec<Entry>>,
    index: usize,
    pending: Option<Instant>,
    armed: bool,
    last_position: u64,
    last_duration: u64,
    shuffle: bool,
    repeat: u8,
    stopped: bool,
    stop_requested: Option<Instant>,
    refill_at: Option<Instant>,
}
impl LocalQueue {
    pub(super) fn is_managed(&self) -> bool {
        !self.items.is_empty()
    }

    fn publish(&self, s: &mut Snapshot) {
        s.queue = self.items.clone();
        s.queue_index = self.index;
        s.shuffle = self.shuffle;
        s.repeat = self.repeat;
        s.radio_active = station(&self.items[self.index]).is_some();
    }

    fn play(
        &mut self,
        index: usize,
        send: &mut impl FnMut(&Control) -> Result<(), String>,
    ) -> Result<(), String> {
        let item = self
            .items
            .get(index)
            .ok_or("Invalid Spotify queue selection")?;
        send(&Control::Play(Some(item.id.clone())))?;
        self.index = index;
        self.pending = Some(Instant::now());
        self.armed = false;
        self.stopped = false;
        self.stop_requested = None;
        self.last_position = 0;
        self.last_duration = 0;
        Ok(())
    }

    fn next(&self, natural: bool) -> Option<usize> {
        use deeznats_core::playback::{next_index, RepeatMode};
        let repeat = match self.repeat {
            1 => RepeatMode::All,
            2 if natural => RepeatMode::One,
            _ => RepeatMode::Off,
        };
        next_index(
            Some(self.index),
            self.items.len(),
            repeat,
            self.shuffle,
            |upper| {
                // Only queue selection needs entropy; no credential or network state.
                let tick = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .subsec_nanos();
                tick as usize % upper
            },
        )
    }

    pub(super) fn dispatch(
        &mut self,
        command: &Command,
        s: &mut Snapshot,
        send: &mut impl FnMut(&Control) -> Result<(), String>,
    ) -> Result<bool, String> {
        match command {
            Command::PlayEntry(entry, supplied) => {
                let mut items = if supplied.is_empty() {
                    vec![(**entry).clone()]
                } else {
                    supplied.clone()
                };
                if items.len() > 10_000 || !items.iter().all(playable) {
                    return Err(
                        "Spotify queue requires valid tracks or episodes (maximum 10000)".into(),
                    );
                }
                let index = selected(entry, &items)?;
                crate::audio::mark_queue_fallback(&mut items);
                if let Some(marker) = station(entry) {
                    for item in &mut items {
                        item.raw[STATION] = marker.clone();
                        metadata(item, entry);
                    }
                }
                // Native repeat/context shuffle apply to Soloist's single-item
                // context. The collection's repeat and shuffle belong to this queue.
                send(&Control::Repeat("off".into()))?;
                send(&Control::Shuffle(false))?;
                send(&Control::Play(Some(items[index].id.clone())))?;
                self.items = Arc::new(items);
                self.index = index;
                self.pending = Some(Instant::now());
                self.armed = false;
                self.stopped = false;
                self.stop_requested = None;
                self.last_position = 0;
                self.last_duration = 0;
                self.refill_at = None;
                self.shuffle = s.shuffle;
                self.repeat = s.repeat;
            }
            Command::Play(_) => {
                *self = Self::default();
                return Ok(false);
            }
            Command::Stop => {
                self.stopped = true;
                self.stop_requested = Some(Instant::now());
                self.armed = false;
                self.pending = None;
                return Ok(false);
            }
            Command::PlayQueueIndex(index) => {
                if !self.is_managed() {
                    self.adopt(s)?;
                }
                self.play(*index, send)?;
            }
            Command::QueueAddLast(entry) | Command::QueueInsertNext(entry) => {
                if !playable(entry) {
                    return Err("Only Spotify tracks and episodes can be queued".into());
                }
                if !self.is_managed() {
                    self.adopt(s)?;
                }
                if self.items.len() >= 10_000 {
                    return Err("Spotify queue is full".into());
                }
                let at = if matches!(command, Command::QueueInsertNext(_)) {
                    self.index + 1
                } else {
                    crate::audio::queue_add_position(&self.items, self.index)
                };
                Arc::make_mut(&mut self.items).insert(at, crate::audio::explicit_queue_entry((**entry).clone()));
            }
            Command::QueueClear => {
                if !self.is_managed() {
                    self.adopt(s)?;
                }
                self.items = Arc::new(vec![self.items[self.index].clone()]);
                self.index = 0;
            }
            Command::QueueMove { from, to, expected } => {
                if !self.is_managed() {
                    self.adopt(s)?;
                }
                if self.items.iter().map(|e| &e.id).ne(expected.iter()) {
                    return Err("Queue changed while dragging; try again".into());
                }
                if *from <= self.index
                    || *to <= self.index
                    || *from >= self.items.len()
                    || *to >= self.items.len()
                {
                    return Err("Only upcoming Spotify items can be reordered".into());
                }
                let items = Arc::make_mut(&mut self.items);
                let item = items.remove(*from);
                items.insert(*to, item);
            }
            Command::Next if self.is_managed() => {
                if let Some(index) = self.next(false) {
                    self.play(index, send)?;
                } else {
                    send(&Control::Pause)?;
                    self.stopped = true;
                    self.stop_requested = Some(Instant::now());
                    self.armed = false;
                }
            }
            Command::Previous if self.is_managed() => {
                if s.position > 3000 {
                    send(&Control::Seek(0))?;
                } else {
                    self.play(self.index.saturating_sub(1), send)?;
                }
            }
            Command::Shuffle(enabled) if self.is_managed() => self.shuffle = *enabled,
            Command::Repeat(mode) if self.is_managed() => {
                if *mode > 2 {
                    return Err("Invalid repeat mode".into());
                }
                self.repeat = *mode;
            }
            Command::Toggle | Command::SetPlaying(true) if self.is_managed() && self.stopped => {
                self.play(self.index, send)?;
            }
            _ => return Ok(false),
        }
        self.publish(s);
        s.message = if self.pending.is_some() {
            "Spotify playback requested"
        } else {
            "Spotify queue updated"
        }
        .into();
        Ok(true)
    }

    fn adopt(&mut self, s: &Snapshot) -> Result<(), String> {
        if !playable(&s.now) {
            return Err("Start Spotify playback before editing its queue".into());
        }
        // Adopting a native context would leave its existing upcoming queue in
        // Soloist. Do not claim to clear/reorder a queue this API cannot replace.
        if s.queue.len() > s.queue_index + 1 {
            return Err("Start a track or collection in DeezNats before editing its queue".into());
        }
        self.items = Arc::new(vec![s.now.clone()]);
        self.index = 0;
        self.shuffle = s.shuffle;
        self.repeat = s.repeat;
        Ok(())
    }

    pub(super) fn refill(&mut self, s: &mut Snapshot) {
        if !self.is_managed()
            || s.state != "Playing"
            || self.repeat == 2
            || self.items.len().saturating_sub(self.index + 1) > 2
            || self
                .refill_at
                .is_some_and(|at| at.elapsed() < Duration::from_secs(30))
        {
            return;
        }
        let Some(marker) = station(&self.items[self.index]).cloned() else {
            return;
        };
        self.refill_at = Some(Instant::now());
        let result = (|| {
            let value = crate::spotify::catalog_invoke(
                marker["name"].as_str().unwrap(),
                marker["args"].as_array().unwrap(),
            )?;
            let mut tracks = station_tracks(&value)?;
            let seed = &self.items[self.index];
            for track in &mut tracks {
                if let (Some(expected), Some(actual)) = (
                    seed.raw.get(crate::style_mix::MARKER),
                    track.raw.get(crate::style_mix::MARKER),
                ) {
                    if expected != actual {
                        return Err(
                            "Spotify station refill returned different style selection".into()
                        );
                    }
                }
                track.raw[STATION] = marker.clone();
                metadata(track, seed);
            }
            let mut items = self.items[self.index..].to_vec();
            if items.len() + tracks.len() > 10_000 {
                return Err("Spotify station refill exceeds queue size limit".into());
            }
            items.extend(tracks);
            self.items = Arc::new(items);
            self.index = 0;
            self.publish(s);
            Ok::<_, String>(())
        })();
        if let Err(error) = result {
            s.message = format!("Spotify station refill: {error}");
        }
    }

    pub(super) fn observe(
        &mut self,
        s: &mut Snapshot,
        state: &state::Snapshot,
        send: &mut impl FnMut(&Control) -> Result<(), String>,
    ) -> bool {
        if !self.is_managed() {
            return false;
        }
        let matching = s.now.id == self.items[self.index].id;
        if self.pending.is_some() {
            // Repeated URIs identify separate occurrences. An old end-of-track
            // event for the same URI cannot confirm the newly requested one.
            if matching && s.position <= 2500 && matches!(s.state.as_str(), "Playing" | "Paused") {
                self.pending = None;
            } else if self
                .pending
                .is_some_and(|at| at.elapsed() >= Duration::from_secs(5))
            {
                *self = Self::default();
                s.message = "Spotify did not confirm selected queue item".into();
                return false;
            } else {
                self.publish(s);
                return true;
            }
        }
        let near_end =
            self.last_duration > 0 && self.last_position.saturating_add(1500) >= self.last_duration;
        let ended = self.armed
            && !self.stopped
            && near_end
            && state.connected
            && state.logged_in
            && (matches!(state.status.as_str(), "idle" | "stopped")
                || (!matching && state.is_active));
        if ended {
            self.armed = false;
            if let Some(index) = self.next(true) {
                if let Err(error) = self.play(index, send) {
                    s.message = error;
                    self.stopped = true;
                }
            } else {
                // Soloist may already have started Spotify's own autoplay item.
                // Stop that audio when the app-owned collection is exhausted.
                if let Err(error) = send(&Control::Pause) {
                    s.message = error;
                }
                self.stopped = true;
                self.stop_requested = Some(Instant::now());
            }
            self.publish(s);
            return true;
        }
        if !matching {
            if self.stopped
                && (state.status != "playing"
                    || self
                        .stop_requested
                        .is_some_and(|at| at.elapsed() < Duration::from_secs(3)))
            {
                // Keep the finished queue available for replay. The actual
                // engine item remains the now-playing metadata until replay;
                // never relabel a native autoplay item as the last queue entry.
                self.publish(s);
                return true;
            }
            // A user selected a different context outside this managed queue.
            *self = Self::default();
            return false;
        }
        let catalog = &self.items[self.index];
        // Incremental engine events can carry identity without decorations.
        // Restore metadata only for the confirmed matching occurrence.
        if s.now.title.is_empty() {
            s.now.title.clone_from(&catalog.title);
        }
        if s.now.subtitle.is_empty() {
            s.now.subtitle.clone_from(&catalog.subtitle);
        }
        if s.now.artwork.is_empty() {
            s.now.artwork.clone_from(&catalog.artwork);
        }
        if s.now.album.is_empty() {
            s.now.album.clone_from(&catalog.album);
        }
        if s.now.artist_id.is_empty() {
            s.now.artist_id.clone_from(&catalog.artist_id);
        }
        if s.now.album_id.is_empty() {
            s.now.album_id.clone_from(&catalog.album_id);
        }
        if s.now.duration == 0 {
            s.now.duration = catalog.duration;
        }
        if s.duration == 0 {
            s.duration = s.now.duration;
        }
        s.title.clone_from(&s.now.title);
        metadata(&mut s.now, catalog);
        if s.state == "Playing" {
            self.armed = true;
        }
        self.last_position = s.position;
        self.last_duration = s.duration;
        self.publish(s);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn entry(id: char) -> Entry {
        Entry {
            id: format!("spotify:track:{}", id.to_string().repeat(22)),
            kind: "track".into(),
            ..Default::default()
        }
    }
    #[test]
    fn collection_selection_duplicates_and_edits_use_local_transport() {
        let a = entry('a');
        let b = entry('b');
        let c = entry('c');
        let mut selected = a.clone();
        selected.raw["spotify_selection_index"] = serde_json::json!(2);
        let mut queue = LocalQueue::default();
        let mut s = Snapshot::default();
        let mut sent = Vec::new();
        let mut send = |control: &Control| {
            sent.push(control.clone());
            Ok(())
        };
        queue
            .dispatch(
                &Command::PlayEntry(Box::new(selected), vec![a.clone(), b, a.clone(), c.clone()]),
                &mut s,
                &mut send,
            )
            .unwrap();
        assert_eq!(s.queue_index, 2);
        queue
            .dispatch(
                &Command::QueueInsertNext(Box::new(a.clone())),
                &mut s,
                &mut send,
            )
            .unwrap();
        let expected = s.queue.iter().map(|e| e.id.clone()).collect();
        queue
            .dispatch(
                &Command::QueueMove {
                    from: 4,
                    to: 3,
                    expected,
                },
                &mut s,
                &mut send,
            )
            .unwrap();
        assert_eq!(s.queue[3].id, c.id);
        queue
            .dispatch(&Command::QueueClear, &mut s, &mut send)
            .unwrap();
        assert_eq!(s.queue.len(), 1);
        drop(send);
        assert_eq!(
            sent,
            vec![
                Control::Repeat("off".into()),
                Control::Shuffle(false),
                Control::Play(Some(a.id))
            ]
        );
    }
    #[test]
    fn failed_play_does_not_replace_queue() {
        let mut q = LocalQueue::default();
        let mut s = Snapshot::default();
        let result = q.dispatch(
            &Command::PlayEntry(Box::new(entry('a')), vec![]),
            &mut s,
            &mut |_| Err("failed".into()),
        );
        assert!(result.is_err());
        assert!(!q.is_managed());
    }
    #[test]
    fn finish_advances_once_and_pause_does_not_advance() {
        let a = entry('a');
        let b = entry('b');
        let mut q = LocalQueue {
            items: Arc::new(vec![a.clone(), b.clone()]),
            armed: true,
            last_position: 9950,
            last_duration: 10000,
            ..Default::default()
        };
        let mut s = Snapshot {
            now: a,
            state: "Paused".into(),
            position: 9950,
            duration: 10000,
            ..Default::default()
        };
        let mut state = state::Snapshot {
            connected: true,
            logged_in: true,
            is_active: true,
            status: "paused".into(),
            ..Default::default()
        };
        let mut sent = Vec::new();
        let mut send = |c: &Control| {
            sent.push(c.clone());
            Ok(())
        };
        q.observe(&mut s, &state, &mut send);
        assert_eq!(q.index, 0);
        state.status = "idle".into();
        s.state = "Stopped".into();
        q.observe(&mut s, &state, &mut send);
        q.observe(&mut s, &state, &mut send);
        drop(send);
        assert_eq!(sent, vec![Control::Play(Some(b.id))]);
        assert_eq!(q.index, 1);
    }
    #[test]
    fn duplicate_occurrence_requires_new_start_before_confirmation() {
        let a = entry('a');
        let mut q = LocalQueue {
            items: Arc::new(vec![a.clone(), a.clone()]),
            index: 1,
            pending: Some(Instant::now()),
            ..Default::default()
        };
        let mut s = Snapshot {
            now: a,
            state: "Playing".into(),
            position: 9900,
            duration: 10000,
            ..Default::default()
        };
        let state = state::Snapshot {
            connected: true,
            logged_in: true,
            is_active: true,
            status: "playing".into(),
            ..Default::default()
        };
        let mut send = |_: &Control| panic!("old occurrence must not start another item");
        q.observe(&mut s, &state, &mut send);
        assert!(q.pending.is_some());
        s.position = 10;
        q.observe(&mut s, &state, &mut send);
        assert!(q.pending.is_none());
        assert_eq!(s.queue_index, 1);
    }
    #[test]
    fn paused_edits_preserve_playback_and_next_skips_repeat_one() {
        let a = entry('a');
        let b = entry('b');
        let c = entry('c');
        let mut q = LocalQueue {
            items: Arc::new(vec![a.clone(), b.clone()]),
            repeat: 2,
            ..Default::default()
        };
        let mut s = Snapshot {
            now: a,
            state: "Paused".into(),
            position: 4321,
            duration: 10000,
            ..Default::default()
        };
        let mut no_control = |_: &Control| panic!("queue edit must not interrupt paused playback");
        q.dispatch(
            &Command::QueueInsertNext(Box::new(c.clone())),
            &mut s,
            &mut no_control,
        )
        .unwrap();
        q.dispatch(&Command::QueueClear, &mut s, &mut no_control)
            .unwrap();
        assert_eq!((s.state.as_str(), s.position), ("Paused", 4321));
        q.dispatch(
            &Command::QueueAddLast(Box::new(b.clone())),
            &mut s,
            &mut no_control,
        )
        .unwrap();
        let mut sent = Vec::new();
        q.dispatch(&Command::Next, &mut s, &mut |c| {
            sent.push(c.clone());
            Ok(())
        })
        .unwrap();
        assert_eq!(sent, vec![Control::Play(Some(b.id))]);
        assert_eq!(q.index, 1);
    }
    #[test]
    fn repeat_and_shuffle_select_valid_queue_occurrences() {
        let mut q = LocalQueue {
            items: Arc::new(vec![entry('a'), entry('b'), entry('a')]),
            index: 2,
            repeat: 1,
            ..Default::default()
        };
        assert_eq!(q.next(true), Some(0));
        q.repeat = 2;
        assert_eq!(q.next(true), Some(2));
        assert_eq!(q.next(false), None);
        q.repeat = 0;
        q.shuffle = true;
        for _ in 0..20 {
            assert!(q.next(false).is_some_and(|i| i < 2));
        }
    }
    #[test]
    fn exhausted_collection_pauses_native_autoplay_and_retains_queue() {
        let a = entry('a');
        let b = entry('b');
        let mut q = LocalQueue {
            items: Arc::new(vec![a]),
            armed: true,
            last_position: 9950,
            last_duration: 10000,
            ..Default::default()
        };
        let mut s = Snapshot {
            now: b.clone(),
            state: "Playing".into(),
            ..Default::default()
        };
        let mut state = state::Snapshot {
            connected: true,
            logged_in: true,
            is_active: true,
            status: "playing".into(),
            ..Default::default()
        };
        let mut sent = Vec::new();
        q.observe(&mut s, &state, &mut |c| {
            sent.push(c.clone());
            Ok(())
        });
        assert_eq!(sent, vec![Control::Pause]);
        state.status = "paused".into();
        s.state = "Paused".into();
        assert!(q.observe(&mut s, &state, &mut |_| panic!(
            "pause acknowledgment must not send more controls"
        )));
        assert_eq!(s.now.id, b.id);
        assert_eq!(s.queue.len(), 1);
    }
}
