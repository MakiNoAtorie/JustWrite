//! Reading Word documents: paragraphs with their emphasis, headings by outline level or style,
//! page breaks, and the review comments with the text they point at.

use super::{Kind, Raw, RawComment, RawPara};
use crate::core::prose::is_scene_break;
use quick_xml::{Reader, events::Event};
use std::{collections::HashMap, io::Read};

#[derive(Default, Clone)]
struct Style {
    name: String,
    based_on: Option<String>,
    outline: Option<u8>,
    bold: Option<bool>,
    italic: Option<bool>,
    underline: Option<bool>,
}

fn local(name: &str) -> &str {
    name.rsplit(':').next().unwrap_or(name)
}
fn attr(e: &quick_xml::events::BytesStart, key: &str) -> Option<String> {
    e.attributes().flatten().find(|a| local(a.key.as_ref()) == key).and_then(|a| a.normalized_value(quick_xml::XmlVersion::default()).ok().map(|v| v.into_owned()))
}
/// `w:b`, `w:b w:val="0"`, ...: whether the property is on.
fn toggle(e: &quick_xml::events::BytesStart) -> bool {
    !matches!(attr(e, "val").as_deref(), Some("0" | "false" | "off" | "none"))
}

fn read_part(archive: &mut zip::ZipArchive<std::io::Cursor<&[u8]>>, name: &str) -> Option<String> {
    let mut file = archive.by_name(name).ok()?;
    let mut text = String::new();
    file.read_to_string(&mut text).ok()?;
    Some(text)
}

fn parse_styles(xml: &str) -> HashMap<String, Style> {
    let mut styles = HashMap::new();
    let mut reader = Reader::from_str(xml);
    let mut current: Option<(String, Style)> = None;
    let mut in_rpr = false;
    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) | Ok(Event::Empty(e)) => match local(e.name().as_ref()) {
                "style" => current = attr(&e, "styleId").map(|id| (id, Style::default())),
                "name" => {
                    if let Some((_, s)) = &mut current {
                        s.name = attr(&e, "val").unwrap_or_default();
                    }
                }
                "basedOn" => {
                    if let Some((_, s)) = &mut current {
                        s.based_on = attr(&e, "val");
                    }
                }
                "outlineLvl" => {
                    if let Some((_, s)) = &mut current {
                        s.outline = attr(&e, "val").and_then(|v| v.parse().ok());
                    }
                }
                "rPr" => in_rpr = true,
                "b" if in_rpr => {
                    if let Some((_, s)) = &mut current {
                        s.bold = Some(toggle(&e));
                    }
                }
                "i" if in_rpr => {
                    if let Some((_, s)) = &mut current {
                        s.italic = Some(toggle(&e));
                    }
                }
                "u" if in_rpr => {
                    if let Some((_, s)) = &mut current {
                        s.underline = Some(toggle(&e));
                    }
                }
                _ => {}
            },
            Ok(Event::End(e)) => match local(e.name().as_ref()) {
                "rPr" => in_rpr = false,
                "style" => {
                    if let Some((id, s)) = current.take() {
                        styles.insert(id, s);
                    }
                }
                _ => {}
            },
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
    }
    styles
}

/// A property of a style, looked up through the styles it is based on.
fn inherited<T>(styles: &HashMap<String, Style>, id: &str, get: impl Fn(&Style) -> Option<T>) -> Option<T> {
    let mut id = id.to_owned();
    for _ in 0..8 {
        let style = styles.get(&id)?;
        if let Some(v) = get(style) {
            return Some(v);
        }
        id = style.based_on.clone()?;
    }
    None
}

#[derive(Default)]
struct RunText {
    text: String,
    bold: bool,
    italic: bool,
}

fn emphasise(runs: &[RunText]) -> String {
    // Join neighbours with the same look, then wrap with markers hugging the words.
    let mut merged: Vec<RunText> = Vec::new();
    for r in runs {
        match merged.last_mut() {
            Some(m) if m.bold == r.bold && m.italic == r.italic => m.text.push_str(&r.text),
            _ => merged.push(RunText { text: r.text.clone(), bold: r.bold, italic: r.italic }),
        }
    }
    let mut out = String::new();
    for m in merged {
        let escaped = m.text.replace('*', "\\*");
        if (!m.bold && !m.italic) || escaped.trim().is_empty() {
            out.push_str(&escaped);
            continue;
        }
        let lead = escaped.len() - escaped.trim_start().len();
        let trail = escaped.len() - escaped.trim_end().len();
        let mark = match (m.bold, m.italic) {
            (true, true) => "***",
            (true, false) => "**",
            _ => "*",
        };
        out.push_str(&escaped[..lead]);
        out.push_str(mark);
        out.push_str(escaped.trim());
        out.push_str(mark);
        out.push_str(&escaped[escaped.len() - trail..]);
    }
    out
}

pub fn read(bytes: &[u8]) -> Result<Raw, String> {
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes)).map_err(|_| "This is not a valid .docx file".to_string())?;
    let document = read_part(&mut archive, "word/document.xml").ok_or("This .docx has no document text")?;
    let styles = read_part(&mut archive, "word/styles.xml").map(|x| parse_styles(&x)).unwrap_or_default();
    let mut raw = Raw::default();

    let mut reader = Reader::from_str(&document);
    let mut runs: Vec<RunText> = Vec::new();
    let mut style_id = String::new();
    let mut outline: Option<u8> = None;
    let mut page_break_before = false;
    let mut pending_break = false;
    let (mut in_run, mut in_text, mut in_rpr, mut in_ppr, mut in_deleted) = (false, false, false, false, false);
    let (mut bold, mut italic, mut underline) = (None::<bool>, None::<bool>, None::<bool>);
    let mut run_style = String::new();
    let mut run = String::new();
    let mut in_paragraph = false;
    let mut marks = String::new();
    loop {
        let event = reader.read_event();
        match event {
            Ok(Event::Start(ref e)) | Ok(Event::Empty(ref e)) => {
                let empty = matches!(event, Ok(Event::Empty(_)));
                match local(e.name().as_ref()) {
                    "p" => {
                        in_paragraph = true;
                        runs.clear();
                        style_id.clear();
                        outline = None;
                        page_break_before = pending_break;
                        pending_break = false;
                        if empty {
                            in_paragraph = false;
                        }
                    }
                    "pPr" => in_ppr = true,
                    "pStyle" if in_ppr => style_id = attr(e, "val").unwrap_or_default(),
                    "outlineLvl" if in_ppr => outline = attr(e, "val").and_then(|v| v.parse().ok()),
                    "pageBreakBefore" if in_ppr && toggle(e) => page_break_before = true,
                    "del" => in_deleted = !empty,
                    "r" if !empty => {
                        in_run = true;
                        (bold, italic, underline) = (None, None, None);
                        run_style.clear();
                        run.clear();
                    }
                    "rPr" if in_run && !empty => in_rpr = true,
                    "rStyle" if in_rpr => run_style = attr(e, "val").unwrap_or_default(),
                    "b" if in_rpr => bold = Some(toggle(e)),
                    "i" if in_rpr => italic = Some(toggle(e)),
                    "u" if in_rpr => underline = Some(toggle(e)),
                    "t" if in_run && !empty => in_text = true,
                    "tab" if in_run => run.push(' '),
                    "br" | "cr" if in_run => {
                        if attr(e, "type").as_deref() == Some("page") {
                            pending_break = true;
                        } else {
                            run.push(' ');
                        }
                    }
                    "commentRangeStart" if in_paragraph => {
                        if let Some(id) = attr(e, "id").and_then(|v| v.parse::<u32>().ok()) {
                            flush_run(&mut run, &mut runs, &styles, &style_id, &run_style, bold, italic, underline);
                            runs.push(RunText { text: char::from_u32(0x100000 + id * 2).map(String::from).unwrap_or_default(), bold: false, italic: false });
                        }
                    }
                    "commentRangeEnd" if in_paragraph => {
                        if let Some(id) = attr(e, "id").and_then(|v| v.parse::<u32>().ok()) {
                            flush_run(&mut run, &mut runs, &styles, &style_id, &run_style, bold, italic, underline);
                            runs.push(RunText { text: char::from_u32(0x100000 + id * 2 + 1).map(String::from).unwrap_or_default(), bold: false, italic: false });
                        }
                    }
                    _ => {}
                }
            }
            Ok(Event::Text(t)) if in_text && !in_deleted => {
                run.push_str(&t.xml10_content());
            }
            Ok(Event::End(e)) => match local(e.name().as_ref()) {
                "t" => in_text = false,
                "rPr" => in_rpr = false,
                "pPr" => in_ppr = false,
                "del" => in_deleted = false,
                "r" => {
                    flush_run(&mut run, &mut runs, &styles, &style_id, &run_style, bold, italic, underline);
                    in_run = false;
                }
                "p" => {
                    in_paragraph = false;
                    let text = emphasise(&runs);
                    let plain: String = runs.iter().map(|r| r.text.as_str()).collect::<String>().chars().filter(|c| (*c as u32) < 0x100000).collect();
                    let level = outline.or_else(|| inherited(&styles, &style_id, |s| s.outline));
                    let name = styles.get(&style_id).map(|s| s.name.to_lowercase()).unwrap_or_else(|| style_id.to_lowercase());
                    let heading = level.filter(|l| *l < 9).map(|l| l + 1).or_else(|| {
                        name.strip_prefix("heading ").or_else(|| name.strip_prefix("heading")).and_then(|n| n.trim().parse::<u8>().ok())
                    });
                    marks.clear();
                    if name == "title" && raw.title.is_none() && !plain.trim().is_empty() {
                        raw.title = Some(plain.trim().to_owned());
                    } else if plain.trim().is_empty() && !text.chars().any(|c| (c as u32) >= 0x100000) {
                        // An empty paragraph: only its page break matters.
                        if page_break_before {
                            pending_break = true;
                        }
                    } else if is_scene_break(plain.trim()) {
                        raw.paras.push(RawPara { text: String::new(), kind: Kind::SceneBreak, page_break_before });
                    } else if let Some(level) = heading {
                        raw.paras.push(RawPara { text: plain.trim().to_owned(), kind: Kind::Heading(level.min(6)), page_break_before });
                    } else {
                        raw.paras.push(RawPara { text: text.trim().to_owned(), kind: Kind::Body, page_break_before });
                    }
                }
                _ => {}
            },
            Ok(Event::Eof) => break,
            Err(e) => return Err(format!("The document text could not be read: {e}")),
            _ => {}
        }
    }

    if let Some(xml) = read_part(&mut archive, "word/comments.xml") {
        raw.comments = parse_comments(&xml);
    }
    Ok(raw)
}

#[allow(clippy::too_many_arguments)]
fn flush_run(run: &mut String, runs: &mut Vec<RunText>, styles: &HashMap<String, Style>, paragraph_style: &str, run_style: &str, bold: Option<bool>, italic: Option<bool>, underline: Option<bool>) {
    if run.is_empty() {
        return;
    }
    let pick = |direct: Option<bool>, get: fn(&Style) -> Option<bool>| {
        direct
            .or_else(|| (!run_style.is_empty()).then(|| inherited(styles, run_style, get)).flatten())
            .or_else(|| inherited(styles, paragraph_style, get))
            .unwrap_or(false)
    };
    runs.push(RunText {
        text: std::mem::take(run),
        bold: pick(bold, |s| s.bold),
        // Typewritten manuscripts underline what is printed in italics.
        italic: pick(italic, |s| s.italic) || pick(underline, |s| s.underline),
    });
}

fn parse_comments(xml: &str) -> Vec<RawComment> {
    let mut out = Vec::new();
    let mut reader = Reader::from_str(xml);
    let mut current: Option<(usize, String, i64)> = None;
    let mut in_text = false;
    let mut paragraphs: Vec<String> = Vec::new();
    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) => match local(e.name().as_ref()) {
                "comment" => {
                    let id = attr(&e, "id").and_then(|v| v.parse().ok()).unwrap_or(0);
                    let created = attr(&e, "date").and_then(|d| d.parse::<jiff::Timestamp>().ok()).map_or(0, |t| t.as_second());
                    current = Some((id, String::new(), created));
                    paragraphs.clear();
                }
                "p" => paragraphs.push(String::new()),
                "t" => in_text = true,
                _ => {}
            },
            Ok(Event::Text(t)) if in_text => {
                if let Some(p) = paragraphs.last_mut() {
                    p.push_str(&t.xml10_content());
                }
            }
            Ok(Event::End(e)) => match local(e.name().as_ref()) {
                "t" => in_text = false,
                "comment" => {
                    if let Some((id, _, created)) = current.take() {
                        let body = paragraphs.iter().map(|p| p.trim()).filter(|p| !p.is_empty()).collect::<Vec<_>>().join("\n");
                        if !body.is_empty() {
                            out.push(RawComment { id, body, created });
                        }
                    }
                }
                _ => {}
            },
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::super::split::{By, Item, Options, split, suggest};
    use super::*;
    use crate::export::{
        docx as writer,
        model::{Scope, build, testing::source},
        preset::{self, CommentsMode, builtins},
    };

    /// A document written by this program's own exporter is read back faithfully.
    #[test]
    fn a_manuscript_exported_as_docx_imports_back_with_chapters_emphasis_and_comments() {
        let mut src = source();
        let id = src.tree.leaves(crate::jw::manifest::Section::Manuscript)[0].0.id().to_owned();
        let text = src.texts[&id].clone();
        let start = text.find("whisper").unwrap();
        src.comments.add(&id, &text, start..start + 7, "Too soft?", "prose", 1_760_000_000).unwrap();
        let mut p = builtins().into_iter().find(|p| p.id == preset::MANUSCRIPT_DOCX).unwrap();
        p.comments = CommentsMode::Docx;
        let bytes = writer::write(&build(&src, &Scope::Whole, &p), &p).unwrap();

        let raw = read(&bytes).unwrap();
        assert!(raw.paras.iter().any(|p| matches!(p.kind, Kind::Heading(1)) && p.text.contains("Prologue")), "{:#?}", raw.paras.iter().take(12).collect::<Vec<_>>());
        let options = suggest(&raw);
        assert_eq!(options.by, By::Headings(1));
        let imported = split(&raw, &options).unwrap();
        // The title page comes first, as a chapter of its own to delete or keep.
        assert_eq!(imported.chapters().next().unwrap().title, "Front matter");
        let first = imported.chapters().nth(1).unwrap();
        assert!(first.text.contains("It began with a *whisper*."), "{}", first.text);
        assert!(first.text.contains("Then **silence** fell."));
        assert!(first.text.lines().any(|l| l == "#"), "the scene break");
        assert!(!first.text.contains("cut this"));
        assert_eq!(first.comments.len(), 1);
        let c = &first.comments[0];
        assert_eq!(c.body, "[prose] Too soft?");
        let anchored: String = first.text.chars().skip(c.range.start).take(c.range.len()).collect();
        assert_eq!(anchored, "whisper");
        assert_eq!(c.created, 1_760_000_000);
        assert!(imported.items.iter().any(|i| matches!(i, Item::Chapter(c) if c.text.contains("Run,"))));
        assert_eq!(imported.chapters().count(), 4);
        let _ = Options::default();
    }

    fn docx_with(body: &str, styles: &str) -> Vec<u8> {
        use std::io::Write;
        let mut out = Vec::new();
        {
            let mut zip = zip::ZipWriter::new(std::io::Cursor::new(&mut out));
            let o = zip::write::SimpleFileOptions::default();
            zip.start_file("word/document.xml", o).unwrap();
            write!(zip, "<w:document xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\"><w:body>{body}</w:body></w:document>").unwrap();
            zip.start_file("word/styles.xml", o).unwrap();
            write!(zip, "<w:styles xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\">{styles}</w:styles>").unwrap();
            zip.finish().unwrap();
        }
        out
    }

    #[test]
    fn headings_come_from_outline_levels_so_localised_styles_work() {
        let styles = r#"<w:style w:styleId="Ttulo1"><w:name w:val="Título 1"/><w:pPr><w:outlineLvl w:val="0"/></w:pPr></w:style>
<w:style w:styleId="Enfasis"><w:name w:val="Énfasis"/><w:rPr><w:i/></w:rPr></w:style>
<w:style w:styleId="Cita"><w:name w:val="Cita"/><w:rPr><w:i/></w:rPr></w:style>"#;
        let body = r#"<w:p><w:pPr><w:pStyle w:val="Ttulo1"/></w:pPr><w:r><w:t>Capítulo uno</w:t></w:r></w:p>
<w:p><w:r><w:t xml:space="preserve">Dijo </w:t></w:r><w:r><w:rPr><w:rStyle w:val="Enfasis"/></w:rPr><w:t>hola</w:t></w:r><w:r><w:t>.</w:t></w:r></w:p>
<w:p><w:r><w:rPr><w:u w:val="single"/></w:rPr><w:t>underlined</w:t></w:r><w:r><w:t xml:space="preserve"> star * here </w:t></w:r></w:p>
<w:p><w:pPr><w:jc w:val="center"/></w:pPr><w:r><w:t>* * *</w:t></w:r></w:p>
<w:p><w:pPr><w:pStyle w:val="Cita"/></w:pPr><w:r><w:t>quoted</w:t></w:r></w:p>
<w:p><w:del><w:r><w:delText>gone</w:delText></w:r></w:del><w:r><w:t>kept</w:t></w:r></w:p>
<w:p><w:r><w:br w:type="page"/></w:r></w:p>
<w:p><w:pPr><w:pStyle w:val="Ttulo1"/></w:pPr><w:r><w:t>Capítulo dos</w:t></w:r></w:p>"#;
        let raw = read(&docx_with(body, styles)).unwrap();
        let t: Vec<String> = raw.paras.iter().map(|p| match &p.kind { Kind::Heading(n) => format!("H{n} {}", p.text), Kind::SceneBreak => "#".into(), Kind::Body => p.text.clone() }).collect();
        assert_eq!(t, ["H1 Capítulo uno", "Dijo *hola*.", "*underlined* star \\* here", "#", "*quoted*", "kept", "H1 Capítulo dos"]);
        assert!(raw.paras.last().unwrap().page_break_before);
    }

    #[test]
    fn broken_files_give_an_error_not_a_panic() {
        assert!(read(b"not a zip").is_err());
        assert!(read(&docx_with("<w:p>", "")).is_err() || read(&docx_with("<w:p>", "")).is_ok());
    }
}
