use crate::{catalog::Entry, engine::Engine};
use image::{codecs::jpeg::JpegEncoder, imageops::FilterType, RgbImage};
use serde_json::{json, Value};
use std::{path::Path, sync::Arc};

pub fn can_edit(entry: &Entry, account: &Value) -> bool {
    if entry.kind != "playlist" {
        return false;
    }
    let account_id = crate::catalog::string(account, &["userId", "id", "uri"]);
    let owner = ["owner", "creator", "user"]
        .iter()
        .find_map(|key| {
            let value = entry.raw.get(*key)?;
            let id = crate::catalog::string(value, &["id", "userId", "uri"]);
            (!id.is_empty()).then_some(id)
        })
        .unwrap_or_else(|| {
            crate::catalog::string(&entry.raw, &["creator_id", "owner_id", "USER_ID"])
        });
    let normalize = |id: String| id.strip_prefix("spotify:user:").unwrap_or(&id).to_owned();
    !account_id.is_empty() && !owner.is_empty() && normalize(account_id) == normalize(owner)
}

#[derive(Clone, Default)]
pub struct Draft {
    pub id: String,
    pub title: String,
    pub description: String,
    pub description_changed: bool,
    pub image: Option<Arc<RgbImage>>,
    pub preview: String,
    pub x: f32,
    pub y: f32,
    pub zoom: f32,
    pub seed: Option<Entry>,
}
impl Draft {
    pub fn from_entry(entry: Option<&Entry>) -> Self {
        Self {
            id: entry.map(|e| e.id.clone()).unwrap_or_default(),
            title: entry.map(|e| e.title.clone()).unwrap_or_default(),
            description: entry
                .map(|e| crate::catalog::string(&e.raw, &["description", "DESCRIPTION"]))
                .unwrap_or_default(),
            preview: entry.map(|e| e.artwork.clone()).unwrap_or_default(),
            x: 0.5,
            y: 0.5,
            zoom: 1.0,
            ..Default::default()
        }
    }
    pub fn choose(&mut self, path: &Path) -> Result<(), String> {
        if std::fs::metadata(path).map_err(|e| e.to_string())?.len() > 20 * 1024 * 1024 {
            return Err("Choose an image smaller than 20 MiB".into());
        }
        let mut reader = image::ImageReader::open(path)
            .map_err(|e| e.to_string())?
            .with_guessed_format()
            .map_err(|e| e.to_string())?;
        let mut limits = image::Limits::default();
        limits.max_image_width = Some(6000);
        limits.max_image_height = Some(6000);
        limits.max_alloc = Some(96 * 1024 * 1024);
        reader.limits(limits);
        let decoded = reader.decode().map_err(|e| e.to_string())?.into_rgb8();
        self.image = Some(Arc::new(decoded));
        self.x = 0.5;
        self.y = 0.5;
        self.zoom = 1.0;
        Ok(())
    }
    pub fn jpeg(&self) -> Result<Vec<u8>, String> {
        let source = self.image.as_ref().ok_or("Choose an image first")?;
        let side =
            ((source.width().min(source.height()) as f32 / self.zoom.clamp(1.0, 4.0)).round()
                as u32)
                .max(1);
        let x = ((source.width() - side) as f32 * self.x.clamp(0.0, 1.0)).round() as u32;
        let y = ((source.height() - side) as f32 * self.y.clamp(0.0, 1.0)).round() as u32;
        let square = image::imageops::crop_imm(source.as_ref(), x, y, side, side).to_image();
        let cover = image::imageops::resize(&square, 500, 500, FilterType::Triangle);
        let mut bytes = Vec::new();
        JpegEncoder::new_with_quality(&mut bytes, 88)
            .encode_image(&cover)
            .map_err(|e| e.to_string())?;
        if bytes.len() > 1024 * 1024 {
            return Err("Cover exceeds Deezer's 1 MiB limit".into());
        }
        Ok(bytes)
    }
}

#[derive(Clone)]
pub enum Mutation {
    Save {
        id: String,
        title: String,
        description: Option<String>,
        cover: Option<Vec<u8>>,
        seed: Option<String>,
    },
    Add(String, String),
    Remove(String, String),
    Delete(String),
}
pub struct Outcome {
    pub id: String,
    pub deleted: bool,
}
fn accepted(value: Value) -> Result<(), String> {
    if value == false || value == 0 || value.is_null() {
        Err("Deezer rejected the playlist change".into())
    } else {
        Ok(())
    }
}
pub fn execute(engine: &Engine, mutation: &mut Mutation) -> Result<Outcome, String> {
    execute_with(mutation, |name, args| engine.invoke(name, args))
}
pub fn execute_spotify(mutation: &mut Mutation) -> Result<Outcome, String> {
    execute_with(mutation, crate::spotify::catalog_invoke)
}
fn execute_with(
    mutation: &mut Mutation,
    mut call: impl FnMut(&str, &[Value]) -> Result<Value, String>,
) -> Result<Outcome, String> {
    let adding = matches!(mutation, Mutation::Add(..));
    match mutation {
        Mutation::Save {
            id,
            title,
            description,
            cover,
            seed,
        } => {
            if title.trim().is_empty() {
                return Err("Enter a playlist name".into());
            }
            if id.is_empty() {
                let value = call("DZCreatePlaylist", &[json!(title.trim())])?;
                *id = crate::catalog::string(&value, &["id"]);
                if id.is_empty() {
                    return Err("Deezer did not return the new playlist ID".into());
                }
            }
            // A creation can succeed before a later step fails; include its ID so
            // the editor retries against it instead of creating duplicate playlists.
            let result = (|| {
                if let Some(description) = description {
                    accepted(call(
                        "DZUpdatePlaylistJSON",
                        &[json!(id), json!(title.trim()), json!(description)],
                    )?)?;
                } else {
                    accepted(call("DZRenamePlaylist", &[json!(id), json!(title.trim())])?)?;
                }
                if let Some(track) = seed.as_ref() {
                    accepted(call("DZAddToPlaylist", &[json!(id), json!(track)])?)?;
                    *seed = None;
                }
                if let Some(bytes) = cover {
                    static NEXT: std::sync::atomic::AtomicU64 =
                        std::sync::atomic::AtomicU64::new(0);
                    let path = std::env::temp_dir().join(format!(
                        "deeznats-cover-{}-{}.jpg",
                        std::process::id(),
                        NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
                    ));
                    std::fs::write(&path, bytes).map_err(|e| e.to_string())?;
                    let result = call(
                        "DZPlaylistCoverJSON",
                        &[json!(id), json!(path.to_string_lossy())],
                    )
                    .and_then(accepted);
                    let _ = std::fs::remove_file(path);
                    result?;
                }
                Ok(())
            })();
            result.map_err(|e: String| {
                format!("Playlist {id}: {e}. Some changes may already be saved.")
            })?;
            Ok(Outcome {
                id: id.clone(),
                deleted: false,
            })
        }
        Mutation::Add(id, track) | Mutation::Remove(id, track) => {
            let name = if adding {
                "DZAddToPlaylist"
            } else {
                "DZRemoveFromPlaylist"
            };
            accepted(call(name, &[json!(id), json!(track)])?)?;
            Ok(Outcome {
                id: id.clone(),
                deleted: false,
            })
        }
        Mutation::Delete(id) => {
            accepted(call("DZDeletePlaylist", &[json!(id)])?)?;
            Ok(Outcome {
                id: id.clone(),
                deleted: true,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn edit_requires_matching_playlist_owner() {
        let mut entry = super::Entry {
            kind: "playlist".into(),
            raw: serde_json::json!({"creator":{"id":42}}),
            ..Default::default()
        };
        assert!(super::can_edit(&entry, &serde_json::json!({"userId":"42"})));
        assert!(!super::can_edit(
            &entry,
            &serde_json::json!({"userId":"43"})
        ));
        entry.raw = serde_json::json!({"owner":{"uri":"spotify:user:alice"}});
        assert!(super::can_edit(&entry, &serde_json::json!({"id":"alice"})));
        assert!(!super::can_edit(&entry, &serde_json::json!({})));
        entry.raw = serde_json::json!({});
        assert!(!super::can_edit(&entry, &serde_json::json!({"id":"alice"})));
    }
    use super::*;

    #[test]
    fn rename_without_description_edit_preserves_unknown_private_metadata() {
        let mut change = Mutation::Save {
            id: "42".into(),
            title: "Renamed".into(),
            description: None,
            cover: None,
            seed: None,
        };
        let mut calls = Vec::new();
        execute_with(&mut change, |name, args| {
            calls.push((name.to_string(), args.to_vec()));
            Ok(json!(1))
        })
        .unwrap();
        assert_eq!(
            calls,
            vec![(
                "DZRenamePlaylist".into(),
                vec![json!("42"), json!("Renamed")]
            )]
        );
    }

    #[test]
    fn existing_playlist_updates_details_without_creating_another() {
        let mut change = Mutation::Save {
            id: "42".into(),
            title: " Renamed ".into(),
            description: Some("Description".into()),
            cover: None,
            seed: None,
        };
        let mut calls = Vec::new();
        let result = execute_with(&mut change, |name, args| {
            calls.push((name.to_string(), args.to_vec()));
            Ok(json!(true))
        })
        .unwrap();
        assert_eq!(result.id, "42");
        assert_eq!(
            calls,
            vec![(
                "DZUpdatePlaylistJSON".into(),
                vec![json!("42"), json!("Renamed"), json!("Description")]
            )]
        );
    }

    #[test]
    fn partial_creation_failure_retains_id_and_retry_skips_completed_track_add() {
        let mut change = Mutation::Save {
            id: String::new(),
            title: "New".into(),
            description: Some(String::new()),
            cover: Some(vec![1, 2]),
            seed: Some("song".into()),
        };
        let error = execute_with(&mut change, |name, args| {
            if name == "DZCreatePlaylist" {
                return Ok(json!({"id":"123"}));
            }
            if name == "DZPlaylistCoverJSON" {
                let path = args[1].as_str().unwrap();
                assert_eq!(std::fs::read(path).unwrap(), [1, 2]);
                return Err("Upload failed".into());
            }
            Ok(json!(true))
        })
        .err()
        .unwrap();
        assert!(error.contains("123") && error.contains("Upload failed"));
        let mut calls = Vec::new();
        execute_with(&mut change, |name, _| {
            calls.push(name.to_string());
            Ok(json!(true))
        })
        .unwrap();
        assert_eq!(calls, ["DZUpdatePlaylistJSON", "DZPlaylistCoverJSON"]);
    }

    #[test]
    fn membership_and_deletion_surface_service_rejection() {
        for mut mutation in [
            Mutation::Add("1".into(), "2".into()),
            Mutation::Remove("1".into(), "2".into()),
            Mutation::Delete("1".into()),
        ] {
            assert!(execute_with(&mut mutation, |_, _| Ok(json!(0))).is_err());
            assert!(
                execute_with(&mut mutation, |_, _| Err("Permission denied".into()))
                    .err()
                    .unwrap()
                    .contains("Permission denied")
            );
        }
    }

    #[test]
    fn cover_crop_pan_zoom_and_jpeg_are_real_and_bounded() {
        let source = RgbImage::from_fn(300, 100, |x, _| {
            if x < 100 {
                image::Rgb([255, 0, 0])
            } else if x < 200 {
                image::Rgb([0, 255, 0])
            } else {
                image::Rgb([0, 0, 255])
            }
        });
        let mut draft = Draft {
            image: Some(Arc::new(source)),
            x: 0.5,
            y: 0.5,
            zoom: 1.0,
            ..Default::default()
        };
        for (x, channel) in [(0.0, 0), (0.5, 1), (1.0, 2)] {
            draft.x = x;
            for zoom in [1.0, 4.0] {
                draft.zoom = zoom;
                let bytes = draft.jpeg().unwrap();
                assert!(bytes.len() < 1024 * 1024);
                let image = image::load_from_memory(&bytes).unwrap().into_rgb8();
                assert_eq!(image.dimensions(), (500, 500));
                assert!(image.get_pixel(250, 250)[channel] > 240);
            }
        }
        assert!(Draft::default().jpeg().is_err());
    }
}
