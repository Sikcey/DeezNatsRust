use super::*;
use std::{io::Write, process::Command, time::Duration};
use zip::{write::SimpleFileOptions, CompressionMethod, ZipWriter};
struct Temporary(PathBuf);
impl Temporary {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("deeznats-soloist-setup-test-{}", nonce()));
        std::fs::create_dir(&path).unwrap();
        Self(path.canonicalize().unwrap())
    }
}
impl Drop for Temporary {
    fn drop(&mut self) {
        if self.0.parent() == Some(std::env::temp_dir().canonicalize().unwrap().as_path())
            && self
                .0
                .file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("deeznats-soloist-setup-test-")
        {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
}
fn zip_file(path: &Path, files: &[(&str, &[u8])]) {
    let mut writer = ZipWriter::new(File::create(path).unwrap());
    for (name, bytes) in files {
        writer
            .start_file(
                *name,
                SimpleFileOptions::default().compression_method(CompressionMethod::Stored),
            )
            .unwrap();
        writer.write_all(bytes).unwrap();
    }
    writer.finish().unwrap();
}
#[test]
fn import_activation_update_and_rollback_preserve_paired_state() {
    let temp = Temporary::new();
    let executable = temp.0.join("fixture.exe");
    let mut compiler = Command::new("rustc");
    compiler
        .arg("--edition=2021")
        .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/soloist_mock.rs"))
        .arg("-o")
        .arg(&executable);
    assert!(
        super::super::process::capture(&mut compiler, Duration::from_secs(30))
            .unwrap()
            .0
            .success()
    );
    let bytes = std::fs::read(&executable).unwrap();
    let zip = temp.0.join("download.zip");
    zip_file(
        &zip,
        &[
            ("package/soloist.exe", &bytes),
            ("package/LICENSE.txt", b"fixture notices"),
            ("package/readme.txt", b"first"),
        ],
    );
    let root = temp.0.join("installation");
    let session = temp.0.join("session");
    std::fs::create_dir(&session).unwrap();
    std::fs::write(
        session.join("paired-session"),
        b"test session; never replace",
    )
    .unwrap();
    let first = import_at(&root, &zip).unwrap();
    assert_eq!(first.build.expiry, "current");
    assert!(active_at(&root).unwrap().is_none());
    assert_eq!(first.id, import_at(&root, &zip).unwrap().id);
    assert_eq!(
        std::fs::read(first.executable.parent().unwrap().join("LICENSE.txt")).unwrap(),
        b"fixture notices"
    );
    activate_at(&root, &session, &first.id).unwrap();
    assert_eq!(
        selected_at(&root, Some(&executable)).unwrap(),
        Some(build::strip_verbatim(&executable))
    );
    // Importing an update remains possible during playback; activation waits for ownership release.
    let busy = lock(&session, "deeznats.lock").unwrap();
    zip_file(
        &zip,
        &[
            ("soloist.exe", &bytes),
            ("LICENSE.txt", b"fixture notices"),
            ("readme.txt", b"second"),
        ],
    );
    let second = import_at(&root, &zip).unwrap();
    assert_ne!(first.id, second.id);
    assert!(activate_at(&root, &session, &second.id)
        .unwrap_err()
        .contains("busy"));
    drop(busy);
    activate_at(&root, &session, &second.id).unwrap();
    assert_eq!(
        selected_at(&root, Some(&first.executable)).unwrap(),
        Some(second.executable.clone())
    );
    assert_eq!(active_at(&root).unwrap().unwrap().id, second.id);
    rollback_at(&root, &session).unwrap();
    assert_eq!(active_at(&root).unwrap().unwrap().id, first.id);
    assert_eq!(
        std::fs::read(session.join("paired-session")).unwrap(),
        b"test session; never replace"
    );
    let before = std::fs::read(root.join("active.json")).unwrap();
    let mut changed = std::fs::read(&second.executable).unwrap();
    let end = changed.len() - 1;
    changed[end] ^= 1;
    std::fs::write(&second.executable, &changed).unwrap();
    assert!(activate_at(&root, &session, &second.id)
        .unwrap_err()
        .contains("changed since import"));
    assert_eq!(std::fs::read(root.join("active.json")).unwrap(), before);
    // A broken managed build must not prevent recovery through a local executable.
    std::fs::write(root.join("active.json"), b"broken metadata").unwrap();
    assert_eq!(
        selected_at(&root, Some(&executable)).unwrap(),
        Some(build::strip_verbatim(&executable))
    );
    assert!(selected_at(&root, None).is_err());
    std::fs::write(root.join("active.json"), &before).unwrap();
    // Known-expired staged builds are rejected without changing the active selection.
    std::fs::write(&second.executable, &bytes).unwrap();
    let receipt_path = root.join("builds").join(&second.id).join(RECEIPT);
    let mut receipt: Receipt = bounded_json(&receipt_path).unwrap();
    receipt.build.build_timestamp = Some(build::now() - 91 * 86400);
    save_json(&receipt_path, &receipt).unwrap();
    assert!(activate_at(&root, &session, &second.id)
        .unwrap_err()
        .contains("expired"));
    assert_eq!(std::fs::read(root.join("active.json")).unwrap(), before);
    receipt.relative_executable = PathBuf::from("../soloist.exe");
    save_json(&receipt_path, &receipt).unwrap();
    assert!(load_at(&root, &second.id).is_err());
}
#[test]
fn invalid_archives_never_commit_a_build_or_leave_staged_files() {
    let temp = Temporary::new();
    let root = temp.0.join("install");
    let zip = temp.0.join("bad.zip");
    for files in [
        vec![("../soloist.exe", b"not an exe".as_slice())],
        vec![
            ("soloist.exe", b"bad".as_slice()),
            ("SOLOIST.EXE", b"bad".as_slice()),
        ],
        vec![("readme.txt", b"no executable".as_slice())],
        vec![("soloist.exe", b"not a PE".as_slice())],
    ] {
        zip_file(&zip, &files);
        assert!(import_at(&root, &zip).is_err());
        assert!(!root.join("builds").exists());
        assert!(!root.join("active.json").exists());
        if root.join("staging").exists() {
            assert_eq!(std::fs::read_dir(root.join("staging")).unwrap().count(), 0);
        }
    }
    let mut writer = ZipWriter::new(File::create(&zip).unwrap());
    writer
        .add_symlink("soloist.exe", "elsewhere", SimpleFileOptions::default())
        .unwrap();
    writer.finish().unwrap();
    assert!(import_at(&root, &zip).is_err());
    zip_file(&zip, &[("soloist.exe", b"payload with CRC")]);
    let mut bytes = std::fs::read(&zip).unwrap();
    let offset = bytes
        .windows(16)
        .position(|s| s == b"payload with CRC")
        .unwrap();
    bytes[offset] ^= 1;
    std::fs::write(&zip, bytes).unwrap();
    assert!(import_at(&root, &zip).unwrap_err().contains("CRC"));
    zip_file(&zip, &[("soloist.exe", b"placeholder")]);
    let mut bytes = std::fs::read(&zip).unwrap();
    let end = bytes.len() - 22;
    bytes[end + 8..end + 12].copy_from_slice(&[255, 255, 255, 255]);
    std::fs::write(&zip, bytes).unwrap();
    assert!(import_at(&root, &zip).unwrap_err().contains("excessive"));
    assert_eq!(std::fs::read_dir(root.join("staging")).unwrap().count(), 0);
}
#[test]
fn executable_architecture_is_checked_without_execution() {
    let temp = Temporary::new();
    let path = temp.0.join("soloist.exe");
    let mut bytes = vec![0u8; 256];
    bytes[..2].copy_from_slice(b"MZ");
    bytes[60..64].copy_from_slice(&128u32.to_le_bytes());
    bytes[128..132].copy_from_slice(b"PE\0\0");
    bytes[132..134].copy_from_slice(&0x8664u16.to_le_bytes());
    bytes[152..154].copy_from_slice(&0x20bu16.to_le_bytes());
    std::fs::write(&path, &bytes).unwrap();
    build::pe_x64(&path).unwrap();
    bytes[132..134].copy_from_slice(&0xaa64u16.to_le_bytes());
    std::fs::write(&path, &bytes).unwrap();
    assert!(build::pe_x64(&path).is_err());
    bytes[132..134].copy_from_slice(&0x8664u16.to_le_bytes());
    bytes[150..152].copy_from_slice(&0x2000u16.to_le_bytes());
    std::fs::write(&path, bytes).unwrap();
    assert!(build::pe_x64(&path).is_err());
}
