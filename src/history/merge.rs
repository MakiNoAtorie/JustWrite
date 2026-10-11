//! Bringing in the version of this project from another computer (cloud saving). Like a restore
//! it destroys nothing: what is here is saved first, the other version is kept whole on a line
//! of its own, and the result is recorded as a snapshot.
//!
//! The two versions are compared with the newest snapshot both computers have. A chapter that
//! changed on one side only is taken from that side. A chapter that changed on both is put
//! together paragraph by paragraph; when both changed the same paragraph the text here stays.

use super::{
    restore::{apply_state, reanchor_comments},
    store::{History, ItemEntry, Kind, Line, SnapshotMeta, State, blob_path, hash, snap_path},
};
use crate::{
    core::notes,
    jw::{
        Manifest, Project, Section, archive,
        manifest::{MANIFEST, Node, Tree},
        time,
        workdir::WorkDir,
    },
};
use similar::{Algorithm, DiffOp, capture_diff_slices};
use std::path::Path;

/// What a merge did, for the message to the author.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Merged {
    /// The line that holds the other computer's version. Empty: that version had nothing this
    /// one lacks, and nothing was done.
    pub line: String,
    /// Chapters and notes taken from there as they are.
    pub taken: usize,
    /// Chapters changed in different places on both computers and put together.
    pub combined: usize,
    /// Chapters changed in the same place on both. The text here was kept.
    pub conflicts: Vec<String>,
    /// The chapter list changed on both computers. The list here was kept.
    pub lists_differ: bool,
}

fn err(e: std::io::Error) -> String {
    e.to_string()
}

/// One side's change: lines `start..end` of the base become `lines`.
struct Hunk<'a> {
    start: usize,
    end: usize,
    lines: &'a [&'a str],
}
fn hunks<'a>(base: &[&str], side: &'a [&'a str]) -> Vec<Hunk<'a>> {
    capture_diff_slices(Algorithm::Myers, base, side)
        .into_iter()
        .filter_map(|op| match op {
            DiffOp::Equal { .. } => None,
            DiffOp::Delete { old_index, old_len, .. } => Some(Hunk { start: old_index, end: old_index + old_len, lines: &[] }),
            DiffOp::Insert { old_index, new_index, new_len } => Some(Hunk { start: old_index, end: old_index, lines: &side[new_index..new_index + new_len] }),
            DiffOp::Replace { old_index, old_len, new_index, new_len } => Some(Hunk { start: old_index, end: old_index + old_len, lines: &side[new_index..new_index + new_len] }),
        })
        .collect()
}

/// Three-way merge by lines; in a chapter a line is a paragraph. `None` when both sides changed
/// the same place in different ways.
pub fn merge3(base: &str, ours: &str, theirs: &str) -> Option<String> {
    let lines = |text| str::split_inclusive(text, '\n').collect::<Vec<_>>();
    let (base, ours, theirs) = (lines(base), lines(ours), lines(theirs));
    let (a, b) = (hunks(&base, &ours), hunks(&base, &theirs));
    let (mut out, mut at, mut i, mut j) = (String::new(), 0, 0, 0);
    while i < a.len() || j < b.len() {
        let next = match (a.get(i), b.get(j)) {
            // The same change on both sides counts once.
            (Some(x), Some(y)) if (x.start, x.end, x.lines) == (y.start, y.end, y.lines) => {
                (i, j) = (i + 1, j + 1);
                x
            }
            (Some(x), Some(y)) => {
                // Changed lines in common, or an insertion where the other side changed: the
                // order of the two is not known.
                let insertion = x.start == x.end || y.start == y.end;
                if (x.start < y.end && y.start < x.end) || (insertion && x.start <= y.end && y.start <= x.end) {
                    return None;
                }
                if x.start < y.start {
                    i += 1;
                    x
                } else {
                    j += 1;
                    y
                }
            }
            (Some(x), None) => {
                i += 1;
                x
            }
            (None, Some(y)) => {
                j += 1;
                y
            }
            (None, None) => break,
        };
        out.extend(base[at..next.start].iter().copied());
        out.extend(next.lines.iter().copied());
        at = next.end;
    }
    out.extend(base[at..].iter().copied());
    Some(out)
}

/// The chapter list without the word counts: ids, titles, order and nesting.
fn shape(tree: &Tree) -> String {
    fn strip(value: &mut serde_json::Value) {
        match value {
            serde_json::Value::Object(map) => {
                map.remove("words");
                map.values_mut().for_each(strip);
            }
            serde_json::Value::Array(list) => list.iter_mut().for_each(strip),
            _ => {}
        }
    }
    let mut value = serde_json::to_value(tree).unwrap_or_default();
    strip(&mut value);
    value.to_string()
}
/// Puts the chapter or note `id` of `from` at the end of the same section of `tree`.
fn add_node(tree: &mut Tree, from: &Tree, id: &str) {
    if let (Some((section, _)), Some(node)) = (from.locate(id), from.find(id)) {
        let at = tree.section(section).len();
        let _ = tree.insert(section, &[], at, node.clone());
    }
}

/// Copies the snapshots that only the other computer has, as a new line, and records its
/// working text as the newest snapshot of that line. Returns the line and that snapshot.
fn import_line(history: &mut History, work: &WorkDir, their_history: &History, their_work: &WorkDir, theirs: &State, now: i64) -> Result<(String, SnapshotMeta), String> {
    let mut line = format!("Cloud {}", time::local(now));
    let mut n = 2;
    while history.lines.lines.iter().any(|l| l.name.eq_ignore_ascii_case(&line)) {
        line = format!("Cloud {} ({n})", time::local(now));
        n += 1;
    }
    for meta in &their_history.snapshots {
        if history.get(&meta.id).is_some() {
            continue;
        }
        // A snapshot whose file is damaged is left out; the rest still comes.
        let (Ok(detail), Ok(bytes)) = (their_history.detail(their_work, &meta.id), their_work.read(&snap_path(&meta.id))) else { continue };
        for blob in detail.items.values().map(|e| &e.blob).chain(&detail.comments) {
            if let Ok(text) = their_work.read(&blob_path(blob)) {
                work.write_once(&blob_path(blob), &text).map_err(err)?;
            }
        }
        work.write(&snap_path(&meta.id), &bytes).map_err(err)?;
        history.snapshots.push(SnapshotMeta { line: line.clone(), ..meta.clone() });
    }
    // The index stays oldest first.
    history.snapshots.sort_by_key(|s| s.time);
    history.rewrite_index(work).map_err(err)?;
    history.lines.lines.push(Line { name: line.clone(), created: now });
    history.save_lines(work).map_err(err)?;
    let head = history.record(work, theirs, Kind::Manual, "The version from the other computer", &line, now, None).map_err(err)?;
    Ok((line, head))
}

/// Merges the `.jw` at `jw`, a version of this project from another computer, into the project.
pub fn merge_file(history: &mut History, project: &mut Project, jw: &Path, now: i64) -> Result<Merged, String> {
    let folder = tempfile::tempdir().map_err(err)?;
    archive::unpack(jw, folder.path()).map_err(err)?;
    let their_work = WorkDir::at(folder.path().to_owned());
    let their_manifest = Manifest::parse(&their_work.read(MANIFEST).map_err(err)?)?;
    if their_manifest.id != project.manifest.id {
        return Err("That file is another project".into());
    }
    let their_history = History::load(&their_work);
    // The newest snapshot both computers have: where the two versions parted, or earlier.
    // When the other computer works on another line, its text is not a later form of the text
    // here: nothing is compared, so every chapter that differs stays as it is here.
    let same_line = their_history.current_line() == history.current_line();
    let common = history.snapshots.iter().rev().find(|s| same_line && their_history.get(&s.id).is_some()).map(|s| s.id.clone());
    let base = match common {
        Some(id) => Some(history.state(&project.work, &id).map_err(err)?),
        None => None,
    };
    let theirs = State::of(&their_work, &their_manifest, None).map_err(err)?;
    // The same text and no snapshot that is missing here: the other file is a copy of this one.
    let here = State::working(project, false).map_err(err)?;
    if theirs.signature() == here.signature() && their_history.snapshots.iter().all(|s| history.get(&s.id).is_some()) {
        return Ok(Merged::default());
    }
    if history.changed_since_head(project).map_err(err)? {
        history.capture(project, Kind::Safety, "Before merging the version from the other computer", now).map_err(err)?;
    }
    let ours = State::working(project, true).map_err(err)?;
    let theirs = State::of(&their_work, &their_manifest, Some(&project.work)).map_err(err)?;
    let (line, their_head) = import_line(history, &project.work, &their_history, &their_work, &theirs, now)?;
    let mut report = Merged { line, ..Merged::default() };

    let mut merged = ours.clone();
    if base.as_ref().is_some_and(|b| b.title == ours.title) {
        merged.title = theirs.title.clone();
    }
    // The chapter list: the side that changed it wins. When both did, the list here stays and
    // the chapters added there go to its end.
    let (here, there, before) = (shape(&ours.tree), shape(&theirs.tree), base.as_ref().map(|b| shape(&b.tree)));
    let in_base = |id: &str| base.as_ref().and_then(|b| b.items.get(id));
    if here != there && before.as_ref() == Some(&here) {
        merged.tree = theirs.tree.clone();
        // A chapter removed there but changed here stays.
        for (id, entry) in &ours.items {
            if merged.tree.find(id).is_none() && in_base(id).is_some_and(|b| b.blob != entry.blob) {
                add_node(&mut merged.tree, &ours.tree, id);
            }
        }
    } else if here != there {
        report.lists_differ = before.as_ref() != Some(&there);
        for id in theirs.items.keys() {
            if merged.tree.find(id).is_none() && in_base(id).is_none() {
                add_node(&mut merged.tree, &theirs.tree, id);
            }
        }
    }

    let leaves: Vec<(String, String)> = [Section::Manuscript, Section::Notes]
        .iter()
        .flat_map(|s| merged.tree.leaves(*s))
        .map(|(node, _)| (node.id().to_owned(), node.title().to_owned()))
        .collect();
    merged.items.clear();
    for (id, title) in leaves {
        let entry = match (ours.items.get(&id), theirs.items.get(&id), in_base(&id)) {
            (Some(o), Some(t), _) if o.blob == t.blob => o.clone(),
            (Some(o), Some(t), Some(b)) if b.blob == o.blob => {
                report.taken += 1;
                t.clone()
            }
            (Some(o), Some(t), Some(b)) if b.blob == t.blob => o.clone(),
            (Some(o), Some(_), b) => {
                let text = |state: &State| state.text(&project.work, &id).unwrap_or_default();
                let together = b.and(base.as_ref()).and_then(|base| merge3(&text(base), &text(&ours), &text(&theirs)));
                match together {
                    Some(text) => {
                        let blob = hash(text.as_bytes());
                        project.work.write_once(&blob_path(&blob), text.as_bytes()).map_err(err)?;
                        report.combined += 1;
                        ItemEntry { blob, words: notes::word_count(&text) as u32 }
                    }
                    None => {
                        report.conflicts.push(title);
                        o.clone()
                    }
                }
            }
            (Some(o), None, _) => o.clone(),
            (None, Some(t), _) => {
                report.taken += 1;
                t.clone()
            }
            (None, None, _) => continue,
        };
        if let Some(Node::Chapter { words, .. } | Node::Page { words, .. }) = merged.tree.find_mut(&id) {
            *words = entry.words;
        }
        merged.items.insert(id, entry);
    }

    if merged.signature() != ours.signature() {
        apply_state(project, &merged)?;
        reanchor_comments(project, &merged.items.keys().cloned().collect::<Vec<_>>())?;
    }
    // ponytail: review comments, book details and export presets stay as they are here; the
    // comments from there are in the snapshot on its line. Merge them when that is missed.
    history
        .capture_with(project, Kind::Restore, "Merged the version from the other computer", now, Some(their_head.id))
        .map_err(err)?;
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::super::store::testing::{Fixture, fixture};
    use super::*;

    #[test]
    fn merge3_joins_separate_changes_and_refuses_the_same_place() {
        let base = "one\ntwo\nthree\nfour\n";
        // Neighbouring paragraphs, a deletion and an addition at the end.
        assert_eq!(merge3(base, "ONE\ntwo\nthree\nfour\n", "one\nTWO\nthree\nfour\nfive\n").unwrap(), "ONE\nTWO\nthree\nfour\nfive\n");
        assert_eq!(merge3(base, "one\nthree\nfour\n", "one\ntwo\nthree\nFOUR\n").unwrap(), "one\nthree\nFOUR\n");
        // The same change on both sides counts once, beside a change on one side.
        assert_eq!(merge3(base, "one\ntwo\nthree\nfour\nfive\n", "zero\none\ntwo\nthree\nfour\nfive\n").unwrap(), "zero\none\ntwo\nthree\nfour\nfive\n");
        assert_eq!(merge3(base, base, "x\n").unwrap(), "x\n");
        // The same paragraph, and two additions at the same place.
        assert_eq!(merge3(base, "one\nmine\nthree\nfour\n", "one\ntheirs\nthree\nfour\n"), None);
        assert_eq!(merge3(base, "one\ntwo\nthree\nfour\nmine\n", "one\ntwo\nthree\nfour\ntheirs\n"), None);
        assert_eq!(merge3(base, "one\nnew\ntwo\nthree\nfour\n", "one\nTWO\nthree\nfour\n"), None);
    }

    /// The fixture project with a first snapshot, saved, and a copy of it open as if on another
    /// computer.
    fn two_computers() -> (Fixture, History, Project, History, tempfile::TempDir) {
        let mut f = fixture();
        let mut history = History::default();
        history.capture(&f.project, Kind::Manual, "Draft 1", 100).unwrap();
        f.project.pack(true).unwrap();
        let there = tempfile::tempdir().unwrap();
        let copy = there.path().join("book.jw");
        std::fs::copy(&f.project.path, &copy).unwrap();
        let Ok(opened) = Project::open(&there.path().join("data"), &copy, false) else { panic!("the copy opens") };
        let their_history = History::load(&opened.project.work);
        (f, history, opened.project, their_history, there)
    }

    #[test]
    fn the_other_computer_comes_in_as_a_line_and_is_merged_chapter_by_chapter() {
        let (mut f, mut history, mut theirs, mut their_history, _there) = two_computers();
        let (one, two, page) = (f.ids[0].clone(), f.ids[1].clone(), f.ids[2].clone());
        // There: the second paragraph of One, all of Two, the notes page, and a new chapter.
        theirs.write_text(&one, "The first chapter.\nSecond paragraph, rewritten there.\n").unwrap();
        theirs.write_text(&two, "Chapter two, their way.\n").unwrap();
        their_history.capture(&theirs, Kind::Auto, "", 200).unwrap();
        theirs.write_text(&page, "Anna: short.\n").unwrap();
        let p = theirs.add(Section::Manuscript, &[], 2, Node::chapter("Three")).unwrap();
        let three = theirs.manifest.tree.get(Section::Manuscript, &p).unwrap().id().to_owned();
        theirs.write_text(&three, "Written there.\n").unwrap();
        theirs.pack(true).unwrap();
        // Here: the first paragraph of One, and all of Two.
        f.project.write_text(&one, "The first chapter, improved here.\nSecond paragraph here.\n").unwrap();
        f.project.write_text(&two, "Chapter two, my way.\n").unwrap();

        let report = merge_file(&mut history, &mut f.project, &theirs.path, 300).unwrap();
        assert_eq!((report.taken, report.combined, report.lists_differ), (2, 1, false));
        assert_eq!(report.conflicts, ["Two"]);
        assert_eq!(f.project.read_text(&one).unwrap(), "The first chapter, improved here.\nSecond paragraph, rewritten there.\n");
        assert_eq!(f.project.read_text(&two).unwrap(), "Chapter two, my way.\n");
        assert_eq!(f.project.read_text(&page).unwrap(), "Anna: short.\n");
        assert_eq!(f.project.read_text(&three).unwrap(), "Written there.\n");
        assert_eq!(f.project.manifest.tree.find(&one).unwrap().words(), 9);

        // Nothing is lost: their text is on its line with their own snapshot, ours in a safety snapshot.
        let their_head = history.head(&report.line).unwrap().id.clone();
        let there = history.state(&f.project.work, &their_head).unwrap();
        assert_eq!(there.text(&f.project.work, &two).unwrap(), "Chapter two, their way.\n");
        assert!(their_history.snapshots.iter().all(|s| history.get(&s.id).is_some()));
        assert_eq!(history.on_line(&report.line).count(), 2);
        let safety = history.snapshots.iter().find(|s| s.kind == Kind::Safety).unwrap().id.clone();
        assert_eq!(history.state(&f.project.work, &safety).unwrap().text(&f.project.work, &one).unwrap(), "The first chapter, improved here.\nSecond paragraph here.\n");
        assert_eq!(history.current_line(), super::super::store::MAIN);
        assert_eq!(history.head(super::super::store::MAIN).unwrap().kind, Kind::Restore);
        // What is on disk is what a new session reads.
        assert_eq!(History::load(&f.project.work).snapshots.len(), history.snapshots.len());
    }

    #[test]
    fn chapter_lists_changed_on_both_sides_keep_ours_and_add_theirs() {
        let (mut f, mut history, mut theirs, _, _there) = two_computers();
        // A copy with nothing new is left alone: no line, no snapshot.
        assert_eq!(merge_file(&mut history, &mut f.project, &theirs.path, 250).unwrap(), Merged::default());
        assert_eq!((history.snapshots.len(), history.lines.lines.len()), (1, 1));
        let p = theirs.add(Section::Manuscript, &[], 0, Node::chapter("Theirs")).unwrap();
        let their_new = theirs.manifest.tree.get(Section::Manuscript, &p).unwrap().id().to_owned();
        theirs.write_text(&their_new, "From there.\n").unwrap();
        theirs.delete(&f.ids[1].clone()).unwrap();
        theirs.pack(true).unwrap();
        let p = f.project.add(Section::Manuscript, &[], 0, Node::chapter("Ours")).unwrap();
        let our_new = f.project.manifest.tree.get(Section::Manuscript, &p).unwrap().id().to_owned();

        let report = merge_file(&mut history, &mut f.project, &theirs.path, 300).unwrap();
        assert!(report.lists_differ && report.conflicts.is_empty());
        let order: Vec<&str> = f.project.manifest.tree.manuscript.iter().map(Node::id).collect();
        assert_eq!(order, [our_new.as_str(), f.ids[0].as_str(), f.ids[1].as_str(), their_new.as_str()]);
        assert_eq!(f.project.read_text(&their_new).unwrap(), "From there.\n");

        // Another project is refused.
        let other = fixture();
        assert!(merge_file(&mut history, &mut f.project, &other.project.path, 400).is_err());
    }
}
