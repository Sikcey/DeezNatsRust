//! Read the existing current-user DPAPI session only after an explicit Connect.
use std::{
    path::PathBuf,
    ptr::{null, null_mut},
};
use windows_sys::Win32::{Foundation::LocalFree, Security::Cryptography::*};
pub(crate) fn deezer_saved_exists() -> bool {
    if crate::storage::data_dir().is_ok_and(|dir| dir.join("deezer-signed-out").exists()) {
        return false;
    }
    if crate::storage::data_dir().is_ok_and(|dir| dir.join("session.dat").is_file()) {
        return true;
    }
    if let Some(dir) = std::env::var_os("DEEZNATS_PROFILE") {
        return PathBuf::from(dir).join("session.dat").is_file();
    }
    std::env::var_os("LOCALAPPDATA").is_some_and(|base| {
        let base = PathBuf::from(base);
        base.join("DeezNats").join("session.dat").is_file()
            || base.join("DeezerNative").join("session.dat").is_file()
    })
}

pub fn saved() -> Result<String, String> {
    if crate::storage::data_dir()?
        .join("deezer-signed-out")
        .exists()
    {
        return Err("No saved session found. Paste your ARL below and choose Connect.".into());
    }
    let base =
        PathBuf::from(std::env::var_os("LOCALAPPDATA").ok_or("User data directory unavailable")?);
    let own = crate::storage::data_dir()?.join("session.dat");
    if own.exists() {
        return read(own);
    }
    if let Some(directory) = std::env::var_os("DEEZNATS_PROFILE") {
        return read(PathBuf::from(directory).join("session.dat"));
    }
    let preferred = base.join("DeezNats");
    let directory = if preferred.exists() {
        preferred
    } else {
        base.join("DeezerNative")
    };
    read(directory.join("session.dat"))
}
pub fn save(session: &str) -> Result<(), String> {
    save_file("session.dat", session)?;
    remove_if_present(&crate::storage::data_dir()?.join("deezer-signed-out"))
}
pub(crate) fn logout() -> Result<(), String> {
    let dir = crate::storage::data_dir()?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    // Prevent legacy profiles from being silently imported again after sign-out.
    std::fs::write(dir.join("deezer-signed-out"), []).map_err(|e| e.to_string())?;
    remove_if_present(&dir.join("session.dat"))
}
fn remove_if_present(path: &std::path::Path) -> Result<(), String> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.to_string()),
    }
}
fn save_file(file: &str, session: &str) -> Result<(), String> {
    let directory = if file == "youtube-lyrics.dat" {
        youtube_session_dir()?
    } else {
        crate::storage::data_dir()?
    };
    std::fs::create_dir_all(&directory).map_err(|e| e.to_string())?;
    let mut bytes = session.as_bytes().to_vec();
    let input = CRYPT_INTEGER_BLOB {
        cbData: bytes.len() as u32,
        pbData: bytes.as_mut_ptr(),
    };
    let mut output = CRYPT_INTEGER_BLOB {
        cbData: 0,
        pbData: null_mut(),
    };
    unsafe {
        if CryptProtectData(
            &input,
            null(),
            null(),
            null(),
            null(),
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut output,
        ) == 0
        {
            return Err("Windows could not protect the session".into());
        }
        let protected = std::slice::from_raw_parts(output.pbData, output.cbData as usize);
        let result = if file.starts_with("spotify") || file == "youtube-lyrics.dat" {
            atomic_write(&directory.join(file), protected)
        } else {
            std::fs::write(directory.join(file), protected).map_err(|e| e.to_string())
        };
        LocalFree(output.pbData.cast());
        bytes.fill(0);
        result
    }
}
fn read(path: PathBuf) -> Result<String, String> {
    let mut bytes = std::fs::read(path).map_err(|_| {
        "No saved session found. Paste your ARL below and choose Connect.".to_string()
    })?;
    let input = CRYPT_INTEGER_BLOB {
        cbData: bytes.len() as u32,
        pbData: bytes.as_mut_ptr(),
    };
    let mut output = CRYPT_INTEGER_BLOB {
        cbData: 0,
        pbData: null_mut(),
    };
    unsafe {
        if CryptUnprotectData(
            &input,
            null_mut(),
            null(),
            null(),
            null(),
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut output,
        ) == 0
        {
            return Err(
                "Windows could not unlock the saved session. Paste your ARL to sign in.".into(),
            );
        }
        let data = std::slice::from_raw_parts_mut(output.pbData, output.cbData as usize);
        let result =
            String::from_utf8(data.to_vec()).map_err(|_| "Saved session is invalid".to_string());
        data.fill(0);
        LocalFree(output.pbData.cast());
        result
    }
}

fn provider_path(name: &str) -> Result<PathBuf, String> {
    if !matches!(
        name,
        "musixmatch"
            | "spotify"
            | "spotify-soloist"
            | "spotify-web"
            | "spotify-web-api"
            | "youtube"
    ) {
        return Err("Unknown credential provider".into());
    }
    let directory = if name == "youtube" {
        youtube_session_dir()?
    } else {
        crate::storage::data_dir()?
    };
    Ok(directory.join(format!("{name}-lyrics.dat")))
}
fn youtube_session_dir() -> Result<PathBuf, String> {
    Ok(
        PathBuf::from(std::env::var_os("LOCALAPPDATA").ok_or("User data directory unavailable")?)
            .join("DeezNatsRust"),
    )
}
pub(crate) fn provider_exists(name: &str) -> bool {
    provider_path(name).is_ok_and(|path| path.is_file())
}
pub fn provider_saved(name: &str) -> Result<String, String> {
    read(provider_path(name)?).map_err(|_| "No protected provider session is available".into())
}
pub fn provider_save(name: &str, value: &str) -> Result<(), String> {
    let path = provider_path(name)?;
    save_file(
        path.file_name()
            .and_then(|s| s.to_str())
            .ok_or("Invalid provider file")?,
        value,
    )
}
pub fn provider_remove(name: &str) -> Result<(), String> {
    match std::fs::remove_file(provider_path(name)?) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err("Could not remove provider session".into()),
    }
}

// Replace protected credentials only after the complete new file reaches disk.
fn atomic_write(path: &std::path::Path, bytes: &[u8]) -> Result<(), String> {
    use std::{
        io::Write,
        sync::atomic::{AtomicU64, Ordering},
    };
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let temporary = path.with_extension(format!(
        "{}.{}.tmp",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let mut file = std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&temporary)
        .map_err(|_| "Could not stage protected credentials")?;
    let result = (|| {
        file.write_all(bytes)
            .and_then(|_| file.sync_all())
            .map_err(|_| "Could not write protected credentials")?;
        drop(file);
        std::fs::rename(&temporary, path)
            .map_err(|_| "Could not replace protected credentials".to_owned())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result
}

#[cfg(test)]
mod spotify_tests {
    use super::*;
    #[test]
    fn spotify_atomic_replace_writes_complete_credentials() {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory = std::env::temp_dir().join(format!(
            "deeznats-spotify-session-{}-{nonce}",
            std::process::id()
        ));
        std::fs::create_dir(&directory).unwrap();
        let file = directory.join("session.dat");
        atomic_write(&file, b"old protected value").unwrap();
        atomic_write(&file, b"new protected value").unwrap();
        assert_eq!(std::fs::read(&file).unwrap(), b"new protected value");
        assert_eq!(std::fs::read_dir(&directory).unwrap().count(), 1);
        std::fs::remove_file(file).unwrap();
        std::fs::remove_dir(directory).unwrap();
    }
}
