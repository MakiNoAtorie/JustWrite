//! Importing `.docx`, `.md` and `.txt` files into a project, split into chapters.
//!
//! Every reader produces the same plain list of paragraphs; [`split`] then cuts it into chapters
//! by heading level, by a pattern, or at page breaks, and turns an existing comment marker (for
//! example lines starting with `#`) into notes. Text comes out in the neutral stored form.

pub mod docx;
pub mod split;
pub mod text;

use crate::{
    core::comments::CommentSet,
    jw::{
        Project,
        manifest::{COMMENTS, Node, Section},
    },
};
use std::path::Path;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Kind {
    Body,
    /// A heading of this level (1 is the highest).
    Heading(u8),
    SceneBreak,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawPara {
    /// With `*italic*` and `**bold**` markers; may hold comment-range placeholders.
    pub text: String,
    pub kind: Kind,
    /// A page break comes right before this paragraph.
    pub page_break_before: bool,
}
/// A Word comment found while reading.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawComment {
    pub id: usize,
    pub body: String,
    pub created: i64,
}
#[derive(Clone, Debug, Default)]
pub struct Raw {
    pub title: Option<String>,
    pub paras: Vec<RawPara>,
    pub comments: Vec<RawComment>,
}

pub fn read(path: &Path) -> Result<Raw, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("Cannot read {}: {e}", path.display()))?;
    let extension = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    let mut raw = match extension.as_str() {
        "docx" => docx::read(&bytes)?,
        "md" | "markdown" | "mdown" => text::read_markdown(&text::decode(&bytes)),
        _ => text::read_plain(&text::decode(&bytes), text::Paragraphs::Auto),
    };
    if raw.title.is_none() {
        raw.title = path.file_stem().map(|s| s.to_string_lossy().into_owned());
    }
    Ok(raw)
}

/// Creates a new project holding the imported chapters.
pub fn into_new_project(data: &Path, jw: &Path, imported: &split::Imported) -> Result<Project, String> {
    let mut project = Project::create(data, jw, &imported.title)?;
    // The starter chapter is replaced.
    let first = project.manifest.tree.manuscript[0].id().to_owned();
    project.delete(&first)?;
    append(&mut project, imported)?;
    project.pack(true).map_err(|e| e.to_string())?;
    Ok(project)
}

/// Adds the imported chapters at the end of an existing manuscript. Part headings are dropped:
/// there are no parts. Word comments become review comments.
pub fn append(project: &mut Project, imported: &split::Imported) -> Result<usize, String> {
    let mut comments = project.work.read(COMMENTS).map(|b| CommentSet::parse(&b)).unwrap_or_default();
    let mut count = 0;
    for item in &imported.items {
        match item {
            split::Item::Part(_) => {}
            split::Item::Chapter(chapter) => {
                let node = Node::chapter(&chapter.title);
                let id = node.id().to_owned();
                let at = project.manifest.tree.manuscript.len();
                project.add(Section::Manuscript, &[], at, node)?;
                project.write_text(&id, &chapter.text).map_err(|e| e.to_string())?;
                for c in &chapter.comments {
                    let _ = comments.add(&id, &chapter.text, c.range.clone(), &c.body, "", c.created);
                }
                count += 1;
            }
        }
    }
    if !comments.comments.is_empty() {
        project.work.write(COMMENTS, comments.to_json().as_bytes()).map_err(|e| e.to_string())?;
    }
    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::comments::CommentSet;

    #[test]
    fn a_markdown_file_becomes_a_project_and_more_chapters_can_be_added() {
        let data = tempfile::tempdir().unwrap();
        let out = tempfile::tempdir().unwrap();
        let file = out.path().join("My Novel.md");
        std::fs::write(&file, "# Part One\n\n## Dawn\n\nIt was *early*.\n\n## Noon\n\nIt was later.\n\n# Part Two\n\n## Night\n\nDark.\n").unwrap();
        let raw = read(&file).unwrap();
        assert_eq!(raw.title.as_deref(), Some("My Novel"));
        let imported = split::split(&raw, &split::suggest(&raw)).unwrap();
        let jw = out.path().join("novel.jw");
        let mut project = into_new_project(data.path(), &jw, &imported).unwrap();
        let tree = &project.manifest.tree.manuscript;
        // The part headings are dropped; the chapters stay in order.
        assert_eq!(tree.iter().map(|n| n.title()).collect::<Vec<_>>(), ["Dawn", "Noon", "Night"]);
        assert!(tree.iter().all(|n| !n.is_part()));
        assert_eq!(project.manifest.title, "My Novel");
        let dawn = tree[0].id().to_owned();
        assert_eq!(project.read_text(&dawn).unwrap(), "It was *early*.\n");
        assert_eq!(project.manuscript_words(), 3 + 3 + 1);
        // Importing into an existing project appends.
        let more = out.path().join("extra.txt");
        std::fs::write(&more, "CHAPTER 9\nExtra words here.\n").unwrap();
        let raw = read(&more).unwrap();
        let imported = split::split(&raw, &split::Options { by: split::By::Pattern("^CHAPTER".into()), ..Default::default() }).unwrap();
        assert_eq!(append(&mut project, &imported).unwrap(), 1);
        assert_eq!(project.manifest.tree.manuscript.len(), 4);
        assert_eq!(project.manifest.tree.manuscript[3].title(), "CHAPTER 9");
        project.close().unwrap();
        // It reopens from the file.
        let again = Project::open(data.path(), &jw, false).ok().unwrap();
        assert_eq!(again.project.manifest.tree.manuscript.len(), 4);
        assert!(read(&out.path().join("missing.docx")).is_err());
        let _ = CommentSet::default();
    }
}
