use crate::core::commands::{COMMANDS, Command, CommandInfo};

/// Menu-bar menus. `None` is a separator. Every entry dispatches through the command registry.
pub type Items = &'static [Option<Command>];
/// (title, Alt+mnemonic, items)
pub const MENUS: &[(&str, char, Items)] = &[
    (
        "File",
        'f',
        &[
            Some(Command::NewProject),
            Some(Command::OpenProject),
            Some(Command::Recent),
            Some(Command::Import),
            None,
            Some(Command::Save),
            Some(Command::SaveAs),
            Some(Command::Snapshot),
            None,
            Some(Command::Export),
            Some(Command::Preview),
            None,
            Some(Command::CloseTab),
            Some(Command::Reopen),
            Some(Command::CloseProject),
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
            Some(Command::ProjectSearch),
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
            None,
            Some(Command::AlignLeft),
            Some(Command::AlignJustify),
            None,
            Some(Command::ToggleNote),
            Some(Command::NoteParagraph),
            Some(Command::AddComment),
            Some(Command::AddWord),
        ],
    ),
    (
        "Project",
        'p',
        &[
            Some(Command::NewChapter),
            Some(Command::NewPage),
            None,
            Some(Command::Rename),
            Some(Command::MoveUp),
            Some(Command::MoveDown),
            Some(Command::Delete),
            None,
            Some(Command::BookDetails),
            Some(Command::ProjectSettings),
            Some(Command::ShowHistory),
        ],
    ),
    (
        "View",
        'v',
        &[
            Some(Command::ToggleTree),
            Some(Command::TogglePanel),
            Some(Command::ShowComments),
            Some(Command::Focus),
            None,
            Some(Command::RawView),
            Some(Command::PreviewView),
            Some(Command::PreviewNotes),
            Some(Command::HideNotes),
            Some(Command::ToggleMarkers),
            None,
            Some(Command::SplitRight),
            Some(Command::SplitDown),
            Some(Command::CloseSplit),
            None,
            Some(Command::WordCount),
        ],
    ),
    (
        "Go",
        'g',
        &[
            Some(Command::QuickOpen),
            Some(Command::NextChapter),
            Some(Command::PreviousChapter),
            None,
            Some(Command::JumpScene),
            Some(Command::GotoLine),
            Some(Command::NextComment),
            Some(Command::PreviousComment),
            None,
            Some(Command::NextTab),
            Some(Command::PreviousTab),
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
    Some(Command::ToggleNote),
    Some(Command::AddComment),
    Some(Command::AddWord),
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
    Some(Command::NewChapter),
    Some(Command::NewPage),
    None,
    Some(Command::Rename),
    Some(Command::MoveUp),
    Some(Command::MoveDown),
    Some(Command::Delete),
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
            .chain([EDITOR_MENU, TAB_MENU, TREE_MENU]);
        for items in all {
            for command in items.iter().flatten() {
                assert!(info(*command).is_some(), "{command:?} missing from COMMANDS");
            }
        }
    }
}
