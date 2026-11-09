//! Optional global shortcuts. Hardware media/volume keys keep their Windows behavior.
use windows_sys::Win32::{
    Foundation::HWND,
    UI::Input::KeyboardAndMouse::{
        GetKeyState, RegisterHotKey, UnregisterHotKey, MOD_ALT, MOD_CONTROL, MOD_NOREPEAT,
        MOD_SHIFT, MOD_WIN,
    },
};

pub const ACTIONS: [&str; 7] = [
    "Play / pause",
    "Previous track",
    "Next track",
    "Stop",
    "Volume up",
    "Volume down",
    "Mute",
];
pub const DEFAULTS: [&str; 7] = [
    "Media play / pause",
    "Media previous",
    "Media next",
    "Media stop",
    "Volume up",
    "Volume down",
    "Volume mute",
];
pub type Bindings = [Option<Binding>; 7];

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Binding {
    pub modifiers: u32,
    pub key: u32,
}

impl Binding {
    pub fn valid(self) -> bool {
        self.modifiers & !(MOD_ALT | MOD_CONTROL | MOD_SHIFT | MOD_WIN) == 0
            && (self.modifiers & (MOD_ALT | MOD_CONTROL | MOD_WIN) != 0
                || (0x70..=0x87).contains(&self.key))
            && (0x20..=0xfe).contains(&self.key)
            && !matches!(self.key, 0x5b | 0x5c | 0xa0..=0xa5 | 0xad..=0xb3 | 0xe7)
    }

    pub fn label(self) -> String {
        let mut parts = Vec::new();
        for (flag, label) in [
            (MOD_CONTROL, "Ctrl"),
            (MOD_ALT, "Alt"),
            (MOD_SHIFT, "Shift"),
            (MOD_WIN, "Win"),
        ] {
            if self.modifiers & flag != 0 {
                parts.push(label.to_owned());
            }
        }
        let key = match self.key {
            0x20 => "Space".into(),
            0x21 => "Page Up".into(),
            0x22 => "Page Down".into(),
            0x23 => "End".into(),
            0x24 => "Home".into(),
            0x25 => "Left".into(),
            0x26 => "Up".into(),
            0x27 => "Right".into(),
            0x28 => "Down".into(),
            0x2d => "Insert".into(),
            0x2e => "Delete".into(),
            0x30..=0x39 | 0x41..=0x5a => char::from_u32(self.key).unwrap().to_string(),
            0x70..=0x87 => format!("F{}", self.key - 0x6f),
            // Use the current keyboard layout for punctuation/numpad keys.
            key => unsafe {
                use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
                    GetKeyNameTextW, MapVirtualKeyW, MAPVK_VK_TO_VSC,
                };
                let mut text = [0u16; 64];
                let scan = MapVirtualKeyW(key, MAPVK_VK_TO_VSC);
                let len =
                    GetKeyNameTextW((scan << 16) as i32, text.as_mut_ptr(), text.len() as i32);
                if len > 0 {
                    String::from_utf16_lossy(&text[..len as usize])
                } else {
                    format!("Key {key:#x}")
                }
            },
        };
        parts.push(key);
        parts.join(" + ")
    }
}

pub fn current_binding(key: u32) -> Binding {
    let mut modifiers = 0;
    for (vk, flag) in [
        (0x11, MOD_CONTROL),
        (0x12, MOD_ALT),
        (0x10, MOD_SHIFT),
        (0x5b, MOD_WIN),
        (0x5c, MOD_WIN),
    ] {
        if unsafe { GetKeyState(vk) } < 0 {
            modifiers |= flag;
        }
    }
    Binding { modifiers, key }
}

pub struct Shortcuts {
    hwnd: HWND,
    registered: [Option<(i32, Binding)>; 7],
}
impl Shortcuts {
    pub fn new(hwnd: HWND) -> Self {
        Self {
            hwnd,
            registered: [None; 7],
        }
    }

    pub fn action(&self, id: usize) -> Option<usize> {
        self.registered
            .iter()
            .position(|entry| entry.is_some_and(|(registered, _)| registered as usize == id))
    }

    pub fn set(&mut self, action: usize, binding: Option<Binding>) -> Result<(), String> {
        let Some(old) = self.registered.get(action).copied() else {
            return Err("Unknown shortcut action.".into());
        };
        if binding.is_some_and(|binding| !binding.valid()) {
            return Err("Use Ctrl, Alt or Win with a key, or a function key. Media keys work automatically.".into());
        }
        if old.map(|(_, binding)| binding) == binding {
            return Ok(());
        }
        let replacement = if let Some(binding) = binding {
            if self.registered.iter().enumerate().any(|(index, entry)| {
                index != action && entry.is_some_and(|(_, other)| other == binding)
            }) {
                return Err("That shortcut is already assigned to another action.".into());
            }
            // Reserve the new combination before releasing the old one. A conflict
            // with another application leaves the existing shortcut intact.
            let first = 0x500 + action as i32;
            let id = if old.is_some_and(|(id, _)| id == first) {
                first + 16
            } else {
                first
            };
            if unsafe {
                RegisterHotKey(self.hwnd, id, binding.modifiers | MOD_NOREPEAT, binding.key)
            } == 0
            {
                return Err("Windows could not register this shortcut. It may be reserved or used by another app.".into());
            }
            Some((id, binding))
        } else {
            None
        };
        if let Some((id, _)) = old {
            unsafe {
                UnregisterHotKey(self.hwnd, id);
            }
        }
        self.registered[action] = replacement;
        Ok(())
    }

    pub fn clear(&mut self) {
        for entry in &mut self.registered {
            if let Some((id, _)) = entry.take() {
                unsafe {
                    UnregisterHotKey(self.hwnd, id);
                }
            }
        }
    }
}
impl Drop for Shortcuts {
    fn drop(&mut self) {
        self.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unsafe_and_media_key_bindings_are_not_registered() {
        for key in [0x41, 0xad, 0xae, 0xaf, 0xb0, 0xb1, 0xb2, 0xb3] {
            assert!(!Binding { modifiers: 0, key }.valid());
        }
        assert!(!Binding {
            modifiers: MOD_CONTROL,
            key: 0xb3
        }
        .valid());
        assert!(Binding {
            modifiers: MOD_CONTROL | MOD_SHIFT,
            key: 0x50
        }
        .valid());
        assert!(Binding {
            modifiers: 0,
            key: 0x78
        }
        .valid());
    }
    #[test]
    fn bindings_roundtrip_and_labels_are_readable() {
        let binding = Binding {
            modifiers: MOD_CONTROL | MOD_ALT,
            key: 0x20,
        };
        assert_eq!(binding.label(), "Ctrl + Alt + Space");
        let mut bindings: Bindings = [None; 7];
        bindings[0] = Some(binding);
        assert_eq!(
            serde_json::from_str::<Bindings>(&serde_json::to_string(&bindings).unwrap()).unwrap(),
            bindings
        );
    }

    #[test]
    #[ignore = "temporarily registers global shortcuts on the interactive Windows desktop"]
    fn registration_conflict_preserves_previous_and_cleanup_releases_keys() {
        use std::sync::mpsc;
        let (ready_tx, ready_rx) = mpsc::channel();
        let (done_tx, done_rx) = mpsc::channel();
        let competitor = std::thread::spawn(move || {
            let mut keys = Shortcuts::new(std::ptr::null_mut());
            let binding = (0x7c..=0x87)
                .map(|key| Binding {
                    modifiers: MOD_CONTROL | MOD_ALT | MOD_SHIFT,
                    key,
                })
                .find(|binding| keys.set(0, Some(*binding)).is_ok())
                .expect("free function-key combination");
            ready_tx.send(binding).unwrap();
            done_rx
                .recv_timeout(std::time::Duration::from_secs(10))
                .unwrap();
        });
        let occupied = ready_rx
            .recv_timeout(std::time::Duration::from_secs(10))
            .unwrap();
        let mut keys = Shortcuts::new(std::ptr::null_mut());
        let original = (0x7c..=0x87)
            .map(|key| Binding {
                modifiers: MOD_CONTROL | MOD_ALT | MOD_SHIFT,
                key,
            })
            .filter(|binding| *binding != occupied)
            .find(|binding| keys.set(0, Some(*binding)).is_ok())
            .expect("second free function-key combination");
        let registered = keys.registered[0];
        assert!(keys.set(0, Some(occupied)).is_err());
        assert_eq!(keys.registered[0], registered);
        assert!(keys.set(1, Some(original)).is_err());
        done_tx.send(()).unwrap();
        competitor.join().unwrap();
        keys.set(0, Some(occupied)).unwrap();
        keys.clear();
        assert!(keys.registered.iter().all(Option::is_none));
        keys.set(1, Some(occupied)).unwrap();
    }
}
