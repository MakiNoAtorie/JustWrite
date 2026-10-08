use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    Open,
    OpenFile,
    Save,
    SaveAs,
    Reload,
    New,
    CloseTab,
    Reopen,
    NextTab,
    PreviousTab,
    Undo,
    Redo,
    SelectAll,
    Copy,
    Cut,
    Paste,
    Bold,
    Italic,
    Strike,
    Heading(u8),
    Bullet,
    Numbered,
    InlineCode,
    CodeBlock,
    Link,
    Indent,
    Outdent,
    Palette,
    QuickOpen,
    JumpHeading,
    GotoLine,
    WordCount,
    Find,
    Replace,
    FolderSearch,
    SplitRight,
    SplitDown,
    CloseSplit,
    Recent,
    ToggleTree,
    ToggleOutline,
    ToggleMarkers,
    Focus,
    Settings,
    Keymap,
    CreateFile,
    CreateFolder,
    RenameFile,
    DeleteFile,
    Quit,
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
    (
        Command::Open,
        "open_folder",
        "Open folder…",
        "Ctrl+O"
    ),
    (
        Command::OpenFile,
        "open_file",
        "Open file…",
        "Ctrl+Shift+L"
    ),
    (Command::Save, "save", "Save", "Ctrl+S"),
    (Command::SaveAs, "save_as", "Save as…", "Ctrl+Shift+S"),
    (
        Command::Reload,
        "reload",
        "Reload from disk (discard my edits)",
        "Ctrl+R"
    ),
    (Command::New, "new", "New file", "Ctrl+N"),
    (Command::CloseTab, "close_tab", "Close tab", "Ctrl+W"),
    (
        Command::Reopen,
        "reopen",
        "Reopen closed tab",
        "Ctrl+Shift+T"
    ),
    (Command::NextTab, "next_tab", "Next tab", "Ctrl+Tab"),
    (
        Command::PreviousTab,
        "previous_tab",
        "Previous tab",
        "Ctrl+Shift+Tab"
    ),
    (Command::Undo, "undo", "Undo", "Ctrl+Z"),
    (Command::Redo, "redo", "Redo", "Ctrl+Y"),
    (Command::SelectAll, "select_all", "Select all", "Ctrl+A"),
    (Command::Copy, "copy", "Copy", "Ctrl+C"),
    (Command::Cut, "cut", "Cut", "Ctrl+X"),
    (Command::Paste, "paste", "Paste", "Ctrl+V"),
    (Command::Bold, "bold", "Bold", "Ctrl+B"),
    (Command::Italic, "italic", "Italic", "Ctrl+I"),
    (Command::Strike, "strike", "Strikethrough", "Alt+Shift+5"),
    (Command::Heading(1), "heading_1", "Heading 1", "Ctrl+1"),
    (Command::Heading(2), "heading_2", "Heading 2", "Ctrl+2"),
    (Command::Heading(3), "heading_3", "Heading 3", "Ctrl+3"),
    (Command::Heading(4), "heading_4", "Heading 4", "Ctrl+4"),
    (Command::Heading(5), "heading_5", "Heading 5", "Ctrl+5"),
    (Command::Heading(6), "heading_6", "Heading 6", "Ctrl+6"),
    (Command::Bullet, "bullet", "Bullet list", "Ctrl+Shift+8"),
    (
        Command::Numbered,
        "numbered",
        "Numbered list",
        "Ctrl+Shift+7"
    ),
    (Command::InlineCode, "inline_code", "Inline code", "Ctrl+E"),
    (
        Command::CodeBlock,
        "code_block",
        "Code block",
        "Ctrl+Shift+K"
    ),
    (Command::Link, "link", "Link", "Ctrl+K"),
    (Command::Indent, "indent", "Indent", "Tab"),
    (Command::Outdent, "outdent", "Outdent", "Shift+Tab"),
    (
        Command::Palette,
        "palette",
        "Command palette",
        "Ctrl+Shift+P"
    ),
    (Command::QuickOpen, "quick_open", "Quick open", "Ctrl+P"),
    (
        Command::JumpHeading,
        "jump_heading",
        "Jump to heading",
        "Ctrl+J"
    ),
    (Command::GotoLine, "goto_line", "Go to line…", "Ctrl+G"),
    (Command::WordCount, "word_count", "Word count", ""),
    (Command::Find, "find", "Find…", "Ctrl+F"),
    (Command::Replace, "replace", "Replace…", "Ctrl+H"),
    (
        Command::FolderSearch,
        "folder_search",
        "Search workspace…",
        "Ctrl+Shift+F"
    ),
    (Command::SplitRight, "split_right", "Split right", "Ctrl+\\"),
    (
        Command::SplitDown,
        "split_down",
        "Split down",
        "Ctrl+Shift+\\"
    ),
    (
        Command::CloseSplit,
        "close_split",
        "Close split pane",
        ""
    ),
    (
        Command::Recent,
        "recent",
        "Open recent folder…",
        ""
    ),
    (
        Command::ToggleTree,
        "toggle_tree",
        "Toggle folder tree",
        "Ctrl+Shift+E"
    ),
    (
        Command::ToggleOutline,
        "toggle_outline",
        "Toggle outline",
        "Ctrl+Shift+O"
    ),
    (
        Command::ToggleMarkers,
        "toggle_markers",
        "Toggle syntax markers",
        "Ctrl+M"
    ),
    (Command::Focus, "focus", "Focus mode", "F11"),
    (Command::Settings, "settings", "Settings…", "Ctrl+,"),
    (Command::Keymap, "keymap", "Keyboard shortcuts…", ""),
    (
        Command::CreateFile,
        "create_file",
        "New note in workspace…",
        ""
    ),
    (
        Command::CreateFolder,
        "create_folder",
        "New folder in workspace…",
        ""
    ),
    (
        Command::RenameFile,
        "rename_file",
        "Rename…",
        "F2"
    ),
    (
        Command::DeleteFile,
        "delete_file",
        "Delete (Recycle Bin)…",
        ""
    ),
    (Command::Quit, "quit", "Exit", "Alt+F4"),
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

#[cfg(test)]
mod tests {
    use super::*;
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
