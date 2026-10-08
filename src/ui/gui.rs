//! Mouse-first chrome: menu bar, popups, tabs, toolbar, panels, dialogs and the settings panel.
//! Everything here only paints and registers hit rectangles; clicks are handled in `app.rs`.
use super::*;
use crate::ui::{
    layout::Divider,
    menu::{self, EDITOR_MENU, MENUS, Popup, TAB_MENU, TREE_MENU},
};
use crate::core::markdown::Format;
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

fn mix(a: u32, b: u32, t: f32) -> u32 {
    let channel = |shift: u32| {
        let (a, b) = (((a >> shift) & 255) as f32, ((b >> shift) & 255) as f32);
        (a * (1. - t) + b * t).round() as u32
    };
    channel(16) << 16 | channel(8) << 8 | channel(0)
}
/// Readable text colour on top of a solid fill.
fn on_color(fill: u32) -> u32 {
    let luma = 0.299 * ((fill >> 16) & 255) as f32
        + 0.587 * ((fill >> 8) & 255) as f32
        + 0.114 * (fill & 255) as f32;
    if luma > 150. { 0x1d1a17 } else { 0xffffff }
}

/// Selected-row fill: the raised surface washed with the accent.
fn selected_fill(base: u32, p: Palette) -> u32 {
    mix(base, p.accent, if p.light { 0.3 } else { 0.24 })
}
fn hovered(base: u32, p: Palette) -> u32 {
    mix(base, p.text, if p.light { 0.07 } else { 0.09 })
}

impl App {
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
            Command::ToggleOutline => self.show_outline,
            Command::ToggleMarkers => self.settings.show_markers,
            Command::Focus => self.window.as_ref().is_some_and(|w| w.fullscreen().is_some()),
            _ => false,
        }
    }
    pub(super) fn enabled(&self, command: Command) -> bool {
        let tab = self.active_id().and_then(|id| self.tabs.get(&id));
        match command {
            Command::Reload => tab.is_some_and(|t| t.external.is_some()),
            Command::Reopen => !self.closed.is_empty(),
            Command::RenameFile | Command::DeleteFile => self.target_path().is_some(),
            Command::Cut | Command::Copy => tab.is_some_and(|t| !t.document.selection().is_empty()),
            Command::GotoLine | Command::WordCount => tab.is_some(),
            _ => true,
        }
    }
    pub(super) fn shortcut_of(&self, command: &commands::CommandInfo) -> String {
        self.settings
            .keymap
            .get(command.key)
            .map(String::as_str)
            .unwrap_or(command.shortcut)
            .replace("Ctrl", if cfg!(target_os = "macos") { "Cmd" } else { "Ctrl" })
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
            Hit::Do(Act::Heading) => return Some((*rect, "Heading level".into())),
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
            Some(Hit::Tree(..)) => TREE_MENU,
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
            let tint = if enabled { p.text } else { mix(p.muted, p.raised, 0.4) };
            if self.checked(*command) {
                let tick = Rect { x: line.x + self.s(6.), width: self.su(18.), ..line };
                self.put("✓", tick, 12., p.accent, Face::Normal, Align::Left);
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
        let width = (usable / count).clamp(self.su(100.), self.su(190.));
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
            let room = width.saturating_sub(self.su(52.)) as f32;
            let text = self.fit(&name, room, 12.5);
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
                self.put(glyph, close, 14., if dirty && glyph == "●" { p.accent } else { p.muted }, Face::Normal, Align::Center);
            }
            self.hits.push((close, Hit::Close(id)));
        }
        let plus = Rect {
            x: rect.x + shown * width as i32 + self.s(4.),
            y: rect.y + self.s(5.),
            width: self.su(26.),
            height: header.saturating_sub(self.su(10.)),
        };
        self.button("+", plus, Hit::Command(Command::New), Look::Plain, p);
    }
    fn paint_toolbar(&mut self, rect: Rect, format: Format, p: Palette) {
        let heading = if format.heading > 0 { format!("Heading {} ▾", format.heading) } else { "Heading ▾".to_owned() };
        let mut x = rect.x + self.s(14.);
        let height = rect.height.saturating_sub(self.su(10.));
        let y = rect.y + self.s(5.);
        let tools: [(&str, Face, Option<Hit>); 12] = [
            (heading.as_str(), Face::Normal, Some(Hit::Do(Act::Heading))),
            ("", Face::Normal, None),
            ("B", Face::Bold, Some(Hit::Command(Command::Bold))),
            ("I", Face::Italic, Some(Hit::Command(Command::Italic))),
            ("S", Face::Normal, Some(Hit::Command(Command::Strike))),
            ("", Face::Normal, None),
            ("• List", Face::Normal, Some(Hit::Command(Command::Bullet))),
            ("1. List", Face::Normal, Some(Hit::Command(Command::Numbered))),
            ("", Face::Normal, None),
            ("</>", Face::Normal, Some(Hit::Command(Command::CodeBlock))),
            ("Link", Face::Normal, Some(Hit::Command(Command::Link))),
            ("Find", Face::Normal, Some(Hit::Command(Command::Find))),
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
            let strike = matches!(&hit, Hit::Command(Command::Strike));
            let on = match &hit {
                Hit::Do(Act::Heading) => format.heading > 0,
                Hit::Command(Command::Bold) => format.bold,
                Hit::Command(Command::Italic) => format.italic,
                Hit::Command(Command::Strike) => format.strike,
                Hit::Command(Command::Bullet) => format.bullet,
                Hit::Command(Command::Numbered) => format.numbered,
                Hit::Command(Command::CodeBlock) => format.code,
                _ => false,
            };
            let look = if on { Look::Active } else { Look::Plain };
            self.button_face(text, Rect { x, y, width, height }, hit, look, face, Align::Center, p);
            if strike {
                let glyph = self.measure(text, 13. * self.scale, face).ceil() as u32;
                let line = Rect { x: x + (width as i32 - glyph as i32) / 2 - 1, y: y + height as i32 / 2, width: glyph + 2, height: 1 };
                self.renderer.fill(line, p.muted);
            }
            x += width as i32 + self.s(2.);
        }
    }
    fn paint_banner(&mut self, rect: Rect, name: &str, p: Palette) {
        self.renderer.fill(rect, mix(p.accent, p.background, 0.75));
        let button = self.su(150.);
        let text = Rect { x: rect.x + self.s(14.), width: rect.width.saturating_sub(button * 2 + self.su(40.)), ..rect };
        self.put(
            &format!("{name} was changed outside this editor."),
            text,
            12.5,
            p.text,
            Face::Normal,
            Align::Left,
        );
        let height = rect.height.saturating_sub(self.su(10.));
        let y = rect.y + self.s(5.);
        let right = rect.x + rect.width as i32 - self.s(10.);
        self.button(
            "Load disk version",
            Rect { x: right - button as i32, y, width: button, height },
            Hit::Command(Command::Reload),
            Look::Outline,
            p,
        );
        self.button(
            "Keep my edits",
            Rect { x: right - button as i32 * 2 - self.s(8.), y, width: button, height },
            Hit::Command(Command::Save),
            Look::Primary,
            p,
        );
    }
    pub(super) fn paint_pane(&mut self, pane_id: u64, rect: Rect, focus: bool, p: Palette) {
        let Some(pane) = self.split.pane(pane_id).cloned() else {
            return;
        };
        let header = if focus { 0 } else { self.su(36.) };
        let toolbar = if focus { 0 } else { self.su(38.) };
        let external = pane
            .tab()
            .and_then(|id| self.tabs.get(&id))
            .filter(|t| t.external.is_some())
            .map(|t| t.name());
        let banner = if focus || external.is_none() { 0 } else { self.su(38.) };
        if !focus {
            self.paint_tabs(&pane, pane_id, rect, header, p);
            let format = pane
                .tab()
                .and_then(|id| self.tabs.get(&id))
                .map(|t| {
                    let (line, column) = Self::line_around(&t.document.text, t.document.cursor);
                    markdown::format_at(&line, column)
                })
                .unwrap_or_default();
            self.paint_toolbar(
                Rect { y: rect.y + header as i32, height: toolbar, ..rect },
                format,
                p,
            );
            self.renderer.fill(
                Rect { y: rect.y + (header + toolbar) as i32, height: 1, ..rect },
                p.border,
            );
            if let Some(name) = external {
                self.paint_banner(
                    Rect { y: rect.y + (header + toolbar) as i32 + 1, height: banner, ..rect },
                    &name,
                    p,
                );
            }
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
            let bar_top = rect.y + (header + toolbar + banner) as i32 + i32::from(!focus);
            self.paint_findbar(Rect { y: bar_top, height: find_height, ..rect }, p);
        }
        let padding = self.su(27.);
        let top = header + toolbar + banner + find_height;
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
        let highlights: Vec<(usize, usize)> = if find.is_some() {
            self.overlay
                .as_ref()
                .filter(|o| !o.query.is_empty())
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
        let tab = self.tabs.get_mut(&id).unwrap();
        tab.view.highlights = highlights;
        tab.view.layout(&tab.document, &tab.blocks, &settings, &mut self.fonts, editor_rect);
        tab.view.paint(&tab.document, &mut self.fonts, &mut self.renderer);
        let empty = tab.document.text.len_chars() == 0 && !tab.loading;
        let untitled = tab.path.is_none();
        let (len_chars, len_lines, top_char) =
            (tab.document.text.len_chars(), tab.document.text.len_lines(), tab.view.top);
        if empty && !focus {
            let hint = Rect {
                x: editor_rect.x + self.s(10.),
                y: editor_rect.y,
                width: editor_rect.width,
                height: (settings.font_size * 1.65) as u32,
            };
            self.put("Start writing…", hint, 16., mix(p.muted, p.background, 0.35), Face::Normal, Align::Left);
            if untitled && self.tree_rect.is_none() {
                let (height, gap) = (self.su(34.), self.s(10.));
                let mut x = editor_rect.x;
                let y = editor_rect.y + self.s(64.);
                let mut buttons = vec![
                    ("Open folder…", Command::Open, Look::Primary),
                    ("Open file…", Command::OpenFile, Look::Outline),
                ];
                if self.root.is_some() {
                    buttons.push(("New note in workspace…", Command::CreateFile, Look::Outline));
                }
                for (text, command, look) in buttons {
                    let width = self.measure(text, 13. * self.scale, Face::Normal).ceil() as u32 + self.su(32.);
                    self.button(text, Rect { x, y, width, height }, Hit::Command(command), look, p);
                    x += width as i32 + gap;
                }
            }
        }
        if !focus {
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
    }
    fn paint_findbar(&mut self, rect: Rect, p: Palette) {
        let Some(overlay) = &self.overlay else {
            return;
        };
        let (replace, query, replacement, secondary, caret, case, focused) = (
            overlay.kind == Kind::Replace,
            overlay.query.clone(),
            overlay.replacement.clone(),
            overlay.secondary,
            overlay.caret,
            overlay.case,
            overlay.focused,
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
        self.put(&text, counter, 12., if found == 0 && !query.is_empty() { 0xd0605e } else { p.muted }, Face::Normal, Align::Right);
        let width = (counter.x - rect.x - pad - self.s(8.)).max(80) as u32;
        let first = Rect { x: rect.x + pad, y, width, height };
        self.field(first, &query, "Find…", if secondary { None } else { caret }, focused && !secondary, Act::Field(false), p);
        if replace {
            let y2 = y + self.s(40.);
            let all = Rect { x: close.x + close.width as i32 - self.s(104.), y: y2, width: self.su(104.), height };
            self.button("Replace all", all, Hit::Do(Act::Replace(true)), Look::Outline, p);
            let one = Rect { x: all.x - self.s(88.), y: y2, width: self.su(80.), height };
            self.button("Replace", one, Hit::Do(Act::Replace(false)), Look::Primary, p);
            let width = (one.x - rect.x - pad - self.s(8.)).max(80) as u32;
            let second = Rect { x: rect.x + pad, y: y2, width, height };
            self.field(second, &replacement, "Replace with…", if secondary { caret } else { None }, focused && secondary, Act::Field(true), p);
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

    fn tree_count(&self, folder: &Path) -> usize {
        self.tree.get(folder).map_or(0, |entries| {
            entries
                .iter()
                .map(|e| {
                    1 + if e.directory && self.expanded.contains(&e.path) {
                        self.tree_count(&e.path)
                    } else {
                        0
                    }
                })
                .sum()
        })
    }
    pub(super) fn paint_tree(&mut self, rect: Rect, p: Palette) {
        self.renderer.fill(rect, p.surface);
        self.renderer.fill(
            Rect { x: rect.x + rect.width as i32 - 1, width: 1, ..rect },
            p.border,
        );
        let inset = self.s(10.);
        let head = Rect { x: rect.x + inset, y: rect.y + self.s(8.), width: rect.width.saturating_sub(inset as u32 * 2), height: self.su(28.) };
        self.put("FOLDER", head, 11., p.muted, Face::Bold, Align::Left);
        if self.root.is_none() {
            let width = rect.width.saturating_sub(self.su(32.));
            let x = rect.x + self.s(16.);
            let mut y = rect.y + self.s(60.);
            self.put(
                "No folder open",
                Rect { x, y, width, height: self.su(24.) },
                13.,
                p.muted,
                Face::Normal,
                Align::Left,
            );
            y += self.s(36.);
            for (text, command, look) in [
                ("Open folder…", Command::Open, Look::Primary),
                ("Open file…", Command::OpenFile, Look::Outline),
                ("Open recent…", Command::Recent, Look::Plain),
            ] {
                if command == Command::Recent && self.recent.is_empty() {
                    continue;
                }
                self.button(text, Rect { x, y, width, height: self.su(32.) }, Hit::Command(command), look, p);
                y += self.s(40.);
            }
        } else {
            let small = self.su(24.);
            let folder = Rect { x: rect.x + rect.width as i32 - self.s(84.), y: head.y + self.s(2.), width: self.su(74.), height: small };
            let note = Rect { x: folder.x - self.s(60.), width: self.su(56.), ..folder };
            self.button("+ Note", note, Hit::Command(Command::CreateFile), Look::Plain, p);
            self.button("+ Folder", folder, Hit::Command(Command::CreateFolder), Look::Plain, p);
            let name = self
                .root
                .as_ref()
                .and_then(|p| p.file_name())
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default();
            let root = Rect { x: rect.x + self.s(6.), y: rect.y + self.s(42.), width: rect.width.saturating_sub(self.su(12.)), height: self.su(28.) };
            let label = self.fit(&format!("▾  {name}"), root.width as f32 - self.s(24.) as f32, 13.);
            self.button_face(&label, root, Hit::Command(Command::Open), Look::Plain, Face::Bold, Align::Left, p);
            let row_height = self.su(28.);
            let list_top = rect.y + self.s(76.);
            let footer = if self.tree_selected.is_some() { self.su(44.) } else { 0 };
            let capacity = (rect.height.saturating_sub(self.su(76.) + footer) / row_height) as usize;
            let total = self.tree_count(Path::new("."));
            self.tree_top = self.tree_top.min(total.saturating_sub(capacity));
            let mut rows = Vec::new();
            let mut seen = 0;
            self.tree_rows(Path::new("."), 0, &mut seen, &mut rows, capacity.max(1));
            if total == 0 {
                let empty = Rect { x: rect.x + self.s(16.), y: list_top + self.s(6.), width: rect.width.saturating_sub(self.su(32.)), height: self.su(24.) };
                self.put("No Markdown files yet", empty, 12., p.muted, Face::Normal, Align::Left);
            }
            for (index, (entry, depth)) in rows.into_iter().enumerate() {
                let row = Rect {
                    x: rect.x + self.s(6.),
                    y: list_top + index as i32 * row_height as i32,
                    width: rect.width.saturating_sub(self.su(12.)),
                    height: row_height,
                };
                if self.tree_selected.as_ref() == Some(&entry.path) {
                    self.renderer.fill_round(row, self.su(5.), selected_fill(p.surface, p));
                } else if self.hot(row) {
                    self.renderer.fill_round(row, self.su(5.), hovered(p.surface, p));
                }
                let name = entry.path.file_name().unwrap_or_default().to_string_lossy().into_owned();
                let indent = self.s(8.) + depth as i32 * self.s(14.);
                if entry.directory {
                    let chevron = if self.expanded.contains(&entry.path) { "▾" } else { "▸" };
                    let area = Rect { x: row.x + indent, width: self.su(14.), ..row };
                    self.put(chevron, area, 10., p.muted, Face::Normal, Align::Center);
                }
                let icon_x = row.x + indent + self.s(18.);
                let icon_y = row.y + (row.height as i32 - self.s(14.)) / 2;
                if entry.directory {
                    self.icon_folder(icon_x, icon_y, mix(p.muted, p.accent, 0.55));
                } else {
                    self.icon_doc(icon_x, icon_y, p.muted, p.surface);
                }
                let text_x = icon_x + self.s(22.);
                let room = (row.x + row.width as i32 - text_x - self.s(6.)).max(10);
                let text = self.fit(&name, room as f32, 13.);
                self.put(&text, Rect { x: text_x, width: room as u32, ..row }, 13., p.text, Face::Normal, Align::Left);
                self.hits.push((row, Hit::Tree(entry.path, entry.directory)));
            }
            if footer > 0 {
                let y = rect.y + rect.height as i32 - self.s(36.);
                let height = self.su(28.);
                self.renderer.fill(Rect { y: y - self.s(6.), height: 1, ..rect }, p.border);
                self.button("Rename", Rect { x: rect.x + self.s(10.), y, width: self.su(80.), height }, Hit::Command(Command::RenameFile), Look::Outline, p);
                self.button("Delete", Rect { x: rect.x + self.s(98.), y, width: self.su(80.), height }, Hit::Command(Command::DeleteFile), Look::Outline, p);
            }
        }
        self.hits.push((
            Rect { x: rect.x + rect.width as i32 - self.s(3.), width: self.su(6.), ..rect },
            Hit::TreeResize,
        ));
    }
    pub(super) fn scroll_outline(&mut self, rows: i32) {
        let Some(tab) = self.active_id().and_then(|id| self.tabs.get(&id)) else {
            return;
        };
        let headings = &tab.blocks.headings;
        for _ in 0..rows.abs() {
            let next = if rows > 0 {
                headings.range(self.outline_top + 1..).next().map(|(l, _)| *l)
            } else {
                headings.range(..self.outline_top).next_back().map(|(l, _)| *l)
            };
            if let Some(line) = next {
                self.outline_top = line;
            }
        }
    }
    pub(super) fn paint_outline(&mut self, rect: Rect, p: Palette) {
        self.renderer.fill(rect, p.surface);
        self.renderer.fill(Rect { width: 1, ..rect }, p.border);
        let head = Rect { x: rect.x + self.s(16.), y: rect.y + self.s(8.), width: rect.width.saturating_sub(self.su(30.)), height: self.su(28.) };
        self.put("OUTLINE", head, 11., p.muted, Face::Bold, Align::Left);
        let Some(id) = self.active_id() else {
            return;
        };
        let tab = &self.tabs[&id];
        if tab.blocks.headings.is_empty() {
            let note = Rect { x: rect.x + self.s(16.), y: rect.y + self.s(46.), width: rect.width.saturating_sub(self.su(30.)), height: self.su(40.) };
            self.label("Headings appear here. Start a line with # to make one.", note, 11.5 * self.scale, p.muted);
            return;
        }
        let current = tab.document.text.char_to_line(tab.document.cursor);
        let active_heading = tab.blocks.headings.range(..=current).next_back().map(|(line, _)| *line);
        let row_height = self.s(28.);
        let mut rows = Vec::new();
        let mut from = self.outline_top;
        while (rows.len() as i32) < (rect.height as i32 - self.s(50.)) / row_height {
            let Some((&line, (level, name))) = tab.blocks.headings.range(from..).next() else {
                break;
            };
            rows.push((line, *level, name.clone()));
            if self.folded.contains(&line) {
                let next = tab.blocks.levels[..*level as usize]
                    .iter()
                    .filter_map(|level| level.range(line + 1..).next().map(|(&line, _)| line))
                    .min();
                if let Some(next) = next {
                    from = next;
                } else {
                    break;
                }
            } else {
                from = line + 1;
            }
        }
        for (index, (line, level, name)) in rows.into_iter().enumerate() {
            let row = Rect {
                x: rect.x + self.s(7.),
                y: rect.y + self.s(42.) + index as i32 * row_height,
                width: rect.width.saturating_sub(self.su(14.)),
                height: row_height as u32 - 1,
            };
            if active_heading == Some(line) {
                self.renderer.fill_round(row, self.su(5.), selected_fill(p.surface, p));
            } else if self.hot(row) {
                self.renderer.fill_round(row, self.su(5.), hovered(p.surface, p));
            }
            let offset = (level as i32 - 1) * self.s(10.);
            let arrow = Rect { x: row.x + self.s(2.) + offset, width: self.su(18.), ..row };
            self.put(if self.folded.contains(&line) { "▸" } else { "▾" }, arrow, 11., p.muted, Face::Normal, Align::Center);
            let text_x = row.x + self.s(22.) + offset;
            let width = (row.x + row.width as i32 - text_x).max(10);
            let text = self.fit(&name, width as f32, 12.5);
            self.put(
                &text,
                Rect { x: text_x, width: width as u32, ..row },
                12.5,
                if active_heading == Some(line) { p.text } else { p.muted },
                if active_heading == Some(line) { Face::Bold } else { Face::Normal },
                Align::Left,
            );
            self.hits.push((row, Hit::Heading(line, false)));
            self.hits.push((arrow, Hit::Heading(line, true)));
        }
        self.hits.push((
            Rect { x: rect.x - self.s(3.), width: self.su(6.), ..rect },
            Hit::OutlineResize,
        ));
    }

    pub(super) fn paint_footer(&mut self, rect: Rect, p: Palette) {
        self.renderer.fill(rect, p.surface);
        self.renderer.fill(Rect { height: 1, ..rect }, p.border);
        let height = rect.height.saturating_sub(self.su(6.));
        let y = rect.y + self.s(3.);
        let mut x = rect.x + rect.width as i32 - self.s(6.);
        let buttons = [
            ("Settings", Command::Settings, false),
            ("Focus", Command::Focus, false),
            ("Split", Command::SplitRight, false),
            ("Outline", Command::ToggleOutline, self.show_outline),
            ("Files", Command::ToggleTree, self.show_tree),
        ];
        for (text, command, on) in buttons {
            let width = self.measure(text, 12. * self.scale, Face::Normal).ceil() as u32 + self.su(18.);
            x -= width as i32;
            self.button(text, Rect { x, y, width, height }, Hit::Command(command), if on { Look::Active } else { Look::Plain }, p);
            x -= self.s(2.);
        }
        let active = self.active_id().and_then(|id| self.tabs.get(&id));
        let place = active.map(|tab| {
            let line = tab.document.text.char_to_line(tab.document.cursor);
            let column = tab.document.cursor - tab.document.text.line_to_char(line) + 1;
            format!(
                "Ln {}, Col {}  ·  {}",
                line + 1,
                column,
                if tab.document.line_ending == "\r\n" { "CRLF" } else { "LF" }
            )
        });
        let busy = active.map_or("", |t| {
            if t.loading {
                "Loading…"
            } else if t.saving {
                "Saving…"
            } else if t.document.dirty() && t.path.is_none() {
                "Not saved yet · File ▸ Save to choose a name"
            } else if t.document.dirty() && self.settings.autosave {
                "Unsaved · autosaves shortly"
            } else if t.document.dirty() {
                "Unsaved changes"
            } else {
                ""
            }
        });
        let mut right = x - self.s(12.);
        if let Some(place) = place {
            let width = self.measure(&place, 11.5 * self.scale, Face::Normal).ceil() as i32;
            right -= width;
            self.put(&place, Rect { x: right, y: rect.y, width: width as u32 + 4, height: rect.height }, 11.5, p.muted, Face::Normal, Align::Left);
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
        self.put(&message, area, 11.5, p.muted, Face::Normal, Align::Left);
    }

    // ---- overlays -------------------------------------------------------------------------

    fn scrim(&mut self, window: Rect, dismiss: bool, p: Palette) {
        self.renderer.shade(window, 0x000000, if p.light { 70 } else { 120 });
        self.hits.push((
            window,
            Hit::Do(if dismiss { Act::Dismiss } else { Act::Block }),
        ));
    }
    fn dialog_box(&mut self, window: Rect, width: f32, height: u32, p: Palette) -> Rect {
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
    fn field(&mut self, rect: Rect, text: &str, placeholder: &str, caret: Option<usize>, focused: bool, hit: Act, p: Palette) {
        self.renderer.framed(rect, self.su(6.), p.background, if focused { p.accent } else { mix(p.border, p.text, 0.12) });
        let inner = Rect { x: rect.x + self.s(12.), width: rect.width.saturating_sub(self.su(24.)), ..rect };
        if text.is_empty() && !(focused && caret.is_some()) {
            if focused {
                self.put("│", inner, 15., p.accent, Face::Normal, Align::Left);
            }
            let hint = Rect { x: inner.x + if focused { self.s(6.) } else { 0 }, ..inner };
            self.put(placeholder, hint, 15., p.muted, Face::Normal, Align::Left);
        } else {
            let chars: Vec<char> = text.chars().collect();
            let at = caret.unwrap_or(chars.len()).min(chars.len());
            let mut shown: String = chars[..at].iter().collect();
            if focused {
                shown.push('│');
            }
            shown.extend(&chars[at..]);
            // Keep the end of long text (where the caret usually is) in view.
            let keep = (inner.width as f32 / (7.5 * self.scale)) as usize;
            let skip = shown.chars().count().saturating_sub(keep.max(8));
            let visible: String = shown.chars().skip(if caret.is_none() { skip } else { 0 }).collect();
            self.put(&visible, inner, 15., p.text, Face::Normal, Align::Left);
        }
        self.hits.push((rect, Hit::Do(hit)));
    }
    pub(super) fn paint_overlay(&mut self, window: Rect, top: i32, p: Palette) {
        let Some(overlay) = &self.overlay else {
            return;
        };
        if overlay.docked() {
            return;
        }
        let dialog = overlay.is_dialog();
        let settings = overlay.kind == Kind::Settings;
        let searching = matches!(overlay.kind, Kind::Find | Kind::Replace);
        self.in_modal = true;
        if searching {
            // Keep the document undimmed so matches stay visible behind the search box.
            self.hits.push((window, Hit::Do(Act::Dismiss)));
        } else {
            self.scrim(window, !dialog || settings, p);
        }
        if settings {
            self.paint_settings(window, p);
        } else if dialog {
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
        let (query, replacement, secondary, caret, case) = (
            overlay.query.clone(),
            overlay.replacement.clone(),
            overlay.secondary,
            overlay.caret,
            overlay.case,
        );
        let placeholder = overlay.placeholder().to_owned();
        let pending = overlay.pending;
        let (selected, scroll) = (overlay.selected, overlay.scroll);
        let searching = matches!(kind, Kind::Find | Kind::Replace | Kind::FolderSearch);
        let keymap = matches!(kind, Kind::Keymap | Kind::Palette | Kind::HeadingLevels);
        let width = window.width.saturating_sub(self.su(40.)).min(self.su(720.));
        let x = (window.width - width) as i32 / 2;
        let y = top + self.s(40.);
        let (row_height, pad) = (self.su(if matches!(kind, Kind::Setting(_)) { 40. } else if keymap { 36. } else { 46. }), self.s(14.));
        let fixed = self.su(46.) + self.su(46.)
            + if kind == Kind::Replace { self.su(46.) } else { 0 }
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
        // Input fields
        let field = |app: &App, y: i32| Rect { x: x + pad, y, width: width - pad as u32 * 2, height: app.su(38.) };
        let first = field(self, cursor);
        self.field(first, &query, &placeholder, caret, !secondary, Act::Field(false), p);
        cursor += self.s(46.);
        if kind == Kind::Replace {
            let second = field(self, cursor);
            self.field(second, &replacement, "Replace with…", caret, secondary, Act::Field(true), p);
            cursor += self.s(46.);
        }
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
            let mut right = bar.x + bar.width as i32;
            let case_button = Rect { x: right - self.s(104.), y: bar.y, width: self.su(104.), height: bar.height };
            self.button("Match case", case_button, Hit::Do(Act::Case), if case { Look::Active } else { Look::Outline }, p);
            right = case_button.x - self.s(8.);
            if kind == Kind::Replace {
                let all = Rect { x: right - self.s(96.), width: self.su(96.), ..case_button };
                self.button("Replace all", all, Hit::Do(Act::Replace(true)), Look::Outline, p);
                let one = Rect { x: all.x - self.s(84.), width: self.su(76.), ..case_button };
                self.button("Replace", one, Hit::Do(Act::Replace(false)), Look::Primary, p);
            }
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
                self.put(&label, Rect { y: line.y + self.s(2.), ..upper }, 13., p.text, Face::Normal, Align::Left);
                let hint = self.fit(&row.hint, inner.width as f32, 11.);
                self.put(&hint, lower, 11., p.muted, Face::Normal, Align::Left);
            }
            self.hits.push((line, Hit::Row(number)));
        }
        if rows_total > visible {
            let track = Rect { x: x + width as i32 - self.s(7.), y: cursor, width: self.su(4.), height: row_height * visible as u32 };
            let thumb = ((track.height as usize * visible / rows_total) as u32).max(self.su(20.));
            let offset = (track.height - thumb) as usize * scroll / (rows_total - visible).max(1);
            self.renderer.fill_round(Rect { y: track.y + offset as i32, height: thumb, ..track }, self.su(2.), p.border);
        }
    }
    fn dialog_buttons(&mut self, rect: Rect, buttons: &[(&str, Act, Look)], p: Palette) {
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
        let (query, caret, note, title) = (overlay.query.clone(), overlay.caret, overlay.note.clone(), overlay.title().to_owned());
        let dirty = self.tabs.values().filter(|t| t.document.dirty()).count();
        let name_of = |id: u64| self.tabs.get(&id).map(|t| t.name()).unwrap_or_default();
        let base = |path: &Path| path.file_name().unwrap_or_default().to_string_lossy().into_owned();
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
                format!("{dirty} open file{} ha{} unsaved changes.\nSave before switching folders?", if dirty == 1 { "" } else { "s" }, if dirty == 1 { "s" } else { "ve" }),
                false,
                vec![("Discard & switch", Act::Press("discard"), Look::Outline), ("Cancel", Act::Press("escape"), Look::Outline), ("Save all & switch", Act::Press("save"), Look::Primary)],
            ),
            Kind::Delete(path) => (
                format!("Move \"{}\" to the Recycle Bin?\nYou can restore it from there later.", base(path)),
                false,
                vec![("Cancel", Act::Press("escape"), Look::Outline), ("Move to Recycle Bin", Act::Press("enter"), Look::Danger)],
            ),
            Kind::GotoLine => (
                "Enter a line number.".into(),
                true,
                vec![("Cancel", Act::Press("escape"), Look::Outline), ("Go", Act::Press("enter"), Look::Primary)],
            ),
            Kind::Create => (
                "Name the new note. It is created inside the workspace,
and .md is added if you leave it out.".into(),
                true,
                vec![("Cancel", Act::Press("escape"), Look::Outline), ("Create", Act::Press("enter"), Look::Primary)],
            ),
            Kind::CreateFolder => (
                "Name the new folder, relative to the workspace folder.".into(),
                true,
                vec![("Cancel", Act::Press("escape"), Look::Outline), ("Create", Act::Press("enter"), Look::Primary)],
            ),
            Kind::Rename(old) => (
                format!("Rename \"{}\".
Type a different folder path to move it as well.", base(old)),
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
        for line in message.lines() {
            self.put(line, Rect { y, height: self.su(22.), ..inner }, 12.5, p.muted, Face::Normal, Align::Left);
            y += self.s(22.);
        }
        y += self.s(10.);
        if input {
            let area = Rect { y, height: self.su(38.), ..inner };
            self.field(area, &query, self.overlay.as_ref().map_or("", |o| o.placeholder()).to_owned().as_str(), caret, true, Act::Field(false), p);
            y += self.s(46.);
        }
        if kind_is_shortcut(&kind) {
            let shown = if query.is_empty() { "No shortcut".to_owned() } else { query.replace("Ctrl", if cfg!(target_os = "macos") { "Cmd" } else { "Ctrl" }) };
            let cap = Rect { x: inner.x, y, width: inner.width, height: self.su(48.) };
            self.renderer.framed(cap, self.su(7.), p.background, p.accent);
            self.put(&shown, cap, 18., p.text, Face::Bold, Align::Center);
            y += self.s(56.);
        }
        if !note.is_empty() {
            self.put(&note, Rect { y, height: self.su(22.), ..inner }, 12., 0xd0605e, Face::Normal, Align::Left);
        }
        self.dialog_buttons(rect, &buttons, p);
    }
    fn section(&mut self, text: &str, x: i32, y: i32, width: u32, p: Palette) {
        let area = Rect { x, y, width, height: self.su(22.) };
        self.put(text, area, 10.5, mix(p.muted, p.accent, 0.35), Face::Bold, Align::Left);
    }
    /// Two or more options in one rounded track; the chosen one is highlighted.
    fn segmented(&mut self, rect: Rect, options: &[(&str, bool, Act)], p: Palette) {
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
    /// A `−  value  +` control.
    fn stepper(&mut self, rect: Rect, value: &str, key: &'static str, p: Palette) {
        self.renderer
            .framed(rect, self.su(7.), p.background, mix(p.border, p.text, 0.14));
        let side = self.su(34.);
        let minus = Rect { width: side, ..rect };
        let plus = Rect { x: rect.x + rect.width as i32 - side as i32, width: side, ..rect };
        for (cell, glyph, direction) in [(minus, "−", -1), (plus, "+", 1)] {
            if self.hot(cell) {
                let inset = Rect {
                    x: cell.x + 2,
                    y: cell.y + 2,
                    width: cell.width - 4,
                    height: cell.height - 4,
                };
                self.renderer.fill_round(inset, self.su(5.), hovered(p.background, p));
            }
            self.put(glyph, cell, 15., p.text, Face::Normal, Align::Center);
            self.hits.push((cell, Hit::Do(Act::Step(key, direction))));
        }
        for x in [minus.x + side as i32, plus.x] {
            self.renderer.fill(
                Rect { x, y: rect.y + self.s(6.), width: 1, height: rect.height - self.su(12.) },
                p.border,
            );
        }
        let middle = Rect { x: minus.x + side as i32, width: rect.width - side * 2, ..rect };
        self.put(value, middle, 13., p.text, Face::Normal, Align::Center);
    }
    fn paint_settings(&mut self, window: Rect, p: Palette) {
        let s = self.settings.clone();
        // Short windows get tighter rows and no description lines so everything still fits.
        let roomy = self.su(58.) + 3 * self.su(34.) + 8 * self.su(30.) + 3 * self.su(42.) + self.su(88.);
        let compact = window.height < roomy + self.su(24.);
        let base = if compact { self.s(26.) } else { ((window.height.saturating_sub(self.su(260.)) / 11).clamp(self.su(30.), self.su(38.))) as i32 };
        let extra = if compact { 0 } else { self.s(12.) };
        let heading = if compact { self.s(26.) } else { self.s(34.) };
        let height = self.su(58.) + (3 * heading) as u32 + (8 * base + 3 * (base + extra)) as u32 + if compact { self.su(70.) } else { self.su(88.) };
        let rect = self.dialog_box(window, 580., height, p);
        let pad = self.s(28.);
        let mut y = rect.y + self.s(14.);
        let head = Rect { x: rect.x + pad, y, width: rect.width - pad as u32 * 2, height: self.su(32.) };
        self.put("Settings", head, 16., p.text, Face::Bold, Align::Left);
        let close = Rect { x: rect.x + rect.width as i32 - self.s(48.), y: y + self.s(1.), width: self.su(30.), height: self.su(30.) };
        self.button("×", close, Hit::Do(Act::Dismiss), Look::Plain, p);
        y += self.s(46.);
        self.renderer.fill(Rect { x: rect.x + 1, y, width: rect.width - 2, height: 1 }, p.border);
        y += self.s(4.);
        let right = rect.x + rect.width as i32 - pad;
        let inner_width = rect.width - pad as u32 * 2;
        let left = rect.x + pad;
        let control_height = if compact { self.su(23.) } else { self.su(30.) };
        // Draws "title" (and optional description) and returns the y that centres a control.
        let label = |app: &mut App, y: i32, row: i32, text: &str, note: Option<&str>| {
            let area = Rect { x: left, y, width: inner_width.saturating_sub(app.su(210.)), height: row as u32 };
            match note {
                Some(note) => {
                    app.put(text, Rect { y: y - app.s(8.), ..area }, 13., p.text, Face::Normal, Align::Left);
                    app.put(note, Rect { y: y + app.s(10.), ..area }, 11., p.muted, Face::Normal, Align::Left);
                }
                None => app.put(text, area, 13., p.text, Face::Normal, Align::Left),
            }
            y + (row - control_height as i32) / 2
        };

        self.section("APPEARANCE", left, y + self.s(8.), inner_width, p);
        y += heading;
        let by = label(self, y, base, "Theme", None);
        self.segmented(
            Rect { x: right - self.s(240.), y: by, width: self.su(240.), height: control_height },
            &[
                ("Light", s.theme == "light", Act::Theme("light")),
                ("Dark", s.theme == "dark", Act::Theme("dark")),
                ("System", s.theme == "system", Act::Theme("system")),
            ],
            p,
        );
        y += base;
        label(self, y, base, "Accent colour", None);
        let size = self.su(20.);
        let gap = self.s(10.);
        for (index, colour) in ACCENTS.into_iter().enumerate() {
            let swatch = Rect {
                x: right - (ACCENTS.len() - index) as i32 * (size as i32 + gap) + gap - self.s(3.),
                y: y + (base - size as i32) / 2,
                width: size,
                height: size,
            };
            let hover = self.hot(swatch);
            if s.accent_color() == colour || hover {
                let ring = |app: &App, grow: i32| Rect {
                    x: swatch.x - app.s(grow as f32),
                    y: swatch.y - app.s(grow as f32),
                    width: size + app.su(grow as f32 * 2.),
                    height: size + app.su(grow as f32 * 2.),
                };
                let (outer, inner) = (ring(self, 4), ring(self, 2));
                let tone = if s.accent_color() == colour { colour } else { p.border };
                self.renderer.fill_round(outer, outer.width / 2, tone);
                self.renderer.fill_round(inner, inner.width / 2, p.raised);
            }
            self.renderer.fill_round(swatch, size / 2, colour);
            self.hits.push((swatch, Hit::Do(Act::Accent(colour))));
        }
        y += base;
        let by = label(self, y, base, "Interface font", None);
        let button = Rect { x: right - self.s(230.), y: by, width: self.su(230.), height: control_height };
        let name = self.fit(&s.interface_font, button.width as f32 - self.s(44.) as f32, 13.);
        self.button(&format!("{name}  ▾"), button, Hit::Do(Act::Font("interface_font")), Look::Outline, p);
        y += base;

        self.section("EDITOR", left, y + self.s(8.), inner_width, p);
        y += heading;
        for (text, key, value) in [
            ("Editor font", "editor_font", s.editor_font.clone()),
            ("Code font", "code_font", s.code_font.clone()),
        ] {
            let by = label(self, y, base, text, None);
            let button = Rect { x: right - self.s(230.), y: by, width: self.su(230.), height: control_height };
            let name = self.fit(&value, button.width as f32 - self.s(44.) as f32, 13.);
            self.button(&format!("{name}  ▾"), button, Hit::Do(Act::Font(key)), Look::Outline, p);
            y += base;
        }
        for (text, key, value) in [
            ("Font size", "font_size", format!("{} pt", s.font_size)),
            ("Maximum line width", "max_width", s.max_width.map_or("Unlimited".into(), |w| format!("{w} px"))),
        ] {
            let by = label(self, y, base, text, None);
            self.stepper(Rect { x: right - self.s(190.), y: by, width: self.su(190.), height: control_height }, &value, key, p);
            y += base;
        }
        let switches = [
            ("Show Markdown symbols", Some("Keep # and ** visible on every line, not just the current one"), "show_markers", s.show_markers),
            ("Typewriter scrolling", Some("Keep the line you are typing in the middle of the screen"), "typewriter", s.typewriter),
        ];
        for (text, note, key, on) in switches {
            y = self.switch_row(rect, y, base + extra, text, if compact { None } else { note }, key, on, p);
        }
        self.section("GENERAL", left, y + self.s(8.), inner_width, p);
        y += heading;
        y = self.switch_row(rect, y, base + extra, "Autosave", if compact { None } else { Some("Save shortly after you stop typing") }, "autosave", s.autosave, p);
        let by = label(self, y, base, "Keyboard shortcuts", None);
        let button = Rect { x: right - self.s(140.), y: by, width: self.su(140.), height: control_height };
        self.button("Customize…", button, Hit::Do(Act::Run(Command::Keymap)), Look::Outline, p);

        let foot = Rect { x: rect.x + pad, y: rect.y + rect.height as i32 - self.s(52.), width: rect.width - pad as u32 * 2, height: self.su(34.) };
        self.renderer.fill(Rect { x: rect.x + 1, y: foot.y - self.s(10.), width: rect.width - 2, height: 1 }, p.border);
        self.put("Changes apply immediately and are saved to settings.toml.", foot, 11.5, p.muted, Face::Normal, Align::Left);
        let done = Rect { x: right - self.s(90.), width: self.su(90.), ..foot };
        self.button("Done", done, Hit::Do(Act::Dismiss), Look::Primary, p);
    }
    #[allow(clippy::too_many_arguments)]
    fn switch_row(&mut self, rect: Rect, y: i32, row: i32, text: &str, note: Option<&str>, key: &'static str, on: bool, p: Palette) -> i32 {
        let pad = self.s(28.);
        let line = Rect { x: rect.x + self.s(10.), y, width: rect.width - self.su(20.), height: row as u32 };
        if self.hot(line) {
            self.renderer.fill_round(line, self.su(7.), hovered(p.raised, p));
        }
        let area = Rect { x: rect.x + pad, width: rect.width - pad as u32 * 2 - self.su(60.), ..line };
        match note {
            Some(note) => {
                self.put(text, Rect { y: y - self.s(9.), ..area }, 13., p.text, Face::Normal, Align::Left);
                self.put(note, Rect { y: y + self.s(10.), ..area }, 11., p.muted, Face::Normal, Align::Left);
            }
            None => self.put(text, area, 13., p.text, Face::Normal, Align::Left),
        }
        let track = Rect {
            x: rect.x + rect.width as i32 - pad - self.s(40.),
            y: y + (row - self.s(22.)) / 2,
            width: self.su(40.),
            height: self.su(22.),
        };
        let off = mix(p.border, p.text, if p.light { 0.22 } else { 0.28 });
        self.renderer.fill_round(track, track.height / 2, if on { p.accent } else { off });
        let knob = track.height - self.su(6.);
        let travel = (track.width - knob) as i32 - self.s(3.);
        self.renderer.fill_round(
            Rect { x: track.x + if on { travel } else { self.s(3.) }, y: track.y + self.s(3.), width: knob, height: knob },
            knob / 2,
            0xffffff,
        );
        self.hits.push((line, Hit::Do(Act::Toggle(key))));
        y + row
    }
    fn icon_doc(&mut self, x: i32, y: i32, color: u32, background: u32) {
        let rect = Rect { x, y, width: self.su(11.), height: self.su(14.) };
        self.renderer.framed(rect, self.su(2.), background, color);
        for line in 0..3 {
            let bar = Rect { x: x + self.s(3.), y: y + self.s(4.) + line * self.s(3.), width: self.su(5.), height: 1 };
            self.renderer.fill(bar, color);
        }
    }
    fn icon_folder(&mut self, x: i32, y: i32, color: u32) {
        let tab = Rect { x, y: y + self.s(1.), width: self.su(6.), height: self.su(5.) };
        self.renderer.fill_round(tab, self.su(2.), color);
        let body = Rect { x, y: y + self.s(3.), width: self.su(15.), height: self.su(10.) };
        self.renderer.fill_round(body, self.su(2.), color);
    }
    pub(super) fn dividers_for(&mut self, center: Rect) {
        self.dividers.clear();
        let mut found: Vec<Divider> = Vec::new();
        self.split.dividers(center, &mut Vec::new(), &mut found);
        self.dividers = found;
    }
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
