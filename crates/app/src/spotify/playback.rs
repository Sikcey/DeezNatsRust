//! Audio worker adapter: Spotify controls never enter the Deezer ABI or PCM pipeline.
use super::{service::Handle, soloist::Control};
use crate::{
    audio::{Command, Snapshot},
    catalog::Entry,
};
use serde_json::{json, Value};
#[path = "playback_queue.rs"]
mod playback_queue;
pub(crate) use playback_queue::Queue;

pub(crate) fn dispatch(
    command: &Command,
    handle: &Handle,
    snapshot: &mut Snapshot,
    queue: &mut Queue,
) -> Result<(), String> {
    if matches!(
        command,
        Command::PlayEntry(..)
            | Command::PlayQueueIndex(_)
            | Command::QueueAddLast(_)
            | Command::QueueInsertNext(_)
            | Command::QueueClear
            | Command::QueueMove { .. }
    ) {
        // Explicit local play must address the paired Soloist, even when another
        // Connect device was active. Never move playback for a queue-only edit.
        let local = handle
            .snapshot()
            .filter(|s| s.connected && s.logged_in)
            .ok_or("Pair Soloist in Spotify before starting local playback")?;
        if local.is_active {
            // The queue request obtains a fresh active device; validate it there.
            snapshot.now.raw["spotify_device_id"] = Value::Null;
            snapshot.now.raw["spotify_local_device_name"] = json!(local.device_name);
        } else {
            let devices = super::web_devices::devices()?;
            let matches: Vec<_> = devices
                .as_array()
                .into_iter()
                .flatten()
                .filter(|d| {
                    d["name"].as_str() == Some(local.device_name.as_str())
                        && d["is_restricted"] == false
                })
                .collect();
            let target = matches
                .iter()
                .find(|d| d["is_active"] == true)
                .copied()
                .or_else(|| (matches.len() == 1).then(|| matches[0]))
                .ok_or(
                    "Soloist device is unavailable or ambiguous; refresh Spotify Connect devices",
                )?;
            let device = target["id"].as_str().ok_or("Invalid Soloist device")?;
            if target["is_active"] != true {
                if !matches!(command, Command::PlayEntry(..) | Command::PlayQueueIndex(_)) {
                    return Err(
                        "Soloist is no longer the active device; refresh before editing its queue"
                            .into(),
                    );
                }
                super::web::device_command(
                    json!({"action":"transfer","device":device,"playing":false,"confirm":false}),
                )?;
            }
            snapshot.now.raw["spotify_device_id"] = json!(device);
            snapshot.now.raw["spotify_local_device_name"] = Value::Null;
        }
    }
    // Queue edits use native HTTP; Soloist remains the playback device.
    if queue.dispatch(command, snapshot)? {
        return Ok(());
    }
    let control = match command {
        Command::SwitchProvider(_) => return Ok(()),
        Command::Play(uri) => Control::Play(Some(uri.clone())),
        Command::PlayEntry(..) => unreachable!(),
        Command::PlayQueueIndex(_) => unreachable!(),
        Command::Toggle => {
            if snapshot.state == "Playing" {
                Control::Pause
            } else {
                Control::Play(None)
            }
        }
        Command::SetPlaying(true) => Control::Play(None),
        Command::SetPlaying(false) | Command::Stop => Control::Pause,
        Command::Next => Control::Next,
        Command::Previous => Control::Previous,
        Command::Seek(percent) => Control::Seek(
            (snapshot.duration.saturating_mul((*percent).min(100) as u64) / 100)
                .min(u32::MAX as u64) as u32,
        ),
        Command::SeekMs(ms) => Control::Seek(
            (if snapshot.duration > 0 {
                (*ms).min(snapshot.duration)
            } else {
                *ms
            })
            .min(u32::MAX as u64) as u32,
        ),
        Command::Volume(value) => Control::Volume((*value).min(100) as u8),
        Command::Shuffle(enabled) => Control::Shuffle(*enabled),
        Command::Repeat(mode) => Control::Repeat(
            match mode {
                0 => "off",
                1 => "context",
                2 => "track",
                _ => return Err("Invalid repeat mode".into()),
            }
            .into(),
        ),
        Command::QueueAddLast(_)
        | Command::QueueInsertNext(_)
        | Command::QueueClear
        | Command::QueueMove { .. } => unreachable!(),
        Command::Autoplay(_) => {
            return Err(
                "Change Spotify Autoplay in the Spotify app; Soloist has no local control".into(),
            )
        }
        Command::Api(name, args) => return super::catalog_invoke(name, args).map(|_| ()),
        Command::Login(_) | Command::LoginSaved | Command::Logout => {
            return Err("Use Spotify account authorization for this provider".into())
        }
        Command::Discord(_) | Command::Quit => return Ok(()),
    };
    handle.control(&control).map(|_| ())
}

pub(crate) fn update(snapshot: &mut Snapshot, handle: &Handle, managed: &mut Queue) {
    let status = handle.status();
    let Some(state) = handle.snapshot().filter(|_| {
        !matches!(
            status.phase,
            super::service::Phase::Failed | super::service::Phase::Stopped
        )
    }) else {
        snapshot.playback_connected = false;
        snapshot.state = "Stopped".into();
        snapshot.message = status.message;
        return;
    };
    snapshot.playback_connected = state.connected && state.logged_in;
    snapshot.state = if !state.is_active {
        "Stopped"
    } else {
        match state.status.as_str() {
            "playing" => "Playing",
            "paused" => "Paused",
            "loading" | "buffering" => "Loading",
            _ => "Stopped",
        }
    }
    .into();
    snapshot.position = state.position_ms;
    snapshot.duration = state.duration_ms;
    snapshot.shuffle = state.shuffle;
    snapshot.repeat = match state.repeat.as_str() {
        "context" => 1,
        "track" => 2,
        _ => 0,
    };
    snapshot.radio_active = false;
    snapshot.eq = Value::Null;
    snapshot.quality.clear(); // No documented Soloist quality telemetry.
    snapshot.now = entity_entry(&state.item).unwrap_or_default();
    snapshot.title = snapshot.now.title.clone();
    if !managed.observe(snapshot, &state) {
        let mut queue = Vec::with_capacity(state.upcoming.len() + 1);
        if !snapshot.now.id.is_empty() {
            queue.push(snapshot.now.clone());
        }
        queue.extend(
            state
                .upcoming
                .iter()
                .filter_map(|v| entity_entry(&v["item"])),
        );
        replace_queue_if_changed(snapshot, queue);
        snapshot.queue_index = 0;
    }
    // Soloist may omit decorations in an incremental item event. Catalog
    // metadata belongs to the same URI and is a valid duration fallback.
    if snapshot.duration == 0 {
        snapshot.duration = snapshot.now.duration;
    }
    if snapshot.duration > 0 {
        snapshot.position = snapshot.position.min(snapshot.duration);
    }
    if let Some(error) = &state.last_error {
        snapshot.message.clone_from(error);
    }
}
fn replace_queue_if_changed(snapshot: &mut Snapshot, queue: Vec<Entry>) {
    if *snapshot.queue != queue {
        snapshot.queue = std::sync::Arc::new(queue);
    }
}

#[cfg(test)]
mod queue_update_tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn unchanged_queue_keeps_identity_and_changed_queue_is_published() {
        let mut snapshot = Snapshot::default();
        let entry = Entry {
            id: "spotify:track:first".into(),
            ..Entry::default()
        };
        replace_queue_if_changed(&mut snapshot, vec![entry.clone()]);
        let original = snapshot.queue.clone();
        replace_queue_if_changed(&mut snapshot, vec![entry]);
        assert!(Arc::ptr_eq(&original, &snapshot.queue));
        replace_queue_if_changed(&mut snapshot, Vec::new());
        assert!(!Arc::ptr_eq(&original, &snapshot.queue));
        assert!(snapshot.queue.is_empty());
    }
}
fn entity_entry(value: &Value) -> Option<Entry> {
    let uri = value["uri"].as_str()?;
    let kind = value["entity_type"]
        .as_str()
        .or_else(|| uri.split(':').nth(1))?;
    if !matches!(kind, "track" | "episode") || !uri.starts_with(&format!("spotify:{kind}:")) {
        return None;
    }
    let decorations = &value["decorations"];
    let parent = &decorations["parent"]["entity"];
    let creators: Vec<_> = decorations["creators"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|v| &v["entity"])
        .collect();
    let artwork = crate::catalog::largest_artwork(
        decorations["visual_identity"]["cover"]
            .as_array()
            .into_iter()
            .flatten(),
    )
    .unwrap_or_default();
    let mut raw = json!({"id":uri,"uri":uri,"provider":"spotify","type":kind,"name":decorations["identity"]["name"],"is_playable":true});
    raw["album"] = json!({"id":parent["uri"],"uri":parent["uri"],"name":parent["decorations"]["identity"]["name"],"provider":"spotify"});
    raw["artists"] = json!(creators.iter().map(|v|json!({"id":v["uri"],"uri":v["uri"],"name":v["decorations"]["identity"]["name"],"provider":"spotify"})).collect::<Vec<_>>());
    Some(Entry {
        id: uri.into(),
        kind: kind.into(),
        title: decorations["identity"]["name"]
            .as_str()
            .unwrap_or_default()
            .into(),
        subtitle: creators
            .iter()
            .filter_map(|v| v["decorations"]["identity"]["name"].as_str())
            .collect::<Vec<_>>()
            .join(", "),
        artwork: artwork.into(),
        duration: decorations["playback"]["duration_ms"].as_u64().unwrap_or(0),
        album: parent["decorations"]["identity"]["name"]
            .as_str()
            .unwrap_or_default()
            .into(),
        album_id: parent["uri"].as_str().unwrap_or_default().into(),
        artist_id: creators
            .first()
            .and_then(|v| v["uri"].as_str())
            .unwrap_or_default()
            .into(),
        raw,
    })
}
