use crate::core::commands::{COMMANDS, Command, CommandInfo};

/// Menu-bar menus. `None` is a separator. Every entry dispatches through the command registry.
pub type Items = &'static [Option<Command>];
/// (title, Alt+mnemonic, items)
pub const MENUS: &[(&str, char, Items)] = &[
    (
        "File",
        'f',
        &[
            Some(Command::New),
            Some(Command::OpenFile),
            Some(Command::Open),
            Some(Command::Recent),
            Some(Command::QuickOpen),
            None,
            Some(Command::Save),
            Some(Command::SaveAs),
            Some(Command::Reload),
            None,
            Some(Command::CreateFile),
            Some(Command::CreateFolder),
            None,
            Some(Command::CloseTab),
            Some(Command::Reopen),
            None,
            Some(Command::Quit),
        ],
    ),
    (
        "Edit",
        'e',
        &[
            Some(Command::Undo),
            Some(Command::Redo),
            None,
            Some(Command::Cut),
            Some(Command::Copy),
            Some(Command::Paste),
            Some(Command::SelectAll),
            None,
            Some(Command::Find),
            Some(Command::Replace),
            Some(Command::FolderSearch),
            None,
            Some(Command::Settings),
            Some(Command::Keymap),
        ],
    ),
    (
        "Format",
        'o',
        &[
            Some(Command::Bold),
            Some(Command::Italic),
            Some(Command::Strike),
            Some(Command::InlineCode),
            Some(Command::CodeBlock),
            Some(Command::Link),
            None,
            Some(Command::Bullet),
            Some(Command::Numbered),
            None,
            Some(Command::Heading(1)),
            Some(Command::Heading(2)),
            Some(Command::Heading(3)),
            Some(Command::Heading(4)),
            Some(Command::Heading(5)),
            Some(Command::Heading(6)),
            None,
            Some(Command::Indent),
            Some(Command::Outdent),
        ],
    ),
    (
        "View",
        'v',
        &[
            Some(Command::ToggleTree),
            Some(Command::ToggleOutline),
            Some(Command::ToggleMarkers),
            Some(Command::Focus),
            None,
            Some(Command::SplitRight),
            Some(Command::SplitDown),
            Some(Command::CloseSplit),
            None,
            Some(Command::NextTab),
            Some(Command::PreviousTab),
            Some(Command::JumpHeading),
            Some(Command::GotoLine),
            Some(Command::WordCount),
        ],
    ),
    (
        "Help",
        'h',
        &[
            Some(Command::Palette),
            Some(Command::Keymap),
        ],
    ),
];
pub const EDITOR_MENU: Items = &[
    Some(Command::Cut),
    Some(Command::Copy),
    Some(Command::Paste),
    Some(Command::SelectAll),
    None,
    Some(Command::Bold),
    Some(Command::Italic),
    Some(Command::Link),
    Some(Command::InlineCode),
    None,
    Some(Command::Find),
];
pub const TAB_MENU: Items = &[
    Some(Command::Save),
    Some(Command::CloseTab),
    None,
    Some(Command::SplitRight),
    Some(Command::SplitDown),
    Some(Command::CloseSplit),
];
pub const TREE_MENU: Items = &[
    Some(Command::CreateFile),
    Some(Command::CreateFolder),
    None,
    Some(Command::RenameFile),
    Some(Command::DeleteFile),
];
pub const HEADING_MENU: Items = &[
    Some(Command::Heading(1)),
    Some(Command::Heading(2)),
    Some(Command::Heading(3)),
    Some(Command::Heading(4)),
    Some(Command::Heading(5)),
    Some(Command::Heading(6)),
];

pub struct Popup {
    pub x: i32,
    pub y: i32,
    pub items: Items,
    /// Index into `MENUS` when this is a menu-bar drop-down.
    pub menu: Option<usize>,
    /// Item highlighted from the keyboard.
    pub selected: Option<usize>,
}

pub fn info(command: Command) -> Option<&'static CommandInfo> {
    COMMANDS.iter().find(|c| c.id == command)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn every_menu_entry_is_a_registered_command() {
        let all = MENUS
            .iter()
            .map(|(_, _, items)| *items)
            .chain([EDITOR_MENU, TAB_MENU, TREE_MENU, HEADING_MENU]);
        for items in all {
            for command in items.iter().flatten() {
                assert!(info(*command).is_some(), "{command:?} missing from COMMANDS");
            }
        }
    }
}
