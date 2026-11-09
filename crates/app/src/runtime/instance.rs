//! One GUI owns provider cookie renewal and persistence for this desktop session.
use windows_sys::Win32::{
    Foundation::{CloseHandle, GetLastError, ERROR_ALREADY_EXISTS, HANDLE},
    System::Threading::CreateMutexW,
    UI::WindowsAndMessaging::{
        FindWindowW, IsIconic, SetForegroundWindow, ShowWindow, SW_RESTORE, SW_SHOW,
    },
};

pub(crate) struct Guard(HANDLE);
impl Drop for Guard {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.0);
        }
    }
}

fn acquire_named(name: &str) -> Result<Option<Guard>, String> {
    let name: Vec<u16> = name.encode_utf16().chain(Some(0)).collect();
    unsafe {
        let handle = CreateMutexW(std::ptr::null(), 0, name.as_ptr());
        if handle.is_null() {
            return Err("Could not reserve the app session".into());
        }
        let existed = GetLastError() == ERROR_ALREADY_EXISTS;
        let guard = Guard(handle);
        Ok(if existed { None } else { Some(guard) })
    }
}

pub(crate) fn acquire() -> Result<Option<Guard>, String> {
    if cfg!(debug_assertions) && std::env::var_os("DEEZNATS_ISOLATED_DEBUG").is_some() {
        return acquire_named("Local\\DeezNatsRust.IsolatedDebug");
    }
    let guard = acquire_named("Local\\DeezNatsRust.GuiSession")?;
    unsafe {
        let class: Vec<u16> = "DeezNatsRustWindow".encode_utf16().chain(Some(0)).collect();
        let existing = FindWindowW(class.as_ptr(), std::ptr::null());
        // Also recognizes older running builds which did not hold the mutex.
        if !existing.is_null() {
            ShowWindow(
                existing,
                if IsIconic(existing) != 0 {
                    SW_RESTORE
                } else {
                    SW_SHOW
                },
            );
            SetForegroundWindow(existing);
            return Ok(None);
        }
    }
    Ok(guard)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_one_owner_until_the_last_handle_closes() {
        let name = format!("Local\\DeezNatsRust.TestInstance.{}", std::process::id());
        let owner = acquire_named(&name).unwrap().unwrap();
        assert!(acquire_named(&name).unwrap().is_none());
        drop(owner);
        assert!(acquire_named(&name).unwrap().is_some());
    }
}
