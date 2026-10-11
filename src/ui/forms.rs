//! Dialogs made of labelled fields: text, numbers, choices, switches and buttons. The same widget
//! serves settings, export, book details, project settings, import and the small prompts.
use super::*;
use super::gui::{Align, Face, Look, count_words};
use crate::{
    core::{
        comments::{Filter as CommentFilterQuery, Status as CommentStatus},
        notes,
        typography::Language,
    },
    export::{
        model::{Scope, Source},
        preset::{self, Align as ChapterAlign, Caps, ChapterHeading, CommentsMode, EpubTheme, Format, HeadingStyle, Italics, NumberStyle, PageNumbers, Preset, RunningHead, TRIMS},
    },
    history,
    import::{self, split},
    jw::{self, manifest::Section},
};
use std::ops::Range;

#[derive(Clone, PartialEq)]
pub enum FormKind {
    Settings,
    Comment { item: String, range: Range<usize> },
    EditComment(String),
    Reply(String),
    DeleteComment(String),
    Snapshot,
    NewNote,
    NewLine,
    RenameLine(String),
    Export,
    Book,
    Project,
    Import,
    Goals,
    Locked(PathBuf),
    Overwrite,
}

#[derive(Clone)]
pub enum Control {
    Text,
    /// Several lines; Enter starts a new one.
    Area(u8),
    Number { min: f32, max: f32, step: f32, decimals: u8, unit: &'static str },
    Choice(Vec<String>),
    Toggle,
    Heading,
    Info,
    Button(String),
}

#[derive(Clone)]
pub struct Field {
    pub key: String,
    pub label: String,
    pub control: Control,
    pub text: String,
    pub index: usize,
    pub on: bool,
    pub hint: String,
    /// Shown only while the choice field `.0` has one of the indices in `.1`.
    pub when: Option<(&'static str, &'static [usize])>,
}
impl Field {
    fn base(key: &str, label: &str, control: Control) -> Self {
        Self { key: key.into(), label: label.into(), control, text: String::new(), index: 0, on: false, hint: String::new(), when: None }
    }
    pub fn text(key: &str, label: &str, value: &str) -> Self {
        Self { text: value.into(), ..Self::base(key, label, Control::Text) }
    }
    pub fn area(key: &str, label: &str, value: &str, rows: u8) -> Self {
        Self { text: value.into(), ..Self::base(key, label, Control::Area(rows)) }
    }
    pub fn number(key: &str, label: &str, value: f32, min: f32, max: f32, step: f32, decimals: u8, unit: &'static str) -> Self {
        Self { text: format_number(value, decimals), ..Self::base(key, label, Control::Number { min, max, step, decimals, unit }) }
    }
    pub fn choice(key: &str, label: &str, options: Vec<String>, index: usize) -> Self {
        let index = index.min(options.len().saturating_sub(1));
        Self { index, ..Self::base(key, label, Control::Choice(options)) }
    }
    pub fn toggle(key: &str, label: &str, on: bool) -> Self {
        Self { on, ..Self::base(key, label, Control::Toggle) }
    }
    pub fn heading(label: &str) -> Self {
        Self::base("", label, Control::Heading)
    }
    pub fn info(text: &str) -> Self {
        Self { text: text.into(), ..Self::base("", "", Control::Info) }
    }
    pub fn button(key: &str, label: &str, caption: &str) -> Self {
        Self::base(key, label, Control::Button(caption.into()))
    }
    pub fn hint(mut self, hint: &str) -> Self {
        self.hint = hint.into();
        self
    }
    pub fn when(mut self, key: &'static str, any: &'static [usize]) -> Self {
        self.when = Some((key, any));
        self
    }
    fn focusable(&self) -> bool {
        !matches!(self.control, Control::Heading | Control::Info)
    }
    fn typed(&self) -> bool {
        matches!(self.control, Control::Text | Control::Area(_) | Control::Number { .. })
    }
    /// Height in logical pixels.
    pub fn height(&self, width: f32) -> f32 {
        match &self.control {
            Control::Heading => 36.,
            Control::Info => {
                let per_line = (width / 7.).max(20.);
                let lines: usize = self.text.lines().map(|l| (l.chars().count() as f32 / per_line).ceil().max(1.) as usize).sum::<usize>().max(1);
                lines as f32 * 19. + 8.
            }
            Control::Area(rows) => *rows as f32 * 22. + 20.,
            _ => 40.,
        }
    }
}

pub fn format_number(value: f32, decimals: u8) -> String {
    let text = format!("{value:.*}", decimals as usize);
    if decimals > 0 {
        text.trim_end_matches('0').trim_end_matches('.').to_owned()
    } else {
        text
    }
}

pub struct Form {
    pub kind: FormKind,
    pub title: String,
    pub intro: String,
    pub fields: Vec<Field>,
    pub focus: usize,
    /// Index of the first field drawn.
    pub scroll: usize,
    pub caret: Option<usize>,
    pub note: String,
    pub submit: Option<String>,
    /// More footer buttons: caption and identifier.
    pub buttons: Vec<(String, &'static str)>,
    pub width: f32,
    /// A dropdown that is open: field and first visible option.
    pub dropdown: Option<(usize, usize)>,
    /// Logical pixels the field list had room for when last painted.
    pub room: f32,
    /// Extra data the submit handler needs.
    pub data: Vec<String>,
    /// The focus was last moved with the keyboard, so the focused row is marked.
    pub keyboard: bool,
    /// The submit button destroys something.
    pub danger: bool,
}
impl Form {
    pub fn new(kind: FormKind, title: &str, fields: Vec<Field>) -> Self {
        let mut form = Self {
            kind,
            title: title.into(),
            intro: String::new(),
            fields,
            focus: 0,
            scroll: 0,
            caret: None,
            note: String::new(),
            submit: Some("OK".into()),
            buttons: Vec::new(),
            width: 600.,
            dropdown: None,
            room: 400.,
            data: Vec::new(),
            keyboard: false,
            danger: false,
        };
        form.focus = form.first_focus();
        form
    }
    pub fn submit(mut self, caption: &str) -> Self {
        self.submit = Some(caption.into());
        self
    }
    pub fn danger(mut self) -> Self {
        self.danger = true;
        self
    }
    pub fn no_submit(mut self) -> Self {
        self.submit = None;
        self
    }
    pub fn intro(mut self, text: &str) -> Self {
        self.intro = text.into();
        self
    }
    pub fn width(mut self, width: f32) -> Self {
        self.width = width;
        self
    }
    pub fn button(mut self, caption: &str, id: &'static str) -> Self {
        self.buttons.push((caption.into(), id));
        self
    }
    fn first_focus(&self) -> usize {
        (0..self.fields.len()).find(|i| self.shown(*i) && self.fields[*i].focusable()).unwrap_or(0)
    }
    pub fn index_of(&self, key: &str) -> Option<usize> {
        self.fields.iter().position(|f| f.key == key)
    }
    pub fn get(&self, key: &str) -> Option<&Field> {
        self.fields.iter().find(|f| f.key == key)
    }
    pub fn text(&self, key: &str) -> String {
        self.get(key).map(|f| f.text.trim().to_owned()).unwrap_or_default()
    }
    /// Text exactly as typed, for multi-line areas.
    pub fn raw(&self, key: &str) -> String {
        self.get(key).map(|f| f.text.clone()).unwrap_or_default()
    }
    pub fn on(&self, key: &str) -> bool {
        self.get(key).is_some_and(|f| f.on)
    }
    pub fn choice(&self, key: &str) -> usize {
        self.get(key).map_or(0, |f| f.index)
    }
    pub fn number(&self, key: &str) -> f32 {
        let Some(field) = self.get(key) else { return 0. };
        let (min, max) = match field.control {
            Control::Number { min, max, .. } => (min, max),
            _ => (f32::MIN, f32::MAX),
        };
        field.text.trim().replace(',', ".").parse::<f32>().ok().filter(|v| v.is_finite()).map_or(min, |v| v.clamp(min, max))
    }
    pub fn set_text(&mut self, key: &str, value: &str) {
        if let Some(f) = self.fields.iter_mut().find(|f| f.key == key) {
            f.text = value.into();
        }
    }
    pub fn set_number(&mut self, key: &str, value: f32) {
        if let Some(f) = self.fields.iter_mut().find(|f| f.key == key) {
            if let Control::Number { decimals, .. } = f.control {
                f.text = format_number(value, decimals);
            }
        }
    }
    pub fn set_on(&mut self, key: &str, on: bool) {
        if let Some(f) = self.fields.iter_mut().find(|f| f.key == key) {
            f.on = on;
        }
    }
    pub fn set_choice(&mut self, key: &str, index: usize) {
        if let Some(f) = self.fields.iter_mut().find(|f| f.key == key) {
            f.index = index;
        }
    }
    pub fn shown(&self, index: usize) -> bool {
        match self.fields[index].when {
            Some((key, any)) => any.contains(&self.choice(key)),
            None => true,
        }
    }
    fn shown_fields(&self) -> Vec<usize> {
        (0..self.fields.len()).filter(|i| self.shown(*i)).collect()
    }
    pub fn move_focus(&mut self, forward: bool) {
        let list: Vec<usize> = self.shown_fields().into_iter().filter(|i| self.fields[*i].focusable()).collect();
        if list.is_empty() {
            return;
        }
        let at = list.iter().position(|i| *i == self.focus);
        let next = match (at, forward) {
            (Some(i), true) => (i + 1) % list.len(),
            (Some(i), false) => (i + list.len() - 1) % list.len(),
            (None, _) => 0,
        };
        self.set_focus(list[next]);
    }
    pub fn set_focus(&mut self, index: usize) {
        self.focus = index;
        self.caret = None;
        self.dropdown = None;
        self.reveal();
    }
    /// Scrolls so the focused field is inside the list.
    pub fn reveal(&mut self) {
        let shown = self.shown_fields();
        let Some(at) = shown.iter().position(|i| *i == self.focus) else { return };
        let start = shown.iter().position(|i| *i >= self.scroll).unwrap_or(0);
        if at < start {
            self.scroll = self.focus;
            return;
        }
        let width = self.width - 56.;
        let mut used = 0.;
        for k in start..=at {
            used += self.fields[shown[k]].height(width);
        }
        let mut first = start;
        while used > self.room && first < at {
            used -= self.fields[shown[first]].height(width);
            first += 1;
        }
        self.scroll = shown[first];
    }
    pub fn scroll_by(&mut self, rows: i32) {
        let shown = self.shown_fields();
        let start = shown.iter().position(|i| *i >= self.scroll).unwrap_or(0) as i32;
        let next = (start + rows).clamp(0, shown.len().saturating_sub(1) as i32) as usize;
        self.scroll = shown.get(next).copied().unwrap_or(0);
    }

    // ---- text editing in the focused field -------------------------------------------------

    fn focused_text(&mut self) -> Option<&mut Field> {
        let f = self.fields.get_mut(self.focus)?;
        f.typed().then_some(f)
    }
    pub fn type_text(&mut self, text: &str) {
        let caret = self.caret;
        let Some(field) = self.focused_text() else { return };
        let multiline = matches!(field.control, Control::Area(_));
        let text: String = text.chars().filter(|c| !c.is_control() || (multiline && *c == '\n')).collect();
        if text.is_empty() {
            return;
        }
        let len = field.text.chars().count();
        let at = caret.unwrap_or(len).min(len);
        let byte = field.text.char_indices().nth(at).map_or(field.text.len(), |(i, _)| i);
        field.text.insert_str(byte, &text);
        let moved = at + text.chars().count();
        self.caret = (moved < len + text.chars().count()).then_some(moved);
        self.note.clear();
    }
    pub fn backspace(&mut self, forward: bool) {
        let caret = self.caret;
        let Some(field) = self.focused_text() else { return };
        let len = field.text.chars().count();
        let at = caret.unwrap_or(len).min(len);
        let (from, to) = if forward { (at, (at + 1).min(len)) } else { (at.saturating_sub(1), at) };
        if from == to {
            return;
        }
        let bytes: Vec<usize> = field.text.char_indices().map(|(i, _)| i).chain([field.text.len()]).collect();
        field.text.replace_range(bytes[from]..bytes[to], "");
        self.caret = (from < len - 1).then_some(from);
        self.note.clear();
    }
    pub fn caret_move(&mut self, delta: i32, edge: Option<bool>) {
        let caret = self.caret;
        let Some(field) = self.focused_text() else { return };
        let len = field.text.chars().count();
        let at = caret.unwrap_or(len).min(len);
        let target = match edge {
            Some(true) => 0,
            Some(false) => len,
            None => (at as i32 + delta).clamp(0, len as i32) as usize,
        };
        self.caret = (target < len).then_some(target);
    }
    pub fn step(&mut self, index: usize, direction: i32) {
        let value = self.fields.get(index).map(|f| f.text.clone()).unwrap_or_default();
        let Some(Control::Number { min, max, step, decimals, .. }) = self.fields.get(index).map(|f| f.control.clone()) else { return };
        let current = value.trim().replace(',', ".").parse::<f32>().unwrap_or(min);
        let next = (current + step * direction as f32).clamp(min, max);
        let rounded = (next / step).round() * step;
        if let Some(f) = self.fields.get_mut(index) {
            f.text = format_number(rounded.clamp(min, max), decimals);
        }
    }
    pub fn cycle(&mut self, index: usize, direction: i32) {
        if let Some(Field { control: Control::Choice(options), index: current, .. }) = self.fields.get_mut(index) {
            let n = options.len().max(1) as i32;
            *current = (*current as i32 + direction).rem_euclid(n) as usize;
        }
    }
}

const PAGED: &[usize] = &[2, 3];
const PDF_ONLY: &[usize] = &[3];
const EPUB_ONLY: &[usize] = &[4];
const FLOWING: &[usize] = &[2, 3, 4];
const TEXT_FORMATS: &[usize] = &[0, 1];
const CUSTOM_TRIM: &[usize] = &[6];

fn labels(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| (*s).to_owned()).collect()
}
fn position<T: PartialEq + Copy>(all: &[T], value: T) -> usize {
    all.iter().position(|v| *v == value).unwrap_or(0)
}

const COMMENT_TAGS: [&str; 6] = ["", "plot", "continuity", "prose", "research", "custom…"];

// ---- export presets as fields --------------------------------------------------------------

const HEADINGS: [ChapterHeading; 4] = [ChapterHeading::None, ChapterHeading::Number, ChapterHeading::Title, ChapterHeading::NumberAndTitle];
const NUMBERS: [NumberStyle; 3] = [NumberStyle::Arabic, NumberStyle::Roman, NumberStyle::Words];
const ITALICS: [Italics; 2] = [Italics::Italic, Italics::Underline];
const HEADS: [RunningHead; 4] = [RunningHead::None, RunningHead::SurnameTitlePage, RunningHead::Alternating, RunningHead::ChapterTitle];
const PAGENUMS: [PageNumbers; 4] = [PageNumbers::None, PageNumbers::HeaderRight, PageNumbers::FooterCenter, PageNumbers::FooterOutside];
const ALIGNS: [ChapterAlign; 2] = [ChapterAlign::Left, ChapterAlign::Center];
const CAPS: [Caps; 3] = [Caps::None, Caps::Upper, Caps::SmallCaps];
const THEMES: [EpubTheme; 3] = [EpubTheme::Classic, EpubTheme::Modern, EpubTheme::Plain];
const STYLES: [HeadingStyle; 4] = [HeadingStyle::Hash, HeadingStyle::Underline, HeadingStyle::Upper, HeadingStyle::Plain];
const COMMENT_MODES: [CommentsMode; 3] = [CommentsMode::Off, CommentsMode::Docx, CommentsMode::Appendix];

fn preset_fields(p: &Preset) -> Vec<Field> {
    let n = |key: &str, label: &str, v: f32, min: f32, max: f32, step: f32, dec: u8, unit: &'static str| Field::number(key, label, v, min, max, step, dec, unit);
    let trim = TRIMS.iter().position(|t| t.0 == p.trim_name()).unwrap_or(TRIMS.len());
    let mut trims: Vec<String> = TRIMS.iter().map(|t| t.0.to_owned()).collect();
    trims.push("Custom".into());
    vec![
        Field::heading("Content"),
        Field::toggle("include_notes", "Include notes", p.include_notes).hint("Notes are removed unless this is on"),
        Field::choice("comments", "Review comments", labels(&["Leave out", "As Word comments (DOCX)", "Appendix after the last chapter"]), position(&COMMENT_MODES, p.comments)),
        Field::toggle("comments_open_only", "Only open comments", p.comments_open_only).when("comments", &[1, 2]),
        Field::toggle("clean_whitespace", "Clean up extra spaces", p.clean_whitespace).hint("Runs of spaces and tabs become one space"),
        Field::heading("Front and back matter"),
        Field::toggle("title_page", "Title page", p.title_page),
        Field::toggle("title_page_contact", "Contact details on the title page", p.title_page_contact),
        Field::toggle("rounded_word_count", "Rounded word count on the title page", p.rounded_word_count),
        Field::toggle("toc", "Table of contents", p.toc),
        Field::toggle("dedication", "Dedication page", p.dedication),
        Field::toggle("epigraph", "Epigraph", p.epigraph),
        Field::toggle("copyright_page", "Copyright page", p.copyright_page),
        Field::toggle("acknowledgments", "Acknowledgments", p.acknowledgments),
        Field::toggle("about_author", "About the author", p.about_author),
        Field::heading("Chapters"),
        Field::choice("chapter_heading", "Chapter titles", labels(&["None", "Number only", "Title only", "Number and title"]), position(&HEADINGS, p.chapter_heading)),
        Field::choice("number_style", "Chapter numbers", labels(&["1, 2, 3", "I, II, III", "One, Two, Three"]), position(&NUMBERS, p.number_style)),
        Field::text("scene_break", "Scene-break symbol", &p.scene_break).hint("For example #, * * * or ❦"),
        Field::heading("Page").when("format", PAGED),
        Field::choice("trim", "Page size", trims, trim).when("format", PAGED),
        n("page_width", "Page width", p.page_width, 3., 20., 0.25, 2, "in").when("trim", CUSTOM_TRIM),
        n("page_height", "Page height", p.page_height, 4., 30., 0.25, 2, "in").when("trim", CUSTOM_TRIM),
        n("margin_top", "Top margin", p.margin_top, 0., 10., 0.05, 2, "in").when("format", PAGED),
        n("margin_bottom", "Bottom margin", p.margin_bottom, 0., 10., 0.05, 2, "in").when("format", PAGED),
        n("margin_inside", "Inside (or left) margin", p.margin_inside, 0., 6., 0.05, 2, "in").when("format", PAGED),
        n("margin_outside", "Outside (or right) margin", p.margin_outside, 0., 6., 0.05, 2, "in").when("format", PAGED),
        Field::toggle("mirrored", "Mirror margins on facing pages", p.mirrored).when("format", PAGED),
        n("gutter", "Gutter", p.gutter, 0., 1.5, 0.05, 2, "in").when("format", PAGED),
        n("bleed", "Bleed", p.bleed, 0., 0.5, 0.05, 2, "in").when("format", PDF_ONLY),
        Field::heading("Text").when("format", FLOWING),
        Field::text("font", "Font", &p.font).when("format", PAGED).hint("Installed font name, or a bundled one"),
        n("font_size", "Font size", p.font_size, 6., 40., 0.5, 1, "pt").when("format", PAGED),
        n("line_spacing", "Line spacing", p.line_spacing, 0.8, 4., 0.1, 1, "×").when("format", PAGED),
        Field::toggle("justify", "Justified text", p.justify).when("format", FLOWING),
        Field::toggle("hyphenate", "Hyphenate words", p.hyphenate).when("format", FLOWING),
        n("indent", "First-line indent", p.indent, 0., 2., 0.05, 2, "in").when("format", PAGED),
        n("paragraph_space", "Space between paragraphs", p.paragraph_space, 0., 36., 1., 0, "pt").when("format", PAGED),
        Field::toggle("flush_after_break", "No indent after a break or chapter start", p.flush_after_break).when("format", PAGED),
        Field::choice("italics", "Emphasis", labels(&["Italic", "Underlined"]), position(&ITALICS, p.italics)).when("format", PAGED),
        Field::heading("Running heads").when("format", PAGED),
        Field::choice("running_head", "Running head", labels(&["None", "Surname / TITLE / page", "Author left, title right", "Chapter title"]), position(&HEADS, p.running_head)).when("format", PAGED),
        Field::choice("page_numbers", "Page numbers", labels(&["None", "Top right", "Bottom centre", "Bottom outside corner"]), position(&PAGENUMS, p.page_numbers)).when("format", PAGED),
        Field::toggle("head_on_chapter_start", "Running head on chapter-opening pages", p.head_on_chapter_start).when("format", PAGED),
        Field::heading("Chapter openings").when("format", PAGED),
        Field::toggle("chapter_new_page", "Start chapters on a new page", p.chapter_new_page).when("format", PAGED),
        n("chapter_drop", "Space above the title", p.chapter_drop, 0., 0.6, 0.05, 2, "").when("format", PAGED),
        Field::choice("chapter_align", "Title alignment", labels(&["Left", "Centred"]), position(&ALIGNS, p.chapter_align)).when("format", PAGED),
        n("chapter_size", "Title size", p.chapter_size, 6., 72., 1., 0, "pt").when("format", PAGED),
        Field::choice("chapter_caps", "Title capitals", labels(&["As typed", "Upper case", "Small capitals"]), position(&CAPS, p.chapter_caps)).when("format", PAGED),
        Field::toggle("chapter_bold", "Bold title", p.chapter_bold).when("format", PAGED),
        Field::text("chapter_ornament", "Ornament under the title", &p.chapter_ornament).when("format", PAGED).hint("Optional, such as ❦"),
        n("drop_cap", "Drop cap", p.drop_cap as f32, 0., 4., 1., 0, "lines (0 is off)").when("format", PDF_ONLY),
        Field::toggle("recto_start", "Chapters start on right-hand pages", p.recto_start).when("format", PDF_ONLY),
        Field::heading("E-book").when("format", EPUB_ONLY),
        Field::choice("epub_theme", "Style", labels(&["Classic", "Modern", "Plain"]), position(&THEMES, p.epub_theme)).when("format", EPUB_ONLY),
        Field::toggle("epub_cover", "Include the cover", p.epub_cover).when("format", EPUB_ONLY),
        Field::heading("Plain text").when("format", TEXT_FORMATS),
        Field::choice("heading_style", "Chapter headings", labels(&["# Title", "Title underlined with =", "TITLE", "Title"]), position(&STYLES, p.heading_style)).when("format", TEXT_FORMATS),
    ]
}

/// Reads the preset fields of `form` onto `base`.
fn read_preset(form: &Form, base: &Preset) -> Preset {
    let mut p = base.clone();
    p.format = Format::ALL[form.choice("format").min(4)];
    p.include_notes = form.on("include_notes");
    p.comments = COMMENT_MODES[form.choice("comments").min(2)];
    p.comments_open_only = form.on("comments_open_only");
    p.clean_whitespace = form.on("clean_whitespace");
    p.title_page = form.on("title_page");
    p.title_page_contact = form.on("title_page_contact");
    p.rounded_word_count = form.on("rounded_word_count");
    p.toc = form.on("toc");
    p.dedication = form.on("dedication");
    p.epigraph = form.on("epigraph");
    p.copyright_page = form.on("copyright_page");
    p.acknowledgments = form.on("acknowledgments");
    p.about_author = form.on("about_author");
    p.chapter_heading = HEADINGS[form.choice("chapter_heading").min(3)];
    p.number_style = NUMBERS[form.choice("number_style").min(2)];
    p.scene_break = form.raw("scene_break").trim().to_owned();
    if p.scene_break.is_empty() {
        p.scene_break = "#".into();
    }
    let trim = form.choice("trim");
    if let Some((_, w, h)) = TRIMS.get(trim) {
        (p.page_width, p.page_height) = (*w, *h);
    } else {
        p.page_width = form.number("page_width");
        p.page_height = form.number("page_height");
    }
    p.margin_top = form.number("margin_top");
    p.margin_bottom = form.number("margin_bottom");
    p.margin_inside = form.number("margin_inside");
    p.margin_outside = form.number("margin_outside");
    p.mirrored = form.on("mirrored");
    p.gutter = form.number("gutter");
    p.bleed = form.number("bleed");
    p.font = form.text("font");
    p.font_size = form.number("font_size");
    p.line_spacing = form.number("line_spacing");
    p.justify = form.on("justify");
    p.hyphenate = form.on("hyphenate");
    p.indent = form.number("indent");
    p.paragraph_space = form.number("paragraph_space");
    p.flush_after_break = form.on("flush_after_break");
    p.italics = ITALICS[form.choice("italics").min(1)];
    p.running_head = HEADS[form.choice("running_head").min(3)];
    p.page_numbers = PAGENUMS[form.choice("page_numbers").min(3)];
    p.head_on_chapter_start = form.on("head_on_chapter_start");
    p.chapter_new_page = form.on("chapter_new_page");
    p.chapter_drop = form.number("chapter_drop");
    p.chapter_align = ALIGNS[form.choice("chapter_align").min(1)];
    p.chapter_size = form.number("chapter_size");
    p.chapter_caps = CAPS[form.choice("chapter_caps").min(2)];
    p.chapter_bold = form.on("chapter_bold");
    p.chapter_ornament = form.text("chapter_ornament");
    p.drop_cap = form.number("drop_cap") as u8;
    p.recto_start = form.on("recto_start");
    p.epub_theme = THEMES[form.choice("epub_theme").min(2)];
    p.epub_cover = form.on("epub_cover");
    p.heading_style = STYLES[form.choice("heading_style").min(3)];
    p.sanitize();
    p
}

/// What an export dialog asked for.
pub struct ExportRequest {
    pub source: Arc<Source>,
    pub scope: Scope,
    pub preset: Preset,
}

impl App {
    fn now(&self) -> i64 {
        jw::time::now()
    }
    /// Writes every changed open chapter now, so what is on disk matches what is on screen.
    pub(super) fn flush_tabs(&mut self) {
        let ids: Vec<u64> = self
            .tabs
            .iter()
            .filter(|(_, t)| (t.document.dirty() || t.marks_stale) && !t.loading && !t.failed && !t.saving && t.path.is_some())
            .map(|(id, _)| *id)
            .collect();
        for id in ids {
            self.sync_marks_to_project(id);
            let Some(tab) = self.tabs.get_mut(&id) else { continue };
            if !tab.document.dirty() {
                continue;
            }
            let (Some(storage), Some(path)) = (tab.storage.clone(), tab.path.clone()) else { continue };
            let text = tab.document.text.clone();
            match storage.write(&path, &text) {
                Ok(()) => {
                    tab.document.mark_saved(tab.document.revision, text);
                    tab.edited = None;
                }
                Err(error) => {
                    let message = format!("Could not save {}: {error}", tab.name());
                    self.fail(message);
                }
            }
        }
    }
    pub(super) fn save_all(&mut self) {
        let ids: Vec<u64> = self.tabs.iter().filter(|(_, t)| t.document.dirty()).map(|(id, _)| *id).collect();
        for id in ids {
            self.save(id, false);
        }
        if self.project.is_some() {
            self.pack_project(false);
        }
        self.status = "Saving…".into();
    }
    pub(super) fn selected_item(&self) -> Option<String> {
        self.project.as_ref().and_then(|p| p.selected.clone()).or_else(|| self.active_item())
    }
    pub(super) fn show_word_count(&mut self) {
        let Some(state) = &self.project else {
            self.status = "Open a project first.".into();
            return;
        };
        let tab = self.active_id().and_then(|id| self.tabs.get(&id));
        let chapter = tab.map(|tab| {
            let doc = &tab.document;
            let range = doc.selection();
            let text = if range.is_empty() { doc.text.slice(..).to_string() } else { doc.text.slice(range.clone()).to_string() };
            (notes::word_count(&notes::to_neutral(&text, &self.marker()).0), range.is_empty())
        });
        let total = state.core.manuscript_words();
        self.status = match chapter {
            Some((n, true)) => format!("This chapter: {n} words · Manuscript: {total} words"),
            Some((n, false)) => format!("Selection: {n} words · Manuscript: {total} words"),
            None => format!("Manuscript: {total} words"),
        };
    }
    pub(super) fn close_form(&mut self) {
        self.form = None;
        self.redraw();
    }
    fn open_form(&mut self, form: Form) {
        self.overlay = None;
        self.popup = None;
        self.form = Some(form);
        self.redraw();
    }

    // ---- builders ----------------------------------------------------------------------------

    pub(super) fn open_settings(&mut self) {
        let s = self.settings.clone();
        let accent = super::gui::ACCENTS.iter().position(|a| *a == s.accent_color()).unwrap_or(0);
        let fields = vec![
            Field::heading("Appearance"),
            Field::choice("theme", "Theme", labels(&["Light", "Dark", "System"]), ["light", "dark", "system"].iter().position(|t| *t == s.theme).unwrap_or(1)),
            Field::choice("accent", "Accent colour", labels(&["Amber", "Orange", "Red", "Pink", "Violet", "Blue", "Teal", "Green"]), accent),
            Field::button("font:interface_font", "Interface font", &s.interface_font),
            Field::heading("Writing"),
            Field::button("font:editor_font", "Editor font", &s.editor_font),
            Field::number("font_size", "Font size", s.font_size, 10., 48., 1., 0, "pt"),
            Field::number("line_height", "Line height", s.line_height, 1., 3., 0.1, 1, "×"),
            Field::number("paragraph_gap", "Space between paragraphs", s.paragraph_gap, 0., 2., 0.1, 1, "lines"),
            Field::number("max_width", "Column width", s.max_width.unwrap_or(0.), 0., 3000., 40., 0, "px (0 is full width)"),
            Field::toggle("typewriter", "Typewriter scrolling", s.typewriter).hint("Keep the line you are typing in the middle"),
            Field::toggle("show_markers", "Show formatting marks", s.show_markers),
            Field::toggle("spellcheck", "Underline possible misspellings", s.spellcheck).hint("Uses the project language"),
            Field::heading("Notes"),
            Field::text("note_marker", "Note marker", &s.note_marker).hint("Two or three punctuation characters, such as $$ or %%"),
            Field::text("note_color", "Note colour", &s.note_color).hint("#rrggbb, or leave empty for the accent colour"),
            Field::toggle("note_italic", "Italic notes", s.note_italic),
            Field::toggle("hide_notes", "Hide notes", s.hide_notes),
            Field::toggle("edit_scene_names", "Edit scene names in the text", s.edit_scene_names).hint("Off: rename scenes by double-clicking them in the Scenes panel"),
            Field::toggle("preview_notes", "Show notes in the preview", s.preview_notes),
            Field::heading("Saving"),
            Field::toggle("autosave", "Save chapters while I write", s.autosave),
            Field::number("repack_minutes", "Write the .jw file every", s.repack_minutes as f32, 1., 60., 1., 0, "min"),
            Field::button("keymap", "Keyboard shortcuts", "Customize…"),
        ];
        let form = Form::new(FormKind::Settings, "Settings", fields).no_submit().width(620.);
        self.open_form(form);
    }

    /// Applies the settings form as it stands.
    fn apply_settings_form(&mut self) {
        let Some(form) = self.form.as_ref() else { return };
        let mut s = self.settings.clone();
        s.theme = ["light", "dark", "system"][form.choice("theme").min(2)].into();
        s.accent = format!("#{:06x}", super::gui::ACCENTS[form.choice("accent").min(7)]);
        s.font_size = form.number("font_size");
        s.line_height = form.number("line_height");
        s.paragraph_gap = form.number("paragraph_gap");
        let width = form.number("max_width");
        s.max_width = (width >= 240.).then_some(width);
        s.typewriter = form.on("typewriter");
        s.show_markers = form.on("show_markers");
        s.spellcheck = form.on("spellcheck");
        s.note_marker = form.text("note_marker");
        s.note_color = form.text("note_color");
        s.note_italic = form.on("note_italic");
        s.hide_notes = form.on("hide_notes");
        s.edit_scene_names = form.on("edit_scene_names");
        s.preview_notes = form.on("preview_notes");
        s.autosave = form.on("autosave");
        s.repack_minutes = form.number("repack_minutes") as u32;
        let old = self.marker();
        match Settings::parse(&s.serialize()) {
            Ok(parsed) => {
                let new = parsed.marker();
                let hide_changed = parsed.hide_notes != self.settings.hide_notes;
                if let Some(f) = &mut self.form {
                    f.note.clear();
                }
                self.adopt(parsed);
                self.save_settings();
                if new != old {
                    self.change_marker(&old, &new);
                } else if hide_changed {
                    for id in self.tabs.keys().copied().collect::<Vec<_>>() {
                        self.scan(id, true);
                    }
                }
            }
            Err(error) => {
                if let Some(f) = &mut self.form {
                    f.note = error;
                }
            }
        }
    }
    /// Re-reads open chapters so they show notes with the new marker. What is stored is unchanged.
    fn change_marker(&mut self, _old: &notes::Marker, new: &notes::Marker) {
        self.flush_tabs();
        if let Some(state) = &self.project {
            if let Ok(mut m) = state.marker.write() {
                *m = new.clone();
            }
        }
        self.reload_tabs();
    }

    pub(super) fn add_comment_form(&mut self) {
        let (Some(item), Some(id)) = (self.active_item(), self.active_id()) else {
            self.status = "Open a chapter first.".into();
            return;
        };
        let Some(tab) = self.tabs.get(&id) else { return };
        let range = tab.document.selection();
        if range.is_empty() {
            self.status = "Select the text you want to comment on first.".into();
            return;
        }
        let quote: String = tab.document.text.slice(range.clone()).to_string();
        let shown: String = quote.chars().take(160).collect();
        let tags: Vec<String> = COMMENT_TAGS.iter().map(|t| if t.is_empty() { "No tag".to_owned() } else { (*t).to_owned() }).collect();
        let fields = vec![
            Field::info(&format!("On: {}{}", shown.replace('\n', " "), if quote.chars().count() > 160 { "…" } else { "" })),
            Field::area("body", "Comment", "", 4),
            Field::choice("tag", "Tag", tags, 0),
            Field::text("custom", "Custom tag", "").when("tag", &[5]),
        ];
        self.open_form(Form::new(FormKind::Comment { item, range }, "Add review comment", fields).submit("Add comment").width(560.));
    }
    pub(super) fn edit_comment_form(&mut self, id: &str) {
        let Some(c) = self.project.as_ref().and_then(|p| p.comments.get(id)).cloned() else { return };
        let known = COMMENT_TAGS.iter().position(|t| *t == c.tag);
        let tags: Vec<String> = COMMENT_TAGS.iter().map(|t| if t.is_empty() { "No tag".to_owned() } else { (*t).to_owned() }).collect();
        let fields = vec![
            Field::info(&format!("On: {}", c.quote.chars().take(160).collect::<String>().replace('\n', " "))),
            Field::area("body", "Comment", &c.body, 4),
            Field::choice("tag", "Tag", tags, known.unwrap_or(5)),
            Field::text("custom", "Custom tag", if known.is_none() { &c.tag } else { "" }).when("tag", &[5]),
        ];
        self.open_form(Form::new(FormKind::EditComment(id.into()), "Edit comment", fields).submit("Save").width(560.));
    }
    pub(super) fn reply_form(&mut self, id: &str) {
        let fields = vec![Field::area("body", "Reply", "", 4)];
        self.open_form(Form::new(FormKind::Reply(id.into()), "Reply to comment", fields).submit("Reply").width(520.));
    }
    pub(super) fn delete_comment_form(&mut self, id: &str) {
        let fields = vec![Field::info("The comment and its replies are removed from the project. This cannot be undone, but earlier snapshots keep their copy.")];
        self.open_form(Form::new(FormKind::DeleteComment(id.into()), "Delete this comment?", fields).submit("Delete").danger().width(480.));
    }
    pub(super) fn locked_form(&mut self, path: PathBuf, conflict: jw::lock::Conflict) {
        let fields = vec![
            Field::info(&conflict.message(self.now())),
            Field::info("Open it here anyway only when you are sure the other copy is closed. Two copies editing the same project can lose work."),
        ];
        self.open_form(Form::new(FormKind::Locked(path), "This project is in use", fields).submit("Open anyway").width(560.));
    }
    pub(super) fn changed_outside_form(&mut self) {
        let fields = vec![Field::info(
            "The .jw file was changed by something other than this window since it was opened, for example by a sync service or another computer. Overwriting replaces those changes with the version open here. “Save a copy” keeps both.",
        )];
        self.open_form(Form::new(FormKind::Overwrite, "The project file changed", fields).submit("Overwrite").danger().button("Save a copy…", "copy").width(580.));
    }
    pub(super) fn snapshot_form(&mut self) {
        let Some(state) = &self.project else {
            self.status = "Open a project first.".into();
            return;
        };
        let line = state.history.current_line().to_owned();
        let fields = vec![
            Field::info(&format!("A snapshot keeps every chapter, note, comment and the book details as they are now, on the “{line}” line.")),
            Field::text("message", "Message", "").hint("What is this version? For example: first draft done"),
        ];
        self.open_form(Form::new(FormKind::Snapshot, "Take a snapshot", fields).submit("Take snapshot").width(560.));
    }
    pub(super) fn new_line_form(&mut self) {
        let fields = vec![
            Field::info("A line is a named branch of the story, such as “Alternate ending”. The project continues on the new line from the current text; lines are never merged."),
            Field::text("name", "Name", ""),
        ];
        self.open_form(Form::new(FormKind::NewLine, "New line", fields).submit("Create line").width(540.));
    }
    pub(super) fn new_note_form(&mut self) {
        let Some(state) = &self.project else {
            self.status = "Open or create a project first.".into();
            return;
        };
        let hint = Self::next_note_title(&state.core.manifest.tree);
        let fields = vec![Field::text("name", "Name", "").hint(&hint)];
        self.open_form(Form::new(FormKind::NewNote, "New note", fields).intro("Leave the name empty to use the suggested one.").submit("Create").width(460.));
    }
    pub(super) fn rename_line_form(&mut self, line: &str) {
        let fields = vec![Field::text("name", "New name", line)];
        self.open_form(Form::new(FormKind::RenameLine(line.into()), "Rename line", fields).submit("Rename").width(460.));
    }
    pub(super) fn book_form(&mut self) {
        let Some(state) = &self.project else {
            self.status = "Open a project first.".into();
            return;
        };
        let b = state.core.book.clone();
        let fields = vec![
            Field::heading("Title"),
            Field::text("title", "Title", &b.title).hint(state.core.title()),
            Field::text("subtitle", "Subtitle", &b.subtitle),
            Field::text("short_title", "Short title", &b.short_title).hint("Used in running heads"),
            Field::text("series", "Series", &b.series),
            Field::heading("Author"),
            Field::text("legal_name", "Legal name", &b.legal_name),
            Field::text("pen_name", "Pen name", &b.pen_name).hint("Printed on the title page when set"),
            Field::text("email", "Email", &b.email),
            Field::text("phone", "Phone", &b.phone),
            Field::area("address", "Postal address", &b.address, 3),
            Field::text("website", "Website", &b.website),
            Field::heading("Publication"),
            Field::text("publisher", "Publisher", &b.publisher),
            Field::text("isbn", "ISBN", &b.isbn),
            Field::text("copyright_line", "Copyright line", &b.copyright_line).hint("Copyright © 2026 Name"),
            Field::area("description", "Description (the blurb)", &b.description, 4),
            Field::text("subjects", "Subjects or keywords", &b.subjects).hint("Separated by commas"),
            Field::button("cover", "Cover image", if b.cover.is_empty() { "Choose a cover…" } else { "Change cover…" }),
            Field::info(&if b.cover.is_empty() { "No cover yet. JPEG or PNG, ideally at least 1600 pixels tall.".to_owned() } else { format!("Cover: {}", b.cover) }),
            Field::heading("Pages"),
            Field::area("dedication", "Dedication", &b.dedication, 3),
            Field::area("epigraph", "Epigraph", &b.epigraph, 3),
            Field::area("acknowledgments", "Acknowledgments", &b.acknowledgments, 5),
            Field::area("about_author", "About the author", &b.about_author, 5),
            Field::area("copyright_page", "Copyright page text", &b.copyright_page, 5).hint("Leave empty to build one from the fields above"),
        ];
        self.open_form(Form::new(FormKind::Book, "Book details", fields).submit("Save").width(640.));
    }
    pub(super) fn project_form(&mut self) {
        let Some(state) = &self.project else {
            self.status = "Open a project first.".into();
            return;
        };
        let s = state.core.manifest.settings.clone();
        let presets = state.presets.all();
        let default = presets.iter().position(|p| p.id == s.default_preset).unwrap_or(0);
        let fields = vec![
            Field::heading("Language"),
            Field::choice("language", "Language of the manuscript", labels(&["English", "Spanish"]), if s.language == Language::Spanish { 1 } else { 0 }),
            Field::info("The language sets the quotation marks, hyphenation and spelling suggestions."),
            Field::heading("Typing"),
            Field::toggle("smart_quotes", "Smart quotation marks", s.typography.smart_quotes),
            Field::toggle("em_dash", "Two hyphens become an em dash", s.typography.em_dash),
            Field::toggle("ellipsis", "Three dots become an ellipsis", s.typography.ellipsis),
            Field::toggle("dialogue_dash", "Spanish dialogue dash", s.typography.dialogue_dash).when("language", &[1]),
            Field::text("scene_break", "Scene-break symbol in the editor", &s.scene_break).hint("Shown in place of a line like #  or ***"),
            Field::heading("Goals"),
            Field::number("project_words", "Manuscript word goal", s.goals.project_words as f32, 0., 2_000_000., 1000., 0, "words (0 for none)"),
            Field::number("session_words", "Words per session", s.goals.session_words as f32, 0., 100_000., 100., 0, "words (0 for none)"),
            Field::heading("Snapshots"),
            Field::number("auto_minutes", "Automatic snapshot every", s.snapshots.auto_minutes as f32, 0., 1440., 5., 0, "min (0 for never)"),
            Field::toggle("on_close", "Snapshot when the project closes", s.snapshots.on_close),
            Field::toggle("thin", "Thin out old automatic snapshots", s.snapshots.thin).hint("Manual snapshots are always kept"),
            Field::number("keep_all_days", "Keep every automatic snapshot for", s.snapshots.keep_all_days as f32, 0., 365., 1., 0, "days"),
            Field::number("keep_daily_days", "Then one per day for", s.snapshots.keep_daily_days as f32, 0., 3650., 5., 0, "days"),
            Field::heading("Export"),
            Field::choice("default_preset", "Start the export dialog from", presets.iter().map(|p| p.name.clone()).collect(), default),
        ];
        self.open_form(Form::new(FormKind::Project, "Project settings", fields).submit("Save").width(620.));
    }

    pub(super) fn export_form(&mut self) {
        let Some(state) = &self.project else {
            self.status = "Open a project first.".into();
            return;
        };
        let presets = state.presets.all();
        let wanted = if state.core.manifest.settings.default_preset.is_empty() { self.settings.default_preset.clone() } else { state.core.manifest.settings.default_preset.clone() };
        let index = presets.iter().position(|p| p.id == wanted).unwrap_or(0);
        let form = self.build_export_form(&presets[index], index, 0, 0, 0, None);
        self.open_form(form);
    }
    /// The export dialog for `preset`; the first fields keep what was chosen in the previous version.
    fn build_export_form(&self, preset: &Preset, preset_index: usize, source: usize, snapshot: usize, scope: usize, old: Option<&Form>) -> Form {
        let state = self.project.as_ref().expect("project is open");
        let presets = state.presets.all();
        let snapshots: Vec<(String, String)> = state
            .history
            .on_line(state.history.current_line())
            .map(|m| (m.id.clone(), format!("{} — {}", jw::time::local(m.time), if m.message.is_empty() { "(no message)" } else { &m.message })))
            .collect();
        let mut snapshots = snapshots;
        snapshots.reverse();
        let mut fields = vec![
            Field::choice("preset", "Preset", presets.iter().map(|p| if p.builtin { p.name.clone() } else { format!("{} (mine)", p.name) }).collect(), preset_index),
            Field::choice("format", "Format", Format::ALL.iter().map(|f| f.name().to_owned()).collect(), position(&Format::ALL, preset.format)),
            Field::heading("What to export"),
            Field::choice("source", "Text from", labels(&["The manuscript as it is now", "A snapshot"]), source),
            Field::choice(
                "snapshot",
                "Snapshot",
                if snapshots.is_empty() { vec!["No snapshots yet".into()] } else { snapshots.iter().map(|s| s.1.clone()).collect() },
                snapshot,
            )
            .when("source", &[1]),
            Field::choice("scope", "Chapters", labels(&["Whole manuscript", "Current chapter only", "Chosen chapters"]), scope),
        ];
        for (node, part) in state.core.manifest.tree.leaves(Section::Manuscript) {
            let label = match part {
                Some(part) => format!("{} · {}", part.title(), node.title()),
                None => node.title().to_owned(),
            };
            let on = old.map_or(true, |f| f.get(&format!("ch:{}", node.id())).is_none_or(|f| f.on));
            fields.push(Field::toggle(&format!("ch:{}", node.id()), &label, on).when("scope", &[2]));
        }
        fields.extend(preset_fields(preset));
        fields.push(Field::heading("Save these settings"));
        fields.push(Field::text("preset_name", "Preset name", if preset.builtin { "" } else { &preset.name }).hint("Name a new preset, or reuse a name to replace it"));
        fields.push(Field::button("save_preset", "", "Save as preset"));
        if !preset.builtin {
            fields.push(Field::button("delete_preset", "", "Delete this preset"));
        }
        let mut form = Form::new(FormKind::Export, "Export", fields)
            .submit("Export…")
            .button("Preview", "preview")
            .width(700.);
        form.data = vec![preset.id.clone()];
        if let Some(old) = old {
            form.scroll = 0;
            form.note = old.note.clone();
        }
        form
    }
    /// Collects what the export dialog asks for. Open chapters are saved first.
    fn export_request(&mut self) -> Result<ExportRequest, String> {
        self.flush_tabs();
        let form = self.form.as_ref().ok_or("No export dialog")?;
        let state = self.project.as_ref().ok_or("Open a project first")?;
        let presets = state.presets.all();
        let base = presets.get(form.choice("preset")).cloned().unwrap_or_default();
        let preset = read_preset(form, &base);
        let now = self.now();
        let source = if form.choice("source") == 1 {
            let snapshots: Vec<String> = {
                let mut ids: Vec<String> = state.history.on_line(state.history.current_line()).map(|m| m.id.clone()).collect();
                ids.reverse();
                ids
            };
            let id = snapshots.get(form.choice("snapshot")).ok_or("There are no snapshots to export from")?;
            let snap = state.history.state(&state.core.work, id).map_err(|e| e.to_string())?;
            Source::from_state(&state.core, &snap, now).map_err(|e| e.to_string())?
        } else {
            Source::from_project(&state.core, now).map_err(|e| e.to_string())?
        };
        let scope = match form.choice("scope") {
            0 => Scope::Whole,
            1 => {
                let item = self.active_item().ok_or("Open the chapter you want to export")?;
                if state.core.manifest.tree.locate(&item).is_none_or(|(s, _)| s != Section::Manuscript) {
                    return Err("The current tab is a note; only manuscript chapters can be exported".into());
                }
                Scope::Chapters(vec![item])
            }
            _ => {
                let chosen: Vec<String> = state
                    .core
                    .manifest
                    .tree
                    .leaves(Section::Manuscript)
                    .into_iter()
                    .filter(|(n, _)| form.on(&format!("ch:{}", n.id())))
                    .map(|(n, _)| n.id().to_owned())
                    .collect();
                if chosen.is_empty() {
                    return Err("Choose at least one chapter".into());
                }
                Scope::Chapters(chosen)
            }
        };
        Ok(ExportRequest { source: Arc::new(source), scope, preset })
    }

    pub(super) fn import_form(&mut self, path: PathBuf, raw: import::Raw) {
        let suggestion = split::suggest(&raw);
        let (by, pattern, level) = match &suggestion.by {
            split::By::Headings(n) => (0, String::new(), *n as usize),
            split::By::Pattern(p) => (1, p.clone(), 1),
            split::By::PageBreaks => (2, String::new(), 1),
            split::By::Whole => (3, String::new(), 1),
        };
        let headings = raw.paras.iter().filter(|p| matches!(p.kind, import::Kind::Heading(_))).count();
        let summary = format!(
            "{} — {} paragraphs, {} headings{}.",
            path.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned()),
            raw.paras.len(),
            headings,
            if raw.comments.is_empty() { String::new() } else { format!(", {} Word comments (they become review comments)", raw.comments.len()) }
        );
        let mut fields = vec![Field::info(&summary)];
        let open = self.project.is_some();
        fields.push(Field::choice("target", "Import into", if open { labels(&["A new project", "The open project (added after the last chapter)"]) } else { labels(&["A new project"]) }, 0));
        fields.push(Field::heading("How to cut it into chapters"));
        fields.push(Field::choice("by", "Start a new chapter at", labels(&["Each heading", "Each paragraph matching a pattern", "Each page break", "Nowhere: one chapter"]), by));
        fields.push(Field::number("level", "Heading level for chapters", level as f32, 1., 6., 1., 0, "").when("by", &[0]));
        fields.push(Field::text("pattern", "Pattern (regular expression)", &pattern).hint("For example ^(Chapter|CHAPTER) \\d+").when("by", &[1]));
        fields.push(Field::heading("Notes"));
        fields.push(Field::text("prefix", "Lines starting with this are notes", &suggestion.comment_prefix).hint("such as # or //"));
        let mut form = Form::new(FormKind::Import, "Import a manuscript", fields).submit("Import").width(620.);
        form.data = vec![path.to_string_lossy().into_owned()];
        self.import_raw = Some(Arc::new(raw));
        self.open_form(form);
    }

    // ---- keys and clicks ---------------------------------------------------------------------

    /// Keyboard input for the open form. Returns false for keys the form leaves to the window.
    pub(super) fn form_key(&mut self, key: &Key, text: Option<&str>) -> bool {
        let primary = platform::primary(self.modifiers);
        let Some(form) = &mut self.form else { return false };
        if let Some((field, _)) = form.dropdown {
            match key {
                Key::Named(NamedKey::Escape) => form.dropdown = None,
                Key::Named(NamedKey::ArrowDown) => form.cycle(field, 1),
                Key::Named(NamedKey::ArrowUp) => form.cycle(field, -1),
                Key::Named(NamedKey::Enter) => form.dropdown = None,
                _ => {}
            }
            let key = form.fields.get(field).map(|f| f.key.clone()).unwrap_or_default();
            self.form_changed(&key);
            self.redraw();
            return true;
        }
        let typed = form.fields.get(form.focus).is_some_and(Field::typed);
        let area = form.fields.get(form.focus).is_some_and(|f| matches!(f.control, Control::Area(_)));
        let mut changed = false;
        match key {
            Key::Named(NamedKey::Escape) => {
                self.close_form();
                return true;
            }
            Key::Named(NamedKey::Tab) => {
                form.keyboard = true;
                form.move_focus(!self.modifiers.shift_key());
            }
            Key::Named(NamedKey::Enter) => {
                if primary && form.submit.is_some() {
                    self.submit_form();
                    return true;
                } else if area {
                    form.type_text("\n");
                    changed = true;
                } else if let Some(Control::Button(_)) = form.fields.get(form.focus).map(|f| f.control.clone()) {
                    let key = form.fields[form.focus].key.clone();
                    self.form_button(&key);
                    return true;
                } else if form.submit.is_some() {
                    self.submit_form();
                    return true;
                } else if let Some(Control::Choice(_)) = form.fields.get(form.focus).map(|f| f.control.clone()) {
                    form.dropdown = Some((form.focus, 0));
                }
            }
            Key::Named(NamedKey::ArrowDown | NamedKey::ArrowUp) if !area => {
                form.keyboard = true;
                form.move_focus(matches!(key, Key::Named(NamedKey::ArrowDown)));
            }
            Key::Named(NamedKey::PageDown) => form.scroll_by(6),
            Key::Named(NamedKey::PageUp) => form.scroll_by(-6),
            Key::Named(NamedKey::Backspace) if typed => {
                form.backspace(false);
                changed = true;
            }
            Key::Named(NamedKey::Delete) if typed => {
                form.backspace(true);
                changed = true;
            }
            Key::Named(NamedKey::ArrowLeft) if typed => form.caret_move(-1, None),
            Key::Named(NamedKey::ArrowRight) if typed => form.caret_move(1, None),
            Key::Named(NamedKey::Home) if typed => form.caret_move(0, Some(true)),
            Key::Named(NamedKey::End) if typed => form.caret_move(0, Some(false)),
            Key::Named(NamedKey::ArrowLeft | NamedKey::ArrowRight) => {
                let direction = if matches!(key, Key::Named(NamedKey::ArrowLeft)) { -1 } else { 1 };
                let at = form.focus;
                match form.fields.get(at).map(|f| f.control.clone()) {
                    Some(Control::Choice(_)) => {
                        form.cycle(at, direction);
                        changed = true;
                    }
                    Some(Control::Toggle) => {
                        form.fields[at].on = direction > 0;
                        changed = true;
                    }
                    _ => {}
                }
            }
            Key::Character(_) | Key::Named(NamedKey::Space) if !typed && (matches!(key, Key::Named(NamedKey::Space)) || matches!(key, Key::Character(c) if c.as_str() == " ")) => {
                let at = form.focus;
                match form.fields.get(at).map(|f| f.control.clone()) {
                    Some(Control::Toggle) => {
                        form.fields[at].on = !form.fields[at].on;
                        changed = true;
                    }
                    Some(Control::Choice(_)) => form.dropdown = Some((at, 0)),
                    Some(Control::Button(_)) => {
                        let key = form.fields[at].key.clone();
                        self.form_button(&key);
                        return true;
                    }
                    _ => {}
                }
            }
            Key::Character(c) if primary && typed && c.eq_ignore_ascii_case("v") => {
                match self.clipboard.read() {
                    Ok(text) => {
                        if let Some(form) = &mut self.form {
                            form.type_text(&text.replace("\r\n", "\n"));
                        }
                        changed = true;
                    }
                    Err(error) => self.status = error,
                }
            }
            Key::Character(_) | Key::Named(NamedKey::Space) if typed && !primary && !self.modifiers.alt_key() && self.preedit.is_empty() => {
                let text = if matches!(key, Key::Named(NamedKey::Space)) { Some(text.unwrap_or(" ")) } else { text };
                if let Some(text) = text.filter(|s| !s.chars().any(char::is_control)) {
                    form.type_text(text);
                    changed = true;
                }
            }
            _ => {}
        }
        if changed {
            if let Some(key) = self.form.as_ref().and_then(|f| f.fields.get(f.focus)).map(|f| f.key.clone()) {
                self.form_changed(&key);
            }
        }
        self.redraw();
        true
    }
    /// Text from the input method.
    pub(super) fn form_text(&mut self, text: &str) {
        if let Some(form) = &mut self.form {
            form.type_text(text);
            let key = form.fields.get(form.focus).map(|f| f.key.clone()).unwrap_or_default();
            self.form_changed(&key);
        }
    }
    pub(super) fn form_act(&mut self, act: &Act) {
        let at = self.probe_char();
        let Some(form) = &mut self.form else { return };
        form.keyboard = false;
        let mut changed: Option<String> = None;
        match act {
            Act::FormFocus(i) => {
                form.set_focus(*i);
                // A click in a text box puts the caret where it landed.
                let len = form.fields.get(*i).map_or(0, |f| f.text.chars().count());
                form.caret = at.filter(|at| *at < len);
            }
            Act::FormToggle(i) => {
                form.set_focus(*i);
                if let Some(f) = form.fields.get_mut(*i) {
                    f.on = !f.on;
                    changed = Some(f.key.clone());
                }
            }
            Act::FormSegment(i, option) => {
                form.set_focus(*i);
                if let Some(f) = form.fields.get_mut(*i) {
                    f.index = *option;
                    changed = Some(f.key.clone());
                }
            }
            Act::FormDrop(i) => {
                let open = form.dropdown.is_some_and(|(f, _)| f == *i);
                form.set_focus(*i);
                if !open {
                    let at = form.fields[*i].index;
                    form.dropdown = Some((*i, at.saturating_sub(3)));
                }
            }
            Act::FormPick(i, option) => {
                if let Some(f) = form.fields.get_mut(*i) {
                    f.index = *option;
                    changed = Some(f.key.clone());
                }
                form.dropdown = None;
            }
            Act::FormStep(i, direction) => {
                form.set_focus(*i);
                form.step(*i, *direction);
                changed = form.fields.get(*i).map(|f| f.key.clone());
            }
            Act::FormButton(i) => {
                let key = form.fields.get(*i).map(|f| f.key.clone()).unwrap_or_default();
                form.set_focus(*i);
                self.form_button(&key);
                return;
            }
            Act::FormExtra(id) => {
                self.form_extra(id);
                return;
            }
            Act::FormSubmit => {
                self.submit_form();
                return;
            }
            Act::FormCloseDrop => form.dropdown = None,
            _ => {}
        }
        if let Some(key) = changed {
            self.form_changed(&key);
        }
    }
    pub(super) fn form_scroll(&mut self, rows: i32) {
        if let Some(form) = &mut self.form {
            if let Some((field, first)) = form.dropdown {
                let n = match form.fields.get(field).map(|f| &f.control) {
                    Some(Control::Choice(o)) => o.len(),
                    _ => 0,
                };
                form.dropdown = Some((field, (first as i32 + rows).clamp(0, n.saturating_sub(1) as i32) as usize));
            } else {
                form.scroll_by(rows);
            }
        }
    }

    /// A field changed.
    fn form_changed(&mut self, key: &str) {
        let Some(form) = &mut self.form else { return };
        match form.kind {
            FormKind::Settings => self.apply_settings_form(),
            FormKind::Export => match key {
                "preset" => {
                    let Some(state) = &self.project else { return };
                    let presets = state.presets.all();
                    let index = form.choice("preset");
                    let (source, snapshot, scope) = (form.choice("source"), form.choice("snapshot"), form.choice("scope"));
                    let focus = form.focus;
                    if let Some(preset) = presets.get(index).cloned() {
                        let old = self.form.take();
                        let mut new = self.build_export_form(&preset, index, source, snapshot, scope, old.as_ref());
                        new.focus = focus.min(new.fields.len().saturating_sub(1));
                        self.form = Some(new);
                    }
                }
                "trim" => {
                    let trim = form.choice("trim");
                    if let Some((_, w, h)) = TRIMS.get(trim) {
                        form.set_number("page_width", *w);
                        form.set_number("page_height", *h);
                    }
                }
                _ => {}
            },
            _ => {}
        }
    }
    fn form_button(&mut self, key: &str) {
        match key {
            "keymap" => {
                self.form = None;
                self.show(Kind::Keymap);
            }
            "cover" => self.choose(Choose::Cover),
            "save_preset" => self.save_export_preset(),
            "delete_preset" => self.delete_export_preset(),
            font if font.starts_with("font:") => {
                let key = font["font:".len()..].to_owned();
                self.form = None;
                self.open_font(&key);
            }
            _ => {}
        }
    }
    pub(super) fn form_extra(&mut self, id: &str) {
        if id == "copy" {
            self.close_form();
            self.choose(Choose::SaveProjectAs);
        }
        if id == "preview" {
            match self.export_request() {
                Ok(request) => self.open_preview_for(request),
                Err(error) => {
                    if let Some(form) = &mut self.form {
                        form.note = error;
                    }
                }
            }
        }
    }
    fn save_export_preset(&mut self) {
        let Some(form) = &self.form else { return };
        let name = form.text("preset_name");
        if name.is_empty() {
            self.form_note("Give the preset a name first.");
            return;
        }
        let Some(state) = &mut self.project else { return };
        let presets = state.presets.all();
        let base = presets.get(form.choice("preset")).cloned().unwrap_or_default();
        let mut preset = read_preset(form, &base);
        preset.name = name.clone();
        // Reusing the name of one of my presets replaces it.
        preset.id = state.presets.custom.iter().find(|p| p.name == name).map(|p| p.id.clone()).unwrap_or_default();
        let id = state.presets.save(preset);
        self.save_presets();
        let Some(state) = &self.project else { return };
        let all = state.presets.all();
        let index = all.iter().position(|p| p.id == id).unwrap_or(0);
        let old = self.form.take();
        let (source, snapshot, scope) = old.as_ref().map_or((0, 0, 0), |f| (f.choice("source"), f.choice("snapshot"), f.choice("scope")));
        let mut new = self.build_export_form(&all[index], index, source, snapshot, scope, old.as_ref());
        new.note = format!("Saved the preset “{name}”.");
        self.form = Some(new);
    }
    fn delete_export_preset(&mut self) {
        let Some(form) = &self.form else { return };
        let Some(state) = &mut self.project else { return };
        let presets = state.presets.all();
        let Some(preset) = presets.get(form.choice("preset")).cloned().filter(|p| !p.builtin) else { return };
        state.presets.custom.retain(|p| p.id != preset.id);
        self.save_presets();
        let old = self.form.take();
        let (source, snapshot, scope) = old.as_ref().map_or((0, 0, 0), |f| (f.choice("source"), f.choice("snapshot"), f.choice("scope")));
        let Some(state) = &self.project else { return };
        let all = state.presets.all();
        let mut new = self.build_export_form(&all[0], 0, source, snapshot, scope, old.as_ref());
        new.note = format!("Deleted the preset “{}”.", preset.name);
        self.form = Some(new);
    }
    fn form_note(&mut self, text: &str) {
        if let Some(form) = &mut self.form {
            form.note = text.into();
        }
    }

    // ---- submitting --------------------------------------------------------------------------

    pub(super) fn submit_form(&mut self) {
        let Some(form) = self.form.as_ref() else { return };
        let kind = form.kind.clone();
        match kind {
            FormKind::Settings => self.close_form(),
            FormKind::Comment { item, range } => {
                let body = form.raw("body");
                let tag = if form.choice("tag") == 5 { form.text("custom") } else { COMMENT_TAGS[form.choice("tag").min(5)].to_owned() };
                match self.create_comment(&item, range, &body, &tag) {
                    Ok(()) => self.close_form(),
                    Err(error) => self.form_note(&error),
                }
            }
            FormKind::EditComment(id) => {
                let body = form.raw("body");
                let tag = if form.choice("tag") == 5 { form.text("custom") } else { COMMENT_TAGS[form.choice("tag").min(5)].to_owned() };
                if body.trim().is_empty() {
                    self.form_note("The comment cannot be empty.");
                    return;
                }
                if let Some(state) = &mut self.project {
                    state.comments.edit_body(&id, &body);
                    state.comments.set_tag(&id, &tag);
                }
                self.save_comments();
                self.close_form();
            }
            FormKind::Reply(id) => {
                let body = form.raw("body");
                let now = self.now();
                let result = self.project.as_mut().map_or(Err("No project".to_owned()), |s| s.comments.reply(&id, &body, now));
                match result {
                    Ok(()) => {
                        self.save_comments();
                        self.close_form();
                    }
                    Err(error) => self.form_note(&error),
                }
            }
            FormKind::DeleteComment(id) => {
                if let Some(state) = &mut self.project {
                    state.comments.delete(&id);
                    if state.active_comment.as_deref() == Some(id.as_str()) {
                        state.active_comment = None;
                    }
                }
                self.save_comments();
                for tab in self.tabs.keys().copied().collect::<Vec<_>>() {
                    if let Some(t) = self.tabs.get_mut(&tab) {
                        t.marks.retain(|m| m.id != id);
                    }
                    self.sync_view_marks(tab);
                }
                self.close_form();
            }
            FormKind::Snapshot => {
                let message = form.text("message");
                self.take_snapshot(&message);
                self.close_form();
            }
            FormKind::NewNote => {
                let name = form.text("name");
                self.close_form();
                self.new_node("page", &name);
            }
            FormKind::NewLine => {
                let name = form.text("name");
                self.flush_tabs();
                let now = self.now();
                let result = self.project.as_mut().map_or(Err("No project".into()), |s| s.history.new_line(&s.core, &name, now));
                match result {
                    Ok(()) => {
                        self.status = format!("Now writing on the “{name}” line.");
                        self.close_form();
                        self.refresh_history();
                    }
                    Err(error) => self.form_note(&error),
                }
            }
            FormKind::RenameLine(old) => {
                let name = form.text("name");
                let result = self.project.as_mut().map_or(Err("No project".into()), |s| s.history.rename_line(&s.core.work, &old, &name));
                match result {
                    Ok(()) => {
                        self.close_form();
                        self.refresh_history();
                    }
                    Err(error) => self.form_note(&error),
                }
            }
            FormKind::Book => self.submit_book(),
            FormKind::Project => self.submit_project(),
            FormKind::Goals => self.close_form(),
            FormKind::Locked(path) => {
                self.close_form();
                self.open_project(path, true);
            }
            FormKind::Overwrite => {
                self.close_form();
                if let Some(state) = &mut self.project {
                    state.blocked = false;
                }
                self.pack_project(true);
            }
            FormKind::Import => self.submit_import(),
            FormKind::Export => self.submit_export(),
        }
        self.redraw();
    }

    fn create_comment(&mut self, item: &str, range: Range<usize>, body: &str, tag: &str) -> Result<(), String> {
        let marker = self.marker();
        let now = self.now();
        let tab_id = self.tabs.iter().find(|(id, _)| self.item_of_tab(**id).as_deref() == Some(item)).map(|(id, _)| *id).ok_or("The chapter is no longer open")?;
        let text = self.tabs.get(&tab_id).map(|t| t.document.text.to_string()).ok_or("The chapter is no longer open")?;
        let (neutral, forward) = notes::to_neutral(&text, &marker);
        let stored = forward.map(range.start)..forward.map(range.end);
        let state = self.project.as_mut().ok_or("No project")?;
        let id = state.comments.add(item, &neutral, stored, body, tag, now)?;
        state.active_comment = Some(id.clone());
        let _ = state.core.work.write(jw::manifest::COMMENTS, state.comments.to_json().as_bytes());
        if let Some(tab) = self.tabs.get_mut(&tab_id) {
            tab.marks.push(project::TabMark { id, start: range.start, end: range.end, orphaned: false });
        }
        self.sync_view_marks(tab_id);
        if let Some(state) = &mut self.project {
            state.panel = Panel::Comments;
        }
        self.show_outline = true;
        Ok(())
    }
    pub(super) fn take_snapshot(&mut self, message: &str) {
        self.flush_tabs();
        let now = self.now();
        let Some(state) = &mut self.project else { return };
        match state.history.capture(&state.core, history::Kind::Manual, message, now) {
            Ok(meta) => {
                state.changed = false;
                state.last_snapshot = Instant::now();
                self.status = format!("Snapshot taken: {}.", count_words(meta.words as u64));
                self.pack_project(false);
                self.refresh_history();
            }
            Err(error) => self.status = format!("The snapshot failed: {error}"),
        }
    }
    fn submit_book(&mut self) {
        let Some(form) = self.form.as_ref() else { return };
        let get = |k: &str| form.text(k);
        let mut b = self.project.as_ref().map(|s| s.core.book.clone()).unwrap_or_default();
        b.title = get("title");
        b.subtitle = get("subtitle");
        b.short_title = get("short_title");
        b.series = get("series");
        b.legal_name = get("legal_name");
        b.pen_name = get("pen_name");
        b.email = get("email");
        b.phone = get("phone");
        b.address = form.raw("address").trim().to_owned();
        b.website = get("website");
        b.publisher = get("publisher");
        b.isbn = get("isbn");
        b.copyright_line = get("copyright_line");
        b.description = form.raw("description").trim().to_owned();
        b.subjects = get("subjects");
        b.dedication = form.raw("dedication").trim().to_owned();
        b.epigraph = form.raw("epigraph").trim().to_owned();
        b.acknowledgments = form.raw("acknowledgments").trim().to_owned();
        b.about_author = form.raw("about_author").trim().to_owned();
        b.copyright_page = form.raw("copyright_page").trim().to_owned();
        let Some(state) = &mut self.project else { return };
        let changed = state.core.book != b;
        state.core.book = b;
        if changed {
            if let Err(error) = state.core.save_book() {
                self.status = format!("Could not save the book details: {error}");
            } else {
                state.changed = true;
            }
        }
        self.close_form();
    }
    fn submit_project(&mut self) {
        let Some(form) = self.form.as_ref() else { return };
        let presets = self.project.as_ref().map(|s| s.presets.all()).unwrap_or_default();
        let default = presets.get(form.choice("default_preset")).map(|p| p.id.clone()).unwrap_or_default();
        let (language, scene) = (if form.choice("language") == 1 { Language::Spanish } else { Language::English }, form.raw("scene_break").trim().to_owned());
        let values = (
            form.on("smart_quotes"),
            form.on("em_dash"),
            form.on("ellipsis"),
            form.on("dialogue_dash"),
            form.number("project_words") as u32,
            form.number("session_words") as u32,
            form.number("auto_minutes") as u32,
            form.on("on_close"),
            form.on("thin"),
            form.number("keep_all_days") as u32,
            form.number("keep_daily_days") as u32,
        );
        let Some(state) = &mut self.project else { return };
        let s = &mut state.core.manifest.settings;
        s.language = language;
        (s.typography.smart_quotes, s.typography.em_dash, s.typography.ellipsis, s.typography.dialogue_dash) = (values.0, values.1, values.2, values.3);
        if !scene.is_empty() {
            s.scene_break = scene;
        }
        s.goals.project_words = values.4;
        s.goals.session_words = values.5;
        s.snapshots.auto_minutes = values.6;
        s.snapshots.on_close = values.7;
        s.snapshots.thin = values.8;
        s.snapshots.keep_all_days = values.9;
        s.snapshots.keep_daily_days = values.10;
        s.default_preset = default;
        if let Err(error) = state.core.save_manifest() {
            self.status = format!("Could not save the project settings: {error}");
        }
        for tab in self.tabs.values_mut() {
            tab.view.clear_cache();
        }
        self.close_form();
    }
    fn submit_import(&mut self) {
        let (Some(form), Some(raw)) = (self.form.as_ref(), self.import_raw.clone()) else { return };
        let by = match form.choice("by") {
            0 => split::By::Headings(form.number("level") as u8),
            1 => split::By::Pattern(form.raw("pattern").trim().to_owned()),
            2 => split::By::PageBreaks,
            _ => split::By::Whole,
        };
        let options = split::Options { by, parts: form.choice("by") == 0, comment_prefix: form.raw("prefix").trim().to_owned() };
        let into_new = form.choice("target") == 0;
        self.import_into_new = into_new;
        let proxy = self.proxy.clone();
        self.form = None;
        self.status = "Importing…".into();
        std::thread::spawn(move || {
            let _ = proxy.send_event(UserEvent::Imported(split::split(&raw, &options).map(Box::new)));
        });
    }
    fn submit_export(&mut self) {
        match self.export_request() {
            Ok(request) => {
                let name = file_stem(request.source.book.display_title(&request.source.title));
                let extension = request.preset.format.extension();
                let id = request.preset.id.clone();
                self.pending_export = Some((request, format!("{name}.{extension}")));
                if let Some(state) = &mut self.project {
                    if state.core.manifest.settings.default_preset != id {
                        state.core.manifest.settings.default_preset = id;
                        let _ = state.core.save_manifest();
                    }
                }
                self.close_form();
                self.choose(Choose::Export);
            }
            Err(error) => self.form_note(&error),
        }
    }
    /// The export was chosen a place to go.
    pub(super) fn run_export(&mut self, path: PathBuf) {
        let Some((request, _)) = self.pending_export.take() else { return };
        let proxy = self.proxy.clone();
        self.status = format!("Exporting to {}…", path.display());
        std::thread::spawn(move || {
            let result = crate::export::export(&request.source, &request.scope, &request.preset).and_then(|(_, output)| {
                let path = if path.extension().is_none() { path.with_extension(output.extension) } else { path };
                let tmp = path.with_extension("tmp-export");
                std::fs::write(&tmp, &output.bytes).map_err(|e| e.to_string())?;
                std::fs::rename(&tmp, &path).map_err(|e| e.to_string())?;
                Ok(path)
            });
            let _ = proxy.send_event(UserEvent::Exported(result));
        });
    }
    pub(super) fn exported(&mut self, result: Result<PathBuf, String>) {
        match result {
            Ok(path) => {
                self.status = format!("Exported {}", path.display());
                platform::open_path(path.parent().unwrap_or(&path));
            }
            Err(error) => self.status = format!("The export failed: {error}"),
        }
    }
    /// A cover image was chosen.
    pub(super) fn cover_chosen(&mut self, path: PathBuf) {
        let Some(state) = &mut self.project else { return };
        match state.core.set_cover(&path) {
            Ok(()) => {
                state.changed = true;
                self.status = "Cover set.".into();
                if matches!(self.form.as_ref().map(|f| &f.kind), None) {
                    self.book_form();
                }
            }
            Err(error) => self.status = error,
        }
    }
}

/// A file-name-safe version of a title.
pub fn file_stem(title: &str) -> String {
    let stem: String = title.chars().map(|c| if c.is_alphanumeric() || matches!(c, ' ' | '-' | '_' | '.') { c } else { '_' }).collect::<String>().trim().to_owned();
    if stem.is_empty() { "manuscript".into() } else { stem }
}

// ---- painting ---------------------------------------------------------------------------------

impl App {
    pub(super) fn paint_form(&mut self, window: Rect, p: Palette) {
        let Some(mut form) = self.form.take() else { return };
        self.in_modal = true;
        self.hits.push((window, Hit::Do(Act::Block)));
        self.renderer.shade(window, 0x000000, if p.light { 70 } else { 120 });
        let scale = self.scale;
        let width_logical = form.width.min(window.width as f32 / scale - 24.);
        let top_margin = 28.;
        let field_width = width_logical - 48.;
        let content: f32 = form.shown_fields().into_iter().map(|i| form.fields[i].height(field_width)).sum();
        let intro = if form.intro.is_empty() { 0. } else { form.intro.lines().count() as f32 * 20. + 6. };
        let wanted = 60. + intro + content + 66. + 8.;
        let height_logical = wanted.min((window.height as f32 / scale - 2. * top_margin).clamp(260., 820.)).max(150.);
        let rect = Rect {
            x: (window.width as i32 - (width_logical * scale) as i32) / 2,
            y: self.s(top_margin).max(((window.height as f32 - height_logical * scale) / 2.) as i32),
            width: (width_logical * scale) as u32,
            height: (height_logical * scale) as u32,
        };
        self.soft_shadow(rect);
        self.renderer.framed(rect, self.su(9.), p.raised, super::gui::mix(p.border, p.text, 0.15));
        self.hits.push((rect, Hit::Do(Act::Block)));
        let pad = self.s(24.);
        let inner_width = rect.width - pad as u32 * 2;
        // Title
        let head = Rect { x: rect.x + pad, y: rect.y + self.s(14.), width: inner_width, height: self.su(32.) };
        self.put(&form.title.clone(), head, 16., p.text, Face::Bold, Align::Left);
        let close = Rect { x: rect.x + rect.width as i32 - self.s(48.), y: head.y + self.s(1.), width: self.su(30.), height: self.su(30.) };
        self.button("×", close, Hit::Do(Act::Dismiss), Look::Plain, p);
        let mut y = rect.y + self.s(54.);
        self.renderer.fill(Rect { x: rect.x + 1, y, width: rect.width - 2, height: 1 }, p.border);
        y += self.s(6.);
        if !form.intro.is_empty() {
            let lines = form.intro.lines().count() as i32;
            self.label(&form.intro, Rect { x: rect.x + pad, y, width: inner_width, height: self.su(20.) * lines as u32 }, 12.5 * scale, p.muted);
            y += self.s(20.) * lines + self.s(6.);
        }
        // Footer
        let footer_height = self.s(66.);
        let footer_top = rect.y + rect.height as i32 - footer_height;
        let list_height = (footer_top - y - self.s(4.)).max(self.s(60.));
        form.room = list_height as f32 / scale;
        let shown: Vec<usize> = (0..form.fields.len()).filter(|i| form.shown(*i)).collect();
        // Scrolling stops when the last field reaches the bottom of the list.
        let (mut last_start, mut tail) = (shown.len(), 0);
        while last_start > 0 && tail + self.s(form.fields[shown[last_start - 1]].height(field_width)) <= list_height {
            tail += self.s(form.fields[shown[last_start - 1]].height(field_width));
            last_start -= 1;
        }
        let start = shown.iter().position(|i| *i >= form.scroll).unwrap_or(0).min(last_start).min(shown.len().saturating_sub(1));
        form.scroll = shown.get(start).copied().unwrap_or(0);
        let label_width = (inner_width as f32 * 0.42) as i32;
        let control_x = rect.x + pad + label_width;
        let control_width = inner_width as i32 - label_width;
        let mut used = 0;
        let mut drawn = 0;
        let mut dropdown_anchor: Option<(usize, Rect)> = None;
        for &i in &shown[start..] {
            let height = self.s(form.fields[i].height(field_width));
            if used + height > list_height && drawn > 0 {
                break;
            }
            let row = Rect { x: rect.x + pad, y: y + used, width: inner_width, height: height as u32 };
            used += height;
            drawn += 1;
            let focused = form.focus == i;
            let marked = focused && form.keyboard;
            let field = form.fields[i].clone();
            match &field.control {
                Control::Heading => {
                    self.section(&field.label.to_uppercase(), row.x, row.y + self.s(14.), row.width, p);
                }
                Control::Info => {
                    self.label(&field.text, Rect { y: row.y + self.s(2.), ..row }, 12.5 * scale, p.muted);
                }
                _ => {
                    let label_rect = Rect { width: label_width as u32 - self.su(10.), ..row };
                    if marked {
                        self.renderer.fill_round(Rect { x: row.x - self.s(8.), y: row.y + self.s(2.), width: row.width + self.su(16.), height: row.height.saturating_sub(self.su(4.)) }, self.su(6.), super::gui::mix(p.raised, p.accent, 0.1));
                    }
                    if !field.label.is_empty() {
                        let text = self.fit(&field.label, label_rect.width as f32, 13.);
                        let area = if matches!(field.control, Control::Area(_)) { Rect { height: self.su(38.), ..label_rect } } else { label_rect };
                        self.put(&text, area, 13., p.text, Face::Normal, Align::Left);
                    }
                    let control = Rect { x: control_x, y: row.y + self.s(5.), width: control_width as u32, height: row.height.saturating_sub(self.su(10.)) };
                    let control = if field.label.is_empty() { Rect { x: row.x, width: row.width, ..control } } else { control };
                    match &field.control {
                        Control::Text => {
                            let caret = if focused { form.caret } else { None };
                            self.field(control, &field.text, &field.hint, caret, focused, false, Act::FormFocus(i), p);
                        }
                        Control::Area(_) => {
                            self.paint_area(control, &field, focused.then_some(form.caret), Act::FormFocus(i), p);
                        }
                        Control::Number { unit, .. } => {
                            let box_width = self.su(86.);
                            let stepper = Rect { x: control.x, width: box_width + self.su(68.), ..control };
                            let minus = Rect { width: self.su(30.), ..stepper };
                            let plus = Rect { x: stepper.x + stepper.width as i32 - self.s(30.), width: self.su(30.), ..stepper };
                            let middle = Rect { x: minus.x + minus.width as i32, width: stepper.width - minus.width * 2, ..stepper };
                            let caret = if focused { form.caret } else { None };
                            self.field(middle, &field.text, "", caret, focused, false, Act::FormFocus(i), p);
                            self.button("−", minus, Hit::Do(Act::FormStep(i, -1)), Look::Outline, p);
                            self.button("+", plus, Hit::Do(Act::FormStep(i, 1)), Look::Outline, p);
                            if !unit.is_empty() {
                                let at = Rect { x: stepper.x + stepper.width as i32 + self.s(8.), width: control.width.saturating_sub(stepper.width + self.su(8.)), ..control };
                                self.put(unit, at, 12., p.muted, Face::Normal, Align::Left);
                            }
                        }
                        Control::Choice(options) => {
                            let current = options.get(field.index).cloned().unwrap_or_default();
                            let short = options.len() <= 3 && options.iter().map(|o| o.chars().count()).sum::<usize>() < 34;
                            if short {
                                let cells: Vec<(&str, bool, Act)> = options.iter().enumerate().map(|(k, o)| (o.as_str(), k == field.index, Act::FormSegment(i, k))).collect();
                                self.segmented(control, &cells, p);
                            } else {
                                let text = self.fit(&current, control.width as f32 - self.s(40.) as f32, 13.);
                                let open = form.dropdown.is_some_and(|(f, _)| f == i);
                                self.button_face(&text, control, Hit::Do(Act::FormDrop(i)), if open || marked { Look::Active } else { Look::Outline }, Face::Normal, Align::Left, p);
                                self.put("▾", Rect { width: control.width.saturating_sub(self.su(12.)), ..control }, 14., p.muted, Face::Normal, Align::Right);
                                if open {
                                    dropdown_anchor = Some((i, control));
                                }
                            }
                        }
                        Control::Toggle => {
                            let track = Rect { x: control.x + control.width as i32 - self.s(44.), y: control.y + (control.height as i32 - self.s(22.)) / 2, width: self.su(40.), height: self.su(22.) };
                            self.paint_switch(track, field.on, p);
                            if !field.hint.is_empty() {
                                let note = Rect { x: control.x, width: control.width.saturating_sub(self.su(54.)), ..control };
                                let text = self.fit(&field.hint, note.width as f32, 11.);
                                self.put(&text, note, 11., p.muted, Face::Normal, Align::Right);
                            }
                            self.hits.push((row, Hit::Do(Act::FormToggle(i))));
                        }
                        Control::Button(caption) => {
                            let text = self.fit(caption, control.width as f32 - self.s(24.) as f32, 13.);
                            self.button(&text, control, Hit::Do(Act::FormButton(i)), if marked { Look::Active } else { Look::Outline }, p);
                        }
                        Control::Heading | Control::Info => {}
                    }
                }
            }
        }
        self.scroll_thumb(Rect { x: rect.x, y, width: rect.width - self.su(4.), height: list_height as u32 }, start, drawn, shown.len(), p);
        // Footer
        self.renderer.fill(Rect { x: rect.x + 1, y: footer_top, width: rect.width - 2, height: 1 }, p.border);
        if !form.note.is_empty() {
            let note = Rect { x: rect.x + pad, y: footer_top + self.s(4.), width: inner_width, height: self.su(20.) };
            let tint = if form.note.starts_with("Saved") || form.note.starts_with("Deleted") { p.muted } else { p.danger_text() };
            let text = self.fit(&form.note, note.width as f32, 12.);
            self.put(&text, note, 12., tint, Face::Normal, Align::Left);
        }
        let button_height = self.su(34.);
        let by = footer_top + footer_height - button_height as i32 - self.s(12.);
        let mut x = rect.x + rect.width as i32 - pad;
        let mut buttons: Vec<(String, Act, Look)> = Vec::new();
        match &form.submit {
            Some(caption) => {
                buttons.push((caption.clone(), Act::FormSubmit, if form.danger { Look::Danger } else { Look::Primary }));
                buttons.push(("Cancel".into(), Act::Dismiss, Look::Outline));
            }
            None => buttons.push(("Done".into(), Act::Dismiss, Look::Primary)),
        }
        for (caption, id) in &form.buttons {
            buttons.push((caption.clone(), Act::FormExtra(id), Look::Outline));
        }
        for (caption, act, look) in buttons {
            let width = self.measure(&caption, 13. * scale, Face::Normal).ceil() as u32 + self.su(32.);
            x -= width as i32;
            self.button(&caption, Rect { x, y: by, width, height: button_height }, Hit::Do(act), look, p);
            x -= self.s(8.);
        }
        // Open dropdown, drawn last so it covers the fields below
        if let Some((i, anchor)) = dropdown_anchor {
            if let Control::Choice(options) = &form.fields[i].control {
                let (_, first) = form.dropdown.unwrap_or((i, 0));
                self.paint_dropdown(window, anchor, options, i, form.fields[i].index, first, p);
            }
        }
        self.in_modal = false;
        self.form = Some(form);
    }
    fn paint_switch(&mut self, track: Rect, on: bool, p: Palette) {
        let off = super::gui::mix(p.border, p.text, if p.light { 0.22 } else { 0.28 });
        self.renderer.fill_round(track, track.height / 2, if on { p.accent } else { off });
        let knob = track.height - self.su(6.);
        let travel = (track.width - knob) as i32 - self.s(3.);
        self.renderer.fill_round(Rect { x: track.x + if on { travel } else { self.s(3.) }, y: track.y + self.s(3.), width: knob, height: knob }, knob / 2, 0xffffff);
    }
    fn paint_area(&mut self, rect: Rect, field: &Field, caret: Option<Option<usize>>, hit: Act, p: Palette) {
        let focused = caret.is_some();
        self.renderer.framed(rect, self.su(6.), p.background, if focused { p.accent_text() } else { super::gui::mix(p.border, p.text, 0.12) });
        let inner = Rect { x: rect.x + self.s(10.), y: rect.y + self.s(6.), width: rect.width.saturating_sub(self.su(20.)), height: rect.height.saturating_sub(self.su(12.)) };
        let size = 14. * self.scale;
        let line = size * 1.5;
        let len = field.text.chars().count();
        let at = caret.flatten().unwrap_or(len).min(len);
        let mut buffer = self.area_buffer(&field.text, Some(inner.width as f32), size, p.text);
        // The caret is drawn over the text, so moving it never changes how the lines wrap.
        let cursor = super::gui::area_cursor(&field.text, at);
        let (x, y) = buffer.cursor_position(&cursor).unwrap_or_else(|| {
            // An empty line has no glyph to stand on: it sits after the rows above it.
            let above = buffer.layout_runs().filter(|run| run.line_i < cursor.line).count();
            (0., above as f32 * line)
        });
        // Keep the caret inside the box when the text is longer than it.
        let offset = (y + line - inner.height as f32).max(0.);
        if field.text.is_empty() {
            self.label(&field.hint, inner, size, p.faint(p.background));
        } else {
            self.renderer.text(&mut self.fonts, &mut buffer, inner.x, inner.y - offset as i32, inner);
        }
        if focused {
            let bar = Rect { x: inner.x + x as i32, y: inner.y + (y - offset) as i32, width: self.su(1.5).max(1), height: line as u32 };
            self.renderer.fill(bar.intersect(inner), p.accent_text());
        }
        self.probes.push(super::gui::AreaProbe { rect, origin: (inner.x as f32, inner.y as f32 - offset), text: field.text.clone(), buffer });
        self.hits.push((rect, Hit::Do(hit)));
    }
    #[allow(clippy::too_many_arguments)]
    fn paint_dropdown(&mut self, window: Rect, anchor: Rect, options: &[String], field: usize, current: usize, first: usize, p: Palette) {
        let row = self.su(30.);
        let visible = options.len().min(8).max(1);
        let height = row * visible as u32 + self.su(8.);
        let below = anchor.y + anchor.height as i32 + self.s(2.);
        let y = if below + height as i32 + 8 > window.height as i32 { (anchor.y - height as i32 - self.s(2.)).max(4) } else { below };
        let width = anchor.width.max(self.su(260.)).min(window.width.saturating_sub(16));
        let rect = Rect { x: (anchor.x).min(window.width as i32 - width as i32 - 8).max(8), y, width, height };
        self.hits.push((window, Hit::Do(Act::FormCloseDrop)));
        self.soft_shadow(rect);
        self.renderer.framed(rect, self.su(7.), p.raised, super::gui::mix(p.border, p.text, 0.18));
        self.hits.push((rect, Hit::Do(Act::Block)));
        let first = first.min(options.len().saturating_sub(visible));
        for (k, option) in options.iter().enumerate().skip(first).take(visible) {
            let line = Rect { x: rect.x + self.s(4.), y: rect.y + self.s(4.) + (k - first) as i32 * row as i32, width: rect.width - self.su(8.), height: row };
            if k == current {
                self.renderer.fill_round(line, self.su(5.), super::gui::selected_fill(p.raised, p));
            } else if self.hot(line) {
                self.renderer.fill_round(line, self.su(5.), super::gui::hovered(p.raised, p));
            }
            let text = self.fit(option, line.width as f32 - self.s(20.) as f32, 13.);
            self.put(&text, Rect { x: line.x + self.s(10.), width: line.width - self.su(14.), ..line }, 13., p.text, Face::Normal, Align::Left);
            self.hits.push((line, Hit::Do(Act::FormPick(field, k))));
        }
    }
}

#[allow(dead_code)]
fn _types(_: CommentFilterQuery, _: CommentStatus, _: preset::Format) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn area_cursor_and_char_offset_round_trip() {
        let text = "Vo vales\npicó\n\nend";
        for at in 0..=text.chars().count() {
            let cursor = super::super::gui::area_cursor(text, at);
            assert_eq!(super::super::gui::area_char(text, &cursor), at, "offset {at}");
        }
    }

    #[test]
    fn numbers_format_without_trailing_zeros() {
        assert_eq!(format_number(1.0, 2), "1");
        assert_eq!(format_number(0.25, 2), "0.25");
        assert_eq!(format_number(12.0, 0), "12");
        assert_eq!(format_number(1.5, 1), "1.5");
    }

    #[test]
    fn fields_follow_their_conditions_and_edit_text() {
        let mut form = Form::new(
            FormKind::Snapshot,
            "T",
            vec![
                Field::choice("mode", "Mode", labels(&["a", "b"]), 0),
                Field::text("only_b", "B", "").when("mode", &[1]),
                Field::number("n", "N", 2., 0., 10., 1., 0, ""),
            ],
        );
        assert!(form.shown(0) && !form.shown(1) && form.shown(2));
        form.set_choice("mode", 1);
        assert!(form.shown(1));
        form.set_focus(1);
        form.type_text("héllo");
        assert_eq!(form.text("only_b"), "héllo");
        form.caret_move(-2, None);
        form.type_text("X");
        assert_eq!(form.raw("only_b"), "hélXlo");
        form.backspace(false);
        assert_eq!(form.raw("only_b"), "héllo");
        form.step(2, 1);
        form.step(2, 1);
        assert_eq!(form.number("n"), 4.);
        form.set_text("n", "99");
        assert_eq!(form.number("n"), 10., "values are clamped when read");
        form.cycle(0, 1);
        assert_eq!(form.choice("mode"), 0);
    }

    #[test]
    fn export_fields_round_trip_a_preset() {
        let all = preset::builtins();
        for original in all {
            let mut fields = vec![Field::choice("format", "Format", Format::ALL.iter().map(|f| f.name().to_owned()).collect(), position(&Format::ALL, original.format))];
            fields.extend(preset_fields(&original));
            let form = Form::new(FormKind::Export, "T", fields);
            let back = read_preset(&form, &original);
            let mut expected = original.clone();
            expected.sanitize();
            assert_eq!(back, expected, "{}", original.name);
        }
    }
}
