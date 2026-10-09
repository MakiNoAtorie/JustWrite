//! Mouse-first chrome: menu bar, popups, tabs, toolbar, panels, dialogs and the settings panel.
//! Everything here only paints and registers hit rectangles; clicks are handled in `app.rs`.
use super::*;
use crate::ui::{
    layout::Divider,
    menu::{self, EDITOR_MENU, MENUS, Popup, TAB_MENU, TREE_MENU},
};
use cosmic_text::{Style as FontStyle, Weight};

#[derive(Clone, Copy, PartialEq)]
pub(super) enum Look {
    Plain,
    Outline,
    Primary,
    Danger,
    Active,
}
#[derive(Clone, Copy, PartialEq)]
pub(super) enum Face {
    Normal,
    Bold,
    Italic,
}
#[derive(Clone, Copy, PartialEq)]
pub(super) enum Align {
    Left,
    Center,
    Right,
}
pub(super) const ACCENTS: [u32; 8] = [
    0xd9aa72, 0xe8864a, 0xd9534f, 0xd96aa0, 0x9b7ed8, 0x5b9bd5, 0x4fb3a5, 0x7fb069,
];

pub(super) fn mix(a: u32, b: u32, t: f32) -> u32 {
    let channel = |shift: u32| {
        let (a, b) = (((a >> shift) & 255) as f32, ((b >> shift) & 255) as f32);
        (a * (1. - t) + b * t).round() as u32
    };
    channel(16) << 16 | channel(8) << 8 | channel(0)
}
/// Readable text colour on top of a solid fill.
pub(super) fn on_color(fill: u32) -> u32 {
    let luma = 0.299 * ((fill >> 16) & 255) as f32
        + 0.587 * ((fill >> 8) & 255) as f32
        + 0.114 * (fill & 255) as f32;
    if luma > 150. { 0x1d1a17 } else { 0xffffff }
}

/// Selected-row fill: the raised surface washed with the accent.
pub(super) fn selected_fill(base: u32, p: Palette) -> u32 {
    mix(base, p.accent, if p.light { 0.3 } else { 0.24 })
}
pub(super) fn hovered(base: u32, p: Palette) -> u32 {
    mix(base, p.text, if p.light { 0.07 } else { 0.09 })
}

struct TreeRow {
    id: String,
    title: String,
    depth: usize,
    part: bool,
    words: u32,
    open: usize,
    heading: Option<(&'static str, manifest::Section)>,
    collapsed: bool,
}

impl App {
    /// The project tree as one flat list: section headings, parts and their chapters, then pages.
    fn tree_rows(&self) -> Vec<TreeRow> {
        let mut rows: Vec<TreeRow> = Vec::new();
        let Some(state) = &self.project else { return rows };
        fn walk(nodes: &[manifest::Node], depth: usize, state: &ProjectState, out: &mut Vec<TreeRow>) {
            for node in nodes {
                let collapsed = state.collapsed.contains(node.id());
                out.push(TreeRow {
                    id: node.id().to_owned(),
                    title: node.title().to_owned(),
                    depth,
                    part: node.is_part(),
                    words: node.words(),
                    open: state.comments.count_open(node.id()),
                    heading: None,
                    collapsed,
                });
                if node.is_part() && !collapsed {
                    walk(node.children(), depth + 1, state, out);
                }
            }
        }
        for (section, label) in [(manifest::Section::Manuscript, "MANUSCRIPT"), (manifest::Section::Notes, "NOTES & RESEARCH")] {
            rows.push(TreeRow { id: String::new(), title: String::new(), depth: 0, part: false, words: 0, open: 0, heading: Some((label, section)), collapsed: false });
            walk(state.core.manifest.tree.section(section), 0, state, &mut rows);
        }
        rows
    }
    /// Soft drop shadow built from stacked translucent layers.
    pub(super) fn soft_shadow(&mut self, rect: Rect) {
        let spread = self.s(1.5).max(1);
        for layer in (1..=9).rev() {
            let grow = layer * spread;
            self.renderer.shade(
                Rect {
                    x: rect.x - grow,
                    y: rect.y - grow + self.s(5.),
                    width: rect.width + 2 * grow as u32,
                    height: rect.height + 2 * grow as u32,
                },
                0,
                3,
            );
        }
    }
    /// A thin scrollbar thumb down the right edge of `track`, for a list showing `visible` of its
    /// `total` rows from row `first`. Nothing is drawn when everything fits.
    pub(super) fn scroll_thumb(&mut self, track: Rect, first: usize, visible: usize, total: usize, p: Palette) {
        if total <= visible || track.height == 0 {
            return;
        }
        let thumb = ((track.height as usize * visible.max(1) / total) as u32).max(self.su(20.)).min(track.height);
        let offset = (track.height - thumb) as usize * first.min(total - visible) / (total - visible);
        let bar = Rect { x: track.x + track.width as i32 - self.s(5.), y: track.y + offset as i32, width: self.su(3.), height: thumb };
        self.renderer.fill_round(bar, self.su(2.), mix(p.border, p.muted, 0.35));
    }
    pub(super) fn s(&self, value: f32) -> i32 {
        (value * self.scale).round() as i32
    }
    pub(super) fn su(&self, value: f32) -> u32 {
        self.s(value).max(0) as u32
    }
    /// Hover test that ignores widgets covered by a menu or dialog.
    pub(super) fn hot(&self, rect: Rect) -> bool {
        contains(rect, self.mouse.x, self.mouse.y) && (self.in_modal || !self.modal())
    }
    pub(super) fn label(&mut self, text: &str, rect: Rect, size: f32, tint: u32) {
        self.label_face(text, rect, size, tint, Face::Normal);
    }
    pub(super) fn label_face(&mut self, text: &str, rect: Rect, size: f32, tint: u32, face: Face) {
        let font = self.text_font();
        let key = format!("{text}\0{}\0{size}\0{tint}\0{font}\0{}", rect.width, face as u8);
        let buffer = self.labels.entry(key).or_insert_with(|| {
            let mut buffer = Buffer::new_empty(Metrics::new(size, size * 1.5));
            buffer.set_size(Some(rect.width as f32), Some(rect.height as f32));
            buffer.set_text(text, &face_attrs(&font, tint, face), Shaping::Advanced, None);
            buffer
        });
        self.renderer
            .text(&mut self.fonts, buffer, rect.x, rect.y, rect);
    }
    /// A shaped copy of `text` for an input box, wrapped at `width` if there is one.
    pub(super) fn area_buffer(&mut self, text: &str, width: Option<f32>, size: f32, tint: u32) -> Buffer {
        let font = self.text_font();
        let mut buffer = Buffer::new_empty(Metrics::new(size, size * 1.5));
        buffer.set_size(width, None);
        buffer.set_text(text, &face_attrs(&font, tint, Face::Normal), Shaping::Advanced, None);
        buffer.shape_until_scroll(&mut self.fonts, false);
        buffer
    }
    /// The interface font, or the font being previewed in the font picker.
    fn text_font(&self) -> String {
        self.preview_font.clone().unwrap_or_else(|| self.settings.interface_font.clone())
    }
    pub(super) fn measure(&mut self, text: &str, size: f32, face: Face) -> f32 {
        let font = self.text_font();
        let key = format!("{text}\0{size}\0{font}\0{}", face as u8);
        if let Some(width) = self.widths.get(&key) {
            return *width;
        }
        let mut buffer = Buffer::new_empty(Metrics::new(size, size * 1.5));
        buffer.set_size(None, None);
        buffer.set_text(text, &face_attrs(&font, 0, face), Shaping::Advanced, None);
        buffer.shape_until_scroll(&mut self.fonts, false);
        let width = buffer.layout_runs().map(|run| run.line_w).fold(0., f32::max);
        if self.widths.len() > 4096 {
            self.widths.clear();
        }
        self.widths.insert(key, width);
        width
    }
    /// One line of text aligned inside a box and vertically centred.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn put(
        &mut self,
        text: &str,
        area: Rect,
        size: f32,
        tint: u32,
        face: Face,
        align: Align,
    ) {
        let size = size * self.scale;
        let width = if align == Align::Left {
            0.
        } else {
            self.measure(text, size, face)
        };
        let line = (size * 1.5).ceil() as i32;
        let x = match align {
            Align::Left => area.x,
            Align::Center => area.x + ((area.width as f32 - width) / 2.).max(0.) as i32,
            Align::Right => area.x + (area.width as f32 - width).max(0.) as i32,
        };
        let available = (area.x + area.width as i32 - x).max(1) as u32;
        self.label_face(
            text,
            Rect {
                x,
                y: area.y + (area.height as i32 - line) / 2,
                width: available + 2,
                height: line as u32,
            },
            size,
            tint,
            face,
        );
    }
    pub(super) fn fit(&mut self, text: &str, width: f32, size: f32) -> String {
        let size = size * self.scale;
        let mut text: String = text.into();
        if self.measure(&text, size, Face::Normal) <= width {
            return text;
        }
        while text.pop().is_some() {
            let candidate = format!("{text}…");
            if self.measure(&candidate, size, Face::Normal) <= width {
                return candidate;
            }
        }
        String::new()
    }
    /// Like `fit`, but keeps the end of the text too: "Chapter 3" and "Chapter 4" stay
    /// different as "Chap…r 3" and "Chap…r 4" where an end cut would show "Chap…" twice.
    pub(super) fn fit_middle(&mut self, text: &str, width: f32, size: f32) -> String {
        let scaled = size * self.scale;
        if self.measure(text, scaled, Face::Normal) <= width {
            return text.into();
        }
        let chars: Vec<char> = text.chars().collect();
        for keep in (1..chars.len()).rev() {
            let head = (keep * 3).div_ceil(5);
            let candidate: String = chars[..head].iter().chain(['…'].iter()).chain(chars[chars.len() - (keep - head)..].iter()).collect();
            if self.measure(&candidate, scaled, Face::Normal) <= width {
                return candidate;
            }
        }
        self.fit(text, width, size)
    }
    pub(super) fn button(&mut self, text: &str, rect: Rect, hit: Hit, look: Look, p: Palette) {
        self.button_face(text, rect, hit, look, Face::Normal, Align::Center, p);
    }
    pub(super) fn button_face(
        &mut self,
        text: &str,
        rect: Rect,
        hit: Hit,
        look: Look,
        face: Face,
        align: Align,
        p: Palette,
    ) {
        let hover = self.hot(rect);
        let radius = self.su(5.);
        let edge = mix(p.border, p.text, 0.2);
        let tint = match look {
            Look::Plain => {
                if hover {
                    self.renderer.fill_round(rect, radius, p.border);
                    p.text
                } else {
                    p.muted
                }
            }
            Look::Outline => {
                let fill = if hover { p.border } else { p.raised };
                self.renderer.framed(rect, radius, fill, edge);
                p.text
            }
            Look::Primary => {
                let fill = if hover { mix(p.accent, 0xffffff, 0.2) } else { p.accent };
                self.renderer.fill_round(rect, radius, fill);
                on_color(p.accent)
            }
            Look::Danger => {
                self.renderer
                    .fill_round(rect, radius, if hover { 0xd2655f } else { 0xc4504a });
                0xffffff
            }
            Look::Active => {
                self.renderer.fill_round(rect, radius, selected_fill(p.raised, p));
                p.text
            }
        };
        let inner = if align == Align::Left {
            Rect { x: rect.x + self.s(10.), width: rect.width.saturating_sub(self.su(14.)), ..rect }
        } else {
            rect
        };
        self.put(text, inner, 13., tint, face, align);
        self.hits.push((rect, hit));
    }
    pub(super) fn checked(&self, command: Command) -> bool {
        match command {
            Command::ToggleTree => self.show_tree,
            Command::TogglePanel => self.show_outline,
            Command::ToggleMarkers => self.settings.show_markers,
            Command::RawView => !self.settings.preview,
            Command::PreviewView => self.settings.preview,
            Command::HideNotes => self.settings.hide_notes,
            Command::PreviewNotes => self.settings.preview_notes,
            Command::Focus => self.window.as_ref().is_some_and(|w| w.fullscreen().is_some()),
            _ => false,
        }
    }
    pub(super) fn enabled(&self, command: Command) -> bool {
        let tab = self.active_id().and_then(|id| self.tabs.get(&id));
        let project = self.project.is_some();
        let selection = tab.is_some_and(|t| !t.document.selection().is_empty());
        let editable = tab.is_some_and(|t| t.path.is_some());
        match command {
            Command::Reopen => !self.closed.is_empty() && project,
            Command::Cut | Command::Copy => selection && project,
            Command::AddComment => selection && project,
            Command::AddWord => tab.is_some_and(|t| t.view.misspelled.iter().any(|(a, b)| t.document.cursor >= *a && t.document.cursor <= *b)),
            Command::Undo
            | Command::Redo
            | Command::SelectAll
            | Command::Paste
            | Command::Bold
            | Command::Italic
            | Command::AlignLeft
            | Command::AlignCenter
            | Command::AlignRight
            | Command::AlignJustify
            | Command::ToggleNote
            | Command::NoteParagraph
            | Command::Find
            | Command::Replace
            | Command::JumpScene
            | Command::GotoLine
            | Command::WordCount => editable && project,
            Command::CloseTab => tab.is_some() && project,
            Command::Rename | Command::Delete | Command::MoveUp | Command::MoveDown => project && self.selected_item().is_some(),
            Command::NewChapter
            | Command::NewPage
            | Command::NextComment
            | Command::PreviousComment
            | Command::ResolveComment
            | Command::ReplyComment
            | Command::EditComment
            | Command::DeleteComment
            | Command::Save
            | Command::SaveAs
            | Command::CloseProject
            | Command::Snapshot
            | Command::Export
            | Command::Preview
            | Command::BookDetails
            | Command::ProjectSettings
            | Command::ShowHistory
            | Command::ShowComments
            | Command::ProjectSearch
            | Command::QuickOpen
            | Command::NextTab
            | Command::NextChapter
            | Command::PreviousChapter
            | Command::PreviousTab
            | Command::SplitRight
            | Command::SplitDown
            | Command::CloseSplit => project,
            _ => true,
        }
    }
    pub(super) fn shortcut_of(&self, command: &commands::CommandInfo) -> String {
        commands::display_shortcut(
            self.settings
                .keymap
                .get(command.key)
                .map(String::as_str)
                .unwrap_or(command.shortcut),
        )
    }
    /// Status-bar hint for whatever button the pointer is over.
    pub(super) fn tip_at_mouse(&self) -> Option<(Rect, String)> {
        if self.modal() {
            return None;
        }
        let (rect, hit) = self
            .hits
            .iter()
            .rev()
            .find(|(rect, _)| contains(*rect, self.mouse.x, self.mouse.y))?;
        let command = match hit {
            Hit::Command(c) | Hit::Do(Act::Run(c)) => *c,
            Hit::Do(Act::MatchStep(d)) => return Some((*rect, if *d > 0 { "Next match  ·  Enter" } else { "Previous match  ·  Shift+Enter" }.into())),
            Hit::Do(Act::Case) => return Some((*rect, "Match case".into())),
            _ => return None,
        };
        let info = menu::info(command)?;
        let shortcut = self.shortcut_of(info);
        Some((
            *rect,
            if shortcut.is_empty() {
                info.name.into()
            } else {
                format!("{}   ·   {shortcut}", info.name)
            },
        ))
    }
    /// Floating label under the hovered button once the pointer has rested on it.
    pub(super) fn paint_tooltip(&mut self, window: Rect, p: Palette) {
        let resting = self.hover_since.is_some_and(|t| t.elapsed() >= Duration::from_millis(450));
        if !resting || self.drag.is_some() {
            return;
        }
        let Some((target, text)) = self.tip_at_mouse() else {
            return;
        };
        let size = 12. * self.scale;
        let width = self.measure(&text, size, Face::Normal).ceil() as u32 + self.su(20.);
        let height = self.su(26.);
        let x = (target.x + (target.width as i32 - width as i32) / 2).clamp(4, (window.width as i32 - width as i32 - 4).max(4));
        let below = target.y + target.height as i32 + self.s(6.);
        let y = if below + height as i32 + 4 > window.height as i32 { target.y - height as i32 - self.s(6.) } else { below };
        let rect = Rect { x, y, width, height };
        self.soft_shadow(rect);
        self.renderer.framed(rect, self.su(5.), p.raised, mix(p.border, p.text, 0.2));
        self.put(&text, rect, 12., p.text, Face::Normal, Align::Center);
    }

    pub(super) fn paint_menubar(&mut self, rect: Rect, p: Palette) {
        self.renderer.fill(rect, p.surface);
        self.renderer.fill(
            Rect {
                y: rect.y + rect.height as i32 - 1,
                height: 1,
                ..rect
            },
            p.border,
        );
        let mut x = rect.x + self.s(8.);
        for (index, (name, mnemonic, _)) in MENUS.iter().enumerate() {
            let width = self.measure(name, 13. * self.scale, Face::Normal).ceil() as u32 + self.su(22.);
            let title = Rect {
                x,
                y: rect.y + self.s(3.),
                width,
                height: rect.height.saturating_sub(self.su(7.)),
            };
            let open = self.popup.as_ref().is_some_and(|m| m.menu == Some(index));
            if open || (!self.modal() && contains(title, self.mouse.x, self.mouse.y)) {
                self.renderer.fill_round(title, self.su(5.), hovered(p.surface, p));
            }
            self.put(name, title, 13., p.text, Face::Normal, Align::Center);
            if self.modifiers.alt_key() {
                // Underline the Alt+letter that opens this menu.
                let size = 13. * self.scale;
                let at = name.to_lowercase().find(*mnemonic).unwrap_or(0);
                let total = self.measure(name, size, Face::Normal);
                let before = self.measure(&name[..at], size, Face::Normal);
                let letter = self.measure(&name[at..at + 1], size, Face::Normal);
                let line = Rect {
                    x: title.x + ((title.width as f32 - total) / 2. + before) as i32,
                    y: title.y + title.height as i32 * 3 / 4 + self.s(1.),
                    width: letter.ceil() as u32,
                    height: 1,
                };
                self.renderer.fill(line, p.text);
            }
            self.hits.push((title, Hit::Do(Act::Menu(index))));
            x += width as i32;
        }
    }
    pub(super) fn open_menu(&mut self, index: usize, anchor: Rect) {
        self.popup = Some(Popup {
            x: anchor.x,
            y: anchor.y + anchor.height as i32,
            items: MENUS[index].2,
            menu: Some(index),
            selected: None,
        });
    }
    pub(super) fn open_popup(&mut self, items: menu::Items, x: i32, y: i32) {
        self.popup = Some(Popup {
            x,
            y,
            items,
            menu: None,
            selected: None,
        });
    }
    pub(super) fn popup_for(&mut self, hit: Option<&Hit>) -> bool {
        let (x, y) = (self.mouse.x as i32, self.mouse.y as i32);
        let items = match hit {
            Some(Hit::Tab(..)) => TAB_MENU,
            Some(Hit::Node(_)) => TREE_MENU,
            None if !self.modal() => EDITOR_MENU,
            _ => return false,
        };
        self.open_popup(items, x, y);
        true
    }
    pub(super) fn paint_popup(&mut self, window: Rect, p: Palette) {
        let Some(popup) = &self.popup else {
            return;
        };
        let (items, left, top, keyed) = (popup.items, popup.x, popup.y, popup.selected);
        self.in_modal = true;
        self.hits.push((window, Hit::Do(Act::Dismiss)));
        let size = 13. * self.scale;
        let (row, separator, pad) = (self.s(28.), self.s(9.), self.s(5.));
        let mut width = 0f32;
        for command in items.iter().flatten() {
            if let Some(info) = menu::info(*command) {
                let shortcut = self.shortcut_of(info);
                width = width.max(
                    self.measure(info.name, size, Face::Normal)
                        + self.measure(&shortcut, size, Face::Normal)
                        + self.s(70.) as f32,
                );
            }
        }
        let width = (width.ceil() as u32).max(self.su(200.));
        let height = pad * 2
            + items
                .iter()
                .map(|item| if item.is_some() { row } else { separator })
                .sum::<i32>();
        let rect = Rect {
            x: left.min(window.width as i32 - width as i32 - 4).max(0),
            y: top.min(window.height as i32 - height - 4).max(0),
            width,
            height: height as u32,
        };
        self.soft_shadow(rect);
        self.renderer.framed(rect, self.su(7.), p.raised, mix(p.border, p.text, 0.15));
        self.hits.push((rect, Hit::Do(Act::Block)));
        let mut y = rect.y + pad;
        for (position, item) in items.iter().enumerate() {
            let Some(command) = item else {
                self.renderer.fill(
                    Rect {
                        x: rect.x + self.s(10.),
                        y: y + separator / 2,
                        width: rect.width.saturating_sub(self.su(20.)),
                        height: 1,
                    },
                    p.border,
                );
                y += separator;
                continue;
            };
            let Some(info) = menu::info(*command) else {
                continue;
            };
            let line = Rect {
                x: rect.x + self.s(4.),
                y,
                width: rect.width.saturating_sub(self.su(8.)),
                height: row as u32,
            };
            let enabled = self.enabled(*command);
            if enabled && (keyed == Some(position) || (keyed.is_none() && contains(line, self.mouse.x, self.mouse.y))) {
                self.renderer.fill_round(line, self.su(5.), selected_fill(p.raised, p));
            }
            let tint = if enabled { p.text } else { p.faint(p.raised) };
            if self.checked(*command) {
                let tick = Rect { x: line.x + self.s(6.), width: self.su(18.), ..line };
                self.put("✓", tick, 12., p.accent_text(), Face::Normal, Align::Left);
            }
            let text = Rect {
                x: line.x + self.s(28.),
                width: line.width.saturating_sub(self.su(40.)),
                ..line
            };
            self.put(info.name, text, 13., tint, Face::Normal, Align::Left);
            let shortcut = self.shortcut_of(info);
            self.put(&shortcut, text, 12., p.muted, Face::Normal, Align::Right);
            self.hits.push((
                line,
                Hit::Do(if enabled { Act::Run(*command) } else { Act::Block }),
            ));
            y += row;
        }
        self.in_modal = false;
    }

    /// Tab sizing shared by painting and drag-and-drop: (first visible tab, how many fit, tab width).
    pub(super) fn tab_geometry(&self, pane: &Pane, rect: Rect) -> (usize, usize, u32) {
        let usable = rect.width.saturating_sub(self.su(34.));
        let count = pane.tabs.len().max(1) as u32;
        let width = (usable / count).clamp(self.su(128.), self.su(200.));
        let fit = (usable / width).max(1) as usize;
        let first = if pane.tabs.len() <= fit {
            0
        } else {
            pane.active.saturating_sub(fit - 1)
        };
        (first, fit, width)
    }
    fn paint_tabs(&mut self, pane: &Pane, pane_id: u64, rect: Rect, header: u32, p: Palette) {
        self.renderer.fill(Rect { height: header, ..rect }, p.surface);
        let (first, fit, width) = self.tab_geometry(pane, rect);
        let mut shown = 0;
        for (column, index) in (first..pane.tabs.len()).take(fit).enumerate() {
            shown += 1;
            let id = pane.tabs[index];
            let tab = &self.tabs[&id];
            let (name, dirty) = (tab.name(), tab.document.dirty());
            let tab_rect = Rect {
                x: rect.x + column as i32 * width as i32,
                y: rect.y,
                width,
                height: header,
            };
            let hover = self.hot(tab_rect);
            let active = pane.active == index;
            if active {
                self.renderer.fill(tab_rect, p.background);
                self.renderer.fill(
                    Rect { height: self.su(2.), ..tab_rect },
                    if pane_id == self.active_pane { p.accent } else { p.border },
                );
            } else if hover {
                self.renderer.fill(tab_rect, mix(p.surface, p.background, 0.5));
            }
            let room = width.saturating_sub(self.su(46.)) as f32;
            let text = self.fit_middle(&name, room, 12.5);
            self.put(
                &text,
                Rect { x: tab_rect.x + self.s(12.), width: room as u32, ..tab_rect },
                12.5,
                if active { p.text } else { p.muted },
                Face::Normal,
                Align::Left,
            );
            if !active && pane.active != index + 1 {
                let line = Rect { x: tab_rect.x + width as i32 - 1, y: tab_rect.y + header as i32 * 28 / 100, width: 1, height: header * 44 / 100 };
                self.renderer.fill(line, p.border);
            }
            self.hits.push((tab_rect, Hit::Tab(pane_id, index)));
            let close = Rect {
                x: tab_rect.x + width as i32 - self.s(26.),
                y: tab_rect.y + (header as i32 - self.s(20.)) / 2,
                width: self.su(20.),
                height: self.su(20.),
            };
            if self.hot(close) {
                self.renderer.fill_round(close, self.su(4.), p.border);
            }
            // The close button doubles as the unsaved marker until the pointer is over the tab.
            let glyph = if dirty && !hover && !self.hot(close) { "●" } else { "×" };
            if active || hover || dirty {
                self.put(glyph, close, 14., if dirty && glyph == "●" { p.accent_text() } else { p.muted }, Face::Normal, Align::Center);
            }
            self.hits.push((close, Hit::Close(id)));
        }
        let plus = Rect {
            x: rect.x + shown * width as i32 + self.s(4.),
            y: rect.y + self.s(5.),
            width: self.su(26.),
            height: header.saturating_sub(self.su(10.)),
        };
        self.button("+", plus, Hit::NewTab(pane_id), Look::Plain, p);
    }
    fn paint_toolbar(&mut self, rect: Rect, format: prose::Format, p: Palette) {
        let mut x = rect.x + self.s(14.);
        let height = rect.height.saturating_sub(self.su(10.));
        let y = rect.y + self.s(5.);
        let tools: [(&str, Face, Option<Hit>); 16] = [
            ("B", Face::Bold, Some(Hit::Command(Command::Bold))),
            ("I", Face::Italic, Some(Hit::Command(Command::Italic))),
            ("", Face::Normal, None),
            ("Left", Face::Normal, Some(Hit::Command(Command::AlignLeft))),
            ("Center", Face::Normal, Some(Hit::Command(Command::AlignCenter))),
            ("Right", Face::Normal, Some(Hit::Command(Command::AlignRight))),
            ("Justify", Face::Normal, Some(Hit::Command(Command::AlignJustify))),
            ("", Face::Normal, None),
            ("Raw", Face::Normal, Some(Hit::Command(Command::RawView))),
            ("Preview", Face::Normal, Some(Hit::Command(Command::PreviewView))),
            ("", Face::Normal, None),
            ("Note", Face::Normal, Some(Hit::Command(Command::ToggleNote))),
            ("Comment", Face::Normal, Some(Hit::Command(Command::AddComment))),
            ("", Face::Normal, None),
            ("Find", Face::Normal, Some(Hit::Command(Command::Find))),
            ("Scenes", Face::Normal, Some(Hit::Command(Command::JumpScene))),
        ];
        for (text, face, hit) in tools {
            let Some(hit) = hit else {
                self.renderer.fill(
                    Rect { x: x + self.s(4.), y: y + self.s(5.), width: 1, height: height.saturating_sub(self.su(10.)) },
                    p.border,
                );
                x += self.s(10.);
                continue;
            };
            let width = (self.measure(text, 13. * self.scale, face).ceil() as u32 + self.su(18.)).max(self.su(30.));
            if x + width as i32 > rect.x + rect.width as i32 - self.s(8.) {
                break;
            }
            let on = match &hit {
                Hit::Command(Command::Bold) => format.bold,
                Hit::Command(Command::Italic) => format.italic,
                Hit::Command(Command::ToggleNote) => format.note,
                Hit::Command(Command::AlignLeft) => format.align == Some(prose::Align::Left),
                Hit::Command(Command::AlignCenter) => format.align == Some(prose::Align::Center),
                Hit::Command(Command::AlignRight) => format.align == Some(prose::Align::Right),
                Hit::Command(Command::AlignJustify) => format.align == Some(prose::Align::Justify),
                Hit::Command(Command::RawView) => !self.settings.preview,
                Hit::Command(Command::PreviewView) => self.settings.preview,
                _ => false,
            };
            let enabled = match &hit {
                Hit::Command(c) => self.enabled(*c),
                _ => true,
            };
            let look = if on { Look::Active } else { Look::Plain };
            if enabled {
                self.button_face(text, Rect { x, y, width, height }, hit, look, face, Align::Center, p);
            } else {
                self.put(text, Rect { x, y, width, height }, 13., p.faint(p.surface), face, Align::Center);
            }
            x += width as i32 + self.s(2.);
        }
    }
    pub(super) fn paint_pane(&mut self, pane_id: u64, rect: Rect, focus: bool, p: Palette) {
        let Some(pane) = self.split.pane(pane_id).cloned() else {
            return;
        };
        let header = if focus { 0 } else { self.su(36.) };
        let toolbar = if focus { 0 } else { self.su(38.) };
        if !focus {
            self.paint_tabs(&pane, pane_id, rect, header, p);
            let marker = self.marker();
            let format = pane
                .tab()
                .and_then(|id| self.tabs.get(&id))
                .map(|t| {
                    let line = t.document.text.char_to_line(t.document.cursor.min(t.document.text.len_chars()));
                    let start = t.document.text.line_to_char(line);
                    let from = start.max(t.document.cursor.saturating_sub(1000));
                    let end = if line + 1 < t.document.text.len_lines() { t.document.text.line_to_char(line + 1) } else { t.document.text.len_chars() };
                    let to = end.min(t.document.cursor + 1000);
                    let text = t.document.text.slice(from..to).to_string();
                    let head: String = t.document.text.slice(start..).chars().take(4).collect();
                    prose::Format { align: prose::align_tag(&head).map(|t| t.0), ..prose::format_at(&text, t.document.cursor - from, &marker) }
                })
                .unwrap_or_default();
            self.paint_toolbar(Rect { y: rect.y + header as i32, height: toolbar, ..rect }, format, p);
            self.renderer.fill(Rect { y: rect.y + (header + toolbar) as i32, height: 1, ..rect }, p.border);
        }
        let Some(id) = pane.tab() else {
            return;
        };
        let find = self
            .overlay
            .as_ref()
            .filter(|o| o.docked() && pane_id == self.active_pane)
            .map(|o| o.kind == Kind::Replace);
        let find_height = match find {
            Some(true) => self.su(92.),
            Some(false) => self.su(50.),
            None => 0,
        };
        if find.is_some() {
            let bar_top = rect.y + (header + toolbar) as i32 + i32::from(!focus);
            self.paint_findbar(Rect { y: bar_top, height: find_height, ..rect }, p);
        }
        let padding = self.su(27.);
        let top = header + toolbar + find_height;
        let width = rect
            .width
            .saturating_sub(padding * 2)
            .min((self.settings.max_width.unwrap_or(10000.) * self.scale) as u32)
            .max(1);
        let editor_rect = Rect {
            x: rect.x + (rect.width.saturating_sub(width) / 2) as i32,
            y: rect.y + (top + padding) as i32,
            width,
            height: rect.height.saturating_sub(top + padding * 2).max(1),
        };
        let mut settings = self.settings.clone();
        settings.font_size *= self.scale;
        if settings.preview {
            // The finished look follows the export preset the Export command starts from.
            let preset = self
                .project
                .as_ref()
                .and_then(|state| state.presets.get(&settings.default_preset))
                .unwrap_or_default();
            settings.preview_indent = (preset.indent * 72. / preset.font_size.max(1.) * 2.).round().clamp(0., 12.) as u8;
            settings.preview_flush = preset.flush_after_break;
            settings.preview_justify = preset.justify;
        }
        let highlights: Vec<(usize, usize)> = if find.is_some() {
            self.overlay
                .as_ref()
                .filter(|o| !o.query.is_empty() && o.tab == Some(id))
                .map(|o| {
                    o.rows
                        .iter()
                        .filter_map(|r| match &r.action {
                            Action::Match(m) => Some((m.start, m.end)),
                            _ => None,
                        })
                        .collect()
                })
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        let symbol = self.scene_symbol();
        let tab = self.tabs.get_mut(&id).unwrap();
        tab.view.highlights = highlights;
        tab.view.configure(&settings, &symbol);
        tab.view.layout(&tab.document, &tab.blocks, &settings, &mut self.fonts, editor_rect);
        // The empty pane of a new split has no file, so no text and no caret.
        let vacant = tab.path.is_none();
        if !vacant {
            tab.view.paint(&tab.document, &mut self.fonts, &mut self.renderer);
        }
        let empty = tab.document.text.len_chars() == 0 && !tab.loading && !vacant;
        let (len_chars, len_lines, top_char) =
            (tab.document.text.len_chars(), tab.document.text.len_lines(), tab.view.top);
        if empty && !focus {
            let hint = Rect {
                x: editor_rect.x + self.s(10.),
                y: editor_rect.y,
                width: editor_rect.width,
                height: (settings.font_size * 1.65) as u32,
            };
            self.put("Start writing…", hint, 16., p.faint(p.background), Face::Normal, Align::Left);
        }
        if vacant {
            let hint = Rect { y: editor_rect.y + editor_rect.height as i32 / 3, height: self.su(24.), ..editor_rect };
            self.put("Click a chapter or note in the tree to open it here.", hint, 13., p.muted, Face::Normal, Align::Center);
        }
        if !focus && !vacant {
            let scrollbar = Rect {
                x: rect.x + rect.width as i32 - self.s(9.),
                y: editor_rect.y,
                width: self.su(5.),
                height: editor_rect.height,
            };
            let fraction = top_char as f64 / len_chars.max(1) as f64;
            let estimated = (len_chars as f32
                / (editor_rect.width as f32 / (settings.font_size * 0.55)).max(1.))
            .max(len_lines as f32);
            let thumb = (scrollbar.height as f32
                * (editor_rect.height as f32 / (estimated * settings.font_size * 1.65)).min(1.))
            .max(self.su(24.) as f32) as u32;
            let thumb = thumb.min(scrollbar.height);
            let track = Rect {
                y: scrollbar.y + (fraction * scrollbar.height.saturating_sub(thumb) as f64) as i32,
                height: thumb,
                ..scrollbar
            };
            let hover = contains(
                Rect { x: scrollbar.x - 6, width: self.su(16.), ..scrollbar },
                self.mouse.x,
                self.mouse.y,
            );
            self.renderer.fill_round(track, self.su(3.), if hover { p.muted } else { p.border });
            self.hits.push((
                Rect { x: scrollbar.x - self.s(6.), width: self.su(16.), ..scrollbar },
                Hit::Scrollbar(id),
            ));
        }
        self.request_spelling(id);
    }
    fn paint_findbar(&mut self, rect: Rect, p: Palette) {
        let Some(overlay) = &self.overlay else {
            return;
        };
        let (replace, query, replacement, secondary, caret, case, focused, whole) = (
            overlay.kind == Kind::Replace,
            overlay.query.clone(),
            overlay.replacement.clone(),
            overlay.secondary,
            overlay.caret,
            overlay.case,
            overlay.focused,
            overlay.select_all,
        );
        let (found, selected, pending) = (overlay.rows.len(), overlay.selected, overlay.pending);
        self.renderer.fill(rect, p.surface);
        self.renderer.fill(Rect { y: rect.y + rect.height as i32 - 1, height: 1, ..rect }, p.border);
        let pad = self.s(12.);
        let height = self.su(32.);
        let y = rect.y + self.s(9.);
        let mut right = rect.x + rect.width as i32 - pad;
        let close = Rect { x: right - self.s(30.), y, width: self.su(30.), height };
        self.button("×", close, Hit::Do(Act::Dismiss), Look::Plain, p);
        right = close.x - self.s(4.);
        let label = if replace { "Replace ▴" } else { "Replace ▾" };
        let width = self.measure(label, 13. * self.scale, Face::Normal).ceil() as u32 + self.su(22.);
        let toggle = Rect { x: right - width as i32, y, width, height };
        self.button(label, toggle, Hit::Do(Act::ToggleReplace), if replace { Look::Active } else { Look::Plain }, p);
        right = toggle.x - self.s(6.);
        let aa = Rect { x: right - self.s(36.), y, width: self.su(36.), height };
        self.button("Aa", aa, Hit::Do(Act::Case), if case { Look::Active } else { Look::Plain }, p);
        right = aa.x - self.s(4.);
        let down = Rect { x: right - self.s(30.), y, width: self.su(30.), height };
        self.button("▼", down, Hit::Do(Act::MatchStep(1)), Look::Plain, p);
        let up = Rect { x: down.x - self.s(30.), ..down };
        self.button("▲", up, Hit::Do(Act::MatchStep(-1)), Look::Plain, p);
        let counter = Rect { x: up.x - self.s(96.), y, width: self.su(92.), height };
        let text = if pending || query.is_empty() {
            String::new()
        } else if found == 0 {
            "No results".into()
        } else if found >= 500 {
            "500+ matches".into()
        } else {
            format!("{} of {found}", selected + 1)
        };
        self.put(&text, counter, 12., if found == 0 && !query.is_empty() { p.danger_text() } else { p.muted }, Face::Normal, Align::Right);
        let width = (counter.x - rect.x - pad - self.s(8.)).max(80) as u32;
        let first = Rect { x: rect.x + pad, y, width, height };
        self.field(first, &query, "Find…", if secondary { None } else { caret }, focused && !secondary, whole, Act::Field(false), p);
        if replace {
            let y2 = y + self.s(40.);
            let all = Rect { x: close.x + close.width as i32 - self.s(104.), y: y2, width: self.su(104.), height };
            self.button("Replace all", all, Hit::Do(Act::Replace(true)), Look::Outline, p);
            let one = Rect { x: all.x - self.s(88.), y: y2, width: self.su(80.), height };
            self.button("Replace", one, Hit::Do(Act::Replace(false)), Look::Primary, p);
            let width = (one.x - rect.x - pad - self.s(8.)).max(80) as u32;
            let second = Rect { x: rect.x + pad, y: y2, width, height };
            self.field(second, &replacement, "Replace with…", if secondary { caret } else { None }, focused && secondary, whole, Act::Field(true), p);
        }
    }
    /// A see-through copy of the dragged tab at the pointer, plus a line where it will be inserted.
    pub(super) fn paint_tab_ghost(&mut self, tab: u64, p: Palette) {
        let Some(name) = self.tabs.get(&tab).map(|t| t.name()) else {
            return;
        };
        let (x, y) = (self.mouse.x as i32, self.mouse.y as i32);
        // Insertion marker inside the tab strip under the pointer.
        let strip = self.pane_rects.iter().find(|(_, r)| {
            contains(*r, self.mouse.x, self.mouse.y) && self.mouse.y < (r.y + self.s(38.)) as f64
        });
        if let Some((pane_id, rect)) = strip.copied() {
            if let Some(pane) = self.split.pane(pane_id) {
                let (first, fit, width) = self.tab_geometry(pane, rect);
                let index = self.tab_drop_index(pane, rect, self.mouse.x);
                let slot = index.saturating_sub(first).min(fit);
                let marker = Rect {
                    x: rect.x + (slot as u32 * width) as i32 - self.s(1.),
                    y: rect.y + self.s(4.),
                    width: self.su(3.),
                    height: self.su(28.),
                };
                self.renderer.fill_round(marker, self.su(1.), p.accent);
            }
        }
        let size = Rect {
            x: x - self.s(80.),
            y: y - self.s(15.),
            width: self.su(160.),
            height: self.su(30.),
        };
        self.soft_shadow(size);
        self.renderer.shade(size, mix(p.raised, p.text, 0.16), 190);
        for edge in [
            Rect { height: self.su(2.), ..size },
            Rect { y: size.y + size.height as i32 - 1, height: 1, ..size },
            Rect { width: 1, ..size },
            Rect { x: size.x + size.width as i32 - 1, width: 1, ..size },
        ] {
            self.renderer.shade(edge, p.accent, 190);
        }
        let label = self.fit(&name, size.width as f32 - self.s(24.) as f32, 12.5);
        self.put(
            &label,
            Rect { x: size.x + self.s(12.), width: size.width - self.su(24.), ..size },
            12.5,
            mix(p.text, p.raised, 0.12),
            Face::Normal,
            Align::Left,
        );
    }
    pub(super) fn paint_dividers(&mut self, p: Palette) {
        for (index, divider) in self.dividers.clone().into_iter().enumerate() {
            let hot = contains(divider.strip, self.mouse.x, self.mouse.y)
                || matches!(&self.drag, Some(Drag::Divider(i)) if *i == index);
            let line = if divider.down {
                Rect { y: divider.strip.y + 2, height: divider.strip.height.saturating_sub(4).max(2), ..divider.strip }
            } else {
                Rect { x: divider.strip.x + 2, width: divider.strip.width.saturating_sub(4).max(2), ..divider.strip }
            };
            self.renderer.fill(line, if hot { p.accent } else { p.border });
            self.hits.push((divider.strip, Hit::Divider(index)));
        }
    }

    pub(super) fn paint_tree(&mut self, rect: Rect, p: Palette) {
        self.renderer.fill(rect, p.surface);
        self.renderer.fill(Rect { x: rect.x + rect.width as i32 - 1, width: 1, ..rect }, p.border);
        let rows = self.tree_rows();
        let Some(state) = &self.project else {
            return;
        };
        let selected = state.selected.clone();
        let dragged_is_chapter = match &self.drag {
            Some(Drag::Node(id, _)) => state.core.manifest.tree.find(id).is_some_and(|n| !n.is_part()),
            _ => false,
        };
        let active = self.active_item();
        let row_height = self.su(28.);
        let footer = self.su(36.);
        let list_top = rect.y + self.s(4.);
        let capacity = (rect.height.saturating_sub(footer + self.su(4.)) / row_height) as usize;
        if std::mem::take(&mut self.tree_reveal) {
            if let Some(at) = selected.as_deref().and_then(|id| rows.iter().position(|r| r.id == id)) {
                if at < self.tree_top {
                    self.tree_top = at.saturating_sub(1);
                } else if at >= self.tree_top + capacity.max(1) {
                    self.tree_top = at + 1 - capacity.max(1);
                }
            }
        }
        self.tree_top = self.tree_top.min(rows.len().saturating_sub(capacity));
        let drag = match &self.drag {
            Some(Drag::Node(id, origin)) if (self.mouse.x - origin.x).abs() + (self.mouse.y - origin.y).abs() > 8. => Some(id.clone()),
            _ => None,
        };
        for (index, row) in rows.iter().enumerate().skip(self.tree_top).take(capacity.max(1)) {
            let line = Rect {
                x: rect.x + self.s(6.),
                y: list_top + (index - self.tree_top) as i32 * row_height as i32,
                width: rect.width.saturating_sub(self.su(12.)),
                height: row_height,
            };
            if let Some((label, section)) = row.heading {
                let head = Rect { x: line.x + self.s(6.), y: line.y + self.s(4.), width: line.width, height: line.height };
                self.put(label, head, 10.5, mix(p.muted, p.accent_text(), 0.5), Face::Bold, Align::Left);
                let text = "+ New";
                let width = self.measure(text, 11. * self.scale, Face::Normal).ceil() as u32 + self.su(16.);
                let add = Rect { x: line.x + line.width as i32 - width as i32 - self.s(2.), y: line.y + self.s(2.), width, height: line.height - self.su(4.) };
                let command = if section == manifest::Section::Notes { Command::NewPage } else { Command::NewChapter };
                self.button(text, add, Hit::Command(command), Look::Plain, p);
                continue;
            }
            let is_selected = selected.as_deref() == Some(row.id.as_str());
            let is_active = active.as_deref() == Some(row.id.as_str());
            let dragging_over = drag.is_some() && drag.as_deref() != Some(row.id.as_str()) && contains(line, self.mouse.x, self.mouse.y);
            if is_selected {
                self.renderer.fill_round(line, self.su(5.), selected_fill(p.surface, p));
            } else if is_active {
                self.renderer.fill_round(line, self.su(5.), mix(p.surface, p.accent, 0.1));
            } else if self.hot(line) && drag.is_none() {
                self.renderer.fill_round(line, self.su(5.), hovered(p.surface, p));
            }
            let indent = self.s(8.) + row.depth as i32 * self.s(14.);
            let mut text_x = line.x + indent;
            if row.part {
                let arrow = Rect { x: text_x - self.s(4.), width: self.su(22.), ..line };
                self.put(if row.collapsed { "▸" } else { "▾" }, arrow, 10., p.muted, Face::Normal, Align::Center);
                self.hits.push((arrow, Hit::Fold(row.id.clone())));
                text_x += self.s(20.);
            } else {
                self.icon_doc(text_x + self.s(2.), line.y + (line.height as i32 - self.s(14.)) / 2, p.muted, p.surface);
                text_x += self.s(24.);
            }
            let words = if row.words > 0 || !row.part { thousands(row.words as u64) } else { String::new() };
            let words_width = self.measure(&words, 11. * self.scale, Face::Normal).ceil() as i32 + self.s(8.);
            let badge = if row.open > 0 { self.measure(&format!("●{}", row.open), 10.5 * self.scale, Face::Normal).ceil() as i32 + self.s(6.) } else { 0 };
            let room = (line.x + line.width as i32 - text_x - words_width - badge - self.s(6.)).max(10);
            let title = self.fit(&row.title, room as f32, 13.);
            self.put(&title, Rect { x: text_x, width: room as u32, ..line }, 13., p.text, if row.part { Face::Bold } else { Face::Normal }, Align::Left);
            if row.open > 0 {
                let at = Rect { x: line.x + line.width as i32 - words_width - badge, width: badge as u32, ..line };
                self.put(&format!("●{}", row.open), at, 10.5, p.accent_text(), Face::Normal, Align::Left);
            }
            let at = Rect { x: line.x, width: line.width - self.su(6.), ..line };
            self.put(&words, at, 11., p.muted, Face::Normal, Align::Right);
            if dragging_over {
                let part_target = row.part && dragged_is_chapter;
                if part_target {
                    self.renderer.framed(line, self.su(5.), mix(p.surface, p.accent, 0.18), p.accent);
                    self.put(&title, Rect { x: text_x, width: room as u32, ..line }, 13., p.text, Face::Bold, Align::Left);
                } else {
                    self.renderer.fill(Rect { y: line.y - 1, height: 2, ..line }, p.accent);
                }
            }
            self.hits.push((line, Hit::Node(row.id.clone())));
        }
        self.scroll_thumb(Rect { y: list_top, height: capacity as u32 * row_height, ..rect }, self.tree_top, capacity, rows.len(), p);
        // Totals
        let state = self.project.as_ref().unwrap();
        let total = state.core.manuscript_words();
        let goal = state.core.manifest.settings.goals.project_words as u64;
        let foot = Rect { y: rect.y + rect.height as i32 - footer as i32, height: footer, ..rect };
        self.renderer.fill(Rect { height: 1, ..foot }, p.border);
        let label = if goal > 0 { format!("{} of {} words", thousands(total), thousands(goal)) } else { count_words(total) };
        self.put(&label, Rect { x: foot.x + self.s(14.), y: foot.y + self.s(2.), width: foot.width - self.su(28.), height: self.su(22.) }, 11.5, p.muted, Face::Normal, Align::Left);
        if goal > 0 {
            let bar = Rect { x: foot.x + self.s(14.), y: foot.y + foot.height as i32 - self.s(10.), width: foot.width - self.su(28.), height: self.su(4.) };
            self.renderer.fill_round(bar, self.su(2.), p.border);
            let done = ((total as f64 / goal as f64).min(1.) * bar.width as f64) as u32;
            self.renderer.fill_round(Rect { width: done.max(1), ..bar }, self.su(2.), p.accent);
        }
        self.hits.push((
            Rect { x: rect.x + rect.width as i32 - self.s(3.), width: self.su(6.), ..rect },
            Hit::TreeResize,
        ));
    }
    pub(super) fn scroll_panel(&mut self, rows: i32) {
        self.outline_top = (self.outline_top as i32 + rows).max(0) as usize;
    }
    pub(super) fn paint_panel(&mut self, rect: Rect, p: Palette) {
        self.renderer.fill(rect, p.surface);
        self.renderer.fill(Rect { width: 1, ..rect }, p.border);
        let Some(state) = &self.project else {
            return;
        };
        let panel = state.panel;
        let open = state.comments.comments.iter().filter(|c| c.status == CommentStatus::Open).count();
        // Tabs
        let tab_height = self.su(34.);
        let half = rect.width / 2;
        for (index, (label, which)) in [("Scenes".to_owned(), Panel::Scenes), (if open > 0 { format!("Comments ({open})") } else { "Comments".to_owned() }, Panel::Comments)].into_iter().enumerate() {
            let cell = Rect { x: rect.x + index as i32 * half as i32, width: half, height: tab_height, ..rect };
            let on = panel == which;
            if on {
                self.renderer.fill(Rect { y: cell.y + cell.height as i32 - self.s(2.), height: self.su(2.), ..cell }, p.accent);
            } else if self.hot(cell) {
                self.renderer.fill(cell, hovered(p.surface, p));
            }
            self.put(&label, cell, 12.5, if on { p.text } else { p.muted }, if on { Face::Bold } else { Face::Normal }, Align::Center);
            self.hits.push((cell, Hit::Do(Act::Panel(which))));
        }
        self.renderer.fill(Rect { y: rect.y + tab_height as i32, height: 1, ..rect }, p.border);
        let body = Rect { y: rect.y + tab_height as i32 + 1, height: rect.height - tab_height - 1, ..rect };
        match panel {
            Panel::Scenes => self.paint_scenes(body, p),
            Panel::Comments => self.paint_comments(body, p),
        }
        self.hits.push((
            Rect { x: rect.x - self.s(3.), width: self.su(6.), ..rect },
            Hit::OutlineResize,
        ));
    }
    fn paint_scenes(&mut self, rect: Rect, p: Palette) {
        let Some(id) = self.active_id() else { return };
        let tab = &self.tabs[&id];
        let text = &tab.document.text;
        let cursor_line = text.char_to_line(tab.document.cursor.min(text.len_chars()));
        let mut starts: Vec<usize> = vec![0];
        starts.extend(tab.blocks.scenes.iter().copied().filter(|l| *l < text.len_lines()));
        let current = starts.iter().rposition(|l| *l <= cursor_line).unwrap_or(0);
        let snippets: Vec<String> = starts
            .iter()
            .enumerate()
            .map(|(n, line)| {
                (*line..text.len_lines())
                    .skip(usize::from(n > 0))
                    .take(40)
                    .map(|l| text.line(l).to_string().trim().to_owned())
                    .find(|l| !l.is_empty() && !prose::is_scene_break(l))
                    .map(|l| crate::core::notes::strip(&l).replace('*', "").trim().to_owned())
                    .unwrap_or_default()
            })
            .collect();
        let head = Rect { x: rect.x + self.s(14.), y: rect.y + self.s(8.), width: rect.width.saturating_sub(self.su(28.)), height: self.su(22.) };
        self.put(&format!("{} scene{} in this chapter", starts.len(), if starts.len() == 1 { "" } else { "s" }), head, 11.5, p.muted, Face::Normal, Align::Left);
        let row_height = self.s(48.);
        let top = rect.y + self.s(36.);
        let capacity = ((rect.height as i32 - self.s(40.)) / row_height).max(1) as usize;
        self.outline_top = self.outline_top.min(starts.len().saturating_sub(capacity));
        for (n, line) in starts.iter().enumerate().skip(self.outline_top).take(capacity) {
            let row = Rect { x: rect.x + self.s(7.), y: top + (n - self.outline_top) as i32 * row_height, width: rect.width.saturating_sub(self.su(14.)), height: row_height as u32 - 2 };
            if n == current {
                self.renderer.fill_round(row, self.su(5.), selected_fill(p.surface, p));
            } else if self.hot(row) {
                self.renderer.fill_round(row, self.su(5.), hovered(p.surface, p));
            }
            self.put(&format!("Scene {}", n + 1), Rect { x: row.x + self.s(10.), y: row.y + self.s(2.), width: row.width - self.su(20.), height: self.su(20.) }, 11., if n == current { p.accent_text() } else { p.muted }, Face::Bold, Align::Left);
            let snippet = self.fit(if snippets[n].is_empty() { "(empty)" } else { &snippets[n] }, row.width as f32 - self.s(20.) as f32, 12.5);
            self.put(&snippet, Rect { x: row.x + self.s(10.), y: row.y + self.s(22.), width: row.width - self.su(20.), height: self.su(22.) }, 12.5, p.text, Face::Normal, Align::Left);
            self.hits.push((row, Hit::Scene(*line)));
        }
        self.scroll_thumb(Rect { y: top, height: (capacity as i32 * row_height) as u32, ..rect }, self.outline_top, capacity, starts.len(), p);
    }
    fn paint_comments(&mut self, rect: Rect, p: Palette) {
        let Some(state) = &self.project else { return };
        let filter = state.filter.clone();
        let active_item = self.active_item();
        let tree = &state.core.manifest.tree;
        let order: Vec<String> = [manifest::Section::Manuscript, manifest::Section::Notes]
            .into_iter()
            .flat_map(|s| tree.leaves(s).into_iter().map(|(n, _)| n.id().to_owned()).collect::<Vec<_>>())
            .collect();
        let query = crate::core::comments::Filter {
            status: filter.status,
            tag: filter.tag.clone(),
            item: if filter.this_chapter { active_item.clone() } else { None },
            orphaned: None,
        };
        let mut list: Vec<crate::core::comments::Comment> = state.comments.query(&query).into_iter().cloned().collect();
        list.sort_by_key(|c| (order.iter().position(|o| *o == c.item).unwrap_or(usize::MAX), c.anchor.start));
        let titles: HashMap<String, String> = order.iter().filter_map(|id| Some((id.clone(), tree.find(id)?.title().to_owned()))).collect();
        let active = state.active_comment.clone();
        let total = state.comments.comments.len();
        // Filters
        let bar_y = rect.y + self.s(8.);
        let mut x = rect.x + self.s(10.);
        let height = self.su(26.);
        let chips: [(String, bool, &'static str); 3] = [
            (match filter.status { None => "All".into(), Some(CommentStatus::Open) => "Open".into(), Some(CommentStatus::Resolved) => "Resolved".into() }, filter.status.is_some(), "status"),
            (filter.tag.clone().map_or("Any tag".into(), |t| t), filter.tag.is_some(), "tag"),
            ("This chapter".into(), filter.this_chapter, "chapter"),
        ];
        for (label, on, key) in chips {
            let width = self.measure(&label, 11.5 * self.scale, Face::Normal).ceil() as u32 + self.su(18.);
            if x + width as i32 > rect.x + rect.width as i32 - self.s(8.) {
                break;
            }
            self.button(&label, Rect { x, y: bar_y, width, height }, Hit::Do(Act::CommentFilter(key)), if on { Look::Active } else { Look::Outline }, p);
            x += width as i32 + self.s(4.);
        }
        let top = bar_y + height as i32 + self.s(8.);
        if list.is_empty() {
            let message = if total == 0 { "No comments yet. Select some text and press Ctrl+Alt+M to leave a note for yourself or a reader." } else { "No comments match these filters." };
            self.label(message, Rect { x: rect.x + self.s(14.), y: top + self.s(6.), width: rect.width.saturating_sub(self.su(28.)), height: self.su(80.) }, 12. * self.scale, p.muted);
            return;
        }
        let card_height = self.s(164.);
        let capacity = ((rect.y + rect.height as i32 - top) / card_height).max(1) as usize;
        self.outline_top = self.outline_top.min(list.len().saturating_sub(capacity));
        let now = jw::time::now();
        for (n, c) in list.iter().enumerate().skip(self.outline_top).take(capacity) {
            let card = Rect { x: rect.x + self.s(8.), y: top + (n - self.outline_top) as i32 * card_height, width: rect.width.saturating_sub(self.su(16.)), height: card_height as u32 - self.su(8.) };
            let is_active = active.as_deref() == Some(c.id.as_str());
            let fill = if is_active { mix(p.surface, p.accent, 0.12) } else { p.raised };
            self.renderer.framed(card, self.su(7.), fill, if is_active { p.accent } else { mix(p.border, p.text, 0.1) });
            self.hits.push((card, Hit::Comment(c.id.clone())));
            let inner_x = card.x + self.s(10.);
            let inner_w = card.width.saturating_sub(self.su(20.));
            let quote: String = c.quote.replace('\n', " ");
            let quote = self.fit(&quote, inner_w as f32, 11.5);
            self.put(&quote, Rect { x: inner_x, y: card.y + self.s(6.), width: inner_w, height: self.su(20.) }, 11.5, p.muted, Face::Italic, Align::Left);
            self.label(&c.body, Rect { x: inner_x, y: card.y + self.s(28.), width: inner_w, height: self.su(58.) }, 12.5 * self.scale, if c.status == CommentStatus::Resolved { p.muted } else { p.text });
            let mut meta = String::new();
            if !c.tag.is_empty() {
                meta.push_str(&format!("{} · ", c.tag));
            }
            meta.push_str(titles.get(&c.item).map_or("(deleted chapter)", String::as_str));
            meta.push_str(&format!(" · {}", jw::time::ago(c.created, now)));
            if c.status == CommentStatus::Resolved {
                meta.push_str(" · resolved");
            }
            let meta = self.fit(&meta, inner_w as f32, 10.5);
            self.put(&meta, Rect { x: inner_x, y: card.y + self.s(88.), width: inner_w, height: self.su(18.) }, 10.5, p.muted, Face::Normal, Align::Left);
            if c.orphaned {
                self.put("The commented text was deleted", Rect { x: inner_x, y: card.y + self.s(104.), width: inner_w, height: self.su(18.) }, 10.5, p.warning_text(), Face::Normal, Align::Left);
            } else if let Some(reply) = c.replies.last() {
                let text = self.fit(&format!("↳ {} repl{}: {}", c.replies.len(), if c.replies.len() == 1 { "y" } else { "ies" }, reply.body.replace('\n', " ")), inner_w as f32, 10.5);
                self.put(&text, Rect { x: inner_x, y: card.y + self.s(104.), width: inner_w, height: self.su(18.) }, 10.5, p.muted, Face::Normal, Align::Left);
            }
            let by = card.y + card.height as i32 - self.s(30.);
            let mut bx = inner_x;
            let resolve = if c.status == CommentStatus::Open { ("Resolve", "resolve") } else { ("Reopen", "reopen") };
            for (label, key) in [resolve, ("Reply", "reply"), ("Edit", "edit"), ("Delete", "delete")] {
                let width = self.measure(label, 11. * self.scale, Face::Normal).ceil() as u32 + self.su(14.);
                if bx + width as i32 > card.x + card.width as i32 - self.s(4.) {
                    break;
                }
                self.button(label, Rect { x: bx, y: by, width, height: self.su(24.) }, Hit::Do(Act::CommentDo(key, c.id.clone())), Look::Plain, p);
                bx += width as i32 + self.s(2.);
            }
        }
        self.scroll_thumb(Rect { y: top, height: (capacity as i32 * card_height) as u32, ..rect }, self.outline_top, capacity, list.len(), p);
    }
    pub(super) fn paint_footer(&mut self, rect: Rect, p: Palette) {
        self.renderer.fill(rect, p.surface);
        self.renderer.fill(Rect { height: 1, ..rect }, p.border);
        let height = rect.height.saturating_sub(self.su(6.));
        let y = rect.y + self.s(3.);
        let mut x = rect.x + rect.width as i32 - self.s(6.);
        let buttons = [
            ("Settings", Command::Settings, false),
            ("Full screen", Command::Focus, false),
            ("Split", Command::SplitRight, false),
            // Lit only while the panel is on screen: a narrow window hides both.
            ("Panel", Command::TogglePanel, self.outline_rect.is_some()),
            ("Chapters", Command::ToggleTree, self.tree_rect.is_some()),
        ];
        for (text, command, on) in buttons {
            // Without a project, or behind a full-window view, there is no tree, panel or split to toggle.
            if (self.project.is_none() || self.screen.is_some()) && matches!(command, Command::ToggleTree | Command::TogglePanel | Command::SplitRight) {
                continue;
            }
            let width = self.measure(text, 12. * self.scale, Face::Normal).ceil() as u32 + self.su(18.);
            x -= width as i32;
            self.button(text, Rect { x, y, width, height }, Hit::Command(command), if on { Look::Active } else { Look::Plain }, p);
            x -= self.s(2.);
        }
        let active = self.active_id().and_then(|id| self.tabs.get(&id));
        let mut stats = String::new();
        if let Some(state) = &self.project {
            let goals = &state.core.manifest.settings.goals;
            let chapter = self.active_item().and_then(|id| state.core.manifest.tree.find(&id).map(|n| n.words()));
            if let Some(words) = chapter {
                stats.push_str(&format!("This chapter {}", count_words(words as u64)));
            }
            let written = format!("Session +{}", thousands(state.core.session_words()));
            if !stats.is_empty() {
                stats.push_str("  ·  ");
            }
            stats.push_str(&written);
            if goals.session_words > 0 {
                stats.push_str(&format!(" of {}", thousands(goals.session_words as u64)));
            }
        }
        let busy = active.map_or("", |t| {
            if t.loading {
                "Loading…"
            } else if t.saving {
                "Saving…"
            } else if t.document.dirty() && self.settings.autosave {
                "Unsaved · autosaves shortly"
            } else if t.document.dirty() {
                "Unsaved changes"
            } else {
                ""
            }
        });
        let mut right = x - self.s(12.);
        if !stats.is_empty() {
            let width = self.measure(&stats, 11.5 * self.scale, Face::Normal).ceil() as i32;
            right -= width;
            self.put(&stats, Rect { x: right, y: rect.y, width: width as u32 + 4, height: rect.height }, 11.5, p.muted, Face::Normal, Align::Left);
            right -= self.s(16.);
        }
        let message = if let Some((_, tip)) = self.tip_at_mouse() {
            tip
        } else if !self.status.is_empty() {
            self.status.clone()
        } else {
            busy.into()
        };
        let area = Rect { x: rect.x + self.s(14.), y: rect.y, width: (right - rect.x - self.s(20.)).max(10) as u32, height: rect.height };
        let tint = if message == self.status_error { p.danger_text() } else { p.muted };
        let message = self.fit(&message, area.width as f32, 11.5);
        self.put(&message, area, 11.5, tint, Face::Normal, Align::Left);
    }
    pub(super) fn paint_welcome(&mut self, rect: Rect, p: Palette) {
        let width = rect.width.saturating_sub(self.su(48.)).min(self.su(560.));
        let x = rect.x + (rect.width.saturating_sub(width) / 2) as i32;
        let mut y = rect.y + (rect.height as i32 / 5).max(self.s(40.));
        self.put("JustWrite", Rect { x, y, width, height: self.su(46.) }, 30., p.text, Face::Bold, Align::Left);
        y += self.s(52.);
        self.put("A quiet place for your manuscript.", Rect { x, y, width, height: self.su(26.) }, 14., p.muted, Face::Normal, Align::Left);
        y += self.s(52.);
        let height = self.su(38.);
        let mut bx = x;
        for (text, command, look) in [("New project…", Command::NewProject, Look::Primary), ("Open project…", Command::OpenProject, Look::Outline), ("Import a manuscript…", Command::Import, Look::Outline)] {
            let w = self.measure(text, 13. * self.scale, Face::Normal).ceil() as u32 + self.su(36.);
            self.button(text, Rect { x: bx, y, width: w, height }, Hit::Command(command), look, p);
            bx += w as i32 + self.s(10.);
        }
        y += self.s(52.);
        let shortcuts = [Command::NewProject, Command::OpenProject, Command::Palette]
            .iter()
            .filter_map(|c| menu::info(*c))
            .map(|info| format!("{} {}", self.shortcut_of(info), info.name.trim_end_matches('…').to_lowercase()))
            .collect::<Vec<_>>()
            .join("   ·   ");
        let shortcuts = self.fit(&shortcuts, width as f32, 12.);
        self.put(&shortcuts, Rect { x, y, width, height: self.su(22.) }, 12., p.muted, Face::Normal, Align::Left);
        y += self.s(40.);
        let recent: Vec<PathBuf> = self.recent.iter().filter(|r| r.is_file()).take(6).cloned().collect();
        if !recent.is_empty() {
            self.section("RECENT PROJECTS", x, y, width, p);
            y += self.s(26.);
            for (index, path) in recent.into_iter().enumerate() {
                let row = Rect { x, y, width, height: self.su(44.) };
                if self.hot(row) {
                    self.renderer.fill_round(row, self.su(6.), hovered(p.background, p));
                }
                let name = path.file_stem().map_or_else(String::new, |n| n.to_string_lossy().into_owned());
                self.put(&name, Rect { x: row.x + self.s(10.), y: row.y + self.s(2.), width: row.width - self.su(20.), height: self.su(24.) }, 14., p.text, Face::Normal, Align::Left);
                let place = self.fit(&path.to_string_lossy(), row.width as f32 - self.s(20.) as f32, 11.);
                self.put(&place, Rect { x: row.x + self.s(10.), y: row.y + self.s(22.), width: row.width - self.su(20.), height: self.su(20.) }, 11., p.muted, Face::Normal, Align::Left);
                self.hits.push((row, Hit::Do(Act::OpenRecent(index))));
                y += self.s(48.);
            }
        }
        let status = self.status.clone();
        if !status.is_empty() {
            self.label(&status, Rect { x, y: rect.y + rect.height as i32 - self.s(60.), width, height: self.su(40.) }, 12.5 * self.scale, p.muted);
        }
    }
    pub(super) fn scrim(&mut self, window: Rect, dismiss: bool, p: Palette) {
        self.renderer.shade(window, 0x000000, if p.light { 70 } else { 120 });
        self.hits.push((
            window,
            Hit::Do(if dismiss { Act::Dismiss } else { Act::Block }),
        ));
    }
    pub(super) fn dialog_box(&mut self, window: Rect, width: f32, height: u32, p: Palette) -> Rect {
        let width = self.su(width).min(window.width.saturating_sub(self.su(24.)));
        let height = height.min(window.height.saturating_sub(self.su(24.)));
        let rect = Rect {
            x: (window.width - width) as i32 / 2,
            y: ((window.height - height) as i32 / 2).max(self.s(12.)).min(self.s(120.)),
            width,
            height,
        };
        self.soft_shadow(rect);
        self.renderer.framed(rect, self.su(9.), p.raised, mix(p.border, p.text, 0.15));
        self.hits.push((rect, Hit::Do(Act::Block)));
        rect
    }
    /// A one-line input. `caret` is in chars (`None` is the end); `selected` shows the whole text
    /// as selected, which is what typing will replace.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn field(&mut self, rect: Rect, text: &str, placeholder: &str, caret: Option<usize>, focused: bool, selected: bool, hit: Act, p: Palette) {
        self.renderer.framed(rect, self.su(6.), p.background, if focused { p.accent_text() } else { mix(p.border, p.text, 0.12) });
        let inner = Rect { x: rect.x + self.s(12.), width: rect.width.saturating_sub(self.su(24.)), ..rect };
        let size = 14. * self.scale;
        let line = (size * 1.5).ceil() as u32;
        let row = Rect { y: inner.y + (inner.height as i32 - line as i32) / 2, height: line, ..inner };
        if text.is_empty() {
            self.put(placeholder, inner, 14., p.faint(p.background), Face::Normal, Align::Left);
        }
        let len = text.chars().count();
        let at = caret.unwrap_or(len).min(len);
        let mut buffer = self.area_buffer(text, None, size, p.text);
        // The caret is drawn over the text, so moving it never shifts the letters.
        let x = buffer.cursor_position(&area_cursor(text, at)).map_or(0., |(x, _)| x);
        // Long text slides left so the caret stays inside the box.
        let slide = (x - inner.width as f32 + self.s(2.) as f32).max(0.);
        let origin = inner.x as f32 - slide;
        if selected && focused && !text.is_empty() {
            let width = buffer.layout_runs().map(|run| run.line_w).fold(0., f32::max).ceil() as u32 + 2;
            self.renderer.fill(Rect { x: origin as i32 - 1, width, ..row }.intersect(inner), p.selection);
        }
        self.renderer.text(&mut self.fonts, &mut buffer, origin as i32, row.y, row);
        if focused && !selected {
            let bar = Rect { x: (origin + x) as i32, y: row.y + self.s(1.), width: self.su(1.5).max(1), height: line.saturating_sub(self.su(2.)) };
            self.renderer.fill(bar, p.accent_text());
        }
        self.probes.push(AreaProbe { rect, origin: (origin, row.y as f32), text: text.to_owned(), buffer });
        self.hits.push((rect, Hit::Do(hit)));
    }
    /// The char under the pointer in the text box it is over, as last painted.
    pub(super) fn probe_char(&self) -> Option<usize> {
        let probe = self.probes.iter().rev().find(|probe| contains(probe.rect, self.mouse.x, self.mouse.y))?;
        let bottom = probe.buffer.layout_runs().last().map_or(0., |run| run.line_top + run.line_height - 1.);
        let (x, y) = (self.mouse.x as f32 - probe.origin.0, (self.mouse.y as f32 - probe.origin.1).clamp(0., bottom));
        Some(probe.buffer.hit(x, y).map_or(probe.text.chars().count(), |cursor| area_char(&probe.text, &cursor)))
    }
    pub(super) fn paint_overlay(&mut self, window: Rect, top: i32, p: Palette) {
        let Some(overlay) = &self.overlay else {
            return;
        };
        if overlay.docked() {
            return;
        }
        let dialog = overlay.is_dialog();
        let searching = matches!(overlay.kind, Kind::Find | Kind::Replace);
        self.in_modal = true;
        if searching {
            // Keep the document undimmed so matches stay visible behind the search box.
            self.hits.push((window, Hit::Do(Act::Dismiss)));
        } else {
            self.scrim(window, !dialog, p);
        }
        if dialog {
            self.paint_dialog(window, p);
        } else {
            self.paint_list(window, top, p);
        }
        self.in_modal = false;
    }
    fn paint_list(&mut self, window: Rect, top: i32, p: Palette) {
        let Some(overlay) = &self.overlay else {
            return;
        };
        let (kind, title, rows_total) = (overlay.kind.clone(), overlay.title().to_owned(), overlay.rows.len());
        let (query, caret, case, all) = (overlay.query.clone(), overlay.caret, overlay.case, overlay.select_all);
        let placeholder = overlay.placeholder().to_owned();
        let pending = overlay.pending;
        let (selected, scroll) = (overlay.selected, overlay.scroll);
        let searching = kind == Kind::ProjectSearch;
        let keymap = matches!(kind, Kind::Keymap | Kind::Palette);
        let width = window.width.saturating_sub(self.su(40.)).min(self.su(720.));
        let x = (window.width - width) as i32 / 2;
        let y = top + self.s(40.);
        let (row_height, pad) = (self.su(if matches!(kind, Kind::Setting(_)) { 40. } else if keymap { 36. } else { 46. }), self.s(14.));
        let fixed = self.su(46.) + self.su(46.)
            + if searching { self.su(40.) } else { 0 }
            + self.su(14.);
        let room = window.height.saturating_sub(y as u32 + self.su(24.) + fixed) / row_height;
        let visible = (room as usize).clamp(1, 10);
        let shown = rows_total.min(visible);
        let height = fixed + shown.max(1) as u32 * row_height;
        let rect = Rect { x, y, width, height };
        self.soft_shadow(rect);
        self.renderer.framed(rect, self.su(9.), p.raised, mix(p.border, p.text, 0.15));
        self.hits.push((rect, Hit::Do(Act::Block)));
        let mut cursor = y;
        // Title row
        let title_row = Rect { x: x + pad, y: cursor, width: width - pad as u32 * 2, height: self.su(46.) };
        self.put(&title, title_row, 13., p.text, Face::Bold, Align::Left);
        let close = Rect { x: x + width as i32 - self.s(40.), y: cursor + self.s(8.), width: self.su(30.), height: self.su(30.) };
        self.button("×", close, Hit::Do(Act::Dismiss), Look::Plain, p);
        if matches!(kind, Kind::Keymap | Kind::Setting(_)) {
            let back = Rect { x: close.x - self.s(100.), width: self.su(94.), ..close };
            self.button("← Settings", back, Hit::Command(Command::Settings), Look::Plain, p);
        }
        cursor += self.s(46.);
        let input = Rect { x: x + pad, y: cursor, width: width - pad as u32 * 2, height: self.su(38.) };
        self.field(input, &query, &placeholder, caret, true, all, Act::Field(false), p);
        cursor += self.s(46.);
        if searching {
            let bar = Rect { x: x + pad, y: cursor, width: width - pad as u32 * 2, height: self.su(32.) };
            let count = if pending {
                "Searching…".to_string()
            } else if query.is_empty() {
                String::new()
            } else if rows_total >= 500 {
                "500+ matches".into()
            } else {
                format!("{rows_total} match{}", if rows_total == 1 { "" } else { "es" })
            };
            self.put(&count, bar, 12., p.muted, Face::Normal, Align::Left);
            let case_button = Rect { x: bar.x + bar.width as i32 - self.s(104.), y: bar.y, width: self.su(104.), height: bar.height };
            self.button("Match case", case_button, Hit::Do(Act::Case), if case { Look::Active } else { Look::Outline }, p);
            cursor += self.s(40.);
        }
        cursor += self.s(6.);
        self.renderer.fill(Rect { x: x + 1, y: cursor - self.s(4.), width: width - 2, height: 1 }, p.border);
        if rows_total == 0 {
            let message = if pending {
                "Searching…"
            } else if query.is_empty() && searching {
                "Type to search"
            } else {
                "No matches"
            };
            let area = Rect { x: x + pad, y: cursor, width: width - pad as u32 * 2, height: row_height };
            self.put(message, area, 12.5, p.muted, Face::Normal, Align::Left);
        }
        let scroll = scroll.min(rows_total.saturating_sub(visible));
        if let Some(o) = &mut self.overlay {
            o.visible = visible;
            o.scroll = scroll;
        }
        let rows: Vec<_> = self
            .overlay
            .as_ref()
            .map(|o| o.rows.iter().skip(scroll).take(visible).cloned().collect())
            .unwrap_or_default();
        for (index, row) in rows.iter().enumerate() {
            let line = Rect {
                x: x + self.s(8.),
                y: cursor + index as i32 * row_height as i32,
                width: width - self.su(16.),
                height: row_height,
            };
            let number = scroll + index;
            if number == selected {
                self.renderer.fill_round(line, self.su(5.), selected_fill(p.raised, p));
            } else if self.hot(line) {
                self.renderer.fill_round(line, self.su(5.), hovered(p.raised, p));
            }
            let inner = Rect { x: line.x + self.s(10.), width: line.width.saturating_sub(self.su(20.)), ..line };
            if let Kind::Setting(_) = kind {
                // Each font is drawn in itself, with a sample line to judge it by.
                self.preview_font = Some(row.label.clone());
                let name = self.fit(&row.label, inner.width as f32 * 0.5, 14.);
                self.put(&name, inner, 14., p.text, Face::Normal, Align::Left);
                let sample = Rect { x: inner.x + inner.width as i32 / 2, width: inner.width / 2, ..inner };
                self.put("The quick brown fox 0123", sample, 14., p.muted, Face::Normal, Align::Left);
                self.preview_font = None;
            } else if keymap {
                self.put(&row.label, inner, 13., p.text, Face::Normal, Align::Left);
                if keymap {
                    if !row.hint.is_empty() || kind == Kind::Keymap {
                        let text = if row.hint.is_empty() { "unassigned" } else { row.hint.as_str() };
                        let cap = self.measure(text, 12. * self.scale, Face::Normal).ceil() as u32 + self.su(18.);
                        let key = Rect { x: inner.x + inner.width as i32 - cap as i32, y: line.y + self.s(6.), width: cap, height: line.height.saturating_sub(self.su(12.)) };
                        self.renderer.framed(key, self.su(4.), p.background, p.border);
                        self.put(text, key, 12., if row.hint.is_empty() { p.muted } else { p.text }, Face::Normal, Align::Center);
                    }
                }
            } else {
                let upper = Rect { height: line.height * 6 / 10, ..inner };
                let lower = Rect { y: line.y + (line.height * 55 / 100) as i32, height: line.height * 4 / 10, ..inner };
                let label = self.fit(&row.label, inner.width as f32, 13.);
                if searching && !query.is_empty() {
                    // Mark the matched words so the eye lands on them.
                    let (hay, needle) = (if case { label.clone() } else { label.to_lowercase() }, if case { query.clone() } else { query.to_lowercase() });
                    if hay.len() == label.len() {
                        if let Some(at) = hay.find(&needle).filter(|at| label.is_char_boundary(*at) && label.is_char_boundary(at + needle.len())) {
                            let size = 13. * self.scale;
                            let before = self.measure(&label[..at], size, Face::Normal);
                            let width = self.measure(&label[at..at + needle.len()], size, Face::Normal);
                            let mark = Rect { x: inner.x + before as i32 - 1, y: line.y + self.s(4.), width: width.ceil() as u32 + 2, height: self.su(20.) };
                            self.renderer.fill_round(mark, self.su(3.), mix(p.raised, p.accent, 0.35));
                        }
                    }
                }
                self.put(&label, Rect { y: line.y + self.s(2.), ..upper }, 13., p.text, Face::Normal, Align::Left);
                let hint = self.fit(&row.hint, inner.width as f32, 11.);
                self.put(&hint, lower, 11., p.muted, Face::Normal, Align::Left);
            }
            self.hits.push((line, Hit::Row(number)));
        }
        self.scroll_thumb(Rect { x, y: cursor, width: width - self.su(2.), height: row_height * visible as u32 }, scroll, visible, rows_total, p);
    }
    pub(super) fn dialog_buttons(&mut self, rect: Rect, buttons: &[(&str, Act, Look)], p: Palette) {
        let height = self.su(34.);
        let y = rect.y + rect.height as i32 - height as i32 - self.s(18.);
        let mut x = rect.x + rect.width as i32 - self.s(20.);
        for (text, act, look) in buttons.iter().rev() {
            let width = self.measure(text, 13. * self.scale, Face::Normal).ceil() as u32 + self.su(32.);
            x -= width as i32;
            self.button(text, Rect { x, y, width, height }, Hit::Do(act.clone()), *look, p);
            x -= self.s(8.);
        }
    }
    fn paint_dialog(&mut self, window: Rect, p: Palette) {
        let Some(overlay) = &self.overlay else {
            return;
        };
        let kind = overlay.kind.clone();
        let (query, caret, note, title, all) = (overlay.query.clone(), overlay.caret, overlay.note.clone(), overlay.title().to_owned(), overlay.select_all);
        let dirty = self.tabs.values().filter(|t| t.document.dirty()).count();
        let name_of = |id: u64| self.tabs.get(&id).map(|t| t.name()).unwrap_or_default();
        let title_of = |id: &str| self.node_title(id).unwrap_or_default();
        let (message, input, buttons): (String, bool, Vec<(&str, Act, Look)>) = match &kind {
            Kind::Close(id) => (
                format!("Do you want to save the changes you made to {}?\nYour changes will be lost if you don't save them.", name_of(*id)),
                false,
                vec![("Don't save", Act::Press("discard"), Look::Outline), ("Cancel", Act::Press("escape"), Look::Outline), ("Save", Act::Press("save"), Look::Primary)],
            ),
            Kind::Quit => (
                format!("{dirty} file{} ha{} unsaved changes.\nSave before exiting?", if dirty == 1 { "" } else { "s" }, if dirty == 1 { "s" } else { "ve" }),
                false,
                vec![("Exit without saving", Act::Press("discard"), Look::Outline), ("Cancel", Act::Press("escape"), Look::Outline), ("Save all & exit", Act::Press("save"), Look::Primary)],
            ),
            Kind::ChangeWorkspace(_) => (
                format!("{dirty} open chapter{} ha{} unsaved changes.\nSave before opening another project?", if dirty == 1 { "" } else { "s" }, if dirty == 1 { "s" } else { "ve" }),
                false,
                vec![("Discard & open", Act::Press("discard"), Look::Outline), ("Cancel", Act::Press("escape"), Look::Outline), ("Save all & open", Act::Press("save"), Look::Primary)],
            ),
            Kind::Delete(id) => (
                format!("Delete \"{}\"?\nA snapshot is saved first, so the text can be brought back from History.", title_of(id)),
                false,
                vec![("Cancel", Act::Press("escape"), Look::Outline), ("Delete", Act::Press("enter"), Look::Danger)],
            ),
            Kind::GotoLine => (
                "Enter a line number.".into(),
                true,
                vec![("Cancel", Act::Press("escape"), Look::Outline), ("Go", Act::Press("enter"), Look::Primary)],
            ),
            Kind::Rename(id) => (
                format!("Rename \"{}\".", title_of(id)),
                true,
                vec![("Cancel", Act::Press("escape"), Look::Outline), ("Rename", Act::Press("enter"), Look::Primary)],
            ),
            Kind::Shortcut(key) => {
                let name = COMMANDS.iter().find(|c| c.key == key).map_or("", |c| c.name);
                (
                    format!("Press the key combination you want for\n\"{name}\"."),
                    false,
                    vec![("Clear", Act::ClearShortcut, Look::Outline), ("Reset to default", Act::ResetShortcut, Look::Outline), ("Cancel", Act::Press("escape"), Look::Outline)],
                )
            }
            _ => return,
        };
        let lines = message.lines().count() as u32;
        let height = self.su(46.) + lines * self.su(22.) + self.su(20.)
            + if input { self.su(52.) } else { 0 }
            + if kind_is_shortcut(&kind) { self.su(64.) } else { 0 }
            + if note.is_empty() { 0 } else { self.su(24.) }
            + self.su(70.);
        let rect = self.dialog_box(window, 480., height, p);
        let pad = self.s(22.);
        let mut y = rect.y + self.s(14.);
        let inner = Rect { x: rect.x + pad, y, width: rect.width - pad as u32 * 2, height: self.su(30.) };
        self.put(&title, inner, 15., p.text, Face::Bold, Align::Left);
        y += self.s(40.);
        for (n, line) in message.lines().enumerate() {
            // The question is the point of the dialog; what follows from it is secondary.
            self.put(line, Rect { y, height: self.su(22.), ..inner }, 12.5, if n == 0 { p.text } else { p.muted }, Face::Normal, Align::Left);
            y += self.s(22.);
        }
        y += self.s(10.);
        if input {
            let area = Rect { y, height: self.su(38.), ..inner };
            self.field(area, &query, self.overlay.as_ref().map_or("", |o| o.placeholder()).to_owned().as_str(), caret, true, all, Act::Field(false), p);
            y += self.s(46.);
        }
        if kind_is_shortcut(&kind) {
            let shown = if query.is_empty() { "No shortcut".to_owned() } else { commands::display_shortcut(&query) };
            let cap = Rect { x: inner.x, y, width: inner.width, height: self.su(48.) };
            self.renderer.framed(cap, self.su(7.), p.background, p.accent_text());
            self.put(&shown, cap, 18., p.text, Face::Bold, Align::Center);
            y += self.s(56.);
        }
        if !note.is_empty() {
            self.put(&note, Rect { y, height: self.su(22.), ..inner }, 12., p.danger_text(), Face::Normal, Align::Left);
        }
        self.dialog_buttons(rect, &buttons, p);
    }
    pub(super) fn section(&mut self, text: &str, x: i32, y: i32, width: u32, p: Palette) {
        let area = Rect { x, y, width, height: self.su(22.) };
        self.put(text, area, 10.5, mix(p.muted, p.accent_text(), 0.5), Face::Bold, Align::Left);
    }
    /// Two or more options in one rounded track; the chosen one is highlighted.
    pub(super) fn segmented(&mut self, rect: Rect, options: &[(&str, bool, Act)], p: Palette) {
        self.renderer
            .framed(rect, self.su(7.), p.background, mix(p.border, p.text, 0.14));
        let each = (rect.width - self.su(4.)) / options.len() as u32;
        for (index, (text, on, act)) in options.iter().enumerate() {
            let cell = Rect {
                x: rect.x + self.s(2.) + index as i32 * each as i32,
                y: rect.y + self.s(2.),
                width: each,
                height: rect.height - self.su(4.),
            };
            if *on {
                self.renderer
                    .fill_round(cell, self.su(5.), mix(p.raised, p.accent, 0.4));
            } else if self.hot(cell) {
                self.renderer.fill_round(cell, self.su(5.), hovered(p.background, p));
            }
            let face = if *on { Face::Bold } else { Face::Normal };
            self.put(text, cell, 13., if *on { p.text } else { p.muted }, face, Align::Center);
            self.hits.push((cell, Hit::Do(act.clone())));
        }
    }
    pub(super) fn icon_doc(&mut self, x: i32, y: i32, color: u32, background: u32) {
        let rect = Rect { x, y, width: self.su(11.), height: self.su(14.) };
        self.renderer.framed(rect, self.su(2.), background, color);
        for line in 0..3 {
            let bar = Rect { x: x + self.s(3.), y: y + self.s(4.) + line * self.s(3.), width: self.su(5.), height: 1 };
            self.renderer.fill(bar, color);
        }
    }
    pub(super) fn dividers_for(&mut self, center: Rect) {
        self.dividers.clear();
        let mut found: Vec<Divider> = Vec::new();
        self.split.dividers(center, &mut Vec::new(), &mut found);
        self.dividers = found;
    }
}

/// An input box as last painted, kept so a click can be turned into a caret position.
pub(super) struct AreaProbe {
    /// The whole box; a click anywhere in it counts.
    pub rect: Rect,
    /// Where the text starts on screen, scrolling included.
    pub origin: (f32, f32),
    pub text: String,
    pub buffer: Buffer,
}

/// The layout cursor for the char at `at` in `text` (lines split at `\n`).
pub(super) fn area_cursor(text: &str, at: usize) -> Cursor {
    let before: String = text.chars().take(at).collect();
    let line = before.matches('\n').count();
    Cursor::new(line, before.len() - before.rfind('\n').map_or(0, |i| i + 1))
}

/// The char offset in `text` that the layout cursor stands for.
pub(super) fn area_char(text: &str, cursor: &Cursor) -> usize {
    let mut start = 0;
    for _ in 0..cursor.line {
        start += text[start..].find('\n').map_or(text.len() - start, |i| i + 1);
    }
    let end = (start + cursor.index).min(text.len());
    text[..end].chars().count()
}

fn kind_is_shortcut(kind: &Kind) -> bool {
    matches!(kind, Kind::Shortcut(_))
}
fn face_attrs(font: &str, tint: u32, face: Face) -> Attrs<'_> {
    let attrs = Attrs::new().family(Family::Name(font)).color(color(tint));
    match face {
        Face::Bold => attrs.weight(Weight::BOLD),
        Face::Italic => attrs.style(FontStyle::Italic),
        Face::Normal => attrs,
    }
}

/// `1 word`, `12,345 words`
pub(super) fn count_words(n: u64) -> String {
    format!("{} word{}", thousands(n), if n == 1 { "" } else { "s" })
}
/// `12,345`
pub(super) fn thousands(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}
