# Next steps

What is left after the UX passes on `claude/gifted-dirac-7edr5f`. Items are grouped by how much
they need a decision from the maintainer.

## Needs checking on a real desktop

* **Performance gate.** `cargo run --release --bin performance-check` reports "not passed" in the
  cloud container used for the UX passes, on the 2560 x 1440 stress cases (about 3.5 ms against
  2.0 ms recorded, and 7.7 ms for the 10 MB single line). The original code, before any UX change,
  gave the same numbers there, and the 1100 x 760 cases match the recorded baseline. Re-run the
  gate on the development machine before merging. `performance-results.json` was deliberately left
  unchanged.
* **Import dialog.** File ▸ Import opens a native file chooser through the desktop portal, which
  could not be driven headlessly. It has not been looked at or tested in the UX passes.
* **Default interface font.** The Linux fallback is DejaVu Sans. Check what the default is on
  Windows and macOS (`platform::default_sans`) and whether it is the intended look.

## Small and safe

* **Tab drop to split.** Dragging a tab shows a ghost label. Confirm that dropping a tab on the
  editor area creates a split; the test drop landed outside the editor.
* **Export dialog and paged preview.** Seen, not audited in depth. Worth a pass for spacing, the
  focus highlight on the first row, and the page indicator in the preview.
* **Error messages fade.** Footer status messages now disappear after 10 seconds
  (`STATUS_LIFETIME` in `src/ui/app.rs`). If a failure message should stay until dismissed, give
  errors a longer lifetime or a separate path.
* **Test-only warnings.** Two "variable does not need to be mutable" warnings remain in test code:
  `src/core/notes.rs` (around line 791) and `src/history/store.rs` (around line 443).

## Needs a decision

* **Keyboard access to comment cards.** Resolve, Reply, Edit and Delete are mouse-only. A fix means
  giving the comments panel its own focus handling (arrow keys to move between cards, letter keys
  or Enter for the actions).
* **Tab-key focus through the window chrome.** Not added: every toolbar and footer button already
  has a shortcut, a menu entry and a palette entry, and Tab types in the editor. Revisit only if
  full keyboard traversal of the main window is a requirement.
* **Live word counts.** The tree and footer counts update on autosave, not per keystroke, to protect
  the 5 ms keystroke budget. If live counts matter, count the active tab incrementally and keep the
  gate in view.
* **Chapter stepping scope.** Ctrl+PageDown and Ctrl+PageUp walk the manuscript and then the notes
  pages, in tree order. Decide whether they should stay inside the manuscript.

## Done in the UX passes (for reference)

* Contrast: secondary text and accent-as-text meet 4.5:1 in both themes, with unit tests over every
  accent choice (`src/render/theme.rs`).
* Status colours (error, warning, added, deleted) are theme-aware.
* Confirmation dialogs put the question in primary text; focus outlines use the readable accent.
* Disabled commands and the editor placeholder stay at least 3:1.
* Tabs keep a readable width and cut long titles in the middle.
* Correct plurals ("1 word", "1 page or chapter") and arrows in shortcut text (Alt+↑).
* Project search previews end on whole words, with the match highlighted.
* Next and previous chapter commands; the welcome screen lists the core shortcuts.
* The footer hides tree, panel and split toggles when no project is open.
* The window opens at a size that fits the screen.

## Large-file performance benchmarks

Goal: JustWrite must not chug on huge manuscripts (50k to 1M words).

### Step 1: dummy `.jw` generator (DONE)

* Tool: `tools/gen_bench_jw.py` (Python 3, stdlib only, deterministic via `--seed`).
* Run: `python3 tools/gen_bench_jw.py` writes to `bench_data/` (git-ignored). Options: `--sizes`, `--out`, `--avg-chapter`, `--chapters-per-part`, `--comment-every`, `--seed`.
* Output (default seed 42), about 7 s total:

  | File | Words | Chapters (parts) | Review comments | Text | Zip |
  | ---- | ----- | ---------------- | --------------- | ---- | --- |
  | `bench_data/bench_50k.jw` | 50,000 | 18 (2) | 90 | 0.3 MB | 0.1 MB |
  | `bench_data/bench_100k.jw` | 100,000 | 32 (3) | 178 | 0.7 MB | 0.3 MB |
  | `bench_data/bench_500k.jw` | 500,000 | 166 (14) | 904 | 3.3 MB | 1.3 MB |
  | `bench_data/bench_1m.jw` | 1,000,000 | 329 (28) | 1812 | 6.6 MB | 2.6 MB |

* Content: `$$` notes stored as U+E000 (line), U+E001..U+E002 (inline) and U+E003/U+E004 (block); a few literal `$$`; italic/bold; dialogue; scene breaks; Word-style review comments in `comments.json` (tags, open/resolved, replies); three note pages.
* Manifest word counts use a Python port of `notes::word_count`. The script re-reads each archive and checks the totals and every comment anchor before it reports.
* Not covered yet: `history/` (snapshots) is not generated. Build it in the benchmarks with `History::capture` if needed.
* Note: the generated files have NOT yet been opened by the Rust code. Step 3 covers that.

### Step 2: timing benchmarks and speed tests (DONE, not yet run on the big files)

* Tool: `src/bin/large-file-check.rs` (bin `large-file-check`, registered in `Cargo.toml`). Separate from `performance-check`, so the existing gate stays fast.
* Run: `cargo run --release --bin large-file-check` (every `bench_data/*.jw`, smallest first), `-- path/to/a.jw` for chosen files, `-- --quiet` for failures and totals only. `JW_BENCH_DIR` overrides the folder. Exit code 0 pass, 1 over limit or correctness problem, 2 no files found.
* Each project is copied to a temp folder first, so `bench_data/*.jw` are never modified or locked.
* Output is CSV, meant to be read by an LLM, so it is terse: a header `result,name,ms,limit`, a `# file words=N chapters=N` line per project, then one row per measurement such as `P,open project (unpack),410.37,2000` (`P` = ms below limit, `F` = at or over). Last line is a `# ... measurements=N over_limit=N problems=N PASS|FAIL` summary. `--quiet` prints only `F` rows (name prefixed with the file) and the summary. The same rows go to `bench_data/large-file-results.json` (git-ignored). 22 measurements per file.
* Measured: read manifest, open (unpack), read every chapter, recount words (also checks the total against the manifest, so it catches Rust and Python word counts disagreeing), load comments, comments through 200 edits, search (common word and no match, across all chapters), hash manuscript, first snapshot, incremental snapshot, save after one-chapter edit, export (Markdown, plain, DOCX, EPUB), load the largest chapter into the editor, keystroke latency at chapter start, middle and end (1100x760) and at the middle at 2560x1440, scroll step.
* Not measured: PDF export (typst, very slow on 1M words; add it on its own if wanted), the whole manuscript as one document, spell check, opening the window.
* Limits are constants at the top of the file, one set for all sizes, sized for the 1M-word book (open 2 s, read all 1 s, search 250 ms, Markdown/plain export 3 s, DOCX/EPUB 10 s, keystroke 5 ms, scroll 16.7 ms). They are first estimates: expect to retune them after step 3.
* Smoke test only: ran on a generated 5k-word file in the scratchpad, 22 of 22 within limits, word counts matched. The "keystroke at middle, 2560x1440" row was 4.82 ms against the 5 ms limit; the earlier UX session saw this cloud container run 2560x1440 cases slower than the dev machine, so a failure there on step 3 may be the container, not the code. Re-checked after the CSV change: the same row read 4.6, 6.3, 6.9, 7.7 and 9.6 ms across runs on the 5k file, so it is noisy here regardless of size. Consider giving that row its own limit (the old gate uses 12 ms for its 4K stress case) once step 3 gives real numbers.

### Step 3: run on the generated files (DONE, 2026-10-09, cloud container, release build)

Ran `large-file-check` on all four files: 88 measurements, 0 correctness problems (Rust word counts match the Python generator on every file). 85 within limit; 3 over, all keystroke rows, treated as noise by decision (see below). Times in ms:

| Measurement | 50k | 100k | 500k | 1M | Limit |
| ----------- | --- | ---- | ---- | -- | ----- |
| open project (unpack) | 4.6 | 4.7 | 17.2 | 32.5 | 2000 |
| read every chapter | 0.9 | 0.8 | 3.9 | 9.3 | 1000 |
| recount words | 1.9 | 3.9 | 19.6 | 56.5 | 1000 |
| load comments | 0.2 | 0.3 | 1.2 | 3.2 | 200 |
| comments through 200 edits | 0.04 | 0.07 | 0.35 | 1.2 | 50 |
| search, common word | 6.3 | 11.6 | 59.3 | 156.5 | 250 |
| search, no match | 0.1 | 0.2 | 1.3 | 3.7 | 250 |
| hash the whole manuscript | 0.5 | 0.6 | 3.0 | 6.4 | 1000 |
| first snapshot | 10.4 | 16.2 | 79.2 | 151.5 | 5000 |
| incremental snapshot | 1.3 | 2.1 | 5.8 | 8.8 | 500 |
| save after editing one chapter | 10.5 | 15.5 | 84.2 | 132.2 | 1000 |
| export Markdown | 10.8 | 25.5 | 102.1 | 196.0 | 3000 |
| export plain text | 9.3 | 19.4 | 92.6 | 147.3 | 3000 |
| export DOCX | 20.1 | 42.1 | 197.6 | 294.0 | 10000 |
| export EPUB | 17.6 | 33.3 | 175.8 | 246.9 | 10000 |
| load largest chapter (about 4.4k words) | 0.35 | 0.39 | 0.35 | 0.41 | 100 |
| keystroke at chapter start | 1.8 | 3.2 | 3.6 | 2.3 | 5 |
| keystroke at chapter middle | 7.3 F | 1.6 | 1.5 | 2.3 | 5 |
| keystroke at chapter end | 1.1 | 1.1 | 0.6 | 1.1 | 5 |
| keystroke at middle, 2560x1440 | 4.2 | 5.8 F | 4.8 | 7.3 F | 5 |
| scroll step | 2.5 | 1.7 | 2.0 | 1.9 | 16.7 |

Findings:
* Everything that scales with book size is linear and far inside its limit at 1M words. Nothing needs fixing.
* Closest to its limit: search for a common word across the whole manuscript, 156 ms of 250 at 1M words (63%). It stays under the limit, but it is the first thing to look at if the book size goal grows or the search runs per keystroke. The benchmark searches chapter by chapter with a 500-match cap per chapter; check how the app's Find panel actually calls it before drawing conclusions.
* The three failures are noise: `keystroke at middle 2560x1440` (5.8 and 7.3 ms, the known container noise) and `keystroke at chapter middle` on the 50k file (7.3 ms, the first project in the process, so cold caches; the same row is 1.5 to 2.3 ms on the other three). Keystroke latency does not grow with book size.
* Limitation: the editor rows use the largest single chapter (about 4.5k words in all four files), so they show that editing is independent of manuscript size, not how a chapter of 100k+ words behaves. The older `performance-check` already covers 1 MB, 100 MB and 10 MB single-line documents.
* Not measured: PDF export, spell check, window startup, a whole manuscript opened as a single document.
* If this gate is kept in CI or run regularly: give the 2560x1440 keystroke row its own limit (about 12 ms) and run one throwaway warm-up project first.
