//! Window, font, dialog and clipboard code. Not compiled for iOS.
use super::{default_mono, default_sans};
use cosmic_text::{FontSystem, fontdb};
use std::path::PathBuf;
use std::sync::Arc;
use winit::keyboard::ModifiersState;
use winit::window::Window;

#[cfg(all(test, windows))]
pub fn test_event_loop<T: 'static>() -> winit::event_loop::EventLoop<T> {
    use winit::platform::windows::EventLoopBuilderExtWindows;
    winit::event_loop::EventLoop::<T>::with_user_event()
        .with_any_thread(true)
        .build()
        .unwrap()
}

/// Ctrl on Windows and Linux, Cmd on macOS.
pub fn primary(modifiers: ModifiersState) -> bool {
    if cfg!(target_os = "macos") {
        modifiers.super_key()
    } else {
        modifiers.control_key()
    }
}
/// Small font database for the first frame; system fonts are enumerated later.
pub fn font_system() -> FontSystem {
    let mut db = fontdb::Database::new();
    #[cfg(windows)]
    for name in ["segoeui.ttf", "segoeuib.ttf", "segoeuii.ttf", "seguisym.ttf", "consola.ttf"] {
        let folder = std::env::var_os("WINDIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("C:\\Windows"));
        let _ = db.load_font_file(folder.join("Fonts").join(name));
    }
    #[cfg(target_os = "macos")]
    for path in ["/System/Library/Fonts/SFNS.ttf", "/System/Library/Fonts/Menlo.ttc"] {
        let _ = db.load_font_file(path);
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    db.load_system_fonts();
    crate::fonts::load_bundled_interface(&mut db);
    db.set_sans_serif_family(default_sans());
    db.set_monospace_family(default_mono());
    FontSystem::new_with_locale_and_db("en-US".into(), db)
}
/// Every installed font plus the bundled book fonts.
pub fn fallback_fonts() -> fontdb::Database {
    let mut db = fontdb::Database::new();
    db.load_system_fonts();
    crate::fonts::load_bundled(&mut db);
    db.set_sans_serif_family(default_sans());
    db.set_monospace_family(default_mono());
    db
}

/// A file filter: display name and extensions without dots.
pub type Filter<'a> = (&'a str, &'a [&'a str]);

fn dialog(parent: &Option<Arc<Window>>, title: &str, filters: &[Filter]) -> rfd::FileDialog {
    let mut dialog = rfd::FileDialog::new().set_title(title);
    if let Some(window) = parent {
        dialog = dialog.set_parent(&**window);
    }
    for (name, extensions) in filters {
        dialog = dialog.add_filter(*name, extensions);
    }
    dialog
}
pub fn choose_folder(parent: Option<Arc<Window>>, title: &str) -> Option<PathBuf> {
    dialog(&parent, title, &[]).pick_folder()
}
/// `JUSTWRITE_DIALOG_FILE` and `JUSTWRITE_DIALOG_DIR` answer the file dialogs without showing
/// them, for automated runs.
pub fn choose_file(parent: Option<Arc<Window>>, title: &str, filters: &[Filter]) -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("JUSTWRITE_DIALOG_FILE") {
        return Some(PathBuf::from(path));
    }
    dialog(&parent, title, filters).pick_file()
}
pub fn choose_files(parent: Option<Arc<Window>>, title: &str, filters: &[Filter]) -> Vec<PathBuf> {
    dialog(&parent, title, filters).pick_files().unwrap_or_default()
}
pub fn choose_save(parent: Option<Arc<Window>>, title: &str, name: &str, filters: &[Filter]) -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("JUSTWRITE_DIALOG_DIR") {
        return Some(PathBuf::from(dir).join(name));
    }
    dialog(&parent, title, filters).set_file_name(name).save_file()
}

pub struct Clipboard(Option<arboard::Clipboard>);
impl Default for Clipboard {
    fn default() -> Self {
        Self(None)
    }
}
impl Clipboard {
    fn get(&mut self) -> Result<&mut arboard::Clipboard, String> {
        if self.0.is_none() {
            self.0 = Some(arboard::Clipboard::new().map_err(|e| e.to_string())?);
        }
        Ok(self.0.as_mut().unwrap())
    }
    pub fn read(&mut self) -> Result<String, String> {
        self.get()?.get_text().map_err(|e| e.to_string())
    }
    pub fn write(&mut self, text: String) -> Result<(), String> {
        self.get()?.set_text(text).map_err(|e| e.to_string())
    }
}
