//! Shared C ABI of the original Go/WASAPI engine. The library is never unloaded
//! because its Go runtime may retain threads until process exit.
use libloading::Library;
use std::{
    ffi::{c_char, CStr, CString},
    mem::ManuallyDrop,
    path::PathBuf,
};
#[path = "engine_api.rs"]
pub mod api;
pub(crate) struct Engine {
    pub(crate) library: ManuallyDrop<Library>,
    pub(crate) init: unsafe extern "C" fn(*const c_char) -> i32,
    pub(crate) play: unsafe extern "C" fn(*const c_char, i64) -> i32,
    pub(crate) toggle: unsafe extern "C" fn(),
    pub(crate) stop: unsafe extern "C" fn(),
    pub(crate) seek: unsafe extern "C" fn(i64),
    pub(crate) volume: unsafe extern "C" fn(f64),
    pub(crate) state: unsafe extern "C" fn() -> i32,
    pub(crate) position: unsafe extern "C" fn() -> i64,
    pub(crate) duration: unsafe extern "C" fn() -> i64,
    pub(crate) error: unsafe extern "C" fn() -> *mut c_char,
    pub(crate) free: unsafe extern "C" fn(*mut c_char),
    pub(crate) login_error: unsafe extern "C" fn() -> i32,
    pub(crate) logout: unsafe extern "C" fn(),
}
impl Engine {
    pub(crate) fn apply_eq(&self, args: &[serde_json::Value]) -> Result<serde_json::Value, String> {
        let update = normalize_eq(args)?;
        if self
            .invoke("DZSetEQJSON", &[serde_json::json!(update.to_string())])?
            .as_i64()
            != Some(1)
        {
            return Err("The audio engine rejected the equalizer update".into());
        }
        self.invoke("DZEQJSON", &[])
    }

    pub(crate) fn load() -> Result<Self, String> {
        // Explicit absolute path only: do not search the working directory/PATH.
        let path = std::env::var_os("DEEZNATS_ENGINE")
            .map(PathBuf::from)
            .unwrap_or(
                std::env::current_exe()
                    .map_err(|e| e.to_string())?
                    .with_file_name("libdeezercore.dll"),
            );
        if !path.is_absolute() {
            return Err("DEEZNATS_ENGINE must be an absolute DLL path".into());
        }
        unsafe {
            let library = ManuallyDrop::new(
                Library::new(&path)
                    .map_err(|e| format!("Engine unavailable: {e}. Local files still work."))?,
            );
            macro_rules! symbol {
                ($name:literal) => {
                    *library
                        .get(concat!($name, "\0").as_bytes())
                        .map_err(|e| e.to_string())?
                };
            }
            let config: unsafe extern "C" fn(*const c_char) = symbol!("DZSetConfigDir");
            let directory = crate::storage::data_dir()?;
            std::fs::create_dir_all(&directory).map_err(|e| e.to_string())?;
            let initialize_cache = !directory.join("media.json").exists();
            let directory =
                CString::new(directory.to_string_lossy().as_bytes()).map_err(|e| e.to_string())?;
            config(directory.as_ptr());
            let engine = Self {
                init: symbol!("DZInit"),
                play: symbol!("DZPlay"),
                toggle: symbol!("DZTogglePause"),
                stop: symbol!("DZStop"),
                seek: symbol!("DZSeek"),
                volume: symbol!("DZSetVolume"),
                state: symbol!("DZState"),
                position: symbol!("DZPositionMS"),
                duration: symbol!("DZDurationMS"),
                error: symbol!("DZLastErrorJSON"),
                free: symbol!("DZFree"),
                login_error: symbol!("DZLoginErrorKind"),
                logout: symbol!("DZLogout"),
                library,
            };
            if initialize_cache {
                engine.invoke("DZSetMediaCacheMB", &[serde_json::json!(1024)])?;
            }
            engine.invoke("DZSetSongDiskBuffering", &[serde_json::json!(1)])?;
            Ok(engine)
        }
    }
    pub(crate) fn error(&self) -> String {
        unsafe {
            let ptr = (self.error)();
            if ptr.is_null() {
                return "Playback failed".into();
            }
            let message = CStr::from_ptr(ptr).to_string_lossy().into_owned();
            (self.free)(ptr);
            message
        }
    }
}

fn normalize_eq(args: &[serde_json::Value]) -> Result<serde_json::Value, String> {
    use serde_json::{json, Value};
    if args.len() != 1 {
        return Err("Equalizer update requires one JSON argument".into());
    }
    let mut value: Value = serde_json::from_str(
        args[0]
            .as_str()
            .ok_or("Equalizer update must be JSON text")?,
    )
    .map_err(|e| format!("Invalid equalizer update: {e}"))?;
    let object = value
        .as_object_mut()
        .ok_or("Equalizer update must be an object")?;
    for (key, value) in object.iter() {
        match key.as_str() {
            "enabled" | "mono" if value.is_boolean() => {}
            "preampDb" if value.as_f64().is_some_and(|v| (-12.0..=12.0).contains(&v)) => {}
            "gainsDb"
                if value.as_array().is_some_and(|items| {
                    items.len() == 10
                        && items
                            .iter()
                            .all(|v| v.as_f64().is_some_and(|n| (-12.0..=12.0).contains(&n)))
                }) => {}
            "preset" if value.is_string() => {}
            "band"
                if value["index"].as_u64().is_some_and(|i| i < 10)
                    && value["gainDb"]
                        .as_f64()
                        .is_some_and(|n| (-12.0..=12.0).contains(&n)) => {}
            _ => return Err(format!("Invalid equalizer field: {key}")),
        }
    }
    if let Some(preset) = object.get("preset").and_then(Value::as_str) {
        let preset = preset.trim().to_ascii_lowercase().replace(' ', "-");
        if preset == "custom" {
            if !object.contains_key("gainsDb") && !object.contains_key("band") {
                return Err("Custom equalizer updates need band gains".into());
            }
            object.remove("preset");
        } else {
            if ![
                "flat",
                "bass-boost",
                "bass-reducer",
                "treble-boost",
                "vocal",
                "rock",
                "pop",
                "jazz",
                "classical",
                "electronic",
            ]
            .contains(&preset.as_str())
            {
                return Err("Unknown equalizer preset".into());
            }
            object.insert("preset".into(), json!(preset));
            // Choosing a preset should be audible unless the caller explicitly disables it.
            object.entry("enabled").or_insert(json!(true));
        }
    }
    Ok(value)
}

#[cfg(test)]
mod eq_tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn presets_enable_eq_and_custom_edits_use_engine_band_semantics() {
        assert_eq!(
            normalize_eq(&[json!(r#"{"preset":"Bass boost"}"#)]).unwrap(),
            json!({"preset":"bass-boost","enabled":true})
        );
        assert_eq!(
            normalize_eq(&[json!(
                r#"{"preset":"custom","band":{"index":2,"gainDb":3}}"#
            )])
            .unwrap(),
            json!({"band":{"index":2,"gainDb":3}})
        );
        assert_eq!(
            normalize_eq(&[json!(r#"{"preset":"flat","enabled":false}"#)]).unwrap()["enabled"],
            false
        );
    }
    #[test]
    fn invalid_eq_is_rejected_before_any_partial_engine_update() {
        for value in [
            json!({"preset":"missing","enabled":true}),
            json!({"gainsDb":[1,2]}),
            json!({"band":{"index":10,"gainDb":0}}),
            json!({"mono":1}),
            json!({"preampDb":50}),
            json!({"preset":"custom"}),
        ] {
            assert!(normalize_eq(&[json!(value.to_string())]).is_err());
        }
    }
}

impl Engine {
    pub(crate) unsafe fn take(&self, ptr: *mut c_char) -> Result<serde_json::Value, String> {
        if ptr.is_null() {
            return Err("Engine returned no data".into());
        }
        let text = CStr::from_ptr(ptr).to_string_lossy().into_owned();
        (self.free)(ptr);
        let value = serde_json::from_str::<serde_json::Value>(&text)
            .unwrap_or(serde_json::Value::String(text));
        if let Some(error) = value.get("error") {
            return Err(error
                .as_str()
                .map(str::to_owned)
                .unwrap_or_else(|| error.to_string()));
        }
        Ok(value)
    }
    pub fn fetch(&self, url: &str) -> Result<Vec<u8>, String> {
        let url = CString::new(url).map_err(|_| "Invalid URL")?;
        unsafe {
            let fetch = self
                .library
                .get::<unsafe extern "C" fn(*const c_char, *mut i32) -> *mut u8>(b"DZFetch\0")
                .map_err(|e| e.to_string())?;
            let free = self
                .library
                .get::<unsafe extern "C" fn(*mut u8)>(b"DZFreeBytes\0")
                .map_err(|e| e.to_string())?;
            let mut length = 0;
            let ptr = fetch(url.as_ptr(), &mut length);
            if ptr.is_null() {
                return Err("Deezer download failed".into());
            }
            if length <= 0 || length > 16 * 1024 * 1024 {
                free(ptr);
                return Err("Response exceeds the 16 MiB limit".into());
            }
            let bytes = std::slice::from_raw_parts(ptr, length as usize).to_vec();
            free(ptr);
            Ok(bytes)
        }
    }
}
