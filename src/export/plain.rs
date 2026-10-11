//! Plain text and Markdown.

use super::{
    model::{Block, Chapter, Doc, Inline, Run, heading_line, part_heading},
    preset::{Format, HeadingStyle, Preset},
};
use crate::jw::time;

fn runs(inlines: &[Inline], markdown: bool) -> String {
    let mut out = String::new();
    for inline in inlines {
        let Inline::Run(Run { text, italic, bold, note }) = inline else { continue };
        if *note {
            out.push_str(&format!("[note: {text}]"));
        } else if markdown && (*italic || *bold) && !text.trim().is_empty() {
            // Emphasis markers hug the words; the spaces stay outside.
            let lead = text.len() - text.trim_start().len();
            let trail = text.len() - text.trim_end().len();
            let mark = match (*bold, *italic) {
                (true, true) => "***",
                (true, false) => "**",
                _ => "*",
            };
            out.push_str(&text[..lead]);
            out.push_str(mark);
            out.push_str(text.trim());
            out.push_str(mark);
            out.push_str(&text[text.len() - trail..]);
        } else {
            out.push_str(text);
        }
    }
    out
}

/// Escapes what Markdown would otherwise read as syntax at the start or inside of prose.
fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for (i, c) in text.chars().enumerate() {
        match c {
            '\\' | '`' | '<' | '>' | '[' | ']' => {
                out.push('\\');
                out.push(c);
            }
            '#' | '-' | '+' | '|' if i == 0 => {
                out.push('\\');
                out.push(c);
            }
            _ => out.push(c),
        }
    }
    out
}
fn escape_runs(inlines: &[Inline]) -> Vec<Inline> {
    inlines
        .iter()
        .map(|i| match i {
            Inline::Run(r) if !r.note => Inline::Run(Run { text: escape_inline(&r.text), ..r.clone() }),
            other => other.clone(),
        })
        .collect()
}
fn escape_inline(text: &str) -> String {
    // `*` in prose is never emphasis here (it was parsed already), so it is escaped.
    text.replace('\\', "\\\\").replace('*', "\\*").replace('`', "\\`").replace('<', "\\<").replace('>', "\\>").replace('[', "\\[").replace(']', "\\]")
}

fn heading(text: &str, level: usize, style: HeadingStyle, markdown: bool, out: &mut String) {
    if text.is_empty() {
        return;
    }
    match (markdown, style) {
        (true, HeadingStyle::Hash) | (true, HeadingStyle::Plain) => out.push_str(&format!("{} {}\n\n", "#".repeat(level), escape(text).trim_start_matches('\\'))),
        (_, HeadingStyle::Underline) => out.push_str(&format!("{text}\n{}\n\n", if level == 1 { "=" } else { "-" }.repeat(text.chars().count()))),
        (_, HeadingStyle::Upper) => out.push_str(&format!("{}\n\n", text.to_uppercase())),
        _ => out.push_str(&format!("{text}\n\n")),
    }
}

fn paragraphs(blocks: &[Block], markdown: bool, scene_break: &str, out: &mut String) {
    for block in blocks {
        match block {
            Block::SceneBreak => out.push_str(&format!("{scene_break}\n\n")),
            Block::Paragraph(inlines) => {
                let text = if markdown { runs(&escape_runs(inlines), true) } else { runs(inlines, false) };
                // Keep a paragraph that begins like a list or heading from becoming one.
                let text = if markdown && text.starts_with(|c: char| c.is_ascii_digit()) && text.contains(". ") && text.find(". ") < Some(4) {
                    text.replacen(". ", "\\. ", 1)
                } else if markdown && text.starts_with(['-', '+', '#', '=']) {
                    format!("\\{text}")
                } else {
                    text
                };
                out.push_str(&text);
                out.push_str("\n\n");
            }
            Block::Quote(inlines, quote) => {
                let text = if markdown { runs(&escape_runs(inlines), true) } else { runs(inlines, false) };
                out.push_str(format!("{}{text}", if markdown { "> " } else { "    " }).trim_end());
                // In Markdown two spaces end the line, so the lines of a block stay apart.
                out.push_str(if quote.last { "\n\n" } else if markdown { "  \n" } else { "\n" });
            }
        }
    }
}
fn matter(title: &str, paras: &[Vec<Inline>], preset: &Preset, markdown: bool, out: &mut String) {
    if paras.is_empty() {
        return;
    }
    heading(title, 1, preset.heading_style, markdown, out);
    for p in paras {
        out.push_str(&if markdown { runs(&escape_runs(p), true) } else { runs(p, false) });
        out.push_str("\n\n");
    }
}

/// The text of the export. Paragraphs are separated by one blank line.
pub fn write(doc: &Doc, preset: &Preset) -> String {
    let markdown = preset.format == Format::Markdown;
    let mut out = String::new();
    let language = doc.language;
    if preset.title_page {
        heading(&doc.title, 1, preset.heading_style, markdown, &mut out);
        if !doc.subtitle.is_empty() {
            out.push_str(&format!("{}\n\n", if markdown { format!("*{}*", doc.subtitle) } else { doc.subtitle.clone() }));
        }
        if !doc.author.is_empty() {
            out.push_str(&format!("{}\n\n", if language == crate::core::typography::Language::Spanish { format!("por {}", doc.author) } else { format!("by {}", doc.author) }));
        }
        if preset.rounded_word_count {
            out.push_str(&format!("{}\n\n", super::preset::rounded_words(doc.words, language)));
        }
        if preset.title_page_contact && !doc.contact.is_empty() {
            out.push_str(&doc.contact.join(if markdown { "  \n" } else { "\n" }));
            out.push_str("\n\n");
        }
    }
    matter(if language == crate::core::typography::Language::Spanish { "Dedicatoria" } else { "Dedication" }, &doc.dedication, preset, markdown, &mut out);
    matter(if language == crate::core::typography::Language::Spanish { "Epígrafe" } else { "Epigraph" }, &doc.epigraph, preset, markdown, &mut out);
    if !doc.copyright.is_empty() {
        for line in &doc.copyright {
            out.push_str(line);
            out.push_str(if markdown { "  \n" } else { "\n" });
        }
        out.push('\n');
    }
    if preset.toc && !doc.chapters.is_empty() {
        heading(if language == crate::core::typography::Language::Spanish { "Contenido" } else { "Contents" }, 1, preset.heading_style, markdown, &mut out);
        for c in &doc.chapters {
            let line = heading_line(c, preset, language);
            let line = if line.is_empty() { c.title.clone() } else { line };
            out.push_str(&format!("{}{}\n", if markdown { "- " } else { "" }, line));
        }
        out.push('\n');
    }
    for chapter in &doc.chapters {
        write_chapter(chapter, doc, preset, markdown, &mut out);
    }
    matter(if language == crate::core::typography::Language::Spanish { "Agradecimientos" } else { "Acknowledgments" }, &doc.acknowledgments, preset, markdown, &mut out);
    matter(if language == crate::core::typography::Language::Spanish { "Sobre el autor" } else { "About the author" }, &doc.about_author, preset, markdown, &mut out);
    if !doc.appendix.is_empty() {
        heading(if language == crate::core::typography::Language::Spanish { "Comentarios" } else { "Comments" }, 1, preset.heading_style, markdown, &mut out);
        let mut last = String::new();
        for c in &doc.appendix {
            if c.chapter != last {
                heading(&c.chapter, 2, preset.heading_style, markdown, &mut out);
                last = c.chapter.clone();
            }
            let quote: String = c.quote.chars().take(160).collect::<String>().replace('\n', " ");
            let tag = if c.tag.is_empty() { String::new() } else { format!(" ({})", c.tag) };
            out.push_str(&format!("> {quote}\n\n{}{tag} — {}\n\n", c.body, time::date(c.created)));
            for (when, reply) in &c.replies {
                out.push_str(&format!("  Reply, {}: {reply}\n\n", time::date(*when)));
            }
        }
    }
    let mut out = out.trim_end().to_owned();
    out.push('\n');
    out
}

fn write_chapter(chapter: &Chapter, doc: &Doc, preset: &Preset, markdown: bool, out: &mut String) {
    if let (Some(part), true) = (&chapter.part, preset.part_headings) {
        heading(&part_heading(part, preset, doc.language), 1, preset.heading_style, markdown, out);
    }
    let line = heading_line(chapter, preset, doc.language);
    heading(&line, if chapter.part.is_some() && preset.part_headings { 2 } else { 1 }, preset.heading_style, markdown, out);
    paragraphs(&chapter.blocks, markdown, &preset.scene_break, out);
}

#[cfg(test)]
mod tests {
    use super::super::{
        model::{Scope, build, testing::source},
        preset::{self, builtins},
    };
    use super::*;

    fn preset(id: &str) -> Preset {
        builtins().into_iter().find(|p| p.id == id).unwrap()
    }

    #[test]
    fn markdown_has_headings_emphasis_and_scene_breaks() {
        let src = source();
        let p = preset(preset::MARKDOWN);
        let doc = build(&src, &Scope::Whole, &p);
        let text = write(&doc, &p);
        assert!(text.starts_with("# The Long Night\n\nby J. Q. Smith\n\n"), "{text}");
        assert!(text.contains("It began with a *whisper*."));
        assert!(text.contains("Then **silence** fell."));
        assert!(text.contains("\n***\n"), "scene break symbol from the preset");
        assert!(text.contains("# Part 1: The Sea\n\n## Chapter 2: The Storm\n\n"), "{text}");
        assert!(!text.contains("cut this") && !text.contains("lighthouse\n"), "notes are stripped");
        assert!(!text.contains("\n\n\n"), "single blank lines only");
        assert!(text.ends_with(".\n"));
    }

    #[test]
    fn plain_text_has_uppercase_headings_and_no_markup() {
        let src = source();
        let p = Preset { toc: true, ..preset(preset::PLAIN) };
        let doc = build(&src, &Scope::Whole, &p);
        let text = write(&doc, &p);
        assert!(text.contains("THE LONG NIGHT"));
        assert!(text.contains("CHAPTER 2: THE STORM"));
        assert!(text.contains("CONTENTS\n\nChapter 1: Prologue"));
        assert!(text.contains("It began with a whisper."), "no emphasis markers in plain text");
        assert!(text.contains("\n#\n"), "the manuscript scene break");
        assert!(!text.contains('*'));
    }

    #[test]
    fn notes_comments_and_matter_can_be_included() {
        let mut src = source();
        let id = src.tree.leaves(crate::jw::manifest::Section::Manuscript)[0].0.id().to_owned();
        let text = src.texts[&id].clone();
        src.comments.add(&id, &text, 0..2, "Open with something stronger", "prose", 100).unwrap();
        let p = Preset { include_notes: true, comments: preset::CommentsMode::Appendix, dedication: true, ..preset(preset::MARKDOWN) };
        let doc = build(&src, &Scope::Whole, &p);
        let out = write(&doc, &p);
        assert!(out.contains("[note: remember the lighthouse]"));
        assert!(out.contains("# Dedication\n\nFor *Anna*."));
        assert!(out.contains("# Comments\n\n## Prologue\n\n> It"), "{out}");
        assert!(out.contains("Open with something stronger (prose)"));
    }

    #[test]
    fn markdown_special_characters_in_prose_are_escaped() {
        let marker = crate::core::notes::Marker::default();
        let mut src = source();
        let id = src.tree.leaves(crate::jw::manifest::Section::Manuscript)[0].0.id().to_owned();
        src.texts.insert(id, crate::core::notes::to_neutral("- not a list\n1. not numbered\n<b>tag</b> and [link](x) \\* star\n", &marker).0);
        let p = preset(preset::MARKDOWN);
        let out = write(&build(&src, &Scope::Whole, &p), &p);
        assert!(out.contains("\\- not a list"), "{out}");
        assert!(out.contains("1\\. not numbered"));
        assert!(out.contains("\\<b\\>tag\\</b\\> and \\[link\\](x)"));
    }
}
