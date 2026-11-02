//! Static Windows executable checks and bounded Soloist --version inspection.
use super::super::process;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    process::Command,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
const LIFETIME: u64 = 90 * 86400;
pub(super) const MAX_FILE: u64 = 256 * 1024 * 1024;

pub(crate) fn strip_verbatim(path: &Path) -> PathBuf {
    use std::path::{Component, Prefix};
    let mut components = path.components();
    if let Some(Component::Prefix(prefix)) = components.next() {
        match prefix.kind() {
            Prefix::VerbatimDisk(drive) => {
                let mut result = PathBuf::from(format!("{}:\\", drive as char));
                result.push(components.as_path());
                return result;
            }
            Prefix::VerbatimUNC(server, share) => {
                let mut result = PathBuf::from(r"\\");
                result.push(server);
                result.push(share);
                result.push(components.as_path());
                return result;
            }
            _ => {}
        }
    }
    path.to_path_buf()
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct Build {
    pub executable: PathBuf,
    pub sha256: String,
    pub version_output: String,
    pub build_timestamp: Option<u64>,
    pub expires_at: Option<u64>,
    pub days_remaining: Option<u64>,
    pub expiry: String,
}
impl Build {
    pub fn at(&mut self, now: u64) {
        self.expires_at = self.build_timestamp.and_then(|t| t.checked_add(LIFETIME));
        self.days_remaining = self
            .expires_at
            .map(|expiry| expiry.saturating_sub(now).div_ceil(86400));
        self.expiry = match (self.build_timestamp, self.expires_at) {
            (Some(build), _) if build > now.saturating_add(86400) => "clock-or-build-date-invalid",
            (_, Some(expiry)) if expiry <= now => "expired",
            (_, Some(expiry)) if expiry - now <= 14 * 86400 => "expiring-soon",
            (_, Some(_)) => "current",
            _ => "unknown",
        }
        .into();
    }
    pub fn usable(&self) -> bool {
        !matches!(
            self.expiry.as_str(),
            "expired" | "clock-or-build-date-invalid"
        )
    }
}
pub(super) fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
pub(super) fn pe_x64(path: &Path) -> Result<(), String> {
    let mut file = File::open(path).map_err(|_| "Could not open the Soloist executable")?;
    let size = file
        .metadata()
        .map_err(|_| "Could not inspect the Soloist executable")?
        .len();
    if !(128..=MAX_FILE).contains(&size) {
        return Err("Soloist executable has an invalid size".into());
    }
    let mut dos = [0u8; 64];
    file.read_exact(&mut dos)
        .map_err(|_| "Truncated Soloist executable")?;
    if &dos[..2] != b"MZ" {
        return Err("Soloist must be a Windows x64 executable".into());
    }
    let offset = u32::from_le_bytes(dos[60..64].try_into().unwrap()) as u64;
    if offset < 64 || offset.saturating_add(26) > size || offset > 1024 * 1024 {
        return Err("Invalid Soloist PE header".into());
    }
    file.seek(SeekFrom::Start(offset))
        .map_err(|_| "Invalid Soloist PE offset")?;
    let mut pe = [0u8; 26];
    file.read_exact(&mut pe)
        .map_err(|_| "Truncated Soloist PE header")?;
    if &pe[..4] != b"PE\0\0"
        || u16::from_le_bytes(pe[4..6].try_into().unwrap()) != 0x8664
        || u16::from_le_bytes(pe[24..26].try_into().unwrap()) != 0x20b
        || u16::from_le_bytes(pe[22..24].try_into().unwrap()) & 0x2000 != 0
    {
        return Err("Soloist must be an executable Windows x64 PE32+ build, not a DLL".into());
    }
    Ok(())
}
pub(super) fn digest(path: &Path) -> Result<String, String> {
    let mut file = File::open(path).map_err(|_| "Could not open Soloist installation file")?;
    let mut hash = Sha256::new();
    let mut bytes = [0u8; 65536];
    loop {
        let count = file
            .read(&mut bytes)
            .map_err(|_| "Could not hash Soloist installation file")?;
        if count == 0 {
            break;
        }
        hash.update(&bytes[..count]);
    }
    Ok(format!("{:x}", hash.finalize()))
}
/// Only call for a user-selected/imported binary. Discovery never executes candidates.
pub(crate) fn inspect(path: &Path) -> Result<Build, String> {
    pe_x64(path)?;
    let canonical = path
        .canonicalize()
        .map_err(|_| "Could not resolve Soloist executable")?;
    let path = strip_verbatim(&canonical);
    let mut command = Command::new(&path);
    command
        .arg("--version")
        .env_remove("DEEZNATS_SOLOIST_API_KEY")
        .env_remove("DEEZNATS_SPOTIFY_CLIENT_ID");
    let (status, bytes) = process::capture(&mut command, Duration::from_secs(3))?;
    if status.code() == Some(10) {
        return Err("Soloist build expired; download a new build from Spotify".into());
    }
    if !status.success() || bytes.len() > 16384 {
        return Err("Soloist --version failed or returned excessive output".into());
    }
    let text = std::str::from_utf8(&bytes)
        .map_err(|_| "Soloist version output is not UTF-8")?
        .trim();
    if !text.to_ascii_lowercase().contains("soloist")
        || text
            .chars()
            .any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t'))
    {
        return Err("Executable did not identify itself as Soloist".into());
    }
    let mut build = Build {
        executable: path.clone(),
        sha256: digest(&path)?,
        version_output: text.into(),
        build_timestamp: build_timestamp(text),
        expires_at: None,
        days_remaining: None,
        expiry: String::new(),
    };
    build.at(now());
    Ok(build)
}
fn build_timestamp(text: &str) -> Option<u64> {
    let mut found = None;
    for line in text.lines() {
        let lower = line.to_ascii_lowercase();
        for key in [
            "build timestamp",
            "build_timestamp",
            "build-timestamp",
            "build date",
            "build_date",
            "built at",
            "built_at",
            "build ",
            "build",
        ] {
            if let Some(index) = lower.find(key) {
                let value =
                    line[index + key.len()..].trim_start_matches([' ', ':', '=', '\t', '"']);
                if let Some(timestamp) = timestamp(value) {
                    if found.is_some_and(|prior| prior != timestamp) {
                        return None;
                    }
                    found = Some(timestamp);
                }
                break;
            }
        }
    }
    found
}
fn timestamp(text: &str) -> Option<u64> {
    let text = text.trim();
    let digits: String = text.chars().take_while(char::is_ascii_digit).collect();
    if digits.len() == 10 || digits.len() == 13 {
        let rest = text[digits.len()..].trim_start();
        if rest.is_empty()
            || rest.starts_with('(')
            || rest.starts_with('g')
            || rest.starts_with('/')
            || rest.starts_with('-')
            || rest.starts_with('"')
        {
            let value = digits.parse::<u64>().ok()?;
            return Some(if digits.len() == 13 {
                value / 1000
            } else {
                value
            });
        }
    }
    // Only explicit UTC dates are accepted; unknown version formats stay unknown.
    let bytes = text.as_bytes();
    if bytes.len() < 20
        || bytes[4] != b'-'
        || bytes[7] != b'-'
        || !matches!(bytes[10], b'T' | b' ')
        || bytes[13] != b':'
        || bytes[16] != b':'
    {
        return None;
    }
    let field = |start: usize, end: usize| -> Option<i64> {
        std::str::from_utf8(bytes.get(start..end)?)
            .ok()?
            .parse()
            .ok()
    };
    let (year, month, day, hour, minute, second) = (
        field(0, 4)?,
        field(5, 7)?,
        field(8, 10)?,
        field(11, 13)?,
        field(14, 16)?,
        field(17, 19)?,
    );
    let zone = &text[19..];
    if !matches!(zone, "Z" | "+00:00" | " UTC") {
        return None;
    }
    if !(1970..=9999).contains(&year)
        || !(1..=12).contains(&month)
        || !(0..=23).contains(&hour)
        || !(0..=59).contains(&minute)
        || !(0..=59).contains(&second)
    {
        return None;
    }
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let month_days = [
        31,
        if leap { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    if day < 1 || day > month_days[(month - 1) as usize] {
        return None;
    }
    // Gregorian civil date to days since 1970-01-01 (400-year eras).
    let y = year - i64::from(month <= 2);
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let m = month + if month > 2 { -3 } else { 9 };
    let doy = (153 * m + 2) / 5 + day - 1;
    let days = era * 146097 + yoe * 365 + yoe / 4 - yoe / 100 + doy - 719468;
    u64::try_from(days * 86400 + hour * 3600 + minute * 60 + second).ok()
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn normalizes_windows_paths_without_losing_unc_roots() {
        for (input, expected) in [
            (r"\\?\C:\Music\soloist.exe", r"C:\Music\soloist.exe"),
            (
                r"\\?\UNC\server\share\soloist.exe",
                r"\\server\share\soloist.exe",
            ),
            (r"C:\Music\soloist.exe", r"C:\Music\soloist.exe"),
            (r"\\server\share\soloist.exe", r"\\server\share\soloist.exe"),
        ] {
            assert_eq!(strip_verbatim(Path::new(input)), PathBuf::from(expected));
        }
    }
    #[test]
    fn parses_only_explicit_build_dates_and_handles_expiry_boundaries() {
        assert_eq!(timestamp("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(timestamp("2000-02-29T00:00:00Z"), Some(951782400));
        assert_eq!(timestamp("1900-02-29T00:00:00Z"), None);
        assert_eq!(timestamp("2026-02-29T00:00:00Z"), None);
        assert_eq!(timestamp("2026-01-01T00:00:00+03:00"), None);
        assert_eq!(
            build_timestamp("Soloist version 1.2.3\nBuild timestamp: 1700000000000"),
            Some(1700000000)
        );
        assert_eq!(build_timestamp("Soloist copyright 2026"), None);
        assert_eq!(
            build_timestamp("Build timestamp: 1700000000\nBuild date: 1800000000"),
            None
        );
        assert_eq!(
            build_timestamp(
                "soloist 1.3.10.21 build 1793124080 (20261027) (g19412df6ea) (windows/x86_64)"
            ),
            Some(1793124080)
        );
        let mut build = Build {
            executable: PathBuf::new(),
            sha256: String::new(),
            version_output: String::new(),
            build_timestamp: Some(1700000000),
            expires_at: None,
            days_remaining: None,
            expiry: String::new(),
        };
        build.at(1700000000 + LIFETIME - 1);
        assert_eq!(build.expiry, "expiring-soon");
        build.at(1700000000 + LIFETIME);
        assert!(!build.usable());
        assert_eq!(build.days_remaining, Some(0));
        build.build_timestamp = None;
        build.at(1700000000);
        assert_eq!(build.expiry, "unknown");
        assert!(build.usable());
    }
}
