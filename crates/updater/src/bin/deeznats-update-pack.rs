use deeznats_updater::{
    hash, Manifest, Patch, ReleaseFile, ALLOWED, MANIFEST_ASSET, MAX_FILE_SIZE,
};
use std::{fs, io::Cursor, path::PathBuf};

fn run() -> Result<(), String> {
    let mut args = std::env::args().skip(1);
    let mut current = None;
    let mut output = None;
    let mut version = None;
    let mut previous = Vec::new();
    while let Some(arg) = args.next() {
        let value = args.next().ok_or("Missing argument value")?;
        match arg.as_str() {
            "--current" => current = Some(PathBuf::from(value)),
            "--output" => output = Some(PathBuf::from(value)),
            "--version" => version = Some(value),
            "--previous" => previous.push(PathBuf::from(value)),
            _ => return Err(format!("Unknown argument: {arg}")),
        }
    }
    let current = current
        .ok_or("--current is required")?
        .canonicalize()
        .map_err(|e| e.to_string())?;
    let output = output.ok_or("--output is required")?;
    fs::create_dir_all(&output).map_err(|e| e.to_string())?;
    let output = output.canonicalize().map_err(|e| e.to_string())?;
    if output.starts_with(&current) || current.starts_with(&output) {
        return Err("Keep output separate from the application directory.".into());
    }
    let mut manifest = Manifest {
        schema: 1,
        version: version.ok_or("--version is required")?,
        files: Vec::new(),
    };
    for name in ALLOWED {
        let path = current.join(name);
        if !path.is_file() {
            continue;
        }
        if fs::metadata(&path).map_err(|e| e.to_string())?.len() > MAX_FILE_SIZE {
            return Err(format!("{name} is too large"));
        }
        let data = fs::read(path).map_err(|e| e.to_string())?;
        let mut file = ReleaseFile {
            name: (*name).into(),
            size: data.len() as u64,
            sha256: hash(&data),
            asset: format!("update-{name}"),
            patches: Vec::new(),
        };
        fs::write(output.join(&file.asset), &data).map_err(|e| e.to_string())?;
        for directory in &previous {
            let path = directory.join(name);
            if !path.is_file() {
                continue;
            }
            if fs::metadata(&path).map_err(|e| e.to_string())?.len() > MAX_FILE_SIZE {
                return Err(format!("Previous {name} is too large"));
            }
            let old = fs::read(path).map_err(|e| e.to_string())?;
            let from_sha256 = hash(&old);
            if from_sha256 == file.sha256
                || file.patches.iter().any(|p| p.from_sha256 == from_sha256)
            {
                continue;
            }
            let mut patch = Vec::new();
            qbsdiff::Bsdiff::new(&old, &data)
                .compare(Cursor::new(&mut patch))
                .map_err(|e| e.to_string())?;
            if patch.len() >= data.len() {
                continue;
            }
            // Verify generated artifacts before publishing a manifest that refers to them.
            let mut reconstructed = Vec::new();
            qbsdiff::Bspatch::new(&patch)
                .map_err(|e| e.to_string())?
                .apply(&old, Cursor::new(&mut reconstructed))
                .map_err(|e| e.to_string())?;
            if reconstructed != data {
                return Err(format!("Delta verification failed for {name}"));
            }
            let asset = format!("patch-{name}-{from_sha256}.bsdiff");
            fs::write(output.join(&asset), &patch).map_err(|e| e.to_string())?;
            file.patches.push(Patch {
                from_sha256,
                sha256: hash(&patch),
                size: patch.len() as u64,
                asset,
            });
        }
        manifest.files.push(file);
    }
    manifest.validate()?;
    fs::write(
        output.join(MANIFEST_ASSET),
        serde_json::to_vec_pretty(&manifest).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    println!(
        "Update {}: {} files, {} delta patches",
        manifest.version,
        manifest.files.len(),
        manifest
            .files
            .iter()
            .map(|f| f.patches.len())
            .sum::<usize>()
    );
    Ok(())
}
fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
