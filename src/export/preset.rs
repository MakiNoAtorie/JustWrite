//! Export presets: every value of an export, named and reusable. The built-in presets are
//! starting points; all of their values can be edited and saved as new presets.

use crate::core::typography::Language;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Format {
    #[serde(rename = "txt")]
    PlainText,
    #[serde(rename = "md")]
    Markdown,
    #[default]
    Docx,
    Pdf,
    Epub,
}
impl Format {
    pub const ALL: [Format; 5] = [Format::PlainText, Format::Markdown, Format::Docx, Format::Pdf, Format::Epub];
    pub fn extension(self) -> &'static str {
        match self {
            Self::PlainText => "txt",
            Self::Markdown => "md",
            Self::Docx => "docx",
            Self::Pdf => "pdf",
            Self::Epub => "epub",
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Self::PlainText => "Plain text",
            Self::Markdown => "Markdown",
            Self::Docx => "Word (DOCX)",
            Self::Pdf => "PDF",
            Self::Epub => "EPUB 3",
        }
    }
    /// Formats that are laid out on pages.
    pub fn paged(self) -> bool {
        matches!(self, Self::Docx | Self::Pdf)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CommentsMode {
    #[default]
    Off,
    /// Review comments as Word comments in the margin (DOCX only; others use the appendix).
    Docx,
    /// A list of comments after the last chapter.
    Appendix,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ChapterHeading {
    None,
    Number,
    Title,
    #[default]
    NumberAndTitle,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NumberStyle {
    #[default]
    Arabic,
    Roman,
    Words,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Italics {
    #[default]
    Italic,
    /// Underlined, as typewritten manuscripts mark emphasis.
    Underline,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RunningHead {
    None,
    /// `Surname / TITLE / page`, right-aligned, as in a submission manuscript.
    #[default]
    SurnameTitlePage,
    /// Author name on left-hand pages, title on right-hand pages.
    Alternating,
    /// The title of the current chapter.
    ChapterTitle,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PageNumbers {
    /// Not printed (the running head may carry them).
    None,
    HeaderRight,
    #[default]
    FooterCenter,
    /// Bottom outside corner.
    FooterOutside,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Align {
    Left,
    #[default]
    Center,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Caps {
    #[default]
    None,
    Upper,
    SmallCaps,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EpubTheme {
    #[default]
    Classic,
    Modern,
    Plain,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HeadingStyle {
    /// `# Title`
    #[default]
    Hash,
    /// `Title` underlined with `=`
    Underline,
    /// `TITLE`
    Upper,
    Plain,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Preset {
    pub id: String,
    pub name: String,
    pub format: Format,
    pub builtin: bool,

    // Content
    pub include_notes: bool,
    pub comments: CommentsMode,
    /// Leave out comments that are marked resolved.
    pub comments_open_only: bool,

    // Matter
    pub title_page: bool,
    /// Contact details at the top left of the title page.
    pub title_page_contact: bool,
    /// "about 82,000 words" on the title page.
    pub rounded_word_count: bool,
    pub toc: bool,
    pub chapter_heading: ChapterHeading,
    pub number_style: NumberStyle,
    pub part_headings: bool,
    pub scene_break: String,
    pub dedication: bool,
    pub epigraph: bool,
    pub copyright_page: bool,
    pub acknowledgments: bool,
    pub about_author: bool,

    // Page (inches)
    pub page_width: f32,
    pub page_height: f32,
    pub margin_top: f32,
    pub margin_bottom: f32,
    pub margin_inside: f32,
    pub margin_outside: f32,
    pub mirrored: bool,
    pub gutter: f32,
    pub bleed: f32,

    // Text
    pub font: String,
    pub font_size: f32,
    /// 1.0 is single spacing, 2.0 double.
    pub line_spacing: f32,
    pub justify: bool,
    pub hyphenate: bool,
    /// First-line indent in inches.
    pub indent: f32,
    /// Space between paragraphs in points.
    pub paragraph_space: f32,
    /// No indent on the first paragraph of a chapter or after a scene break.
    pub flush_after_break: bool,
    pub italics: Italics,

    // Running heads and page numbers
    pub running_head: RunningHead,
    pub page_numbers: PageNumbers,
    pub head_on_chapter_start: bool,

    // Chapter openings
    pub chapter_new_page: bool,
    /// Fraction of the page height left empty above the chapter heading.
    pub chapter_drop: f32,
    pub chapter_align: Align,
    pub chapter_size: f32,
    pub chapter_caps: Caps,
    pub chapter_bold: bool,
    /// A line of ornament under the heading, such as ❦.
    pub chapter_ornament: String,
    /// 0 off, 1 raised initial, 2-4 dropped over that many lines.
    pub drop_cap: u8,
    /// Chapters start on right-hand pages; blank pages are inserted where needed.
    pub recto_start: bool,

    // EPUB
    pub epub_theme: EpubTheme,
    pub epub_cover: bool,

    // Plain text and Markdown
    pub heading_style: HeadingStyle,
}

impl Default for Preset {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: "Untitled preset".into(),
            format: Format::Docx,
            builtin: false,
            include_notes: false,
            comments: CommentsMode::Off,
            comments_open_only: true,
            title_page: true,
            title_page_contact: false,
            rounded_word_count: false,
            toc: false,
            chapter_heading: ChapterHeading::NumberAndTitle,
            number_style: NumberStyle::Arabic,
            part_headings: true,
            scene_break: "* * *".into(),
            dedication: false,
            epigraph: false,
            copyright_page: false,
            acknowledgments: false,
            about_author: false,
            page_width: 8.5,
            page_height: 11.0,
            margin_top: 1.0,
            margin_bottom: 1.0,
            margin_inside: 1.0,
            margin_outside: 1.0,
            mirrored: false,
            gutter: 0.0,
            bleed: 0.0,
            font: "Libertinus Serif".into(),
            font_size: 11.0,
            line_spacing: 1.2,
            justify: false,
            hyphenate: false,
            indent: 0.3,
            paragraph_space: 0.0,
            flush_after_break: true,
            italics: Italics::Italic,
            running_head: RunningHead::None,
            page_numbers: PageNumbers::FooterCenter,
            head_on_chapter_start: false,
            chapter_new_page: true,
            chapter_drop: 0.15,
            chapter_align: Align::Center,
            chapter_size: 18.0,
            chapter_caps: Caps::None,
            chapter_bold: true,
            chapter_ornament: String::new(),
            drop_cap: 0,
            recto_start: false,
            epub_theme: EpubTheme::Classic,
            epub_cover: true,
            heading_style: HeadingStyle::Hash,
        }
    }
}

/// Page sizes offered in the dialog: name and size in inches.
pub const TRIMS: &[(&str, f32, f32)] = &[
    ("5 × 8 in", 5.0, 8.0),
    ("5.25 × 8 in", 5.25, 8.0),
    ("5.5 × 8.5 in", 5.5, 8.5),
    ("6 × 9 in", 6.0, 9.0),
    ("US Letter", 8.5, 11.0),
    ("A4", 8.27, 11.69),
];
impl Preset {
    pub fn trim_name(&self) -> &'static str {
        TRIMS
            .iter()
            .find(|(_, w, h)| (self.page_width - w).abs() < 0.01 && (self.page_height - h).abs() < 0.01)
            .map_or("Custom", |t| t.0)
    }
    pub fn set_trim(&mut self, name: &str) {
        if let Some((_, w, h)) = TRIMS.iter().find(|t| t.0 == name) {
            self.page_width = *w;
            self.page_height = *h;
        }
    }
    /// Clamps every value to something that produces a usable document.
    pub fn sanitize(&mut self) {
        let clamp = |v: &mut f32, lo: f32, hi: f32| *v = if v.is_finite() { v.clamp(lo, hi) } else { lo };
        clamp(&mut self.page_width, 3.0, 20.0);
        clamp(&mut self.page_height, 4.0, 30.0);
        let (w, h) = (self.page_width, self.page_height);
        clamp(&mut self.margin_top, 0.0, h / 3.0);
        clamp(&mut self.margin_bottom, 0.0, h / 3.0);
        clamp(&mut self.margin_inside, 0.0, w / 3.0);
        clamp(&mut self.margin_outside, 0.0, w / 3.0);
        clamp(&mut self.gutter, 0.0, 1.5);
        clamp(&mut self.bleed, 0.0, 0.5);
        clamp(&mut self.font_size, 6.0, 40.0);
        clamp(&mut self.line_spacing, 0.8, 4.0);
        clamp(&mut self.indent, 0.0, 2.0);
        clamp(&mut self.paragraph_space, 0.0, 36.0);
        clamp(&mut self.chapter_drop, 0.0, 0.6);
        clamp(&mut self.chapter_size, 6.0, 72.0);
        self.drop_cap = self.drop_cap.min(4);
        if self.font.trim().is_empty() {
            self.font = "Libertinus Serif".into();
        }
        self.name = self.name.trim().to_owned();
        if self.name.is_empty() {
            self.name = "Untitled preset".into();
        }
    }
}

pub const MANUSCRIPT_DOCX: &str = "builtin-manuscript-docx";
pub const MANUSCRIPT_PDF: &str = "builtin-manuscript-pdf";
pub const PRINT_BOOK: &str = "builtin-print-book";
pub const EPUB: &str = "builtin-epub";
pub const MARKDOWN: &str = "builtin-markdown";
pub const PLAIN: &str = "builtin-plain";

fn manuscript(format: Format) -> Preset {
    Preset {
        id: if format == Format::Docx { MANUSCRIPT_DOCX } else { MANUSCRIPT_PDF }.into(),
        name: format!("Standard manuscript — {}", if format == Format::Docx { "Word" } else { "PDF" }),
        format,
        builtin: true,
        title_page: true,
        title_page_contact: true,
        rounded_word_count: true,
        scene_break: "#".into(),
        chapter_heading: ChapterHeading::NumberAndTitle,
        number_style: NumberStyle::Arabic,
        page_width: 8.5,
        page_height: 11.0,
        margin_top: 1.0,
        margin_bottom: 1.0,
        margin_inside: 1.0,
        margin_outside: 1.0,
        font: "Times New Roman".into(),
        font_size: 12.0,
        line_spacing: 2.0,
        justify: false,
        hyphenate: false,
        indent: 0.5,
        paragraph_space: 0.0,
        flush_after_break: false,
        italics: Italics::Italic,
        running_head: RunningHead::SurnameTitlePage,
        page_numbers: PageNumbers::None,
        head_on_chapter_start: true,
        chapter_new_page: true,
        chapter_drop: 0.33,
        chapter_align: Align::Center,
        chapter_size: 12.0,
        chapter_caps: Caps::None,
        chapter_bold: false,
        part_headings: true,
        ..Preset::default()
    }
}
fn print_book() -> Preset {
    Preset {
        id: PRINT_BOOK.into(),
        name: "Print book (self-publishing)".into(),
        format: Format::Pdf,
        builtin: true,
        title_page: true,
        toc: false,
        copyright_page: true,
        dedication: true,
        epigraph: true,
        acknowledgments: true,
        about_author: true,
        scene_break: "* * *".into(),
        page_width: 5.5,
        page_height: 8.5,
        margin_top: 0.75,
        margin_bottom: 0.85,
        margin_inside: 0.8,
        margin_outside: 0.6,
        mirrored: true,
        gutter: 0.15,
        bleed: 0.0,
        font: "Libertinus Serif".into(),
        font_size: 11.0,
        line_spacing: 1.2,
        justify: true,
        hyphenate: true,
        indent: 0.25,
        paragraph_space: 0.0,
        flush_after_break: true,
        running_head: RunningHead::Alternating,
        page_numbers: PageNumbers::FooterCenter,
        head_on_chapter_start: false,
        chapter_new_page: true,
        chapter_drop: 0.2,
        chapter_align: Align::Center,
        chapter_size: 20.0,
        chapter_caps: Caps::SmallCaps,
        chapter_bold: false,
        chapter_ornament: String::new(),
        drop_cap: 0,
        recto_start: true,
        ..Preset::default()
    }
}
fn epub() -> Preset {
    Preset {
        id: EPUB.into(),
        name: "E-book (EPUB 3)".into(),
        format: Format::Epub,
        builtin: true,
        title_page: true,
        toc: true,
        copyright_page: true,
        dedication: true,
        epigraph: true,
        acknowledgments: true,
        about_author: true,
        scene_break: "* * *".into(),
        justify: true,
        hyphenate: true,
        epub_theme: EpubTheme::Classic,
        epub_cover: true,
        ..Preset::default()
    }
}
fn markdown(format: Format) -> Preset {
    Preset {
        id: if format == Format::Markdown { MARKDOWN } else { PLAIN }.into(),
        name: if format == Format::Markdown { "Markdown" } else { "Plain text" }.into(),
        format,
        builtin: true,
        title_page: true,
        toc: false,
        scene_break: if format == Format::Markdown { "***" } else { "#" }.into(),
        heading_style: if format == Format::Markdown { HeadingStyle::Hash } else { HeadingStyle::Upper },
        ..Preset::default()
    }
}

pub fn builtins() -> Vec<Preset> {
    vec![
        manuscript(Format::Docx),
        manuscript(Format::Pdf),
        print_book(),
        epub(),
        markdown(Format::Markdown),
        markdown(Format::PlainText),
    ]
}

/// Built-in presets plus the project's own, which live in `export-presets.json`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Presets {
    pub custom: Vec<Preset>,
}
#[derive(Serialize, Deserialize)]
struct File {
    #[serde(default = "one")]
    version: u32,
    #[serde(default)]
    presets: Vec<Preset>,
}
fn one() -> u32 {
    1
}
impl Presets {
    pub fn parse(bytes: &[u8]) -> Self {
        let file: File = serde_json::from_slice(bytes).unwrap_or(File { version: 1, presets: Vec::new() });
        let mut custom = file.presets;
        for p in &mut custom {
            p.builtin = false;
            p.sanitize();
            if p.id.is_empty() || p.id.starts_with("builtin-") {
                p.id = crate::jw::new_id();
            }
        }
        Self { custom }
    }
    pub fn to_json(&self) -> String {
        let file = File { version: 1, presets: self.custom.clone() };
        serde_json::to_string_pretty(&file).expect("serialize presets")
    }
    pub fn all(&self) -> Vec<Preset> {
        let mut all = builtins();
        all.extend(self.custom.iter().cloned());
        all
    }
    pub fn get(&self, id: &str) -> Option<Preset> {
        self.all().into_iter().find(|p| p.id == id)
    }
    /// Saves `preset` under its id (user presets only; a built-in is saved as a new copy).
    pub fn save(&mut self, mut preset: Preset) -> String {
        preset.sanitize();
        preset.builtin = false;
        if preset.id.is_empty() || preset.id.starts_with("builtin-") {
            preset.id = crate::jw::new_id();
        }
        let id = preset.id.clone();
        match self.custom.iter_mut().find(|p| p.id == id) {
            Some(existing) => *existing = preset,
            None => self.custom.push(preset),
        }
        id
    }
    pub fn delete(&mut self, id: &str) -> bool {
        let before = self.custom.len();
        self.custom.retain(|p| p.id != id);
        self.custom.len() != before
    }
}

/// Words for a chapter or part number.
pub fn number_text(n: usize, style: NumberStyle, language: Language) -> String {
    match style {
        NumberStyle::Arabic => n.to_string(),
        NumberStyle::Roman => roman(n),
        NumberStyle::Words => words(n, language).unwrap_or_else(|| n.to_string()),
    }
}
fn roman(mut n: usize) -> String {
    if n == 0 || n >= 4000 {
        return n.to_string();
    }
    let table = [(1000, "M"), (900, "CM"), (500, "D"), (400, "CD"), (100, "C"), (90, "XC"), (50, "L"), (40, "XL"), (10, "X"), (9, "IX"), (5, "V"), (4, "IV"), (1, "I")];
    let mut out = String::new();
    for (value, text) in table {
        while n >= value {
            out.push_str(text);
            n -= value;
        }
    }
    out
}
fn words(n: usize, language: Language) -> Option<String> {
    if n == 0 || n > 99 {
        return None;
    }
    Some(match language {
        Language::English => {
            const ONES: [&str; 20] = ["", "One", "Two", "Three", "Four", "Five", "Six", "Seven", "Eight", "Nine", "Ten", "Eleven", "Twelve", "Thirteen", "Fourteen", "Fifteen", "Sixteen", "Seventeen", "Eighteen", "Nineteen"];
            const TENS: [&str; 10] = ["", "", "Twenty", "Thirty", "Forty", "Fifty", "Sixty", "Seventy", "Eighty", "Ninety"];
            if n < 20 {
                ONES[n].to_owned()
            } else if n % 10 == 0 {
                TENS[n / 10].to_owned()
            } else {
                format!("{}-{}", TENS[n / 10], ONES[n % 10])
            }
        }
        Language::Spanish => {
            const BASE: [&str; 30] = ["", "Uno", "Dos", "Tres", "Cuatro", "Cinco", "Seis", "Siete", "Ocho", "Nueve", "Diez", "Once", "Doce", "Trece", "Catorce", "Quince", "Dieciséis", "Diecisiete", "Dieciocho", "Diecinueve", "Veinte", "Veintiuno", "Veintidós", "Veintitrés", "Veinticuatro", "Veinticinco", "Veintiséis", "Veintisiete", "Veintiocho", "Veintinueve"];
            const TENS: [&str; 10] = ["", "", "", "Treinta", "Cuarenta", "Cincuenta", "Sesenta", "Setenta", "Ochenta", "Noventa"];
            if n < 30 {
                BASE[n].to_owned()
            } else if n % 10 == 0 {
                TENS[n / 10].to_owned()
            } else {
                format!("{} y {}", TENS[n / 10], BASE[n % 10].to_lowercase())
            }
        }
    })
}

/// "Chapter", "Part" and similar words in the project's language.
pub fn chapter_word(language: Language) -> &'static str {
    match language {
        Language::English => "Chapter",
        Language::Spanish => "Capítulo",
    }
}
pub fn part_word(language: Language) -> &'static str {
    match language {
        Language::English => "Part",
        Language::Spanish => "Parte",
    }
}
/// "about 82,000 words".
pub fn rounded_words(n: u64, language: Language) -> String {
    let step = match n {
        0..=999 => 100,
        1000..=9999 => 500,
        _ => 1000,
    };
    let rounded = ((n + step / 2) / step * step).max(step);
    let (about, words, separator) = match language {
        Language::English => ("about", "words", ','),
        Language::Spanish => ("unas", "palabras", '.'),
    };
    let digits = rounded.to_string();
    let mut grouped = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            grouped.push(separator);
        }
        grouped.push(c);
    }
    format!("{about} {grouped} {words}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn built_in_presets_cover_the_spec() {
        let all = builtins();
        assert_eq!(all.len(), 6);
        let ids: std::collections::HashSet<_> = all.iter().map(|p| p.id.clone()).collect();
        assert_eq!(ids.len(), 6);
        let m = all.iter().find(|p| p.id == MANUSCRIPT_DOCX).unwrap();
        assert_eq!((m.font.as_str(), m.font_size, m.line_spacing), ("Times New Roman", 12.0, 2.0));
        assert_eq!((m.margin_top, m.margin_inside, m.indent, m.paragraph_space), (1.0, 1.0, 0.5, 0.0));
        assert!(!m.justify && m.title_page && m.title_page_contact && m.rounded_word_count);
        assert_eq!(m.running_head, RunningHead::SurnameTitlePage);
        assert_eq!(m.chapter_drop, 0.33);
        assert_eq!(m.scene_break, "#");
        let p = all.iter().find(|p| p.id == PRINT_BOOK).unwrap();
        assert_eq!(p.format, Format::Pdf);
        assert!(p.mirrored && p.justify && p.hyphenate && p.recto_start && p.gutter > 0.0);
        assert_eq!(p.trim_name(), "5.5 × 8.5 in");
        assert_eq!(all.iter().find(|p| p.id == EPUB).unwrap().format, Format::Epub);
        for p in &all {
            let mut c = p.clone();
            c.sanitize();
            assert_eq!(&c, p, "{} must already be valid", p.name);
        }
    }

    #[test]
    fn custom_presets_round_trip_and_never_shadow_built_ins() {
        let mut presets = Presets::default();
        let mut mine = builtins().remove(2);
        mine.name = "My 6x9".into();
        mine.set_trim("6 × 9 in");
        mine.font_size = f32::NAN;
        let id = presets.save(mine);
        assert!(!id.starts_with("builtin-"));
        let loaded = Presets::parse(presets.to_json().as_bytes());
        assert_eq!(loaded.custom.len(), 1);
        let p = loaded.get(&id).unwrap();
        assert_eq!((p.page_width, p.page_height, p.name.as_str()), (6.0, 9.0, "My 6x9"));
        assert!(p.font_size.is_finite() && !p.builtin);
        assert_eq!(loaded.all().len(), 7);
        let mut again = loaded.clone();
        assert!(again.delete(&id) && !again.delete(&id));
        // A file claiming a built-in id cannot replace it.
        let spoof = r#"{"presets":[{"id":"builtin-epub","name":"Evil","format":"txt"}]}"#;
        let parsed = Presets::parse(spoof.as_bytes());
        assert_eq!(parsed.get(EPUB).unwrap().format, Format::Epub);
        assert!(!parsed.custom[0].id.starts_with("builtin-"));
        assert!(Presets::parse(b"nonsense").custom.is_empty());
    }

    #[test]
    fn numbers_and_counts_read_naturally() {
        assert_eq!(number_text(7, NumberStyle::Arabic, Language::English), "7");
        assert_eq!(number_text(14, NumberStyle::Roman, Language::English), "XIV");
        assert_eq!(number_text(21, NumberStyle::Words, Language::English), "Twenty-One");
        assert_eq!(number_text(30, NumberStyle::Words, Language::Spanish), "Treinta");
        assert_eq!(number_text(32, NumberStyle::Words, Language::Spanish), "Treinta y dos");
        assert_eq!(number_text(24, NumberStyle::Words, Language::Spanish), "Veinticuatro");
        assert_eq!(number_text(120, NumberStyle::Words, Language::English), "120");
        assert_eq!(rounded_words(81_730, Language::English), "about 82,000 words");
        assert_eq!(rounded_words(4_260, Language::English), "about 4,500 words");
        assert_eq!(rounded_words(130, Language::English), "about 100 words");
        assert_eq!(rounded_words(1_200_400, Language::Spanish), "unas 1.200.000 palabras");
        assert_eq!(chapter_word(Language::Spanish), "Capítulo");
    }
}
