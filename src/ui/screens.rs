//! Full-window views: the snapshot history, the comparison of two versions, and the paged preview.
use super::*;
use super::gui::{Align, Face, Look, hovered, mix, selected_fill};
use crate::{
    core::comments::CommentSet,
    export::typeset::{Bitmap, Typeset},
    history::{
        self, diff, restore,
        store::{Kind as SnapshotKind, SnapshotMeta, State},
    },
    jw::{self, manifest::COMMENTS},
};
use forms::ExportRequest;
use std::collections::{HashMap, HashSet};

#[derive(Clone)]
pub enum ScreenAct {
    Close,
    Select(usize),
    Mark(usize),
    CompareCurrent(usize),
    CompareMarked(usize),
    RestoreProject(usize),
    RestoreItem(usize, String),
    CompareItem(usize, String),
    SwitchLine(String),
    NewLine,
    RenameLine,
    Pick(usize),
    HideNotes,
    ShowComments,
    ZoomIn,
    ZoomOut,
    ZoomFit,
    ExportNow,
}

pub enum Screen {
    History(HistoryView),
    Compare(Box<CompareView>),
    Preview(Box<PreviewView>),
}

pub struct HistoryView {
    /// The current line, newest first.
    pub snapshots: Vec<SnapshotMeta>,
    pub selected: usize,
    pub scroll: usize,
    pub marked: Option<String>,
    /// What differs between the selected snapshot and the text now.
    pub changes: Vec<diff::ItemChange>,
    pub change_scroll: usize,
    pub lines: Vec<String>,
    pub current: String,
    pub split: i32,
}

pub struct CompareView {
    pub a: State,
    pub b: State,
    pub comparison: diff::Comparison,
    pub items: Vec<diff::ItemChange>,
    /// `None` shows the comment differences.
    pub selected: Option<usize>,
    pub rows: Vec<diff::Row>,
    pub comment_lines: Vec<(char, String)>,
    pub list_scroll: usize,
    pub scroll: usize,
    pub hide_notes: bool,
    pub show_comments: bool,
    pub split: i32,
}

pub struct PreviewView {
    pub request: ExportRequest,
    pub typeset: Option<Arc<Typeset>>,
    pub error: Option<String>,
    pub version: u64,
    pub zoom: f32,
    /// Distance from the top of the first page, in points.
    pub scroll: f32,
    pub spreads: bool,
    pub cache: HashMap<(usize, u32), Arc<Bitmap>>,
    pub requested: HashSet<(usize, u32)>,
    pub worker: Option<mpsc::Sender<(usize, u32)>>,
    pub viewport: f32,
    pub total: f32,
    pub first_visible: usize,
    pub warnings: Vec<String>,
}

fn kind_name(kind: SnapshotKind) -> &'static str {
    match kind {
        SnapshotKind::Manual => "Snapshot",
        SnapshotKind::Auto => "Automatic",
        SnapshotKind::Safety => "Safety copy",
        SnapshotKind::Restore => "Restore",
    }
}

impl App {
    pub(super) fn close_screen(&mut self) {
        self.screen = None;
        if let Some(form) = self.parked_form.take() {
            self.form = Some(form);
        }
        self.redraw();
    }

    // ---- history ----------------------------------------------------------------------------

    pub(super) fn open_history(&mut self) {
        if self.project.is_none() {
            self.status = "Open a project first.".into();
            return;
        }
        self.flush_tabs();
        self.form = None;
        self.screen = Some(Screen::History(HistoryView {
            snapshots: Vec::new(),
            selected: 0,
            scroll: 0,
            marked: None,
            changes: Vec::new(),
            change_scroll: 0,
            lines: Vec::new(),
            current: String::new(),
            split: 0,
        }));
        self.refresh_history();
    }
    /// Rebuilds the history view from the project, keeping what was selected if it still exists.
    pub(super) fn refresh_history(&mut self) {
        let Some(Screen::History(view)) = &mut self.screen else { return };
        let Some(state) = &self.project else { return };
        let keep = view.snapshots.get(view.selected).map(|s| s.id.clone());
        let line = state.history.current_line().to_owned();
        view.snapshots = state.history.on_line(&line).cloned().collect();
        view.snapshots.reverse();
        view.lines = state.history.lines.lines.iter().map(|l| l.name.clone()).collect();
        view.current = line;
        view.selected = keep.and_then(|id| view.snapshots.iter().position(|s| s.id == id)).unwrap_or(0);
        self.select_snapshot(self.history_selected());
    }
    fn history_selected(&self) -> usize {
        match &self.screen {
            Some(Screen::History(v)) => v.selected,
            _ => 0,
        }
    }
    fn select_snapshot(&mut self, index: usize) {
        let Some(Screen::History(view)) = &mut self.screen else { return };
        let Some(state) = &self.project else { return };
        view.selected = index.min(view.snapshots.len().saturating_sub(1));
        view.change_scroll = 0;
        view.changes.clear();
        if let Some(meta) = view.snapshots.get(view.selected) {
            if let (Ok(snapshot), Ok(working)) = (state.history.state(&state.core.work, &meta.id), State::working(&state.core, false)) {
                view.changes = diff::compare(&snapshot, &working).changed().cloned().collect();
            }
        }
    }

    /// Everything open is stale after a restore: tree, titles, text and comments.
    fn after_restore(&mut self) {
        let Some(state) = &mut self.project else { return };
        state.comments = state.core.work.read(COMMENTS).map(|b| CommentSet::parse(&b)).unwrap_or_default();
        state.changed = true;
        let gone: Vec<u64> = self
            .tabs
            .iter()
            .filter(|(id, t)| t.path.is_some() && self.item_of_tab(**id).is_none())
            .map(|(id, _)| *id)
            .collect();
        for id in gone {
            self.close_tab(id, true);
        }
        self.refresh_titles();
        self.reload_tabs();
        self.refresh_history();
    }

    fn history_act(&mut self, act: ScreenAct) {
        let now = jw::time::now();
        match act {
            ScreenAct::Select(i) => self.select_snapshot(i),
            ScreenAct::Mark(i) => {
                if let Some(Screen::History(v)) = &mut self.screen {
                    let id = v.snapshots.get(i).map(|s| s.id.clone());
                    v.marked = if v.marked == id { None } else { id };
                }
            }
            ScreenAct::CompareCurrent(i) => {
                let Some(id) = self.snapshot_id(i) else { return };
                self.flush_tabs();
                self.compare_with_working(&id, None);
            }
            ScreenAct::CompareMarked(i) => {
                let (Some(id), Some(Screen::History(v))) = (self.snapshot_id(i), &self.screen) else { return };
                let Some(marked) = v.marked.clone() else { return };
                self.compare_snapshots(&marked, &id);
            }
            ScreenAct::CompareItem(i, item) => {
                let Some(id) = self.snapshot_id(i) else { return };
                self.flush_tabs();
                self.compare_with_working(&id, Some(item));
            }
            ScreenAct::RestoreProject(i) => {
                let Some(id) = self.snapshot_id(i) else { return };
                self.flush_tabs();
                let result = self.project.as_mut().map_or(Err("No project".into()), |s| restore::restore_project(&mut s.history, &mut s.core, &id, now));
                match result {
                    Ok(()) => {
                        self.after_restore();
                        self.status = "Restored. What was there before is kept in a safety snapshot.".into();
                    }
                    Err(error) => self.status = format!("Could not restore: {error}"),
                }
            }
            ScreenAct::RestoreItem(i, item) => {
                let Some(id) = self.snapshot_id(i) else { return };
                self.flush_tabs();
                let result = self.project.as_mut().map_or(Err("No project".into()), |s| restore::restore_item(&mut s.history, &mut s.core, &id, &item, now));
                match result {
                    Ok(()) => {
                        self.after_restore();
                        self.status = "Chapter restored. The earlier text is kept in a safety snapshot.".into();
                    }
                    Err(error) => self.status = format!("Could not restore: {error}"),
                }
            }
            ScreenAct::SwitchLine(name) => {
                self.flush_tabs();
                let result = self.project.as_mut().map_or(Err("No project".into()), |s| restore::switch_line(&mut s.history, &mut s.core, &name, now));
                match result {
                    Ok(()) => {
                        self.after_restore();
                        self.status = format!("Now on the “{name}” line.");
                    }
                    Err(error) => self.status = error,
                }
            }
            ScreenAct::NewLine => self.new_line_form(),
            ScreenAct::RenameLine => {
                if let Some(Screen::History(v)) = &self.screen {
                    let current = v.current.clone();
                    self.rename_line_form(&current);
                }
            }
            _ => {}
        }
    }
    fn snapshot_id(&self, index: usize) -> Option<String> {
        match &self.screen {
            Some(Screen::History(v)) => v.snapshots.get(index).map(|s| s.id.clone()),
            _ => None,
        }
    }

    // ---- comparing --------------------------------------------------------------------------

    fn compare_with_working(&mut self, snapshot: &str, item: Option<String>) {
        let Some(state) = &self.project else { return };
        let (Ok(a), Ok(b)) = (state.history.state(&state.core.work, snapshot), State::working(&state.core, false)) else {
            self.status = "Could not read the snapshot.".into();
            return;
        };
        self.open_compare(a, b, item);
    }
    fn compare_snapshots(&mut self, first: &str, second: &str) {
        let Some(state) = &self.project else { return };
        let (Some(x), Some(y)) = (state.history.get(first), state.history.get(second)) else { return };
        let (older, newer) = if x.time <= y.time { (first, second) } else { (second, first) };
        let (Ok(a), Ok(b)) = (state.history.state(&state.core.work, older), state.history.state(&state.core.work, newer)) else {
            self.status = "Could not read the snapshots.".into();
            return;
        };
        self.open_compare(a, b, None);
    }
    fn open_compare(&mut self, a: State, b: State, select: Option<String>) {
        let comparison = diff::compare(&a, &b);
        let items: Vec<diff::ItemChange> = comparison.changed().cloned().collect();
        let selected = select.and_then(|id| items.iter().position(|i| i.id == id)).or(if items.is_empty() { None } else { Some(0) });
        let comment_lines = comment_changes(&self.project.as_ref().map(|s| s.core.work.clone()), &a, &b);
        let mut view = CompareView {
            a,
            b,
            comparison,
            items,
            selected,
            rows: Vec::new(),
            comment_lines,
            list_scroll: 0,
            scroll: 0,
            hide_notes: false,
            show_comments: true,
            split: 0,
        };
        self.load_compare_rows(&mut view);
        self.screen = Some(Screen::Compare(Box::new(view)));
        self.redraw();
    }
    fn load_compare_rows(&self, view: &mut CompareView) {
        view.rows.clear();
        view.scroll = 0;
        let (Some(state), Some(index)) = (&self.project, view.selected) else { return };
        let Some(item) = view.items.get(index) else { return };
        let marker = self.marker();
        let options = diff::Options { hide_notes: view.hide_notes, context: 2, marker: &marker };
        view.rows = diff::item_rows(&state.core.work, &view.a, &view.b, &item.id, &options);
    }
    fn compare_act(&mut self, act: ScreenAct) {
        let Some(Screen::Compare(mut view)) = self.screen.take() else { return };
        match act {
            ScreenAct::Pick(i) => {
                view.selected = Some(i).filter(|i| *i < view.items.len());
                self.load_compare_rows(&mut view);
            }
            ScreenAct::HideNotes => {
                view.hide_notes = !view.hide_notes;
                self.load_compare_rows(&mut view);
            }
            ScreenAct::ShowComments => {
                view.show_comments = !view.show_comments;
                if !view.show_comments && view.selected.is_none() && !view.items.is_empty() {
                    view.selected = Some(0);
                    self.load_compare_rows(&mut view);
                }
            }
            _ => {}
        }
        self.screen = Some(Screen::Compare(view));
    }

    // ---- preview ----------------------------------------------------------------------------

    /// The Preview command: the export dialog's preset laid out in pages.
    pub(super) fn open_preview(&mut self) {
        if self.project.is_none() {
            self.status = "Open a project first.".into();
            return;
        }
        self.export_form();
        if self.form.is_some() {
            self.form_extra("preview");
        }
    }
    pub(super) fn open_preview_for(&mut self, request: ExportRequest) {
        self.preview_version += 1;
        let version = self.preview_version;
        let spreads = request.preset.mirrored || request.preset.recto_start;
        let proxy = self.proxy.clone();
        let (source, scope, preset) = (request.source.clone(), request.scope.clone(), request.preset.clone());
        std::thread::spawn(move || {
            let result = crate::export::preview(&source, &scope, &preset).map(Arc::new);
            let _ = proxy.send_event(UserEvent::Preview(version, result));
        });
        if self.parked_form.is_none() {
            self.parked_form = self.form.take();
        }
        self.form = None;
        self.screen = Some(Screen::Preview(Box::new(PreviewView {
            request,
            typeset: None,
            error: None,
            version,
            zoom: 1.,
            scroll: 0.,
            spreads,
            cache: HashMap::new(),
            requested: HashSet::new(),
            worker: None,
            viewport: 800.,
            total: 0.,
            first_visible: 0,
            warnings: Vec::new(),
        })));
        self.redraw();
    }
    pub(super) fn preview_ready(&mut self, version: u64, result: Result<Arc<Typeset>, String>) {
        let Some(Screen::Preview(view)) = &mut self.screen else { return };
        if view.version != version {
            return;
        }
        match result {
            Ok(typeset) => {
                view.warnings = typeset.warnings.clone();
                let (sender, receiver) = mpsc::channel::<(usize, u32)>();
                let worker = typeset.clone();
                let proxy = self.proxy.clone();
                std::thread::spawn(move || {
                    while let Ok((page, q)) = receiver.recv() {
                        // Only the newest requests matter when scrolling fast.
                        let mut latest = (page, q);
                        let mut skipped = Vec::new();
                        while let Ok(next) = receiver.try_recv() {
                            skipped.push(latest);
                            latest = next;
                        }
                        for (page, q) in skipped.into_iter().chain([latest]) {
                            if let Some(bitmap) = worker.render(page, q as f32 / 20.) {
                                let _ = proxy.send_event(UserEvent::PreviewPage(version, page, q, Arc::new(bitmap)));
                            }
                        }
                    }
                });
                view.worker = Some(sender);
                view.typeset = Some(typeset);
            }
            Err(error) => view.error = Some(error),
        }
        self.redraw();
    }
    pub(super) fn preview_page(&mut self, version: u64, page: usize, q: u32, bitmap: Arc<Bitmap>) {
        let Some(Screen::Preview(view)) = &mut self.screen else { return };
        if view.version != version {
            return;
        }
        view.cache.insert((page, q), bitmap);
        view.requested.remove(&(page, q));
        if view.cache.len() > 14 {
            let keep = view.first_visible;
            let far = view.cache.keys().copied().max_by_key(|(p, _)| p.abs_diff(keep));
            if let Some(far) = far {
                view.cache.remove(&far);
            }
        }
        self.redraw();
    }
    fn preview_act(&mut self, act: ScreenAct) {
        let Some(Screen::Preview(view)) = &mut self.screen else { return };
        match act {
            ScreenAct::ZoomIn => view.zoom = (view.zoom * 1.25).min(4.),
            ScreenAct::ZoomOut => view.zoom = (view.zoom / 1.25).max(0.3),
            ScreenAct::ZoomFit => view.zoom = 1.,
            ScreenAct::ExportNow => {
                let name = format!("{}.{}", forms::file_stem(view.request.source.book.display_title(&view.request.source.title)), view.request.preset.format.extension());
                let request = ExportRequest { source: view.request.source.clone(), scope: view.request.scope.clone(), preset: view.request.preset.clone() };
                self.pending_export = Some((request, name));
                self.choose(Choose::Export);
            }
            _ => {}
        }
    }

    // ---- input ------------------------------------------------------------------------------

    pub(super) fn screen_act(&mut self, act: ScreenAct) {
        if matches!(act, ScreenAct::Close) {
            // Closing a comparison returns to the history it was opened from.
            if matches!(self.screen, Some(Screen::Compare(_))) && self.project.is_some() {
                self.open_history();
                return;
            }
            self.close_screen();
            return;
        }
        match &self.screen {
            Some(Screen::History(_)) => self.history_act(act),
            Some(Screen::Compare(_)) => self.compare_act(act),
            Some(Screen::Preview(_)) => self.preview_act(act),
            None => {}
        }
        self.redraw();
    }
    pub(super) fn screen_key(&mut self, key: &Key) -> bool {
        if self.screen.is_none() {
            return false;
        }
        let shift = self.modifiers.shift_key();
        let primary = platform::primary(self.modifiers);
        match key {
            Key::Named(NamedKey::Escape) => self.screen_act(ScreenAct::Close),
            Key::Named(NamedKey::ArrowDown | NamedKey::ArrowUp) => {
                let down = matches!(key, Key::Named(NamedKey::ArrowDown));
                match &mut self.screen {
                    Some(Screen::History(v)) => {
                        let next = if down { (v.selected + 1).min(v.snapshots.len().saturating_sub(1)) } else { v.selected.saturating_sub(1) };
                        if next < v.scroll {
                            v.scroll = next;
                        }
                        self.select_snapshot(next);
                    }
                    Some(Screen::Compare(v)) => {
                        v.scroll = if down { (v.scroll + 1).min(v.rows.len().saturating_sub(1)) } else { v.scroll.saturating_sub(1) };
                    }
                    Some(Screen::Preview(v)) => v.scroll = (v.scroll + if down { 60. } else { -60. }).clamp(0., (v.total - v.viewport).max(0.)),
                    None => {}
                }
            }
            Key::Named(NamedKey::PageDown | NamedKey::PageUp) => {
                let down = matches!(key, Key::Named(NamedKey::PageDown));
                self.screen_scroll(if down { 12 } else { -12 }, true);
            }
            Key::Named(NamedKey::Home) => self.screen_jump(false),
            Key::Named(NamedKey::End) => self.screen_jump(true),
            Key::Character(c) if matches!(self.screen, Some(Screen::Preview(_))) && (c.as_str() == "+" || c.as_str() == "=") => self.screen_act(ScreenAct::ZoomIn),
            Key::Character(c) if matches!(self.screen, Some(Screen::Preview(_))) && c.as_str() == "-" => self.screen_act(ScreenAct::ZoomOut),
            Key::Character(c) if matches!(self.screen, Some(Screen::Preview(_))) && c.as_str() == "0" => self.screen_act(ScreenAct::ZoomFit),
            _ => {
                // Everything else still works while a view is open (menus, find...).
                let _ = (shift, primary);
                return false;
            }
        }
        self.redraw();
        true
    }
    fn screen_jump(&mut self, end: bool) {
        match &mut self.screen {
            Some(Screen::Compare(v)) => v.scroll = if end { v.rows.len().saturating_sub(1) } else { 0 },
            Some(Screen::Preview(v)) => v.scroll = if end { (v.total - v.viewport).max(0.) } else { 0. },
            Some(Screen::History(v)) => {
                let target = if end { v.snapshots.len().saturating_sub(1) } else { 0 };
                v.scroll = target;
                self.select_snapshot(target);
            }
            None => {}
        }
    }
    pub(super) fn screen_scroll(&mut self, rows: i32, _key: bool) {
        let x = self.mouse.x as i32;
        match &mut self.screen {
            Some(Screen::History(v)) => {
                if x < v.split {
                    v.scroll = (v.scroll as i32 + rows).clamp(0, v.snapshots.len().saturating_sub(1) as i32) as usize;
                } else {
                    v.change_scroll = (v.change_scroll as i32 + rows).clamp(0, v.changes.len().saturating_sub(1) as i32) as usize;
                }
            }
            Some(Screen::Compare(v)) => {
                if x < v.split {
                    v.list_scroll = (v.list_scroll as i32 + rows).clamp(0, v.items.len().saturating_sub(1) as i32) as usize;
                } else {
                    let max = if v.selected.is_none() { v.comment_lines.len() } else { v.rows.len() };
                    v.scroll = (v.scroll as i32 + rows * 2).clamp(0, max.saturating_sub(1) as i32) as usize;
                }
            }
            Some(Screen::Preview(v)) => {
                if self.modifiers.control_key() || self.modifiers.super_key() {
                    v.zoom = if rows < 0 { (v.zoom * 1.1).min(4.) } else { (v.zoom / 1.1).max(0.3) };
                } else {
                    v.scroll = (v.scroll + rows as f32 * 40.).clamp(0., (v.total - v.viewport).max(0.));
                }
            }
            None => {}
        }
    }

    // ---- painting ---------------------------------------------------------------------------

    pub(super) fn paint_screen(&mut self, rect: Rect, p: Palette) {
        let Some(screen) = self.screen.take() else { return };
        self.renderer.fill(rect, p.background);
        let screen = match screen {
            Screen::History(mut v) => {
                self.paint_history(rect, &mut v, p);
                Screen::History(v)
            }
            Screen::Compare(mut v) => {
                self.paint_compare(rect, &mut v, p);
                Screen::Compare(v)
            }
            Screen::Preview(mut v) => {
                self.paint_preview(rect, &mut v, p);
                Screen::Preview(v)
            }
        };
        self.screen = Some(screen);
    }
    fn screen_header(&mut self, rect: Rect, title: &str, p: Palette) -> Rect {
        let height = self.su(46.);
        self.renderer.fill(Rect { height, ..rect }, p.surface);
        self.renderer.fill(Rect { y: rect.y + height as i32 - 1, height: 1, ..rect }, p.border);
        let back = Rect { x: rect.x + self.s(10.), y: rect.y + self.s(7.), width: self.su(150.), height: self.su(32.) };
        self.button("← Back to writing", back, Hit::Do(Act::Screen(ScreenAct::Close)), Look::Outline, p);
        let text = Rect { x: back.x + back.width as i32 + self.s(16.), y: rect.y, width: rect.width.saturating_sub(back.width + self.su(40.)), height };
        self.put(title, text, 15., p.text, Face::Bold, Align::Left);
        Rect { y: rect.y + height as i32, height: rect.height - height, ..rect }
    }

    fn paint_history(&mut self, rect: Rect, v: &mut HistoryView, p: Palette) {
        let body = self.screen_header(rect, "Snapshots and history", p);
        // Lines
        let bar = Rect { height: self.su(44.), ..body };
        let mut x = bar.x + self.s(14.);
        self.put("LINES", Rect { x, width: self.su(54.), ..bar }, 10.5, p.muted, Face::Bold, Align::Left);
        x += self.s(58.);
        for name in v.lines.clone() {
            let width = self.measure(&name, 13. * self.scale, Face::Normal).ceil() as u32 + self.su(26.);
            let chip = Rect { x, y: bar.y + self.s(7.), width, height: self.su(30.) };
            let on = name == v.current;
            self.button(&name, chip, Hit::Do(Act::Screen(ScreenAct::SwitchLine(name.clone()))), if on { Look::Primary } else { Look::Outline }, p);
            x += width as i32 + self.s(6.);
        }
        let new = Rect { x, y: bar.y + self.s(7.), width: self.su(100.), height: self.su(30.) };
        self.button("+ New line", new, Hit::Do(Act::Screen(ScreenAct::NewLine)), Look::Plain, p);
        let rename = Rect { x: new.x + new.width as i32 + self.s(4.), width: self.su(70.), ..new };
        self.button("Rename", rename, Hit::Do(Act::Screen(ScreenAct::RenameLine)), Look::Plain, p);
        self.renderer.fill(Rect { y: bar.y + bar.height as i32 - 1, height: 1, ..bar }, p.border);
        let area = Rect { y: bar.y + bar.height as i32, height: body.height - bar.height, ..body };
        let left_width = (area.width as f32 * 0.42).min(self.s(520.) as f32) as u32;
        v.split = area.x + left_width as i32;
        let list = Rect { width: left_width, ..area };
        self.renderer.fill(Rect { x: list.x + list.width as i32 - 1, width: 1, ..list }, p.border);
        if v.snapshots.is_empty() {
            self.put("No snapshots on this line yet. Press Ctrl+Alt+S to take one.", Rect { x: list.x + self.s(16.), y: list.y + self.s(16.), width: list.width - self.su(32.), height: self.su(30.) }, 12.5, p.muted, Face::Normal, Align::Left);
        }
        let row_height = self.s(58.);
        let visible = (list.height as i32 / row_height).max(1) as usize;
        if v.selected >= v.scroll + visible {
            v.scroll = v.selected + 1 - visible;
        }
        v.scroll = v.scroll.min(v.snapshots.len().saturating_sub(visible));
        let now = jw::time::now();
        for (n, meta) in v.snapshots.iter().enumerate().skip(v.scroll).take(visible) {
            let row = Rect { x: list.x + self.s(6.), y: list.y + (n - v.scroll) as i32 * row_height, width: list.width - self.su(12.), height: row_height as u32 - 2 };
            if n == v.selected {
                self.renderer.fill_round(row, self.su(6.), selected_fill(p.background, p));
            } else if self.hot(row) {
                self.renderer.fill_round(row, self.su(6.), hovered(p.background, p));
            }
            let marked = v.marked.as_deref() == Some(meta.id.as_str());
            let title = if meta.message.is_empty() { kind_name(meta.kind).to_owned() } else { meta.message.clone() };
            let title = self.fit(&title, row.width as f32 - self.s(110.) as f32, 13.5);
            self.put(&title, Rect { x: row.x + self.s(12.), y: row.y + self.s(4.), width: row.width - self.su(24.), height: self.su(24.) }, 13.5, p.text, if meta.kind == SnapshotKind::Manual { Face::Bold } else { Face::Normal }, Align::Left);
            let sub = format!("{} · {} · {} words", jw::time::local(meta.time), jw::time::ago(meta.time, now), meta.words);
            self.put(&sub, Rect { x: row.x + self.s(12.), y: row.y + self.s(30.), width: row.width - self.su(24.), height: self.su(20.) }, 11.5, p.muted, Face::Normal, Align::Left);
            let badge = if marked { "marked".to_owned() } else { kind_name(meta.kind).to_owned() };
            let tint = match meta.kind {
                SnapshotKind::Manual => p.accent,
                SnapshotKind::Auto => p.muted,
                SnapshotKind::Safety => 0xd08a50,
                SnapshotKind::Restore => 0x6aa6d6,
            };
            self.put(&badge, Rect { x: row.x, y: row.y + self.s(4.), width: row.width - self.su(12.), height: self.su(24.) }, 11., if marked { p.accent } else { tint }, Face::Bold, Align::Right);
            self.hits.push((row, Hit::Do(Act::Screen(ScreenAct::Select(n)))));
        }
        // Details
        let detail = Rect { x: list.x + list.width as i32 + self.s(20.), y: area.y + self.s(16.), width: area.width.saturating_sub(left_width + self.su(40.)), height: area.height.saturating_sub(self.su(32.)) };
        let Some(meta) = v.snapshots.get(v.selected).cloned() else { return };
        let title = if meta.message.is_empty() { kind_name(meta.kind).to_owned() } else { meta.message.clone() };
        let title = self.fit(&title, detail.width as f32, 17.);
        self.put(&title, Rect { height: self.su(30.), ..detail }, 17., p.text, Face::Bold, Align::Left);
        let mut info = format!("{} · {} · {} words in the manuscript · {} pages and chapters", jw::time::local(meta.time), kind_name(meta.kind), meta.words, meta.items);
        if let Some(from) = meta.restored_from.as_deref().and_then(|id| self.project.as_ref().and_then(|s| s.history.get(id))) {
            info.push_str(&format!(" · restored from {}", jw::time::local(from.time)));
        }
        let info = self.fit(&info, detail.width as f32, 12.);
        self.put(&info, Rect { y: detail.y + self.s(32.), height: self.su(22.), ..detail }, 12., p.muted, Face::Normal, Align::Left);
        let mut x = detail.x;
        let y = detail.y + self.s(64.);
        let marked_other = v.marked.as_deref().is_some_and(|m| m != meta.id);
        let mut buttons: Vec<(String, ScreenAct, Look)> = vec![
            ("Compare with current text".into(), ScreenAct::CompareCurrent(v.selected), Look::Primary),
            ("Restore the whole project".into(), ScreenAct::RestoreProject(v.selected), Look::Outline),
            (if v.marked.as_deref() == Some(meta.id.as_str()) { "Unmark" } else { "Mark to compare" }.into(), ScreenAct::Mark(v.selected), Look::Outline),
        ];
        if marked_other {
            buttons.push(("Compare with the marked one".into(), ScreenAct::CompareMarked(v.selected), Look::Outline));
        }
        let height = self.su(32.);
        let mut yy = y;
        for (text, act, look) in buttons {
            let width = self.measure(&text, 13. * self.scale, Face::Normal).ceil() as u32 + self.su(28.);
            if x + width as i32 > detail.x + detail.width as i32 && x > detail.x {
                x = detail.x;
                yy += height as i32 + self.s(8.);
            }
            self.button(&text, Rect { x, y: yy, width, height }, Hit::Do(Act::Screen(act)), look, p);
            x += width as i32 + self.s(8.);
        }
        let list_top = yy + height as i32 + self.s(20.);
        let heading = format!("CHANGED SINCE THEN ({})", v.changes.len());
        self.section(&heading, detail.x, list_top - self.s(4.), detail.width, p);
        if v.changes.is_empty() {
            self.put("The text is the same as in this snapshot.", Rect { x: detail.x, y: list_top + self.s(22.), width: detail.width, height: self.su(26.) }, 12.5, p.muted, Face::Normal, Align::Left);
        }
        let item_height = self.s(50.);
        let avail = ((detail.y + detail.height as i32 - list_top - self.s(26.)) / item_height).max(1) as usize;
        v.change_scroll = v.change_scroll.min(v.changes.len().saturating_sub(avail));
        for (n, change) in v.changes.iter().enumerate().skip(v.change_scroll).take(avail) {
            let row = Rect { x: detail.x, y: list_top + self.s(22.) + (n - v.change_scroll) as i32 * item_height, width: detail.width, height: item_height as u32 - 4 };
            self.renderer.framed(row, self.su(6.), p.surface, p.border);
            let (label, tint) = match change.status {
                diff::Status::Added => ("Added since", 0x6cc27a),
                diff::Status::Removed => ("Deleted since", 0xe0706c),
                diff::Status::Changed => ("Edited", p.accent),
                diff::Status::Unchanged => (if change.was.is_some() { "Renamed" } else { "Moved" }, p.muted),
            };
            let name = self.fit(&change.title, row.width as f32 * 0.4, 13.);
            self.put(&name, Rect { x: row.x + self.s(12.), y: row.y + self.s(3.), width: row.width / 2, height: self.su(24.) }, 13., p.text, Face::Normal, Align::Left);
            let words = format!("{label} · {} → {} words", change.words_a, change.words_b);
            self.put(&words, Rect { x: row.x + self.s(12.), y: row.y + self.s(22.), width: row.width / 2, height: self.su(20.) }, 11., tint, Face::Normal, Align::Left);
            let view = Rect { x: row.x + row.width as i32 - self.s(250.), y: row.y + self.s(7.), width: self.su(110.), height: self.su(30.) };
            self.button("Compare", view, Hit::Do(Act::Screen(ScreenAct::CompareItem(v.selected, change.id.clone()))), Look::Outline, p);
            if change.status != diff::Status::Added {
                let restore = Rect { x: view.x + view.width as i32 + self.s(8.), width: self.su(124.), ..view };
                self.button("Restore this", restore, Hit::Do(Act::Screen(ScreenAct::RestoreItem(v.selected, change.id.clone()))), Look::Outline, p);
            }
        }
    }

    fn paint_compare(&mut self, rect: Rect, v: &mut CompareView, p: Palette) {
        let title = format!("Compare: {}  →  {}", v.a.label, v.b.label);
        let title = self.fit(&title, rect.width as f32 - self.s(200.) as f32, 15.);
        let body = self.screen_header(rect, &title, p);
        let left_width = (body.width as f32 * 0.28).clamp(self.s(220.) as f32, self.s(340.) as f32) as u32;
        v.split = body.x + left_width as i32;
        let list = Rect { width: left_width, ..body };
        self.renderer.fill(Rect { x: list.x + list.width as i32 - 1, width: 1, ..list }, p.border);
        self.renderer.fill(Rect { width: list.width - 1, ..list }, p.surface);
        let summary = format!("{} changed · {} → {} words", v.items.len(), v.comparison.words_a, v.comparison.words_b);
        self.put(&summary, Rect { x: list.x + self.s(12.), y: list.y + self.s(6.), width: list.width - self.su(24.), height: self.su(24.) }, 11.5, p.muted, Face::Normal, Align::Left);
        let row_height = self.s(50.);
        let top = list.y + self.s(34.);
        let show_comment_entry = v.show_comments && !v.comment_lines.is_empty();
        let comments_row = if show_comment_entry { row_height } else { 0 };
        let visible = ((list.height as i32 - self.s(34.) - comments_row) / row_height).max(1) as usize;
        v.list_scroll = v.list_scroll.min(v.items.len().saturating_sub(visible));
        for (n, item) in v.items.iter().enumerate().skip(v.list_scroll).take(visible) {
            let row = Rect { x: list.x + self.s(6.), y: top + (n - v.list_scroll) as i32 * row_height, width: list.width - self.su(14.), height: row_height as u32 - 2 };
            if v.selected == Some(n) {
                self.renderer.fill_round(row, self.su(6.), selected_fill(p.surface, p));
            } else if self.hot(row) {
                self.renderer.fill_round(row, self.su(6.), hovered(p.surface, p));
            }
            let name = match &item.was {
                Some(was) => format!("{was} → {}", item.title),
                None => item.title.clone(),
            };
            let name = self.fit(&name, row.width as f32 - self.s(20.) as f32, 13.);
            self.put(&name, Rect { x: row.x + self.s(10.), y: row.y + self.s(3.), width: row.width - self.su(20.), height: self.su(24.) }, 13., p.text, Face::Normal, Align::Left);
            let (label, tint) = match item.status {
                diff::Status::Added => ("Added", 0x6cc27a),
                diff::Status::Removed => ("Deleted", 0xe0706c),
                diff::Status::Changed => ("Edited", p.accent),
                diff::Status::Unchanged => (if item.was.is_some() { "Renamed" } else { "Moved" }, p.muted),
            };
            self.put(&format!("{label} · {} → {} words", item.words_a, item.words_b), Rect { x: row.x + self.s(10.), y: row.y + self.s(24.), width: row.width - self.su(20.), height: self.su(20.) }, 11., tint, Face::Normal, Align::Left);
            self.hits.push((row, Hit::Do(Act::Screen(ScreenAct::Pick(n)))));
        }
        if show_comment_entry {
            let row = Rect { x: list.x + self.s(6.), y: list.y + list.height as i32 - row_height, width: list.width - self.su(14.), height: row_height as u32 - 4 };
            if v.selected.is_none() {
                self.renderer.fill_round(row, self.su(6.), selected_fill(p.surface, p));
            }
            self.put(&format!("Review comments ({})", v.comment_lines.len()), Rect { x: row.x + self.s(10.), width: row.width - self.su(20.), ..row }, 13., p.text, Face::Normal, Align::Left);
            self.hits.push((row, Hit::Do(Act::Screen(ScreenAct::Pick(usize::MAX)))));
        }
        // Options bar
        let main = Rect { x: list.x + list.width as i32, width: body.width - list.width, ..body };
        let bar = Rect { height: self.su(44.), ..main };
        let mut x = bar.x + self.s(14.);
        for (text, on, act) in [("Hide notes", v.hide_notes, ScreenAct::HideNotes), ("Show comment changes", v.show_comments, ScreenAct::ShowComments)] {
            let width = self.measure(text, 13. * self.scale, Face::Normal).ceil() as u32 + self.su(28.);
            self.button(text, Rect { x, y: bar.y + self.s(7.), width, height: self.su(30.) }, Hit::Do(Act::Screen(act)), if on { Look::Active } else { Look::Outline }, p);
            x += width as i32 + self.s(8.);
        }
        self.renderer.fill(Rect { y: bar.y + bar.height as i32 - 1, height: 1, ..bar }, p.border);
        let content = Rect { x: main.x + self.s(24.), y: bar.y + bar.height as i32 + self.s(10.), width: main.width.saturating_sub(self.su(48.)).min(self.su(960.)), height: main.height.saturating_sub(bar.height + self.su(20.)) };
        if v.selected.is_none() {
            let mut y = content.y;
            if v.comment_lines.is_empty() {
                self.put("The review comments are the same.", Rect { height: self.su(26.), ..content }, 12.5, p.muted, Face::Normal, Align::Left);
            }
            for (sign, line) in v.comment_lines.iter().skip(v.scroll) {
                if y > content.y + content.height as i32 - self.s(20.) {
                    break;
                }
                let tint = match sign {
                    '+' => 0x6cc27a,
                    '-' => 0xe0706c,
                    _ => p.accent,
                };
                self.put(&sign.to_string(), Rect { x: content.x, y, width: self.su(18.), height: self.su(22.) }, 13., tint, Face::Bold, Align::Left);
                let used = self.paragraph(line, Rect { x: content.x + self.s(20.), y, width: content.width - self.su(20.), height: content.height }, p.text, content);
                y += used + self.s(6.);
            }
            return;
        }
        if v.rows.is_empty() {
            self.put("No differences in the text.", Rect { height: self.su(26.), ..content }, 12.5, p.muted, Face::Normal, Align::Left);
        }
        let mut y = content.y;
        let limit = content.y + content.height as i32;
        for n in v.scroll..v.rows.len() {
            if y >= limit {
                break;
            }
            let row = v.rows[n].clone();
            y += self.paint_diff_row(&row, content.x, y, content.width, Rect { height: (limit - content.y) as u32, ..content }, p);
        }
        let _ = &v.show_comments;
    }
    /// Wrapped text; returns the height it used.
    fn paragraph(&mut self, text: &str, area: Rect, tint: u32, clip: Rect) -> i32 {
        let size = 14. * self.scale;
        let font = self.settings.editor_font.clone();
        let key = format!("para\0{text}\0{}\0{tint}\0{font}", area.width);
        let buffer = self.labels.entry(key).or_insert_with(|| {
            let mut buffer = Buffer::new_empty(Metrics::new(size, size * 1.5));
            buffer.set_size(Some(area.width as f32), None);
            buffer.set_text(text, &Attrs::new().family(Family::Name(&font)).color(color(tint)), Shaping::Advanced, None);
            buffer
        });
        buffer.shape_until_scroll(&mut self.fonts, false);
        let lines = buffer.layout_runs().count().max(1) as i32;
        self.renderer.text(&mut self.fonts, buffer, area.x, area.y, clip);
        lines * (size * 1.5).ceil() as i32
    }
    /// One row of a text comparison; returns its height.
    fn paint_diff_row(&mut self, row: &diff::Row, x: i32, y: i32, width: u32, clip: Rect, p: Palette) -> i32 {
        let size = 14. * self.scale;
        let line_height = (size * 1.5).ceil() as i32;
        if let diff::RowKind::Skip(n) = row.kind {
            let area = Rect { x, y, width, height: line_height as u32 };
            self.put(&format!("⋯ {n} unchanged line{}", if n == 1 { "" } else { "s" }), area, 12., p.muted, Face::Normal, Align::Center);
            return line_height + self.s(4.);
        }
        let (fill, tint, sign) = match row.kind {
            diff::RowKind::Delete => (mix(p.background, if p.light { 0xd0605e } else { 0xc4504a }, 0.16), if p.light { 0xa8322e } else { 0xe8837f }, "−"),
            diff::RowKind::Insert => (mix(p.background, if p.light { 0x4f9a5a } else { 0x5fb06a }, 0.16), if p.light { 0x2f7a3a } else { 0x78cc86 }, "+"),
            _ => (p.background, p.text, ""),
        };
        let text_x = x + self.s(26.);
        let text_width = width.saturating_sub(self.su(34.));
        let font = self.settings.editor_font.clone();
        let mut signature = String::new();
        for s in &row.segments {
            signature.push(if s.changed { '\u{1}' } else { '\u{2}' });
            signature.push_str(&s.text);
        }
        let key = format!("diff\0{signature}\0{text_width}\0{}\0{font}", p.light);
        if !self.labels.contains_key(&key) {
            let mut buffer = Buffer::new_empty(Metrics::new(size, size * 1.5));
            buffer.set_size(Some(text_width as f32), None);
            let base = Attrs::new().family(Family::Name(&font)).color(color(p.text));
            let strong = Attrs::new().family(Family::Name(&font)).color(color(tint)).weight(cosmic_text::Weight::BOLD);
            let plain = Attrs::new().family(Family::Name(&font)).color(color(if row.kind == diff::RowKind::Equal { p.text } else { mix(p.text, tint, 0.35) }));
            let spans: Vec<(&str, Attrs)> = row.segments.iter().map(|s| (s.text.as_str(), if s.changed { strong.clone() } else { plain.clone() })).collect();
            buffer.set_rich_text(spans, &base, Shaping::Advanced, None);
            self.labels.insert(key.clone(), buffer);
        }
        let lines = match self.labels.get_mut(&key) {
            Some(buffer) => {
                buffer.shape_until_scroll(&mut self.fonts, false);
                buffer.layout_runs().count().max(1) as i32
            }
            None => 1,
        };
        let height = lines * line_height + self.s(6.);
        if row.kind != diff::RowKind::Equal {
            self.renderer.fill_round(Rect { x, y, width, height: height as u32 }, self.su(4.), fill);
            self.put(sign, Rect { x: x + self.s(6.), y, width: self.su(16.), height: line_height as u32 + self.su(6.) }, 14., tint, Face::Bold, Align::Left);
        }
        let text_y = y + self.s(3.);
        if let Some(buffer) = self.labels.get_mut(&key) {
            self.renderer.text(&mut self.fonts, buffer, text_x, text_y, clip);
        }
        height + self.s(2.)
    }

    fn paint_preview(&mut self, rect: Rect, v: &mut PreviewView, p: Palette) {
        let title = format!("Preview — {}", v.request.preset.name);
        let body = self.screen_header(rect, &title, p);
        // Buttons at the right of the header
        let y = rect.y + self.s(7.);
        let mut right = rect.x + rect.width as i32 - self.s(12.);
        for (text, act, look) in [("Export…", ScreenAct::ExportNow, Look::Primary), ("Fit", ScreenAct::ZoomFit, Look::Outline), ("+", ScreenAct::ZoomIn, Look::Outline), ("−", ScreenAct::ZoomOut, Look::Outline)] {
            let width = (self.measure(text, 13. * self.scale, Face::Normal).ceil() as u32 + self.su(24.)).max(self.su(36.));
            right -= width as i32;
            self.button(text, Rect { x: right, y, width, height: self.su(32.) }, Hit::Do(Act::Screen(act)), look, p);
            right -= self.s(6.);
        }
        self.renderer.fill(body, mix(p.background, p.text, if p.light { 0.07 } else { 0.03 }));
        let Some(typeset) = v.typeset.clone() else {
            let message = match &v.error {
                Some(error) => format!("The preview could not be made: {error}"),
                None => "Typesetting the book…".to_owned(),
            };
            self.put(&message, Rect { x: body.x + self.s(24.), y: body.y + self.s(24.), width: body.width - self.su(48.), height: self.su(30.) }, 13., if v.error.is_some() { 0xd0605e } else { p.muted }, Face::Normal, Align::Left);
            return;
        };
        let pages = typeset.pages();
        if pages == 0 {
            self.put("The book has no pages.", Rect { x: body.x + self.s(24.), y: body.y + self.s(24.), width: body.width - self.su(48.), height: self.su(30.) }, 13., p.muted, Face::Normal, Align::Left);
            return;
        }
        // Rows of pages: a lone first page then facing pairs for books, one page per row otherwise.
        let (page_w, page_h) = typeset.size(0).unwrap_or((612., 792.));
        let gap_pt = 18.;
        let rows: Vec<[Option<usize>; 2]> = if v.spreads {
            let mut rows = vec![[None, Some(0)]];
            let mut i = 1;
            while i < pages {
                rows.push([Some(i), if i + 1 < pages { Some(i + 1) } else { None }]);
                i += 2;
            }
            rows
        } else {
            (0..pages).map(|i| [Some(i), None]).collect()
        };
        let row_pt_width = if v.spreads { page_w * 2. + 2. } else { page_w };
        let margin = self.s(32.) as f32;
        let fit = ((body.width as f32 - 2. * margin) / row_pt_width).min((body.height as f32 - 2. * margin) / (page_h + gap_pt)).max(0.05);
        let k = fit * v.zoom;
        let row_height_pt = page_h + gap_pt;
        v.total = rows.len() as f32 * row_height_pt;
        v.viewport = body.height as f32 / k;
        v.scroll = v.scroll.clamp(0., (v.total - v.viewport).max(0.));
        let q = ((k * 20.).round() as u32).max(2);
        let first_row = (v.scroll / row_height_pt) as usize;
        v.first_visible = rows.get(first_row).and_then(|r| r[0].or(r[1])).unwrap_or(0);
        let clip = body;
        let mut row_index = first_row;
        loop {
            let Some(row) = rows.get(row_index) else { break };
            let top = body.y + ((row_index as f32 * row_height_pt - v.scroll) * k) as i32 + (gap_pt * k / 2.) as i32;
            if top > body.y + body.height as i32 {
                break;
            }
            let row_width_px = (row_pt_width * k) as i32;
            let left = body.x + (body.width as i32 - row_width_px) / 2;
            for (slot, page) in row.iter().enumerate() {
                let Some(page) = page else { continue };
                let x = left + (slot as f32 * (page_w + 2.) * k) as i32;
                let dest = Rect { x, y: top, width: (page_w * k) as u32, height: (page_h * k) as u32 };
                self.renderer.shade(Rect { x: dest.x + 3, y: dest.y + 4, ..dest }, 0x000000, if p.light { 40 } else { 90 });
                self.renderer.fill(dest, 0xffffff);
                match v.cache.get(&(*page, q)) {
                    Some(bitmap) => self.renderer.image(dest, &bitmap.pixels, bitmap.width, bitmap.height, clip),
                    None => {
                        // Show a stale rendering at another zoom until the right one arrives.
                        let stale = v.cache.iter().find(|((pg, _), _)| pg == page).map(|(_, b)| b.clone());
                        if let Some(bitmap) = stale {
                            self.renderer.image(dest, &bitmap.pixels, bitmap.width, bitmap.height, clip);
                        }
                        if v.requested.insert((*page, q)) {
                            if let Some(worker) = &v.worker {
                                let _ = worker.send((*page, q));
                            }
                        }
                    }
                }
            }
            row_index += 1;
        }
        // Page indicator
        let last = (v.scroll + v.viewport / 2.) / row_height_pt;
        let shown = rows.get((last as usize).min(rows.len() - 1)).and_then(|r| r[0].or(r[1])).unwrap_or(0);
        let note = if v.warnings.is_empty() { String::new() } else { format!("  ·  {}", v.warnings[0]) };
        let label = format!("Page {} of {pages}{note}", shown + 1);
        let width = self.measure(&label, 12. * self.scale, Face::Normal).ceil() as u32 + self.su(24.);
        let chip = Rect { x: body.x + (body.width as i32 - width as i32) / 2, y: body.y + body.height as i32 - self.s(36.), width, height: self.su(26.) };
        self.renderer.shade(chip, 0x000000, 150);
        self.put(&label, chip, 12., 0xffffff, Face::Normal, Align::Center);
    }
}

/// Differences between the review comments of two states, as lines with a `+`, `-` or `~` sign.
fn comment_changes(work: &Option<jw::workdir::WorkDir>, a: &State, b: &State) -> Vec<(char, String)> {
    let Some(work) = work else { return Vec::new() };
    let load = |s: &State| s.comments_json(work).map(|bytes| CommentSet::parse(&bytes)).unwrap_or_default();
    let (old, new) = (load(a), load(b));
    let mut out = Vec::new();
    let describe = |c: &crate::core::comments::Comment| {
        let tag = if c.tag.is_empty() { String::new() } else { format!("[{}] ", c.tag) };
        format!("{tag}{} — on “{}”", c.body.replace('\n', " "), c.quote.chars().take(60).collect::<String>().replace('\n', " "))
    };
    for c in &new.comments {
        match old.get(&c.id) {
            None => out.push(('+', describe(c))),
            Some(was) => {
                if was.status != c.status {
                    out.push(('~', format!("{} — now {:?}", describe(c), c.status)));
                } else if was.body != c.body || was.replies.len() != c.replies.len() {
                    out.push(('~', format!("{} — edited or answered", describe(c))));
                }
            }
        }
    }
    for c in &old.comments {
        if new.get(&c.id).is_none() {
            out.push(('-', describe(c)));
        }
    }
    out
}

#[allow(dead_code)]
fn _unused(_: &history::History) {}
