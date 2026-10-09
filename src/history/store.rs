//! Snapshot storage inside the working copy:
//!
//! ```text
//! history/index.jsonl      one line per snapshot (summary), appended
//! history/lines.json       named lines and which one is current
//! history/snap/<id>.json   the tree and the blob of every chapter and page
//! history/blobs/<hash>     text, named by its BLAKE3 hash
//! ```

use crate::jw::{
    Project,
    manifest::{COMMENTS, HISTORY, Section, Tree},
    workdir::WorkDir,
};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, io};

pub const MAIN: &str = "Main";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    /// Taken by the author with a message. Never removed.
    Manual,
    /// Taken by the timer or when closing. Old ones are thinned out.
    Auto,
    /// Taken before a restore or a deletion so nothing is lost. Never removed.
    Safety,
    /// Records a restore.
    Restore,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnapshotMeta {
    pub id: String,
    pub time: i64,
    pub message: String,
    pub kind: Kind,
    pub line: String,
    pub words: u64,
    pub items: u32,
    /// Hash of everything the snapshot holds, to tell whether the project changed since.
    pub signature: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub restored_from: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ItemEntry {
    pub blob: String,
    pub words: u32,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SnapshotDetail {
    pub id: String,
    pub title: String,
    pub tree: Tree,
    pub items: BTreeMap<String, ItemEntry>,
    /// Blob of `comments.json`, if there was one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub comments: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Line {
    pub name: String,
    pub created: i64,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Lines {
    pub current: String,
    pub lines: Vec<Line>,
}
impl Default for Lines {
    fn default() -> Self {
        Self { current: MAIN.into(), lines: vec![Line { name: MAIN.into(), created: 0 }] }
    }
}

pub fn hash(bytes: &[u8]) -> String {
    blake3::hash(bytes).to_hex().to_string()
}
fn blob_path(hash: &str) -> String {
    format!("{HISTORY}/blobs/{hash}")
}
fn snap_path(id: &str) -> String {
    format!("{HISTORY}/snap/{id}.json")
}
const INDEX: &str = "history/index.jsonl";
const LINES: &str = "history/lines.json";

/// The project as it is (or was) at one moment: tree and content hashes, without the text.
#[derive(Clone, Debug)]
pub struct State {
    pub label: String,
    pub title: String,
    pub tree: Tree,
    pub items: BTreeMap<String, ItemEntry>,
    pub comments: Option<String>,
    /// The snapshot this came from; `None` for the working copy.
    pub snapshot: Option<String>,
}
impl State {
    /// The working copy as it is on disk now.
    pub fn working(project: &Project, write_blobs: bool) -> io::Result<State> {
        let mut items = BTreeMap::new();
        for section in [Section::Manuscript, Section::Notes] {
            for (node, _) in project.manifest.tree.leaves(section) {
                let Some(file) = node.file() else { continue };
                let bytes = project.work.read(&file).unwrap_or_default();
                let blob = hash(&bytes);
                if write_blobs {
                    project.work.write_once(&blob_path(&blob), &bytes)?;
                }
                items.insert(node.id().to_owned(), ItemEntry { blob, words: node.words() });
            }
        }
        let comments = match project.work.read(COMMENTS) {
            Ok(bytes) if !bytes.is_empty() => {
                let blob = hash(&bytes);
                if write_blobs {
                    project.work.write_once(&blob_path(&blob), &bytes)?;
                }
                Some(blob)
            }
            _ => None,
        };
        Ok(State {
            label: "Current text".into(),
            title: project.manifest.title.clone(),
            tree: project.manifest.tree.clone(),
            items,
            comments,
            snapshot: None,
        })
    }
    pub fn signature(&self) -> String {
        let mut hasher = blake3::Hasher::new();
        hasher.update(serde_json::to_string(&self.tree).unwrap_or_default().as_bytes());
        hasher.update(self.title.as_bytes());
        for (id, entry) in &self.items {
            hasher.update(id.as_bytes());
            hasher.update(entry.blob.as_bytes());
        }
        if let Some(c) = &self.comments {
            hasher.update(c.as_bytes());
        }
        hasher.finalize().to_hex()[..32].to_string()
    }
    pub fn words(&self) -> u64 {
        self.tree.manuscript_words()
    }
    /// The neutral text of a chapter or page in this state.
    pub fn text(&self, work: &WorkDir, id: &str) -> Option<String> {
        let entry = self.items.get(id)?;
        // The working copy's blobs are only written when a snapshot is taken; its text is read
        // from the chapter file instead.
        let bytes = match work.read(&blob_path(&entry.blob)) {
            Ok(bytes) => bytes,
            Err(_) if self.snapshot.is_none() => work.read(&self.tree.find(id)?.file()?).ok()?,
            Err(_) => return None,
        };
        String::from_utf8(bytes).ok()
    }
    pub fn comments_json(&self, work: &WorkDir) -> Option<Vec<u8>> {
        self.comments.as_ref().and_then(|b| work.read(&blob_path(b)).ok())
    }
}

#[derive(Clone, Debug, Default)]
pub struct History {
    /// Oldest first.
    pub snapshots: Vec<SnapshotMeta>,
    pub lines: Lines,
}

impl History {
    pub fn load(work: &WorkDir) -> History {
        let mut snapshots = Vec::new();
        if let Ok(text) = work.read_string(INDEX) {
            for line in text.lines() {
                // A line cut off by a crash is skipped.
                if let Ok(meta) = serde_json::from_str::<SnapshotMeta>(line) {
                    if !snapshots.iter().any(|s: &SnapshotMeta| s.id == meta.id) {
                        snapshots.push(meta);
                    }
                }
            }
        }
        let mut lines: Lines = work
            .read(LINES)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        // Lines named by snapshots always exist, even if lines.json was lost.
        for s in &snapshots {
            if !lines.lines.iter().any(|l| l.name == s.line) {
                lines.lines.push(Line { name: s.line.clone(), created: s.time });
            }
        }
        if !lines.lines.iter().any(|l| l.name == lines.current) {
            lines.current = lines.lines.first().map_or(MAIN.into(), |l| l.name.clone());
        }
        History { snapshots, lines }
    }
    fn save_lines(&self, work: &WorkDir) -> io::Result<()> {
        work.write(LINES, serde_json::to_string_pretty(&self.lines).map_err(io::Error::other)?.as_bytes())
    }
    pub fn current_line(&self) -> &str {
        &self.lines.current
    }
    pub fn get(&self, id: &str) -> Option<&SnapshotMeta> {
        self.snapshots.iter().find(|s| s.id == id)
    }
    pub fn head(&self, line: &str) -> Option<&SnapshotMeta> {
        self.snapshots.iter().rev().find(|s| s.line == line)
    }
    pub fn on_line<'a>(&'a self, line: &'a str) -> impl Iterator<Item = &'a SnapshotMeta> {
        self.snapshots.iter().filter(move |s| s.line == line)
    }
    pub fn last_auto_time(&self) -> Option<i64> {
        self.snapshots.iter().rev().find(|s| s.kind == Kind::Auto).map(|s| s.time)
    }

    pub fn detail(&self, work: &WorkDir, id: &str) -> io::Result<SnapshotDetail> {
        let bytes = work.read(&snap_path(id))?;
        serde_json::from_slice(&bytes).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
    }
    pub fn state(&self, work: &WorkDir, id: &str) -> io::Result<State> {
        let meta = self.get(id).ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "No such snapshot"))?;
        let d = self.detail(work, id)?;
        Ok(State {
            label: if meta.message.is_empty() { crate::jw::time::local(meta.time) } else { meta.message.clone() },
            title: d.title,
            tree: d.tree,
            items: d.items,
            comments: d.comments,
            snapshot: Some(id.to_owned()),
        })
    }

    /// Whether the project differs from the newest snapshot of the current line.
    pub fn changed_since_head(&self, project: &Project) -> io::Result<bool> {
        let state = State::working(project, false)?;
        Ok(self.head(&self.lines.current).is_none_or(|h| h.signature != state.signature()))
    }

    /// Records the project as it is now on the current line.
    pub fn capture(&mut self, project: &Project, kind: Kind, message: &str, now: i64) -> io::Result<SnapshotMeta> {
        self.capture_with(project, kind, message, now, None)
    }
    pub(crate) fn capture_with(
        &mut self,
        project: &Project,
        kind: Kind,
        message: &str,
        now: i64,
        restored_from: Option<String>,
    ) -> io::Result<SnapshotMeta> {
        let state = State::working(project, true)?;
        let id = crate::jw::new_id();
        let detail = SnapshotDetail {
            id: id.clone(),
            title: state.title.clone(),
            tree: state.tree.clone(),
            items: state.items.clone(),
            comments: state.comments.clone(),
        };
        project.work.write(&snap_path(&id), serde_json::to_string(&detail).map_err(io::Error::other)?.as_bytes())?;
        let meta = SnapshotMeta {
            id,
            time: now,
            message: message.trim().to_owned(),
            kind,
            line: self.lines.current.clone(),
            words: state.words(),
            items: state.items.len() as u32,
            signature: state.signature(),
            restored_from,
        };
        project.work.append(INDEX, format!("{}\n", serde_json::to_string(&meta).map_err(io::Error::other)?).as_bytes())?;
        self.snapshots.push(meta.clone());
        Ok(meta)
    }

    /// A snapshot the timer or closing asks for; skipped when nothing changed since the last one.
    pub fn capture_auto(&mut self, project: &Project, message: &str, now: i64) -> io::Result<Option<SnapshotMeta>> {
        if !self.changed_since_head(project)? {
            return Ok(None);
        }
        self.capture(project, Kind::Auto, message, now).map(Some)
    }

    // ---- lines ------------------------------------------------------------------------------

    /// Starts a new line from the project as it is now and makes it current. What was there is
    /// first saved on the line being left, so switching back finds it.
    pub fn new_line(&mut self, project: &Project, name: &str, now: i64) -> Result<(), String> {
        let name = name.trim();
        if name.is_empty() || name.len() > 60 || name.chars().any(char::is_control) {
            return Err("Give the line a short name".into());
        }
        if self.lines.lines.iter().any(|l| l.name.eq_ignore_ascii_case(name)) {
            return Err(format!("There is already a line called \"{name}\""));
        }
        let leaving = self.lines.current.clone();
        if self.changed_since_head(project).map_err(|e| e.to_string())? {
            self.capture(project, Kind::Safety, &format!("Before starting \"{name}\""), now).map_err(|e| e.to_string())?;
        }
        self.lines.lines.push(Line { name: name.to_owned(), created: now });
        self.lines.current = name.to_owned();
        self.save_lines(&project.work).map_err(|e| e.to_string())?;
        self.capture(project, Kind::Manual, &format!("Start of \"{name}\" (from \"{leaving}\")"), now).map_err(|e| e.to_string())?;
        Ok(())
    }
    pub fn rename_line(&mut self, work: &WorkDir, old: &str, new: &str) -> Result<(), String> {
        let new = new.trim();
        if new.is_empty() || new.len() > 60 {
            return Err("Give the line a short name".into());
        }
        if self.lines.lines.iter().any(|l| l.name.eq_ignore_ascii_case(new) && l.name != old) {
            return Err(format!("There is already a line called \"{new}\""));
        }
        let line = self.lines.lines.iter_mut().find(|l| l.name == old).ok_or("No such line")?;
        line.name = new.to_owned();
        if self.lines.current == old {
            self.lines.current = new.to_owned();
        }
        // The index is append-only, so renaming rewrites it once.
        for s in self.snapshots.iter_mut().filter(|s| s.line == old) {
            s.line = new.to_owned();
        }
        self.rewrite_index(work).map_err(|e| e.to_string())?;
        self.save_lines(work).map_err(|e| e.to_string())
    }
    pub(crate) fn set_current_line(&mut self, work: &WorkDir, name: &str) -> io::Result<()> {
        self.lines.current = name.to_owned();
        self.save_lines(work)
    }
    pub(crate) fn rewrite_index(&self, work: &WorkDir) -> io::Result<()> {
        let mut out = String::new();
        for s in &self.snapshots {
            out.push_str(&serde_json::to_string(s).map_err(io::Error::other)?);
            out.push('\n');
        }
        work.write(INDEX, out.as_bytes())
    }
    pub(crate) fn remove_snapshots(&mut self, work: &WorkDir, ids: &[String]) -> io::Result<()> {
        self.snapshots.retain(|s| !ids.contains(&s.id));
        self.rewrite_index(work)?;
        for id in ids {
            work.remove(&snap_path(id))?;
        }
        self.collect_garbage(work)
    }
    /// Deletes blobs no snapshot refers to.
    pub fn collect_garbage(&self, work: &WorkDir) -> io::Result<()> {
        let mut used = std::collections::HashSet::new();
        for s in &self.snapshots {
            if let Ok(d) = self.detail(work, &s.id) {
                used.extend(d.items.values().map(|e| e.blob.clone()));
                used.extend(d.comments);
            }
        }
        for name in work.list(&format!("{HISTORY}/blobs")).unwrap_or_default() {
            if !used.contains(&name) {
                work.remove(&blob_path(&name))?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
pub(crate) mod testing {
    use crate::jw::{Project, manifest::{Node, Section}};

    /// A project with chapters "One" and "Two" and a notes page, in temporary folders.
    pub struct Fixture {
        pub project: Project,
        pub ids: Vec<String>,
        _data: tempfile::TempDir,
        _out: tempfile::TempDir,
    }
    pub fn fixture() -> Fixture {
        let data = tempfile::tempdir().unwrap();
        let out = tempfile::tempdir().unwrap();
        let mut project = Project::create(data.path(), &out.path().join("book.jw"), "Novel").unwrap();
        let first = project.manifest.tree.manuscript[0].id().to_owned();
        project.rename(&first, "One").unwrap();
        project.write_text(&first, "The first chapter.\nSecond paragraph here.\n").unwrap();
        let p = project.add(Section::Manuscript, &[], 1, Node::chapter("Two")).unwrap();
        let second = project.manifest.tree.get(Section::Manuscript, &p).unwrap().id().to_owned();
        project.write_text(&second, "Chapter two begins.\n").unwrap();
        let p = project.add(Section::Notes, &[], 0, Node::page("Characters")).unwrap();
        let page = project.manifest.tree.get(Section::Notes, &p).unwrap().id().to_owned();
        project.write_text(&page, "Anna: tall.\n").unwrap();
        Fixture { project, ids: vec![first, second, page], _data: data, _out: out }
    }
}

#[cfg(test)]
mod tests {
    use super::testing::fixture;
    use super::*;

    #[test]
    fn unchanged_chapters_are_stored_once() {
        let mut f = fixture();
        let mut history = History::default();
        let a = history.capture(&f.project, Kind::Manual, "Draft 1 done", 1000).unwrap();
        let blobs = f.project.work.list("history/blobs").unwrap().len();
        // Only chapter one changes.
        f.project.write_text(&f.ids[0].clone(), "The first chapter, revised.\n").unwrap();
        let b = history.capture(&f.project, Kind::Auto, "", 2000).unwrap();
        assert_eq!(f.project.work.list("history/blobs").unwrap().len(), blobs + 1);
        assert_ne!(a.signature, b.signature);
        assert_eq!(history.snapshots.len(), 2);
        assert_eq!(a.items, 3);
        assert_eq!(a.words, 9); // manuscript words only; the notes page does not count
        // Content is retrievable from either snapshot.
        let old = history.state(&f.project.work, &a.id).unwrap();
        assert_eq!(old.text(&f.project.work, &f.ids[0]).unwrap(), "The first chapter.\nSecond paragraph here.\n");
        let new = history.state(&f.project.work, &b.id).unwrap();
        assert_eq!(new.text(&f.project.work, &f.ids[0]).unwrap(), "The first chapter, revised.\n");
        assert_eq!(old.items[&f.ids[1]].blob, new.items[&f.ids[1]].blob);
    }

    #[test]
    fn automatic_snapshots_skip_unchanged_projects() {
        let mut f = fixture();
        let mut history = History::default();
        assert!(history.capture_auto(&f.project, "timer", 10).unwrap().is_some());
        assert!(history.capture_auto(&f.project, "timer", 20).unwrap().is_none());
        f.project.write_text(&f.ids[1].clone(), "Changed.\n").unwrap();
        assert!(history.capture_auto(&f.project, "timer", 30).unwrap().is_some());
        assert_eq!(history.snapshots.len(), 2);
        assert_eq!(history.last_auto_time(), Some(30));
    }

    #[test]
    fn history_survives_reloading_and_a_torn_index_line() {
        let f = fixture();
        let mut history = History::default();
        history.capture(&f.project, Kind::Manual, "one", 1).unwrap();
        history.capture(&f.project, Kind::Manual, "two", 2).unwrap();
        f.project.work.append("history/index.jsonl", b"{\"id\":\"cut").unwrap();
        let loaded = History::load(&f.project.work);
        assert_eq!(loaded.snapshots.len(), 2);
        assert_eq!(loaded.snapshots[1].message, "two");
        assert_eq!(loaded.current_line(), MAIN);
    }

    #[test]
    fn lines_branch_from_the_current_state_and_switch_back() {
        let mut f = fixture();
        let mut history = History::default();
        history.capture(&f.project, Kind::Manual, "Draft 1 done", 100).unwrap();
        f.project.write_text(&f.ids[0].clone(), "Edited after draft one.\n").unwrap();
        history.new_line(&f.project, "Alt ending", 200).unwrap();
        assert_eq!(history.current_line(), "Alt ending");
        // Unsaved work was kept safe on Main before leaving it.
        let main: Vec<_> = history.on_line("Main").collect();
        assert_eq!(main.len(), 2);
        assert_eq!(main[1].kind, Kind::Safety);
        assert_eq!(history.head("Alt ending").unwrap().kind, Kind::Manual);
        assert!(history.new_line(&f.project, "alt ending", 300).is_err());
        assert!(history.new_line(&f.project, "  ", 300).is_err());
        history.rename_line(&f.project.work, "Alt ending", "Alternate ending").unwrap();
        assert_eq!(history.current_line(), "Alternate ending");
        assert_eq!(History::load(&f.project.work).current_line(), "Alternate ending");
        assert_eq!(History::load(&f.project.work).head("Alternate ending").unwrap().kind, Kind::Manual);
    }
}
