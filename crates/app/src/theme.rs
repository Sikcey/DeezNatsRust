//! Portable appearance-only themes. No account, playback or local-path data.
use crate::ui::Ui;
use serde_json::{json, Value};
use std::{collections::HashMap, io::Read, path::Path};

const LIMIT: usize = 64 * 1024;
const COLORS: &[&str] = &[
    "Background",
    "Panel",
    "Raised",
    "Border",
    "Text",
    "Muted",
    "Icon",
    "Accent",
    "AccentText",
    "TextHover",
    "GradientStart",
    "GradientEnd",
];
pub struct Theme {
    value: Value,
}
fn hex(color: u32) -> String {
    format!(
        "#{:02X}{:02X}{:02X}",
        color & 255,
        (color >> 8) & 255,
        (color >> 16) & 255
    )
}
fn color(value: &Value) -> Result<u32, String> {
    let text = value
        .as_str()
        .ok_or("Theme colors must be #RRGGBB strings")?;
    if text.len() != 7
        || !text.starts_with('#')
        || !text[1..].bytes().all(|c| c.is_ascii_hexdigit())
    {
        return Err("Theme colors must use #RRGGBB".into());
    }
    let rgb = u32::from_str_radix(&text[1..], 16).map_err(|_| "Invalid theme color")?;
    Ok(((rgb >> 16) & 255) | (rgb & 0xff00) | ((rgb & 255) << 16))
}
fn palette(value: &Value) -> Result<HashMap<String, u32>, String> {
    let map = value.as_object().ok_or("Theme palette must be an object")?;
    if map.len() > COLORS.len() {
        return Err("Too many theme palette entries".into());
    }
    map.iter()
        .map(|(key, value)| {
            if !COLORS.contains(&key.as_str()) {
                return Err(format!("Unknown theme color: {key}"));
            }
            Ok((key.clone(), color(value)?))
        })
        .collect()
}
impl Theme {
    pub fn capture(ui: &Ui) -> Self {
        let encode = |map: &HashMap<String, u32>| {
            map.iter()
                .filter(|(k, _)| COLORS.contains(&k.as_str()))
                .map(|(key, value)| (key.clone(), json!(hex(*value))))
                .collect::<serde_json::Map<_, _>>()
        };
        Self {
            value: json!({"format":"deeznats-theme","version":1,"name":"Custom theme","light":ui.light,"adaptive_colors":ui.adaptive_colors,
            "background":{"mode":(["acrylic","artwork","image","solid"][ui.background_mode.min(3)]),
                "blur":ui.blur,"tint_opacity":ui.light_opacity,"tint":hex(ui.light_tint),
                "color":hex(ui.solid_color),"brightness":ui.brightness},
            "colors":{"dark":encode(&ui.dark_colors),"light":encode(&ui.light_colors)},
            "corners":ui.corner_style,"solid_theme":ui.solid_theme,"window_blur":ui.window_blur,"ui_blur":ui.ui_blur,
            "player":{"layout":ui.player_layout,"transparent":ui.transparent_player,"opacity":ui.surface_opacity}}),
        }
    }
    pub fn parse(bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() > LIMIT {
            return Err("Theme file exceeds 64 KiB".into());
        }
        let value: Value = serde_json::from_slice(bytes).map_err(|_| "Invalid theme JSON")?;
        if value["format"] != "deeznats-theme" || value["version"] != 1 {
            return Err("Unsupported DeezNats theme format/version".into());
        }
        let name = value["name"].as_str().ok_or("Theme name is required")?;
        if name.trim().is_empty() || name.chars().count() > 80 || name.chars().any(char::is_control)
        {
            return Err("Invalid theme name".into());
        }
        let theme = Self { value };
        // Validate the complete document before touching the active appearance.
        theme.apply_fields(&mut Ui::default())?;
        Ok(theme)
    }
    pub fn read(path: &Path) -> Result<Self, String> {
        let file = std::fs::File::open(path).map_err(|e| format!("Cannot open theme: {e}"))?;
        let mut bytes = Vec::new();
        file.take((LIMIT + 1) as u64)
            .read_to_end(&mut bytes)
            .map_err(|e| format!("Cannot read theme: {e}"))?;
        Self::parse(&bytes)
    }
    pub fn write(&self, path: &Path) -> Result<(), String> {
        let bytes = serde_json::to_vec_pretty(&self.value).map_err(|_| "Cannot encode theme")?;
        std::fs::write(path, bytes).map_err(|e| format!("Cannot save theme: {e}"))
    }
    pub fn name(&self) -> &str {
        self.value["name"].as_str().unwrap_or("Custom theme")
    }
    pub fn apply(&self, ui: &mut Ui) {
        self.apply_fields(ui).expect("validated theme");
    }
    fn apply_fields(&self, ui: &mut Ui) -> Result<(), String> {
        let v = &self.value;
        let number = |pointer: &str, max: u64| -> Result<u32, String> {
            v.pointer(pointer)
                .and_then(Value::as_u64)
                .filter(|n| *n <= max)
                .map(|n| n as u32)
                .ok_or_else(|| format!("Invalid theme value: {pointer}"))
        };
        let boolean = |pointer: &str| {
            v.pointer(pointer)
                .and_then(Value::as_bool)
                .ok_or_else(|| format!("Invalid theme flag: {pointer}"))
        };
        let mode = match v["background"]["mode"].as_str() {
            Some("acrylic") => 0,
            Some("artwork") => 1,
            Some("image") => 2,
            Some("solid") => 3,
            _ => return Err("Invalid theme background mode".into()),
        };
        let dark = palette(&v["colors"]["dark"])?;
        let light = palette(&v["colors"]["light"])?;
        ui.light = boolean("/light")?;
        ui.blur = number("/background/blur", 100)?;
        ui.light_opacity = number("/background/tint_opacity", 100)?;
        ui.brightness = number("/background/brightness", 110)?;
        ui.light_tint = color(&v["background"]["tint"])?;
        ui.solid_color = color(&v["background"]["color"])?;
        ui.corner_style = number("/corners", 3)? as usize;
        ui.solid_theme =
            number("/solid_theme", (crate::ui::SOLID_THEMES.len() - 1) as u64)? as usize;
        ui.window_blur = boolean("/window_blur")?;
        ui.ui_blur = boolean("/ui_blur")?;
        ui.adaptive_colors = v["adaptive_colors"].as_bool().unwrap_or(false);
        ui.player_layout = number("/player/layout", 1)? as usize;
        ui.transparent_player = boolean("/player/transparent")?;
        ui.surface_opacity = number("/player/opacity", 100)?;
        ui.dark_colors = dark;
        ui.light_colors = light;
        ui.background_mode = mode;
        ui.solid = mode != 0;
        ui.choices.insert("Background", mode);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bundled_midnight_theme_is_importable() {
        let theme = Theme::parse(include_bytes!(
            "../../../themes/Midnight.deeznats-theme.json"
        ))
        .unwrap();
        let mut ui = Ui::default();
        theme.apply(&mut ui);
        assert_eq!(theme.name(), "Midnight");
        assert_eq!(ui.background_mode, 3);
        assert_eq!(ui.color("Accent", 0), 0x00ff9ab5);
    }
    #[test]
    fn portable_theme_roundtrip_keeps_colors_and_leaves_account_and_paths_alone() {
        let mut source = Ui::default();
        source.dark_colors.insert("Accent".into(), 0x00ab3412);
        source.light = true;
        source.adaptive_colors = true;
        source.background_mode = 1;
        source.blur = 37;
        source.background_file = "private-wallpaper.png".into();
        source.account = json!({"private":"account-data"});
        let bytes = serde_json::to_vec(&Theme::capture(&source).value).unwrap();
        let text = String::from_utf8(bytes.clone()).unwrap();
        assert!(!text.contains("private") && !text.contains("account-data"));
        let theme = Theme::parse(&bytes).unwrap();
        let mut target = Ui::default();
        target.background_file = "existing.png".into();
        target.initial_volume = 73;
        theme.apply(&mut target);
        assert_eq!(target.dark_colors["Accent"], 0x00ab3412);
        assert!(target.light);
        assert!(target.adaptive_colors);
        assert_eq!(target.blur, 37);
        assert_eq!(target.background_mode, 1);
        assert_eq!(target.background_file, "existing.png");
        assert_eq!(target.initial_volume, 73);
    }
    #[test]
    fn malformed_or_oversized_themes_are_rejected_before_application() {
        let base = Theme::capture(&Ui::default()).value;
        for (pointer, bad) in [
            ("/version", json!(999)),
            ("/background/blur", json!(101)),
            ("/colors/dark", json!({"Accent":"red"})),
            ("/player/layout", json!(-1)),
            ("/window_blur", json!("yes")),
        ] {
            let mut value = base.clone();
            *value.pointer_mut(pointer).unwrap() = bad;
            assert!(Theme::parse(&serde_json::to_vec(&value).unwrap()).is_err());
        }
        assert!(Theme::parse(&vec![b' '; LIMIT + 1]).is_err());
    }
}
