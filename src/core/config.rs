use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, path::PathBuf};

#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub theme: String,
    pub accent: String,
    pub editor_font: String,
    pub interface_font: String,
    /// Show formatting and note markers on every line, not just the one being edited.
    pub show_markers: bool,
    pub font_size: f32,
    /// Line height as a multiple of the font size.
    pub line_height: f32,
    /// Extra space between paragraphs, in lines of text.
    pub paragraph_gap: f32,
    #[serde(default)]
    pub max_width: Option<f32>,
    pub typewriter: bool,
    /// Justify all prose in the editor (notes excepted); left-aligned when off.
    #[serde(default)]
    pub justify: bool,
    /// Save chapters shortly after typing stops.
    pub autosave: bool,
    /// How often the `.jw` file is rewritten from the working copy, in minutes.
    pub repack_minutes: u32,
    /// The marker for notes, such as `$$`. Stored text does not depend on it.
    pub note_marker: String,
    /// Colour of notes (`#rrggbb`); empty for the accent colour.
    pub note_color: String,
    pub note_italic: bool,
    pub hide_notes: bool,
    /// Keep notes in the preview view. Off by default: the preview shows the text as exported.
    pub preview_notes: bool,
    /// Underline possible spelling mistakes.
    pub spellcheck: bool,
    /// Export preset the Export command starts from.
    pub default_preset: String,
    pub keymap: BTreeMap<String, String>,
    /// Whether Windows is currently in light mode; only used when `theme` is "system".
    #[serde(skip)]
    pub system_light: bool,
    /// Show the text as it will be exported (hidden markers, first-line indent) instead of raw.
    /// Not saved: every launch starts raw. The three fields after it are filled in from the export
    /// preset each time the editor is painted.
    #[serde(skip)]
    pub preview: bool,
    /// First-line indent in en spaces (half an em each).
    #[serde(skip)]
    pub preview_indent: u8,
    /// No indent on the first paragraph of a chapter or after a scene break.
    #[serde(skip)]
    pub preview_flush: bool,
    #[serde(skip)]
    pub preview_justify: bool,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            theme: "dark".into(),
            accent: "#d9aa72".into(),
            editor_font: crate::fonts::DEFAULT_EDITOR_FONT.into(),
            interface_font: crate::platform::default_sans().into(),
            show_markers: false,
            font_size: 18.,
            line_height: 1.6,
            paragraph_gap: 0.4,
            max_width: Some(720.),
            typewriter: false,
            autosave: true,
            repack_minutes: 3,
            note_marker: "$$".into(),
            note_color: String::new(),
            note_italic: true,
            hide_notes: false,
            preview_notes: false,
            spellcheck: true,
            default_preset: String::new(),
            keymap: BTreeMap::new(),
            system_light: false,
            justify: false,
            preview: false,
            preview_indent: 0,
            preview_flush: true,
            preview_justify: false,
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
        if !(1.0..=3.0).contains(&settings.line_height) || !settings.line_height.is_finite() {
            return Err("Line height must be between 1.0 and 3.0".into());
        }
        if !(1..=60).contains(&settings.repack_minutes) {
            return Err("The file is saved every 1 to 60 minutes".into());
        }
        super::notes::Marker::new(&settings.note_marker)?;
        if !settings.note_color.is_empty()
            && (settings.note_color.len() != 7
                || !settings.note_color.starts_with('#')
                || u32::from_str_radix(&settings.note_color[1..], 16).is_err())
        {
            return Err("Note colour must be a hex colour such as #8a8175, or empty".into());
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
    pub fn marker(&self) -> super::notes::Marker {
        super::notes::Marker::new(&self.note_marker).unwrap_or_default()
    }
    /// The note colour, or `None` for the theme's muted colour.
    pub fn note_colour(&self) -> Option<u32> {
        u32::from_str_radix(self.note_color.trim_start_matches('#'), 16).ok().filter(|_| self.note_color.len() == 7)
    }
    pub fn accent_color(&self) -> u32 {
        u32::from_str_radix(self.accent.trim_start_matches('#'), 16).unwrap_or(0xd9aa72)
    }
}

#[derive(Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Session {
    /// The `.jw` project that was open.
    pub project: Option<PathBuf>,
    pub split: Option<SessionSplit>,
    pub tree_width: f32,
    pub outline_width: f32,
    pub show_tree: bool,
    pub show_outline: bool,
    /// Recently opened projects, newest first.
    pub recent: Vec<PathBuf>,
    pub window: Option<WindowState>,
    /// `.jw` files have been associated with the program (Windows).
    pub file_type: bool,
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
    /// Path of the chapter or page file inside the project.
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
            project: Some(PathBuf::from("C:/notes/novel.jw")),
            split: Some(SessionSplit::Split {
                down: false,
                ratio: 0.4,
                first: Box::new(SessionSplit::Pane {
                    active: 0,
                    tabs: vec![SessionTab {
                        path: "chapters/note.txt".into(),
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
