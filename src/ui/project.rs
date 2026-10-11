//! The open project: its storage adapter, its state in the application, tree operations, and the
//! anchors of review comments inside open tabs.
use super::*;
use crate::{
    core::{
        comments::{self, CommentSet, EditRec, Status},
        notes::{self, Marker},
    },
    export::preset::Presets,
    history::{self, History},
    jw::{
        self, Project,
        manifest::{self, COMMENTS, Node, PRESETS, Section},
        project::{OpenError, Opened},
    },
};
use std::{collections::BTreeSet, io, sync::RwLock};

/// Chapters are stored with notes in a neutral form; the editor shows them with the author's
/// marker. This converts on the way in and out.
pub struct ProjectStorage {
    work: jw::workdir::WorkDir,
    marker: Arc<RwLock<Marker>>,
    saved: Arc<dyn Fn(String, usize) + Send + Sync>,
}
impl ProjectStorage {
    pub fn new(work: jw::workdir::WorkDir, marker: Arc<RwLock<Marker>>, saved: Arc<dyn Fn(String, usize) + Send + Sync>) -> Self {
        Self { work, marker, saved }
    }
    fn relative(path: &Path) -> io::Result<String> {
        let text = path.to_string_lossy().replace('\\', "/");
        if jw::workdir::valid(&text) {
            Ok(text)
        } else {
            Err(io::Error::new(io::ErrorKind::InvalidInput, "Not a chapter of this project"))
        }
    }
}
fn unsupported() -> io::Error {
    io::Error::new(io::ErrorKind::Unsupported, "Chapters are managed through the project tree")
}
impl Storage for ProjectStorage {
    fn read(&self, path: &Path, sink: ReadSink) {
        let (work, marker) = (self.work.clone(), self.marker.clone());
        let path = path.to_path_buf();
        std::thread::spawn(move || {
            let result = Self::relative(&path).and_then(|rel| work.read_string(&rel));
            match result {
                Ok(neutral) => {
                    let marker = marker.read().map(|m| m.clone()).unwrap_or_default();
                    let (display, _) = notes::to_display(&neutral, &marker);
                    // Cut at line ends, so the first screen can show before the rest arrives.
                    let mut rest = display.as_str();
                    let mut size = 16 * 1024;
                    while !rest.is_empty() {
                        let mut cut = rest.len().min(size);
                        while !rest.is_char_boundary(cut) {
                            cut += 1;
                        }
                        if cut < rest.len() {
                            cut = rest[cut..].find('\n').map_or(rest.len(), |n| cut + n + 1);
                        }
                        sink(ReadEvent::Chunk(Rope::from_str(&rest[..cut])));
                        rest = &rest[cut..];
                        size = 1024 * 1024;
                    }
                    sink(ReadEvent::Complete);
                }
                Err(error) => sink(ReadEvent::Error(error)),
            }
        });
    }
    fn write(&self, path: &Path, text: &Rope) -> io::Result<()> {
        let rel = Self::relative(path)?;
        let marker = self.marker.read().map(|m| m.clone()).unwrap_or_default();
        let (neutral, _) = notes::to_neutral(&text.to_string(), &marker);
        let words = notes::word_count(&neutral);
        self.work.write(&rel, neutral.as_bytes())?;
        (self.saved)(rel, words);
        Ok(())
    }
    fn list(&self, _: &Path) -> io::Result<Vec<Entry>> {
        Ok(Vec::new())
    }
    fn watch(&self, _: Arc<dyn Fn(PathBuf) + Send + Sync>) -> io::Result<notify::RecommendedWatcher> {
        Err(unsupported())
    }
    fn rename(&self, _: &Path, _: &Path) -> io::Result<()> {
        Err(unsupported())
    }
    fn remove(&self, _: &Path) -> io::Result<()> {
        Err(unsupported())
    }
    fn create(&self, _: &Path, _: bool) -> io::Result<()> {
        Err(unsupported())
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Panel {
    Scenes,
    Comments,
}
#[derive(Clone, Default)]
pub struct CommentFilter {
    pub status: Option<Status>,
    pub tag: Option<String>,
    /// Only the chapter in the active tab.
    pub this_chapter: bool,
}

pub struct ProjectState {
    pub core: Project,
    pub history: History,
    pub comments: CommentSet,
    pub presets: Presets,
    pub marker: Arc<RwLock<Marker>>,
    /// The row last clicked; the tree keys and single-item commands act on it.
    pub selected: Option<String>,
    /// Rows marked with Ctrl/Shift click. Empty means only `selected` is marked.
    pub selection: Vec<String>,
    /// Where a Shift range starts.
    pub anchor: Option<String>,
    pub collapsed: BTreeSet<String>,
    pub filter: CommentFilter,
    pub active_comment: Option<String>,
    pub last_pack: Instant,
    pub last_beat: Instant,
    pub last_snapshot: Instant,
    /// Text was saved since the last snapshot.
    pub changed: bool,
    pub packing: bool,
    pub panel: Panel,
    /// The `.jw` changed outside the program; packing waits for the author's decision.
    pub blocked: bool,
    /// The project has a file in the cloud folder (known after the first look at the cloud).
    pub cloud_linked: bool,
    /// The last chapter save that is not packed yet.
    pub last_save: Option<Instant>,
}

/// A comment's range in an open tab, in editor coordinates.
#[derive(Clone, Debug, PartialEq)]
pub struct TabMark {
    pub id: String,
    pub start: usize,
    pub end: usize,
    pub orphaned: bool,
}

pub fn project_error(error: OpenError) -> String {
    match error {
        OpenError::Invalid(message) => message,
        OpenError::Locked(c) => c.message(jw::time::now()),
    }
}

impl App {
    /// Identifies the chapter or page shown in a tab.
    pub(super) fn item_of_tab(&self, tab: u64) -> Option<String> {
        let path = self.tabs.get(&tab)?.path.as_ref()?.to_string_lossy().replace('\\', "/");
        self.project.as_ref()?.core.id_of_file(&path)
    }
    pub(super) fn active_item(&self) -> Option<String> {
        self.item_of_tab(self.active_id()?)
    }
    pub(super) fn marker(&self) -> Marker {
        self.settings.marker()
    }
    pub(super) fn scene_symbol(&self) -> String {
        self.project.as_ref().map_or("*  *  *".into(), |p| p.core.manifest.settings.scene_break.clone())
    }

    // ---- opening and closing -----------------------------------------------------------

    pub(super) fn open_project(&mut self, path: PathBuf, force: bool) {
        let proxy = self.proxy.clone();
        let data = platform::data_directory();
        self.status = format!("Opening {}…", path.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned()));
        std::thread::spawn(move || {
            let result = Project::open(&data, &path, force);
            let _ = proxy.send_event(UserEvent::ProjectOpened(path, match result {
                Ok(opened) => Ok(Box::new(opened)),
                Err(OpenError::Locked(conflict)) => Err(Err(conflict)),
                Err(OpenError::Invalid(message)) => Err(Ok(message)),
            }));
        });
    }
    pub(super) fn create_project(&mut self, path: PathBuf, title: String) {
        let proxy = self.proxy.clone();
        let data = platform::data_directory();
        std::thread::spawn(move || {
            let result = Project::create(&data, &path, &title);
            let _ = proxy.send_event(UserEvent::ProjectOpened(path, match result {
                Ok(project) => Ok(Box::new(Opened { project, recovered: None, duplicate: false })),
                Err(message) => Err(Ok(message)),
            }));
        });
    }
    /// Makes an opened project the one in the window.
    pub(super) fn attach_project(&mut self, opened: Opened, path: PathBuf) {
        self.shutdown_project();
        self.clear_tabs();
        let Opened { project, recovered, duplicate } = opened;
        let marker = Arc::new(RwLock::new(self.marker()));
        let storage = self.make_storage(&project, &marker);
        let comments = project.work.read(COMMENTS).map(|b| CommentSet::parse(&b)).unwrap_or_default();
        let presets = project.work.read(PRESETS).map(|b| Presets::parse(&b)).unwrap_or_default();
        let history = History::load(&project.work);
        self.root = Some(project.work.root().to_path_buf());
        self.storage = Some(storage);
        self.recent.retain(|r| r != &path);
        self.recent.insert(0, path.clone());
        self.recent.truncate(10);
        let now = Instant::now();
        self.project = Some(ProjectState {
            core: project,
            history,
            comments,
            presets,
            marker,
            selected: None,
            selection: Vec::new(),
            anchor: None,
            collapsed: BTreeSet::new(),
            filter: CommentFilter::default(),
            active_comment: None,
            last_pack: now,
            last_beat: now,
            last_snapshot: now,
            changed: false,
            packing: false,
            panel: Panel::Scenes,
            blocked: false,
            cloud_linked: false,
            last_save: None,
        });
        self.dirty_session();
        let mut note = String::new();
        if let Some(r) = recovered {
            note = format!("Recovered {} unsaved file{} from an earlier session.", r.unsaved.len(), if r.unsaved.len() == 1 { "" } else { "s" });
            if r.also_changed_outside {
                note.push_str(" The .jw file also changed outside JustWrite; saving will ask what to keep.");
            }
        }
        if duplicate {
            note = "This file is a copy of another project; it now has its own identity.".into();
        }
        self.status = if note.is_empty() { format!("Opened {}", path.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned())) } else { note };
        // Open the first chapter so the window is not empty.
        let first = self
            .project
            .as_ref()
            .and_then(|p| p.core.manifest.tree.leaves(Section::Manuscript).first().map(|(n, _)| n.id().to_owned()));
        if let Some(id) = first {
            self.open_item(&id);
        }
        self.title();
        self.redraw();
    }
    /// Writes everything out and lets go of the project: final snapshot, repack, lock.
    pub(super) fn shutdown_project(&mut self) {
        self.sync_stale_marks();
        let Some(mut state) = self.project.take() else { return };
        self.root = None;
        self.storage = None;
        let now = jw::time::now();
        let policy = state.core.manifest.settings.snapshots.clone();
        if policy.on_close && state.changed {
            let _ = state.history.capture_auto(&state.core, "Closed", now);
        }
        let _ = history::prune::prune(&mut state.history, &state.core, now);
        let linked = state.cloud_linked.then(|| state.core.path.clone());
        match state.core.close() {
            Ok(()) => {
                if let Some(jw) = linked {
                    self.cloud_flush(jw);
                }
            }
            Err(error) => self.fail(format!("Could not save the project file: {error}")),
        }
    }

    // ---- items -------------------------------------------------------------------------

    pub(super) fn open_item(&mut self, id: &str) {
        let Some(file) = self.project.as_ref().and_then(|p| p.core.file_of(id)) else { return };
        if let Some(p) = &mut self.project {
            p.selected = Some(id.to_owned());
            p.anchor = p.selected.clone();
            p.selection.clear();
        }
        self.tree_reveal = true;
        self.open_relative(PathBuf::from(file), 0);
    }
    /// Opens the chapter or page after (or before) the one being written, in the order of the tree.
    pub(super) fn step_chapter(&mut self, delta: i32) {
        let Some(state) = &self.project else { return };
        let tree = &state.core.manifest.tree;
        let order: Vec<String> = [Section::Manuscript, Section::Notes]
            .into_iter()
            .flat_map(|s| tree.leaves(s).into_iter().map(|(n, _)| n.id().to_owned()).collect::<Vec<_>>())
            .collect();
        let here = self.active_item().or_else(|| state.selected.clone()).and_then(|id| order.iter().position(|o| *o == id));
        let target = match here {
            Some(i) => i as i64 + i64::from(delta),
            None if delta > 0 => 0,
            None => order.len() as i64 - 1,
        };
        match usize::try_from(target).ok().and_then(|i| order.get(i)).cloned() {
            Some(id) => self.open_item(&id),
            None => self.status = if delta > 0 { "That is the last chapter." } else { "That is the first chapter." }.into(),
        }
    }
    pub(super) fn node_title(&self, id: &str) -> Option<String> {
        self.project.as_ref()?.core.manifest.tree.find(id).map(|n| n.title().to_owned())
    }
    /// Keeps tab titles in step with the tree.
    pub(super) fn refresh_titles(&mut self) {
        let Some(project) = &self.project else { return };
        for tab in self.tabs.values_mut() {
            tab.title = tab
                .path
                .as_ref()
                .and_then(|p| project.core.id_of_file(&p.to_string_lossy().replace('\\', "/")))
                .and_then(|id| project.core.manifest.tree.find(&id).map(|n| n.title().to_owned()));
        }
    }

    /// `title` names a new note; chapters are numbered.
    pub(super) fn new_node(&mut self, kind: &str, title: &str) {
        let Some(state) = &mut self.project else {
            self.status = "Open or create a project first.".into();
            return;
        };
        let tree = &state.core.manifest.tree;
        let (section, index, node) = match kind {
            "page" => {
                let title = if title.trim().is_empty() { Self::next_note_title(tree) } else { title.trim().to_owned() };
                (Section::Notes, tree.notes.len(), Node::page(&title))
            }
            _ => {
                let title = tree.next_chapter_title();
                (Section::Manuscript, tree.manuscript.len(), Node::chapter(&title))
            }
        };
        let id = node.id().to_owned();
        match state.core.add(section, &[], index, node) {
            Ok(_) => {
                state.selected = Some(id.clone());
                state.changed = true;
                self.open_item(&id);
                self.dirty_session();
            }
            Err(error) => self.status = error,
        }
        self.redraw();
    }
    /// "Note 1", "Note 2", …: the first number no note in the tree already uses.
    pub(super) fn next_note_title(tree: &manifest::Tree) -> String {
        let used: Vec<String> = tree.leaves(Section::Notes).iter().map(|(n, _)| n.title().to_owned()).collect();
        (tree.leaves(Section::Notes).len() + 1..).map(|n| format!("Note {n}")).find(|t| !used.contains(t)).unwrap()
    }
    pub(super) fn rename_node(&mut self, id: &str, title: &str) {
        let Some(state) = &mut self.project else { return };
        match state.core.rename(id, title) {
            Ok(()) => {
                self.refresh_titles();
                self.title();
            }
            Err(error) => self.note(&error),
        }
    }
    /// Deletes chapters, parts or notes after ONE safety snapshot that still holds them.
    pub(super) fn delete_nodes(&mut self, ids: &[String]) {
        let Some(state) = &mut self.project else { return };
        let tree = &state.core.manifest.tree;
        let ids: Vec<String> = ids.iter().filter(|id| tree.find(id).is_some()).cloned().collect();
        if ids.is_empty() {
            return;
        }
        let title = tree.find(&ids[0]).map(|n| n.title().to_owned()).unwrap_or_default();
        let label = if ids.len() == 1 { format!("\"{title}\"") } else { noun(tree, &ids) };
        let mut files = Vec::new();
        fn collect(n: &Node, out: &mut Vec<String>) {
            out.extend(n.file());
            n.children().iter().for_each(|c| collect(c, out));
        }
        for id in &ids {
            if let Some(node) = tree.find(id) {
                collect(node, &mut files);
            }
        }
        let now = jw::time::now();
        if let Err(error) = state.history.capture(&state.core, history::Kind::Safety, &format!("Before deleting {label}"), now) {
            self.status = format!("Not deleted: the safety snapshot failed ({error})");
            return;
        }
        let mut failed = None;
        for id in &ids {
            // A part deleted earlier already took its chapters.
            if state.core.manifest.tree.find(id).is_none() {
                continue;
            }
            if let Err(error) = state.core.delete(id) {
                failed = Some(error);
                break;
            }
        }
        state.selection.clear();
        if state.selected.as_ref().is_some_and(|s| state.core.manifest.tree.find(s).is_none()) {
            state.selected = None;
        }
        state.changed = true;
        let doomed: Vec<u64> = self
            .tabs
            .iter()
            .filter(|(_, t)| t.path.as_ref().is_some_and(|p| files.contains(&p.to_string_lossy().replace('\\', "/"))))
            .map(|(id, _)| *id)
            .collect();
        for tab in doomed {
            self.close_tab(tab, true);
        }
        self.status = match failed {
            Some(error) => error,
            None => format!("Deleted {label}. A snapshot before the deletion keeps the text."),
        };
        self.redraw();
    }
    /// The items the tree commands act on: the marked rows in tree order (an item inside a marked
    /// part is left out), else the selected or active item.
    pub(super) fn marked_ids(&self) -> Vec<String> {
        let Some(state) = &self.project else { return Vec::new() };
        let tree = &state.core.manifest.tree;
        let ids: Vec<String> = if state.selection.is_empty() { self.selected_item().into_iter().collect() } else { state.selection.clone() };
        let mut located: Vec<(bool, Vec<usize>, String)> = ids
            .into_iter()
            .filter_map(|id| tree.locate(&id).map(|(s, path)| (s == Section::Notes, path, id)))
            .collect();
        located.sort();
        located.dedup();
        let keep: Vec<bool> = located
            .iter()
            .map(|(notes, path, _)| !located.iter().any(|(n, p, _)| n == notes && p.len() < path.len() && path[..p.len()] == p[..]))
            .collect();
        located.into_iter().zip(keep).filter(|(_, k)| *k).map(|((_, _, id), _)| id).collect()
    }
    /// Ids of the rows shown in the tree, top to bottom (no headings, no hidden children).
    pub(super) fn visible_items(&self) -> Vec<String> {
        self.tree_rows().into_iter().filter(|r| r.heading.is_none()).map(|r| r.id).collect()
    }
    /// Ctrl toggles `id`, Shift selects from the anchor to `id` (Ctrl+Shift adds the range).
    pub(super) fn extend_selection(&mut self, id: &str, ctrl: bool, shift: bool) {
        let visible = self.visible_items();
        let Some(state) = &mut self.project else { return };
        if state.selection.is_empty() {
            state.selection.extend(state.selected.clone());
        }
        let tree = &state.core.manifest.tree;
        let section = |i: &str| tree.locate(i).map(|(s, _)| s);
        let anchor = state.anchor.clone().or_else(|| state.selected.clone()).unwrap_or_else(|| id.to_owned());
        if shift {
            let ids = if section(&anchor) == section(id) { range_between(&visible, &anchor, id) } else { vec![id.to_owned()] };
            if !ctrl {
                state.selection.clear();
            }
            for i in ids {
                if !state.selection.contains(&i) {
                    state.selection.push(i);
                }
            }
        } else {
            if let Some(at) = state.selection.iter().position(|s| s == id) {
                state.selection.remove(at);
            } else {
                state.selection.push(id.to_owned());
            }
            state.anchor = Some(id.to_owned());
        }
        state.selected = Some(id.to_owned());
        self.redraw();
    }
    /// Plain click on a tree row: select it and open it (a part folds instead).
    pub(super) fn click_node(&mut self, id: &str) {
        let is_part = self.project.as_ref().and_then(|p| p.core.manifest.tree.find(id)).is_some_and(Node::is_part);
        if let Some(state) = &mut self.project {
            state.selected = Some(id.to_owned());
            state.anchor = state.selected.clone();
            state.selection.clear();
        }
        if is_part {
            self.toggle_fold(id);
        } else {
            self.open_item(id);
        }
        // Opening a chapter makes a tab, which takes the keys; the tree click keeps them.
        self.focus = Focus::Tree;
    }
    /// Keys while the tree owns the keyboard. Returns whether the key was used.
    pub(super) fn tree_key(&mut self, key: &Key, primary: bool, shift: bool) -> bool {
        if self.project.is_none() || self.marked_ids().is_empty() {
            return false;
        }
        match key {
            Key::Named(NamedKey::Delete) => self.command(Command::Delete),
            Key::Named(NamedKey::Enter) => {
                if let Some(id) = self.selected_item() {
                    self.click_node(&id);
                }
            }
            Key::Named(step @ (NamedKey::ArrowUp | NamedKey::ArrowDown)) if !primary => {
                let visible = self.visible_items();
                let Some(state) = &mut self.project else { return false };
                let here = state.selected.as_ref().and_then(|s| visible.iter().position(|v| v == s));
                let next = match (here, matches!(step, NamedKey::ArrowDown)) {
                    (Some(i), true) => (i + 1).min(visible.len().saturating_sub(1)),
                    (Some(i), false) => i.saturating_sub(1),
                    (None, _) => 0,
                };
                let Some(id) = visible.get(next).cloned() else { return false };
                if shift {
                    self.extend_selection(&id, false, true);
                } else {
                    state.selection.clear();
                    state.selected = Some(id.clone());
                    state.anchor = Some(id);
                }
                self.tree_reveal = true;
            }
            Key::Character(c) if primary && !shift && c.eq_ignore_ascii_case("a") => {
                let Some(state) = &mut self.project else { return false };
                let Some(section) = state.selected.as_ref().and_then(|s| state.core.manifest.tree.locate(s)).map(|(s, _)| s) else { return false };
                let tree = &state.core.manifest.tree;
                let mut all: Vec<String> = Vec::new();
                fn walk(nodes: &[Node], out: &mut Vec<String>) {
                    for n in nodes {
                        out.push(n.id().to_owned());
                        walk(n.children(), out);
                    }
                }
                walk(tree.section(section), &mut all);
                state.selection = all;
            }
            _ => return false,
        }
        self.redraw();
        true
    }
    /// Moves the marked items up or down, keeping their order. Stops at the first one that cannot move.
    pub(super) fn move_selected(&mut self, down: bool) {
        let mut ids = self.marked_ids();
        if down {
            ids.reverse();
        }
        for id in ids {
            if !self.move_item(&id, down) {
                break;
            }
        }
    }
    /// Moves one item up or down among its siblings (or across a part boundary).
    fn move_item(&mut self, id: &str, down: bool) -> bool {
        let Some(state) = &mut self.project else { return false };
        let Some((section, path)) = state.core.manifest.tree.locate(id) else { return false };
        let tree = &state.core.manifest.tree;
        let (parent, at) = (path[..path.len() - 1].to_vec(), *path.last().unwrap());
        let siblings = if parent.is_empty() { tree.section(section).len() } else { tree.get(section, &parent).map_or(0, |n| n.children().len()) };
        let (to_parent, to_index) = if down {
            if at + 1 < siblings {
                // After the next sibling, or into it when it is a part.
                match tree.get(section, &[parent.clone(), vec![at + 1]].concat()) {
                    Some(n) if n.is_part() && !tree.get(section, &path).is_some_and(Node::is_part) => ([parent.clone(), vec![at + 1]].concat(), 0),
                    _ => (parent.clone(), at + 2),
                }
            } else if !parent.is_empty() {
                // Out of the part, after it.
                (parent[..parent.len() - 1].to_vec(), parent[parent.len() - 1] + 1)
            } else {
                self.status = "Already the last one.".into();
                return false;
            }
        } else if at > 0 {
            match tree.get(section, &[parent.clone(), vec![at - 1]].concat()) {
                Some(n) if n.is_part() && !tree.get(section, &path).is_some_and(Node::is_part) => ([parent.clone(), vec![at - 1]].concat(), n.children().len()),
                _ => (parent.clone(), at - 1),
            }
        } else if !parent.is_empty() {
            (parent[..parent.len() - 1].to_vec(), parent[parent.len() - 1])
        } else {
            self.status = "Already the first one.".into();
            return false;
        };
        let moved = match state.core.move_node(section, &path, &to_parent, to_index) {
            Ok(()) => {
                state.changed = true;
                self.tree_reveal = true;
                self.dirty_session();
                true
            }
            Err(error) => {
                self.status = error;
                false
            }
        };
        self.redraw();
        moved
    }
    /// Drops `id` before `before` (or at the end of a section/part when `before` is `None`).
    pub(super) fn drop_node(&mut self, id: &str, target: DropTarget) {
        let Some(state) = &mut self.project else { return };
        let Some((section, from)) = state.core.manifest.tree.locate(id) else { return };
        let tree = &state.core.manifest.tree;
        let (to_section, parent, index) = match target {
            DropTarget::Before(other) => {
                let Some((s, p)) = tree.locate(&other) else { return };
                (s, p[..p.len() - 1].to_vec(), *p.last().unwrap())
            }
            DropTarget::Into(part) => {
                let Some((s, p)) = tree.locate(&part) else { return };
                let n = tree.get(s, &p).map_or(0, |n| n.children().len());
                (s, p, n)
            }
            DropTarget::End(s) => (s, Vec::new(), tree.section(s).len()),
        };
        if to_section != section {
            self.status = "Chapters stay in the manuscript and notes stay in Notes & research.".into();
            return;
        }
        match state.core.move_node(section, &from, &parent, index) {
            Ok(()) => {
                state.changed = true;
                self.dirty_session();
            }
            Err(error) => self.status = error,
        }
    }

    // ---- saving ------------------------------------------------------------------------

    /// A chapter was written; record its word count.
    pub(super) fn chapter_saved(&mut self, rel: &str, words: usize) {
        let Some(state) = &mut self.project else { return };
        if let Some(id) = state.core.id_of_file(rel) {
            let _ = state.core.set_words(&id, words as u32);
        }
        state.changed = true;
        state.last_save = Some(Instant::now());
    }
    pub(super) fn pack_project(&mut self, overwrite: bool) {
        let Some(state) = &mut self.project else { return };
        if state.packing {
            return;
        }
        state.packing = true;
        state.last_pack = Instant::now();
        state.last_save = None;
        let (work, path) = (state.core.work.clone(), state.core.path.clone());
        let proxy = self.proxy.clone();
        std::thread::spawn(move || {
            let result = work.pack(&path, overwrite);
            let _ = proxy.send_event(UserEvent::Packed(match result {
                Ok(()) => Ok(()),
                Err(jw::workdir::PackError::ExternalChange) => Err(None),
                Err(jw::workdir::PackError::Io(e)) => Err(Some(e.to_string())),
            }));
        });
    }
    pub(super) fn save_comments(&mut self) {
        if let Some(state) = &self.project {
            let _ = state.core.work.write(COMMENTS, state.comments.to_json().as_bytes());
        }
    }
    pub(super) fn save_presets(&mut self) {
        if let Some(state) = &self.project {
            let _ = state.core.work.write(PRESETS, state.presets.to_json().as_bytes());
        }
    }

    // ---- comment anchors in tabs -------------------------------------------------------

    /// Places the project's comments of this tab's chapter in the editor text.
    pub(super) fn rebuild_marks(&mut self, tab_id: u64) {
        let Some(item) = self.item_of_tab(tab_id) else { return };
        let marker = self.marker();
        let Some(state) = &mut self.project else { return };
        let Some(tab) = self.tabs.get_mut(&tab_id) else { return };
        if !state.comments.comments.iter().any(|c| c.item == item) {
            tab.marks.clear();
            tab.view.marks.clear();
            return;
        }
        let (neutral, forward) = notes::to_neutral(&tab.document.text.to_string(), &marker);
        let changed = state.comments.reanchor_item(&item, &neutral);
        let back = forward.inverse();
        tab.marks = state
            .comments
            .comments
            .iter()
            .filter(|c| c.item == item)
            .map(|c| TabMark { id: c.id.clone(), start: back.map(c.anchor.start), end: back.map(c.anchor.end), orphaned: c.orphaned })
            .collect();
        if changed > 0 {
            let _ = state.core.work.write(COMMENTS, state.comments.to_json().as_bytes());
        }
        self.sync_view_marks(tab_id);
    }
    pub(super) fn sync_view_marks(&mut self, tab_id: u64) {
        let active = self.project.as_ref().and_then(|p| p.active_comment.clone());
        let Some(tab) = self.tabs.get_mut(&tab_id) else { return };
        let mut marks: Vec<crate::ui::editor::Mark> = tab
            .marks
            .iter()
            .map(|m| crate::ui::editor::Mark { start: m.start, end: m.end, orphaned: m.orphaned, active: active.as_deref() == Some(m.id.as_str()) })
            .collect();
        marks.sort_by_key(|m| m.start);
        tab.view.marks = marks;
    }
    /// Moves comment anchors along with the edits just made in a tab.
    pub(super) fn follow_edits(&mut self, tab_id: u64) {
        let Some(tab) = self.tabs.get_mut(&tab_id) else { return };
        let edits: Vec<EditRec> = tab.document.take_edits();
        if edits.is_empty() || tab.marks.is_empty() {
            return;
        }
        let mut lost = false;
        for edit in edits {
            for m in tab.marks.iter_mut().filter(|m| !m.orphaned) {
                match comments::map_range(m.start..m.end, edit) {
                    Some(r) => (m.start, m.end) = (r.start, r.end),
                    None => {
                        m.orphaned = true;
                        m.start = edit.start.min(m.start);
                        m.end = m.start;
                        lost = true;
                    }
                }
            }
            // Positions of orphans keep following the text so they can be found again.
            for m in tab.marks.iter_mut().filter(|m| m.orphaned) {
                if edit.start <= m.start {
                    let shifted = (m.start as isize + edit.inserted as isize - edit.removed as isize).max(edit.start as isize) as usize;
                    (m.start, m.end) = (shifted, shifted);
                }
            }
        }
        let _ = lost;
        self.sync_view_marks(tab_id);
    }
    /// After undo or redo some orphaned comments may have their text back.
    pub(super) fn retry_orphans(&mut self, tab_id: u64) {
        if self.tabs.get(&tab_id).is_some_and(|t| t.marks.iter().any(|m| m.orphaned)) {
            self.rebuild_marks(tab_id);
        }
    }
    /// Writes the editor positions of a tab's comments back into the project, in stored form.
    pub(super) fn sync_marks_to_project(&mut self, tab_id: u64) {
        if let Some(tab) = self.tabs.get_mut(&tab_id) {
            tab.marks_stale = false;
        }
        let Some(item) = self.item_of_tab(tab_id) else { return };
        let marker = self.marker();
        let Some(tab) = self.tabs.get(&tab_id) else { return };
        if tab.marks.is_empty() {
            return;
        }
        let (_, forward) = notes::to_neutral(&tab.document.text.to_string(), &marker);
        let marks = tab.marks.clone();
        let Some(state) = &mut self.project else { return };
        let mut changed = false;
        for m in marks {
            if let Some(c) = state.comments.get_mut(&m.id).filter(|c| c.item == item) {
                let anchor = comments::Anchor { start: forward.map(m.start), end: forward.map(m.end) };
                if c.anchor != anchor || c.orphaned != m.orphaned {
                    c.anchor = anchor;
                    c.orphaned = m.orphaned;
                    changed = true;
                }
            }
        }
        if changed {
            let _ = state.core.work.write(COMMENTS, state.comments.to_json().as_bytes());
        }
    }

    /// Writes back the marks of every tab that autosave left alone. Cheap when there are none.
    pub(super) fn sync_stale_marks(&mut self) {
        let ids: Vec<u64> = self.tabs.iter().filter(|(_, t)| t.marks_stale).map(|(id, _)| *id).collect();
        for id in ids {
            self.sync_marks_to_project(id);
        }
    }

    /// Re-reads open chapters after the stored text changed under them (a restore, a marker change).
    pub(super) fn reload_tabs(&mut self) {
        let ids: Vec<u64> = self.tabs.iter().filter(|(_, t)| t.path.is_some() && t.root == self.root).map(|(id, _)| *id).collect();
        for id in ids {
            self.reload_tab(id);
        }
    }
    pub(super) fn reload_tab(&mut self, id: u64) {
        let (Some(storage), Some(tab)) = (self.storage.clone(), self.tabs.get_mut(&id)) else { return };
        let Some(path) = tab.path.clone() else { return };
        let cursor = tab.document.cursor;
        let old = std::mem::replace(&mut tab.document, crate::core::buffer::Document::default());
        std::thread::spawn(move || drop(old));
        tab.view.clear_cache();
        tab.loading = true;
        tab.failed = false;
        tab.disk_epoch += 1;
        tab.restore_cursor = Some(cursor);
        tab.edited = None;
        tab.marks.clear();
        let proxy = self.proxy.clone();
        storage.read(&path, Arc::new(move |event| {
            let _ = proxy.send_event(UserEvent::Read(id, event));
        }));
    }
}


impl App {
    fn make_storage(&self, project: &Project, marker: &Arc<RwLock<Marker>>) -> Arc<dyn Storage> {
        let proxy = self.proxy.clone();
        Arc::new(ProjectStorage::new(
            project.work.clone(),
            marker.clone(),
            Arc::new(move |rel, words| {
                let _ = proxy.send_event(UserEvent::ChapterSaved(rel, words));
            }),
        ))
    }
    /// A file the user picked in a dialog.
    pub(super) fn chosen(&mut self, path: PathBuf, choice: Choose) {
        match choice {
            Choose::OpenProject => self.open_project_checked(path),
            Choose::NewProject => {
                if let Some(imported) = self.pending_import.take() {
                    let proxy = self.proxy.clone();
                    let data = platform::data_directory();
                    self.status = "Creating the project…".into();
                    std::thread::spawn(move || {
                        let result = crate::import::into_new_project(&data, &path, &imported);
                        let _ = proxy.send_event(UserEvent::ProjectOpened(path, match result {
                            Ok(project) => Ok(Box::new(Opened { project, recovered: None, duplicate: false })),
                            Err(message) => Err(Ok(message)),
                        }));
                    });
                } else {
                    let title = path.file_stem().map_or_else(|| "My novel".to_owned(), |s| s.to_string_lossy().into_owned());
                    self.flush_tabs();
                    self.create_project(path, title);
                }
            }
            Choose::SaveProjectAs => self.save_project_as(path),
            Choose::ImportFile => {
                let proxy = self.proxy.clone();
                self.status = "Reading the file…".into();
                std::thread::spawn(move || {
                    let result = crate::import::read(&path).map(Box::new);
                    let _ = proxy.send_event(UserEvent::ImportRead(path, result));
                });
            }
            Choose::Cover => self.cover_chosen(path),
            Choose::Export => self.run_export(path),
            Choose::CloudSave => self.cloud_fetch(path),
        }
    }
    /// Opens a project, unless it is the one already open.
    pub(super) fn open_project_checked(&mut self, path: PathBuf) {
        let same = self.project.as_ref().is_some_and(|p| p.core.path == path || (p.core.path.canonicalize().ok().is_some() && p.core.path.canonicalize().ok() == path.canonicalize().ok()));
        if same {
            self.status = "That project is already open.".into();
            return;
        }
        self.flush_tabs();
        self.open_project(path, false);
    }
    pub(super) fn dropped(&mut self, path: PathBuf) {
        let extension = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
        match extension.as_str() {
            "jw" => self.open_project_checked(path),
            "docx" | "md" | "markdown" | "txt" => self.chosen(path, Choose::ImportFile),
            _ => self.status = "JustWrite opens .jw projects and imports .docx, .md and .txt files.".into(),
        }
    }
    pub(super) fn project_opened(&mut self, path: PathBuf, result: Result<Box<Opened>, Result<String, jw::lock::Conflict>>) {
        match result {
            Ok(opened) => {
                self.flush_tabs();
                self.attach_project(*opened, path);
                if let Some(split) = self.pending_session.take() {
                    self.tabs.clear();
                    self.split = self.restore_split(split);
                    self.active_pane = self.split.first_pane();
                    if self.tabs.values().all(|t| t.path.is_none()) {
                        let first = self.project.as_ref().and_then(|p| p.core.manifest.tree.leaves(Section::Manuscript).first().map(|(n, _)| n.id().to_owned()));
                        if let Some(id) = first {
                            self.open_item(&id);
                        }
                    }
                }
                self.cloud_sync();
            }
            Err(Ok(message)) => {
                self.pending_session = None;
                self.status = message;
            }
            Err(Err(conflict)) => {
                self.pending_session = None;
                self.locked_form(path, conflict);
            }
        }
        self.redraw();
    }
    pub(super) fn packed(&mut self, result: Result<(), Option<String>>) {
        let Some(state) = &mut self.project else { return };
        state.packing = false;
        match result {
            Ok(()) => {
                if self.status.is_empty() || self.status == "Saving…" || self.status == "Saved" {
                    self.status = "Saved".into();
                }
                self.cloud_sync();
            }
            Err(None) => {
                state.blocked = true;
                self.changed_outside_form();
            }
            Err(Some(error)) => self.fail(format!("Could not write the project file: {error}")),
        }
    }
    pub(super) fn save_project_as(&mut self, path: PathBuf) {
        self.flush_tabs();
        let Some(state) = &mut self.project else { return };
        match state.core.save_as(&path) {
            Ok(()) => {
                state.history = History::load(&state.core.work);
                state.last_pack = Instant::now();
                let storage = {
                    let marker = state.marker.clone();
                    let project = &self.project.as_ref().unwrap().core;
                    self.make_storage(project, &marker)
                };
                self.root = self.project.as_ref().map(|p| p.core.work.root().to_path_buf());
                self.storage = Some(storage.clone());
                for tab in self.tabs.values_mut() {
                    if tab.path.is_some() {
                        tab.storage = Some(storage.clone());
                        tab.root = self.root.clone();
                    }
                }
                self.recent.retain(|r| r != &path);
                self.recent.insert(0, path.clone());
                self.status = format!("Saved a copy; now working in {}", path.display());
            }
            Err(error) => self.status = format!("Could not save the copy: {error}"),
        }
    }
    pub(super) fn comment_filter(&mut self, key: &str) {
        let active = self.active_item();
        let Some(state) = &mut self.project else { return };
        match key {
            "status" => {
                state.filter.status = match state.filter.status {
                    None => Some(Status::Open),
                    Some(Status::Open) => Some(Status::Resolved),
                    Some(Status::Resolved) => None,
                };
            }
            "tag" => {
                let tags = state.comments.tags();
                state.filter.tag = match &state.filter.tag {
                    None => tags.first().cloned(),
                    Some(t) => tags.iter().position(|x| x == t).and_then(|i| tags.get(i + 1)).cloned(),
                };
            }
            "chapter" => {
                state.filter.this_chapter = !state.filter.this_chapter;
                let _ = active;
            }
            _ => {}
        }
        self.outline_top = 0;
    }
    pub(super) fn comment_action(&mut self, action: &str, id: &str) {
        match action {
            "resolve" | "reopen" => {
                if let Some(state) = &mut self.project {
                    state.comments.set_status(id, if action == "resolve" { Status::Resolved } else { Status::Open });
                }
                self.save_comments();
            }
            "reply" => self.reply_form(id),
            "edit" => self.edit_comment_form(id),
            "delete" => self.delete_comment_form(id),
            _ => {}
        }
    }
    /// The comment the caret is in, or else the one last opened from the panel.
    fn comment_at_caret(&self) -> Option<String> {
        let tab = self.active_id().and_then(|id| self.tabs.get(&id))?;
        let cursor = tab.document.cursor;
        tab.marks
            .iter()
            .filter(|m| !m.orphaned && m.start <= cursor && cursor <= m.end)
            .min_by_key(|m| m.end - m.start)
            .map(|m| m.id.clone())
            .or_else(|| self.project.as_ref()?.active_comment.clone().filter(|id| self.project.as_ref().is_some_and(|p| p.comments.get(id).is_some())))
    }
    /// Keyboard version of the comment cards: Go to the next or previous comment in reading order.
    pub(super) fn step_comment(&mut self, forward: bool) {
        let Some(state) = &self.project else { return };
        let items: Vec<String> = [Section::Manuscript, Section::Notes]
            .into_iter()
            .flat_map(|s| state.core.manifest.tree.leaves(s).into_iter().map(|(n, _)| n.id().to_owned()).collect::<Vec<_>>())
            .collect();
        let from = self.comment_at_caret();
        match state.comments.step(&items, from.as_deref(), forward).map(str::to_owned) {
            Some(id) => self.jump_to_comment(&id),
            None => self.status = "There are no review comments yet.".into(),
        }
    }
    /// Resolve, reply, edit or delete the comment at the caret.
    pub(super) fn comment_command(&mut self, action: &str) {
        let Some(id) = self.comment_at_caret() else {
            self.status = "Put the caret in a commented passage first, or press F8 to go to a comment.".into();
            return;
        };
        let action = match (action, self.project.as_ref().and_then(|p| p.comments.get(&id)).map(|c| c.status)) {
            ("resolve", Some(Status::Resolved)) => "reopen",
            (other, _) => other,
        };
        self.comment_action(action, &id);
    }
    pub(super) fn toggle_fold(&mut self, id: &str) {
        if let Some(state) = &mut self.project {
            if !state.collapsed.remove(id) {
                state.collapsed.insert(id.to_owned());
            }
        }
    }
    /// Shows a comment's text: opens its chapter and selects the commented words.
    pub(super) fn jump_to_comment(&mut self, id: &str) {
        let Some((item, orphaned)) = self.project.as_ref().and_then(|p| p.comments.get(id)).map(|c| (c.item.clone(), c.orphaned)) else { return };
        if let Some(state) = &mut self.project {
            state.active_comment = Some(id.to_owned());
        }
        if self.item_exists(&item) {
            self.pending_jump = Some(id.to_owned());
            self.open_item(&item);
            self.apply_jump();
        }
        if orphaned {
            self.status = "The text this comment was about has been deleted. Its place is kept.".into();
        }
        for tab in self.tabs.keys().copied().collect::<Vec<_>>() {
            self.sync_view_marks(tab);
        }
    }
    fn item_exists(&self, id: &str) -> bool {
        self.project.as_ref().is_some_and(|p| p.core.manifest.tree.find(id).is_some())
    }
    /// Selects the pending comment's words if its chapter is open and loaded.
    pub(super) fn apply_jump(&mut self) {
        let Some(id) = self.pending_jump.clone() else { return };
        let Some(tab_id) = self.tabs.iter().find(|(_, t)| t.marks.iter().any(|m| m.id == id) && !t.loading).map(|(i, _)| *i) else { return };
        let Some(tab) = self.tabs.get_mut(&tab_id) else { return };
        if let Some(mark) = tab.marks.iter().find(|m| m.id == id).cloned() {
            tab.document.anchor = mark.start.min(tab.document.text.len_chars());
            tab.document.cursor = mark.end.min(tab.document.text.len_chars());
            tab.view.ensure_cursor(&tab.document);
            self.pending_jump = None;
            // Make that tab the visible one.
            let mut leaves = Vec::new();
            self.split.leaves(Rect { x: 0, y: 0, width: 1, height: 1 }, &mut leaves);
            for (pane, _) in leaves {
                if let Some(p) = self.split.pane_mut(pane) {
                    if let Some(index) = p.tabs.iter().position(|t| *t == tab_id) {
                        p.active = index;
                        self.active_pane = pane;
                    }
                }
            }
        }
    }
    pub(super) fn finish_node_drag(&mut self, id: &str) {
        let group = self.marked_ids();
        let group = if group.len() > 1 && group.iter().any(|g| g == id) { group } else { vec![id.to_owned()] };
        let target = self.hit_at(self.mouse.x, self.mouse.y).and_then(|(_, hit)| match hit {
            Hit::Node(other) if !group.contains(&other) => {
                let tree = &self.project.as_ref()?.core.manifest.tree;
                let dragged_part = tree.find(id).is_some_and(Node::is_part);
                let target_part = tree.find(&other).is_some_and(Node::is_part);
                Some(if target_part && !dragged_part { DropTarget::Into(other) } else { DropTarget::Before(other) })
            }
            _ => None,
        });
        let target = target.or_else(|| {
            let over_tree = self.tree_rect.is_some_and(|r| contains(r, self.mouse.x, self.mouse.y));
            let section = self.project.as_ref()?.core.manifest.tree.locate(id)?.0;
            over_tree.then_some(DropTarget::End(section))
        });
        if let Some(target) = target {
            for id in &group {
                self.drop_node(id, target.clone());
            }
        }
    }

    // ---- import ----------------------------------------------------------------------------

    pub(super) fn import_read(&mut self, path: PathBuf, result: Result<Box<crate::import::Raw>, String>) {
        match result {
            Ok(raw) => {
                self.status.clear();
                self.import_form(path, *raw);
            }
            Err(error) => self.status = error,
        }
    }
    pub(super) fn imported(&mut self, result: Result<Box<crate::import::split::Imported>, String>) {
        match result {
            Err(error) => self.status = error,
            Ok(imported) => {
                if self.import_into_new || self.project.is_none() {
                    self.pending_import = Some(*imported);
                    self.choose(Choose::NewProject);
                    return;
                }
                self.flush_tabs();
                let Some(state) = &mut self.project else { return };
                match crate::import::append(&mut state.core, &imported) {
                    Ok(count) => {
                        state.comments = state.core.work.read(COMMENTS).map(|b| CommentSet::parse(&b)).unwrap_or_default();
                        state.changed = true;
                        self.status = format!("Imported {count} chapter{}.", if count == 1 { "" } else { "s" });
                    }
                    Err(error) => self.status = format!("The import failed: {error}"),
                }
            }
        }
    }

    // ---- timers ----------------------------------------------------------------------------

    /// Periodic project work: lock heartbeat, rewriting the `.jw`, automatic snapshots. Returns
    /// when it next needs to run.
    pub(super) fn project_tick(&mut self, now: Instant) -> Option<Instant> {
        let repack = Duration::from_secs(self.settings.repack_minutes.max(1) as u64 * 60);
        let beat = Duration::from_secs(60);
        let (due_pack, due_beat, due_snapshot) = {
            let state = self.project.as_ref()?;
            let auto = state.core.manifest.settings.snapshots.auto_minutes;
            // A linked project is packed soon after the writing stops, so the cloud gets it.
            let quick = state.last_save.filter(|_| state.cloud_linked).map(|saved| saved + super::cloud::QUIET);
            (
                quick.map_or(state.last_pack + repack, |quick| quick.min(state.last_pack + repack)),
                state.last_beat + beat,
                (auto > 0).then(|| state.last_snapshot + Duration::from_secs(auto as u64 * 60)),
            )
        };
        let mut next = due_pack.min(due_beat);
        if let Some(due) = due_snapshot {
            next = next.min(due);
        }
        if now >= due_beat {
            let state = self.project.as_mut()?;
            state.last_beat = now;
            if let Err(message) = state.core.heartbeat() {
                self.status = message;
            }
        }
        if now >= due_pack {
            self.sync_stale_marks();
            self.project.as_mut()?.last_pack = now;
            self.project.as_mut()?.last_save = None;
            if self.project.as_ref()?.core.is_dirty() && !self.project.as_ref()?.blocked {
                self.pack_project(false);
            }
        }
        if due_snapshot.is_some_and(|due| now >= due) {
            self.flush_tabs();
            let stamp = jw::time::now();
            let state = self.project.as_mut()?;
            state.last_snapshot = now;
            if let Ok(Some(_)) = state.history.capture_auto(&state.core, "Automatic snapshot", stamp) {
                state.changed = false;
                let _ = history::prune::prune(&mut state.history, &state.core, stamp);
            }
        }
        if self.project.as_ref()?.cloud_linked {
            if now >= self.cloud_pull + super::cloud::PULL {
                self.cloud_sync();
                self.cloud_pull = now;
            }
            next = next.min(self.cloud_pull + super::cloud::PULL);
        }
        Some(next.max(now + Duration::from_secs(5)))
    }
}

#[derive(Clone, Debug)]
pub enum DropTarget {
    Before(String),
    Into(String),
    End(Section),
}

#[allow(dead_code)]
fn _keep(_: &manifest::Manifest) {}

/// "4 chapters", "2 notes" or "3 items", for snapshot labels and prompts.
pub(super) fn noun(tree: &manifest::Tree, ids: &[String]) -> String {
    let kinds = |notes: bool| {
        ids.iter().all(|id| tree.locate(id).is_some_and(|(s, _)| (s == Section::Notes) == notes) && tree.find(id).is_some_and(|n| !n.is_part()))
    };
    let word = if kinds(false) { "chapters" } else if kinds(true) { "notes" } else { "items" };
    format!("{} {word}", ids.len())
}

/// The rows from `a` to `b` (either order) in `visible`; just `b` when one of them is not shown.
fn range_between(visible: &[String], a: &str, b: &str) -> Vec<String> {
    match (visible.iter().position(|v| v == a), visible.iter().position(|v| v == b)) {
        (Some(x), Some(y)) => visible[x.min(y)..=x.max(y)].to_vec(),
        _ => vec![b.to_owned()],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(n: usize) -> Vec<String> {
        (1..=n).map(|i| format!("c{i}")).collect()
    }
    #[test]
    fn range_runs_either_way() {
        assert_eq!(range_between(&ids(8), "c2", "c4"), ["c2", "c3", "c4"]);
        assert_eq!(range_between(&ids(8), "c6", "c5"), ["c5", "c6"]);
    }
    #[test]
    fn range_with_a_hidden_end_is_the_clicked_row() {
        assert_eq!(range_between(&ids(3), "gone", "c2"), ["c2"]);
    }
    #[test]
    fn noun_counts_chapters_notes_and_mixes() {
        let mut tree = manifest::Tree::default();
        let (a, b) = (Node::chapter("A"), Node::chapter("B"));
        let (ia, ib) = (a.id().to_owned(), b.id().to_owned());
        tree.manuscript.push(a);
        tree.manuscript.push(b);
        let page = Node::page("N");
        let ip = page.id().to_owned();
        tree.notes.push(page);
        assert_eq!(noun(&tree, &[ia.clone(), ib]), "2 chapters");
        assert_eq!(noun(&tree, &[ia, ip]), "2 items");
    }
}
