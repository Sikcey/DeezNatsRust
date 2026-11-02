//! Local ZIP import with strict Windows path, entry, expansion, and CRC limits.
use super::build::MAX_FILE;
use std::os::windows::fs::OpenOptionsExt;
use std::{
    collections::HashSet,
    fs::{File, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
const MAX_ARCHIVE: u64 = 512 * 1024 * 1024;
const MAX_EXPANDED: u64 = 768 * 1024 * 1024;
const MAX_ENTRIES: usize = 2048;

pub(super) fn archive_file(path: &Path) -> Result<File, String> {
    let mut file = OpenOptions::new()
        .read(true)
        .share_mode(1)
        .open(path)
        .map_err(|_| "Could not open the downloaded Soloist ZIP")?;
    let size = file
        .metadata()
        .map_err(|_| "Could not inspect Soloist ZIP")?
        .len();
    if !(22..=MAX_ARCHIVE).contains(&size) {
        return Err("Soloist ZIP is empty or exceeds 512 MB".into());
    }
    // Bound directory allocation before the ZIP library reads attacker-controlled metadata.
    let length = size.min(65557) as usize;
    file.seek(SeekFrom::End(-(length as i64)))
        .map_err(|_| "Invalid ZIP trailer")?;
    let mut tail = vec![0; length];
    file.read_exact(&mut tail)
        .map_err(|_| "Truncated ZIP trailer")?;
    let offset = (0..=tail.len() - 22)
        .rev()
        .find(|i| {
            tail[*i..*i + 4] == *b"PK\x05\x06"
                && *i + 22 + u16::from_le_bytes(tail[*i + 20..*i + 22].try_into().unwrap()) as usize
                    == tail.len()
        })
        .ok_or("Invalid ZIP directory trailer")?;
    if offset >= 20 && &tail[offset - 20..offset - 16] == b"PK\x06\x07" {
        return Err("ZIP64 Soloist archives are not supported".into());
    }
    let header = &tail[offset..offset + 22];
    let u16_at = |i| u16::from_le_bytes(header[i..i + 2].try_into().unwrap());
    let u32_at = |i| u32::from_le_bytes(header[i..i + 4].try_into().unwrap());
    if u16_at(4) != 0
        || u16_at(6) != 0
        || u16_at(8) != u16_at(10)
        || !(1..=MAX_ENTRIES).contains(&(u16_at(10) as usize))
        || u32_at(12) > 2 * 1024 * 1024
        || u32_at(16) as u64 + u32_at(12) as u64 > size
    {
        return Err("Unsupported multi-volume, ZIP64, or excessive Soloist ZIP directory".into());
    }
    file.seek(SeekFrom::Start(0))
        .map_err(|_| "Could not rewind Soloist ZIP")?;
    Ok(file)
}
pub(super) fn safe_relative(name: &str) -> Result<PathBuf, String> {
    if name.is_empty() || name.len() > 220 || !name.is_ascii() {
        return Err("Unsupported Soloist ZIP path".into());
    }
    let normalized = name.replace('\\', "/");
    let normalized = normalized.strip_suffix('/').unwrap_or(&normalized);
    let mut path = PathBuf::new();
    for component in normalized.split('/') {
        let stem = component
            .split('.')
            .next()
            .unwrap_or_default()
            .trim_end_matches(['.', ' '])
            .to_ascii_uppercase();
        let reserved = matches!(
            stem.as_str(),
            "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$" | "CLOCK$"
        ) || (stem.len() == 4
            && (stem.starts_with("COM") || stem.starts_with("LPT"))
            && matches!(stem.as_bytes()[3], b'1'..=b'9'));
        if component.is_empty()
            || matches!(component, "." | "..")
            || component.ends_with(['.', ' '])
            || component
                .chars()
                .any(|c| c.is_control() || "<>:\"|?*".contains(c))
            || reserved
        {
            return Err("Unsafe Windows path in Soloist ZIP".into());
        }
        path.push(component);
    }
    Ok(path)
}
pub(super) fn extract(file: File, destination: &Path) -> Result<PathBuf, String> {
    let mut zip = zip::ZipArchive::new(file).map_err(|_| "Invalid or unsupported Soloist ZIP")?;
    if zip.is_empty()
        || zip.len() > MAX_ENTRIES
        || zip.has_duplicate_names()
        || zip
            .has_overlapping_files()
            .map_err(|_| "Invalid ZIP file layout")?
    {
        return Err("Soloist ZIP has duplicate, overlapping, or excessive entries".into());
    }
    let mut names = HashSet::new();
    let mut files = Vec::new();
    let mut declared = 0u64;
    let mut executable = None;
    // Validate every entry before writing the first one.
    for i in 0..zip.len() {
        let entry = zip
            .by_index(i)
            .map_err(|_| "Unreadable or encrypted Soloist ZIP entry")?;
        let name = entry
            .name()
            .map_err(|_| "Soloist ZIP filename is not supported")?
            .into_owned();
        let relative = safe_relative(&name)?;
        let kind = entry.unix_mode().unwrap_or(0) & 0o170000;
        if entry.encrypted() || entry.is_symlink() || !matches!(kind, 0 | 0o100000 | 0o040000) {
            return Err("Soloist ZIP must not contain links, devices, or encrypted entries".into());
        }
        let key = relative
            .to_string_lossy()
            .replace('\\', "/")
            .to_ascii_lowercase();
        if !names.insert(key) {
            return Err("Soloist ZIP contains Windows filename collisions".into());
        }
        if entry.size() > MAX_FILE {
            return Err("Soloist ZIP entry exceeds 256 MB".into());
        }
        declared = declared
            .checked_add(entry.size())
            .filter(|v| *v <= MAX_EXPANDED)
            .ok_or("Soloist ZIP expands beyond 768 MB")?;
        if !entry.is_dir()
            && relative
                .file_name()
                .is_some_and(|s| s.eq_ignore_ascii_case("soloist.exe"))
        {
            if executable.replace(relative.clone()).is_some() {
                return Err("Soloist ZIP contains multiple soloist.exe files".into());
            }
        }
        files.push((i, relative, entry.is_dir(), entry.size()));
    }
    let executable = executable.ok_or("Soloist ZIP does not contain soloist.exe")?;
    let started = Instant::now();
    let mut total = 0u64;
    for (i, relative, directory, expected_size) in files {
        let path = destination.join(relative);
        if directory {
            std::fs::create_dir_all(&path)
                .map_err(|_| "Could not create Soloist package directory")?;
            continue;
        }
        std::fs::create_dir_all(path.parent().unwrap())
            .map_err(|_| "Could not create Soloist package directory")?;
        let mut output = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(path)
            .map_err(|_| "Could not create Soloist package file")?;
        let mut input = zip
            .by_index(i)
            .map_err(|_| "Could not read Soloist ZIP file")?;
        let mut count = 0u64;
        let mut bytes = [0u8; 65536];
        loop {
            if started.elapsed() > Duration::from_secs(30) {
                return Err("Soloist ZIP extraction timed out".into());
            }
            let n = input
                .read(&mut bytes)
                .map_err(|_| "Soloist ZIP is corrupt or failed its CRC check")?;
            if n == 0 {
                break;
            }
            count += n as u64;
            total += n as u64;
            if count > expected_size || total > MAX_EXPANDED {
                return Err("Soloist ZIP exceeded its declared expansion limit".into());
            }
            output
                .write_all(&bytes[..n])
                .map_err(|_| "Could not write Soloist package file")?;
        }
        if count != expected_size {
            return Err("Soloist ZIP entry size did not match its directory".into());
        }
        output
            .sync_all()
            .map_err(|_| "Could not flush Soloist package file")?;
    }
    Ok(executable)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_windows_escapes_devices_and_aliases() {
        for name in [
            "../soloist.exe",
            "C:/soloist.exe",
            "/soloist.exe",
            "a/../soloist.exe",
            "a\\..\\soloist.exe",
            "//server/share/soloist.exe",
            "soloist.exe:stream",
            "NUL.txt",
            "NUL .txt",
            "CONIN$",
            "CONOUT$",
            "CLOCK$",
            "COM1",
            "LPT9.log",
            "foo./soloist.exe",
            "foo /soloist.exe",
            "a//soloist.exe",
        ] {
            assert!(safe_relative(name).is_err(), "{name}");
        }
        assert_eq!(
            safe_relative("package/soloist.exe").unwrap(),
            PathBuf::from("package/soloist.exe")
        );
    }
}
