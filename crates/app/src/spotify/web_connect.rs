//! Connect commands over the native authenticated HTTP transport.
use super::*;
const ROOT: &str = "https://spclient.wg.spotify.com/connect-state/v1/";

fn request(
    s: &Value,
    method: &str,
    path: &str,
    body: Option<&Value>,
) -> Result<(u32, Value), String> {
    let bytes = body
        .map(serde_json::to_vec)
        .transpose()
        .map_err(|_| "Invalid Connect command")?
        .unwrap_or_default();
    super::request(
        s,
        method,
        &format!("{ROOT}{path}"),
        &bytes,
        "application/json",
    )
}
type Reply = Result<(u32, Value), String>;
fn read(send: &mut impl FnMut(&str, &str, Option<&Value>) -> Reply) -> Result<Value, String> {
    let (_, mut v) = send("GET", "cluster", None)?;
    if v.is_null() {
        return Ok(json!({"devices":{}}));
    }
    if let Some(cluster) = v.get("cluster") {
        v = cluster.clone();
    }
    if v.get("devices").is_none()
        && v["active_device_id"].is_null()
        && v["timestamp"].is_number()
        && v["player_state"].is_object()
    {
        v["devices"] = json!({});
    }
    if !v["devices"].is_object() {
        return Err("Invalid Spotify Connect cluster".into());
    }
    Ok(v)
}
fn id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 256
        && value
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"_%:-".contains(&c))
}
fn command_id() -> Result<String, String> {
    use sha2::{Digest, Sha256};
    let random = super::super::super::random()?;
    Ok(format!("{:x}", Sha256::digest(random.as_bytes()))[..32].to_owned())
}
fn restriction(state: &Value, key: &str) -> Result<(), String> {
    if state["player_state"]["restrictions"][format!("disallow_{key}_reasons")]
        .as_array()
        .is_some_and(|v| !v.is_empty())
    {
        return Err("Spotify Connect control is restricted".into());
    }
    Ok(())
}
pub(super) fn execute(s: &Value, c: &Value) -> Result<Value, String> {
    execute_with(c, &mut |method, path, body| request(s, method, path, body))
}
fn execute_with(
    c: &Value,
    send: &mut impl FnMut(&str, &str, Option<&Value>) -> Reply,
) -> Result<Value, String> {
    let state = read(send)?;
    let action = c["action"].as_str().ok_or("Missing Connect action")?;
    if action == "state" {
        return Ok(state);
    }
    let target = if action == "transfer" {
        &c["device"]
    } else {
        &state["active_device_id"]
    };
    let target = target
        .as_str()
        .filter(|v| id(v))
        .ok_or("No active Spotify device; select a device first")?;
    let device = state["devices"]
        .get(target)
        .ok_or("Spotify Connect target is unavailable")?;
    if device["capabilities"]["is_controllable"] != true {
        return Err("Spotify device does not allow remote control".into());
    }
    if c["expected"]["device"]
        .as_str()
        .is_some_and(|v| v != target)
    {
        return Err("Spotify active device changed; refresh before controlling playback".into());
    }
    let p = &state["player_state"];
    if c["expected"]["uri"]
        .as_str()
        .is_some_and(|v| p["track"]["uri"].as_str() != Some(v))
    {
        return Err("Spotify current item changed; refresh before editing queue".into());
    }
    // Match the native client protocol: use the observed play origin, with
    // the protocol's `local` origin for a cluster without a play origin.
    // Never depend on a browser registration surviving browser shutdown.
    let sender = p["play_origin"]["device_identifier"]
        .as_str()
        .filter(|v| id(v))
        .unwrap_or("local");
    let (path, method, payload) = if action == "transfer" {
        if device["can_play"] != true
            || device["capabilities"]["hidden"] == true
            || device["is_restricted"] == true
        {
            return Err("Spotify device cannot receive playback".into());
        }
        restriction(&state, "transferring_playback")?;
        (
            "connect/transfer",
            "POST",
            json!({"transfer_options":{"restore_paused":if c["playing"] == true {"resume"} else {"pause"}},"command_id":command_id()?}),
        )
    } else if action == "volume" {
        let value = c["value"]
            .as_u64()
            .filter(|v| *v <= 100)
            .ok_or("Invalid Spotify volume")?;
        if device["capabilities"]["disable_volume"] == true {
            return Err("Spotify volume unavailable".into());
        }
        (
            "connect/volume",
            "PUT",
            json!({"volume":(value * 65535 + 50) / 100}),
        )
    } else {
        let (endpoint, key) = match action {
            "pause" => ("pause", "pausing"),
            "resume" => ("resume", "resuming"),
            "next" => ("skip_next", "skipping_next"),
            "previous" => ("skip_prev", "skipping_prev"),
            "seek" => ("seek_to", "seeking"),
            "play" => ("play", "interrupting_playback"),
            "queue" => ("set_queue", "set_queue"),
            "shuffle" => ("set_options", "toggling_shuffle"),
            "repeat" => ("set_options", "toggling_repeat_context"),
            _ => return Err("Unsupported Spotify Connect control".into()),
        };
        restriction(&state, key)?;
        let mut command = json!({"endpoint":endpoint});
        match action {
            "seek" => {
                command["value"] = json!(c["value"]
                    .as_u64()
                    .filter(|v| *v <= 9_007_199_254_740_991)
                    .ok_or("Invalid Spotify seek")?);
            }
            "play" => {
                let descriptor = c["descriptor"]
                    .as_object()
                    .filter(|d| {
                        d.get("context").is_some_and(Value::is_object)
                            && d.get("options").is_some_and(Value::is_object)
                    })
                    .ok_or("Invalid Spotify play descriptor")?;
                command.as_object_mut().unwrap().extend(descriptor.clone());
                command["endpoint"] = json!(endpoint);
            }
            "queue" => {
                if !c["next_tracks"].is_array()
                    || (!p["prev_tracks"].is_null() && !p["prev_tracks"].is_array())
                {
                    return Err("Spotify queue state unavailable".into());
                }
                if !c["expected"]["revision"].is_null()
                    && c["expected"]["revision"] != p["queue_revision"]
                {
                    return Err("Spotify queue changed; refresh before editing".into());
                }
                command["next_tracks"] = c["next_tracks"].clone();
                command["prev_tracks"] = p
                    .get("prev_tracks")
                    .filter(|v| v.is_array())
                    .cloned()
                    .unwrap_or(json!([]));
                command["queue_revision"] = p["queue_revision"].clone();
            }
            "shuffle" => {
                command["shuffling_context"] = json!(
                    c["value"]
                        .as_u64()
                        .filter(|v| *v <= 1)
                        .ok_or("Invalid shuffle")?
                        == 1
                );
            }
            "repeat" => {
                restriction(&state, "toggling_repeat_track")?;
                let value = c["value"]
                    .as_u64()
                    .filter(|v| *v <= 2)
                    .ok_or("Invalid repeat")?;
                command["repeating_context"] = json!(value == 1);
                command["repeating_track"] = json!(value == 2);
            }
            _ => {}
        }
        ("player/command", "POST", json!({"command":command}))
    };
    let (status, response) = send(
        method,
        &format!("{path}/from/{sender}/to/{target}"),
        Some(&payload),
    )?;
    if !((status == 200 && response["ack_id"].as_str().is_some_and(|v| !v.is_empty()))
        || status == 202)
    {
        return Err("Spotify Connect did not acknowledge command".into());
    }
    if action == "transfer" {
        if c["confirm"].as_bool() == Some(false) {
            return Ok(Value::Null);
        }
        for _ in 0..5 {
            std::thread::sleep(Duration::from_millis(100));
            let after = read(send)?;
            if after["active_device_id"] == target
                && after["player_state"]["is_paused"].as_bool() == Some(c["playing"] != true)
            {
                return Ok(Value::Null);
            }
        }
        return Err(
            "Spotify accepted transfer but target state is unconfirmed; refresh devices".into(),
        );
    }
    Ok(Value::Null)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn connect_ids_accept_the_same_targets_as_the_device_selector() {
        for value in ["speaker", "speaker:room", "device%20name", "room_1-2"] {
            assert!(id(value));
        }
        for value in ["", "../other", "room?query", "room\n"] {
            assert!(!id(value));
        }
    }
    #[test]
    fn playing_transfer_addresses_selected_device_and_confirms_playback() {
        let mut transferred = false;
        execute_with(
            &json!({"action":"transfer","device":"speaker:room","playing":true}),
            &mut |method, path, body| {
                if method == "GET" {
                    let mut state = cluster();
                    state["devices"]["speaker:room"] = state["devices"]["soloist"].clone();
                    state["player_state"]["is_paused"] = json!(false);
                    if transferred {
                        state["active_device_id"] = json!("speaker:room");
                    }
                    return Ok((200, state));
                }
                assert_eq!(path, "connect/transfer/from/origin/to/speaker:room");
                assert_eq!(
                    body.unwrap()["transfer_options"]["restore_paused"],
                    "resume"
                );
                transferred = true;
                Ok((202, Value::Null))
            },
        )
        .unwrap();
        assert!(transferred);
    }
    fn cluster() -> Value {
        json!({"active_device_id":"soloist", "devices":{"soloist":{"can_play":true,"capabilities":{"is_controllable":true}}},
            "player_state":{"track":{"uri":"spotify:track:current"},"queue_revision":"rev1","prev_tracks":[],
                "play_origin":{"device_identifier":"origin"}}})
    }
    #[test]
    fn queue_uses_http_origin_and_preserves_revision_without_browser_sender() {
        let mut writes = 0;
        execute_with(
            &json!({"action":"queue","next_tracks":[{"uri":"spotify:track:next"}],
            "expected":{"device":"soloist","uri":"spotify:track:current","revision":"rev1"}}),
            &mut |method, path, body| {
                if method == "GET" {
                    assert_eq!(path, "cluster");
                    return Ok((200, cluster()));
                }
                writes += 1;
                assert_eq!(path, "player/command/from/origin/to/soloist");
                let command = &body.unwrap()["command"];
                assert_eq!(command["endpoint"], "set_queue");
                assert_eq!(command["queue_revision"], "rev1");
                assert_eq!(command["next_tracks"][0]["uri"], "spotify:track:next");
                assert_eq!(command["prev_tracks"], json!([]));
                Ok((202, Value::Null))
            },
        )
        .unwrap();
        assert_eq!(writes, 1);
    }
    #[test]
    fn stale_device_track_or_queue_never_sends_mutation() {
        for expected in [
            json!({"device":"other"}),
            json!({"uri":"spotify:track:other"}),
            json!({"revision":"rev0"}),
        ] {
            let result = execute_with(
                &json!({"action":"queue","next_tracks":[],"expected":expected}),
                &mut |method, _, _| {
                    assert_eq!(method, "GET", "stale queue must not send a command");
                    Ok((200, cluster()))
                },
            );
            assert!(result.is_err());
        }
    }
    #[test]
    fn idle_transfer_uses_native_local_origin_and_requires_observed_target() {
        let mut calls = 0;
        execute_with(
            &json!({"action":"transfer","device":"soloist","playing":false}),
            &mut |method, path, body| {
                calls += 1;
                if method == "GET" {
                    let mut state = cluster();
                    state["player_state"]["play_origin"] = Value::Null;
                    if calls == 1 {
                        state["active_device_id"] = Value::Null;
                    }
                    state["player_state"]["is_paused"] = json!(true);
                    return Ok((200, state));
                }
                assert_eq!(path, "connect/transfer/from/local/to/soloist");
                assert_eq!(body.unwrap()["transfer_options"]["restore_paused"], "pause");
                Ok((202, Value::Null))
            },
        )
        .unwrap();
        assert!(calls >= 2);
    }
}
