use super::*;
use std::path::PathBuf;
struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "deeznats-update-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
    fn dir(&self, name: &str) -> PathBuf {
        let path = self.0.join(name);
        fs::create_dir(&path).unwrap();
        path
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn entry(name: &str, data: &[u8]) -> ReleaseFile {
    ReleaseFile {
        name: name.into(),
        size: data.len() as u64,
        sha256: hash(data),
        asset: format!("update-{name}"),
        patches: Vec::new(),
    }
}
fn manifest(app: &[u8], engine: &[u8]) -> Manifest {
    Manifest {
        schema: 1,
        version: "0.2.0".into(),
        files: vec![
            entry("deeznats-prototype.exe", app),
            entry("libdeezercore.dll", engine),
        ],
    }
}
#[test]
fn unchanged_files_are_skipped_and_unknown_versions_use_full_files() {
    let f = Fixture::new();
    let installed = f.dir("installed");
    fs::write(installed.join("deeznats-prototype.exe"), b"old").unwrap();
    fs::write(installed.join("libdeezercore.dll"), b"same engine").unwrap();
    let changes = manifest(b"new app", b"same engine")
        .changes(&installed)
        .unwrap();
    assert_eq!(changes.len(), 1);
    assert_eq!(changes[0].file.name, "deeznats-prototype.exe");
    assert!(matches!(changes[0].transfer, Transfer::Full));
    assert!(manifest(b"new app", b"same engine")
        .newer_than("0.1.9")
        .unwrap());
    assert!(!manifest(b"new app", b"same engine")
        .newer_than("0.2.0")
        .unwrap());
}
#[test]
fn real_binary_delta_reconstructs_exact_bytes_and_rejects_damage() {
    let f = Fixture::new();
    let installed = f.dir("installed");
    let mut seed = 7u32;
    let old: Vec<_> = (0..200_000)
        .map(|_| {
            seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
            (seed >> 24) as u8
        })
        .collect();
    let mut new = old.clone();
    new[50_000..50_012].copy_from_slice(b"new behavior");
    let source = installed.join("deeznats-prototype.exe");
    fs::write(&source, &old).unwrap();
    fs::write(installed.join("libdeezercore.dll"), b"engine").unwrap();
    let mut data = Vec::new();
    qbsdiff::Bsdiff::new(&old, &new)
        .compare(io::Cursor::new(&mut data))
        .unwrap();
    assert!(data.len() < new.len() / 10);
    let patch = Patch {
        from_sha256: hash(&old),
        sha256: hash(&data),
        size: data.len() as u64,
        asset: "test.bsdiff".into(),
    };
    let patch_path = f.0.join("patch");
    fs::write(&patch_path, &data).unwrap();
    let mut release = manifest(&new, b"engine");
    release.files[0].patches.push(patch.clone());
    assert!(matches!(
        release.changes(&installed).unwrap()[0].transfer,
        Transfer::Delta(_)
    ));
    let result = f.0.join("reconstructed");
    apply_delta(&source, &patch_path, &result, &release.files[0], &patch).unwrap();
    assert_eq!(fs::read(&result).unwrap(), new);
    fs::write(&patch_path, b"damaged").unwrap();
    assert!(apply_delta(&source, &patch_path, &result, &release.files[0], &patch).is_err());
    fs::write(&patch_path, &data).unwrap();
    fs::write(&source, b"changed source").unwrap();
    assert!(apply_delta(&source, &patch_path, &result, &release.files[0], &patch).is_err());
}
#[test]
fn manifests_reject_traversal_duplicate_files_and_unsupported_versions() {
    let mut value = manifest(b"app", b"engine");
    value.files[0].name = "../ui.json".into();
    assert!(value.validate().is_err());
    let mut value = manifest(b"app", b"engine");
    value.files.push(value.files[0].clone());
    assert!(value.validate().is_err());
    let mut value = manifest(b"app", b"engine");
    value.schema = 2;
    assert!(value.validate().is_err());
    let mut value = manifest(b"app", b"engine");
    value.files[0].asset = "https://other/app.exe".into();
    assert!(value.validate().is_err());
}
#[test]
fn staged_install_preserves_settings_and_refuses_unverified_files() {
    let f = Fixture::new();
    let installed = f.dir("installed");
    let stage = f.dir("stage");
    fs::write(installed.join("ui.json"), b"private settings").unwrap();
    fs::write(installed.join("deeznats-prototype.exe"), b"old").unwrap();
    fs::write(stage.join("deeznats-prototype.exe"), b"new").unwrap();
    let plan = ApplyPlan {
        directory: installed.clone(),
        parent_pid: 0,
        version: "0.2.0".into(),
        files: vec![ApplyFile {
            name: "deeznats-prototype.exe".into(),
            size: 3,
            sha256: hash(b"new"),
        }],
    };
    fs::write(stage.join("deeznats-prototype.exe"), b"bad").unwrap();
    assert!(commit(&plan, &stage).is_err());
    assert_eq!(
        fs::read(installed.join("deeznats-prototype.exe")).unwrap(),
        b"old"
    );
    fs::write(stage.join("deeznats-prototype.exe"), b"new").unwrap();
    commit(&plan, &stage).unwrap();
    assert_eq!(
        fs::read(installed.join("deeznats-prototype.exe")).unwrap(),
        b"new"
    );
    assert_eq!(
        fs::read(installed.join("ui.json")).unwrap(),
        b"private settings"
    );
    assert!(!installed.join(".deeznats-update-backup").exists());
}
#[test]
#[cfg(windows)]
fn locked_second_file_rolls_back_first_file() {
    use std::os::windows::fs::OpenOptionsExt;
    let f = Fixture::new();
    let installed = f.dir("installed");
    let stage = f.dir("stage");
    let mut files = Vec::new();
    for name in ["libdeezercore.dll", "deeznats-prototype.exe"] {
        fs::write(installed.join(name), b"old").unwrap();
        fs::write(stage.join(name), b"new").unwrap();
        files.push(ApplyFile {
            name: name.into(),
            size: 3,
            sha256: hash(b"new"),
        });
    }
    let _locked = fs::OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(installed.join("deeznats-prototype.exe"))
        .unwrap();
    let plan = ApplyPlan {
        directory: installed.clone(),
        parent_pid: 0,
        version: "0.2.0".into(),
        files,
    };
    assert!(commit(&plan, &stage).is_err());
    assert_eq!(
        fs::read(installed.join("libdeezercore.dll")).unwrap(),
        b"old"
    );
    assert!(!installed.join(".deeznats-update-backup").exists());
}
