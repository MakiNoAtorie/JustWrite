use crate::core::{commands::Command, workspace::Match};
use std::path::PathBuf;

#[derive(Clone, PartialEq)]
pub enum Kind {
    Palette,
    QuickOpen,
    Scenes,
    GotoLine,
    Find,
    Replace,
    ProjectSearch,
    Recent,
    Keymap,
    Setting(String),
    Shortcut(String),
    /// Rename the chapter, part or note with this id.
    Rename(String),
    /// Delete the chapter, part or note with this id.
    Delete(String),
    Close(u64),
    Quit,
    ChangeWorkspace(PathBuf),
}
#[derive(Clone)]
pub enum Action {
    Command(Command),
    File(PathBuf, usize),
    Scene(usize),
    Match(Match),
    Project(PathBuf),
    Setting(String),
    Shortcut(String),
    None,
}
#[derive(Clone)]
pub struct Row {
    pub label: String,
    pub hint: String,
    pub action: Action,
}
pub struct Overlay {
    pub kind: Kind,
    pub query: String,
    pub replacement: String,
    pub secondary: bool,
    pub selected: usize,
    pub rows: Vec<Row>,
    pub version: u64,
    pub pending: bool,
    pub select_all: bool,
    /// Caret position in the active field, in chars; `None` means the end.
    pub caret: Option<usize>,
    pub case: bool,
    /// Validation message shown inside the dialog.
    pub note: String,
    pub scroll: usize,
    pub visible: usize,
    /// Whether keystrokes go to this overlay; a docked find bar loses focus when you click the text.
    pub focused: bool,
}
impl Overlay {
    pub fn new(kind: Kind) -> Self {
        Self {
            kind,
            query: String::new(),
            replacement: String::new(),
            secondary: false,
            selected: 0,
            rows: Vec::new(),
            version: 0,
            pending: false,
            select_all: false,
            caret: None,
            case: false,
            note: String::new(),
            scroll: 0,
            visible: 10,
            focused: true,
        }
    }
    /// Find and replace live in a bar above the text instead of a centred box.
    pub fn docked(&self) -> bool {
        matches!(self.kind, Kind::Find | Kind::Replace)
    }
    pub fn title(&self) -> &str {
        match &self.kind {
            Kind::Palette => "Command palette",
            Kind::QuickOpen => "Go to chapter or note",
            Kind::Scenes => "Jump to scene",
            Kind::Find => "Find in this chapter",
            Kind::Replace => "Find and replace",
            Kind::ProjectSearch => "Search the project",
            Kind::Recent => "Open recent project",
            Kind::Keymap => "Keyboard shortcuts",
            Kind::Setting(_) => "Choose a font",
            Kind::Shortcut(_) => "Change shortcut",
            Kind::GotoLine => "Go to line",
            Kind::Rename(_) => "Rename",
            Kind::Delete(_) => "Delete",
            Kind::Close(_) | Kind::Quit | Kind::ChangeWorkspace(_) => "Unsaved changes",
        }
    }
    pub fn placeholder(&self) -> &str {
        match &self.kind {
            Kind::Palette => "Type a command…",
            Kind::QuickOpen => "Type a chapter or note name…",
            Kind::Scenes => "Type words from a scene…",
            Kind::Find | Kind::Replace => "Find…",
            Kind::ProjectSearch => "Search all chapters and pages…",
            Kind::Setting(_) => "Type to filter fonts…",
            Kind::Keymap => "Filter commands…",
            Kind::Recent => "Type a project name…",
            Kind::GotoLine => "Line number",
            Kind::Rename(_) => "New title",
            _ => "",
        }
    }
    /// Small centred boxes with buttons, as opposed to searchable lists.
    pub fn is_dialog(&self) -> bool {
        matches!(
            self.kind,
            Kind::Shortcut(_)
                | Kind::GotoLine
                | Kind::Rename(_)
                | Kind::Delete(_)
                | Kind::Close(_)
                | Kind::Quit
                | Kind::ChangeWorkspace(_)
        )
    }
    pub fn has_input(&self) -> bool {
        !matches!(
            self.kind,
            Kind::Shortcut(_)
                | Kind::Delete(_)
                | Kind::Close(_)
                | Kind::Quit
                | Kind::ChangeWorkspace(_)
        )
    }
    pub fn input(&mut self) -> &mut String {
        if self.secondary {
            &mut self.replacement
        } else {
            &mut self.query
        }
    }
    pub fn type_text(&mut self, text: &str) {
        let text: String = text.chars().filter(|c| !c.is_control()).collect();
        self.note.clear();
        if self.select_all {
            self.input().clear();
            self.select_all = false;
            self.caret = None;
        }
        let at = self.caret();
        let field = self.input();
        let byte = field.char_indices().nth(at).map_or(field.len(), |(i, _)| i);
        field.insert_str(byte, &text);
        let end = field.chars().count();
        let moved = at + text.chars().count();
        self.caret = (moved < end).then_some(moved);
    }
    /// Scrolls the result list so the selected row is on screen.
    pub fn reveal(&mut self) {
        if self.selected < self.scroll {
            self.scroll = self.selected;
        } else if self.selected >= self.scroll + self.visible {
            self.scroll = self.selected + 1 - self.visible;
        }
    }
    pub fn caret(&mut self) -> usize {
        let len = self.input().chars().count();
        self.caret.unwrap_or(len).min(len)
    }
    /// Moves the caret one char, or with `delete` removes the char before (`back`) or after it.
    pub fn edit(&mut self, back: bool, delete: bool) {
        self.note.clear();
        if self.select_all {
            self.select_all = false;
            if delete {
                self.input().clear();
                self.caret = None;
                return;
            }
        }
        let at = self.caret();
        let field = self.input();
        let len = field.chars().count();
        let target = if back { at.saturating_sub(1) } else { (at + 1).min(len) };
        if delete {
            let (from, to) = (at.min(target), at.max(target));
            let bytes: Vec<_> = field.char_indices().map(|(i, _)| i).chain([field.len()]).collect();
            field.replace_range(bytes[from]..bytes[to], "");
            self.caret = (from < len - (to - from)).then_some(from);
        } else {
            self.caret = (target < len).then_some(target);
        }
    }
}
