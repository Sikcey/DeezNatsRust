//! Soloist event reduction. State changes come from the engine, never optimistic controls.
use serde::Serialize;
use serde_json::Value;
use std::time::Instant;

#[derive(Clone, Debug, Default, Serialize)]
pub(crate) struct Snapshot {
    pub connected: bool,
    pub logged_in: bool,
    pub is_active: bool,
    pub device_name: String,
    pub status: String,
    pub item: Value,
    pub context: Value,
    pub position_ms: u64,
    pub duration_ms: u64,
    pub volume: u8,
    pub shuffle: bool,
    pub repeat: String,
    pub available_actions: Value,
    pub previous: Vec<Value>,
    pub upcoming: Vec<Value>,
    pub last_error: Option<String>,
    pub revision: u64,
}
pub(crate) struct State {
    snapshot: Snapshot,
    anchor: Instant,
    speed: f64,
}
impl Default for State {
    fn default() -> Self {
        Self {
            snapshot: Snapshot {
                status: "idle".into(),
                repeat: "off".into(),
                ..Snapshot::default()
            },
            anchor: Instant::now(),
            speed: 0.0,
        }
    }
}
impl State {
    pub fn snapshot(&self) -> Snapshot {
        let mut snapshot = self.snapshot.clone();
        snapshot.position_ms = self.position(Instant::now());
        snapshot
    }
    fn position(&self, at: Instant) -> u64 {
        let advance = if self.snapshot.status == "playing" && self.snapshot.connected {
            at.saturating_duration_since(self.anchor).as_secs_f64() * 1000.0 * self.speed
        } else {
            0.0
        };
        let position = self
            .snapshot
            .position_ms
            .saturating_add(advance.max(0.0) as u64);
        if self.snapshot.duration_ms > 0 {
            position.min(self.snapshot.duration_ms)
        } else {
            position
        }
    }
    fn freeze(&mut self) {
        self.snapshot.position_ms = self.position(Instant::now());
        self.anchor = Instant::now();
    }
    pub fn disconnected(&mut self, error: &str) {
        self.freeze();
        self.snapshot.connected = false;
        self.speed = 0.0;
        self.snapshot.is_active = false;
        self.snapshot.last_error = Some(error.into());
        self.snapshot.revision = self.snapshot.revision.wrapping_add(1);
    }
    pub fn apply(&mut self, event: &Value) -> Result<(), String> {
        let kind = event["type"].as_str().ok_or("Soloist event has no type")?;
        match kind {
            "auth_state" => {
                let logged_in = event["logged_in"]
                    .as_bool()
                    .ok_or("Invalid Soloist authentication state")?;
                self.snapshot.connected = true;
                self.snapshot.logged_in = logged_in;
                self.snapshot.is_active = event["is_active"].as_bool().unwrap_or(false);
                self.snapshot.device_name =
                    event["device_name"].as_str().unwrap_or_default().into();
                self.snapshot.last_error = None;
                if !logged_in {
                    let revision = self.snapshot.revision;
                    let connected = self.snapshot.connected;
                    let device_name = self.snapshot.device_name.clone();
                    *self = Self::default();
                    self.snapshot.revision = revision;
                    self.snapshot.connected = connected;
                    self.snapshot.device_name = device_name;
                }
            }
            "playback_state" => {
                validate_status(&event["status"])?;
                validate_position(&event["position"])?;
                let volume = volume(&event["volume"])?;
                self.freeze();
                self.snapshot.connected = true;
                self.snapshot.status = event["status"].as_str().unwrap().into();
                self.set_item(&event["item"]);
                self.snapshot.context = event["context"].clone();
                self.set_position(&event["position"]);
                self.snapshot.volume = volume;
                self.snapshot.is_active = event["is_active"].as_bool().unwrap_or(false);
                self.options(&event["options"]);
                self.snapshot.available_actions = event["available_actions"].clone();
            }
            "track_changed" => {
                self.freeze();
                self.set_item(&event["item"]);
                self.snapshot.position_ms = 0;
                self.speed = 0.0;
            }
            "playback_changed" => {
                validate_status(&event["status"])?;
                self.freeze();
                let status = event["status"].as_str().unwrap();
                if status == "playing" && self.speed == 0.0 {
                    self.speed = 1.0;
                    self.anchor = Instant::now();
                } else if status == "paused" || status == "idle" {
                    self.speed = 0.0;
                }
                self.snapshot.status = status.into();
            }
            "position_sync" => {
                validate_position(&event["position"])?;
                self.set_position(&event["position"]);
            }
            "volume_changed" => self.snapshot.volume = volume(&event["volume"])?,
            "device_changed" => {
                self.snapshot.is_active = event["is_active"]
                    .as_bool()
                    .ok_or("Invalid Soloist device state")?;
                if let Some(name) = event["device_name"].as_str() {
                    self.snapshot.device_name = name.into();
                }
            }
            "context_changed" => self.snapshot.context = event["context"].clone(),
            "options_changed" => self.options(&event["options"]),
            "queue_changed" => {
                let previous = event["previous"]
                    .as_array()
                    .ok_or("Invalid Soloist queue")?;
                let upcoming = event["upcoming"]
                    .as_array()
                    .ok_or("Invalid Soloist queue")?;
                self.snapshot.previous = previous.iter().take(80).cloned().collect();
                self.snapshot.upcoming = upcoming.iter().take(80).cloned().collect();
            }
            "error" => {
                self.snapshot.last_error =
                    Some("Soloist rejected a request; check pairing and available actions".into())
            }
            _ => return Ok(()), // New event kinds and command acknowledgments do not invent playback state.
        }
        self.snapshot.revision = self.snapshot.revision.wrapping_add(1);
        Ok(())
    }
    fn set_item(&mut self, item: &Value) {
        let previous_duration =
            if item["uri"].as_str().is_some() && item["uri"] == self.snapshot.item["uri"] {
                self.snapshot.duration_ms
            } else {
                0
            };
        self.snapshot.item = item.clone();
        self.snapshot.duration_ms = item["decorations"]["playback"]["duration_ms"]
            .as_u64()
            .filter(|duration| *duration > 0)
            .or((previous_duration > 0).then_some(previous_duration))
            .unwrap_or_default();
    }
    fn set_position(&mut self, position: &Value) {
        self.snapshot.position_ms = position["position_ms"].as_u64().unwrap_or_default();
        self.speed = position["speed"].as_f64().unwrap_or_default();
        self.anchor = Instant::now();
    }
    fn options(&mut self, options: &Value) {
        if let Some(shuffle) = options["shuffle"].as_bool() {
            self.snapshot.shuffle = shuffle;
        }
        if let Some(repeat) = options["repeat"]
            .as_str()
            .filter(|s| matches!(*s, "off" | "context" | "track"))
        {
            self.snapshot.repeat = repeat.into();
        }
    }
}
fn validate_status(value: &Value) -> Result<(), String> {
    if value
        .as_str()
        .is_some_and(|s| matches!(s, "idle" | "playing" | "paused" | "buffering"))
    {
        Ok(())
    } else {
        Err("Invalid Soloist playback status".into())
    }
}
fn validate_position(value: &Value) -> Result<(), String> {
    if value["position_ms"].as_u64().is_some()
        && value["speed"]
            .as_f64()
            .is_some_and(|v| v.is_finite() && (0.0..=16.0).contains(&v))
    {
        Ok(())
    } else {
        Err("Invalid Soloist playback position".into())
    }
}
fn volume(value: &Value) -> Result<u8, String> {
    value
        .as_u64()
        .filter(|v| *v <= 100)
        .map(|v| v as u8)
        .ok_or_else(|| "Invalid Soloist volume".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::time::Duration;
    #[test]
    fn sparse_item_updates_keep_duration_only_for_the_same_uri() {
        let mut state = State::default();
        state.set_item(&json!({"uri":"spotify:track:first", "decorations":{"playback":{"duration_ms":210000}}}));
        state.set_item(&json!({"uri":"spotify:track:first"}));
        assert_eq!(state.snapshot().duration_ms, 210000);
        state.set_item(&json!({"uri":"spotify:track:second"}));
        assert_eq!(state.snapshot().duration_ms, 0);
        state.set_item(&Value::Null);
        assert_eq!(state.snapshot().duration_ms, 0);
    }
    #[test]
    fn position_uses_monotonic_time_stops_on_pause_and_clamps_to_duration() {
        let mut state = State::default();
        state
            .apply(&json!({"type":"auth_state","logged_in":true,"is_active":true}))
            .unwrap();
        state.apply(&json!({"type":"playback_state","status":"playing","item":{"decorations":{"playback":{"duration_ms":2000}}},"position":{"position_ms":1000,"timestamp_ms":0,"speed":1.0},"volume":50})).unwrap();
        assert_eq!(state.position(state.anchor + Duration::from_secs(10)), 2000);
        state
            .apply(&json!({"type":"playback_changed","status":"paused"}))
            .unwrap();
        assert_eq!(
            state.position(state.anchor + Duration::from_secs(10)),
            state.snapshot.position_ms
        );
        state.disconnected("lost connection");
        assert!(!state.snapshot().connected);
    }
    #[test]
    fn logout_clears_stale_track_and_queue() {
        let mut state = State::default();
        state
            .apply(&json!({"type":"track_changed","item":{"uri":"spotify:track:old"}}))
            .unwrap();
        state
            .apply(&json!({"type":"queue_changed","previous":[],"upcoming":[{"uid":"next"}]}))
            .unwrap();
        state
            .apply(&json!({"type":"auth_state","logged_in":false,"is_active":false}))
            .unwrap();
        let snapshot = state.snapshot();
        assert!(snapshot.item.is_null());
        assert!(snapshot.upcoming.is_empty());
        assert_eq!(snapshot.position_ms, 0);
    }
    #[test]
    fn ignores_new_events_and_rejects_invalid_values() {
        let mut state = State::default();
        state.apply(&json!({"type":"future_event"})).unwrap();
        assert_eq!(state.snapshot().revision, 0);
        assert!(state
            .apply(&json!({"type":"volume_changed","volume":101}))
            .is_err());
        assert!(state
            .apply(&json!({"type":"position_sync","position":{"position_ms":-1,"speed":1.0}}))
            .is_err());
        state
            .apply(&json!({"type":"command_result","command":"play"}))
            .unwrap();
        assert_eq!(state.snapshot().status, "idle");
    }
    #[test]
    fn malformed_snapshot_does_not_replace_good_state() {
        let mut state = State::default();
        state
            .apply(&json!({"type":"volume_changed","volume":40}))
            .unwrap();
        let before = state.snapshot();
        assert!(state.apply(&json!({"type":"playback_state","status":"playing","position":{"position_ms":12,"speed":1.0},"volume":999})).is_err());
        assert_eq!(state.snapshot().revision, before.revision);
        assert_eq!(state.snapshot().status, before.status);
    }
    #[test]
    fn reconnect_waits_for_a_new_position_anchor() {
        let mut state = State::default();
        state.apply(&json!({"type":"playback_state","status":"playing","position":{"position_ms":1000,"speed":1.0},"volume":50})).unwrap();
        state.disconnected("lost connection");
        let frozen = state.snapshot.position_ms;
        state
            .apply(&json!({"type":"auth_state","logged_in":true,"is_active":true}))
            .unwrap();
        assert_eq!(
            state.position(state.anchor + Duration::from_secs(10)),
            frozen
        );
        state
            .apply(&json!({"type":"position_sync","position":{"position_ms":2000,"speed":1.0}}))
            .unwrap();
        assert_eq!(state.position(state.anchor + Duration::from_secs(1)), 3000);
    }
}
