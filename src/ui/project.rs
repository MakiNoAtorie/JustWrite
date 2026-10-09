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
    pub selected: Option<String>,
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
        self.status = if note.is_empty() { format!("Opened {}", path.display()) } else { note };
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
        match state.core.close() {
            Ok(()) => {}
            Err(error) => self.status = format!("Could not save the project file: {error}"),
        }
    }

    // ---- items -------------------------------------------------------------------------

    pub(super) fn open_item(&mut self, id: &str) {
        let Some(file) = self.project.as_ref().and_then(|p| p.core.file_of(id)) else { return };
        if let Some(p) = &mut self.project {
            p.selected = Some(id.to_owned());
        }
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

    pub(super) fn new_node(&mut self, kind: &str) {
        let Some(state) = &mut self.project else {
            self.status = "Open or create a project first.".into();
            return;
        };
        let tree = &state.core.manifest.tree;
        let (section, parent, index, node) = match kind {
            "page" => {
                let n = tree.notes.iter().filter(|n| !n.is_part()).count() + 1;
                (Section::Notes, Vec::new(), tree.notes.len(), Node::page(&format!("Page {n}")))
            }
            "part" => {
                let n = tree.manuscript.iter().filter(|n| n.is_part()).count() + 1;
                (Section::Manuscript, Vec::new(), tree.manuscript.len(), Node::part(&format!("Part {n}")))
            }
            _ => {
                let count = tree.leaves(Section::Manuscript).len() + 1;
                // After the selected chapter, or at the end of the selected part.
                let place = state.selected.as_deref().and_then(|id| tree.locate(id)).filter(|(s, _)| *s == Section::Manuscript);
                let (parent, index) = match place {
                    Some((_, path)) => match tree.get(Section::Manuscript, &path) {
                        Some(n) if n.is_part() => (path.clone(), n.children().len()),
                        _ => (path[..path.len() - 1].to_vec(), path[path.len() - 1] + 1),
                    },
                    None => (Vec::new(), tree.manuscript.len()),
                };
                (Section::Manuscript, parent, index, Node::chapter(&format!("Chapter {count}")))
            }
        };
        let id = node.id().to_owned();
        let is_part = node.is_part();
        match state.core.add(section, &parent, index, node) {
            Ok(_) => {
                state.selected = Some(id.clone());
                state.changed = true;
                if !is_part {
                    self.open_item(&id);
                }
                self.dirty_session();
            }
            Err(error) => self.status = error,
        }
        self.redraw();
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
    /// Deletes a chapter, part or page, after saving a snapshot that still holds it.
    pub(super) fn delete_node(&mut self, id: &str) {
        let Some(state) = &mut self.project else { return };
        let title = state.core.manifest.tree.find(id).map(|n| n.title().to_owned()).unwrap_or_default();
        let now = jw::time::now();
        if let Err(error) = state.history.capture(&state.core, history::Kind::Safety, &format!("Before deleting \"{title}\""), now) {
            self.status = format!("Not deleted: the safety snapshot failed ({error})");
            return;
        }
        let mut files = Vec::new();
        fn collect(n: &Node, out: &mut Vec<String>) {
            out.extend(n.file());
            n.children().iter().for_each(|c| collect(c, out));
        }
        if let Some(node) = state.core.manifest.tree.find(id) {
            collect(node, &mut files);
        }
        match state.core.delete(id) {
            Ok(_) => {
                if state.selected.as_deref() == Some(id) {
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
                self.status = format!("Deleted \"{title}\". A snapshot before the deletion keeps its text.");
            }
            Err(error) => self.status = error,
        }
        self.redraw();
    }
    /// Moves the selected item up or down among its siblings (or across a part boundary).
    pub(super) fn move_selected(&mut self, down: bool) {
        let Some(state) = &mut self.project else { return };
        let Some(id) = state.selected.clone() else { return };
        let Some((section, path)) = state.core.manifest.tree.locate(&id) else { return };
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
                return;
            }
        } else if at > 0 {
            match tree.get(section, &[parent.clone(), vec![at - 1]].concat()) {
                Some(n) if n.is_part() && !tree.get(section, &path).is_some_and(Node::is_part) => ([parent.clone(), vec![at - 1]].concat(), n.children().len()),
                _ => (parent.clone(), at - 1),
            }
        } else if !parent.is_empty() {
            (parent[..parent.len() - 1].to_vec(), parent[parent.len() - 1])
        } else {
            return;
        };
        match state.core.move_node(section, &path, &to_parent, to_index) {
            Ok(()) => {
                state.changed = true;
                self.dirty_session();
            }
            Err(error) => self.status = error,
        }
        self.redraw();
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
            self.status = "Chapters stay in the manuscript and pages stay in Notes & research.".into();
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
    }
    pub(super) fn pack_project(&mut self, overwrite: bool) {
        let Some(state) = &mut self.project else { return };
        if state.packing {
            return;
        }
        state.packing = true;
        state.last_pack = Instant::now();
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
            }
            Err(None) => {
                state.blocked = true;
                self.changed_outside_form();
            }
            Err(Some(error)) => self.status = format!("Could not write the project file: {error}"),
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
        let target = self.hit_at(self.mouse.x, self.mouse.y).and_then(|(_, hit)| match hit {
            Hit::Node(other) if other != id => {
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
            self.drop_node(id, target);
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
            (
                state.last_pack + repack,
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
