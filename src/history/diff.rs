//! Comparing two states of the project: the working text or any snapshot.

use super::store::State;
use crate::{
    core::notes::{self, Marker},
    jw::{manifest::Section, workdir::WorkDir},
};
use similar::{ChangeTag, DiffOp, TextDiff};
use std::{collections::BTreeSet, time::Duration};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Unchanged,
    Added,
    Removed,
    Changed,
}
#[derive(Clone, Debug)]
pub struct ItemChange {
    pub id: String,
    pub section: Section,
    pub title: String,
    /// The title in the older state when it was renamed.
    pub was: Option<String>,
    pub status: Status,
    pub words_a: u32,
    pub words_b: u32,
    /// The position among the chapters changed.
    pub moved: bool,
}
#[derive(Clone, Debug, Default)]
pub struct Comparison {
    pub items: Vec<ItemChange>,
    pub words_a: u64,
    pub words_b: u64,
}
impl Comparison {
    pub fn changed(&self) -> impl Iterator<Item = &ItemChange> {
        self.items.iter().filter(|i| i.status != Status::Unchanged || i.moved || i.was.is_some())
    }
}

/// Which chapters and pages differ between `a` (older) and `b` (newer), by comparing hashes; no
/// text is read.
pub fn compare(a: &State, b: &State) -> Comparison {
    let order = |s: &State| -> Vec<String> {
        [Section::Manuscript, Section::Notes]
            .iter()
            .flat_map(|sec| s.tree.leaves(*sec).into_iter().map(|(n, _)| n.id().to_owned()).collect::<Vec<_>>())
            .collect()
    };
    let (oa, ob) = (order(a), order(b));
    let mut seen = BTreeSet::new();
    let mut items = Vec::new();
    // Newer order first, then what only the older state had.
    for id in ob.iter().chain(oa.iter()) {
        if !seen.insert(id.clone()) {
            continue;
        }
        let (na, nb) = (a.tree.find(id), b.tree.find(id));
        let node = nb.or(na).unwrap();
        let section = b.tree.locate(id).or_else(|| a.tree.locate(id)).map_or(Section::Manuscript, |l| l.0);
        let (ea, eb) = (a.items.get(id), b.items.get(id));
        let status = match (ea, eb) {
            (None, Some(_)) => Status::Added,
            (Some(_), None) => Status::Removed,
            (Some(x), Some(y)) if x.blob != y.blob => Status::Changed,
            _ => Status::Unchanged,
        };
        let was = match (na, nb) {
            (Some(x), Some(y)) if x.title() != y.title() => Some(x.title().to_owned()),
            _ => None,
        };
        let rank = |order: &Vec<String>| order.iter().filter(|o| ob.contains(o) && oa.contains(o)).position(|o| o == id);
        let moved = ea.is_some() && eb.is_some() && rank(&oa) != rank(&ob);
        items.push(ItemChange {
            id: id.clone(),
            section,
            title: node.title().to_owned(),
            was,
            status,
            words_a: ea.map_or(0, |e| e.words),
            words_b: eb.map_or(0, |e| e.words),
            moved,
        });
    }
    Comparison { items, words_a: a.words(), words_b: b.words() }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Segment {
    pub text: String,
    /// This part is what changed within the line.
    pub changed: bool,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RowKind {
    Equal,
    Delete,
    Insert,
    /// This many unchanged lines were left out.
    Skip(usize),
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub kind: RowKind,
    pub segments: Vec<Segment>,
}
impl Row {
    fn plain(kind: RowKind, text: &str, changed: bool) -> Self {
        Self { kind, segments: vec![Segment { text: text.trim_end_matches(['\r', '\n']).to_owned(), changed }] }
    }
    pub fn text(&self) -> String {
        self.segments.iter().map(|s| s.text.as_str()).collect()
    }
}

#[derive(Clone, Copy)]
pub struct Options<'a> {
    /// Leave notes out of both sides before comparing.
    pub hide_notes: bool,
    /// Lines of unchanged text kept around each change.
    pub context: usize,
    /// How notes are written when shown.
    pub marker: &'a Marker,
}

fn prepare(text: &str, options: &Options) -> String {
    if options.hide_notes {
        notes::strip(text)
    } else {
        notes::to_display(text, options.marker).0
    }
}

/// Line-by-line differences for one chapter, with changed words marked inside changed lines.
pub fn rows(old: &str, new: &str, options: &Options) -> Vec<Row> {
    let (old, new) = (prepare(old, options), prepare(new, options));
    let diff = TextDiff::configure().timeout(Duration::from_secs(2)).diff_lines(&old, &new);
    let old_lines: Vec<&str> = old.split_inclusive('\n').collect();
    let new_lines: Vec<&str> = new.split_inclusive('\n').collect();
    let mut out = Vec::new();
    for op in diff.ops() {
        match *op {
            DiffOp::Equal { old_index, len, .. } => {
                let lines = &old_lines[old_index..old_index + len];
                let first = out.is_empty();
                let last = std::ptr::eq(op, diff.ops().last().unwrap());
                let (head, tail) = (if first { 0 } else { options.context }, if last { 0 } else { options.context });
                if len > head + tail + 1 {
                    for l in &lines[..head] {
                        out.push(Row::plain(RowKind::Equal, l, false));
                    }
                    out.push(Row { kind: RowKind::Skip(len - head - tail), segments: Vec::new() });
                    for l in &lines[len - tail..] {
                        out.push(Row::plain(RowKind::Equal, l, false));
                    }
                } else {
                    for l in lines {
                        out.push(Row::plain(RowKind::Equal, l, false));
                    }
                }
            }
            DiffOp::Delete { old_index, old_len, .. } => {
                for l in &old_lines[old_index..old_index + old_len] {
                    out.push(Row::plain(RowKind::Delete, l, true));
                }
            }
            DiffOp::Insert { new_index, new_len, .. } => {
                for l in &new_lines[new_index..new_index + new_len] {
                    out.push(Row::plain(RowKind::Insert, l, true));
                }
            }
            DiffOp::Replace { old_index, old_len, new_index, new_len } => {
                let pairs = old_len.max(new_len);
                for k in 0..pairs {
                    match (old_lines.get(old_index + k).filter(|_| k < old_len), new_lines.get(new_index + k).filter(|_| k < new_len)) {
                        (Some(a), Some(b)) => {
                            let (da, db) = word_rows(a, b);
                            out.push(da);
                            out.push(db);
                        }
                        (Some(a), None) => out.push(Row::plain(RowKind::Delete, a, true)),
                        (None, Some(b)) => out.push(Row::plain(RowKind::Insert, b, true)),
                        (None, None) => {}
                    }
                }
            }
        }
    }
    out
}

/// The same line before and after, with the changed words marked.
fn word_rows(a: &str, b: &str) -> (Row, Row) {
    let (a, b) = (a.trim_end_matches(['\r', '\n']), b.trim_end_matches(['\r', '\n']));
    let diff = TextDiff::configure().timeout(Duration::from_millis(200)).diff_words(a, b);
    let (mut left, mut right): (Vec<Segment>, Vec<Segment>) = (Vec::new(), Vec::new());
    let push = |list: &mut Vec<Segment>, text: &str, changed: bool| match list.last_mut() {
        Some(s) if s.changed == changed => s.text.push_str(text),
        _ => list.push(Segment { text: text.to_owned(), changed }),
    };
    for change in diff.iter_all_changes() {
        match change.tag() {
            ChangeTag::Equal => {
                push(&mut left, change.value(), false);
                push(&mut right, change.value(), false);
            }
            ChangeTag::Delete => push(&mut left, change.value(), true),
            ChangeTag::Insert => push(&mut right, change.value(), true),
        }
    }
    (Row { kind: RowKind::Delete, segments: left }, Row { kind: RowKind::Insert, segments: right })
}

/// Differences for one chapter between two states.
pub fn item_rows(work: &WorkDir, a: &State, b: &State, id: &str, options: &Options) -> Vec<Row> {
    let old = a.text(work, id).unwrap_or_default();
    let new = b.text(work, id).unwrap_or_default();
    rows(&old, &new, options)
}

#[cfg(test)]
mod tests {
    use super::super::store::{History, Kind, testing::fixture};
    use super::*;

    fn options(marker: &Marker) -> Options<'_> {
        Options { hide_notes: false, context: 1, marker }
    }

    #[test]
    fn rows_show_changed_words_inside_changed_lines() {
        let marker = Marker::default();
        let r = rows("one\nThe old man smiled.\nthree\n", "one\nThe young man smiled.\nthree\n", &options(&marker));
        let kinds: Vec<_> = r.iter().map(|r| r.kind.clone()).collect();
        assert_eq!(kinds, [RowKind::Equal, RowKind::Delete, RowKind::Insert, RowKind::Equal]);
        assert_eq!(r[1].text(), "The old man smiled.");
        assert!(r[1].segments.iter().any(|s| s.changed && s.text.contains("old")));
        assert!(r[2].segments.iter().any(|s| s.changed && s.text.contains("young")));
        assert!(r[1].segments.iter().any(|s| !s.changed && s.text.contains("man smiled")));
    }

    #[test]
    fn unchanged_stretches_collapse_to_a_skip() {
        let marker = Marker::default();
        let old: String = (0..30).map(|i| format!("line {i}\n")).collect();
        let new = old.replace("line 15\n", "changed\n");
        let r = rows(&old, &new, &options(&marker));
        assert!(r.iter().any(|r| matches!(r.kind, RowKind::Skip(n) if n > 10)));
        assert!(r.len() < 12);
        assert!(rows("same\n", "same\n", &options(&marker)).iter().all(|r| r.kind == RowKind::Equal || matches!(r.kind, RowKind::Skip(_))));
    }

    #[test]
    fn notes_can_be_hidden_in_the_diff() {
        let marker = Marker::default();
        let (old, _) = notes::to_neutral("Same prose. $$ remember to fix\n", &marker);
        let (new, _) = notes::to_neutral("Same prose. $$ fixed now\n", &marker);
        let shown = rows(&old, &new, &options(&marker));
        assert!(shown.iter().any(|r| r.kind == RowKind::Delete));
        assert!(shown.iter().any(|r| r.text().contains("$$ fixed now")));
        let hidden = rows(&old, &new, &Options { hide_notes: true, ..options(&marker) });
        assert!(hidden.iter().all(|r| r.kind == RowKind::Equal), "{hidden:?}");
    }

    #[test]
    fn comparing_snapshots_and_the_working_text_finds_what_changed() {
        let mut f = fixture();
        let (one, two, page) = (f.ids[0].clone(), f.ids[1].clone(), f.ids[2].clone());
        let mut history = History::default();
        let a = history.capture(&f.project, Kind::Manual, "Draft 1", 10).unwrap();
        f.project.write_text(&one, "The first chapter, rewritten.\n").unwrap();
        f.project.rename(&two, "Two (renamed)").unwrap();
        f.project.delete(&page).unwrap();
        let p = f.project.add(Section::Manuscript, &[], 2, crate::jw::Node::chapter("Three")).unwrap();
        let three = f.project.manifest.tree.get(Section::Manuscript, &p).unwrap().id().to_owned();
        f.project.write_text(&three, "A new chapter with words.\n").unwrap();
        let old = history.state(&f.project.work, &a.id).unwrap();
        let now = State::working(&f.project, false).unwrap();
        let cmp = compare(&old, &now);
        let status = |id: &str| cmp.items.iter().find(|i| i.id == id).unwrap();
        assert_eq!(status(&one).status, Status::Changed);
        assert_eq!(status(&two).status, Status::Unchanged);
        assert_eq!(status(&two).was.as_deref(), Some("Two"));
        assert_eq!(status(&page).status, Status::Removed);
        assert_eq!(status(&three).status, Status::Added);
        assert!(cmp.words_b > cmp.words_a - 6);
        assert_eq!(cmp.changed().count(), 4);
        // A snapshot compared with itself differs in nothing.
        let same = compare(&old, &old);
        assert_eq!(same.changed().count(), 0);
        let marker = Marker::default();
        let r = item_rows(&f.project.work, &old, &now, &one, &options(&marker));
        assert!(r.iter().any(|r| r.kind == RowKind::Insert && r.text().contains("rewritten")));
    }

    #[test]
    fn reordering_chapters_is_reported_as_a_move() {
        let mut f = fixture();
        let (one, two) = (f.ids[0].clone(), f.ids[1].clone());
        let mut history = History::default();
        let a = history.capture(&f.project, Kind::Manual, "d", 1).unwrap();
        f.project.move_node(Section::Manuscript, &[1], &[], 0).unwrap();
        let cmp = compare(&history.state(&f.project.work, &a.id).unwrap(), &State::working(&f.project, false).unwrap());
        assert!(cmp.items.iter().any(|i| i.id == one && i.moved) || cmp.items.iter().any(|i| i.id == two && i.moved));
    }
}
