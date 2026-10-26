//! Read the existing current-user DPAPI session only after an explicit Connect.
use std::{
    path::PathBuf,
    ptr::{null, null_mut},
};
use windows_sys::Win32::{Foundation::LocalFree, Security::Cryptography::*};
pub fn saved() -> Result<String, String> {
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
    save_file("session.dat", session)
}
fn save_file(file: &str, session: &str) -> Result<(), String> {
    let directory = crate::storage::data_dir()?;
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
        let result = std::fs::write(directory.join(file), protected).map_err(|e| e.to_string());
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
    if !matches!(name, "musixmatch") {
        return Err("Unknown credential provider".into());
    }
    Ok(
        crate::storage::data_dir()?.join(format!("{name}-lyrics.dat")),
    )
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
