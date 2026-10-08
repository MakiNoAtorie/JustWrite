use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, path::PathBuf};

#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub theme: String,
    pub accent: String,
    pub editor_font: String,
    pub interface_font: String,
    pub code_font: String,
    pub show_markers: bool,
    pub font_size: f32,
    #[serde(default)]
    pub max_width: Option<f32>,
    pub typewriter: bool,
    pub autosave: bool,
    pub keymap: BTreeMap<String, String>,
    /// Whether Windows is currently in light mode; only used when `theme` is "system".
    #[serde(skip)]
    pub system_light: bool,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            theme: "dark".into(),
            accent: "#d9aa72".into(),
            editor_font: "Segoe UI".into(),
            interface_font: "Segoe UI".into(),
            code_font: "Consolas".into(),
            show_markers: false,
            font_size: 18.,
            max_width: Some(920.),
            typewriter: false,
            autosave: true,
            keymap: BTreeMap::new(),
            system_light: false,
        }
    }
}
impl Settings {
    pub fn parse(text: &str) -> Result<Self, String> {
        let settings: Self = toml::from_str(text).map_err(|e| e.to_string())?;
        if !["light", "dark", "system"].contains(&settings.theme.as_str()) {
            return Err("Theme must be light, dark or system".into());
        }
        if !(10.0..=48.0).contains(&settings.font_size) || !settings.font_size.is_finite() {
            return Err("Font size must be between 10 and 48".into());
        }
        if settings
            .max_width
            .is_some_and(|w| !w.is_finite() || !(240.0..=10000.0).contains(&w))
        {
            return Err("Maximum width must be between 240 and 10000 pixels".into());
        }
        if settings.accent.len() != 7
            || !settings.accent.starts_with('#')
            || u32::from_str_radix(&settings.accent[1..], 16).is_err()
        {
            return Err("Accent must be a hex color, such as #d9aa72".into());
        }
        for shortcut in settings.keymap.values() {
            super::commands::parse_shortcut(shortcut)?;
        }
        Ok(settings)
    }
    pub fn serialize(&self) -> String {
        toml::to_string_pretty(self).expect("Serialize validated settings")
    }
    pub fn light(&self) -> bool {
        self.theme == "light" || (self.theme == "system" && self.system_light)
    }
    pub fn accent_color(&self) -> u32 {
        u32::from_str_radix(self.accent.trim_start_matches('#'), 16).unwrap_or(0xd9aa72)
    }
}

#[derive(Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Session {
    pub workspace: Option<PathBuf>,
    pub split: Option<SessionSplit>,
    pub tree_width: f32,
    pub outline_width: f32,
    pub show_tree: bool,
    pub show_outline: bool,
    /// Recently opened workspace folders, newest first.
    pub recent: Vec<PathBuf>,
    pub window: Option<WindowState>,
}
/// Outer window position and inner size in physical pixels.
#[derive(Clone, Copy, Serialize, Deserialize)]
pub struct WindowState {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub maximized: bool,
}
#[derive(Serialize, Deserialize)]
pub enum SessionSplit {
    Pane {
        tabs: Vec<SessionTab>,
        active: usize,
    },
    Split {
        down: bool,
        ratio: f32,
        first: Box<SessionSplit>,
        second: Box<SessionSplit>,
    },
}
#[derive(Serialize, Deserialize)]
pub struct SessionTab {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub root: Option<PathBuf>,
    pub path: PathBuf,
    pub cursor: usize,
    pub top: usize,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn settings_and_device_session_round_trip_separately() {
        let mut settings = Settings::default();
        settings.max_width = None;
        settings.keymap.insert("bold".into(), "Ctrl+Alt+B".into());
        let parsed = Settings::parse(&settings.serialize()).unwrap();
        assert!(parsed.max_width.is_none());
        assert_eq!(parsed.keymap["bold"], "Ctrl+Alt+B");
        settings.accent = "invalid".into();
        assert!(Settings::parse(&settings.serialize()).is_err());
        let session = Session {
            workspace: Some(PathBuf::from("C:/notes")),
            split: Some(SessionSplit::Split {
                down: false,
                ratio: 0.4,
                first: Box::new(SessionSplit::Pane {
                    active: 0,
                    tabs: vec![SessionTab {
                        root: None,
                        path: "note.md".into(),
                        cursor: 42,
                        top: 12,
                    }],
                }),
                second: Box::new(SessionSplit::Pane {
                    active: 0,
                    tabs: vec![],
                }),
            }),
            ..Session::default()
        };
        let source = toml::to_string_pretty(&session).unwrap();
        let restored: Session = toml::from_str(&source).unwrap();
        assert!(matches!(
            restored.split,
            Some(SessionSplit::Split { down: false, .. })
        ));
        assert!(!source.contains("editor_font"));
    }
}
