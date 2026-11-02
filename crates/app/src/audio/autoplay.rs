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
                            if let Err(error) = self.play_one(dz, s, version, &pending.stamp, item)
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

    fn begin_next(&mut self, dz: &Arc<Engine>, s: &mut Snapshot, version: &mut i64, manual: bool) {
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
