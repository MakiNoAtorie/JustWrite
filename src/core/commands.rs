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
    CloudConnect,
    CloudDisconnect,
    CloudSave,
    CloudOpen,
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
    AlignLeft,
    AlignJustify,
    FontSmaller,
    FontLarger,
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
    NewScene,
    Epigraph,
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
    PreviewChapter,
    PreviewView,
    HideNotes,
    PreviewNotes,
    Focus,
    HideInterface,
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
    ($(($id:expr, $key:literal, $name:literal, $shortcut:expr)),* $(,)?) => {
        pub const COMMANDS: &[CommandInfo] = &[$(CommandInfo { id: $id, key: $key, name: $name, shortcut: $shortcut }),*];
    };
}
/// A default that differs on a Mac, where the usual key belongs to the system or to habit.
const fn mac(mac: &'static str, other: &'static str) -> &'static str {
    if cfg!(target_os = "macos") { mac } else { other }
}
commands![
    (Command::NewProject, "new_project", "New project…", "Ctrl+Shift+N"),
    (Command::OpenProject, "open_project", "Open project…", "Ctrl+O"),
    (Command::Recent, "recent", "Open recent project…", ""),
    (Command::Import, "import", "Import .docx, .md or .txt…", ""),
    (Command::Save, "save", "Save", "Ctrl+S"),
    (Command::SaveAs, "save_as", "Save project as…", "Ctrl+Shift+S"),
    (Command::CloseProject, "close_project", "Close project", ""),
    (Command::CloudConnect, "cloud_connect", "Connect to cloud…", ""),
    (Command::CloudDisconnect, "cloud_disconnect", "Disconnect from cloud", ""),
    (Command::CloudSave, "cloud_save", "Save project to cloud", ""),
    (Command::CloudOpen, "cloud_open", "Open from cloud…", ""),
    (Command::Quit, "quit", "Exit", mac("Ctrl+Q", "Alt+F4")),
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
    (Command::Redo, "redo", "Redo", mac("Ctrl+Shift+Z", "Ctrl+Y")),
    (Command::SelectAll, "select_all", "Select all", "Ctrl+A"),
    (Command::Copy, "copy", "Copy", "Ctrl+C"),
    (Command::Cut, "cut", "Cut", "Ctrl+X"),
    (Command::Paste, "paste", "Paste", "Ctrl+V"),
    (Command::Bold, "bold", "Bold", "Ctrl+B"),
    (Command::Italic, "italic", "Italic", "Ctrl+I"),
    (Command::AlignLeft, "align_left", "Align text left", "Ctrl+Shift+L"),
    (Command::AlignJustify, "align_justify", "Justify text", "Ctrl+Shift+J"),
    (Command::FontSmaller, "font_smaller", "Smaller text", "Ctrl+-"),
    (Command::FontLarger, "font_larger", "Larger text", "Ctrl+="),
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
    (Command::NewScene, "new_scene", "New scene", "Ctrl+Alt+Enter"),
    (Command::Epigraph, "epigraph", "Epigraph", ""),
    (Command::JumpScene, "jump_scene", "Jump to scene", "Ctrl+J"),
    (Command::GotoLine, "goto_line", "Go to line…", "Ctrl+G"),
    (Command::WordCount, "word_count", "Word count", ""),
    (Command::ShowComments, "show_comments", "Show comments panel", "Ctrl+Shift+M"),
    (Command::ShowHistory, "show_history", "Snapshots and history", "Ctrl+Shift+H"),
    (Command::Snapshot, "snapshot", "Take snapshot…", "Ctrl+Alt+S"),
    (Command::Export, "export", "Export…", "Ctrl+E"),
    (Command::Preview, "preview", "Paged preview…", "Ctrl+Alt+P"),
    (Command::BookDetails, "book_details", "Book details…", ""),
    (Command::ProjectSettings, "project_settings", "Project settings…", ""),
    (Command::ToggleTree, "toggle_tree", "Toggle project tree", "Ctrl+Shift+E"),
    (Command::TogglePanel, "toggle_panel", "Toggle side panel", "Ctrl+Shift+O"),
    (Command::ToggleMarkers, "toggle_markers", "Show formatting and note markers", "Ctrl+M"),
    (Command::RawView, "raw_view", "Raw view: text as typed", ""),
    (Command::PreviewChapter, "preview_chapter", "Preview this chapter", ""),
    (Command::PreviewView, "preview_view", "Soft preview (editable)", ""),
    (Command::HideNotes, "hide_notes", "Hide notes", "Ctrl+Shift+Y"),
    (Command::PreviewNotes, "preview_notes", "Show notes in the preview", ""),
    (Command::Focus, "focus", "Full screen", mac("Ctrl+Alt+F", "F11")),
    (Command::HideInterface, "hide_interface", "Hide or show all panels", "Tab"),
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
    // `=` and `+` are one key with or without Shift, depending on the keyboard layout.
    let plus = matches!(key, "=" | "+");
    let event = Shortcut {
        key: if plus { "=".into() } else { key.to_lowercase() },
        primary,
        shift: shift && !plus,
        alt,
    };
    COMMANDS.iter().find_map(|command| {
        let value = keymap
            .get(command.key)
            .map(String::as_str)
            .unwrap_or(command.shortcut);
        let wanted = parse_shortcut(value).ok().flatten().map(|mut s| {
            s.shift &= s.key != "=";
            s
        });
        if wanted.as_ref() == Some(&event) {
            Some(command.id)
        } else {
            None
        }
    })
}

/// The defaults of every platform before a Mac got its own.
const BEFORE_MAC: &[(&str, &str)] = &[("quit", "Alt+F4"), ("redo", "Ctrl+Y"), ("focus", "F11")];
/// Leaves only the shortcuts a person changed. A settings file once listed every default, and so
/// kept the old one when a default changed.
pub fn prune_defaults(keymap: &mut BTreeMap<String, String>) {
    let listed = keymap.len();
    keymap.retain(|key, value| COMMANDS.iter().all(|c| c.key != key || c.shortcut != value));
    // Only a file that listed defaults has the old ones; in any other file they are a choice.
    if cfg!(target_os = "macos") && keymap.len() < listed {
        keymap.retain(|key, value| !BEFORE_MAC.contains(&(key.as_str(), value.as_str())));
    }
}

/// A stored shortcut as shown to people: Cmd and Option on a Mac, arrows as arrows.
pub fn display_shortcut(raw: &str) -> String {
    let mac = cfg!(target_os = "macos");
    // Cmd+Tab belongs to macOS, so the tab shortcuts keep Ctrl there.
    raw.replace("Ctrl", if mac && !raw.ends_with("Tab") { "Cmd" } else { "Ctrl" })
        .replace("Alt", if mac { "Option" } else { "Alt" })
        .replace("ArrowUp", "↑")
        .replace("ArrowDown", "↓")
        .replace("ArrowLeft", "←")
        .replace("ArrowRight", "→")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn a_listed_default_does_not_hide_a_new_default() {
        let mut keymap: BTreeMap<String, String> =
            [("bold", "Ctrl+B"), ("italic", "Ctrl+Alt+I"), ("quit", "Alt+F4"), ("undo", "")].map(|(k, v)| (k.into(), v.into())).into();
        prune_defaults(&mut keymap);
        assert_eq!(keymap.get("italic").map(String::as_str), Some("Ctrl+Alt+I"));
        assert_eq!(keymap.get("undo").map(String::as_str), Some(""));
        assert!(!keymap.contains_key("bold"));
        assert!(!keymap.contains_key("quit"));
        // With no default listed, the same shortcut is a choice on a Mac and stays.
        let mut chosen: BTreeMap<String, String> = [("quit".to_owned(), "Alt+F4".to_owned())].into();
        prune_defaults(&mut chosen);
        assert_eq!(chosen.len(), if cfg!(target_os = "macos") { 1 } else { 0 });
    }
    #[test]
    fn shortcuts_are_shown_with_arrows() {
        assert_eq!(display_shortcut("Alt+ArrowUp"), if cfg!(target_os = "macos") { "Option+↑" } else { "Alt+↑" });
        assert_eq!(display_shortcut("Ctrl+Shift+Tab"), "Ctrl+Shift+Tab");
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
        // Larger text: `=` alone (US), Shift+`=` (Spanish), `+` with or without Shift.
        for (key, shift) in [("=", false), ("=", true), ("+", true), ("+", false)] {
            assert_eq!(resolve(key, true, shift, false, &BTreeMap::new()), Some(Command::FontLarger), "{key} {shift}");
        }
        let captured = parse_shortcut(&format_shortcut(true, true, false, "f2")).unwrap();
        assert_eq!(captured.unwrap().key, "f2");
        keys.insert("bold".into(), "".into());
        assert_eq!(resolve("b", true, false, false, &keys), None);
    }
}
