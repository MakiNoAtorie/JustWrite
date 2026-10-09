use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    // Project
    NewProject,
    OpenProject,
    Recent,
    Import,
    Save,
    SaveAs,
    CloseProject,
    Quit,
    // Chapters and notes
    NewChapter,
    NewPage,
    Rename,
    Delete,
    MoveUp,
    MoveDown,
    // Tabs and panes
    CloseTab,
    Reopen,
    NextTab,
    PreviousTab,
    SplitRight,
    SplitDown,
    CloseSplit,
    // Editing
    Undo,
    Redo,
    SelectAll,
    Copy,
    Cut,
    Paste,
    Bold,
    Italic,
    ToggleNote,
    NoteParagraph,
    AddComment,
    NextComment,
    PreviousComment,
    ResolveComment,
    ReplyComment,
    EditComment,
    DeleteComment,
    AddWord,
    Find,
    Replace,
    ProjectSearch,
    // Navigation
    Palette,
    QuickOpen,
    NextChapter,
    PreviousChapter,
    JumpScene,
    GotoLine,
    WordCount,
    // Review and history
    ShowComments,
    ShowHistory,
    Snapshot,
    // Output
    Export,
    Preview,
    BookDetails,
    ProjectSettings,
    // View and settings
    ToggleTree,
    TogglePanel,
    ToggleMarkers,
    RawView,
    PreviewView,
    HideNotes,
    PreviewNotes,
    Focus,
    Settings,
    Keymap,
}
pub struct CommandInfo {
    pub id: Command,
    pub key: &'static str,
    pub name: &'static str,
    pub shortcut: &'static str,
}
macro_rules! commands {
    ($(($id:expr, $key:literal, $name:literal, $shortcut:literal)),* $(,)?) => {
        pub const COMMANDS: &[CommandInfo] = &[$(CommandInfo { id: $id, key: $key, name: $name, shortcut: $shortcut }),*];
    };
}
commands![
    (Command::NewProject, "new_project", "New project…", "Ctrl+Shift+N"),
    (Command::OpenProject, "open_project", "Open project…", "Ctrl+O"),
    (Command::Recent, "recent", "Open recent project…", ""),
    (Command::Import, "import", "Import .docx, .md or .txt…", ""),
    (Command::Save, "save", "Save", "Ctrl+S"),
    (Command::SaveAs, "save_as", "Save project as…", "Ctrl+Shift+S"),
    (Command::CloseProject, "close_project", "Close project", ""),
    (Command::Quit, "quit", "Exit", "Alt+F4"),
    (Command::NewChapter, "new_chapter", "New chapter", "Ctrl+N"),
    (Command::NewPage, "new_page", "New note", "Ctrl+Alt+N"),
    (Command::Rename, "rename", "Rename…", "F2"),
    (Command::Delete, "delete", "Delete…", ""),
    (Command::MoveUp, "move_up", "Move up", "Alt+ArrowUp"),
    (Command::MoveDown, "move_down", "Move down", "Alt+ArrowDown"),
    (Command::CloseTab, "close_tab", "Close tab", "Ctrl+W"),
    (Command::Reopen, "reopen", "Reopen closed tab", "Ctrl+Shift+T"),
    (Command::NextTab, "next_tab", "Next tab", "Ctrl+Tab"),
    (Command::PreviousTab, "previous_tab", "Previous tab", "Ctrl+Shift+Tab"),
    (Command::SplitRight, "split_right", "Split right", "Ctrl+\\"),
    (Command::SplitDown, "split_down", "Split down", "Ctrl+Shift+\\"),
    (Command::CloseSplit, "close_split", "Close split pane", ""),
    (Command::Undo, "undo", "Undo", "Ctrl+Z"),
    (Command::Redo, "redo", "Redo", "Ctrl+Y"),
    (Command::SelectAll, "select_all", "Select all", "Ctrl+A"),
    (Command::Copy, "copy", "Copy", "Ctrl+C"),
    (Command::Cut, "cut", "Cut", "Ctrl+X"),
    (Command::Paste, "paste", "Paste", "Ctrl+V"),
    (Command::Bold, "bold", "Bold", "Ctrl+B"),
    (Command::Italic, "italic", "Italic", "Ctrl+I"),
    (Command::ToggleNote, "toggle_note", "Turn line or selection into a note, and back", "Ctrl+/"),
    (Command::NoteParagraph, "note_paragraph", "Turn paragraph into a note, and back", "Ctrl+Alt+/"),
    (Command::AddComment, "add_comment", "Add review comment…", "Ctrl+Alt+M"),
    (Command::NextComment, "next_comment", "Go to next review comment", "F8"),
    (Command::PreviousComment, "previous_comment", "Go to previous review comment", "Shift+F8"),
    (Command::ResolveComment, "resolve_comment", "Resolve or reopen the comment at the caret", ""),
    (Command::ReplyComment, "reply_comment", "Reply to the comment at the caret…", ""),
    (Command::EditComment, "edit_comment", "Edit the comment at the caret…", ""),
    (Command::DeleteComment, "delete_comment", "Delete the comment at the caret…", ""),
    (Command::AddWord, "add_word", "Add word to the dictionary", ""),
    (Command::Find, "find", "Find…", "Ctrl+F"),
    (Command::Replace, "replace", "Replace…", "Ctrl+H"),
    (Command::ProjectSearch, "project_search", "Search the project…", "Ctrl+Shift+F"),
    (Command::Palette, "palette", "Command palette", "Ctrl+Shift+P"),
    (Command::QuickOpen, "quick_open", "Go to chapter or note…", "Ctrl+P"),
    (Command::NextChapter, "next_chapter", "Next chapter or note", "Ctrl+PageDown"),
    (Command::PreviousChapter, "previous_chapter", "Previous chapter or note", "Ctrl+PageUp"),
    (Command::JumpScene, "jump_scene", "Jump to scene", "Ctrl+J"),
    (Command::GotoLine, "goto_line", "Go to line…", "Ctrl+G"),
    (Command::WordCount, "word_count", "Word count", ""),
    (Command::ShowComments, "show_comments", "Show comments panel", "Ctrl+Shift+M"),
    (Command::ShowHistory, "show_history", "Snapshots and history…", "Ctrl+Shift+H"),
    (Command::Snapshot, "snapshot", "Take snapshot…", "Ctrl+Alt+S"),
    (Command::Export, "export", "Export…", "Ctrl+E"),
    (Command::Preview, "preview", "Paged preview…", "Ctrl+Alt+P"),
    (Command::BookDetails, "book_details", "Book details…", ""),
    (Command::ProjectSettings, "project_settings", "Project settings…", ""),
    (Command::ToggleTree, "toggle_tree", "Toggle project tree", "Ctrl+Shift+E"),
    (Command::TogglePanel, "toggle_panel", "Toggle side panel", "Ctrl+Shift+O"),
    (Command::ToggleMarkers, "toggle_markers", "Show formatting and note markers", "Ctrl+M"),
    (Command::RawView, "raw_view", "Raw view: text as typed", ""),
    (Command::PreviewView, "preview_view", "Preview view: text as exported", ""),
    (Command::HideNotes, "hide_notes", "Hide notes", "Ctrl+Shift+Y"),
    (Command::PreviewNotes, "preview_notes", "Show notes in the preview", ""),
    (Command::Focus, "focus", "Focus mode", "F11"),
    (Command::Settings, "settings", "Settings…", "Ctrl+,"),
    (Command::Keymap, "keymap", "Keyboard shortcuts…", ""),
];

#[derive(Clone, PartialEq, Eq)]
pub struct Shortcut {
    pub primary: bool,
    pub shift: bool,
    pub alt: bool,
    pub key: String,
}
pub fn parse_shortcut(value: &str) -> Result<Option<Shortcut>, String> {
    if value.trim().is_empty() {
        return Ok(None);
    }
    let mut shortcut = Shortcut {
        primary: false,
        shift: false,
        alt: false,
        key: String::new(),
    };
    for part in value.split('+') {
        match part.trim().to_lowercase().as_str() {
            "ctrl" | "cmd" => shortcut.primary = true,
            "shift" => shortcut.shift = true,
            "alt" | "option" => shortcut.alt = true,
            "" => return Err(format!("Invalid shortcut: {value}")),
            key if shortcut.key.is_empty() => shortcut.key = key.into(),
            _ => return Err(format!("Invalid shortcut: {value}")),
        }
    }
    if shortcut.key.is_empty() {
        return Err(format!("A shortcut needs a key: {value}"));
    }
    Ok(Some(shortcut))
}
/// Builds a shortcut string such as `Ctrl+Shift+P` from a key press; it round-trips through `parse_shortcut`.
pub fn format_shortcut(primary: bool, shift: bool, alt: bool, key: &str) -> String {
    let mut out = String::new();
    for (on, name) in [(primary, "Ctrl+"), (shift, "Shift+"), (alt, "Alt+")] {
        if on {
            out.push_str(name);
        }
    }
    let mut chars = key.chars();
    out.extend(chars.next().map(|c| c.to_uppercase().to_string()));
    out.push_str(chars.as_str());
    out
}
pub fn resolve(
    key: &str,
    primary: bool,
    shift: bool,
    alt: bool,
    keymap: &BTreeMap<String, String>,
) -> Option<Command> {
    let event = Shortcut {
        key: key.to_lowercase(),
        primary,
        shift,
        alt,
    };
    COMMANDS.iter().find_map(|command| {
        let value = keymap
            .get(command.key)
            .map(String::as_str)
            .unwrap_or(command.shortcut);
        if parse_shortcut(value).ok().flatten().as_ref() == Some(&event) {
            Some(command.id)
        } else {
            None
        }
    })
}

/// A stored shortcut as shown to people: Cmd for Ctrl on a Mac, arrows as arrows.
pub fn display_shortcut(raw: &str) -> String {
    raw.replace("Ctrl", if cfg!(target_os = "macos") { "Cmd" } else { "Ctrl" })
        .replace("ArrowUp", "↑")
        .replace("ArrowDown", "↓")
        .replace("ArrowLeft", "←")
        .replace("ArrowRight", "→")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shortcuts_are_shown_with_arrows() {
        assert_eq!(display_shortcut("Alt+ArrowUp"), "Alt+↑");
        assert_eq!(display_shortcut("Ctrl+Shift+ArrowRight"), format!("{}+Shift+→", if cfg!(target_os = "macos") { "Cmd" } else { "Ctrl" }));
        assert_eq!(display_shortcut("F11"), "F11");
    }
    #[test]
    fn every_default_shortcut_round_trips_and_can_be_disabled() {
        for command in COMMANDS {
            if let Some(key) = parse_shortcut(command.shortcut).unwrap() {
                assert_eq!(
                    resolve(&key.key, key.primary, key.shift, key.alt, &BTreeMap::new()),
                    Some(command.id),
                    "{}",
                    command.name
                );
            }
        }
        let mut keys = BTreeMap::new();
        keys.insert("bold".into(), "Cmd+Alt+B".into());
        assert_eq!(resolve("b", true, false, true, &keys), Some(Command::Bold));
        let captured = parse_shortcut(&format_shortcut(true, true, false, "f2")).unwrap();
        assert_eq!(captured.unwrap().key, "f2");
        keys.insert("bold".into(), "".into());
        assert_eq!(resolve("b", true, false, false, &keys), None);
    }
}
