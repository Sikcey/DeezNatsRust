use deeznats_updater::{
    apply_delta, commit, hash, ApplyFile, ApplyPlan, Manifest, Transfer, MANIFEST_ASSET,
};
use std::{fs, path::PathBuf, process::Command};
struct Fixture(PathBuf);
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
#[test]
fn generated_release_patches_only_changed_binary_and_preserves_other_files() {
    let root =
        Fixture(std::env::temp_dir().join(format!("deeznats-release-test-{}", std::process::id())));
    fs::create_dir(&root.0).unwrap();
    let old = root.0.join("old");
    let new = root.0.join("new");
    let output = root.0.join("output");
    let stage = root.0.join("stage");
    for dir in [&old, &new, &stage] {
        fs::create_dir(dir).unwrap();
    }
    let mut seed = 123u32;
    let before: Vec<_> = (0..100_000)
        .map(|_| {
            seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
            (seed >> 24) as u8
        })
        .collect();
    let mut after = before.clone();
    after[1000..1012].copy_from_slice(b"patch change");
    fs::write(old.join("deeznats.exe"), &before).unwrap();
    fs::write(new.join("deeznats.exe"), &after).unwrap();
    for dir in [&old, &new] {
        fs::write(dir.join("libdeezercore.dll"), b"unchanged engine").unwrap();
    }
    fs::write(old.join("ui.json"), b"private settings").unwrap();
    let engine_time = fs::metadata(old.join("libdeezercore.dll"))
        .unwrap()
        .modified()
        .unwrap();
    let status = Command::new(env!("CARGO_BIN_EXE_deeznats-update-pack"))
        .arg("--current")
        .arg(&new)
        .arg("--previous")
        .arg(&old)
        .arg("--output")
        .arg(&output)
        .args(["--version", "0.2.0"])
        .status()
        .unwrap();
    assert!(status.success());
    let manifest = Manifest::parse(&fs::read(output.join(MANIFEST_ASSET)).unwrap()).unwrap();
    let changes = manifest.changes(&old).unwrap();
    assert_eq!(changes.len(), 1);
    let change = &changes[0];
    let Transfer::Delta(patch) = &change.transfer else {
        panic!("expected an actual binary delta");
    };
    assert!(patch.size < change.file.size / 10);
    apply_delta(
        &old.join(&change.file.name),
        &output.join(&patch.asset),
        &stage.join(&change.file.name),
        &change.file,
        patch,
    )
    .unwrap();
    let plan = ApplyPlan {
        directory: old.clone(),
        parent_pid: 0,
        version: manifest.version,
        files: vec![ApplyFile {
            name: change.file.name.clone(),
            sha256: hash(&after),
            size: after.len() as u64,
        }],
    };
    commit(&plan, &stage).unwrap();
    assert_eq!(fs::read(old.join("deeznats.exe")).unwrap(), after);
    assert_eq!(
        fs::metadata(old.join("libdeezercore.dll"))
            .unwrap()
            .modified()
            .unwrap(),
        engine_time
    );
    assert_eq!(fs::read(old.join("ui.json")).unwrap(), b"private settings");
}
