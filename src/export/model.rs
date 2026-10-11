//! The document handed to every exporter: chapters of paragraphs of styled runs, with notes
//! removed (or kept for personal copies), scene breaks marked, comment ranges placed and the
//! front and back matter resolved.

use super::preset::{self, ChapterHeading, CommentsMode, Preset};
use crate::{
    core::{
        comments::{Comment, CommentSet, Status},
        notes::{BLOCK_CLOSE, BLOCK_OPEN, CLOSE, LINE, OPEN, word_count},
        prose::{Emphasis, char_emphasis, is_scene_break},
        typography::Language,
    },
    history::State,
    jw::{
        Project,
        book::BookDetails,
        manifest::{COMMENTS, ProjectSettings, Section, Tree},
    },
};
use std::{collections::HashMap, io, ops::Range};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Run {
    pub text: String,
    pub italic: bool,
    pub bold: bool,
    /// Part of a note (only present when notes are included).
    pub note: bool,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Inline {
    Run(Run),
    CommentStart(usize),
    CommentEnd(usize),
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Block {
    Paragraph(Vec<Inline>),
    SceneBreak,
}
impl Block {
    pub fn text(&self) -> String {
        match self {
            Block::Paragraph(inlines) => inlines
                .iter()
                .filter_map(|i| match i {
                    Inline::Run(r) => Some(r.text.as_str()),
                    _ => None,
                })
                .collect(),
            Block::SceneBreak => String::new(),
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PartHeader {
    pub number: usize,
    pub title: String,
}
#[derive(Clone, Debug)]
pub struct Chapter {
    pub id: String,
    /// Position in the whole manuscript, so a single exported chapter keeps its number.
    pub number: usize,
    pub title: String,
    /// Set on the first chapter of a part.
    pub part: Option<PartHeader>,
    pub blocks: Vec<Block>,
    pub words: usize,
}
#[derive(Clone, Debug)]
pub struct ExportComment {
    pub chapter: String,
    pub quote: String,
    pub body: String,
    pub replies: Vec<(i64, String)>,
    pub created: i64,
    pub resolved: bool,
    pub tag: String,
}

/// Everything read from a project (or a snapshot of it) that an export may use.
pub struct Source {
    pub title: String,
    pub book: BookDetails,
    pub settings: ProjectSettings,
    pub tree: Tree,
    /// Neutral text of each chapter and page by id.
    pub texts: HashMap<String, String>,
    pub comments: CommentSet,
    pub cover: Option<(Vec<u8>, String)>,
    pub now: i64,
}
impl Source {
    pub fn from_project(project: &Project, now: i64) -> io::Result<Source> {
        let mut texts = HashMap::new();
        for section in [Section::Manuscript, Section::Notes] {
            for (node, _) in project.manifest.tree.leaves(section) {
                texts.insert(node.id().to_owned(), project.read_text(node.id()).unwrap_or_default());
            }
        }
        let comments = project.work.read(COMMENTS).map(|b| CommentSet::parse(&b)).unwrap_or_default();
        Ok(Source {
            title: project.manifest.title.clone(),
            book: project.book.clone(),
            settings: project.manifest.settings.clone(),
            tree: project.manifest.tree.clone(),
            texts,
            comments,
            cover: cover_of(project),
            now,
        })
    }
    /// A snapshot's text; book details and settings are the project's current ones.
    pub fn from_state(project: &Project, state: &State, now: i64) -> io::Result<Source> {
        let mut texts = HashMap::new();
        for id in state.items.keys() {
            texts.insert(id.clone(), state.text(&project.work, id).unwrap_or_default());
        }
        let comments = state.comments_json(&project.work).map(|b| CommentSet::parse(&b)).unwrap_or_default();
        Ok(Source {
            title: state.title.clone(),
            book: project.book.clone(),
            settings: project.manifest.settings.clone(),
            tree: state.tree.clone(),
            texts,
            comments,
            cover: cover_of(project),
            now,
        })
    }
}
fn cover_of(project: &Project) -> Option<(Vec<u8>, String)> {
    let bytes = project.cover_bytes()?;
    let ext = std::path::Path::new(&project.book.cover).extension()?.to_str()?.to_ascii_lowercase();
    Some((bytes, ext))
}

/// What to export.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Scope {
    /// Every chapter of the manuscript.
    Whole,
    /// These chapters (any order; they are exported in manuscript order).
    Chapters(Vec<String>),
}

#[derive(Clone, Debug)]
pub struct Doc {
    pub title: String,
    pub subtitle: String,
    pub author: String,
    pub surname: String,
    pub running_title: String,
    pub language: Language,
    pub isbn: String,
    pub publisher: String,
    pub series: String,
    pub description: String,
    pub subjects: Vec<String>,
    /// Contact details, one line each.
    pub contact: Vec<String>,
    pub words: u64,
    pub date: i64,
    pub year: i32,
    pub cover: Option<(Vec<u8>, String)>,
    pub dedication: Vec<Vec<Inline>>,
    pub epigraph: Vec<Vec<Inline>>,
    pub copyright: Vec<String>,
    pub acknowledgments: Vec<Vec<Inline>>,
    pub about_author: Vec<Vec<Inline>>,
    pub chapters: Vec<Chapter>,
    /// Comments placed in the text, indexed by the numbers in [`Inline::CommentStart`].
    pub comments: Vec<ExportComment>,
    /// Comments listed after the last chapter.
    pub appendix: Vec<ExportComment>,
    /// The preset's scene-break text.
    pub scene_break: String,
}

fn year_of(seconds: i64) -> i32 {
    crate::jw::time::iso(seconds).get(..4).and_then(|y| y.parse().ok()).unwrap_or(2026)
}

/// One neutral char with its offset, as seen while reading a line.
struct Ch {
    c: char,
    offset: usize,
    note: bool,
}

/// Splits neutral text into blocks. `marks` are comment ranges in chars of `text`.
pub fn parse_text(text: &str, keep_notes: bool, marks: &[(usize, Range<usize>)]) -> Vec<Block> {
    #[derive(Clone, Copy, PartialEq)]
    enum State {
        Pending,
        Open,
        Done,
    }
    let mut state: Vec<State> = vec![State::Pending; marks.len()];
    let mut blocks = Vec::new();
    let mut offset = 0usize;
    let mut in_block = false;
    for line in text.split_inclusive('\n') {
        let body = line.trim_end_matches(['\r', '\n']);
        let line_end = offset + body.chars().count();
        // Read the line into characters with their note status.
        let mut chars: Vec<Ch> = Vec::new();
        let mut inline = false;
        let mut line_note = false;
        for (i, c) in body.chars().enumerate() {
            let at = offset + i;
            match c {
                BLOCK_OPEN => in_block = true,
                BLOCK_CLOSE => in_block = false,
                LINE => line_note = true,
                OPEN => inline = true,
                CLOSE => inline = false,
                c => chars.push(Ch { c, offset: at, note: in_block || line_note || inline }),
            }
        }
        // Block notes continue over following lines; a line inside one has no prose.
        let line_start = offset;
        offset += line.chars().count();

        // Emphasis over the prose of the line only.
        let prose: String = chars.iter().filter(|c| !c.note).map(|c| c.c).collect();
        let styles = char_emphasis(&prose);
        let mut prose_at = 0;
        struct V {
            c: char,
            offset: usize,
            emphasis: Emphasis,
            note: bool,
        }
        let mut visible: Vec<V> = Vec::new();
        let mut removed_note = false;
        for ch in &chars {
            if ch.note {
                if keep_notes {
                    visible.push(V { c: ch.c, offset: ch.offset, emphasis: Emphasis::default(), note: true });
                } else {
                    removed_note = true;
                }
                continue;
            }
            let style = styles[prose_at];
            prose_at += 1;
            let Some(emphasis) = style else { continue };
            // Removing a note between two spaces must not leave a double space.
            if removed_note && ch.c.is_whitespace() && visible.last().is_none_or(|v| v.c.is_whitespace()) {
                continue;
            }
            removed_note = false;
            visible.push(V { c: ch.c, offset: ch.offset, emphasis, note: false });
        }
        // Trim the paragraph.
        while visible.last().is_some_and(|v| v.c.is_whitespace()) {
            visible.pop();
        }
        let lead = visible.iter().take_while(|v| v.c.is_whitespace()).count();
        visible.drain(..lead);

        let prose_text: String = visible.iter().filter(|v| !v.note).map(|v| v.c).collect();
        if visible.is_empty() {
            // A blank line, or a line that held only a note.
            continue;
        }
        // A scene's name (`## The storm ##`) is for the editor only.
        if crate::core::prose::scene_name(&prose_text).is_some() {
            continue;
        }
        if !prose_text.is_empty() && is_scene_break(&prose_text) {
            blocks.push(Block::SceneBreak);
            if keep_notes && visible.iter().any(|v| v.note) {
                let run = Run { text: visible.iter().filter(|v| v.note).map(|v| v.c).collect(), italic: false, bold: false, note: true };
                blocks.push(Block::Paragraph(vec![Inline::Run(run)]));
            }
            continue;
        }
        if prose_text.is_empty() && keep_notes {
            // A line of notes only: kept as its own paragraph.
            let run = Run { text: visible.iter().map(|v| v.c).collect(), italic: false, bold: false, note: true };
            blocks.push(Block::Paragraph(vec![Inline::Run(run)]));
            continue;
        }
        // Group into runs, placing comment markers.
        let mut inlines: Vec<Inline> = Vec::new();
        let mut current: Option<Run> = None;
        let flush = |current: &mut Option<Run>, inlines: &mut Vec<Inline>| {
            if let Some(run) = current.take() {
                inlines.push(Inline::Run(run));
            }
        };
        for v in &visible {
            for (k, (id, range)) in marks.iter().enumerate() {
                if state[k] == State::Pending && v.offset >= range.start {
                    flush(&mut current, &mut inlines);
                    inlines.push(Inline::CommentStart(*id));
                    state[k] = State::Open;
                }
                if state[k] == State::Open && v.offset >= range.end {
                    flush(&mut current, &mut inlines);
                    inlines.push(Inline::CommentEnd(*id));
                    state[k] = State::Done;
                }
            }
            let (italic, bold) = (v.emphasis.italic, v.emphasis.bold);
            match &mut current {
                Some(run) if run.italic == italic && run.bold == bold && run.note == v.note => run.text.push(v.c),
                _ => {
                    flush(&mut current, &mut inlines);
                    current = Some(Run { text: v.c.to_string(), italic, bold, note: v.note });
                }
            }
        }
        flush(&mut current, &mut inlines);
        for (k, (id, range)) in marks.iter().enumerate() {
            if state[k] == State::Open && range.end <= line_end.max(line_start) {
                inlines.push(Inline::CommentEnd(*id));
                state[k] = State::Done;
            }
        }
        blocks.push(Block::Paragraph(inlines));
    }
    // A range that ran to the very end of the text is closed with the last paragraph.
    for (k, (id, _)) in marks.iter().enumerate() {
        if state[k] == State::Open {
            if let Some(Block::Paragraph(inlines)) = blocks.iter_mut().rev().find(|b| matches!(b, Block::Paragraph(_))) {
                inlines.push(Inline::CommentEnd(*id));
            }
        }
    }
    blocks
}

/// Runs of spaces and tabs inside a paragraph become one space, also across style changes.
pub fn collapse_spaces(blocks: &mut [Block]) {
    for block in blocks {
        let Block::Paragraph(inlines) = block else { continue };
        let mut after_space = false;
        for inline in inlines {
            let Inline::Run(run) = inline else { continue };
            let mut cleaned = String::with_capacity(run.text.len());
            for c in run.text.chars() {
                let space = c == ' ' || c == '\t';
                if space && after_space {
                    continue;
                }
                after_space = space;
                cleaned.push(if space { ' ' } else { c });
            }
            run.text = cleaned;
        }
    }
}

/// Plain paragraphs of front and back matter (author-written, with `*emphasis*`).
pub fn parse_matter(text: &str) -> Vec<Vec<Inline>> {
    parse_text(text, false, &[])
        .into_iter()
        .filter_map(|b| match b {
            Block::Paragraph(i) => Some(i),
            Block::SceneBreak => None,
        })
        .collect()
}

/// Whether `title` is only the chapter's number ("Chapter 3", "3"), so no title line is wanted.
fn number_of_chapter_title(title: &str, language: Language) -> bool {
    let t = title.trim().to_lowercase();
    let word = preset::chapter_word(language).to_lowercase();
    t.strip_prefix(&word).is_some_and(|rest| rest.trim().chars().all(|c| c.is_ascii_digit() || c.is_alphabetic() && rest.len() < 4)) || t.chars().all(|c| c.is_ascii_digit()) && !t.is_empty()
}

/// The heading of a chapter as the preset wants it: (number line, title line).
pub fn heading_lines(chapter: &Chapter, preset: &Preset, language: Language) -> (Option<String>, Option<String>) {
    let number = || {
        format!(
            "{} {}",
            preset::chapter_word(language),
            preset::number_text(chapter.number, preset.number_style, language)
        )
    };
    let title = || {
        let t = chapter.title.trim();
        (!t.is_empty() && !number_of_chapter_title(t, language)).then(|| t.to_owned())
    };
    match preset.chapter_heading {
        ChapterHeading::None => (None, None),
        ChapterHeading::Number => (Some(number()), None),
        ChapterHeading::Title => (None, title().or_else(|| Some(number()))),
        ChapterHeading::NumberAndTitle => (Some(number()), title()),
    }
}
/// "Chapter 3: Title" on one line, for plain text, Markdown and tables of contents.
pub fn heading_line(chapter: &Chapter, preset: &Preset, language: Language) -> String {
    match heading_lines(chapter, preset, language) {
        (Some(n), Some(t)) => format!("{n}: {t}"),
        (Some(n), None) => n,
        (None, Some(t)) => t,
        (None, None) => String::new(),
    }
}
pub fn part_heading(part: &PartHeader, preset: &Preset, language: Language) -> String {
    let word = preset::part_word(language);
    let number = preset::number_text(part.number, preset.number_style, language);
    if part.title.trim().is_empty() || part.title.trim().to_lowercase().starts_with(&word.to_lowercase()) {
        format!("{word} {number}")
    } else {
        format!("{word} {number}: {}", part.title.trim())
    }
}

fn export_comment(c: &Comment, chapter_title: &str) -> ExportComment {
    ExportComment {
        chapter: chapter_title.to_owned(),
        quote: c.quote.clone(),
        body: c.body.clone(),
        replies: c.replies.iter().map(|r| (r.created, r.body.clone())).collect(),
        created: c.created,
        resolved: c.status == Status::Resolved,
        tag: c.tag.clone(),
    }
}

/// Builds the document for `scope` according to `preset`.
pub fn build(source: &Source, scope: &Scope, preset: &Preset) -> Doc {
    let language = source.settings.language;
    let leaves = source.tree.leaves(Section::Manuscript);
    let mut chapters = Vec::new();
    let (mut number, mut part_number) = (0usize, 0usize);
    let mut last_part: Option<String> = None;
    let mut comments: Vec<ExportComment> = Vec::new();
    let mut appendix: Vec<ExportComment> = Vec::new();
    let mut words = 0u64;
    let effective_comments = match (preset.comments, preset.format) {
        (CommentsMode::Docx, super::preset::Format::Docx) => CommentsMode::Docx,
        (CommentsMode::Off, _) => CommentsMode::Off,
        _ => CommentsMode::Appendix,
    };
    for (node, part) in leaves {
        if node.is_part() {
            continue;
        }
        number += 1;
        let first_of_part = part.map(|p| p.id().to_owned()).filter(|id| last_part.as_deref() != Some(id));
        if let Some(part) = part {
            if first_of_part.is_some() {
                part_number += 1;
                last_part = Some(part.id().to_owned());
            }
        } else {
            last_part = None;
        }
        let included = match scope {
            Scope::Whole => true,
            Scope::Chapters(ids) => ids.iter().any(|i| i == node.id()),
        };
        if !included {
            continue;
        }
        let text = source.texts.get(node.id()).map(String::as_str).unwrap_or("");
        // Comments of this chapter: anchored ones become ranges, the rest go to the appendix.
        let mut marks: Vec<(usize, Range<usize>)> = Vec::new();
        for c in source.comments.comments.iter().filter(|c| c.item == node.id()) {
            if preset.comments_open_only && c.status == Status::Resolved || effective_comments == CommentsMode::Off {
                continue;
            }
            let entry = export_comment(c, node.title());
            if effective_comments == CommentsMode::Docx && !c.orphaned && c.anchor.start < c.anchor.end {
                marks.push((comments.len(), c.anchor.start..c.anchor.end));
                comments.push(entry);
            } else {
                appendix.push(entry);
            }
        }
        let mut blocks = parse_text(text, preset.include_notes, &marks);
        if preset.clean_whitespace {
            collapse_spaces(&mut blocks);
        }
        let chapter_words = word_count(text);
        words += chapter_words as u64;
        chapters.push(Chapter {
            id: node.id().to_owned(),
            number,
            title: node.title().to_owned(),
            part: part
                .filter(|p| first_of_part.as_deref() == Some(p.id()))
                .map(|p| PartHeader { number: part_number, title: p.title().to_owned() }),
            blocks,
            words: chapter_words,
        });
    }
    let book = &source.book;
    let mut contact = Vec::new();
    if !book.legal_name.trim().is_empty() {
        contact.push(book.legal_name.trim().to_owned());
    }
    contact.extend(book.address.lines().map(str::trim).filter(|l| !l.is_empty()).map(str::to_owned));
    for field in [&book.phone, &book.email, &book.website] {
        if !field.trim().is_empty() {
            contact.push(field.trim().to_owned());
        }
    }
    let year = year_of(source.now);
    Doc {
        title: book.display_title(&source.title).to_owned(),
        subtitle: book.subtitle.trim().to_owned(),
        author: book.author().to_owned(),
        surname: book.surname(),
        running_title: book.running_title(&source.title),
        language,
        isbn: book.isbn.trim().to_owned(),
        publisher: book.publisher.trim().to_owned(),
        series: book.series.trim().to_owned(),
        description: book.description.trim().to_owned(),
        subjects: book.subjects.split(',').map(str::trim).filter(|s| !s.is_empty()).map(str::to_owned).collect(),
        contact,
        words,
        date: source.now,
        year,
        cover: source.cover.clone(),
        dedication: if preset.dedication { parse_matter(&book.dedication) } else { Vec::new() },
        epigraph: if preset.epigraph { parse_matter(&book.epigraph) } else { Vec::new() },
        copyright: if preset.copyright_page {
            book.copyright_text(&source.title, year).lines().map(str::to_owned).collect()
        } else {
            Vec::new()
        },
        acknowledgments: if preset.acknowledgments { parse_matter(&book.acknowledgments) } else { Vec::new() },
        about_author: if preset.about_author { parse_matter(&book.about_author) } else { Vec::new() },
        chapters,
        comments,
        appendix,
        scene_break: preset.scene_break.clone(),
    }
}

#[cfg(test)]
pub(crate) mod testing {
    use super::*;
    use crate::{
        core::notes::{Marker, to_neutral},
        jw::manifest::{Node, ProjectSettings},
    };

    /// A small manuscript: a prologue, a part with two chapters, and a notes page.
    pub fn source() -> Source {
        let marker = Marker::default();
        let n = |s: &str| to_neutral(s, &marker).0;
        let mut tree = Tree::default();
        let m = Section::Manuscript;
        let prologue = Node::chapter("Prologue");
        let one = Node::chapter("The Storm");
        let two = Node::chapter("Chapter 3");
        let page = Node::page("Characters");
        let mut texts = HashMap::new();
        texts.insert(prologue.id().to_owned(), n("It began with a *whisper*.\nThen **silence** fell. $$ cut this?\n#\nA second scene.\n"));
        texts.insert(one.id().to_owned(), n("\"Run,\" she said.\n$$ remember the lighthouse\nHe ran. $$ aside $$ Far.\n$$$\nblock of notes\n$$$\nThe end.\n"));
        texts.insert(two.id().to_owned(), n("Chapter three text goes here.\n"));
        texts.insert(page.id().to_owned(), n("Anna: tall.\n"));
        tree.insert(m, &[], 0, prologue).unwrap();
        tree.insert(m, &[], 1, Node::part("The Sea")).unwrap();
        tree.insert(m, &[1], 0, one).unwrap();
        tree.insert(m, &[1], 1, two).unwrap();
        tree.insert(Section::Notes, &[], 0, page).unwrap();
        Source {
            title: "The Long Night".into(),
            book: BookDetails {
                legal_name: "Jane Q. Doe".into(),
                pen_name: "J. Q. Smith".into(),
                email: "jane@example.com".into(),
                phone: "555-0100".into(),
                address: "12 Harbour Road\nPortsmouth".into(),
                short_title: "Night".into(),
                isbn: "978-3-16-148410-0".into(),
                publisher: "Lantern Press".into(),
                description: "A storm, a lighthouse, a secret.".into(),
                subjects: "Fiction, Sea stories".into(),
                dedication: "For *Anna*.".into(),
                acknowledgments: "Thanks to everyone.\nAnd the sea.".into(),
                about_author: "J. Q. Smith lives by the sea.".into(),
                ..Default::default()
            },
            settings: ProjectSettings::default(),
            tree,
            texts,
            comments: CommentSet::default(),
            cover: None,
            now: 1_760_000_000,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::testing::source;
    use super::*;
    use crate::export::preset::builtins;

    fn text_of(c: &Chapter) -> Vec<String> {
        c.blocks.iter().map(|b| if *b == Block::SceneBreak { "#".to_owned() } else { b.text() }).collect()
    }

    #[test]
    fn notes_are_stripped_and_whitespace_cleaned() {
        let src = source();
        let preset = Preset::default();
        let doc = build(&src, &Scope::Whole, &preset);
        assert_eq!(doc.chapters.len(), 3);
        assert_eq!(text_of(&doc.chapters[0]), ["It began with a whisper.", "Then silence fell.", "#", "A second scene."]);
        assert_eq!(text_of(&doc.chapters[1]), ["\"Run,\" she said.", "He ran. Far.", "The end."]);
        // Emphasis survives as runs.
        let Block::Paragraph(inlines) = &doc.chapters[0].blocks[0] else { panic!() };
        let runs: Vec<_> = inlines.iter().filter_map(|i| if let Inline::Run(r) = i { Some((r.text.as_str(), r.italic)) } else { None }).collect();
        assert_eq!(runs, [("It began with a ", false), ("whisper", true), (".", false)]);
        assert_eq!(doc.words, 11 + 8 + 5);
        assert_eq!(doc.chapters.iter().map(|c| c.words).collect::<Vec<_>>(), [11, 8, 5]);
    }

    #[test]
    fn personal_copies_can_keep_the_notes() {
        let src = source();
        let preset = Preset { include_notes: true, ..Preset::default() };
        let doc = build(&src, &Scope::Whole, &preset);
        let texts = text_of(&doc.chapters[1]);
        assert!(texts.iter().any(|t| t == "remember the lighthouse"), "{texts:?}");
        assert!(texts.iter().any(|t| t == "block of notes"));
        let Block::Paragraph(inlines) = &doc.chapters[1].blocks[2] else { panic!() };
        assert!(inlines.iter().any(|i| matches!(i, Inline::Run(r) if r.note && r.text.contains("aside"))));
    }

    #[test]
    fn chapter_numbers_and_parts_follow_the_whole_manuscript() {
        let src = source();
        let ids: Vec<String> = src.tree.leaves(Section::Manuscript).iter().map(|(n, _)| n.id().to_owned()).collect();
        let doc = build(&src, &Scope::Chapters(vec![ids[2].clone()]), &Preset::default());
        assert_eq!(doc.chapters.len(), 1);
        assert_eq!(doc.chapters[0].number, 3);
        assert!(doc.chapters[0].part.is_none(), "only the first chapter of a part carries its header");
        let all = build(&src, &Scope::Whole, &Preset::default());
        assert_eq!(all.chapters[1].part.as_ref().map(|p| (p.number, p.title.as_str())), Some((1, "The Sea")));
        assert!(all.chapters[0].part.is_none());
        // The notes page is never part of a manuscript export.
        assert!(all.chapters.iter().all(|c| c.title != "Characters"));
    }

    #[test]
    fn headings_follow_the_preset_and_skip_default_titles() {
        let src = source();
        let doc = build(&src, &Scope::Whole, &Preset::default());
        let p = |h| Preset { chapter_heading: h, ..Preset::default() };
        let lang = Language::English;
        assert_eq!(heading_line(&doc.chapters[1], &p(ChapterHeading::NumberAndTitle), lang), "Chapter 2: The Storm");
        assert_eq!(heading_line(&doc.chapters[1], &p(ChapterHeading::Number), lang), "Chapter 2");
        assert_eq!(heading_line(&doc.chapters[1], &p(ChapterHeading::Title), lang), "The Storm");
        assert_eq!(heading_line(&doc.chapters[1], &p(ChapterHeading::None), lang), "");
        // A title that only repeats the number is not printed twice.
        assert_eq!(heading_lines(&doc.chapters[2], &p(ChapterHeading::NumberAndTitle), lang), (Some("Chapter 3".into()), None));
        assert_eq!(heading_line(&doc.chapters[1], &p(ChapterHeading::NumberAndTitle), Language::Spanish), "Capítulo 2: The Storm");
        let part = doc.chapters[1].part.as_ref().unwrap();
        assert_eq!(part_heading(part, &Preset::default(), lang), "Part 1: The Sea");
    }

    #[test]
    fn comments_become_ranges_or_an_appendix() {
        let mut src = source();
        let id = src.tree.leaves(Section::Manuscript)[0].0.id().to_owned();
        let text = src.texts[&id].clone();
        let start = text.find("whisper").unwrap();
        let c1 = src.comments.add(&id, &text, start..start + 7, "Too soft?", "prose", 5).unwrap();
        let c2 = src.comments.add(&id, &text, 0..2, "Resolved one", "", 6).unwrap();
        src.comments.set_status(&c2, Status::Resolved);
        let c3 = src.comments.add(&id, &text, 3..6, "Orphan", "", 7).unwrap();
        src.comments.get_mut(&c3).unwrap().orphaned = true;
        src.comments.reply(&c1, "Fixed", 9).unwrap();
        let docx = Preset { comments: CommentsMode::Docx, format: super::super::preset::Format::Docx, ..Preset::default() };
        let doc = build(&src, &Scope::Whole, &docx);
        assert_eq!(doc.comments.len(), 1);
        assert_eq!(doc.comments[0].body, "Too soft?");
        assert_eq!(doc.comments[0].replies.len(), 1);
        assert_eq!(doc.appendix.len(), 1, "orphans go to the appendix, resolved ones are left out");
        let Block::Paragraph(inlines) = &doc.chapters[0].blocks[0] else { panic!() };
        let order: Vec<String> = inlines
            .iter()
            .map(|i| match i {
                Inline::Run(r) => r.text.clone(),
                Inline::CommentStart(n) => format!("<{n}"),
                Inline::CommentEnd(n) => format!("{n}>"),
            })
            .collect();
        assert_eq!(order, ["It began with a ", "<0", "whisper", "0>", "."]);
        // Other formats list all comments in an appendix.
        let md = Preset { comments: CommentsMode::Docx, format: super::super::preset::Format::Markdown, ..Preset::default() };
        let doc = build(&src, &Scope::Whole, &md);
        assert!(doc.comments.is_empty());
        assert_eq!(doc.appendix.len(), 2);
        let off = build(&src, &Scope::Whole, &Preset::default());
        assert!(off.comments.is_empty() && off.appendix.is_empty());
        let all = Preset { comments: CommentsMode::Appendix, comments_open_only: false, ..Preset::default() };
        assert_eq!(build(&src, &Scope::Whole, &all).appendix.len(), 3);
    }

    #[test]
    fn matter_and_metadata_feed_from_book_details() {
        let src = source();
        let preset = builtins().into_iter().find(|p| p.id == preset::PRINT_BOOK).unwrap();
        let doc = build(&src, &Scope::Whole, &preset);
        assert_eq!((doc.author.as_str(), doc.surname.as_str(), doc.running_title.as_str()), ("J. Q. Smith", "Smith", "NIGHT"));
        assert_eq!(doc.contact, ["Jane Q. Doe", "12 Harbour Road", "Portsmouth", "555-0100", "jane@example.com"]);
        assert_eq!(doc.dedication.len(), 1);
        assert_eq!(doc.acknowledgments.len(), 2);
        assert!(doc.copyright.iter().any(|l| l.contains("ISBN 978-3-16-148410-0")));
        assert_eq!(doc.subjects, ["Fiction", "Sea stories"]);
        assert_eq!(doc.year, 2025);
        // Matter the preset leaves out is not carried.
        let bare = build(&src, &Scope::Whole, &Preset::default());
        assert!(bare.dedication.is_empty() && bare.copyright.is_empty() && bare.about_author.is_empty());
    }

    #[test]
    fn parsing_never_panics_on_hostile_text() {
        let alphabet = ["*", "**", "#", "\n", "\r\n", "\u{E000}", "\u{E001}", "\u{E002}", "\u{E003}", "\u{E004}", "a", " ", "é", "\\"];
        let mut seed = 3u64;
        for _ in 0..5000 {
            let mut text = String::new();
            for _ in 0..(seed % 20) {
                seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
                text.push_str(alphabet[(seed >> 33) as usize % alphabet.len()]);
            }
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            let len = text.chars().count();
            let marks = vec![(0usize, (seed as usize % (len + 1))..(len))];
            for keep in [false, true] {
                let _ = parse_text(&text, keep, &marks);
            }
        }
    }
}

#[cfg(test)]
mod whitespace_tests {
    use super::*;

    #[test]
    fn extra_spaces_collapse_across_runs() {
        let mut blocks = parse_text("A  *b*   c\t\td", false, &[]);
        collapse_spaces(&mut blocks);
        let Block::Paragraph(inlines) = &blocks[0] else { panic!() };
        let text: String = inlines.iter().filter_map(|i| if let Inline::Run(r) = i { Some(r.text.as_str()) } else { None }).collect();
        assert_eq!(text, "A b c d");
    }
}
