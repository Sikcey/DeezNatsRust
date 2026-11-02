//! Confirmed Spotify queue replacement. All I/O runs on the host audio worker.
use super::{Command, Entry, Snapshot};
use crate::spotify::{state, web_devices};
use serde_json::{json, Value};
use std::{
    sync::atomic::{AtomicU64, Ordering},
    sync::Arc,
    time::{Duration, Instant},
};

const STATION: &str = "spotify_station";
#[derive(Default)]
pub(crate) struct Queue {
    items: Arc<Vec<Entry>>,
    index: usize,
    window_end: usize,
    confirmed: Option<Instant>,
    awaiting_play: bool,
    refill_at: Option<Instant>,
    device: Option<String>,
    generation: Option<(Arc<AtomicU64>, u64)>,
}
fn playable(entry: &Entry) -> bool {
    let mut parts = entry.id.split(':');
    parts.next() == Some("spotify")
        && matches!(parts.next(), Some("track" | "episode"))
        && parts
            .next()
            .is_some_and(|id| id.len() == 22 && id.bytes().all(|c| c.is_ascii_alphanumeric()))
        && parts.next().is_none()
}
fn selected(entry: &Entry, items: &[Entry]) -> Result<usize, String> {
    if let Some(index) = entry.raw["spotify_selection_index"].as_u64() {
        let index = usize::try_from(index).map_err(|_| "Invalid Spotify selection index")?;
        return items
            .get(index)
            .filter(|item| item.id == entry.id)
            .map(|_| index)
            .ok_or_else(|| {
                "Spotify selection index does not match the supplied collection".into()
            });
    }
    // Exact raw metadata distinguishes repeated occurrences where the caller supplies it.
    items
        .iter()
        .position(|e| e.id == entry.id && e.raw == entry.raw)
        .or_else(|| items.iter().position(|e| e.id == entry.id))
        .ok_or_else(|| "Selected Spotify item is absent from the supplied collection".into())
}
fn body(items: &[Entry], index: usize, position: u64) -> Result<Value, String> {
    if items.is_empty() || index >= items.len() || !items.iter().all(playable) {
        return Err(
            "Spotify queue requires Spotify tracks or episodes and a valid selection".into(),
        );
    }
    // Reject rather than silently truncate a caller's collection.
    if items.len() > 80 {
        return Err("Spotify queue replacement is limited to 80 verifiable items".into());
    }
    Ok(
        json!({"context":{"uri":"spotify:internal:harmony-play-tracks","metadata":{},
        "pages":[{"tracks":items.iter().map(|e|json!({"uri":e.id,"metadata":{}})).collect::<Vec<_>>()}]},
        "play_origin":{"feature_identifier":"harmony"},
        "options":{"license":"","skip_to":{"track_index":index},"seek_to":position,
            "initially_paused":false,"player_options_override":{"shuffling_context":false}}}),
    )
}
fn window_body(items: &[Entry], index: usize, position: u64) -> Result<(Value, usize), String> {
    if items.is_empty()
        || items.len() > 10_000
        || index >= items.len()
        || !items.iter().all(playable)
    {
        return Err("Spotify queue requires valid Spotify items and a selection".into());
    }
    let end = (index + 80).min(items.len());
    Ok((body(&items[index..end], 0, position)?, end))
}
fn station(entry: &Entry) -> Option<&Value> {
    let marker = entry.raw.get(STATION)?;
    let name = marker["name"].as_str()?;
    if !matches!(
        name,
        "DZFlowForConfigJSON" | "SPStyleMixJSON" | "DZTrackMixJSON" | "DZArtistMixJSON"
    ) || !marker["args"].is_array()
    {
        return None;
    }
    Some(marker)
}
fn metadata(target: &mut Entry, original: &Entry) {
    for name in [STATION, crate::style_mix::MARKER, "added_by"] {
        if let Some(value) = original.raw.get(name) {
            target.raw[name] = value.clone();
        }
    }
}
fn queue_matches(value: &Value, items: &[Entry], index: usize) -> bool {
    value["uri"].as_str() == Some(items[index].id.as_str())
        && value["next_tracks"].as_array().is_some_and(|upcoming| {
            upcoming.len() >= (items.len() - index - 1).min(10)
                && upcoming.len() <= items.len() - index - 1
                && upcoming
                    .iter()
                    .zip(&items[index + 1..])
                    .all(|(v, e)| v["uri"].as_str() == Some(e.id.as_str()))
        })
}
impl Queue {
    fn reset(&mut self) {
        let generation = self.generation.clone();
        *self = Self {
            generation,
            ..Default::default()
        };
    }
    pub fn bind_generation(&mut self, live: Arc<AtomicU64>, epoch: u64) {
        self.generation = Some((live, epoch));
    }
    fn check_current(&self) -> Result<(), String> {
        if self
            .generation
            .as_ref()
            .is_some_and(|(live, epoch)| *epoch == 0 || live.load(Ordering::SeqCst) != *epoch)
        {
            return Err("Spotify playback request cancelled after device/provider change".into());
        }
        Ok(())
    }
    pub fn dispatch(&mut self, command: &Command, s: &mut Snapshot) -> Result<bool, String> {
        let (mut items, index, position, paused) = match command {
            Command::PlayEntry(entry, supplied) => {
                let mut items = if supplied.is_empty() {
                    vec![(**entry).clone()]
                } else {
                    supplied.clone()
                };
                let index = selected(entry, &items)?;
                crate::audio::mark_queue_fallback(&mut items);
                (items, index, 0, false)
            }
            Command::PlayQueueIndex(index) => (s.queue.as_ref().clone(), *index, 0, false),
            Command::QueueMove { from, to, expected } => {
                if s.queue.iter().map(|e| &e.id).ne(expected.iter()) {
                    return Err("Queue changed while dragging; try again".into());
                }
                if *from <= s.queue_index
                    || *to <= s.queue_index
                    || *from >= s.queue.len()
                    || *to >= s.queue.len()
                {
                    return Err("Only upcoming Spotify items can be reordered".into());
                }
                let mut items = s.queue.as_ref().clone();
                let item = items.remove(*from);
                items.insert(*to, item);
                (items, s.queue_index, s.position, s.state != "Playing")
            }
            Command::QueueClear => {
                if !playable(&s.now) {
                    return Err(
                        "Spotify has no current item whose upcoming queue can be cleared".into(),
                    );
                }
                (vec![s.now.clone()], 0, s.position, s.state != "Playing")
            }
            Command::QueueInsertNext(entry) | Command::QueueAddLast(entry) => {
                if !playable(entry) {
                    return Err("Only Spotify tracks and episodes can be queued".into());
                }
                let mut items = s.queue.as_ref().clone();
                if items.is_empty() && playable(&s.now) {
                    items.push(s.now.clone());
                }
                if items.is_empty() {
                    return Err("Start Spotify playback before editing its queue".into());
                }
                let at = if matches!(command, Command::QueueInsertNext(_)) {
                    (s.queue_index + 1).min(items.len())
                } else {
                    crate::audio::queue_add_position(&items, s.queue_index)
                };
                items.insert(at, crate::audio::explicit_queue_entry((**entry).clone()));
                (items, s.queue_index, s.position, s.state != "Playing")
            }
            Command::Play(uri) => {
                if !uri.starts_with("spotify:") {
                    return Err("Stale non-Spotify playback request rejected".into());
                }
                self.reset();
                return Ok(false);
            }
            Command::Stop => {
                self.reset();
                return Ok(false);
            }
            _ => return Ok(false),
        };
        if let Command::PlayEntry(entry, _) = command {
            if let Some(marker) = station(entry) {
                for item in &mut items {
                    item.raw[STATION] = marker.clone();
                    metadata(item, entry);
                }
            }
        }
        self.replace(
            s,
            items,
            index,
            position,
            paused,
            !matches!(command, Command::PlayEntry(..) | Command::PlayQueueIndex(_)),
        )?;
        Ok(true)
    }
    fn replace(
        &mut self,
        s: &mut Snapshot,
        items: Vec<Entry>,
        index: usize,
        mut position: u64,
        mut paused: bool,
        preserve: bool,
    ) -> Result<(), String> {
        crate::spotify_diagnostics::record(
            "queue.replace",
            &format!(
                "items={} index={} preserve={}",
                items.len(),
                index,
                preserve
            ),
        );
        window_body(&items, index, position)?;
        self.check_current()?;
        let before = web_devices::playback_target().map_err(|e| {
            crate::spotify_diagnostics::record(
                "queue.target.error",
                crate::spotify_diagnostics::error_kind(&e),
            );
            e
        })?;
        crate::spotify_diagnostics::record("queue.target", "ok");
        if s.now.raw["spotify_local_device_name"]
            .as_str()
            .is_some_and(|name| before["device"]["name"].as_str() != Some(name))
        {
            return Err("Spotify active device changed; refresh before editing playback".into());
        }
        if s.now.raw["spotify_device_id"]
            .as_str()
            .is_some_and(|id| before["device"]["id"].as_str() != Some(id))
        {
            return Err("Spotify active device changed; refresh before editing playback".into());
        }
        if preserve {
            if before["uri"].as_str() != Some(items[index].id.as_str()) {
                return Err(
                    "Spotify current item changed; refresh before editing the queue".into(),
                );
            }
            position = before["position_ms"]
                .as_u64()
                .ok_or("Spotify returned no playback position")?;
            paused = !before["is_playing"]
                .as_bool()
                .ok_or("Spotify returned no playback state")?;
        }
        let (mut request, window_end) = window_body(&items, index, position)?;
        let device = before["device"]["id"]
            .as_str()
            .ok_or("Spotify returned no active device")?
            .to_owned();
        if !preserve {
            request["options"]["initially_paused"] = json!(paused);
            self.check_current()?;
            web_devices::play(request, &device).map_err(|e| {
                crate::spotify_diagnostics::record(
                    "queue.play.error",
                    crate::spotify_diagnostics::error_kind(&e),
                );
                e
            })?;
            crate::spotify_diagnostics::record("queue.play", "acknowledged");
            self.items = Arc::new(items);
            self.device = Some(device.clone());
            self.index = index;
            self.window_end = window_end;
            self.confirmed = Some(Instant::now());
            self.awaiting_play = true;
            s.queue = self.items.clone();
            s.queue_index = index;
            s.now = self.items[index].clone();
            s.position = position;
            s.duration = s.now.duration;
            s.state = if paused { "Paused" } else { "Playing" }.into();
            s.radio_active = station(&s.now).is_some();
            s.message = "Spotify playback started".into();
            return Ok(());
        }
        // SetQueue replaces upcoming tracks without restarting current playback. Revision
        // and device guards reject concurrent changes instead of overwriting another player.
        let mut observed = before;
        let next: Vec<_> = items[index + 1..window_end]
            .iter()
            .map(|e| json!({"uri":e.id,"metadata":{}}))
            .collect();
        self.check_current()?;
        web_devices::set_queue(&observed, &next).map_err(|e| {
            crate::spotify_diagnostics::record(
                "queue.set.error",
                crate::spotify_diagnostics::error_kind(&e),
            );
            e
        })?;
        crate::spotify_diagnostics::record("queue.set", "acknowledged");
        let mut verified = false;
        for _ in 0..5 {
            self.check_current()?;
            observed = web_devices::snapshot()?.ok_or("Spotify active device disappeared")?;
            if observed["device"]["id"].as_str() != Some(&device) {
                return Err("Spotify active device changed during queue replacement".into());
            }
            if queue_matches(&observed, &items[..window_end], index)
                && observed["is_playing"].as_bool() == Some(!paused)
                && observed["position_ms"]
                    .as_u64()
                    .is_some_and(|p| p.abs_diff(position) < 5000)
            {
                verified = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(150));
        }
        if !verified {
            self.items = Arc::default();
            return Err("Spotify did not confirm queue order, position and playback state; refresh the queue (remote playback may have changed)".into());
        }
        self.items = Arc::new(items);
        self.device = Some(device.into());
        self.index = index;
        self.window_end = window_end;
        self.confirmed = Some(Instant::now());
        s.queue = self.items.clone();
        s.queue_index = index;
        s.now = self.items[index].clone();
        s.position = observed["position_ms"].as_u64().unwrap_or(position);
        s.state = if paused { "Paused" } else { "Playing" }.into();
        s.radio_active = station(&s.now).is_some();
        s.message = "Spotify queue confirmed".into();
        Ok(())
    }
    pub fn observe(&mut self, s: &mut Snapshot, state: &state::Snapshot) -> bool {
        if self.items.is_empty() {
            return false;
        }
        let matching = |index: usize| {
            index < self.window_end
                && self.items[index].id == s.now.id
                && state
                    .upcoming
                    .iter()
                    .zip(&self.items[index + 1..self.window_end])
                    .all(|(v, e)| v["item"]["uri"].as_str() == Some(e.id.as_str()))
                && state.upcoming.len().min(10) == (self.window_end - index - 1).min(10)
        };
        let waiting = self.awaiting_play
            && self
                .confirmed
                .is_some_and(|at| at.elapsed() < Duration::from_secs(3));
        let index = (self.index..self.items.len())
            .chain(0..self.index)
            .find(|&i| (!waiting || i == self.index) && matching(i));
        if let Some(index) = index {
            self.awaiting_play = false;
            self.index = index;
            let catalog = &self.items[index];
            if s.now.title.is_empty() {
                s.now.title.clone_from(&catalog.title);
            }
            if s.now.subtitle.is_empty() {
                s.now.subtitle.clone_from(&catalog.subtitle);
            }
            if s.now.artwork.is_empty()
                || (!catalog.artwork.is_empty() && s.now.artwork != catalog.artwork)
            {
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
            s.queue = self.items.clone();
            s.queue_index = index;
            s.radio_active = station(&s.now).is_some();
        } else if self
            .confirmed
            .is_some_and(|at| at.elapsed() < Duration::from_secs(3))
        {
            // Acknowledged REST state can precede the corresponding Soloist events.
            s.queue = self.items.clone();
            s.queue_index = self.index;
            s.now = self.items[self.index].clone();
            s.title = s.now.title.clone();
            s.duration = s.now.duration;
            if waiting {
                s.position = 0;
            }
            if s.state == "Stopped" {
                s.state = "Playing".into();
            }
            s.radio_active = station(&s.now).is_some();
        } else {
            self.items = Arc::default();
            return false;
        }
        true
    }
    pub fn observe_remote(&mut self, s: &mut Snapshot, remote: &Value) {
        if self
            .device
            .as_deref()
            .is_some_and(|id| remote["device"]["id"].as_str() != Some(id))
        {
            self.reset();
            return;
        }
        let upcoming = remote["next_tracks"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|t| json!({"item":{"uri":t["uri"]}}))
            .collect();
        self.observe(
            s,
            &state::Snapshot {
                upcoming,
                ..Default::default()
            },
        );
    }
    pub fn refill(&mut self, s: &mut Snapshot) {
        if self.check_current().is_err() {
            return;
        }
        if self.items.is_empty()
            || s.state != "Playing"
            || s.repeat == 2
            || self.window_end.saturating_sub(self.index + 1) > 2
            || self
                .refill_at
                .is_some_and(|at| at.elapsed() < Duration::from_secs(30))
        {
            return;
        }
        if self.window_end < self.items.len() {
            self.refill_at = Some(Instant::now());
            if let Err(error) = self.replace(
                s,
                self.items.as_ref().clone(),
                self.index,
                s.position,
                s.state != "Playing",
                true,
            ) {
                s.message = format!("Spotify queue continuation: {error}");
            }
            return;
        }
        let Some(marker) = station(&s.now).cloned() else {
            return;
        };
        self.refill_at = Some(Instant::now());
        let result = (|| {
            let value = crate::spotify::catalog_invoke(
                marker["name"].as_str().unwrap(),
                marker["args"].as_array().unwrap(),
            )?;
            let mut tracks = station_tracks(&value)?;
            for track in &mut tracks {
                if let (Some(expected), Some(actual)) = (
                    s.now.raw.get(crate::style_mix::MARKER),
                    track.raw.get(crate::style_mix::MARKER),
                ) {
                    if expected != actual {
                        return Err(
                            "Spotify station refill returned a different style selection".into(),
                        );
                    }
                }
                track.raw[STATION] = marker.clone();
                if let Some(key) = s.now.raw.get(crate::style_mix::MARKER) {
                    track.raw[crate::style_mix::MARKER] = key.clone();
                }
            }
            // Retain current and upcoming occurrences; history cannot grow without bound.
            let mut items = s.queue[s.queue_index..].to_vec();
            if items.len() + tracks.len() > 10_000 {
                return Err("Station refill exceeds the verifiable Spotify queue size".into());
            }
            items.extend(tracks);
            self.replace(s, items, 0, s.position, false, true)
        })();
        if let Err(error) = result {
            s.message = format!("Spotify station refill: {error}");
        }
    }
}
fn station_tracks(value: &Value) -> Result<Vec<Entry>, String> {
    let values = value
        .as_array()
        .or_else(|| value["data"].as_array())
        .or_else(|| value["tracks"].as_array())
        .or_else(|| value["items"].as_array())
        .ok_or("Spotify station returned no track list")?;
    let entries = values
        .iter()
        .map(|v| {
            crate::catalog::entry(v, "track")
                .filter(playable)
                .ok_or_else(|| "Spotify station returned an invalid track".to_owned())
        })
        .collect::<Result<Vec<_>, _>>()?;
    if entries.is_empty() {
        return Err("Spotify station returned no playable tracks".into());
    }
    Ok(entries)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn provider_generation_cancels_between_queue_requests_and_survives_reset() {
        let live = Arc::new(AtomicU64::new(3));
        let mut queue = Queue::default();
        queue.bind_generation(live.clone(), 3);
        assert!(queue.check_current().is_ok());
        queue.reset();
        live.store(4, Ordering::SeqCst);
        assert!(queue.check_current().is_err());
    }
    fn track(id: char) -> Entry {
        Entry {
            id: format!("spotify:track:{}", id.to_string().repeat(22)),
            kind: "track".into(),
            raw: json!({}),
            ..Default::default()
        }
    }
    #[test]
    fn supplied_order_repeats_offset_and_position_are_preserved() {
        let a = track('a');
        let b = track('b');
        let items = vec![a.clone(), b.clone(), a.clone()];
        let wire = body(&items, 2, 12345).unwrap();
        assert_eq!(
            wire["context"]["pages"][0]["tracks"],
            json!([{"uri":a.id,"metadata":{}},{"uri":b.id,"metadata":{}},{"uri":a.id,"metadata":{}}])
        );
        assert_eq!(wire["options"]["skip_to"]["track_index"], 2);
        assert_eq!(wire["options"]["seek_to"], 12345);
        let mut selected = a;
        selected.raw["spotify_selection_index"] = json!(2);
        assert_eq!(super::selected(&selected, &items).unwrap(), 2);
        selected.raw["spotify_selection_index"] = json!(1);
        assert!(super::selected(&selected, &items).is_err());
    }
    #[test]
    fn foreign_identity_bounds_and_empty_lists_fail_before_io() {
        let mut foreign = track('a');
        foreign.id = "12345".into();
        assert!(body(&[foreign], 0, 0).is_err());
        assert!(body(&[], 0, 0).is_err());
        assert!(body(&[track('a')], 1, 0).is_err());
        assert!(body(&vec![track('a'); 81], 0, 0).is_err());
    }
    #[test]
    fn confirmation_rejects_wrong_occurrence_order_or_leftover_manual_queue() {
        let a = track('a');
        let b = track('b');
        let c = track('c');
        let items = vec![a.clone(), b.clone(), a.clone()];
        let mut response = json!({"uri":a.id,"next_tracks":[{"uri":b.id},{"uri":a.id}]});
        assert!(queue_matches(&response, &items, 0));
        response["next_tracks"] = json!([{"uri":a.id},{"uri":b.id}]);
        assert!(!queue_matches(&response, &items, 0));
        response["next_tracks"] = json!([{"uri":b.id},{"uri":a.id},{"uri":c.id}]);
        assert!(!queue_matches(&response, &items, 0));
    }
    #[test]
    fn station_marker_and_attribution_survive_normalization() {
        let mut source = track('a');
        source.raw = json!({"spotify_station":{"name":"SPStyleMixJSON","args":["1:2"]},"style_mix":"1:2","added_by":"Host"});
        let mut telemetry = track('a');
        metadata(&mut telemetry, &source);
        assert_eq!(telemetry.raw, source.raw);
        assert!(station(&telemetry).is_some());
        telemetry.raw[STATION]["name"] = json!("GenericSearch");
        assert!(station(&telemetry).is_none());
    }
    #[test]
    fn long_collections_use_overlapping_windows_without_losing_queue_entries() {
        let items: Vec<_> = (0..170)
            .map(|i| Entry {
                id: format!("spotify:track:{i:022}"),
                kind: "track".into(),
                ..Default::default()
            })
            .collect();
        let (request, end) = window_body(&items, 78, 1234).unwrap();
        assert_eq!(end, 158);
        assert_eq!(
            request["context"]["pages"][0]["tracks"]
                .as_array()
                .unwrap()
                .len(),
            80
        );
        assert_eq!(
            request["context"]["pages"][0]["tracks"][0]["uri"],
            items[78].id
        );
        assert_eq!(request["options"]["skip_to"]["track_index"], 0);
        assert_eq!(request["options"]["seek_to"], 1234);
        let queue = json!({"uri":items[78].id,"next_tracks":items[79..89].iter().map(|e|json!({"uri":e.id})).collect::<Vec<_>>()});
        assert!(queue_matches(&queue, &items[..end], 78));
        let (last, end) = window_body(&items, 158, 0).unwrap();
        assert_eq!(end, 170);
        assert_eq!(
            last["context"]["pages"][0]["tracks"]
                .as_array()
                .unwrap()
                .len(),
            12
        );
        assert_eq!(items.len(), 170);
    }
    #[test]
    fn pending_play_ignores_stale_previous_track_and_position() {
        let a = track('a');
        let b = track('b');
        let mut queue = Queue {
            items: Arc::new(vec![a.clone(), b.clone()]),
            index: 1,
            window_end: 2,
            confirmed: Some(Instant::now()),
            awaiting_play: true,
            ..Default::default()
        };
        let mut snapshot = Snapshot {
            now: a,
            position: 42_000,
            state: "Playing".into(),
            ..Default::default()
        };
        let state = state::Snapshot {
            upcoming: vec![json!({"item":{"uri":b.id}})],
            ..Default::default()
        };
        assert!(queue.observe(&mut snapshot, &state));
        assert_eq!(snapshot.now.id, b.id);
        assert_eq!(snapshot.position, 0);
        assert_eq!(snapshot.queue_index, 1);
        let state = state::Snapshot::default();
        assert!(queue.observe(&mut snapshot, &state));
        assert!(!queue.awaiting_play);
    }

    #[test]
    fn observation_distinguishes_repeated_occurrences_and_retains_markers() {
        let mut a = track('a');
        let b = track('b');
        a.raw[STATION] = json!({"name":"DZTrackMixJSON","args":[a.id]});
        let mut queue = Queue {
            items: Arc::new(vec![a.clone(), b.clone(), a.clone()]),
            window_end: 3,
            ..Default::default()
        };
        let mut snapshot = Snapshot {
            now: track('a'),
            ..Default::default()
        };
        let state = state::Snapshot {
            upcoming: vec![],
            ..Default::default()
        };
        assert!(queue.observe(&mut snapshot, &state));
        assert_eq!(snapshot.queue_index, 2);
        assert_eq!(snapshot.now.raw[STATION], a.raw[STATION]);
        assert!(snapshot.radio_active);
        let published = snapshot.queue.clone();
        assert!(Arc::ptr_eq(&published, &queue.items));
        assert!(queue.observe(&mut snapshot, &state));
        assert!(Arc::ptr_eq(&published, &snapshot.queue));
        snapshot.now = track('c');
        assert!(!queue.observe(&mut snapshot, &state));
        assert!(queue.items.is_empty());
        assert_eq!(published.len(), 3);
    }
}
