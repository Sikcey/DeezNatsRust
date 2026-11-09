//! Release manifests, verified BSDIFF40 patches and selective installation.
mod install;
pub use install::{commit, ApplyFile, ApplyPlan};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{self, Write},
    path::Path,
};

pub const MANIFEST_ASSET: &str = "deeznats-update.json";
pub const MAX_FILE_SIZE: u64 = 256 * 1024 * 1024;
pub const ALLOWED: &[&str] = &[
    "deeznats.exe",
    "libdeezercore.dll",
    "DeezNats.ico",
    "LICENSE",
    "NOTICE.md",
    "SOURCE-AND-DEPENDENCIES.md",
    "qbsdiff.LICENSE",
];

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub schema: u32,
    pub version: String,
    pub files: Vec<ReleaseFile>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReleaseFile {
    pub name: String,
    pub size: u64,
    pub sha256: String,
    pub asset: String,
    #[serde(default)]
    pub patches: Vec<Patch>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Patch {
    pub from_sha256: String,
    pub sha256: String,
    pub size: u64,
    pub asset: String,
}
#[derive(Clone, Debug)]
pub enum Transfer {
    Full,
    Delta(Patch),
}
#[derive(Clone, Debug)]
pub struct Change {
    pub file: ReleaseFile,
    pub transfer: Transfer,
}

pub fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
pub fn hash_file(path: &Path) -> Result<String, String> {
    let mut file = fs::File::open(path).map_err(|e| e.to_string())?;
    let mut hasher = Sha256::new();
    io::copy(&mut file, &mut hasher).map_err(|e| e.to_string())?;
    Ok(format!("{:x}", hasher.finalize()))
}
pub fn valid_hash(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
pub fn newer_version(candidate: &str, current: &str) -> Result<bool, String> {
    Ok(
        semver::Version::parse(candidate).map_err(|e| e.to_string())?
            > semver::Version::parse(current).map_err(|e| e.to_string())?,
    )
}
fn valid_asset(value: &str) -> bool {
    !value.is_empty()
        && value.len() < 160
        && !value.starts_with('.')
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_'))
}
pub fn verify(path: &Path, size: u64, digest: &str) -> Result<(), String> {
    if fs::metadata(path).map_err(|e| e.to_string())?.len() != size || hash_file(path)? != digest {
        return Err("Update file verification failed.".into());
    }
    Ok(())
}
impl Manifest {
    pub fn parse(bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() > 128 * 1024 {
            return Err("Update manifest is too large.".into());
        }
        let result: Self = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
        result.validate()?;
        Ok(result)
    }
    pub fn validate(&self) -> Result<(), String> {
        let version = semver::Version::parse(&self.version).map_err(|e| e.to_string())?;
        if self.schema != 1 || !version.pre.is_empty() || self.files.len() > ALLOWED.len() {
            return Err("Unsupported update manifest.".into());
        }
        let mut names = std::collections::HashSet::new();
        for file in &self.files {
            if !ALLOWED.contains(&file.name.as_str())
                || !names.insert(&file.name)
                || file.size == 0
                || file.size > MAX_FILE_SIZE
                || !valid_hash(&file.sha256)
                || !valid_asset(&file.asset)
                || file.patches.len() > 32
            {
                return Err("Invalid update file.".into());
            }
            for patch in &file.patches {
                if !valid_hash(&patch.from_sha256)
                    || !valid_hash(&patch.sha256)
                    || patch.size == 0
                    || patch.size > MAX_FILE_SIZE
                    || !valid_asset(&patch.asset)
                {
                    return Err("Invalid update patch.".into());
                }
            }
        }
        if !["deeznats.exe", "libdeezercore.dll"]
            .iter()
            .all(|name| self.files.iter().any(|f| &f.name == name))
        {
            return Err("Update is missing required application files.".into());
        }
        Ok(())
    }
    pub fn newer_than(&self, current: &str) -> Result<bool, String> {
        newer_version(&self.version, current)
    }
    pub fn changes(&self, directory: &Path) -> Result<Vec<Change>, String> {
        self.validate()?;
        let mut changes = Vec::new();
        for file in &self.files {
            let path = directory.join(&file.name);
            let old_hash = if path.exists() {
                Some(hash_file(&path)?)
            } else {
                None
            };
            if old_hash.as_deref() == Some(&file.sha256) {
                continue;
            }
            let transfer = file
                .patches
                .iter()
                .filter(|patch| {
                    Some(patch.from_sha256.as_str()) == old_hash.as_deref()
                        && patch.size < file.size
                })
                .min_by_key(|patch| patch.size)
                .cloned()
                .map(Transfer::Delta)
                .unwrap_or(Transfer::Full);
            changes.push(Change {
                file: file.clone(),
                transfer,
            });
        }
        Ok(changes)
    }
}

struct BoundedWriter<W> {
    inner: W,
    remaining: u64,
}
impl<W: Write> Write for BoundedWriter<W> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.len() as u64 > self.remaining {
            return Err(io::Error::other("Patch output exceeds declared size"));
        }
        let written = self.inner.write(bytes)?;
        self.remaining -= written as u64;
        Ok(written)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

pub fn apply_delta(
    source: &Path,
    patch_path: &Path,
    target: &Path,
    file: &ReleaseFile,
    patch: &Patch,
) -> Result<(), String> {
    if fs::metadata(source).map_err(|e| e.to_string())?.len() > MAX_FILE_SIZE {
        return Err("Patch source is too large.".into());
    }
    // Hash exactly the bytes the patcher will use, even if another updater races us.
    let source = fs::read(source).map_err(|e| e.to_string())?;
    if hash(&source) != patch.from_sha256 {
        return Err("Update source changed.".into());
    }
    verify(patch_path, patch.size, &patch.sha256)?;
    let data = fs::read(patch_path).map_err(|e| e.to_string())?;
    let patcher = qbsdiff::Bspatch::new(&data).map_err(|e| e.to_string())?;
    if patcher.hint_target_size() != file.size {
        return Err("Unexpected patch output size.".into());
    }
    let output = fs::File::create(target).map_err(|e| e.to_string())?;
    patcher
        .apply(
            &source,
            BoundedWriter {
                inner: output,
                remaining: file.size,
            },
        )
        .map_err(|e| e.to_string())?;
    verify(target, file.size, &file.sha256)
}

#[cfg(test)]
mod tests;
