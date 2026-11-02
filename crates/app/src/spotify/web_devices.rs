//! Private Web Player Connect bridge. No OAuth fallback while a web account is active.
use serde_json::{json, Value};

fn state() -> Result<Value, String> {
    if !super::web::has_session() {
        return Err("Spotify Web Player session required for remote control".into());
    }
    super::web::device_command(json!({"action":"state"}))
}

pub(crate) fn devices() -> Result<Value, String> {
    normalize_devices(&state()?)
}

fn normalize_devices(cluster: &Value) -> Result<Value, String> {
    let devices = cluster["devices"]
        .as_object()
        .ok_or("Invalid Spotify Connect device state")?;
    let active = cluster["active_device_id"].as_str();
    let mut result = Vec::new();
    for (key, d) in devices {
        // Our hidden WebView runs the catalog/Connect transport, not local audio.
        if cluster["deeznats_web_device_id"].as_str() == Some(key.as_str()) {
            continue;
        }
        let id = d["device_id"]
            .as_str()
            .filter(|id| *id == key && valid_id(id));
        let cap = &d["capabilities"];
        // Observers are not playback targets. Missing capabilities stay restricted.
        if cap["hidden"].as_bool() == Some(true) || cap["can_be_player"].as_bool() == Some(false) {
            continue;
        }
        let restricted = id.is_none()
            || d["can_play"].as_bool() != Some(true)
            || cap["is_controllable"].as_bool() != Some(true)
            || d["is_restricted"].as_bool() == Some(true);
        let volume = if cap["disable_volume"].as_bool() == Some(true) {
            None
        } else {
            d["volume"]
                .as_f64()
                .filter(|n| n.is_finite() && (0.0..=65535.0).contains(n))
                .map(|n| (n * 100.0 / 65535.0).round() as u64)
        };
        result.push(json!({"id":id,"name":d["name"].as_str().unwrap_or("Spotify device"),
            "type":d["device_type"].as_str().unwrap_or("unknown").to_lowercase(),
            "is_active":id.is_some() && id==active,"is_restricted":restricted,"volume_percent":volume}));
    }
    Ok(Value::Array(result))
}

fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 256
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_%:-".contains(&b))
}

pub(crate) fn transfer(device: &str, playing: bool) -> Result<Value, String> {
    if !valid_id(device) {
        return Err("Invalid Spotify device".into());
    }
    if !super::web::has_session() {
        return Err("Spotify Web Player session required".into());
    }
    // The app may still show stopped/local playback while another Connect
    // device is playing. Preserve the observed session rather than pausing it
    // as a side effect of selecting a different output.
    let playing = transfer_playing(&state()?, playing);
    let result =
        super::web::device_command(json!({"action":"transfer","device":device,"playing":playing}))?;
    super::remote::select_device(Some(device.into()));
    Ok(result)
}

fn transfer_playing(cluster: &Value, fallback: bool) -> bool {
    cluster["player_state"]["is_paused"]
        .as_bool()
        .map(|paused| !paused)
        .unwrap_or(fallback)
}

#[cfg(test)]
mod transfer_tests {
    use super::*;
    #[test]
    fn transfer_preserves_live_playback_instead_of_stale_local_state() {
        assert!(transfer_playing(
            &json!({"player_state":{"is_paused":false}}),
            false
        ));
        assert!(!transfer_playing(
            &json!({"player_state":{"is_paused":true}}),
            true
        ));
        assert!(transfer_playing(&json!({}), true));
        assert!(!transfer_playing(&json!({}), false));
    }
}

/// Fresh, global playback state, independent of the local Soloist instance.
/// `None` means no observable active device/track, never a synthetic local state.
/// Position is the server's sampled position; callers may advance it from timestamp.
pub(crate) fn snapshot() -> Result<Option<Value>, String> {
    normalize_snapshot(&state()?)
}
pub(crate) fn playback_target() -> Result<Value, String> {
    let cluster = state()?;
    if cluster["active_device_id"]
        .as_str()
        .is_some_and(|id| cluster["deeznats_web_device_id"].as_str() == Some(id))
    {
        return Err(
            "Pair DeezNats in Spotify for local audio, or select another Spotify Connect device"
                .into(),
        );
    }
    if let Some(snapshot) = normalize_snapshot(&cluster)? {
        return Ok(snapshot);
    }
    let devices = normalize_devices(&cluster)?;
    let device = devices
        .as_array()
        .and_then(|d| {
            d.iter()
                .find(|d| d["is_active"] == true && d["is_restricted"] == false)
        })
        .ok_or("No active Spotify playback device; select a device first")?;
    Ok(json!({"device":device,"uri":"","is_playing":false,"position_ms":0}))
}

fn normalize_snapshot(cluster: &Value) -> Result<Option<Value>, String> {
    let devices = normalize_devices(cluster)?;
    let Some(device) = devices
        .as_array()
        .and_then(|ds| ds.iter().find(|d| d["is_active"] == true))
    else {
        return Ok(None);
    };
    let id = device["id"]
        .as_str()
        .ok_or("Invalid Spotify active device")?;
    if cluster["devices"][id]["capabilities"]["is_observable"].as_bool() != Some(true) {
        return Ok(None);
    }
    let p = &cluster["player_state"];
    let Some(uri) = p["track"]["uri"]
        .as_str()
        .filter(|s| s.starts_with("spotify:track:") || s.starts_with("spotify:episode:"))
    else {
        return Ok(None);
    };
    let Some(paused) = p["is_paused"].as_bool() else {
        return Ok(None);
    };
    let number = |key: &str| {
        p[key]
            .as_u64()
            .or_else(|| p[key].as_str()?.parse::<u64>().ok())
    };
    Ok(Some(json!({"device":device,"uri":uri,"is_playing":!paused,
        "position_ms":number("position_as_of_timestamp").or_else(||number("position")),
        "timestamp_ms":number("timestamp"),"duration_ms":number("duration"),
        "restrictions":p["restrictions"],"options":p["options"],"next_tracks":p["next_tracks"],
        "prev_tracks":p["prev_tracks"],"queue_revision":p["queue_revision"],"track_uid":p["track"]["uid"],
        "context_uri":p["context_uri"]})))
}

/// Controls the freshly observed global active device, never a cached transfer target.
/// Supported actions: pause/resume/next/previous (None), seek (milliseconds),
/// volume (integer 0..100). Success is command acceptance; poll snapshot for state.
pub(crate) fn control(action: &str, value: Option<u64>) -> Result<(), String> {
    control_on(action, value, None)
}
pub(crate) fn control_on(
    action: &str,
    value: Option<u64>,
    device: Option<&str>,
) -> Result<(), String> {
    let valid = match action {
        "pause" | "resume" | "next" | "previous" => value.is_none(),
        "seek" => value.is_some_and(|v| v <= 9_007_199_254_740_991),
        "volume" => value.is_some_and(|v| v <= 100),
        "shuffle" => value.is_some_and(|v| v <= 1),
        "repeat" => value.is_some_and(|v| v <= 2),
        _ => false,
    };
    if !valid {
        return Err("Invalid Spotify remote control".into());
    }
    if !super::web::has_session() {
        return Err("Spotify Web Player session required".into());
    }
    super::web::device_command(json!({"action":action,"value":value,"expected":{"device":device}}))
        .map(|_| ())
}

// Source: current-vendor~web-player.19d7605d.js Controller.playURI/playTracks,
// _generatePlayOptions, ConnectAPIClient.play/setQueue/_sendPlayerCommand.
pub(crate) fn play(descriptor: Value, device: &str) -> Result<(), String> {
    super::web::device_command(json!({"action":"play","descriptor":descriptor,
        "expected":{"device":device}}))
    .map(|_| ())
}
pub(crate) fn play_context(uri: &str, device: &str) -> Result<(), String> {
    let mut parts = uri.split(':');
    if parts.next() != Some("spotify")
        || !matches!(
            parts.next(),
            Some("track" | "episode" | "album" | "playlist" | "artist" | "show")
        )
        || !parts
            .next()
            .is_some_and(|s| s.len() == 22 && s.bytes().all(|b| b.is_ascii_alphanumeric()))
        || parts.next().is_some()
    {
        return Err("Invalid Spotify playback URI".into());
    }
    play(
        json!({"context":{"uri":uri,"url":format!("context://{uri}"),"metadata":{}},
        "play_origin":{"feature_identifier":"harmony"},"options":{"license":""}}),
        device,
    )
}
pub(crate) fn set_queue(before: &Value, next: &[Value]) -> Result<(), String> {
    super::web::device_command(json!({"action":"queue","next_tracks":next,
        "expected":{"device":before["device"]["id"],"uri":before["uri"],"revision":before["queue_revision"]}})).map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> Value {
        json!({"active_device_id":"speaker","devices":{
            "observer":{"device_id":"observer","capabilities":{"hidden":true}},
            "speaker":{"device_id":"speaker","name":"Room","device_type":"SPEAKER","can_play":true,"volume":32768,
                "capabilities":{"is_controllable":true,"is_observable":true}},
            "missing":{"name":"Unselectable"}},
            "player_state":{"track":{"uri":"spotify:track:fixture"},"is_paused":false,"position_as_of_timestamp":"1234","timestamp":"5678","duration":"9000"}})
    }
    #[test]
    fn cluster_devices_are_minimal_and_observers_hidden() {
        let v = normalize_devices(&fixture()).unwrap();
        let ds = v.as_array().unwrap();
        assert_eq!(ds.len(), 2);
        let active = ds.iter().find(|d| d["is_active"] == true).unwrap();
        assert_eq!(active["volume_percent"], 50);
        assert_eq!(active.as_object().unwrap().len(), 6);
        assert!(ds
            .iter()
            .any(|d| d["id"].is_null() && d["is_restricted"] == true));
    }
    #[test]
    fn hidden_catalog_browser_is_never_an_audio_target_or_playback_snapshot() {
        let mut cluster = fixture();
        cluster["deeznats_web_device_id"] = json!("speaker");
        let devices = normalize_devices(&cluster).unwrap();
        assert!(!devices
            .as_array()
            .unwrap()
            .iter()
            .any(|device| device["id"] == "speaker"));
        assert!(normalize_snapshot(&cluster).unwrap().is_none());
    }
    #[test]
    fn global_snapshot_requires_observed_active_device_and_track() {
        let mut c = fixture();
        let s = normalize_snapshot(&c).unwrap().unwrap();
        assert_eq!(s["position_ms"], 1234);
        assert_eq!(s["is_playing"], true);
        c["active_device_id"] = json!("absent");
        assert!(normalize_snapshot(&c).unwrap().is_none());
        c["active_device_id"] = json!("speaker");
        c["devices"]["speaker"]["capabilities"]["is_observable"] = json!(false);
        assert!(normalize_snapshot(&c).unwrap().is_none());
    }
    #[test]
    fn guards_and_disabled_volume() {
        for id in ["", "a/b", "a?b", "a\nb", "@local"] {
            assert!(!valid_id(id));
        }
        let mut c = fixture();
        c["devices"]["speaker"]["capabilities"]["disable_volume"] = json!(true);
        assert!(normalize_devices(&c)
            .unwrap()
            .as_array()
            .unwrap()
            .iter()
            .find(|d| d["is_active"] == true)
            .unwrap()["volume_percent"]
            .is_null());
        assert!(normalize_devices(&json!({})).is_err());
        assert!(control("logout", None).is_err());
        assert!(control("volume", Some(101)).is_err());
    }
}
