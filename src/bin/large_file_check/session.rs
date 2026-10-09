//! A scripted writing session, replayed on one document the way the application drives it.
//!
//! Every action is timed as the work the interface thread does for it: the edit itself, moving
//! the comment marks through it (`follow_edits`), handing a copy of the text to the indexer
//! (`scan`), then layout, paint and the copy to the window buffer. Work the application does on
//! worker threads (re-indexing, saving, spell checking, replace-all) is timed apart and labelled
//! "background", because it competes with typing for CPU even though it does not block a frame.

use justwrite::{
    core::{
        buffer::Document,
        comments::{self, EditRec},
        config::Settings,
        notes::{self, Marker, to_neutral},
        prose::BlockIndex,
        spell::{self, Speller},
        typography::{Language, Typography},
        workspace,
    },
    platform,
    render::{CpuRenderer, Rect, Renderer},
    ui::editor::EditorView,
};
use ropey::Rope;
use std::{hint::black_box, path::Path, time::Instant};

pub struct Row {
    pub name: String,
    pub ms: f64,
    pub limit: f64,
}

/// What a scenario is allowed to take on top of the per-frame budget.
pub struct Limits {
    pub open_document: f64,
    pub autosave_background: f64,
    pub replace_all_background: f64,
}

const FRAME: f64 = 5.;
/// The single worst of thousands of keystrokes is a scheduler spike more often than a slow path, so it is held to a 60 Hz frame; the p95 above it is held to the 5 ms budget.
const FRAME_WORST: f64 = 1000. / 60.;
const BIG_EDIT: f64 = 50.;
const REPLACE_APPLY: f64 = 100.;
const SPELL_SCREEN: f64 = 100.;
const FIND: f64 = 250.;
const TYPED_CHARS: usize = 3000;

struct Rng(u64);
impl Rng {
    fn below(&mut self, n: usize) -> usize {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        ((self.0 >> 33) as usize) % n
    }
}
const VOCAB: &[&str] = &[
    "rain", "night", "house", "river", "window", "she", "he", "they", "walked", "waited", "the", "a", "and", "slowly",
    "quiet", "old", "mill", "letter", "door", "morning", "didn't", "remembered", "kitchen", "road", "listened",
    "strange", "light", "harbor", "without", "answer",
];

enum Op {
    Char(char),
    Enter,
}
/// What an author types: sentences with quotes, apostrophes, double hyphens and ellipses (which
/// auto-typography rewrites as they go), and a new paragraph every few sentences.
fn script(rng: &mut Rng, chars: usize) -> Vec<Op> {
    let mut ops = Vec::new();
    let mut sentences = 0;
    while ops.len() < chars {
        let n = 5 + rng.below(10);
        let quoted = rng.below(4) == 0;
        if quoted {
            ops.push(Op::Char('"'));
        }
        for i in 0..n {
            let mut word = VOCAB[rng.below(VOCAB.len())].to_owned();
            if i == 0 {
                word = word[..1].to_uppercase() + &word[1..];
            }
            ops.extend(word.chars().map(Op::Char));
            if i + 1 < n {
                if rng.below(14) == 0 {
                    ops.extend("--".chars().map(Op::Char));
                } else {
                    ops.push(Op::Char(if rng.below(9) == 0 { ',' } else { ' ' }));
                    if matches!(ops.last(), Some(Op::Char(','))) {
                        ops.push(Op::Char(' '));
                    }
                }
            }
        }
        ops.extend(if rng.below(12) == 0 { "...".chars() } else { ".".chars() }.map(Op::Char));
        if quoted {
            ops.push(Op::Char('"'));
        }
        sentences += 1;
        if sentences % (3 + rng.below(4)) == 0 {
            ops.push(Op::Enter);
        } else {
            ops.push(Op::Char(' '));
        }
    }
    ops
}
/// `paragraphs` paragraphs of about 100 words, to paste.
fn block(rng: &mut Rng, paragraphs: usize) -> String {
    (0..paragraphs)
        .map(|_| (0..100).map(|_| VOCAB[rng.below(VOCAB.len())]).collect::<Vec<_>>().join(" ") + ".")
        .collect::<Vec<_>>()
        .join("\n")
}

struct Mark {
    start: usize,
    end: usize,
    orphaned: bool,
}
struct Rig {
    doc: Document,
    view: EditorView,
    renderer: CpuRenderer,
    fonts: cosmic_text::FontSystem,
    settings: Settings,
    marker: Marker,
    blocks: BlockIndex,
    output: Vec<u32>,
    rect: Rect,
    marks: Vec<Mark>,
}
impl Rig {
    fn new(display: &str, marks: &[(usize, usize)], width: u32, height: u32) -> (Rig, f64) {
        let settings = Settings::default();
        let marker = settings.marker();
        let start = Instant::now();
        let mut doc = Document::new(Rope::from_str(display));
        let mut blocks = BlockIndex::default();
        blocks.update_range(&doc.text, 0..doc.text.len_lines(), &marker);
        let opened = start.elapsed().as_secs_f64() * 1000.;
        let mut renderer = CpuRenderer::default();
        renderer.resize(width, height);
        let at = doc.text.line_to_char(doc.text.char_to_line(doc.text.len_chars() / 2));
        doc.select(at, false);
        let len = doc.text.len_chars();
        let mut rig = Rig {
            doc,
            view: EditorView::default(),
            renderer,
            fonts: platform::font_system(),
            settings,
            marker,
            blocks,
            output: vec![0; width as usize * height as usize],
            rect: Rect { x: 36, y: 42, width: width - 72, height: height - 90 },
            marks: marks.iter().filter(|(s, e)| s < e && *e <= len).map(|&(start, end)| Mark { start, end, orphaned: false }).collect(),
        };
        // Settle: the first layouts reveal the caret and warm the shaping caches.
        rig.view.ensure_cursor(&rig.doc);
        for _ in 0..4 {
            rig.frame();
        }
        (rig, opened)
    }
    fn frame(&mut self) {
        self.view.layout(&self.doc, &self.blocks, &self.settings, &mut self.fonts, self.rect);
        self.view.paint(&self.doc, &mut self.fonts, &mut self.renderer);
        self.output.copy_from_slice(self.renderer.pixels());
        black_box(&self.output);
        let _ = self.renderer.damage();
    }
    /// Runs `action`, then everything the interface thread does after an edit. Returns the
    /// interface-thread milliseconds and the background re-index milliseconds.
    fn act(&mut self, action: impl FnOnce(&mut Document, &Marker)) -> (f64, f64) {
        let start = Instant::now();
        action(&mut self.doc, &self.marker);
        for edit in self.doc.take_edits() {
            self.map_marks(edit);
        }
        self.view.ensure_cursor(&self.doc);
        black_box(self.doc.text.clone());
        self.frame();
        let ui = start.elapsed().as_secs_f64() * 1000.;
        let start = Instant::now();
        self.blocks.update_range(&self.doc.text, self.doc.changed.clone(), &self.marker);
        (ui, start.elapsed().as_secs_f64() * 1000.)
    }
    /// The same walk as `follow_edits` in the application.
    fn map_marks(&mut self, edit: EditRec) {
        for m in self.marks.iter_mut().filter(|m| !m.orphaned) {
            match comments::map_range(m.start..m.end, edit) {
                Some(r) => (m.start, m.end) = (r.start, r.end),
                None => {
                    m.orphaned = true;
                    m.start = edit.start.min(m.start);
                    m.end = m.start;
                }
            }
        }
        for m in self.marks.iter_mut().filter(|m| m.orphaned) {
            if edit.start <= m.start {
                let shifted = (m.start as isize + edit.inserted as isize - edit.removed as isize).max(edit.start as isize) as usize;
                (m.start, m.end) = (shifted, shifted);
            }
        }
    }
    /// Selects the next word and leaves it selected.
    fn select_word(&mut self) {
        self.doc.horizontal(true, false, true);
        self.doc.horizontal(true, false, false);
        self.doc.horizontal(true, true, true);
    }
}

fn percentile(samples: &[f64], fraction: f64) -> f64 {
    let mut sorted = samples.to_vec();
    sorted.sort_by(f64::total_cmp);
    sorted.get(((sorted.len().max(1) - 1) as f64 * fraction).ceil() as usize).copied().unwrap_or(0.)
}
fn max(samples: &[f64]) -> f64 {
    samples.iter().copied().fold(0., f64::max)
}
fn timed<T>(f: impl FnOnce() -> T) -> (T, f64) {
    let start = Instant::now();
    let value = f();
    (value, start.elapsed().as_secs_f64() * 1000.)
}

/// Replays the session on `display`, a document in the form the editor holds it. `marks` are the
/// review comments' ranges. `label` prefixes every row name.
pub fn run(label: &str, display: &str, marks: &[(usize, usize)], limits: &Limits, speller: Option<&Speller>, scratch: &Path) -> (Vec<Row>, Vec<String>) {
    let mut rows = Vec::new();
    let mut problems = Vec::new();
    let mut row = |name: &str, ms: f64, limit: f64| rows.push(Row { name: format!("{label} {name}"), ms, limit });
    let mut check = |ok: bool, what: &str| {
        if !ok {
            problems.push(format!("{label}: {what}"));
        }
    };
    let mut rng = Rng(0x5eed);
    let (mut rig, opened) = Rig::new(display, marks, 1100, 760);
    row("open document", opened, limits.open_document);
    let typography = Typography::default();

    // Typing real prose through the typing path, with auto-typography.
    let ops = script(&mut rng, TYPED_CHARS);
    let length_before = rig.doc.text.len_chars();
    let (mut ui, mut reindex) = (Vec::new(), Vec::new());
    for op in &ops {
        let (a, b) = rig.act(|doc, _| match op {
            Op::Char(c) => doc.type_text(&c.to_string(), Some((&typography, Language::English))),
            Op::Enter => doc.enter(),
        });
        ui.push(a);
        reindex.push(b);
    }
    check(rig.doc.text.len_chars() >= length_before + ops.len() / 2 && rig.doc.dirty(), "typing did not change the document");
    let edge = 500.min(ui.len() / 3);
    row("type p50 first 500 keys", percentile(&ui[..edge], 0.5), FRAME);
    row("type p50 last 500 keys", percentile(&ui[ui.len() - edge..], 0.5), FRAME);
    row("type p95", percentile(&ui, 0.95), FRAME);
    row("type max of 3000 keys", max(&ui), FRAME_WORST);
    row("reindex after key max (background)", max(&reindex), FRAME);

    // Undo and redo, which replay whole words.
    let length_typed = rig.doc.text.len_chars();
    let undone: Vec<f64> = (0..20).map(|_| rig.act(|doc, _| doc.undo()).0).collect();
    row("undo max", max(&undone), FRAME);
    let redone: Vec<f64> = (0..20).map(|_| rig.act(|doc, _| doc.redo()).0).collect();
    row("redo max", max(&redone), FRAME);
    check(rig.doc.text.len_chars() == length_typed, "redo did not restore what undo took back");

    // Moving about by word.
    let moved: Vec<f64> = (0..60).map(|_| rig.act(|doc, _| doc.horizontal(false, false, true)).0).collect();
    row("move by word max", max(&moved), FRAME);

    // Formatting and notes on a selected word.
    let mut italic = Vec::new();
    for _ in 0..20 {
        rig.select_word();
        italic.push(rig.act(|doc, _| doc.toggle_emphasis("*")).0);
    }
    row("toggle italic max", max(&italic), FRAME);
    let mut note = Vec::new();
    for n in 0..20 {
        if n % 2 == 0 {
            rig.select_word();
        } else {
            let cursor = rig.doc.cursor;
            rig.doc.select(cursor, false);
        }
        note.push(
            rig.act(|doc, marker| {
                if let Some(t) = notes::toggle(&doc.text, doc.selection(), marker) {
                    doc.replace(t.range, &t.text);
                    doc.anchor = t.select.start;
                    doc.cursor = t.select.end;
                }
            })
            .0,
        );
    }
    row("toggle note max", max(&note), FRAME);

    // Deleting a paragraph.
    let mut deleted = Vec::new();
    for _ in 0..10 {
        let line = rig.doc.text.char_to_line(rig.doc.cursor);
        let (a, b) = (rig.doc.text.line_to_char(line), rig.doc.text.line_to_char((line + 1).min(rig.doc.text.len_lines().saturating_sub(1))));
        rig.doc.select(a, false);
        rig.doc.select(b, true);
        deleted.push(rig.act(|doc, _| doc.delete(true)).0);
    }
    row("delete paragraph max", max(&deleted), FRAME);

    // A big paste, deleting it again, and undoing that.
    let pasted_text = block(&mut rng, 50);
    let (mut paste, mut cut, mut restore) = (Vec::new(), Vec::new(), Vec::new());
    for _ in 0..3 {
        let length = rig.doc.text.len_chars();
        let from = rig.doc.cursor;
        paste.push(rig.act(|doc, _| doc.insert(&pasted_text)).0);
        let to = rig.doc.cursor;
        rig.doc.select(from, false);
        rig.doc.select(to, true);
        cut.push(rig.act(|doc, _| doc.delete(true)).0);
        restore.push(rig.act(|doc, _| doc.undo()).0);
        check(rig.doc.text.len_chars() == length + pasted_text.chars().count(), "undo of the big delete did not restore the paste");
        rig.doc.select(from, false);
        let _ = rig.act(|doc, _| doc.undo());
        check(rig.doc.text.len_chars() == length, "undo of the paste did not restore the text");
    }
    row("paste 5000 words max", max(&paste), BIG_EDIT);
    row("delete 5000 words max", max(&cut), BIG_EDIT);
    row("undo 5000-word delete max", max(&restore), BIG_EDIT);

    // Saving while writing: the interface thread only takes a copy of the rope; the conversion,
    // word count and file write run on a worker.
    let (_, clone_ms) = timed(|| black_box(rig.doc.text.clone()));
    row("autosave snapshot on interface thread", clone_ms, FRAME);
    let mut saves = Vec::new();
    for _ in 0..3 {
        let (_, ms) = timed(|| {
            let (neutral, _) = to_neutral(&rig.doc.text.to_string(), &rig.marker);
            black_box(notes::word_count(&neutral));
            let _ = std::fs::write(scratch.join("autosave.tmp"), neutral);
        });
        saves.push(ms);
    }
    row("autosave work (background)", max(&saves), limits.autosave_background);

    // Replace all: the worker builds the new text, the interface thread swaps it in.
    let snapshot = rig.doc.text.clone();
    let (new_text, ms) = timed(|| Rope::from_str(&workspace::replace_all(&snapshot.to_string(), "the", "THE", false)));
    row("replace all (background)", ms, limits.replace_all_background);
    check(new_text.len_chars() == snapshot.len_chars(), "replace all changed the length (the -> THE should not)");
    let (ui, _) = rig.act(|doc, _| {
        let end = doc.text.len_chars();
        doc.replace_rope(0..end, new_text)
    });
    row("replace all apply and redraw", ui, REPLACE_APPLY);

    // Spell check of what is on screen.
    if let Some(speller) = speller {
        let mut screens = Vec::new();
        for n in 1..=5 {
            let len = rig.doc.text.len_chars();
            let at = len * n / 6;
            let page: String = rig.doc.text.slice(at..(at + 4000).min(len)).into();
            screens.push(timed(|| black_box(spell::misspelled(speller, &page, &rig.marker, false))).1);
        }
        row("spellcheck one screen (background)", max(&screens), SPELL_SCREEN);
    }

    // Find in the document, as the Find panel does.
    for (name, query) in [("find common word", "the"), ("find no match", "zzqxj")] {
        let (found, ms) = timed(|| workspace::find(&rig.doc.text, query, false, 500).len());
        black_box(found);
        row(name, ms, FIND);
    }
    (rows, problems)
}
