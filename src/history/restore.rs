//! Restoring and switching lines. Both are non-destructive: what is there is saved first, and a
//! restore is itself recorded as a snapshot.

use super::store::{History, Kind, State};
use crate::jw::{
    Project,
    manifest::{COMMENTS, Node, Section},
};
use std::io;

fn err(e: io::Error) -> String {
    e.to_string()
}

/// Makes the working copy match `state` (tree, text and comments).
fn apply_state(project: &mut Project, state: &State) -> Result<(), String> {
    // Files of chapters that are not part of that state go; the history still has their text.
    let current: Vec<String> = [Section::Manuscript, Section::Notes]
        .iter()
        .flat_map(|s| project.manifest.tree.leaves(*s).into_iter().filter_map(|(n, _)| n.file()).collect::<Vec<_>>())
        .collect();
    for file in current {
        project.work.remove(&file).map_err(err)?;
    }
    project.manifest.tree = state.tree.clone();
    project.manifest.title = state.title.clone();
    for (id, entry) in &state.items {
        let Some(file) = project.manifest.tree.find(id).and_then(Node::file) else { continue };
        let bytes = project.work.read(&format!("history/blobs/{}", entry.blob)).map_err(err)?;
        project.work.write(&file, &bytes).map_err(err)?;
    }
    project.save_manifest().map_err(err)
}

/// Re-anchors the comments after text changed under them. Comments are never dropped.
fn reanchor_comments(project: &Project, items: &[String]) -> Result<(), String> {
    use crate::core::comments::CommentSet;
    let mut set = project.work.read(COMMENTS).map(|b| CommentSet::parse(&b)).unwrap_or_default();
    if set.comments.is_empty() {
        return Ok(());
    }
    for id in items {
        let text = project.read_text(id).unwrap_or_default();
        set.reanchor_item(id, &text);
    }
    // Comments on items that no longer exist stay, orphaned.
    for c in &mut set.comments {
        if project.manifest.tree.find(&c.item).is_none() {
            c.orphaned = true;
        }
    }
    project.work.write(COMMENTS, set.to_json().as_bytes()).map_err(err)
}

/// Restores the whole project (tree, text) to `snapshot`.
pub fn restore_project(history: &mut History, project: &mut Project, snapshot: &str, now: i64) -> Result<(), String> {
    let state = history.state(&project.work, snapshot).map_err(err)?;
    let message = history.get(snapshot).map(|m| m.message.clone()).unwrap_or_default();
    if history.changed_since_head(project).map_err(err)? {
        history.capture(project, Kind::Safety, "Before restoring an older version", now).map_err(err)?;
    }
    apply_state(project, &state)?;
    let items: Vec<String> = state.items.keys().cloned().collect();
    reanchor_comments(project, &items)?;
    let label = if message.is_empty() { crate::jw::time::local(history.get(snapshot).map_or(now, |m| m.time)) } else { message };
    history
        .capture_with(project, Kind::Restore, &format!("Restored \"{label}\""), now, Some(snapshot.to_owned()))
        .map_err(err)?;
    Ok(())
}

/// Restores one chapter or page to its text in `snapshot`. A chapter that was deleted since is
/// brought back at the end of its section.
pub fn restore_item(history: &mut History, project: &mut Project, snapshot: &str, item: &str, now: i64) -> Result<(), String> {
    let state = history.state(&project.work, snapshot).map_err(err)?;
    let text = state.text(&project.work, item).ok_or("That chapter is not in the snapshot")?;
    let node = state.tree.find(item).cloned().ok_or("That chapter is not in the snapshot")?;
    if history.changed_since_head(project).map_err(err)? {
        history.capture(project, Kind::Safety, &format!("Before restoring \"{}\"", node.title()), now).map_err(err)?;
    }
    if project.manifest.tree.find(item).is_none() {
        let (section, _) = state.tree.locate(item).ok_or("That chapter is not in the snapshot")?;
        let at = project.manifest.tree.section(section).len();
        project.manifest.tree.insert(section, &[], at, node.clone())?;
    }
    project.write_text(item, &text).map_err(err)?;
    project.save_manifest().map_err(err)?;
    reanchor_comments(project, &[item.to_owned()])?;
    let label = history.get(snapshot).map(|m| m.message.clone()).filter(|m| !m.is_empty()).unwrap_or_else(|| "an older version".into());
    history
        .capture_with(project, Kind::Restore, &format!("Restored \"{}\" from \"{label}\"", node.title()), now, Some(snapshot.to_owned()))
        .map_err(err)?;
    Ok(())
}

/// Makes `name` the current line and loads its newest snapshot. Work in progress on the line
/// being left is saved first.
pub fn switch_line(history: &mut History, project: &mut Project, name: &str, now: i64) -> Result<(), String> {
    if history.lines.current == name {
        return Ok(());
    }
    let head = history.head(name).ok_or_else(|| format!("There is no line called \"{name}\""))?.id.clone();
    if history.changed_since_head(project).map_err(err)? {
        history.capture(project, Kind::Safety, &format!("Before switching to \"{name}\""), now).map_err(err)?;
    }
    let state = history.state(&project.work, &head).map_err(err)?;
    apply_state(project, &state)?;
    let items: Vec<String> = state.items.keys().cloned().collect();
    reanchor_comments(project, &items)?;
    history.set_current_line(&project.work, name).map_err(err)
}

#[cfg(test)]
mod tests {
    use super::super::store::testing::fixture;
    use super::*;
    use crate::core::comments::CommentSet;

    #[test]
    fn restoring_the_project_keeps_everything_and_records_itself() {
        let mut f = fixture();
        let (one, two) = (f.ids[0].clone(), f.ids[1].clone());
        let mut history = History::default();
        let draft = history.capture(&f.project, Kind::Manual, "Draft 1 done", 100).unwrap();
        // Rewrite chapter one, delete chapter two, add a chapter three.
        f.project.write_text(&one, "A completely new opening.\n").unwrap();
        f.project.delete(&two).unwrap();
        let p = f.project.add(Section::Manuscript, &[], 1, Node::chapter("Three")).unwrap();
        let three = f.project.manifest.tree.get(Section::Manuscript, &p).unwrap().id().to_owned();
        f.project.write_text(&three, "Brand new.\n").unwrap();
        restore_project(&mut history, &mut f.project, &draft.id, 200).unwrap();
        // The text is back, the later chapter is gone from the tree.
        assert_eq!(f.project.read_text(&one).unwrap(), "The first chapter.\nSecond paragraph here.\n");
        assert_eq!(f.project.read_text(&two).unwrap(), "Chapter two begins.\n");
        assert!(f.project.manifest.tree.find(&three).is_none());
        // History was not rewritten: the draft, a safety snapshot of what was there, and the restore.
        let kinds: Vec<Kind> = history.snapshots.iter().map(|s| s.kind).collect();
        assert_eq!(kinds, [Kind::Manual, Kind::Safety, Kind::Restore]);
        assert_eq!(history.snapshots[2].restored_from.as_deref(), Some(draft.id.as_str()));
        // The work that was undone is still reachable.
        let safety = history.state(&f.project.work, &history.snapshots[1].id.clone()).unwrap();
        assert_eq!(safety.text(&f.project.work, &three).unwrap(), "Brand new.\n");
        assert_eq!(safety.text(&f.project.work, &one).unwrap(), "A completely new opening.\n");
        // Restoring the safety snapshot undoes the restore.
        let id = history.snapshots[1].id.clone();
        restore_project(&mut history, &mut f.project, &id, 300).unwrap();
        assert_eq!(f.project.read_text(&one).unwrap(), "A completely new opening.\n");
        // Nothing had changed since the previous restore, so no new safety snapshot was needed.
        assert_eq!(history.snapshots.len(), 4);
    }

    #[test]
    fn restoring_one_chapter_leaves_the_rest_alone_and_brings_back_deleted_ones() {
        let mut f = fixture();
        let (one, two) = (f.ids[0].clone(), f.ids[1].clone());
        let mut history = History::default();
        let draft = history.capture(&f.project, Kind::Manual, "Draft 1", 100).unwrap();
        f.project.write_text(&one, "Rewritten one.\n").unwrap();
        f.project.write_text(&two, "Rewritten two.\n").unwrap();
        restore_item(&mut history, &mut f.project, &draft.id, &one, 200).unwrap();
        assert_eq!(f.project.read_text(&one).unwrap(), "The first chapter.\nSecond paragraph here.\n");
        assert_eq!(f.project.read_text(&two).unwrap(), "Rewritten two.\n");
        assert_eq!(f.project.manifest.tree.find(&one).unwrap().words(), 6);
        // A deleted chapter comes back with its title.
        f.project.delete(&two).unwrap();
        restore_item(&mut history, &mut f.project, &draft.id, &two, 300).unwrap();
        assert_eq!(f.project.manifest.tree.find(&two).unwrap().title(), "Two");
        assert_eq!(f.project.read_text(&two).unwrap(), "Chapter two begins.\n");
        assert!(restore_item(&mut history, &mut f.project, &draft.id, "nope", 400).is_err());
    }

    #[test]
    fn comments_survive_a_restore_re_anchored_or_orphaned() {
        let mut f = fixture();
        let one = f.ids[0].clone();
        let mut history = History::default();
        let text = f.project.read_text(&one).unwrap();
        let start = text.find("Second").unwrap();
        let mut set = CommentSet::default();
        let id = set.add(&one, &text, start..start + 6, "Check", "plot", 1).unwrap();
        f.project.work.write(COMMENTS, set.to_json().as_bytes()).unwrap();
        let draft = history.capture(&f.project, Kind::Manual, "d", 100).unwrap();
        // The paragraph is deleted and the comment orphaned by the editor.
        f.project.write_text(&one, "Only this remains.\n").unwrap();
        restore_item(&mut history, &mut f.project, &draft.id, &one, 200).unwrap();
        let back = CommentSet::parse(&f.project.work.read(COMMENTS).unwrap());
        let c = back.get(&id).unwrap();
        assert!(!c.orphaned);
        assert_eq!(&text[start..start + 6], "Second");
        assert_eq!(c.anchor.start, start);
        // Restoring a state in which the commented text never existed orphans, never deletes.
        f.project.write_text(&one, "Wholly different words.\n").unwrap();
        let other = history.capture(&f.project, Kind::Manual, "other", 300).unwrap();
        restore_project(&mut history, &mut f.project, &other.id, 400).unwrap();
        let back = CommentSet::parse(&f.project.work.read(COMMENTS).unwrap());
        assert_eq!(back.comments.len(), 1);
        assert!(back.get(&id).unwrap().orphaned);
    }

    #[test]
    fn switching_lines_swaps_the_text_and_keeps_work_in_progress() {
        let mut f = fixture();
        let one = f.ids[0].clone();
        let mut history = History::default();
        history.capture(&f.project, Kind::Manual, "Draft 1", 100).unwrap();
        history.new_line(&f.project, "Alt ending", 200).unwrap();
        f.project.write_text(&one, "The alternate ending text.\n").unwrap();
        // Unsaved work on the alternate line is kept when switching away.
        switch_line(&mut history, &mut f.project, "Main", 300).unwrap();
        assert_eq!(history.current_line(), "Main");
        assert_eq!(f.project.read_text(&one).unwrap(), "The first chapter.\nSecond paragraph here.\n");
        switch_line(&mut history, &mut f.project, "Alt ending", 400).unwrap();
        assert_eq!(f.project.read_text(&one).unwrap(), "The alternate ending text.\n");
        assert!(switch_line(&mut history, &mut f.project, "Missing", 500).is_err());
        let alt: Vec<_> = history.on_line("Alt ending").map(|s| s.kind).collect();
        assert_eq!(alt, [Kind::Manual, Kind::Safety]);
    }
}
