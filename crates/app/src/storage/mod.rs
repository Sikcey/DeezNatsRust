//! The small location manifest stays in LocalAppData; application data may live elsewhere.
use std::{
    fs,
    path::{Path, PathBuf},
};

#[derive(Default, serde::Deserialize)]
#[serde(default)]
struct Location {
    active: Option<PathBuf>,
    pending: Option<PathBuf>,
    downloads: Option<PathBuf>,
}

fn base() -> Result<PathBuf, String> {
    std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .ok_or("User data directory unavailable".into())
}
fn manifest(base: &Path) -> PathBuf {
    base.join("DeezNatsRust/storage-location.json")
}
fn read(base: &Path) -> Result<Location, String> {
    let primary = manifest(base);
    let backup = primary.with_extension("json.bak");
    let bytes = match fs::read(&primary) {
        Ok(bytes) if serde_json::from_slice::<Location>(&bytes).is_ok() => Ok(bytes),
        Ok(_) => fs::read(&backup)
            .map_err(|e| format!("Invalid storage location; backup unavailable: {e}")),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => match fs::read(&backup) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Location::default()),
            result => result.map_err(|e| e.to_string()),
        },
        Err(e) => return Err(e.to_string()),
    }?;
    serde_json::from_slice(&bytes).map_err(|e| format!("Invalid storage location: {e}"))
}
fn write(base: &Path, location: &Location) -> Result<(), String> {
    fs::create_dir_all(base.join("DeezNatsRust")).map_err(|e| e.to_string())?;
    let path = manifest(base);
    let temporary = path.with_extension("json.tmp");
    fs::write(&temporary, serde_json::to_vec(&serde_json::json!({"active":location.active,"pending":location.pending,"downloads":location.downloads})).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    fs::rename(temporary, &path).map_err(|e| e.to_string())?;
    let backup = path.with_extension("json.bak");
    let backup_temporary = path.with_extension("json.bak.tmp");
    fs::copy(&path, &backup_temporary).map_err(|e| e.to_string())?;
    fs::rename(backup_temporary, backup).map_err(|e| e.to_string())
}
pub fn data_dir_at(base: &Path) -> Result<PathBuf, String> {
    match read(base)?.active {
        Some(path) if !path.is_absolute() || !path.is_dir() => Err(format!(
            "The configured data folder is unavailable: {}. Reconnect its drive and try again.",
            path.display()
        )),
        Some(path) => Ok(path),
        None => Ok(base.join("DeezNatsRust")),
    }
}
pub fn data_dir() -> Result<PathBuf, String> {
    data_dir_at(&base()?)
}
pub fn pending_dir() -> Result<Option<PathBuf>, String> {
    Ok(read(&base()?)?.pending)
}

pub fn choose(parent: &Path) -> Result<PathBuf, String> {
    let base = base()?;
    let target = schedule(&base, &parent.join("DeezNatsRust"))?;
    let source = data_dir_at(&base)?;
    let configured = std::env::var_os("OPENDEEZER_DOWNLOAD_DIR")
        .map(PathBuf::from)
        .or_else(|| {
            fs::read_to_string(source.join("download-dir.txt"))
                .ok()
                .filter(|s| !s.trim().is_empty())
                .map(|s| PathBuf::from(s.trim()))
        })
        .or_else(|| {
            std::env::var_os("USERPROFILE").map(|home| {
                let home = PathBuf::from(home);
                home.join(if home.join("Music").is_dir() {
                    "Music"
                } else {
                    "Downloads"
                })
                .join("OpenDeezer")
            })
        });
    let mut location = read(&base)?;
    location.downloads = configured.filter(|path| path.is_dir());
    write(&base, &location)?;
    Ok(target)
}
fn schedule(base: &Path, target: &Path) -> Result<PathBuf, String> {
    let source = data_dir_at(base)?;
    fs::create_dir_all(&source).map_err(|e| e.to_string())?;
    fs::create_dir_all(target).map_err(|e| e.to_string())?;
    let source = fs::canonicalize(source).map_err(|e| e.to_string())?;
    let target = fs::canonicalize(target).map_err(|e| e.to_string())?;
    if target == source {
        return Err("This data folder is already in use.".into());
    }
    if target.starts_with(&source) || source.starts_with(&target) {
        return Err("Choose a folder outside the current data folder.".into());
    }
    if fs::read_dir(&target)
        .map_err(|e| e.to_string())?
        .next()
        .is_some()
    {
        return Err("The destination data folder must be empty.".into());
    }
    // Check access now, before recording a pending change.
    let probe = target.join(".deeznats-write-test");
    fs::write(&probe, []).map_err(|e| e.to_string())?;
    fs::remove_file(probe).map_err(|e| e.to_string())?;
    let mut location = read(base)?;
    location.pending = Some(target.clone());
    location.downloads = None;
    write(base, &location)?;
    Ok(target)
}
pub fn cancel() -> Result<(), String> {
    let base = base()?;
    let mut location = read(&base)?;
    location.pending = None;
    location.downloads = None;
    write(&base, &location)
}
pub fn initialize() -> Result<(), String> {
    initialize_at(&base()?)
}
fn initialize_at(base: &Path) -> Result<(), String> {
    let mut location = read(base)?;
    let Some(target) = location.pending.clone() else {
        if location.active.as_ref().is_some_and(|path| !path.is_dir()) {
            return Err(
                "The configured data folder is unavailable. Reconnect its drive and try again."
                    .into(),
            );
        }
        if location.active.is_some() {
            write(base, &location)?;
        }
        return Ok(());
    };
    let source = location
        .active
        .clone()
        .unwrap_or_else(|| base.join("DeezNatsRust"));
    let source = fs::canonicalize(source).map_err(|e| e.to_string())?;
    let resolved_target = fs::canonicalize(&target).map_err(|e| e.to_string())?;
    if resolved_target != target || source.starts_with(&target) || target.starts_with(&source) {
        return Err("The destination data folder changed; relocation stopped.".into());
    }
    let marker = target.join(".deeznats-migration.json");
    if marker.exists() {
        let expected = serde_json::to_vec(&source).map_err(|e| e.to_string())?;
        if fs::read(&marker).map_err(|e| e.to_string())? != expected {
            return Err("The destination belongs to a different data migration.".into());
        }
    } else {
        if fs::read_dir(&target)
            .map_err(|e| e.to_string())?
            .next()
            .is_some()
        {
            return Err("The destination data folder must be empty.".into());
        }
        fs::write(
            &marker,
            serde_json::to_vec(&source).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
    }
    // No engine, browser profile or download worker has started yet. Copy first;
    // commit the manifest last. An interrupted copy can safely resume next launch.
    let downloads_target = target.join("downloads");
    let extra = location
        .downloads
        .as_deref()
        .map(|old| (old, downloads_target.as_path()));
    copy_tree(&source, &target, &source, &target, extra)?;
    if let Some(downloads) = &location.downloads {
        let original = fs::canonicalize(downloads).map_err(|e| e.to_string())?;
        if !original.starts_with(&source) {
            if original.starts_with(&target) || target.starts_with(&original) {
                return Err("Download folder overlaps the new data folder.".into());
            }
            copy_tree(&original, &downloads_target, &source, &target, extra)?;
        } else if original != source.join("downloads") {
            copy_tree(&original, &downloads_target, &source, &target, extra)?;
        }
    }
    fs::create_dir_all(&downloads_target).map_err(|e| e.to_string())?;
    fs::write(
        target.join("download-dir.txt"),
        downloads_target.to_string_lossy().as_bytes(),
    )
    .map_err(|e| e.to_string())?;
    location.active = Some(target);
    location.pending = None;
    location.downloads = None;
    write(base, &location)?;
    let _ = fs::remove_file(marker);
    Ok(())
}
fn relocate_paths(value: &mut serde_json::Value, source: &Path, target: &Path) {
    match value {
        serde_json::Value::String(text) => {
            if let Ok(relative) = Path::new(text.as_str()).strip_prefix(source) {
                *text = target.join(relative).to_string_lossy().into_owned();
            }
        }
        serde_json::Value::Array(values) => {
            for v in values {
                relocate_paths(v, source, target);
            }
        }
        serde_json::Value::Object(values) => {
            for v in values.values_mut() {
                relocate_paths(v, source, target);
            }
        }
        _ => {}
    }
}
fn copy_tree(
    source: &Path,
    target: &Path,
    root: &Path,
    destination: &Path,
    extra: Option<(&Path, &Path)>,
) -> Result<(), String> {
    fs::create_dir_all(target).map_err(|e| e.to_string())?;
    for entry in fs::read_dir(source).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let name = entry.file_name();
        if source == root
            && (name == "storage-location.json"
                || name == "storage-location.json.tmp"
                || name == "storage-location.json.bak"
                || name == "storage-location.json.bak.tmp"
                || name == ".deeznats-migration.json"
                || name == "youtube-lyrics.dat"
                || name.to_string_lossy().starts_with("youtube-lyrics.")
                || name == "page-cache")
        {
            continue;
        }
        let path = entry.path();
        let output = target.join(name);
        let kind = entry.file_type().map_err(|e| e.to_string())?;
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            if fs::symlink_metadata(&path)
                .map_err(|e| e.to_string())?
                .file_attributes()
                & 0x400
                != 0
            {
                return Err("Data folder contains a reparse point; relocation stopped.".into());
            }
        }
        if kind.is_symlink() {
            return Err("Data folder contains a symbolic link; relocation stopped.".into());
        }
        if kind.is_dir() {
            copy_tree(&path, &output, root, destination, extra)?;
        } else if kind.is_file() {
            let temporary = output.with_file_name(format!(
                "{}.deeznats-copy",
                output.file_name().unwrap().to_string_lossy()
            ));
            fs::copy(&path, &temporary)
                .map_err(|e| format!("Could not copy {}: {e}", path.display()))?;
            if path.extension().is_some_and(|ext| ext == "json") {
                let bytes = fs::read(&temporary).map_err(|e| e.to_string())?;
                if let Ok(mut value) = serde_json::from_slice::<serde_json::Value>(&bytes) {
                    relocate_paths(&mut value, root, destination);
                    // canonicalize adds the Windows extended-path prefix; old files
                    // generally contain normal drive-letter paths.
                    let plain_root =
                        PathBuf::from(root.to_string_lossy().trim_start_matches(r"\\?\"));
                    relocate_paths(&mut value, &plain_root, destination);
                    if let Some((old, new)) = extra {
                        relocate_paths(&mut value, old, new);
                    }
                    fs::write(
                        &temporary,
                        serde_json::to_vec(&value).map_err(|e| e.to_string())?,
                    )
                    .map_err(|e| e.to_string())?;
                }
            }
            fs::rename(temporary, output).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn relocated_data_is_found_after_primary_pointer_is_lost_or_corrupted() {
        let base = fixture();
        let target = base.join("external/data");
        fs::create_dir_all(&target).unwrap();
        write(
            &base,
            &Location {
                active: Some(target.clone()),
                ..Default::default()
            },
        )
        .unwrap();
        fs::remove_file(manifest(&base)).unwrap();
        assert_eq!(data_dir_at(&base).unwrap(), target);
        initialize_at(&base).unwrap();
        assert!(manifest(&base).is_file());
        fs::write(manifest(&base), b"broken JSON").unwrap();
        assert_eq!(data_dir_at(&base).unwrap(), target);
        initialize_at(&base).unwrap();
        assert_eq!(data_dir_at(&base).unwrap(), target);
        fs::write(manifest(&base), br#"{"active":42}"#).unwrap();
        assert_eq!(data_dir_at(&base).unwrap(), target);
        fs::remove_dir_all(base).unwrap();
    }
    #[test]
    fn existing_redirect_gains_a_backup_on_upgrade() {
        let base = fixture();
        let target = base.join("external/data");
        fs::create_dir_all(&target).unwrap();
        fs::create_dir_all(manifest(&base).parent().unwrap()).unwrap();
        fs::write(
            manifest(&base),
            serde_json::to_vec(&serde_json::json!({"active":target})).unwrap(),
        )
        .unwrap();
        initialize_at(&base).unwrap();
        fs::remove_file(manifest(&base)).unwrap();
        assert_eq!(data_dir_at(&base).unwrap(), target);
        fs::remove_dir_all(base).unwrap();
    }
    #[test]
    fn unavailable_relocated_data_does_not_fall_back_to_an_empty_profile() {
        let base = fixture();
        let target = base.join("disconnected-drive/data");
        write(
            &base,
            &Location {
                active: Some(target.clone()),
                ..Default::default()
            },
        )
        .unwrap();
        assert!(data_dir_at(&base).is_err());
        assert!(initialize_at(&base).is_err());
        assert!(!target.exists());
        assert_eq!(read(&base).unwrap().active, Some(target));
        fs::remove_dir_all(base).unwrap();
    }
    fn fixture() -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "deeznats-storage-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        if root.exists() {
            fs::remove_dir_all(&root).unwrap();
        }
        fs::create_dir_all(&root).unwrap();
        root
    }
    #[test]
    fn relocation_does_not_copy_youtube_credentials() {
        let base = fixture();
        let source = base.join("DeezNatsRust");
        let target = base.join("target");
        fs::create_dir_all(&source).unwrap();
        fs::write(source.join("youtube-lyrics.dat"), b"encrypted").unwrap();
        fs::write(source.join("youtube-lyrics.123.0.tmp"), b"encrypted").unwrap();
        copy_tree(&source, &target, &source, &target, None).unwrap();
        assert!(!target.join("youtube-lyrics.dat").exists());
        assert!(!target.join("youtube-lyrics.123.0.tmp").exists());
        assert!(source.join("youtube-lyrics.dat").exists());
        fs::remove_dir_all(base).unwrap();
    }
    #[test]
    fn migration_commits_after_copy_preserves_originals_and_rewrites_paths() {
        let base = fixture();
        let source = base.join("DeezNatsRust");
        fs::create_dir_all(source.join("cache")).unwrap();
        fs::write(source.join("cache/song.bin"), b"audio bytes").unwrap();
        fs::write(source.join("session.dat"), b"protected session").unwrap();
        fs::write(
            source.join("ui.json"),
            serde_json::to_vec(
                &serde_json::json!({"ShowOnStartup":3,"BackgroundFile":source.join("cover.png")}),
            )
            .unwrap(),
        )
        .unwrap();
        let target = schedule(&base, &base.join("other/data")).unwrap();
        assert_eq!(data_dir_at(&base).unwrap(), source);
        initialize_at(&base).unwrap();
        assert_eq!(data_dir_at(&base).unwrap(), target);
        assert_eq!(
            fs::read(target.join("cache/song.bin")).unwrap(),
            b"audio bytes"
        );
        assert_eq!(
            fs::read(target.join("session.dat")).unwrap(),
            b"protected session"
        );
        assert!(source.join("session.dat").exists());
        let value: serde_json::Value =
            serde_json::from_slice(&fs::read(target.join("ui.json")).unwrap()).unwrap();
        assert_eq!(value["ShowOnStartup"], 3);
        assert_eq!(
            value["BackgroundFile"],
            target.join("cover.png").to_string_lossy().as_ref()
        );
        initialize_at(&base).unwrap();
        fs::remove_dir_all(base).unwrap();
    }
    #[test]
    fn downloaded_episodes_move_with_metadata_and_originals_remain() {
        let base = fixture();
        let source = base.join("DeezNatsRust");
        let external = base.join("old-podcast-downloads");
        fs::create_dir_all(&source).unwrap();
        fs::create_dir_all(&external).unwrap();
        fs::write(external.join("episode.mp3"), b"podcast").unwrap();
        fs::write(
            source.join("offline.json"),
            serde_json::to_vec(&serde_json::json!([{"path":external.join("episode.mp3")}]))
                .unwrap(),
        )
        .unwrap();
        let target = schedule(&base, &base.join("new/data")).unwrap();
        let mut location = read(&base).unwrap();
        location.downloads = Some(external.clone());
        write(&base, &location).unwrap();
        initialize_at(&base).unwrap();
        assert_eq!(
            fs::read(target.join("downloads/episode.mp3")).unwrap(),
            b"podcast"
        );
        assert_eq!(fs::read(external.join("episode.mp3")).unwrap(), b"podcast");
        let metadata: serde_json::Value =
            serde_json::from_slice(&fs::read(target.join("offline.json")).unwrap()).unwrap();
        assert_eq!(
            metadata[0]["path"],
            target
                .join("downloads/episode.mp3")
                .to_string_lossy()
                .as_ref()
        );
        assert_eq!(
            fs::read_to_string(target.join("download-dir.txt")).unwrap(),
            target.join("downloads").to_string_lossy()
        );
        fs::remove_dir_all(base).unwrap();
    }
    #[test]
    fn copy_failure_keeps_active_location_and_can_resume() {
        let base = fixture();
        let source = base.join("DeezNatsRust");
        fs::create_dir_all(source.join("cache")).unwrap();
        fs::write(source.join("cache/song"), b"song").unwrap();
        let target = schedule(&base, &base.join("other")).unwrap();
        let canonical_source = fs::canonicalize(&source).unwrap();
        fs::write(
            target.join(".deeznats-migration.json"),
            serde_json::to_vec(&canonical_source).unwrap(),
        )
        .unwrap();
        fs::write(target.join("cache"), b"obstruct directory").unwrap();
        assert!(initialize_at(&base).is_err());
        assert_eq!(data_dir_at(&base).unwrap(), source);
        assert_eq!(read(&base).unwrap().pending, Some(target.clone()));
        fs::remove_file(target.join("cache")).unwrap();
        initialize_at(&base).unwrap();
        assert_eq!(data_dir_at(&base).unwrap(), target);
        assert_eq!(fs::read(source.join("cache/song")).unwrap(), b"song");
        fs::remove_dir_all(base).unwrap();
    }
    #[test]
    fn rejects_nested_and_occupied_destinations_without_changing_active_path() {
        let base = fixture();
        let source = base.join("DeezNatsRust");
        fs::create_dir_all(&source).unwrap();
        assert!(schedule(&base, &source.join("nested")).is_err());
        let occupied = base.join("occupied");
        fs::create_dir_all(&occupied).unwrap();
        fs::write(occupied.join("keep"), b"keep").unwrap();
        assert!(schedule(&base, &occupied).is_err());
        assert_eq!(data_dir_at(&base).unwrap(), source);
        assert_eq!(fs::read(occupied.join("keep")).unwrap(), b"keep");
        fs::remove_dir_all(base).unwrap();
    }
}
