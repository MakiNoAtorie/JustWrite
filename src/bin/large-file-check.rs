//! Timing gate for very large manuscripts (50k to 1M words).
//!
//!     python3 tools/gen_bench_jw.py                      # make bench_data/*.jw once
//!     cargo run --release --bin large-file-check         # every bench_data/*.jw
//!     cargo run --release --bin large-file-check -- path/to/a.jw   # chosen files
//!     cargo run --release --bin large-file-check -- --quiet        # only failures and totals
//!
//! Each project is copied to a temporary folder first, so the generated files are never
//! modified or locked. One line is printed per measurement; the same rows go to
//! `large-file-results.json` next to the first project. The exit code is 1 when a limit is missed
//! or a correctness check fails, and 2 when there is nothing to run.
//!
//! The limits below are budgets for a 1M-word book: the same limit applies to every size, so the
//! smaller books show how much headroom there is. They are first estimates; tune them in one
//! place, here, once real numbers exist.

use justwrite::{
    core::{
        buffer::Document,
        comments::{CommentSet, EditRec},
        config::Settings,
        notes::{self, to_display},
        prose::BlockIndex,
        workspace,
    },
    export::{
        self,
        model::{Scope, Source},
        preset,
    },
    history::{self, History},
    jw::{
        Project,
        archive,
        manifest::{COMMENTS, Node, Section},
    },
    platform,
    render::{CpuRenderer, Rect, Renderer},
    ui::editor::EditorView,
};
use ropey::Rope;
use std::{
    fs,
    hint::black_box,
    path::{Path, PathBuf},
    time::Instant,
};

// Budgets in milliseconds.
const READ_MANIFEST: f64 = 100.;
const OPEN_PROJECT: f64 = 2000.;
const READ_ALL_CHAPTERS: f64 = 1000.;
const RECOUNT_WORDS: f64 = 1000.;
const HASH_MANUSCRIPT: f64 = 1000.;
const FIRST_SNAPSHOT: f64 = 5000.;
const INCREMENTAL_SNAPSHOT: f64 = 500.;
const LOAD_COMMENTS: f64 = 200.;
const COMMENTS_THROUGH_EDITS: f64 = 50.;
const SEARCH_ALL: f64 = 250.;
const EXPORT_TEXT: f64 = 3000.;
const EXPORT_PACKAGED: f64 = 10000.;
const SAVE_AFTER_EDIT: f64 = 1000.;
const LOAD_CHAPTER_INTO_EDITOR: f64 = 100.;
/// One keystroke: layout, paint and copy to the window buffer (the 5 ms of the specification).
const KEYSTROKE: f64 = 5.;
/// One scroll step: a frame at 60 Hz.
const SCROLL_STEP: f64 = 1000. / 60.;

struct Row {
    file: String,
    name: String,
    ms: f64,
    limit: f64,
}
impl Row {
    fn pass(&self) -> bool {
        self.ms < self.limit
    }
}
struct Report {
    rows: Vec<Row>,
    problems: Vec<String>,
    quiet: bool,
}
impl Report {
    fn record(&mut self, file: &str, name: &str, ms: f64, limit: f64) {
        let row = Row { file: file.into(), name: name.into(), ms, limit };
        if !self.quiet || !row.pass() {
            println!("{} {file:<10} {name:<38} {ms:>9.2} ms  (limit {limit:.1})", if row.pass() { "ok  " } else { "FAIL" });
        }
        self.rows.push(row);
    }
    fn problem(&mut self, file: &str, message: String) {
        println!("FAIL {file:<10} {message}");
        self.problems.push(format!("{file}: {message}"));
    }
}

fn ms_since(start: Instant) -> f64 {
    start.elapsed().as_secs_f64() * 1000.
}
/// Runs `f` and returns its result with the time it took in milliseconds.
fn timed<T>(f: impl FnOnce() -> T) -> (T, f64) {
    let start = Instant::now();
    let value = f();
    (value, ms_since(start))
}
fn max_of(samples: &[f64]) -> f64 {
    samples.iter().copied().fold(0., f64::max)
}

/// Types and deletes a character `rounds` times at `at`, timing layout, paint and copy together.
/// Returns the worst keystroke.
fn keystrokes(document: &mut Document, at: usize, width: u32, height: u32, rounds: usize) -> f64 {
    let mut fonts = platform::font_system();
    let mut view = EditorView::default();
    let mut renderer = CpuRenderer::default();
    renderer.resize(width, height);
    let rect = Rect { x: 36, y: 42, width: width - 72, height: height - 90 };
    let settings = Settings::default();
    let marker = settings.marker();
    let mut blocks = BlockIndex::default();
    blocks.update_range(&document.text, 0..document.text.len_lines(), &marker);
    let mut output = vec![0; width as usize * height as usize];
    document.select(at.min(document.text.len_chars()), false);
    view.ensure_cursor(document);
    // Settle: the first layouts reveal the caret and warm the shaping caches.
    for _ in 0..4 {
        view.layout(document, &blocks, &settings, &mut fonts, rect);
        view.paint(document, &mut fonts, &mut renderer);
    }
    let mut samples = Vec::new();
    for iteration in 0..rounds + 20 {
        let start = Instant::now();
        if iteration % 2 == 0 {
            document.insert("x");
        } else {
            document.delete(true);
        }
        view.layout(document, &blocks, &settings, &mut fonts, rect);
        view.paint(document, &mut fonts, &mut renderer);
        output.copy_from_slice(renderer.pixels());
        black_box(&output);
        let _ = renderer.damage();
        if iteration >= 20 {
            samples.push(ms_since(start));
        }
        // The application re-indexes on a worker thread, off the typing path.
        blocks.update_range(&document.text, document.changed.clone(), &marker);
    }
    max_of(&samples)
}
/// Scrolls `steps` times from `at`, timing layout, paint and copy; returns the worst step.
fn scrolling(document: &Document, at: usize, width: u32, height: u32, steps: usize) -> f64 {
    let mut fonts = platform::font_system();
    let mut view = EditorView::default();
    let mut renderer = CpuRenderer::default();
    renderer.resize(width, height);
    let rect = Rect { x: 36, y: 42, width: width - 72, height: height - 90 };
    let settings = Settings::default();
    let marker = settings.marker();
    let mut blocks = BlockIndex::default();
    blocks.update_range(&document.text, 0..document.text.len_lines(), &marker);
    let mut output = vec![0; width as usize * height as usize];
    let mut doc = Document::new(document.text.clone());
    doc.select(at.min(doc.text.len_chars()), false);
    view.ensure_cursor(&doc);
    for _ in 0..4 {
        view.layout(&doc, &blocks, &settings, &mut fonts, rect);
        view.paint(&doc, &mut fonts, &mut renderer);
    }
    let mut samples = Vec::new();
    for iteration in 0..steps + 20 {
        let start = Instant::now();
        view.scroll(&doc, 3);
        view.layout(&doc, &blocks, &settings, &mut fonts, rect);
        view.paint(&doc, &mut fonts, &mut renderer);
        output.copy_from_slice(renderer.pixels());
        black_box(&output);
        let _ = renderer.damage();
        if iteration >= 20 {
            samples.push(ms_since(start));
        }
    }
    max_of(&samples)
}

fn bench(jw: &Path, report: &mut Report) -> Result<(), String> {
    let file = jw.file_stem().map_or_else(|| "project".into(), |s| s.to_string_lossy().into_owned());
    let scratch = tempfile::tempdir().map_err(|e| e.to_string())?;
    let copy = scratch.path().join("project.jw");
    let data = scratch.path().join("data");
    fs::copy(jw, &copy).map_err(|e| format!("cannot copy {}: {e}", jw.display()))?;

    // Opening.
    let (manifest, ms) = timed(|| archive::read_manifest(&copy));
    let manifest = manifest?;
    report.record(&file, "read manifest.json", ms, READ_MANIFEST);
    let words_in_file = manifest.tree.manuscript_words();
    let chapters = manifest.tree.leaves(Section::Manuscript).len();
    println!("---- {file}: {words_in_file} words, {chapters} chapters, {:.1} MB zip", fs::metadata(&copy).map_or(0., |m| m.len() as f64 / 1e6));
    let (opened, ms) = timed(|| Project::open(&data, &copy, false));
    let mut project = opened.map_err(|_| "cannot open the project".to_string())?.project;
    report.record(&file, "open project (unpack)", ms, OPEN_PROJECT);

    // Reading and counting everything.
    let (source, ms) = timed(|| Source::from_project(&project, 0));
    let source = source.map_err(|e| e.to_string())?;
    report.record(&file, "read every chapter", ms, READ_ALL_CHAPTERS);
    let (recounted, ms) = timed(|| project.recount());
    recounted.map_err(|e| e.to_string())?;
    report.record(&file, "recount words", ms, RECOUNT_WORDS);
    if project.manuscript_words() != words_in_file {
        report.problem(&file, format!("word count mismatch: manifest says {words_in_file}, recount gives {}", project.manuscript_words()));
    }

    // Comments.
    let (set, ms) = timed(|| CommentSet::parse(&project.work.read(COMMENTS).unwrap_or_default()));
    report.record(&file, "load comments", ms, LOAD_COMMENTS);
    if set.comments.is_empty() {
        report.problem(&file, "no review comments were loaded".into());
    }
    let busiest = set
        .comments
        .iter()
        .fold(std::collections::HashMap::<&str, usize>::new(), |mut m, c| {
            *m.entry(c.item.as_str()).or_default() += 1;
            m
        })
        .into_iter()
        .max_by_key(|(_, n)| *n)
        .map(|(id, _)| id.to_owned());
    if let Some(item) = busiest {
        let mut set = set.clone();
        let (_, ms) = timed(|| {
            for n in 0..200 {
                set.map_edit(&item, EditRec { start: n * 3, removed: 0, inserted: 1 });
            }
        });
        report.record(&file, "comments through 200 edits", ms, COMMENTS_THROUGH_EDITS);
    }

    // Search across the whole manuscript, as the Find panel does chapter by chapter.
    let ropes: Vec<Rope> = source.tree.leaves(Section::Manuscript).iter().map(|(n, _)| Rope::from_str(&source.texts[n.id()])).collect();
    for (name, query) in [("search: common word", "the"), ("search: no match", "zzqxj")] {
        let (found, ms) = timed(|| ropes.iter().map(|r| workspace::find(r, query, false, 500).len()).sum::<usize>());
        black_box(found);
        report.record(&file, name, ms, SEARCH_ALL);
    }

    // History.
    let mut history = History::default();
    let (state, ms) = timed(|| history::store::State::working(&project, false));
    black_box(state.map_err(|e| e.to_string())?);
    report.record(&file, "hash the whole manuscript", ms, HASH_MANUSCRIPT);
    let (captured, ms) = timed(|| history.capture(&project, history::Kind::Manual, "first", 1000));
    captured.map_err(|e| e.to_string())?;
    report.record(&file, "first snapshot (writes every blob)", ms, FIRST_SNAPSHOT);

    // The chapter with the most words is the heaviest one to open in the editor.
    let (largest, _) = manifest
        .tree
        .leaves(Section::Manuscript)
        .into_iter()
        .filter_map(|(n, _)| if let Node::Chapter { id, words, .. } = n { Some((id.clone(), *words)) } else { None })
        .max_by_key(|(_, w)| *w)
        .ok_or("the project has no chapters")?;
    let neutral = project.read_text(&largest).map_err(|e| e.to_string())?;

    // Save and snapshot after editing that one chapter.
    project.write_text(&largest, &format!("{neutral}An edit.\n")).map_err(|e| e.to_string())?;
    let (captured, ms) = timed(|| history.capture(&project, history::Kind::Auto, "auto", 2000));
    captured.map_err(|e| e.to_string())?;
    report.record(&file, "incremental snapshot (one chapter)", ms, INCREMENTAL_SNAPSHOT);
    let (packed, ms) = timed(|| project.pack(true));
    packed.map_err(|_| "cannot save the project".to_string())?;
    report.record(&file, "save after editing one chapter", ms, SAVE_AFTER_EDIT);

    // Exports.
    let presets = preset::builtins();
    for (name, id, limit) in [
        ("export Markdown", preset::MARKDOWN, EXPORT_TEXT),
        ("export plain text", preset::PLAIN, EXPORT_TEXT),
        ("export DOCX", preset::MANUSCRIPT_DOCX, EXPORT_PACKAGED),
        ("export EPUB", preset::EPUB, EXPORT_PACKAGED),
    ] {
        let Some(chosen) = presets.iter().find(|p| p.id == id) else { continue };
        let (result, ms) = timed(|| export::export(&source, &Scope::Whole, chosen));
        black_box(result.map_err(|e| format!("{name}: {e}"))?);
        report.record(&file, name, ms, limit);
    }

    // The editor, on the largest chapter.
    let settings = Settings::default();
    let marker = settings.marker();
    let (mut document, ms) = timed(|| {
        let (display, _) = to_display(&neutral, &marker);
        let mut document = Document::new(Rope::from_str(&display));
        let mut blocks = BlockIndex::default();
        blocks.update_range(&document.text, 0..document.text.len_lines(), &marker);
        black_box(blocks);
        document.select(0, false);
        document
    });
    report.record(&file, &format!("load a {}-word chapter", notes::word_count(&neutral)), ms, LOAD_CHAPTER_INTO_EDITOR);
    let length = document.text.len_chars();
    for (name, at) in [("start", 0), ("middle", length / 2), ("end", length)] {
        let worst = keystrokes(&mut document, at, 1100, 760, 100);
        report.record(&file, &format!("keystroke at chapter {name}"), worst, KEYSTROKE);
    }
    let worst = keystrokes(&mut document, length / 2, 2560, 1440, 100);
    report.record(&file, "keystroke at middle, 2560x1440", worst, KEYSTROKE);
    let worst = scrolling(&document, length / 2, 1100, 760, 100);
    report.record(&file, "scroll step", worst, SCROLL_STEP);

    project.abandon();
    Ok(())
}

fn main() {
    let mut quiet = false;
    let mut paths: Vec<PathBuf> = Vec::new();
    for arg in std::env::args().skip(1) {
        match arg.as_str() {
            "--quiet" => quiet = true,
            other => paths.push(PathBuf::from(other)),
        }
    }
    if paths.is_empty() {
        let dir = std::env::var_os("JW_BENCH_DIR").map_or_else(|| PathBuf::from("bench_data"), PathBuf::from);
        if let Ok(entries) = fs::read_dir(&dir) {
            paths = entries.filter_map(Result::ok).map(|e| e.path()).filter(|p| p.extension().is_some_and(|e| e == "jw")).collect();
        }
    }
    paths.sort_by_key(|p| fs::metadata(p).map_or(0, |m| m.len()));
    if paths.is_empty() {
        eprintln!("No .jw files found. Run `python3 tools/gen_bench_jw.py` first, or pass paths (JW_BENCH_DIR overrides bench_data).");
        std::process::exit(2);
    }

    let mut report = Report { rows: Vec::new(), problems: Vec::new(), quiet };
    for path in &paths {
        if let Err(message) = bench(path, &mut report) {
            report.problem(&path.display().to_string(), message);
        }
    }

    let failed: Vec<&Row> = report.rows.iter().filter(|r| !r.pass()).collect();
    let passed = failed.is_empty() && report.problems.is_empty();
    println!("{} measurements, {} over limit, {} problems: {}", report.rows.len(), failed.len(), report.problems.len(), if passed { "PASS" } else { "FAIL" });

    let rows: Vec<serde_json::Value> = report
        .rows
        .iter()
        .map(|r| serde_json::json!({"file": r.file, "name": r.name, "ms": (r.ms * 1000.).round() / 1000., "limit_ms": r.limit, "pass": r.pass()}))
        .collect();
    let report_path = paths[0].with_file_name("large-file-results.json");
    let json = serde_json::json!({"passed": passed, "problems": report.problems, "results": rows});
    if let Err(error) = fs::write(&report_path, serde_json::to_string_pretty(&json).unwrap_or_default() + "\n") {
        eprintln!("Could not write {}: {error}", report_path.display());
    }
    if !passed {
        std::process::exit(1);
    }
}
