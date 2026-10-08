//! DOCX, written directly as WordprocessingML so that every layout value of the preset maps to
//! what Word shows: fonts, spacing, indents, margins, running heads with page fields, chapter
//! openings a third of the way down the page, and review comments as Word comments.

use super::{
    model::{Block, Chapter, Doc, Inline, Run, heading_lines, part_heading},
    preset::{Align, Italics, PageNumbers, Preset, RunningHead},
};
use crate::{core::typography::Language, jw::time};
use std::io::{self, Write};
use zip::{CompressionMethod, ZipWriter, write::SimpleFileOptions};

const W: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
const R: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";

pub fn esc(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            c if (c as u32) < 0x20 && !matches!(c, '\t' | '\n' | '\r') => {}
            '\u{FFFE}' | '\u{FFFF}' => {}
            c => out.push(c),
        }
    }
    out
}
fn twips(inches: f32) -> i64 {
    (inches * 1440.0).round() as i64
}
fn points_to_twips(points: f32) -> i64 {
    (points * 20.0).round() as i64
}

/// The parts of the preset that decide which header and footer parts exist.
struct Furniture {
    default_header: Option<String>,
    even_header: Option<String>,
    default_footer: Option<String>,
    even_footer: Option<String>,
    odd_even: bool,
}

fn field(instruction: &str, cached: &str, props: &str) -> String {
    format!(
        "<w:r>{props}<w:fldChar w:fldCharType=\"begin\"/></w:r><w:r>{props}<w:instrText xml:space=\"preserve\"> {instruction} </w:instrText></w:r><w:r>{props}<w:fldChar w:fldCharType=\"separate\"/></w:r><w:r>{props}<w:t>{cached}</w:t></w:r><w:r>{props}<w:fldChar w:fldCharType=\"end\"/></w:r>"
    )
}
fn text_run(text: &str, props: &str) -> String {
    format!("<w:r>{props}<w:t xml:space=\"preserve\">{}</w:t></w:r>", esc(text))
}
fn part_xml(root: &str, namespaces: &str, body: &str) -> String {
    format!("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<w:{root} xmlns:w=\"{W}\" xmlns:r=\"{R}\"{namespaces}>{body}</w:{root}>")
}

fn furniture(doc: &Doc, preset: &Preset) -> Furniture {
    let align = |a: &str| format!("<w:pPr><w:pStyle w:val=\"Header\"/><w:jc w:val=\"{a}\"/></w:pPr>");
    let page = field("PAGE", "1", "");
    let head_text = |left: bool| -> Option<String> {
        match preset.running_head {
            RunningHead::None => None,
            RunningHead::SurnameTitlePage => {
                let mut label = String::new();
                if !doc.surname.is_empty() {
                    label.push_str(&format!("{} / ", doc.surname));
                }
                label.push_str(&format!("{} / ", doc.running_title));
                Some(format!("<w:p>{}{}{page}</w:p>", align("right"), text_run(&label, "")))
            }
            RunningHead::Alternating => {
                let (text, jc) = if left { (doc.author.clone(), "left") } else { (doc.running_title.clone(), "right") };
                Some(format!("<w:p>{}{}</w:p>", align(jc), text_run(&text, "")))
            }
            RunningHead::ChapterTitle => Some(format!(
                "<w:p>{}{}</w:p>",
                align("center"),
                field("STYLEREF \"Heading 1\" \\* MERGEFORMAT", &esc(&doc.title), "")
            )),
        }
    };
    let alternating = preset.running_head == RunningHead::Alternating || preset.page_numbers == PageNumbers::FooterOutside;
    let number_in_head = matches!(preset.running_head, RunningHead::SurnameTitlePage);
    let mut header_default = head_text(false);
    let header_even = head_text(true);
    if preset.page_numbers == PageNumbers::HeaderRight && !number_in_head {
        let numbering = format!("<w:p>{}{page}</w:p>", align("right"));
        header_default = match header_default {
            Some(h) if preset.running_head != RunningHead::Alternating => Some(h + &numbering),
            Some(h) => Some(h),
            None => Some(numbering),
        };
    }
    let footer = |outside_left: bool| -> Option<String> {
        let footer_p = |jc: &str| format!("<w:p><w:pPr><w:pStyle w:val=\"Footer\"/><w:jc w:val=\"{jc}\"/></w:pPr>{page}</w:p>");
        match preset.page_numbers {
            PageNumbers::FooterCenter => Some(footer_p("center")),
            PageNumbers::FooterOutside => Some(footer_p(if outside_left { "left" } else { "right" })),
            _ => None,
        }
    };
    Furniture {
        default_header: header_default,
        even_header: if alternating { header_even.or(Some("<w:p/>".into())) } else { None },
        default_footer: footer(false),
        even_footer: if alternating { footer(true) } else { None },
        odd_even: alternating,
    }
}

fn style_xml(preset: &Preset, doc: &Doc) -> String {
    let font = esc(&preset.font);
    let size = (preset.font_size * 2.0).round() as i64;
    let line = (preset.line_spacing * 240.0).round() as i64;
    let after = points_to_twips(preset.paragraph_space);
    let jc = if preset.justify { "both" } else { "left" };
    let indent = twips(preset.indent);
    let head_size = (preset.chapter_size * 2.0).round() as i64;
    let heading_jc = if preset.chapter_align == Align::Center { "center" } else { "left" };
    let heading_caps = match preset.chapter_caps {
        super::preset::Caps::None => "",
        super::preset::Caps::Upper => "<w:caps/>",
        super::preset::Caps::SmallCaps => "<w:smallCaps/>",
    };
    let bold = if preset.chapter_bold { "<w:b/>" } else { "" };
    let lang = doc.language.tag();
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<w:styles xmlns:w=\"{W}\">\
<w:docDefaults><w:rPrDefault><w:rPr><w:rFonts w:ascii=\"{font}\" w:hAnsi=\"{font}\" w:cs=\"{font}\" w:eastAsia=\"{font}\"/><w:sz w:val=\"{size}\"/><w:szCs w:val=\"{size}\"/><w:lang w:val=\"{lang}\"/></w:rPr></w:rPrDefault>\
<w:pPrDefault><w:pPr><w:spacing w:after=\"0\" w:line=\"{line}\" w:lineRule=\"auto\"/></w:pPr></w:pPrDefault></w:docDefaults>\
<w:style w:type=\"paragraph\" w:default=\"1\" w:styleId=\"Normal\"><w:name w:val=\"Normal\"/><w:qFormat/><w:pPr><w:jc w:val=\"left\"/></w:pPr></w:style>\
<w:style w:type=\"paragraph\" w:styleId=\"Body\"><w:name w:val=\"Body Text\"/><w:basedOn w:val=\"Normal\"/><w:qFormat/><w:pPr><w:spacing w:after=\"{after}\"/><w:ind w:firstLine=\"{indent}\"/><w:jc w:val=\"{jc}\"/></w:pPr></w:style>\
<w:style w:type=\"paragraph\" w:styleId=\"BodyFlush\"><w:name w:val=\"Body Text First\"/><w:basedOn w:val=\"Body\"/><w:qFormat/><w:pPr><w:ind w:firstLine=\"0\"/></w:pPr></w:style>\
<w:style w:type=\"paragraph\" w:styleId=\"SceneBreak\"><w:name w:val=\"Scene Break\"/><w:basedOn w:val=\"Normal\"/><w:qFormat/><w:pPr><w:keepNext/><w:spacing w:after=\"{after}\"/><w:jc w:val=\"center\"/></w:pPr></w:style>\
<w:style w:type=\"paragraph\" w:styleId=\"Heading1\"><w:name w:val=\"heading 1\"/><w:basedOn w:val=\"Normal\"/><w:next w:val=\"Body\"/><w:qFormat/><w:pPr><w:keepNext/><w:spacing w:after=\"{head_after}\"/><w:jc w:val=\"{heading_jc}\"/><w:outlineLvl w:val=\"0\"/></w:pPr><w:rPr>{bold}{heading_caps}<w:sz w:val=\"{head_size}\"/><w:szCs w:val=\"{head_size}\"/></w:rPr></w:style>\
<w:style w:type=\"paragraph\" w:styleId=\"PartHeading\"><w:name w:val=\"Part Heading\"/><w:basedOn w:val=\"Heading1\"/><w:next w:val=\"Heading1\"/><w:qFormat/><w:pPr><w:pageBreakBefore/><w:outlineLvl w:val=\"9\"/></w:pPr></w:style>\
<w:style w:type=\"paragraph\" w:styleId=\"Title\"><w:name w:val=\"Title\"/><w:basedOn w:val=\"Normal\"/><w:qFormat/><w:pPr><w:jc w:val=\"center\"/></w:pPr><w:rPr><w:sz w:val=\"{title_size}\"/><w:szCs w:val=\"{title_size}\"/></w:rPr></w:style>\
<w:style w:type=\"paragraph\" w:styleId=\"TitleInfo\"><w:name w:val=\"Title Info\"/><w:basedOn w:val=\"Normal\"/><w:pPr><w:spacing w:line=\"240\" w:lineRule=\"auto\"/></w:pPr></w:style>\
<w:style w:type=\"paragraph\" w:styleId=\"Matter\"><w:name w:val=\"Matter\"/><w:basedOn w:val=\"Normal\"/><w:pPr><w:spacing w:after=\"{after_matter}\"/><w:jc w:val=\"center\"/></w:pPr></w:style>\
<w:style w:type=\"paragraph\" w:styleId=\"TOC1\"><w:name w:val=\"toc 1\"/><w:basedOn w:val=\"Normal\"/><w:next w:val=\"Normal\"/><w:uiPriority w:val=\"39\"/><w:pPr><w:spacing w:line=\"360\" w:lineRule=\"auto\"/></w:pPr></w:style>\
<w:style w:type=\"paragraph\" w:styleId=\"Header\"><w:name w:val=\"header\"/><w:basedOn w:val=\"Normal\"/><w:pPr><w:spacing w:line=\"240\" w:lineRule=\"auto\"/></w:pPr></w:style>\
<w:style w:type=\"paragraph\" w:styleId=\"Footer\"><w:name w:val=\"footer\"/><w:basedOn w:val=\"Normal\"/><w:pPr><w:spacing w:line=\"240\" w:lineRule=\"auto\"/></w:pPr></w:style>\
<w:style w:type=\"paragraph\" w:styleId=\"CommentText\"><w:name w:val=\"annotation text\"/><w:basedOn w:val=\"Normal\"/><w:pPr><w:spacing w:line=\"240\" w:lineRule=\"auto\"/></w:pPr><w:rPr><w:sz w:val=\"20\"/></w:rPr></w:style>\
<w:style w:type=\"character\" w:styleId=\"CommentReference\"><w:name w:val=\"annotation reference\"/><w:rPr><w:sz w:val=\"16\"/></w:rPr></w:style>\
</w:styles>",
        head_after = line_twips(preset) * 2,
        title_size = (preset.font_size * 2.0 * if preset.title_page_contact { 1.0 } else { 1.8 }).round() as i64,
        after_matter = line_twips(preset),
    )
}
/// One line of body text in twips, used to space headings.
fn line_twips(preset: &Preset) -> i64 {
    points_to_twips(preset.font_size * 1.2 * preset.line_spacing)
}

/// Header and footer parts of the document and the references that attach them to a section.
struct Parts {
    /// (file name, content type, xml)
    files: Vec<(String, &'static str, String)>,
    /// Relationship elements for `document.xml.rels`.
    rels: String,
    /// `headerReference`/`footerReference` elements for a section's properties.
    refs: String,
}
fn build_parts(furniture: &Furniture, preset: &Preset) -> Parts {
    let mut parts = Parts { files: Vec::new(), rels: String::new(), refs: String::new() };
    let mut add = |name: &str, kind: &str, rid: &str, xml: String, ref_type: &str| {
        parts.rels.push_str(&format!("<Relationship Id=\"{rid}\" Type=\"{R}/{kind}\" Target=\"{name}.xml\"/>"));
        let tag = if kind == "header" { "headerReference" } else { "footerReference" };
        parts.refs.push_str(&format!("<w:{tag} w:type=\"{ref_type}\" r:id=\"{rid}\"/>"));
        let root = if kind == "header" { "hdr" } else { "ftr" };
        parts.files.push((format!("word/{name}.xml"), content_type_of(kind), part_xml(root, "", &xml)));
    };
    if let Some(h) = &furniture.default_header {
        add("header1", "header", "rIdH1", h.clone(), "default");
        if furniture.odd_even {
            add("header2", "header", "rIdH2", furniture.even_header.clone().unwrap_or("<w:p/>".into()), "even");
        }
        // Pages that open a chapter carry no running head unless the preset asks for it.
        add("header3", "header", "rIdH3", "<w:p/>".into(), "first");
    }
    if let Some(f) = &furniture.default_footer {
        add("footer1", "footer", "rIdF1", f.clone(), "default");
        if furniture.odd_even {
            add("footer2", "footer", "rIdF2", furniture.even_footer.clone().unwrap_or("<w:p/>".into()), "even");
        }
        add("footer3", "footer", "rIdF3", f.clone(), "first");
    }
    let _ = preset;
    parts
}

struct Writer<'a> {
    preset: &'a Preset,
    doc: &'a Doc,
    body: String,
    parts: &'a Parts,
    /// Whether the next body paragraph follows a heading or scene break.
    after_break: bool,
    text_width: i64,
    text_height: i64,
}

impl Writer<'_> {
    fn run_props(&self, run: &Run) -> String {
        let mut props = String::new();
        if run.bold {
            props.push_str("<w:b/>");
        }
        if run.italic {
            match self.preset.italics {
                Italics::Italic => props.push_str("<w:i/>"),
                Italics::Underline => props.push_str("<w:u w:val=\"single\"/>"),
            }
        }
        if run.note {
            props.push_str("<w:i/><w:color w:val=\"7F7F7F\"/><w:shd w:val=\"clear\" w:color=\"auto\" w:fill=\"EEEEEE\"/>");
        }
        if props.is_empty() { String::new() } else { format!("<w:rPr>{props}</w:rPr>") }
    }
    fn inlines(&self, inlines: &[Inline]) -> String {
        let mut out = String::new();
        for inline in inlines {
            match inline {
                Inline::Run(run) => out.push_str(&text_run(&run.text, &self.run_props(run))),
                Inline::CommentStart(n) => out.push_str(&format!("<w:commentRangeStart w:id=\"{n}\"/>")),
                Inline::CommentEnd(n) => out.push_str(&format!(
                    "<w:commentRangeEnd w:id=\"{n}\"/><w:r><w:rPr><w:rStyle w:val=\"CommentReference\"/></w:rPr><w:commentReference w:id=\"{n}\"/></w:r>"
                )),
            }
        }
        out
    }
    fn paragraph(&mut self, style: &str, ppr_extra: &str, content: &str) {
        self.body.push_str(&format!("<w:p><w:pPr><w:pStyle w:val=\"{style}\"/>{ppr_extra}</w:pPr>{content}</w:p>"));
    }
    fn sect_pr(&self, with_refs: bool, first_page_special: bool, restart_numbers: bool) -> String {
        let p = self.preset;
        let (w, h) = (twips(p.page_width), twips(p.page_height));
        let (top, bottom) = (twips(p.margin_top), twips(p.margin_bottom));
        let (left, right) = (twips(p.margin_inside), twips(p.margin_outside));
        let distance = (top.min(bottom) / 2).max(360);
        format!(
            "<w:sectPr>{refs}<w:type w:val=\"nextPage\"/><w:pgSz w:w=\"{w}\" w:h=\"{h}\"/><w:pgMar w:top=\"{top}\" w:right=\"{right}\" w:bottom=\"{bottom}\" w:left=\"{left}\" w:header=\"{distance}\" w:footer=\"{distance}\" w:gutter=\"{g}\"/>{numbering}{title_pg}</w:sectPr>",
            refs = if with_refs { self.parts.refs.as_str() } else { "" },
            g = twips(p.gutter),
            numbering = if restart_numbers { "<w:pgNumType w:start=\"1\"/>" } else { "" },
            title_pg = if first_page_special { "<w:titlePg/>" } else { "" },
        )
    }
    /// Ends a section: its properties ride on an empty paragraph.
    fn end_section(&mut self, with_refs: bool, first_page_special: bool, restart_numbers: bool) {
        let props = self.sect_pr(with_refs, first_page_special, restart_numbers);
        self.body.push_str(&format!("<w:p><w:pPr><w:spacing w:line=\"240\" w:lineRule=\"auto\"/>{props}</w:pPr></w:p>"));
    }
}

/// Creates the DOCX.
pub fn write(doc: &Doc, preset: &Preset) -> io::Result<Vec<u8>> {
    let furniture = furniture(doc, preset);
    let parts = build_parts(&furniture, preset);
    let mut w = Writer {
        preset,
        doc,
        body: String::new(),
        parts: &parts,
        after_break: true,
        text_width: twips(preset.page_width - preset.margin_inside - preset.margin_outside - preset.gutter),
        text_height: twips(preset.page_height - preset.margin_top - preset.margin_bottom),
    };
    let spanish = doc.language == Language::Spanish;
    let matter = |w: &mut Writer, title: &str, paras: &[Vec<Inline>]| {
        if paras.is_empty() {
            return;
        }
        w.paragraph("Heading1", "<w:pageBreakBefore/>", &text_run(title, ""));
        for p in paras {
            let content = w.inlines(p);
            w.paragraph("Matter", "", &content);
        }
    };

    // Front matter: its own section, with no running head or numbers.
    if preset.title_page {
        w.title_page(doc, preset);
    }
    matter(&mut w, if spanish { "Dedicatoria" } else { "Dedication" }, &doc.dedication);
    matter(&mut w, if spanish { "Epígrafe" } else { "Epigraph" }, &doc.epigraph);
    if !doc.copyright.is_empty() {
        w.paragraph("Heading1", "<w:pageBreakBefore/>", &text_run(if spanish { "Derechos de autor" } else { "Copyright" }, ""));
        for line in &doc.copyright {
            w.paragraph("Matter", "", &text_run(line, ""));
        }
    }
    if preset.toc && !doc.chapters.is_empty() {
        w.table_of_contents(doc, preset);
    }
    let has_front = !w.body.is_empty();
    if has_front {
        w.end_section(false, false, false);
    }
    // Chapters. When chapter openings carry no running head each chapter is its own section so
    // that its first page can use the blank header.
    let per_chapter = !preset.head_on_chapter_start && furniture.default_header.is_some();
    for (i, chapter) in doc.chapters.iter().enumerate() {
        if per_chapter && i > 0 {
            w.end_section(true, true, false);
        }
        w.chapter(chapter, preset, i == 0 && has_front || per_chapter && i > 0);
    }
    matter(&mut w, if spanish { "Agradecimientos" } else { "Acknowledgments" }, &doc.acknowledgments);
    matter(&mut w, if spanish { "Sobre el autor" } else { "About the author" }, &doc.about_author);
    if !doc.appendix.is_empty() {
        w.appendix(doc);
    }
    // The last section; numbering restarts here when it is the first of the chapters.
    let last = w.sect_pr(true, per_chapter, !per_chapter || doc.chapters.len() <= 1);
    let last = if !has_front && per_chapter && doc.chapters.len() > 1 { last.replace("<w:titlePg/>", "<w:titlePg/>") } else { last };
    let document = part_xml("document", "", &format!("<w:body>{}{last}</w:body>", w.body));

    let mut comments_xml = String::new();
    if !doc.comments.is_empty() {
        let author = if doc.author.is_empty() { "JustWrite".to_owned() } else { doc.author.clone() };
        let initials: String = author.split_whitespace().filter_map(|s| s.chars().next()).take(3).collect();
        for (i, c) in doc.comments.iter().enumerate() {
            let tag = if c.tag.is_empty() { String::new() } else { format!("[{}] ", c.tag) };
            let mut paragraphs = format!(
                "<w:p><w:pPr><w:pStyle w:val=\"CommentText\"/></w:pPr><w:r><w:rPr><w:rStyle w:val=\"CommentReference\"/></w:rPr><w:annotationRef/></w:r>{}</w:p>",
                text_run(&format!("{tag}{}", c.body), "")
            );
            for (when, reply) in &c.replies {
                paragraphs.push_str(&format!(
                    "<w:p><w:pPr><w:pStyle w:val=\"CommentText\"/></w:pPr>{}</w:p>",
                    text_run(&format!("Reply ({}): {reply}", time::date(*when)), "")
                ));
            }
            comments_xml.push_str(&format!(
                "<w:comment w:id=\"{i}\" w:author=\"{}\" w:date=\"{}\" w:initials=\"{}\">{paragraphs}</w:comment>",
                esc(&author),
                time::iso(c.created),
                esc(&initials)
            ));
        }
    }

    // The order of children inside w:settings is fixed by the schema.
    let mut settings = String::new();
    if preset.mirrored {
        settings.push_str("<w:mirrorMargins/>");
    }
    if furniture.odd_even {
        settings.push_str("<w:evenAndOddHeaders/>");
    }
    settings.push_str("<w:defaultTabStop w:val=\"720\"/>");
    if preset.hyphenate {
        settings.push_str("<w:autoHyphenation/>");
    }
    settings.push_str(&format!("<w:themeFontLang w:val=\"{}\"/>", doc.language.tag()));
    let settings_xml = part_xml("settings", "", &settings);

    let mut rels_doc = format!(
        "<Relationship Id=\"rIdS\" Type=\"{R}/styles\" Target=\"styles.xml\"/><Relationship Id=\"rIdSet\" Type=\"{R}/settings\" Target=\"settings.xml\"/>{}",
        parts.rels
    );
    if !doc.comments.is_empty() {
        rels_doc.push_str(&format!("<Relationship Id=\"rIdC\" Type=\"{R}/comments\" Target=\"comments.xml\"/>"));
    }

    let mut out = Vec::new();
    {
        let mut zip = ZipWriter::new(io::Cursor::new(&mut out));
        let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
        let mut put = |name: &str, content: &str| -> io::Result<()> {
            zip.start_file(name, options).map_err(io::Error::other)?;
            zip.write_all(content.as_bytes())
        };
        let mut overrides = String::new();
        for (name, content_type, _) in &parts.files {
            overrides.push_str(&format!("<Override PartName=\"/{name}\" ContentType=\"{content_type}\"/>"));
        }
        if !doc.comments.is_empty() {
            overrides.push_str("<Override PartName=\"/word/comments.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.comments+xml\"/>");
        }
        put(
            "[Content_Types].xml",
            &format!(
                "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\"><Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/><Default Extension=\"xml\" ContentType=\"application/xml\"/><Override PartName=\"/word/document.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml\"/><Override PartName=\"/word/styles.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml\"/><Override PartName=\"/word/settings.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.settings+xml\"/><Override PartName=\"/docProps/core.xml\" ContentType=\"application/vnd.openxmlformats-package.core-properties+xml\"/><Override PartName=\"/docProps/app.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.extended-properties+xml\"/>{overrides}</Types>"
            ),
        )?;
        put(
            "_rels/.rels",
            &format!(
                "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"><Relationship Id=\"rId1\" Type=\"{R}/officeDocument\" Target=\"word/document.xml\"/><Relationship Id=\"rId2\" Type=\"http://schemas.openxmlformats.org/package/2006/relationships/metadata/core-properties\" Target=\"docProps/core.xml\"/><Relationship Id=\"rId3\" Type=\"{R}/extended-properties\" Target=\"docProps/app.xml\"/></Relationships>"
            ),
        )?;
        put(
            "docProps/core.xml",
            &format!(
                "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<cp:coreProperties xmlns:cp=\"http://schemas.openxmlformats.org/package/2006/metadata/core-properties\" xmlns:dc=\"http://purl.org/dc/elements/1.1/\" xmlns:dcterms=\"http://purl.org/dc/terms/\" xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\"><dc:title>{}</dc:title><dc:creator>{}</dc:creator><dc:language>{}</dc:language><dcterms:created xsi:type=\"dcterms:W3CDTF\">{d}</dcterms:created><dcterms:modified xsi:type=\"dcterms:W3CDTF\">{d}</dcterms:modified></cp:coreProperties>",
                esc(&doc.title),
                esc(&doc.author),
                doc.language.tag(),
                d = time::iso(doc.date)
            ),
        )?;
        put(
            "docProps/app.xml",
            &format!(
                "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<Properties xmlns=\"http://schemas.openxmlformats.org/officeDocument/2006/extended-properties\"><Application>JustWrite</Application><Words>{}</Words></Properties>",
                doc.words
            ),
        )?;
        put("word/document.xml", &document)?;
        put("word/styles.xml", &style_xml(preset, doc))?;
        put("word/settings.xml", &settings_xml)?;
        put(
            "word/_rels/document.xml.rels",
            &format!("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">{rels_doc}</Relationships>"),
        )?;
        for (name, _, xml) in &parts.files {
            put(name, xml)?;
        }
        if !doc.comments.is_empty() {
            put("word/comments.xml", &part_xml("comments", "", &comments_xml))?;
        }
        zip.finish().map_err(io::Error::other)?;
    }
    Ok(out)
}
fn content_type_of(kind: &str) -> &'static str {
    match kind {
        "header" => "application/vnd.openxmlformats-officedocument.wordprocessingml.header+xml",
        _ => "application/vnd.openxmlformats-officedocument.wordprocessingml.footer+xml",
    }
}

impl Writer<'_> {
    fn title_page(&mut self, doc: &Doc, preset: &Preset) {
        let spanish = doc.language == Language::Spanish;
        if preset.title_page_contact && !doc.contact.is_empty() {
            let count = if preset.rounded_word_count { super::preset::rounded_words(doc.words, doc.language) } else { String::new() };
            for (i, line) in doc.contact.iter().enumerate() {
                let mut content = text_run(line, "");
                let mut ppr = String::new();
                if i == 0 && !count.is_empty() {
                    ppr = format!("<w:tabs><w:tab w:val=\"right\" w:pos=\"{}\"/></w:tabs>", self.text_width);
                    content.push_str("<w:r><w:tab/></w:r>");
                    content.push_str(&text_run(&count, ""));
                }
                self.paragraph("TitleInfo", &ppr, &content);
            }
        } else if preset.rounded_word_count {
            self.paragraph("TitleInfo", "<w:jc w:val=\"right\"/>", &text_run(&super::preset::rounded_words(doc.words, doc.language), ""));
        }
        let drop = (self.text_height as f32 * if preset.title_page_contact { 0.28 } else { 0.30 }) as i64;
        self.paragraph("Title", &format!("<w:spacing w:before=\"{drop}\" w:after=\"0\"/>"), &text_run(&doc.title.to_uppercase(), &if preset.title_page_contact { "<w:rPr><w:b/></w:rPr>".to_owned() } else { "<w:rPr><w:b/></w:rPr>".to_owned() }));
        if !doc.subtitle.is_empty() {
            self.paragraph("Title", "", &text_run(&doc.subtitle, ""));
        }
        if !doc.author.is_empty() {
            self.paragraph("Title", &format!("<w:spacing w:before=\"{}\"/>", line_twips(preset)), &text_run(&format!("{} {}", if spanish { "por" } else { "by" }, doc.author), ""));
        }
        if !preset.title_page_contact && !doc.series.is_empty() {
            self.paragraph("Title", &format!("<w:spacing w:before=\"{}\"/>", line_twips(preset)), &text_run(&doc.series, ""));
        }
    }
    fn table_of_contents(&mut self, doc: &Doc, preset: &Preset) {
        let spanish = doc.language == Language::Spanish;
        self.paragraph("Heading1", "<w:pageBreakBefore/>", &text_run(if spanish { "Contenido" } else { "Contents" }, ""));
        let begin = "<w:r><w:fldChar w:fldCharType=\"begin\"/></w:r><w:r><w:instrText xml:space=\"preserve\"> TOC \\o \"1-1\" \\h \\z \\u </w:instrText></w:r><w:r><w:fldChar w:fldCharType=\"separate\"/></w:r>";
        let end = "<w:r><w:fldChar w:fldCharType=\"end\"/></w:r>";
        for (i, chapter) in doc.chapters.iter().enumerate() {
            let line = super::model::heading_line(chapter, preset, doc.language);
            let line = if line.is_empty() { chapter.title.clone() } else { line };
            let mut content = String::new();
            if i == 0 {
                content.push_str(begin);
            }
            content.push_str(&text_run(&line, ""));
            if i + 1 == doc.chapters.len() {
                content.push_str(end);
            }
            self.paragraph("TOC1", "", &content);
        }
    }
    fn chapter(&mut self, chapter: &Chapter, preset: &Preset, first_in_section: bool) {
        let doc = self.doc;
        let drop = (self.text_height as f32 * preset.chapter_drop) as i64;
        if let (Some(part), true) = (&chapter.part, preset.part_headings) {
            self.paragraph(
                "PartHeading",
                &format!("<w:spacing w:before=\"{drop}\"/>"),
                &text_run(&part_heading(part, preset, doc.language), ""),
            );
        }
        let (number, title) = heading_lines(chapter, preset, doc.language);
        let after_part = chapter.part.is_some() && preset.part_headings;
        if number.is_some() || title.is_some() {
            let mut content = String::new();
            if let Some(n) = &number {
                content.push_str(&text_run(n, ""));
            }
            if let Some(t) = &title {
                if number.is_some() {
                    content.push_str("<w:r><w:br/></w:r>");
                }
                content.push_str(&text_run(t, ""));
            }
            let mut ppr = String::new();
            if preset.chapter_new_page && !first_in_section && !after_part {
                ppr.push_str("<w:pageBreakBefore/>");
            }
            if !after_part {
                ppr.push_str(&format!("<w:spacing w:before=\"{drop}\"/>"));
            }
            self.paragraph("Heading1", &ppr, &content);
            if !preset.chapter_ornament.is_empty() {
                self.paragraph("Matter", "", &text_run(&preset.chapter_ornament, ""));
            }
        }
        self.after_break = true;
        for block in &chapter.blocks {
            match block {
                Block::SceneBreak => {
                    self.paragraph("SceneBreak", "", &text_run(&preset.scene_break, ""));
                    self.after_break = true;
                }
                Block::Paragraph(inlines) => {
                    let style = if self.after_break && preset.flush_after_break { "BodyFlush" } else { "Body" };
                    let content = self.inlines(inlines);
                    self.paragraph(style, "", &content);
                    self.after_break = false;
                }
            }
        }
    }
    fn appendix(&mut self, doc: &Doc) {
        let spanish = doc.language == Language::Spanish;
        self.paragraph("Heading1", "<w:pageBreakBefore/>", &text_run(if spanish { "Comentarios" } else { "Comments" }, ""));
        let mut last = String::new();
        for c in &doc.appendix {
            if c.chapter != last {
                self.paragraph("TitleInfo", "<w:spacing w:before=\"240\"/>", &text_run(&c.chapter, "<w:rPr><w:b/></w:rPr>"));
                last = c.chapter.clone();
            }
            let quote: String = c.quote.chars().take(160).collect();
            self.paragraph("TitleInfo", "<w:ind w:left=\"360\"/>", &text_run(&format!("“{quote}”"), "<w:rPr><w:i/></w:rPr>"));
            let tag = if c.tag.is_empty() { String::new() } else { format!("[{}] ", c.tag) };
            self.paragraph("TitleInfo", "<w:ind w:left=\"360\"/>", &text_run(&format!("{tag}{}", c.body), ""));
            for (when, reply) in &c.replies {
                self.paragraph("TitleInfo", "<w:ind w:left=\"720\"/>", &text_run(&format!("Reply ({}): {reply}", time::date(*when)), ""));
            }
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::super::{
        model::{Scope, build, testing::source},
        preset::{self, CommentsMode, builtins},
    };
    use super::*;
    use std::{collections::BTreeMap, io::Read};

    pub fn parts_of(bytes: &[u8]) -> BTreeMap<String, String> {
        let mut archive = zip::ZipArchive::new(io::Cursor::new(bytes)).unwrap();
        let mut out = BTreeMap::new();
        for i in 0..archive.len() {
            let mut f = archive.by_index(i).unwrap();
            let mut s = String::new();
            f.read_to_string(&mut s).unwrap();
            out.insert(f.name().to_owned(), s);
        }
        out
    }
    pub fn assert_well_formed(name: &str, xml: &str) {
        let mut reader = quick_xml::Reader::from_str(xml);
        loop {
            match reader.read_event() {
                Ok(quick_xml::events::Event::Eof) => break,
                Ok(_) => {}
                Err(e) => panic!("{name} is not well-formed XML: {e}\n{}", &xml[..xml.len().min(400)]),
            }
        }
    }
    fn docx(id: &str, edit: impl FnOnce(&mut Preset)) -> (Vec<u8>, Doc, Preset) {
        let mut p = builtins().into_iter().find(|p| p.id == id).unwrap();
        edit(&mut p);
        let doc = build(&source(), &Scope::Whole, &p);
        (write(&doc, &p).unwrap(), doc, p)
    }

    #[test]
    fn every_part_is_well_formed_and_wired_up() {
        let (bytes, _, _) = docx(preset::MANUSCRIPT_DOCX, |_| {});
        let parts = parts_of(&bytes);
        assert_eq!(parts.keys().next().unwrap(), "[Content_Types].xml", "content types first");
        for (name, xml) in &parts {
            assert_well_formed(name, xml);
        }
        let rels = &parts["word/_rels/document.xml.rels"];
        for target in ["styles.xml", "settings.xml", "header1.xml", "header3.xml"] {
            assert!(rels.contains(target), "{target} missing from relationships");
            assert!(parts.contains_key(&format!("word/{target}")));
        }
        // Every part has a content type.
        let types = &parts["[Content_Types].xml"];
        for name in parts.keys().filter(|n| n.starts_with("word/") && !n.contains("_rels")) {
            assert!(types.contains(&format!("/{name}")), "{name} has no content type");
        }
        // Every style the document uses is defined.
        let (doc, styles) = (&parts["word/document.xml"], &parts["word/styles.xml"]);
        for chunk in doc.split("<w:pStyle w:val=\"").skip(1) {
            let id = chunk.split('"').next().unwrap();
            assert!(styles.contains(&format!("w:styleId=\"{id}\"")), "style {id} is used but not defined");
        }
    }

    #[test]
    fn the_standard_manuscript_has_the_required_layout() {
        let (bytes, doc, _) = docx(preset::MANUSCRIPT_DOCX, |_| {});
        let parts = parts_of(&bytes);
        let (document, styles, header) = (&parts["word/document.xml"], &parts["word/styles.xml"], &parts["word/header1.xml"]);
        // 12 pt Times New Roman, double spaced, ragged right, half-inch indent, no paragraph gap.
        assert!(styles.contains("w:ascii=\"Times New Roman\"") && styles.contains("<w:sz w:val=\"24\"/>"));
        assert!(styles.contains("w:line=\"480\" w:lineRule=\"auto\""));
        assert!(styles.contains("<w:ind w:firstLine=\"720\"/>") && styles.contains("<w:jc w:val=\"left\"/>"));
        assert!(styles.contains("w:styleId=\"Body\"") && styles.contains("<w:spacing w:after=\"0\"/>"));
        // One-inch margins on US Letter.
        assert!(document.contains("<w:pgSz w:w=\"12240\" w:h=\"15840\"/>"));
        assert!(document.contains("w:top=\"1440\" w:right=\"1440\" w:bottom=\"1440\" w:left=\"1440\""));
        // Title page with contact info, the rounded word count and the title in capitals.
        assert!(document.contains("Jane Q. Doe</w:t></w:r><w:r><w:tab/></w:r>"));
        assert!(document.contains("about 1,000 words") || document.contains("about 100 words"), "rounded count");
        assert!(document.contains("THE LONG NIGHT") && document.contains("by J. Q. Smith"));
        // Running head: Surname / TITLE / page number field.
        assert!(header.contains("Smith / NIGHT / ") && header.contains("PAGE") && header.contains("w:val=\"right\""));
        // Chapters start about a third of the way down: 0.33 of the 9 inch text height.
        assert!(document.contains("<w:spacing w:before=\"4276\"/>"), "0.33 of the 9 inch text height");
        assert!(document.contains("<w:pageBreakBefore/>"));
        // Scene breaks are the centered `#`.
        assert!(document.contains("<w:pStyle w:val=\"SceneBreak\"/></w:pPr><w:r><w:t xml:space=\"preserve\">#</w:t>"));
        // Italics stay italics by default, and notes never appear.
        assert!(document.contains("<w:i/>") && !document.contains("cut this") && !document.contains("lighthouse"));
        assert_eq!(doc.chapters.len(), 3);
        // Chapter headings carry the number and the title in one paragraph.
        assert!(document.contains("Chapter 2</w:t></w:r><w:r><w:br/></w:r><w:r><w:t xml:space=\"preserve\">The Storm"));
        // The title page is its own section without a running head.
        assert!(document.contains("<w:sectPr><w:type w:val=\"nextPage\"/>"));
        assert!(document.matches("<w:sectPr>").count() == 2);
        assert!(document.contains("<w:pgNumType w:start=\"1\"/>"));
    }

    #[test]
    fn underline_option_replaces_italics() {
        let (bytes, _, _) = docx(preset::MANUSCRIPT_DOCX, |p| p.italics = Italics::Underline);
        let document = &parts_of(&bytes)["word/document.xml"];
        assert!(document.contains("<w:u w:val=\"single\"/>") && !document.contains("<w:i/>"));
        assert!(document.contains("<w:b/>"), "bold stays bold");
    }

    #[test]
    fn comments_become_word_comments_with_ranges() {
        let mut src = source();
        let id = src.tree.leaves(crate::jw::manifest::Section::Manuscript)[0].0.id().to_owned();
        let text = src.texts[&id].clone();
        let start = text.find("whisper").unwrap();
        let c = src.comments.add(&id, &text, start..start + 7, "Too soft? Use <b> & more", "prose", 1_760_000_000).unwrap();
        src.comments.reply(&c, "Done", 1_760_000_100).unwrap();
        let mut p = builtins().into_iter().find(|p| p.id == preset::MANUSCRIPT_DOCX).unwrap();
        p.comments = CommentsMode::Docx;
        let doc = build(&src, &Scope::Whole, &p);
        let parts = parts_of(&write(&doc, &p).unwrap());
        for (name, xml) in &parts {
            assert_well_formed(name, xml);
        }
        let comments = &parts["word/comments.xml"];
        assert!(comments.contains("w:id=\"0\"") && comments.contains("w:author=\"J. Q. Smith\"") && comments.contains("w:initials=\"JQS\""));
        assert!(comments.contains("[prose] Too soft? Use &lt;b&gt; &amp; more") && comments.contains("Reply ("));
        let document = &parts["word/document.xml"];
        assert!(document.contains("<w:commentRangeStart w:id=\"0\"/>") && document.contains("<w:commentReference w:id=\"0\"/>"));
        let a = document.find("commentRangeStart").unwrap();
        let b = document.find("commentRangeEnd").unwrap();
        assert!(document[a..b].contains("whisper"));
        assert!(parts["[Content_Types].xml"].contains("comments+xml"));
        assert!(parts["word/_rels/document.xml.rels"].contains("comments.xml"));
    }

    #[test]
    fn mirrored_print_layouts_use_alternating_heads() {
        let (bytes, _, _) = docx(preset::PRINT_BOOK, |p| p.format = preset::Format::Docx);
        let parts = parts_of(&bytes);
        let settings = &parts["word/settings.xml"];
        assert!(settings.contains("<w:mirrorMargins/>") && settings.contains("<w:evenAndOddHeaders/>") && settings.contains("<w:autoHyphenation/>"));
        assert!(parts["word/header1.xml"].contains("THE LONG NIGHT") || parts["word/header1.xml"].contains("NIGHT"));
        assert!(parts["word/header2.xml"].contains("J. Q. Smith"));
        assert!(parts.contains_key("word/footer1.xml") && parts["word/footer1.xml"].contains("PAGE"));
        let doc = &parts["word/document.xml"];
        assert!(doc.contains("w:gutter=\"216\""));
        assert!(doc.contains("<w:titlePg/>"), "chapter openings use the blank first-page header");
        // Justified body text with a quarter-inch indent.
        assert!(parts["word/styles.xml"].contains("<w:ind w:firstLine=\"360\"/><w:jc w:val=\"both\"/>"));
    }

    #[test]
    fn xml_special_characters_and_control_characters_are_escaped() {
        assert_eq!(esc("a & b < c > d \" e \u{1}\u{7f}"), "a &amp; b &lt; c &gt; d &quot; e \u{7f}");
        let mut src = source();
        src.title = "Tom & Jerry <3 \u{1}".into();
        src.book.pen_name = "A \"B\" C".into();
        let p = builtins().into_iter().find(|p| p.id == preset::MANUSCRIPT_DOCX).unwrap();
        let doc = build(&src, &Scope::Whole, &p);
        for (name, xml) in &parts_of(&write(&doc, &p).unwrap()) {
            assert_well_formed(name, xml);
        }
    }
}

#[cfg(test)]
mod samples {
    use super::super::{
        model::{Scope, build, testing::source},
        preset::{self, CommentsMode, builtins},
    };
    use crate::core::notes::{Marker, to_neutral};

    /// Writes sample exports for looking at with real software:
    /// `JW_SAMPLES=/some/dir cargo test --lib samples -- --ignored`
    #[test]
    #[ignore]
    fn write_samples() {
        let Some(dir) = std::env::var_os("JW_SAMPLES") else { return };
        let dir = std::path::PathBuf::from(dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut src = source();
        let words = "the old lighthouse keeper watched storm clouds gather over grey water while gulls wheeled and cried above rocks that had seen a thousand wrecks";
        let paragraph = |n: usize| {
            let w: Vec<&str> = words.split(' ').collect();
            let mut s = String::new();
            for i in 0..n {
                s.push_str(w[(i * 7 + n) % w.len()]);
                s.push(' ');
            }
            let mut c = s.trim().to_owned();
            if let Some(f) = c.get(..1) {
                c = format!("{}{}.", f.to_uppercase(), &c[1..]);
            }
            c
        };
        let marker = Marker::default();
        let ids: Vec<String> = src.tree.leaves(crate::jw::manifest::Section::Manuscript).iter().map(|(n, _)| n.id().to_owned()).collect();
        for (k, id) in ids.iter().enumerate() {
            let mut text = String::new();
            for i in 0..14 + k * 3 {
                text.push_str(&format!("\"{}\" she said, and *{}* **{}**.\n", paragraph(40 + i % 9 * 4), paragraph(3), paragraph(2)));
                if i % 6 == 5 {
                    text.push_str("#\n");
                }
                if i == 4 {
                    text.push_str("$$ a note to myself\n");
                }
            }
            src.texts.insert(id.clone(), to_neutral(&text, &marker).0);
        }
        let first = ids[0].clone();
        let text = src.texts[&first].clone();
        src.comments.add(&first, &text, 20..60, "This line drags — tighten it.", "prose", 1_760_000_000).unwrap();
        for id in [preset::MANUSCRIPT_DOCX, preset::PRINT_BOOK] {
            let mut p = builtins().into_iter().find(|p| p.id == id).unwrap();
            p.format = preset::Format::Docx;
            p.comments = CommentsMode::Docx;
            let doc = build(&src, &Scope::Whole, &p);
            std::fs::write(dir.join(format!("{id}.docx")), super::write(&doc, &p).unwrap()).unwrap();
        }
    }
}
