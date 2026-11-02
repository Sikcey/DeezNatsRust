//! Observe the Windows default used by Soloist's local audio stream.
use std::time::{Duration, Instant};
use windows::Win32::{
    Media::Audio::{eConsole, eRender, IMMDeviceEnumerator, MMDeviceEnumerator},
    System::Com::{
        CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize, CLSCTX_ALL,
        COINIT_MULTITHREADED,
    },
};

pub(super) struct Watcher {
    enumerator: Option<IMMDeviceEnumerator>,
    initialized: bool,
    endpoint: Option<String>,
    checked: Instant,
}
impl Watcher {
    pub fn new() -> Self {
        let initialized = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED).is_ok() };
        let enumerator = unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).ok() };
        let mut watcher = Self {
            enumerator,
            initialized,
            endpoint: None,
            checked: Instant::now(),
        };
        watcher.endpoint = watcher.read().ok().flatten();
        watcher
    }
    fn read(&self) -> Result<Option<String>, windows::core::Error> {
        let Some(enumerator) = &self.enumerator else {
            return Ok(None);
        };
        unsafe {
            let device = match enumerator.GetDefaultAudioEndpoint(eRender, eConsole) {
                Ok(device) => device,
                Err(error) if error.code().0 as u32 == 0x80070490 => return Ok(None),
                Err(error) => return Err(error),
            };
            let id = device.GetId()?;
            let result = id.to_string();
            CoTaskMemFree(Some(id.0.cast()));
            Ok(Some(result?))
        }
    }
    pub fn changed(&mut self) -> bool {
        if self.checked.elapsed() < Duration::from_secs(1) {
            return false;
        }
        self.checked = Instant::now();
        let Ok(endpoint) = self.read() else {
            return false;
        };
        observe(&mut self.endpoint, endpoint)
    }
}
fn observe(previous: &mut Option<String>, endpoint: Option<String>) -> bool {
    let changed = endpoint.is_some() && *previous != endpoint;
    *previous = endpoint;
    changed
}
impl Drop for Watcher {
    fn drop(&mut self) {
        self.enumerator.take();
        if self.initialized {
            unsafe { CoUninitialize() };
        }
    }
}
#[cfg(test)]
mod tests {
    use super::observe;
    #[test]
    fn detects_switch_and_reconnection_without_restarting_on_every_poll() {
        let mut previous = Some("speakers".into());
        assert!(!observe(&mut previous, Some("speakers".into())));
        assert!(observe(&mut previous, Some("headphones".into())));
        assert!(!observe(&mut previous, None));
        assert!(observe(&mut previous, Some("headphones".into())));
        assert!(!observe(&mut previous, Some("headphones".into())));
    }
}
