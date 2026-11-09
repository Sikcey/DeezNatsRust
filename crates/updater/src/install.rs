use crate::{verify, ALLOWED, MAX_FILE_SIZE};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApplyFile {
    pub name: String,
    pub sha256: String,
    pub size: u64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApplyPlan {
    pub directory: PathBuf,
    pub parent_pid: u32,
    pub version: String,
    pub files: Vec<ApplyFile>,
}

impl ApplyPlan {
    pub fn validate(&self, stage: &Path) -> Result<(), String> {
        if !self.directory.is_absolute()
            || self.files.is_empty()
            || self.files.len() > ALLOWED.len()
            || !self.directory.is_dir()
            || !stage.is_absolute()
            || !stage.is_dir()
            || stage.starts_with(&self.directory)
            || self.directory.starts_with(stage)
        {
            return Err("Invalid update installation paths.".into());
        }
        semver::Version::parse(&self.version).map_err(|e| e.to_string())?;
        let mut names = std::collections::HashSet::new();
        for file in &self.files {
            if !ALLOWED.contains(&file.name.as_str())
                || !names.insert(&file.name)
                || !crate::valid_hash(&file.sha256)
                || file.size == 0
                || file.size > MAX_FILE_SIZE
            {
                return Err("Invalid staged file.".into());
            }
            verify(&stage.join(&file.name), file.size, &file.sha256)?;
            let target = self.directory.join(&file.name);
            if let Ok(metadata) = fs::symlink_metadata(&target) {
                if !metadata.is_file() || metadata.file_type().is_symlink() {
                    return Err("Update target is not a regular file.".into());
                }
            }
        }
        Ok(())
    }
}

/// All replacements are already staged and verified. Backups stay on the same
/// filesystem as the destination so each rename is atomic, even across drives.
pub fn commit(plan: &ApplyPlan, stage: &Path) -> Result<(), String> {
    plan.validate(stage)?;
    let backup = plan.directory.join(".deeznats-update-backup");
    fs::create_dir(&backup).map_err(|e| format!("Could not create update backup: {e}"))?;
    let result = commit_inner(plan, stage, &backup);
    if result.is_ok() {
        let _ = fs::remove_dir(&backup);
    }
    result
}

fn commit_inner(plan: &ApplyPlan, stage: &Path, backup: &Path) -> Result<(), String> {
    let mut completed: Vec<(&ApplyFile, bool)> = Vec::new();
    // Replace the application last, after its engine/resources are ready.
    let mut files: Vec<_> = plan.files.iter().collect();
    files.sort_by_key(|file| file.name == "deeznats.exe");
    for file in files {
        let target = plan.directory.join(&file.name);
        let incoming = backup.join(format!("{}.new", file.name));
        let saved = backup.join(&file.name);
        let existed = target.exists();
        let install = (|| -> Result<(), String> {
            fs::copy(stage.join(&file.name), &incoming).map_err(|e| e.to_string())?;
            verify(&incoming, file.size, &file.sha256)?;
            if existed {
                fs::rename(&target, &saved).map_err(|e| e.to_string())?;
            }
            if let Err(error) = fs::rename(&incoming, &target) {
                if existed {
                    fs::rename(&saved, &target)
                        .map_err(|e| format!("{error}; restore failed: {e}"))?;
                }
                return Err(error.to_string());
            }
            Ok(())
        })();
        if let Err(error) = install {
            let _ = fs::remove_file(&incoming);
            let mut restore_errors = Vec::new();
            for (done, had_original) in completed.into_iter().rev() {
                let target = plan.directory.join(&done.name);
                if let Err(e) = fs::remove_file(&target) {
                    restore_errors.push(e.to_string());
                    continue;
                }
                if had_original {
                    if let Err(e) = fs::rename(backup.join(&done.name), target) {
                        restore_errors.push(e.to_string());
                    }
                }
            }
            if restore_errors.is_empty() {
                let _ = fs::remove_dir(backup);
            }
            return Err(format!(
                "Update could not be applied: {error}{}",
                if restore_errors.is_empty() {
                    String::new()
                } else {
                    format!("; backups retained: {}", restore_errors.join("; "))
                }
            ));
        }
        completed.push((file, existed));
    }
    for (file, existed) in completed {
        if existed {
            let _ = fs::remove_file(backup.join(&file.name));
        }
    }
    Ok(())
}
