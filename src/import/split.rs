//! Cutting imported paragraphs into chapters and writing them in the stored form.

use super::{Kind, Raw};
use crate::core::notes;
use std::{collections::BTreeMap, ops::Range};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum By {
    /// A new chapter starts at each heading of this level or higher.
    Headings(u8),
    /// A new chapter starts at each paragraph matching this regular expression; it becomes the title.
    Pattern(String),
    PageBreaks,
    /// The whole file is one chapter.
    Whole,
}
#[derive(Clone, Debug)]
pub struct Options {
    pub by: By,
    /// With heading splitting: headings above the chapter level become parts.
    pub parts: bool,
    /// Lines starting with this (such as `#` or `//`) were the author's comments: they become
    /// notes. Empty for none.
    pub comment_prefix: String,
}
impl Default for Options {
    fn default() -> Self {
        Self { by: By::Whole, parts: false, comment_prefix: String::new() }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChapterComment {
    pub range: Range<usize>,
    pub body: String,
    pub created: i64,
}
#[derive(Clone, Debug)]
pub struct Chapter {
    pub title: String,
    /// Neutral text.
    pub text: String,
    pub comments: Vec<ChapterComment>,
}
#[derive(Clone, Debug)]
pub enum Item {
    Part(String),
    Chapter(Chapter),
}
#[derive(Clone, Debug)]
pub struct Imported {
    pub title: String,
    pub items: Vec<Item>,
}
impl Imported {
    pub fn chapters(&self) -> impl Iterator<Item = &Chapter> {
        self.items.iter().filter_map(|i| if let Item::Chapter(c) = i { Some(c) } else { None })
    }
}

/// A sensible way to split `raw`, for the dialog to start from.
pub fn suggest(raw: &Raw) -> Options {
    let mut counts: BTreeMap<u8, usize> = BTreeMap::new();
    for p in &raw.paras {
        if let Kind::Heading(n) = p.kind {
            *counts.entry(n).or_default() += 1;
        }
    }
    // Levels that occur more than once are candidates. The shallowest is the chapter level,
    // unless it is rare and a deeper one is at least as common: then it holds the parts.
    let candidates: Vec<(u8, usize)> = counts.iter().filter(|(_, c)| **c >= 2).map(|(l, c)| (*l, *c)).collect();
    let chapters = match candidates.as_slice() {
        [] => None,
        [only] => Some((only.0, false)),
        [first, second, ..] if first.1 <= 8 && second.1 >= first.1 => Some((second.0, true)),
        [first, ..] => Some((first.0, false)),
    };
    match chapters {
        Some((level, parts)) => Options { by: By::Headings(level), parts, comment_prefix: String::new() },
        None if raw.paras.iter().any(|p| p.page_break_before) => Options { by: By::PageBreaks, ..Options::default() },
        None => Options::default(),
    }
}

fn plain(text: &str) -> String {
    text.replace('*', "").trim().to_owned()
}

struct Builder {
    title: String,
    lines: Vec<String>,
}
impl Builder {
    fn new(title: &str) -> Self {
        Self { title: title.to_owned(), lines: Vec::new() }
    }
}

fn is_placeholder(c: char) -> bool {
    (c as u32) >= 0x100000
}

pub fn split(raw: &Raw, options: &Options) -> Result<Imported, String> {
    let pattern = match &options.by {
        By::Pattern(p) => Some(regex_lite::Regex::new(p).map_err(|e| format!("That pattern is not valid: {e}"))?),
        _ => None,
    };
    let mut current: Option<Builder> = None;
    enum Slot {
        Part(String),
        Chapter(Builder),
    }
    let mut slots: Vec<Slot> = Vec::new();
    let comment_line = |text: &str| -> Option<String> {
        let prefix = options.comment_prefix.trim();
        let t = text.trim_start();
        (!prefix.is_empty() && t.starts_with(prefix)).then(|| {
            let rest = t[prefix.len()..].trim();
            format!("{}{}{}", notes::LINE, if rest.is_empty() { "" } else { " " }, rest)
        })
    };
    let flush = |current: &mut Option<Builder>, slots: &mut Vec<Slot>| {
        if let Some(b) = current.take() {
            slots.push(Slot::Chapter(b));
        }
    };
    for para in &raw.paras {
        let boundary: Option<(bool, String)> = match (&options.by, &para.kind) {
            (By::Headings(level), Kind::Heading(n)) if n < level && options.parts => Some((true, plain(&para.text))),
            (By::Headings(level), Kind::Heading(n)) if n <= level => Some((false, plain(&para.text))),
            (By::Pattern(_), Kind::Body) if pattern.as_ref().is_some_and(|re| re.is_match(&plain(&para.text))) => Some((false, plain(&para.text))),
            (By::PageBreaks, _) if para.page_break_before && current.as_ref().is_some_and(|c| !c.lines.is_empty()) => Some((false, String::new())),
            _ => None,
        };
        if let Some((is_part, title)) = boundary {
            flush(&mut current, &mut slots);
            if is_part {
                slots.push(Slot::Part(title));
            } else {
                current = Some(Builder::new(&title));
            }
            // A page break starts a chapter but is not a heading: the paragraph is its text.
            if !matches!(options.by, By::PageBreaks) {
                continue;
            }
        }
        let line = match &para.kind {
            Kind::SceneBreak => "#".to_owned(),
            Kind::Heading(_) | Kind::Body => comment_line(&para.text).unwrap_or_else(|| para.text.clone()),
        };
        if line.trim().is_empty() && para.kind != Kind::SceneBreak {
            continue;
        }
        current.get_or_insert_with(|| Builder::new("")).lines.push(line);
    }
    flush(&mut current, &mut slots);

    // Name chapters, drop an untitled empty lead, and turn the lines into stored text.
    let comments: BTreeMap<usize, &super::RawComment> = raw.comments.iter().map(|c| (c.id, c)).collect();
    let mut out: Vec<Item> = Vec::new();
    let mut number = 0;
    let count = slots.iter().filter(|s| matches!(s, Slot::Chapter(_))).count();
    // A part with no chapter under it (a document title, say) is not kept.
    let keep: Vec<bool> = slots
        .iter()
        .enumerate()
        .map(|(i, s)| !matches!(s, Slot::Part(_)) || matches!(slots.get(i + 1), Some(Slot::Chapter(_))))
        .collect();
    let mut first_slot = true;
    let mut document_title: Option<String> = None;
    for (slot, keep) in slots.into_iter().zip(keep) {
        if !keep {
            first_slot = false;
            continue;
        }
        let lead = first_slot;
        first_slot = false;
        if lead {
            if let Slot::Chapter(b) = &slot {
                // A heading before anything else, with nothing under it, names the document.
                if !b.title.is_empty() && b.lines.is_empty() && count > 1 {
                    document_title = Some(b.title.clone());
                    continue;
                }
            }
        }
        match slot {
            Slot::Part(title) => out.push(Item::Part(if title.is_empty() { "Part".into() } else { title })),
            Slot::Chapter(b) => {
                if b.title.is_empty() && b.lines.is_empty() {
                    continue;
                }
                number += 1;
                let title = if !b.title.is_empty() {
                    b.title.clone()
                } else if lead && count > 1 && !matches!(options.by, By::PageBreaks) {
                    "Front matter".into()
                } else if count == 1 {
                    raw.title.clone().unwrap_or_else(|| "Chapter 1".into())
                } else {
                    format!("Chapter {number}")
                };
                let mut text = b.lines.join("\n");
                text.push('\n');
                // Comment ranges were marked by private placeholders.
                let mut clean = String::with_capacity(text.len());
                let (mut starts, mut ranges) = (BTreeMap::new(), Vec::new());
                let mut position = 0usize;
                for c in text.chars() {
                    if is_placeholder(c) {
                        let n = c as u32 - 0x100000;
                        let id = (n / 2) as usize;
                        if n % 2 == 0 {
                            starts.insert(id, position);
                        } else if let Some(start) = starts.remove(&id) {
                            ranges.push((id, start..position));
                        }
                    } else {
                        clean.push(c);
                        position += 1;
                    }
                }
                let chars: Vec<char> = clean.chars().collect();
                let comments = ranges
                    .into_iter()
                    .map(|(id, mut r)| {
                        // A range around emphasised words should hold the words, not the asterisks.
                        let lead = chars[r.start..r.end].iter().take_while(|c| **c == '*').count();
                        let trail = chars[r.start..r.end].iter().rev().take_while(|c| **c == '*').count();
                        let n = lead.min(trail).min(3);
                        if r.len() > 2 * n {
                            r = r.start + n..r.end - n;
                        }
                        (id, r)
                    })
                    .filter(|(_, r)| r.start < r.end)
                    .filter_map(|(id, range)| comments.get(&id).map(|c| ChapterComment { range, body: c.body.clone(), created: c.created }))
                    .collect();
                out.push(Item::Chapter(Chapter { title: title.chars().take(120).collect(), text: clean, comments }));
            }
        }
    }
    let title = raw.title.clone().or(document_title).unwrap_or_else(|| "Imported manuscript".into());
    Ok(Imported { title, items: out })
}

#[cfg(test)]
mod tests {
    use super::super::text::{Paragraphs, read_markdown, read_plain};
    use super::*;

    fn titles(i: &Imported) -> Vec<String> {
        i.items
            .iter()
            .map(|i| match i {
                Item::Part(t) => format!("[{t}]"),
                Item::Chapter(c) => c.title.clone(),
            })
            .collect()
    }

    #[test]
    fn markdown_splits_by_heading_with_parts() {
        let md = "# Book\n\n# Part One\n\n## Chapter 1\n\nText one.\n\n## Chapter 2\n\nText *two*.\n\n# Part Two\n\n## Chapter 3\n\nText three.\n";
        let raw = read_markdown(md);
        let options = suggest(&raw);
        assert_eq!(options.by, By::Headings(2));
        assert!(options.parts);
        let imported = split(&raw, &options).unwrap();
        // The title heading has nothing under it, so it is not a part.
        assert_eq!(titles(&imported), ["[Part One]", "Chapter 1", "Chapter 2", "[Part Two]", "Chapter 3"]);
        let two = imported.chapters().nth(1).unwrap();
        assert_eq!(two.text, "Text *two*.\n");
        // Without parts the higher headings are chapters too.
        let flat = split(&raw, &Options { by: By::Headings(2), parts: false, ..Options::default() }).unwrap();
        assert_eq!(flat.chapters().count(), 5);
    }

    #[test]
    fn a_single_title_heading_is_not_a_part() {
        let raw = read_markdown("# The Book\n\n## One\n\nA.\n\n## Two\n\nB.\n");
        let options = suggest(&raw);
        assert_eq!(options.by, By::Headings(2));
        assert!(!options.parts);
        let imported = split(&raw, &options).unwrap();
        assert_eq!(imported.chapters().count(), 2);
    }

    #[test]
    fn plain_text_splits_by_pattern_and_converts_comment_lines_to_notes() {
        let txt = "CHAPTER ONE\nIt began.\n# remember the storm\nThen it rained.\n***\nLater.\nCHAPTER TWO\nIt ended.\n";
        let raw = read_plain(txt, Paragraphs::Lines);
        let options = Options { by: By::Pattern("^CHAPTER".into()), comment_prefix: "#".into(), ..Options::default() };
        let imported = split(&raw, &options).unwrap();
        assert_eq!(titles(&imported), ["CHAPTER ONE", "CHAPTER TWO"]);
        let one = imported.chapters().next().unwrap();
        assert_eq!(one.text, format!("It began.\n{} remember the storm\nThen it rained.\n#\nLater.\n", notes::LINE));
        assert_eq!(notes::word_count(&one.text), 6);
        assert!(split(&raw, &Options { by: By::Pattern("(".into()), ..Options::default() }).is_err());
        // Other markers work the same way.
        let raw = read_plain("Text\n// aside\nMore\n", Paragraphs::Lines);
        let imported = split(&raw, &Options { comment_prefix: "//".into(), ..Options::default() }).unwrap();
        assert_eq!(imported.chapters().next().unwrap().text, format!("Text\n{} aside\nMore\n", notes::LINE));
    }

    #[test]
    fn page_breaks_and_whole_files() {
        let mut raw = read_plain("A\nB\nC\nD\n", Paragraphs::Lines);
        raw.paras[2].page_break_before = true;
        let imported = split(&raw, &suggest(&raw)).unwrap();
        assert_eq!(titles(&imported), ["Chapter 1", "Chapter 2"]);
        assert_eq!(imported.chapters().nth(1).unwrap().text, "C\nD\n");
        raw.title = Some("My Story".into());
        let whole = split(&raw, &Options::default()).unwrap();
        assert_eq!(titles(&whole), ["My Story"]);
        assert_eq!(whole.chapters().next().unwrap().text, "A\nB\nC\nD\n");
        assert!(split(&Raw::default(), &Options::default()).unwrap().items.is_empty());
    }

    #[test]
    fn comment_range_placeholders_become_anchors() {
        let mut raw = read_plain("One two three\nFour five\n", Paragraphs::Lines);
        let start = char::from_u32(0x100000).unwrap();
        let end = char::from_u32(0x100001).unwrap();
        raw.paras[0].text = format!("One {start}two{end} three");
        raw.comments.push(super::super::RawComment { id: 0, body: "Why two?".into(), created: 5 });
        let imported = split(&raw, &Options::default()).unwrap();
        let c = imported.chapters().next().unwrap();
        assert_eq!(c.text, "One two three\nFour five\n");
        assert_eq!(c.comments, [ChapterComment { range: 4..7, body: "Why two?".into(), created: 5 }]);
    }
}
