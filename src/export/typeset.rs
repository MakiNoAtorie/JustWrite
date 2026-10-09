//! Typesetting for PDF and the in-app preview.
//!
//! The document is turned into Typst markup, compiled in memory with the bundled fonts, and the
//! resulting pages are written as a PDF (fonts embedded) or drawn as bitmaps. Both come from the
//! same layout, so the preview shows exactly what the PDF will contain.

use super::{
    model::{Block, Chapter, Doc, Inline, Run, heading_lines, part_heading},
    preset::{self, Align, Caps, Italics, PageNumbers, Preset, RunningHead},
};
use crate::{core::typography::Language, fonts};
use std::{
    collections::HashMap,
    sync::{Arc, OnceLock},
};
use typst::{
    Library, LibraryExt, World, WorldExt,
    diag::{FileError, FileResult, Severity},
    foundations::{Bytes, Datetime, Duration},
    syntax::{FileId, RootedPath, Source, VirtualPath, VirtualRoot},
    text::{Font, FontBook},
    utils::LazyHash,
};
use typst_kit::fonts::FontStore;
use typst_layout::PagedDocument;

fn font_store() -> Arc<FontStore> {
    static STORE: OnceLock<Arc<FontStore>> = OnceLock::new();
    STORE
        .get_or_init(|| {
            let mut store = FontStore::new();
            for face in fonts::BUNDLED {
                store.extend(Font::iter(Bytes::new(face.data)).map(|font| {
                    let info = font.info().clone();
                    (font, info)
                }));
            }
            store.extend(typst_kit::fonts::system());
            Arc::new(store)
        })
        .clone()
}

struct Memory {
    library: LazyHash<Library>,
    fonts: Arc<FontStore>,
    main: Source,
    files: HashMap<FileId, Bytes>,
}
impl World for Memory {
    fn library(&self) -> &LazyHash<Library> {
        &self.library
    }
    fn book(&self) -> &LazyHash<FontBook> {
        self.fonts.book()
    }
    fn main(&self) -> FileId {
        self.main.id()
    }
    fn source(&self, id: FileId) -> FileResult<Source> {
        if id == self.main.id() { Ok(self.main.clone()) } else { Err(FileError::AccessDenied) }
    }
    fn file(&self, id: FileId) -> FileResult<Bytes> {
        self.files.get(&id).cloned().ok_or(FileError::AccessDenied)
    }
    fn font(&self, index: usize) -> Option<Font> {
        self.fonts.font(index)
    }
    fn today(&self, _: Option<Duration>) -> Option<Datetime> {
        None
    }
}

// ---- markup -------------------------------------------------------------------------------

/// Text as Typst markup that means exactly what it says.
fn esc(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 8);
    for c in text.chars() {
        match c {
            '\\' | '#' | '$' | '*' | '_' | '`' | '<' | '>' | '@' | '[' | ']' | '~' | '-' | '\'' | '"' | '/' | '=' | '+' => {
                out.push('\\');
                out.push(c);
            }
            '\n' | '\r' | '\t' => out.push(' '),
            c if (c as u32) < 0x20 => {}
            c => out.push(c),
        }
    }
    out
}
fn lit(text: &str) -> String {
    let mut out = String::from("\"");
    for c in text.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' | '\r' => out.push(' '),
            c if (c as u32) < 0x20 => {}
            c => out.push(c),
        }
    }
    out.push('"');
    out
}
fn inches(value: f32) -> String {
    format!("{value:.4}in")
}

fn run_markup(run: &Run, preset: &Preset) -> String {
    let mut text = esc(&run.text);
    if text.is_empty() {
        return text;
    }
    if run.bold {
        text = format!("#strong[{text}]");
    }
    if run.italic {
        text = match preset.italics {
            Italics::Italic => format!("#emph[{text}]"),
            Italics::Underline => format!("#underline[{text}]"),
        };
    }
    if run.note {
        text = format!("#text(fill: luma(110), style: \"italic\")[{text}]");
    }
    text
}
fn inlines_markup(inlines: &[Inline], preset: &Preset) -> String {
    inlines
        .iter()
        .filter_map(|i| match i {
            Inline::Run(r) => Some(run_markup(r, preset)),
            _ => None,
        })
        .collect()
}

fn font_list(preset: &Preset) -> String {
    let mut list = vec![preset.font.clone()];
    match preset.font.as_str() {
        "Times New Roman" | "Times" => list.push("Tinos".into()),
        "Courier New" | "Courier" => list.push("Courier Prime".into()),
        _ => {}
    }
    list.push("Libertinus Serif".into());
    list.iter().map(|f| lit(f)).collect::<Vec<_>>().join(", ")
}

/// Everything about the page and its furniture, as Typst definitions.
fn prelude(doc: &Doc, preset: &Preset) -> String {
    let p = preset;
    let bleed = p.bleed;
    let (width, height) = (p.page_width + bleed, p.page_height + 2.0 * bleed);
    let inside = p.margin_inside + p.gutter;
    let outside = p.margin_outside + bleed;
    let (top, bottom) = (p.margin_top + bleed, p.margin_bottom + bleed);
    let margin = if p.mirrored || bleed > 0.0 {
        format!("(top: {}, bottom: {}, inside: {}, outside: {})", inches(top), inches(bottom), inches(inside), inches(outside))
    } else {
        format!("(top: {}, bottom: {}, left: {}, right: {})", inches(top), inches(bottom), inches(inside), inches(p.margin_outside))
    };
    let lang = doc.language.code();
    let region = if doc.language == Language::Spanish { "ES" } else { "US" };
    let natural = 1.17;
    let leading = (p.line_spacing * natural - 1.1).max(0.1);
    let spacing = if p.paragraph_space > 0.0 { format!("{}pt", p.paragraph_space) } else { format!("{leading:.3}em") };
    let all_indent = !p.flush_after_break;

    let page_number = "counter(page).display()";
    let head = match p.running_head {
        RunningHead::None => "none".to_owned(),
        RunningHead::SurnameTitlePage => {
            let label = if doc.surname.is_empty() {
                format!("{} / ", esc(&doc.running_title))
            } else {
                format!("{} / {} / ", esc(&doc.surname), esc(&doc.running_title))
            };
            format!("align(right)[{label}#{page_number}]")
        }
        RunningHead::Alternating => format!(
            "if calc.odd(here().page()) {{ align(right)[#smallcaps[{}]] }} else {{ align(left)[#smallcaps[{}]] }}",
            esc(&doc.running_title.to_lowercase()),
            esc(&doc.author.to_lowercase())
        ),
        RunningHead::ChapterTitle => "{ let found = query(selector(<chapter-start>).before(here())); if found.len() > 0 { align(center)[#smallcaps[#found.last().value]] } }".to_owned(),
    };
    let numbers_in_head = matches!(p.running_head, RunningHead::SurnameTitlePage);
    let header_number = if p.page_numbers == PageNumbers::HeaderRight && !numbers_in_head {
        format!("align(right)[#{page_number}]")
    } else {
        "none".to_owned()
    };
    let footer = match p.page_numbers {
        PageNumbers::FooterCenter => format!("align(center)[#{page_number}]"),
        PageNumbers::FooterOutside => format!("if calc.odd(here().page()) {{ align(right)[#{page_number}] }} else {{ align(left)[#{page_number}] }}"),
        _ => "none".to_owned(),
    };
    let head_on_chapter = p.head_on_chapter_start;
    format!(
        r##"#set document(title: {title}, author: {author})
#set text(font: ({fonts}), size: {size}pt, lang: "{lang}", region: "{region}", hyphenate: {hyphenate}, top-edge: "ascender", bottom-edge: "descender")
#set smartquote(enabled: false)
#let is-chapter-page() = {{ query(<chapter-start>).map(m => m.location().page()).contains(here().page()) }}
#let running-head() = context {{
  if {head_on_chapter} == false and is-chapter-page() {{ none }} else {{
    let h = {head}
    let n = {header_number}
    if h == none {{ n }} else if n == none {{ h }} else {{ h; n }}
  }}
}}
#let running-foot() = context {{ {footer} }}
#set page(width: {width}, height: {height}, margin: {margin}, binding: left, header: running-head(), footer: running-foot())
#set par(justify: {justify}, leading: {leading:.3}em, spacing: {spacing}, first-line-indent: (amount: {indent}, all: {all_indent}))
"##,
        title = lit(&doc.title),
        author = lit(&doc.author),
        fonts = font_list(p),
        size = p.font_size,
        hyphenate = p.hyphenate,
        width = inches(width),
        height = inches(height),
        justify = p.justify,
        indent = inches(p.indent),
    )
}

fn chapter_functions(preset: &Preset) -> String {
    let p = preset;
    let recto = p.recto_start;
    let align = if p.chapter_align == Align::Center { "center" } else { "left" };
    let text_height = p.page_height - p.margin_top - p.margin_bottom;
    let drop = text_height * p.chapter_drop;
    let size = p.chapter_size;
    let weight = if p.chapter_bold { "\"bold\"" } else { "\"regular\"" };
    let wrap = |inner: &str| match p.chapter_caps {
        Caps::None => inner.to_owned(),
        Caps::Upper => format!("upper({inner})"),
        Caps::SmallCaps => format!("smallcaps({inner})"),
    };
    let number_line = wrap("num");
    let title_line = wrap("title");
    format!(
        r##"#let chapter-start() = {{
  pagebreak(weak: true)
  if {recto} {{ context {{ if calc.even(here().page()) {{ page(header: none, footer: none)[] }} }} }}
}}
#let chapter(num, title, toc, first: false) = {{
  chapter-start()
  [#metadata(toc) <chapter-start>]
  place(top + left, hide(heading(level: 2, outlined: true)[#toc]))
  v({drop:.3}in)
  align({align})[
    #set par(first-line-indent: 0pt, justify: false)
    #text(size: {size}pt, weight: {weight})[
      #if num != none {{ [#{number_line}] }}
      #if num != none and title != none {{ linebreak() }}
      #if title != none {{ [#{title_line}] }}
    ]
  ]
  v({after:.3}em)
}}
#let part(title) = {{
  chapter-start()
  [#metadata(title) <chapter-start>]
  place(top + left, hide(heading(level: 1, outlined: true)[#title]))
  v({part_drop:.3}in)
  align(center)[#set par(first-line-indent: 0pt); #text(size: {psize}pt)[#{part_title}]]
}}
#let scene-break(symbol) = block(above: 1em, below: 1em, width: 100%, breakable: false)[#align(center)[#set par(first-line-indent: 0pt); #symbol]]
#let matter(title, body) = {{
  chapter-start()
  v({drop:.3}in)
  align(center)[#set par(first-line-indent: 0pt); #text(size: {size}pt)[#smallcaps(title)]]
  v(1.5em)
  body
}}
"##,
        after = 1.6,
        psize = p.chapter_size * 1.15,
        part_drop = text_height * 0.3,
        part_title = wrap("title"),
    )
}

/// Typst markup for the whole book.
pub fn markup(doc: &Doc, preset: &Preset) -> String {
    let p = preset;
    let mut out = prelude(doc, p);
    out.push_str(&chapter_functions(p));
    let spanish = doc.language == Language::Spanish;
    let text_height = p.page_height - p.margin_top - p.margin_bottom;
    let bare = "header: none, footer: none";
    // Title page.
    if p.title_page {
        if p.title_page_contact && !doc.contact.is_empty() {
            let contact = doc.contact.iter().map(|l| esc(l)).collect::<Vec<_>>().join(" \\\n");
            let count = if p.rounded_word_count { esc(&preset::rounded_words(doc.words, doc.language)) } else { String::new() };
            out.push_str(&format!(
                "#page({bare})[\n#set par(leading: 0.3em, first-line-indent: 0pt, justify: false)\n#grid(columns: (1fr, auto), [{contact}], [{count}])\n#v({:.3}in)\n#align(center)[#text(size: {}pt, weight: \"bold\")[{}] \\\n{}]\n]\n",
                text_height * 0.30,
                p.font_size,
                esc(&doc.title.to_uppercase()),
                if doc.author.is_empty() { String::new() } else { format!("#v(1em)\n{} {}", if spanish { "por" } else { "by" }, esc(&doc.author)) },
            ));
        } else {
            let mut lines = format!("#text(size: {}pt, weight: \"bold\")[{}]", p.font_size * 2.2, esc(&doc.title));
            if !doc.subtitle.is_empty() {
                lines.push_str(&format!(" \\\n#v(0.6em)\n#text(size: {}pt, style: \"italic\")[{}]", p.font_size * 1.2, esc(&doc.subtitle)));
            }
            if !doc.author.is_empty() {
                lines.push_str(&format!(" \\\n#v(2em)\n#text(size: {}pt)[{}]", p.font_size * 1.3, esc(&doc.author)));
            }
            if !doc.publisher.is_empty() {
                lines.push_str(&format!("\n#v(1fr)\n#text(size: {}pt, style: \"italic\")[{}]", p.font_size * 0.9, esc(&doc.publisher)));
            }
            out.push_str(&format!("#page({bare})[\n#set par(first-line-indent: 0pt, justify: false)\n#v(0.18 * 1fr)\n#align(center)[{lines}]\n#v(0.5 * 1fr)\n]\n"));
        }
    }
    let centered = |paras: &[Vec<Inline>], style: &str| -> String {
        paras.iter().map(|para| format!("#text(style: \"{style}\")[{}]", inlines_markup(para, p))).collect::<Vec<_>>().join(" \\\n#v(0.5em)\n")
    };
    if !doc.dedication.is_empty() {
        out.push_str(&format!("#page({bare})[\n#set par(first-line-indent: 0pt, justify: false)\n#v(0.3 * 1fr)\n#align(center)[{}]\n#v(0.7 * 1fr)\n]\n", centered(&doc.dedication, "italic")));
    }
    if !doc.epigraph.is_empty() {
        out.push_str(&format!("#page({bare})[\n#set par(first-line-indent: 0pt, justify: false)\n#v(0.3 * 1fr)\n#align(center)[{}]\n#v(0.7 * 1fr)\n]\n", centered(&doc.epigraph, "italic")));
    }
    if !doc.copyright.is_empty() {
        let lines = doc.copyright.iter().map(|l| esc(l)).collect::<Vec<_>>().join(" \\\n");
        out.push_str(&format!("#page({bare})[\n#set par(first-line-indent: 0pt, justify: false)\n#set text(size: 0.8em)\n#v(1fr)\n{lines}\n]\n"));
    }
    if p.toc && !doc.chapters.is_empty() {
        out.push_str(&format!(
            "#page({bare})[\n#set par(first-line-indent: 0pt, justify: false)\n#v(0.1 * 1fr)\n#align(center)[#text(size: {}pt)[#smallcaps[{}]]]\n#v(1.5em)\n#outline(title: none, indent: 1.2em)\n]\n",
            p.chapter_size,
            if spanish { "Contenido" } else { "Contents" }
        ));
    }
    // Chapters.
    out.push_str("#if true { pagebreak(weak: true, to: ");
    out.push_str(if p.recto_start { "\"odd\"" } else { "none" });
    out.push_str(") }\n#counter(page).update(1)\n");
    for chapter in &doc.chapters {
        chapter_markup(chapter, doc, p, &mut out);
    }
    let matter = |title: &str, paras: &[Vec<Inline>], out: &mut String| {
        if paras.is_empty() {
            return;
        }
        let body: String = paras.iter().map(|para| format!("{}\n\n", inlines_markup(para, p))).collect();
        out.push_str(&format!("#matter({}, [\n{body}])\n", lit(title)));
    };
    matter(if spanish { "Agradecimientos" } else { "Acknowledgments" }, &doc.acknowledgments, &mut out);
    matter(if spanish { "Sobre el autor" } else { "About the author" }, &doc.about_author, &mut out);
    if !doc.appendix.is_empty() {
        let mut body = String::new();
        let mut last = String::new();
        for c in &doc.appendix {
            if c.chapter != last {
                body.push_str(&format!("#v(1em)\n#strong[{}]\n\n", esc(&c.chapter)));
                last = c.chapter.clone();
            }
            let tag = if c.tag.is_empty() { String::new() } else { format!("[{}] ", esc(&c.tag)) };
            body.push_str(&format!(
                "#pad(left: 1em)[#set par(first-line-indent: 0pt, justify: false)\n#emph[{}] \\\n{tag}{}]\n\n",
                esc(&c.quote.chars().take(200).collect::<String>()),
                esc(&c.body)
            ));
        }
        out.push_str(&format!("#matter({}, [\n{body}])\n", lit(if spanish { "Comentarios" } else { "Comments" })));
    }
    out
}

fn chapter_markup(chapter: &Chapter, doc: &Doc, p: &Preset, out: &mut String) {
    if let (Some(part), true) = (&chapter.part, p.part_headings) {
        out.push_str(&format!("#part({})\n", lit(&part_heading(part, p, doc.language))));
    }
    let (number, title) = heading_lines(chapter, p, doc.language);
    let toc = match (&number, &title) {
        (Some(n), Some(t)) => format!("{n}: {t}"),
        (Some(n), None) => n.clone(),
        (None, Some(t)) => t.clone(),
        (None, None) => chapter.title.clone(),
    };
    let opt = |v: &Option<String>| v.as_ref().map_or("none".to_owned(), |s| lit(s));
    out.push_str(&format!("#chapter({}, {}, {})\n", opt(&number), opt(&title), lit(&toc)));
    if !p.chapter_ornament.is_empty() {
        out.push_str(&format!("#align(center)[#v(-0.6em)#{}#v(0.8em)]\n", lit(&p.chapter_ornament)));
    }
    let mut first = true;
    // A paragraph that follows a block (a dropped initial) would not be indented by itself.
    let mut indent_next = false;
    for block in &chapter.blocks {
        match block {
            Block::SceneBreak => {
                out.push_str(&format!("#scene-break({})\n", lit(&p.scene_break)));
                first = false;
                indent_next = false;
            }
            Block::Paragraph(inlines) => {
                if indent_next && p.flush_after_break {
                    out.push_str(&format!("#h({})", inches(p.indent)));
                }
                indent_next = false;
                if first && p.drop_cap > 0 {
                    out.push_str(&initial_markup(inlines, p));
                    indent_next = p.drop_cap > 1;
                } else {
                    out.push_str(&inlines_markup(inlines, p));
                }
                out.push_str("\n\n");
                first = false;
            }
        }
    }
    out.push('\n');
}

/// Words of a paragraph (after `skip` characters) as Typst content, with their styles.
fn words_markup(inlines: &[Inline], skip: usize, p: &Preset) -> Vec<String> {
    let mut words = Vec::new();
    let mut current = String::new();
    let mut seen = 0;
    for inline in inlines {
        let Inline::Run(run) = inline else { continue };
        let mut piece = String::new();
        let flush_piece = |piece: &mut String, current: &mut String| {
            if !piece.is_empty() {
                current.push_str(&run_markup(&Run { text: std::mem::take(piece), ..run.clone() }, p));
            }
        };
        for c in run.text.chars() {
            seen += 1;
            if seen <= skip {
                continue;
            }
            if c == ' ' {
                flush_piece(&mut piece, &mut current);
                if !current.is_empty() {
                    words.push(std::mem::take(&mut current));
                }
            } else {
                piece.push(c);
            }
        }
        flush_piece(&mut piece, &mut current);
    }
    if !current.is_empty() {
        words.push(current);
    }
    words
}

/// The first paragraph of a chapter with a raised or dropped initial.
fn initial_markup(inlines: &[Inline], p: &Preset) -> String {
    // The initial is the first letter, with any opening quotation mark before it.
    let text: String = inlines
        .iter()
        .filter_map(|i| if let Inline::Run(r) = i { Some(r.text.as_str()) } else { None })
        .collect();
    let chars: Vec<char> = text.chars().collect();
    let lead = chars.iter().take_while(|c| !c.is_alphanumeric()).count();
    if lead >= chars.len() {
        return inlines_markup(inlines, p);
    }
    let cap: String = chars[..=lead].iter().collect();
    let consumed = lead + 1;
    if p.drop_cap == 1 {
        let mut rest = String::new();
        let mut seen = 0;
        for inline in inlines {
            if let Inline::Run(r) = inline {
                let count = r.text.chars().count();
                if seen + count > consumed {
                    let tail: String = r.text.chars().skip(consumed.saturating_sub(seen)).collect();
                    rest.push_str(&run_markup(&Run { text: tail, ..r.clone() }, p));
                }
                seen += count;
            }
        }
        return format!("#text(size: 1.9em)[{}]{rest}", esc(&cap));
    }
    let words = words_markup(inlines, consumed, p);
    if words.is_empty() {
        return inlines_markup(inlines, p);
    }
    let list = words.iter().map(|w| format!("[{w}]")).collect::<Vec<_>>().join(", ");
    format!("#dropcap({}, {}, ({list},))", p.drop_cap, lit(&cap))
}

/// Sets the initial in a narrow column beside the first `lines` lines, measuring how many words
/// fit there, and flows the rest of the paragraph at full width.
fn dropcap_function() -> &'static str {
    r##"#let dropcap(lines, cap, words) = layout(avail => context {
  let line = measure([Hg]).height
  let lead = par.leading.to-absolute()
  let step = line + lead
  let cap-body = measure(text(top-edge: "cap-height", bottom-edge: "baseline")[H]).height
  let asc = measure(text(top-edge: "ascender", bottom-edge: "baseline")[H]).height
  let unit = 100pt
  let ratio = measure(text(size: unit, top-edge: "cap-height", bottom-edge: "baseline")[H]).height / unit
  let letter = text(size: ((lines - 1) * step + cap-body) / ratio, top-edge: "cap-height", bottom-edge: "baseline")[#cap]
  let lw = measure(letter).width + text.size * 0.18
  let narrow = avail.width - lw
  let hang = lines * line + (lines - 1) * lead
  let setup(t) = { set par(justify: true, first-line-indent: 0pt); t }
  let fits(k) = measure(block(width: narrow, setup(words.slice(0, k).join([ ])))).height <= hang + 0.5pt
  let lo = 0
  let hi = words.len()
  while lo < hi {
    let mid = calc.floor((lo + hi + 1) / 2)
    if fits(mid) { lo = mid } else { hi = mid - 1 }
  }
  set block(spacing: 0pt)
  if lo == 0 {
    [#letter#words.join([ ])]
  } else {
    let head = words.slice(0, lo).join([ ])
    grid(
      columns: (lw, 1fr), column-gutter: 0pt,
      move(dy: asc - cap-body, letter),
      setup[#head#linebreak(justify: lo < words.len())],
    )
    if lo < words.len() {
      block(width: 100%)[#set par(first-line-indent: 0pt); #words.slice(lo).join([ ])]
    }
  }
})
"##
}

// ---- compiling ----------------------------------------------------------------------------

pub struct Typeset {
    pub document: PagedDocument,
    pub warnings: Vec<String>,
}

fn compile_source(markup: String, files: HashMap<FileId, Bytes>) -> Result<Typeset, String> {
    let id = FileId::new(RootedPath::new(VirtualRoot::Project, VirtualPath::new("book.typ").map_err(|e| format!("{e:?}"))?));
    let world = Memory { library: LazyHash::new(Library::default()), fonts: font_store(), main: Source::new(id, markup), files };
    let output = typst::compile::<PagedDocument>(&world);
    let warnings: Vec<String> = output
        .warnings
        .iter()
        .filter(|w| w.severity == Severity::Warning)
        .map(|w| w.message.to_string())
        .filter(|m| !m.contains("unknown font family"))
        .collect();
    match output.output {
        Ok(document) => Ok(Typeset { document, warnings }),
        Err(errors) => Err(errors
            .iter()
            .take(4)
            .map(|e| {
                let at = world.range(e.span).map(|r| world.main.text()[..r.start.min(world.main.text().len())].lines().count());
                match at {
                    Some(line) => format!("{} (markup line {line})", e.message),
                    None => e.message.to_string(),
                }
            })
            .collect::<Vec<_>>()
            .join("; ")),
    }
}

/// Lays the book out.
pub fn typeset(doc: &Doc, preset: &Preset) -> Result<Typeset, String> {
    let mut source = markup(doc, preset);
    if preset.drop_cap > 1 {
        let at = source.find("#let chapter-start").unwrap_or(0);
        source.insert_str(at, dropcap_function());
    }
    compile_source(source, HashMap::new())
}

pub struct Bitmap {
    pub width: u32,
    pub height: u32,
    /// `0x00RRGGBB`, row by row.
    pub pixels: Vec<u32>,
}

impl Typeset {
    pub fn pages(&self) -> usize {
        self.document.pages().len()
    }
    /// Page size in points.
    pub fn size(&self, index: usize) -> Option<(f32, f32)> {
        let page = self.document.pages().get(index)?;
        let size = page.frame.size();
        Some((size.x.to_pt() as f32, size.y.to_pt() as f32))
    }
    pub fn pdf(&self) -> Result<Vec<u8>, String> {
        typst_pdf::pdf(&self.document, &typst_pdf::PdfOptions::default())
            .map_err(|e| e.iter().take(3).map(|d| d.message.to_string()).collect::<Vec<_>>().join("; "))
    }
    /// Draws a page at `pixels_per_point`.
    pub fn render(&self, index: usize, pixels_per_point: f32) -> Option<Bitmap> {
        let page = self.document.pages().get(index)?;
        let pixmap = typst_render::render(page, &typst_render::RenderOptions { pixel_per_pt: (pixels_per_point as f64).into(), render_bleed: false });
        let pixels = pixmap
            .data()
            .chunks_exact(4)
            .map(|p| {
                // Premultiplied RGBA over white paper.
                let a = p[3] as u32;
                let blend = |c: u8| (c as u32 + (255 - a)).min(255);
                blend(p[0]) << 16 | blend(p[1]) << 8 | blend(p[2])
            })
            .collect();
        Some(Bitmap { width: pixmap.width(), height: pixmap.height(), pixels })
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::super::{
        model::{Scope, build, testing::source},
        preset::{self, builtins},
    };
    use super::*;

    pub fn book(id: &str, edit: impl FnOnce(&mut Preset, &mut super::super::model::Source)) -> (Doc, Preset) {
        let mut src = source();
        let mut p = builtins().into_iter().find(|p| p.id == id).unwrap();
        edit(&mut p, &mut src);
        (build(&src, &Scope::Whole, &p), p)
    }

    #[test]
    fn escaping_keeps_markup_characters_literal() {
        assert_eq!(esc("a *b* #c $d$ [e] ~ - 'q' \"r\" // /* = + \\ _ ` < > @"), "a \\*b\\* \\#c \\$d\\$ \\[e\\] \\~ \\- \\'q\\' \\\"r\\\" \\/\\/ \\/\\* \\= \\+ \\\\ \\_ \\` \\< \\> \\@");
        assert_eq!(lit("a \"b\" \\ c\n"), "\"a \\\"b\\\" \\\\ c \"");
        assert_eq!(esc("tab\tbecomes a space and\u{1} goes"), "tab becomes a space and goes");
    }

    #[test]
    fn the_standard_manuscript_typesets_to_pdf() {
        let (doc, p) = book(preset::MANUSCRIPT_PDF, |_, _| {});
        let ts = typeset(&doc, &p).unwrap_or_else(|e| panic!("{e}\n{}", markup(&doc, &p)));
        assert!(ts.pages() >= 4, "title page plus a page per chapter, got {}", ts.pages());
        let (w, h) = ts.size(0).unwrap();
        assert!((w - 612.0).abs() < 1.0 && (h - 792.0).abs() < 1.0, "US Letter, got {w}x{h}");
        let pdf = ts.pdf().unwrap();
        assert!(pdf.starts_with(b"%PDF-") && pdf.len() > 3000);
        assert!(pdf.windows(5).any(|w| w == b"/Font"), "fonts are embedded");
    }

    #[test]
    fn the_print_book_has_the_trim_size_and_starts_chapters_on_right_hand_pages() {
        let (doc, p) = book(preset::PRINT_BOOK, |_, _| {});
        let ts = typeset(&doc, &p).unwrap_or_else(|e| panic!("{e}\n{}", markup(&doc, &p)));
        let (w, h) = ts.size(0).unwrap();
        assert!((w - 5.5 * 72.0).abs() < 1.0 && (h - 8.5 * 72.0).abs() < 1.0, "{w}x{h}");
        assert!(ts.pages() >= 8);
        assert!(ts.render(2, 1.0).is_some() && ts.render(999, 1.0).is_none());
    }

    #[test]
    fn bleed_adds_to_the_page_and_odd_characters_do_not_break_the_markup() {
        let (doc, p) = book(preset::PRINT_BOOK, |p, src| {
            p.bleed = 0.125;
            src.title = "Tom & Jerry: #1 *Best* [Of] $5 <3".into();
            src.book.dedication = "To \"you\" — and #me".into();
        });
        let ts = typeset(&doc, &p).unwrap_or_else(|e| panic!("{e}\n{}", markup(&doc, &p)));
        let (w, h) = ts.size(0).unwrap();
        assert!((w - (5.5 + 0.125) * 72.0).abs() < 1.0 && (h - (8.5 + 0.25) * 72.0).abs() < 1.0, "{w}x{h}");
    }

    #[test]
    fn every_builtin_layout_option_compiles() {
        for drop_cap in [0u8, 1, 2, 3] {
            for (running, numbers) in [
                (RunningHead::None, PageNumbers::None),
                (RunningHead::Alternating, PageNumbers::FooterOutside),
                (RunningHead::ChapterTitle, PageNumbers::HeaderRight),
                (RunningHead::SurnameTitlePage, PageNumbers::FooterCenter),
            ] {
                let (doc, p) = book(preset::PRINT_BOOK, |p, _| {
                    p.drop_cap = drop_cap;
                    p.running_head = running;
                    p.page_numbers = numbers;
                    p.toc = true;
                    p.chapter_ornament = "❦".into();
                    p.recto_start = drop_cap % 2 == 0;
                });
                if let Err(e) = typeset(&doc, &p) {
                    panic!("drop_cap {drop_cap} head {running:?}: {e}\n{}", markup(&doc, &p));
                }
            }
        }
    }

    /// Writes PDFs and page images for looking at: `JW_SAMPLES=/dir cargo test --lib typeset -- --ignored`
    #[test]
    #[ignore]
    fn write_samples() {
        let Some(dir) = std::env::var_os("JW_SAMPLES") else { return };
        let dir = std::path::PathBuf::from(dir);
        std::fs::create_dir_all(&dir).unwrap();
        for (id, name) in [(preset::MANUSCRIPT_PDF, "manuscript"), (preset::PRINT_BOOK, "print")] {
            let (doc, mut p) = book(id, |p, src| {
                if p.id == preset::PRINT_BOOK {
                    p.toc = true;
                    p.drop_cap = 3;
                    p.chapter_ornament = "❦".into();
                }
                let marker = crate::core::notes::Marker::default();
                let ids: Vec<String> = src.tree.leaves(crate::jw::manifest::Section::Manuscript).iter().map(|(n, _)| n.id().to_owned()).collect();
                let words = "the old lighthouse keeper watched storm clouds gather over grey water while gulls wheeled and cried above rocks that had seen a thousand wrecks".split(' ').collect::<Vec<_>>();
                for (k, id) in ids.iter().enumerate() {
                    let mut text = String::new();
                    for i in 0..12 + k * 4 {
                        let mut s = String::new();
                        for j in 0..(40 + (i * 13) % 50) {
                            s.push_str(words[(i * 5 + j * 7 + k) % words.len()]);
                            s.push(' ');
                        }
                        let s = s.trim();
                        text.push_str(&format!("\"{}{}.\" she said, and *{}* **{}**.\n", s[..1].to_uppercase(), &s[1..], words[(i + 3) % words.len()], words[(i + 5) % words.len()]));
                        if i % 7 == 6 {
                            text.push_str("#\n");
                        }
                    }
                    src.texts.insert(id.clone(), crate::core::notes::to_neutral(&text, &marker).0);
                }
            });
            p.sanitize();
            let ts = typeset(&doc, &p).unwrap_or_else(|e| panic!("{e}"));
            std::fs::write(dir.join(format!("{name}.pdf")), ts.pdf().unwrap()).unwrap();
            for i in 0..ts.pages().min(14) {
                let bm = ts.render(i, 1.2).unwrap();
                let mut ppm = format!("P6\n{} {}\n255\n", bm.width, bm.height).into_bytes();
                for px in &bm.pixels {
                    ppm.extend_from_slice(&[(px >> 16) as u8, (px >> 8) as u8, *px as u8]);
                }
                std::fs::write(dir.join(format!("{name}-{i:02}.ppm")), ppm).unwrap();
            }
        }
    }
}
