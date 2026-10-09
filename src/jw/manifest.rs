//! `manifest.json`: format version, the project tree, metadata and per-project settings.

use crate::core::typography::{Language, Typography};
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU64, Ordering};

pub const FORMAT: &str = "justwrite";
/// Bumped when the layout of a `.jw` file changes incompatibly. Files from a newer version are
/// refused rather than damaged.
pub const FORMAT_VERSION: u32 = 1;

pub const MANIFEST: &str = "manifest.json";
pub const COMMENTS: &str = "comments.json";
pub const BOOK: &str = "book.json";
pub const PRESETS: &str = "export-presets.json";
pub const HISTORY: &str = "history";
pub const ASSETS: &str = "assets";

/// A short random identifier for projects, chapters and pages.
pub fn new_id() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let mut hasher = blake3::Hasher::new();
    hasher.update(&nanos.to_le_bytes());
    hasher.update(&std::process::id().to_le_bytes());
    hasher.update(&COUNTER.fetch_add(1, Ordering::Relaxed).to_le_bytes());
    let local = 0u8;
    hasher.update(&(&local as *const u8 as usize).to_le_bytes());
    hasher.finalize().to_hex()[..12].to_string()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Section {
    Manuscript,
    Notes,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Node {
    /// A part or act grouping chapters (in the notes section: a plain group of pages).
    Part {
        id: String,
        title: String,
        #[serde(default)]
        children: Vec<Node>,
    },
    Chapter {
        id: String,
        title: String,
        #[serde(default)]
        words: u32,
    },
    Page {
        id: String,
        title: String,
        #[serde(default)]
        words: u32,
    },
}
impl Node {
    pub fn chapter(title: &str) -> Self {
        Self::Chapter { id: new_id(), title: title.into(), words: 0 }
    }
    pub fn page(title: &str) -> Self {
        Self::Page { id: new_id(), title: title.into(), words: 0 }
    }
    pub fn part(title: &str) -> Self {
        Self::Part { id: new_id(), title: title.into(), children: Vec::new() }
    }
    pub fn id(&self) -> &str {
        match self {
            Self::Part { id, .. } | Self::Chapter { id, .. } | Self::Page { id, .. } => id,
        }
    }
    pub fn title(&self) -> &str {
        match self {
            Self::Part { title, .. } | Self::Chapter { title, .. } | Self::Page { title, .. } => title,
        }
    }
    pub fn set_title(&mut self, new: &str) {
        match self {
            Self::Part { title, .. } | Self::Chapter { title, .. } | Self::Page { title, .. } => *title = new.into(),
        }
    }
    pub fn words(&self) -> u32 {
        match self {
            Self::Part { children, .. } => children.iter().map(Node::words).sum(),
            Self::Chapter { words, .. } | Self::Page { words, .. } => *words,
        }
    }
    pub fn children(&self) -> &[Node] {
        match self {
            Self::Part { children, .. } => children,
            _ => &[],
        }
    }
    pub fn is_part(&self) -> bool {
        matches!(self, Self::Part { .. })
    }
    /// The text file of a chapter or page, relative to the project root.
    pub fn file(&self) -> Option<String> {
        match self {
            Self::Chapter { id, .. } => Some(format!("chapters/{id}.txt")),
            Self::Page { id, .. } => Some(format!("pages/{id}.txt")),
            Self::Part { .. } => None,
        }
    }
}

/// Index path from the top of a section: `[2, 0]` is the first child of the third node.
pub type NodePath = Vec<usize>;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Tree {
    #[serde(default)]
    pub manuscript: Vec<Node>,
    #[serde(default)]
    pub notes: Vec<Node>,
}
impl Tree {
    pub fn section(&self, section: Section) -> &Vec<Node> {
        match section {
            Section::Manuscript => &self.manuscript,
            Section::Notes => &self.notes,
        }
    }
    pub fn section_mut(&mut self, section: Section) -> &mut Vec<Node> {
        match section {
            Section::Manuscript => &mut self.manuscript,
            Section::Notes => &mut self.notes,
        }
    }
    pub fn get(&self, section: Section, path: &[usize]) -> Option<&Node> {
        let (first, rest) = path.split_first()?;
        let mut node = self.section(section).get(*first)?;
        for index in rest {
            node = match node {
                Node::Part { children, .. } => children.get(*index)?,
                _ => return None,
            };
        }
        Some(node)
    }
    fn list_mut(&mut self, section: Section, parent: &[usize]) -> Option<&mut Vec<Node>> {
        let mut list = self.section_mut(section);
        for index in parent {
            list = match list.get_mut(*index)? {
                Node::Part { children, .. } => children,
                _ => return None,
            };
        }
        Some(list)
    }
    pub fn get_mut(&mut self, section: Section, path: &[usize]) -> Option<&mut Node> {
        let (last, parent) = path.split_last()?;
        self.list_mut(section, parent)?.get_mut(*last)
    }
    /// Where `id` lives.
    pub fn locate(&self, id: &str) -> Option<(Section, NodePath)> {
        fn walk(nodes: &[Node], id: &str, path: &mut NodePath) -> bool {
            for (i, node) in nodes.iter().enumerate() {
                path.push(i);
                if node.id() == id || walk(node.children(), id, path) {
                    return true;
                }
                path.pop();
            }
            false
        }
        for section in [Section::Manuscript, Section::Notes] {
            let mut path = Vec::new();
            if walk(self.section(section), id, &mut path) {
                return Some((section, path));
            }
        }
        None
    }
    pub fn find(&self, id: &str) -> Option<&Node> {
        let (section, path) = self.locate(id)?;
        self.get(section, &path)
    }
    pub fn find_mut(&mut self, id: &str) -> Option<&mut Node> {
        let (section, path) = self.locate(id)?;
        self.get_mut(section, &path)
    }
    /// Inserts under `parent` (empty path: the top of the section) at `index`.
    pub fn insert(&mut self, section: Section, parent: &[usize], index: usize, node: Node) -> Result<NodePath, String> {
        if section == Section::Manuscript && matches!(node, Node::Page { .. }) {
            return Err("Notes belong in Notes & research".into());
        }
        if section == Section::Notes && matches!(node, Node::Chapter { .. }) {
            return Err("Chapters belong in the manuscript".into());
        }
        if node.is_part() && !parent.is_empty() {
            return Err("Parts cannot be nested".into());
        }
        let list = self.list_mut(section, parent).ok_or("No such place in the tree")?;
        let index = index.min(list.len());
        list.insert(index, node);
        let mut path = parent.to_vec();
        path.push(index);
        Ok(path)
    }
    pub fn remove(&mut self, section: Section, path: &[usize]) -> Option<Node> {
        let (last, parent) = path.split_last()?;
        let list = self.list_mut(section, parent)?;
        (*last < list.len()).then(|| list.remove(*last))
    }
    /// Moves a node to `index` under `parent`, where `parent` and `index` refer to the tree as it
    /// is *before* the move.
    pub fn move_node(&mut self, section: Section, from: &[usize], parent: &[usize], index: usize) -> Result<NodePath, String> {
        if parent.len() >= from.len() && parent[..from.len()] == *from {
            return Err("Cannot move something into itself".into());
        }
        let node = self.get(section, from).cloned().ok_or("Nothing to move")?;
        if node.is_part() && !parent.is_empty() {
            return Err("Parts cannot be nested".into());
        }
        // Adjust the destination for the removal of the source.
        let (source_index, source_parent) = from.split_last().unwrap();
        let mut parent = parent.to_vec();
        let mut index = index;
        if source_parent.len() <= parent.len() && parent[..source_parent.len()] == *source_parent {
            if parent.len() == source_parent.len() {
                if index > *source_index {
                    index -= 1;
                }
            } else if parent[source_parent.len()] > *source_index {
                parent[source_parent.len()] -= 1;
            }
        }
        self.remove(section, from);
        self.insert(section, &parent, index, node)
    }
    /// Chapters and pages of a section in reading order, with the part each belongs to.
    pub fn leaves(&self, section: Section) -> Vec<(&Node, Option<&Node>)> {
        let mut out = Vec::new();
        for node in self.section(section) {
            match node {
                Node::Part { children, .. } => out.extend(children.iter().map(|c| (c, Some(node)))),
                other => out.push((other, None)),
            }
        }
        out
    }
    pub fn manuscript_words(&self) -> u64 {
        self.manuscript.iter().map(|n| n.words() as u64).sum()
    }
    pub fn notes_words(&self) -> u64 {
        self.notes.iter().map(|n| n.words() as u64).sum()
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Goals {
    /// Words the finished manuscript should have; 0 means no goal.
    pub project_words: u32,
    /// Words to write in one sitting; 0 means no goal.
    pub session_words: u32,
}
impl Default for Goals {
    fn default() -> Self {
        Self { project_words: 0, session_words: 0 }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SnapshotPolicy {
    /// Minutes between automatic snapshots; 0 turns the timer off.
    pub auto_minutes: u32,
    pub on_close: bool,
    /// Automatic snapshots newer than this many days are all kept.
    pub keep_all_days: u32,
    /// Older ones are thinned to one per day until this many days...
    pub keep_daily_days: u32,
    /// ...and to one per week after that. Manual snapshots are never removed.
    pub thin: bool,
}
impl Default for SnapshotPolicy {
    fn default() -> Self {
        Self { auto_minutes: 30, on_close: true, keep_all_days: 2, keep_daily_days: 30, thin: true }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ProjectSettings {
    pub language: Language,
    pub typography: Typography,
    /// Shown for scene-break lines.
    pub scene_break: String,
    pub goals: Goals,
    pub snapshots: SnapshotPolicy,
    /// Id of the export preset the Export command starts from.
    pub default_preset: String,
}
impl Default for ProjectSettings {
    fn default() -> Self {
        Self {
            language: Language::English,
            typography: Typography::default(),
            scene_break: "*  *  *".into(),
            goals: Goals::default(),
            snapshots: SnapshotPolicy::default(),
            default_preset: String::new(),
        }
    }
}

/// How the private-use characters in stored text are read; written into every manifest so a
/// project can be understood without this program.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct NoteTokens {
    pub line_note: String,
    pub inline_open: String,
    pub inline_close: String,
    pub block_open: String,
    pub block_close: String,
}
impl Default for NoteTokens {
    fn default() -> Self {
        Self {
            line_note: "U+E000: starts a note that runs to the end of the line".into(),
            inline_open: "U+E001: opens a note that ends at U+E002 on the same line".into(),
            inline_close: "U+E002".into(),
            block_open: "U+E003: alone on a line, opens a block note".into(),
            block_close: "U+E004: alone on a line, closes it".into(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Manifest {
    pub format: String,
    pub format_version: u32,
    pub app_version: String,
    pub id: String,
    pub title: String,
    pub created: i64,
    pub modified: i64,
    pub tree: Tree,
    #[serde(default)]
    pub settings: ProjectSettings,
    #[serde(default)]
    pub note_tokens: NoteTokens,
    /// Fields written by newer versions that this one does not know; kept on save.
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}
impl Manifest {
    pub fn new(title: &str) -> Self {
        let now = super::time::now();
        Self {
            format: FORMAT.into(),
            format_version: FORMAT_VERSION,
            app_version: env!("CARGO_PKG_VERSION").into(),
            id: new_id(),
            title: title.into(),
            created: now,
            modified: now,
            tree: Tree::default(),
            settings: ProjectSettings::default(),
            note_tokens: NoteTokens::default(),
            extra: Default::default(),
        }
    }
    pub fn parse(bytes: &[u8]) -> Result<Self, String> {
        let manifest: Self = serde_json::from_slice(bytes).map_err(|e| format!("Unreadable manifest.json: {e}"))?;
        if manifest.format != FORMAT {
            return Err("This is not a JustWrite project".into());
        }
        if manifest.format_version > FORMAT_VERSION {
            return Err(format!(
                "This project was saved by a newer version of JustWrite (format {}). Update the app to open it.",
                manifest.format_version
            ));
        }
        Ok(manifest)
    }
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).expect("serialize manifest")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_unique_and_short() {
        let ids: std::collections::HashSet<String> = (0..2000).map(|_| new_id()).collect();
        assert_eq!(ids.len(), 2000);
        assert!(ids.iter().all(|i| i.len() == 12 && i.chars().all(|c| c.is_ascii_hexdigit())));
    }

    fn sample() -> Tree {
        let mut t = Tree::default();
        let m = Section::Manuscript;
        t.insert(m, &[], 0, Node::chapter("Prologue")).unwrap();
        let part = t.insert(m, &[], 1, Node::part("Act I")).unwrap();
        t.insert(m, &part, 0, Node::chapter("One")).unwrap();
        t.insert(m, &part, 1, Node::chapter("Two")).unwrap();
        t.insert(m, &[], 2, Node::chapter("Epilogue")).unwrap();
        t.insert(Section::Notes, &[], 0, Node::page("Characters")).unwrap();
        t
    }
    fn titles(t: &Tree) -> Vec<String> {
        t.leaves(Section::Manuscript).iter().map(|(n, _)| n.title().to_owned()).collect()
    }

    #[test]
    fn tree_navigation_and_rules() {
        let mut t = sample();
        assert_eq!(titles(&t), ["Prologue", "One", "Two", "Epilogue"]);
        let two = t.find("nonexistent");
        assert!(two.is_none());
        let id = t.get(Section::Manuscript, &[1, 1]).unwrap().id().to_owned();
        assert_eq!(t.locate(&id), Some((Section::Manuscript, vec![1, 1])));
        assert!(t.insert(Section::Manuscript, &[], 0, Node::page("x")).is_err());
        assert!(t.insert(Section::Notes, &[], 0, Node::chapter("x")).is_err());
        assert!(t.insert(Section::Manuscript, &[1], 0, Node::part("nested")).is_err());
        assert!(t.insert(Section::Manuscript, &[0], 0, Node::chapter("inside a chapter")).is_err());
        let leaves = t.leaves(Section::Manuscript);
        assert_eq!(leaves[1].1.map(Node::title), Some("Act I"));
        assert_eq!(leaves[0].1, None);
    }

    #[test]
    fn moving_nodes_adjusts_for_the_removed_source() {
        let m = Section::Manuscript;
        let mut t = sample();
        // Move "Prologue" after "Epilogue" (destination index in the pre-move tree).
        t.move_node(m, &[0], &[], 3).unwrap();
        assert_eq!(titles(&t), ["One", "Two", "Epilogue", "Prologue"]);
        // Move "Epilogue" into the part, first position.
        let mut t = sample();
        t.move_node(m, &[2], &[1], 0).unwrap();
        assert_eq!(titles(&t), ["Prologue", "Epilogue", "One", "Two"]);
        // Move a chapter out of the part to the very top.
        let mut t = sample();
        t.move_node(m, &[1, 1], &[], 0).unwrap();
        assert_eq!(titles(&t), ["Two", "Prologue", "One", "Epilogue"]);
        // A chapter can move into a part after the part's own children.
        let mut t = sample();
        t.move_node(m, &[0], &[1], 2).unwrap();
        assert_eq!(titles(&t), ["One", "Two", "Prologue", "Epilogue"]);
    }

    #[test]
    fn moving_into_itself_is_refused() {
        let m = Section::Manuscript;
        let mut t = sample();
        assert!(t.move_node(m, &[1], &[1], 0).is_err());
        assert!(t.move_node(m, &[1], &[1, 0], 0).is_err());
        assert_eq!(titles(&t), ["Prologue", "One", "Two", "Epilogue"]);
    }

    #[test]
    fn manifest_round_trips_and_keeps_unknown_fields() {
        let mut m = Manifest::new("My Novel");
        m.tree = sample();
        let mut json: serde_json::Value = serde_json::from_str(&m.to_json()).unwrap();
        json["future_feature"] = serde_json::json!({"a": 1});
        let back = Manifest::parse(json.to_string().as_bytes()).unwrap();
        assert_eq!(back.tree, m.tree);
        assert!(back.extra.contains_key("future_feature"));
        assert!(back.to_json().contains("future_feature"));
        assert_eq!(m.to_json().contains("\"format_version\": 1"), true);
        // Newer or foreign files are refused.
        json["format_version"] = serde_json::json!(99);
        assert!(Manifest::parse(json.to_string().as_bytes()).unwrap_err().contains("newer version"));
        json["format_version"] = serde_json::json!(1);
        json["format"] = serde_json::json!("other");
        assert!(Manifest::parse(json.to_string().as_bytes()).is_err());
        assert!(Manifest::parse(b"{}").is_err());
    }

    #[test]
    fn word_totals_ignore_notes_section() {
        let mut t = sample();
        if let Some(Node::Chapter { words, .. }) = t.get_mut(Section::Manuscript, &[0]) {
            *words = 100;
        }
        if let Some(Node::Chapter { words, .. }) = t.get_mut(Section::Manuscript, &[1, 0]) {
            *words = 50;
        }
        if let Some(Node::Page { words, .. }) = t.get_mut(Section::Notes, &[0]) {
            *words = 999;
        }
        assert_eq!(t.manuscript_words(), 150);
        assert_eq!(t.notes_words(), 999);
        assert_eq!(t.manuscript[1].words(), 50);
    }
}
