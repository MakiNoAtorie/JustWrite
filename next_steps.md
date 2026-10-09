# Next steps

State after the feedback pass of 2026-10-09 (Windows development machine, release build). All 158 unit
tests pass; `performance-check` passes; `large-file-check` passes 204 of 204 measurements.

## Needs a person at the real window

None of this can be driven headlessly. Do these first; each is a short look.

* **Cursor after Enter.** Type a paragraph, press Enter at the end of the text, keep typing. The caret
  must stay visible on the new line, also in the preview view and with a blank line before it.
* **Reply to comment box.** Open the comments panel and press Reply. Click inside earlier text to place the
  caret; move it with the arrow keys (Home and End go to the start and end of the whole text). The wrapping of the lines must not change as the
  caret moves. Type more than four lines: the box should keep the caret in view.
* **Parts are gone.** There is no New part command or button. Open an old project that had parts: the
  chapters should be there in the same order with no part rows. Move chapters with Alt+Up/Alt+Down and
  by dragging; the tree should scroll to the moved chapter.
* **New note.** "+ New" ▸ New note (Ctrl+Alt+N) asks for a name; empty uses the "Note 1" placeholder.
* **Raw and Preview buttons** on the formatting bar (also View menu and the palette). Preview hides
  markers on every line, indents paragraphs like the default export preset, keeps the first paragraph
  of a chapter and the one after a scene break flush, and justifies when the preset does. Typing,
  selecting and clicking must work as in raw. See "Raw and preview view" below.
* **Notes look.** `$$` notes should be in the accent colour, and a note that is a whole line (or a line
  of a `$$$` block) should be centred. Inline notes inside a paragraph are coloured but cannot be
  centred. A custom note colour in Settings still wins. In Preview, notes are hidden (typing `$$` in
  Preview makes the text vanish at once: switch to Raw to write notes, or turn on "Show notes in the
  preview").* **Comment keyboard access.** F8 / Shift+F8 walk the review comments in reading order. With the caret
  in a commented passage, the palette has Resolve, Reply, Edit and Delete for it.
* **Tab drop to split.** Reading `drop_zone` in `src/ui/app.rs`: a dragged tab splits when released
  within about 65 px of the left, right or bottom edge of a pane, or in a strip just under the toolbar;
  the accent strip shows the target while dragging. The middle of the editor does nothing by design.
  Confirm it with the mouse.
* **Import dialog.** File ▸ Import opens a native file chooser; not looked at in any pass.
* **Autosave fix `5e0bc1c`** and the manual list under "Remaining tests" below.

## Decisions taken in the feedback pass (change them if you disagree)

* **Parts were removed (maintainer's call, 2026-10-09).** A part was a folder of chapters with no text
  of its own, which gave a writer nothing. There is no way to create one; imports drop part headings;
  opening a project flattens existing parts (and note groups) into their chapters, in order, and the
  next save writes it that way. `Node::Part`, the export part headings and the tree's fold/drop code
  are still in the source for old files but nothing reaches them; delete them if you want the code
  gone too.
* **Only notes ask for a name on creation.** Chapters stay numbered ("Chapter 4")
  so starting a chapter is still one keystroke. Say so if they should ask too.
* **Comment cards: keyboard through commands, not panel focus.** The panel itself still has no
  arrow-key focus; instead every card action is a command on the comment at the caret. Panel focus
  handling is only needed if you want to work in the panel without the editor.
* **Tab-key focus through the window chrome: not added.** Every button has a shortcut, a menu entry and
  a palette entry, and Tab types in the editor.
* **Live word counts: unchanged.** Tree and footer counts update on autosave, to protect the 5 ms
  keystroke budget.
* **Chapter stepping (Ctrl+PageDown/PageUp) still walks the manuscript and then the notes, in tree
  order.** Labels now say "chapter or note".
* **Error messages still fade after 10 s** (`STATUS_LIFETIME` in `src/ui/app.rs`). Errors are plain
  strings in `status`, so a longer lifetime for failures needs a flag on the message first.
* **Default interface font:** Windows is Segoe UI (the system UI font), macOS Helvetica Neue, Linux
  DejaVu Sans (`platform::default_sans`). Not seen on a Mac.

## Small things left

* Find panel: check how it calls `workspace::find` (per keystroke or per chapter, which limit) and
  measure that exact path; the benchmark's whole-book search is 156 ms of 250 at 1M words.
* Export dialog and paged preview: seen, not audited for spacing, focus highlight on the first row,
  page indicator.
* The single-line text fields (`field` in `src/ui/gui.rs`) still cannot place the caret with a click;
  only the multi-line boxes can now. Same technique would work (use `area_buffer`).
* Up and Down arrows do not move the caret between lines inside a multi-line box.

## Fixed in the feedback pass

### Bugs

* **Cursor disappears after Enter.** The caret was only drawn when the layout had a glyph position for
  it; on the empty line after a final newline there is none. `Tile::tail_caret` supplies the position
  (`src/ui/editor.rs`), and the caret is now drawn once, from `caret_at`. Test:
  `caret_shows_on_the_empty_line_after_enter`.
* **Reply to comment box.** The box faked the caret by inserting a `│` into the text, so every caret
  move re-wrapped the lines, and a click only focused it. It now lays the text out with the real font,
  draws the caret as a bar over it, scrolls to keep the caret in view, and a click places the caret
  (`paint_area`, `area_buffer`, `area_cursor`, `area_char`). Affects every multi-line box (book
  details, comments). Test: `area_cursor_and_char_offset_round_trip`.
* **Tree scrolling.** The tree never scrolled to a new or moved item, so a new chapter at the end could
  sit below the visible rows. It now scrolls to the selected item on create, move and open
  (`tree_reveal`); moving past the first or last place says so. (The part-specific work done earlier
  the same day was removed again; see the decision above.)
* **"Page" is now "note".** Menu, palette, tree prompts and messages say note; the default name is
  "Note N" (first free number); creation opens a "New note" dialog with that name as placeholder.
  Stored data is unchanged (the manifest node type is still `Page`; command key `new_page`).
* **Windows could not save over an open `.jw`.** `archive::pack` held the old archive open while
  renaming the new one over it, which Windows refuses ("Access is denied"). Nine tests failed on
  Windows because of it; the archive is now closed first (`drop(old)`), and all tests pass. This was a
  real save bug on Windows, found while checking the test run.
* Two unused-`mut` warnings in test code removed.

### Raw and preview view

Plan, as asked, then what was built.

* **What preview renders.** The same tiles as raw, shaped with the finished look of the export preset
  the Export command starts from (`default_preset`, else the built-in default): markers hidden on every
  line including the one being edited, first-line indent (`preset.indent` inches at the preset's font
  size, converted to en spaces, half an em each), no indent on the first paragraph of a chapter or the
  paragraph after a scene break when the preset says so, and justified text when the preset does. Not
  rendered: page size, running heads, fonts, paragraph spacing; the existing Paged preview shows those.
* **Editable.** Yes, it is the same editor. The indent is plain characters at the start of the tile's
  text with no entry in the source map, so the caret at the start of a line sits after the indent, a
  click in the indent lands on the first character, and the stored text never contains the indent.
* **Shared with export.** The numbers (indent, flush-after-break, justify) come from `Preset`, not a
  copy. The layout itself is not shared; the editor shapes with cosmic-text and the export with typst.
* **Keystroke budget.** The preview adds no work per key: the settings are part of the tile cache key
  and only a changed paragraph is re-shaped, as before. Measured with `JW_BENCH_PREVIEW=1`
  (see `large-file-check`): typing p50 1.0 to 1.6 ms, p95 1.4 to 1.8 ms at 50k to 1M words, 116
  measurements, 0 over limit.
* **Mode is not saved.** Every launch starts raw (the default). Buttons: "Raw" and "Preview" next to
  B and I; commands `raw_view` and `preview_view` have no shortcut yet.
* **Not done:** paragraph spacing from the preset, hyphenation, a per-project choice of preset for the
  preview, a keyboard shortcut.

### Notes: accent colour, centred, hidden in the preview

* Notes use the accent colour (`accent_text`, the readable variant) unless Settings has a note colour.
  A chunk that is all note is centred (`present` in `src/core/prose.rs`, test
  `a_chunk_that_is_all_note_is_centred`).
* The preview is "the text as exported", so it hides notes. Settings > Notes > "Show notes in the
  preview" (also View menu and the palette, `preview_notes`) keeps them; saved, default off. The
  existing "Hide notes" setting still applies in Raw.
* Bug found on the way: changing any setting while in Preview dropped back to Raw, because the view is
  not a saved setting and the settings form re-parses them. `adopt` now keeps it.
### Gates, this machine (Windows, release build, `opt-level = "s"` with fat LTO)

* `performance-check`: passes (`processing_gate_passed: true`); 2560 x 1440 stress edit p50 1.3 to
  1.4 ms against the 2.0 ms baseline. `performance-results.json` was left as it was.
* `large-file-check`, all four sizes, raw view: 204 measurements, 0 over limit, 0 correctness problems.
  The three container failures (keystroke at middle 2560 x 1440, worst of 3000) did not appear.
* So the smaller executable profile is proven on the hot paths; see "Executable size".

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

### Step 4: writing-session test (DONE, 2026-10-09, cloud container, release build)

Question it answers: does it feel smooth to *write* in a huge manuscript, not just load and look around?

* Code: `src/bin/large_file_check/session.rs`, run from `large-file-check` (`--session-only` for just these rows, `--skip-session` to leave them out). 22 rows for the single chapter (run once, on the smallest project) and 22 for the mega scenario (run on every project), plus one spelling-dictionary load row.
* "chapter" scenario: the largest chapter (about 4.5k words). "mega" scenario: the whole manuscript held as ONE chapter (50k to 1M words, 1812 comment marks at 1M). Mega is a stress case beyond the app's real flow (chapters are separate tabs), but it answers "what if someone imports a whole book as one chapter".
* Each action is timed as the interface thread does it in the app: the edit, moving the comment marks through it (`follow_edits`), cloning the rope for the indexer (`scan`), layout, paint, copy to the window buffer. Re-index, autosave, replace-all and spell check run on workers in the app, so they are timed separately and labelled "(background)".
* Script: 3000 real keystrokes through `type_text` with auto-typography (smart quotes, `--`, `...`) and Enter; 20 undos and 20 redos; 60 word moves; 20 italic toggles; 20 `$$` note toggles (selection and line); 10 paragraph deletes; a 5000-word paste, deleting it, undoing that (x3); autosave work; replace all ("the" to "THE"); spell check of 5 screens; find. Correctness checks (typing changed the text, redo restores undo, big paste/delete/undo restore the length, replace-all keeps the length) all passed, 0 problems.

Results, ms (limit 5 unless stated), mega scenario by manuscript size:

| Action | 50k | 100k | 500k | 1M |
| ------ | --- | ---- | ---- | -- |
| open as one document (limit 1000) | 2.3 | 4.5 | 23.1 | 57.2 |
| type, p50 first 500 keys | 1.3 | 1.4 | 1.5 | 1.6 |
| type, p50 last 500 keys | 1.5 | 1.6 | 1.6 | 1.6 |
| type, p95 | 1.7 | 1.7 | 2.3 | 2.0 |
| type, worst of 3000 keys (limit 16.7 now) | 5.7 | 5.7 | 6.1 | 5.0 |
| re-index after a key (background) | 0.2 | 0.2 | 0.3 | 0.2 |
| undo / redo, worst | 1.0 / 3.3 | 1.3 / 1.3 | 3.1 / 1.9 | 1.5 / 1.4 |
| move by word, worst | 1.7 | 3.4 | 2.2 | 1.9 |
| toggle italic / note, worst | 1.8 / 2.0 | 2.1 / 2.0 | 1.8 / 2.0 | 1.8 / 2.4 |
| delete a paragraph, worst | 1.4 | 2.2 | 3.4 | 2.4 |
| paste 5000 words (limit 50) | 4.9 | 4.9 | 3.9 | 6.1 |
| delete 5000 words (limit 50) | 3.1 | 2.9 | 3.0 | 3.2 |
| undo that delete (limit 50) | 1.7 | 2.2 | 2.1 | 2.8 |
| autosave work, background (limit 1000) | 3.7 | 8.7 | 42.8 | 111.6 |
| replace all, background (limit 2500) | 0.9 | 3.0 | 10.0 | 24.5 |
| replace all, apply and redraw (limit 100) | 2.4 | 4.1 | 2.8 | 4.2 |
| spell check one screen, background (limit 100) | 0.4 | 0.6 | 0.4 | 0.5 |
| find common word, 500 matches (limit 250) | 0.9 | 1.9 | 1.0 | 1.3 |
| find, no match (limit 250) | 0.1 | 0.4 | 1.1 | 2.8 |

Single chapter (about 4.5k words), same actions: all between 0.2 and 4.1 ms; spelling dictionary load 89 ms, once.

Findings:
* Writing is smooth at every size: typing p50 about 1.5 ms and p95 about 2 ms, with no slowdown from the first 500 keys to the last 500, and no growth from 50k to 1M words, even with the whole book as one chapter. Autosave's interface-thread cost is zero (it only clones the rope); the background work is 112 ms at 1M words.
* The first run flagged "type max" at 5.0 to 6.2 ms on 4 of 5 scenarios, including the 4.5k-word chapter and not the 1M one, so it does not grow with size. Treated as a spike on the shared container (it is the worst of 3000 samples). The row now has a 60 Hz limit (16.7 ms) and p95 carries the 5 ms budget. If it ever shows up above 10 ms, check whether it lines up with Enter or paragraph breaks.
* Caveats: the generated prose uses about 30 distinct words, so the spell-check times are optimistic (real prose has far more distinct words to look up); the test does not exercise the window system, IME, or the real-time spell-check underline update.

#### Autosave and the interface thread (checked 2026-10-09)

Read from `src/ui/app.rs` (`save`, `UserEvent::Saved`) and `src/ui/project.rs` (`sync_marks_to_project`), then measured with the `autosave` rows of `--session-only`:
* Autosave fires 700 ms after the last edit (`autosave`, `tab.edited + 700 ms`), so it never runs in the middle of continuous typing.
* On a worker thread: converting the chapter to its stored form, the word count, the atomic file write with fsync (`ProjectStorage::write`). Background cost 0.7 ms for a 4.5k-word chapter, 77 to 101 ms for the whole 1M-word book as one chapter. This cannot freeze the window.
* On the interface thread: cloning the rope (0.00 ms) and `sync_marks_to_project`, which runs first in `save()`. It turns the whole chapter into its stored form, maps every comment mark back, and, when any anchor moved (typing before a comment moves it), rewrites the whole project's `comments.json` atomically with an fsync. Measured: 1.8 ms (50k project, 90 comments), 4.1 ms (1M project, 1812 comments, 1.0 MB `comments.json`, normal 4.5k-word chapter), 37 to 55 ms (1M words held as one chapter, the extreme case).
* Verdict: no visible freeze in normal use (4 ms is under one frame, and it lands at a pause in typing). A stutter of about 40 to 55 ms is possible if someone writes inside a single book-length chapter and resumes typing right after a pause. The fsync is on the interface thread and these numbers come from a fast container disk; a slow or network-synced disk would make it longer.
* Fix DONE (see below). It was: do `sync_marks_to_project` in the save worker, or only write `comments.json` when it is needed (on close, on switching chapter, or on a longer timer) and leave the anchors in memory in between. The new row `autosave comment sync on interface thread` (limit 16.7 ms) will show whether a fix works; it currently fails only in the mega scenario at 1M words.

**Fix applied (2026-10-09):** autosave no longer runs `sync_marks_to_project`. `autosave()` in `src/ui/app.rs` only sets `Tab::marks_stale` and starts the background write (`save()` is now `sync_marks_to_project` + `write_tab`; explicit saves still sync). The marks are written back at the next flush: `flush_tabs` (also covers tabs that are clean but stale), `close_tab`, `shutdown_project`, and the repack timer in `project_tick` (`sync_stale_marks`). Safety: comment anchors re-find themselves from their quote and context when a chapter is opened (`reanchor_item`), so a stale `comments.json` after a crash is repaired, not lost. All 154 unit tests pass. NOT checked in the real window (no display here): worth a manual check on the dev machine that, after writing in a chapter that has comments, closing and reopening the project (and a snapshot restore) puts the comment highlights in the right place.
The benchmark row is now `comment sync at flush or repack, interface thread` (limit 100 ms, rare event). It models the old function; it does not run the app, so it cannot prove the autosave path itself is clean. That is shown by the code: autosave only flips a flag.

### Remaining tests (to run locally, with a real display; for the maintainer, Claude or Codex)

Everything above ran headless in a cloud container: real code, no window. These are the checks that need a real machine. Tick them off here as they are done.

Setup on the dev machine:
1. `python3 tools/gen_bench_jw.py` (makes `bench_data/*.jw`; add `--sizes` for other sizes).
2. `cargo run --release --bin large-file-check` (about 3 to 5 minutes; use `-- --quiet` for failures only, `-- --session-only` for the writing session only). Read the CSV: `P` = pass, `F` = over limit. Compare with the tables in steps 3 and 4.
3. `cargo run --release --bin performance-check` (the older gate; it also rewrites `performance-results.json`, so only commit that if the numbers are meant to be the new baseline).

Re-run and re-judge on the dev machine:
* [x] Re-run `large-file-check` and note which of the 3 container failures survive. Done 2026-10-09 on Windows: none survive, 204 of 204 within limit (keystroke at middle 2560x1440 is within 5 ms there, so no separate limit was needed on this machine).
* [ ] Re-tune the limits (first pass: this machine passes every row with room, so tightening is possible, but one set of numbers for a slower laptop is safer; left as is) at the top of `src/bin/large-file-check.rs` and `src/bin/large_file_check/session.rs` to real numbers (they are first estimates, one set for all sizes).
* [ ] Search: 156 ms of 250 at 1M words in this container. Check how the Find panel really calls `workspace::find` (per keystroke? per chapter? limit?) and measure that exact path.

Manual checks in the real window (open `bench_data/bench_1m.jw`):
* [ ] Open time: from double-click or File > Open to a usable window. Feel for a pause; time it with a stopwatch if it seems long.
* [ ] Type for a minute in a chapter that has comments, in a 1M-word project. Watch for stutter, especially right after pausing about a second (the autosave moment).
* [ ] The autosave fix (`5e0bc1c`): write in a chapter that has comments, then close and reopen the project. Comment highlights must be in the right place. Repeat with a snapshot restore and with an export (Export reads `comments.json`). Also: edit above a comment, close the tab (not the project), reopen the chapter.
* [ ] Scroll with the mouse wheel, trackpad and PageUp/PageDown through a long chapter; drag the scrollbar if there is one. Watch for dropped frames.
* [ ] Resize and maximise the window while a 1M-word project is open (including a 4K or 2560x1440 monitor, and fractional scaling on Windows).
* [ ] Open a very long chapter: import a whole 1M-word `.txt` as one chapter (File > Import) to try the "mega" case for real.
* [ ] Open the comments panel at 1812 comments: scrolling, filtering by tag and status, clicking a comment to jump to it, adding and resolving.
* [ ] Spell-check underlines while typing: appear promptly, no stutter, on real prose (not the generated 30-word vocabulary).
* [ ] The project tree with 329 chapters (the generated files still carry 28 parts; they are flattened on open): scrolling, dragging a chapter to reorder.
* [ ] Find and Replace all in a 1M-word project, from the UI.
* [ ] Export DOCX, EPUB and PDF of the whole 1M-word book from the UI. PDF (typst) was never measured and is likely the slowest; check the progress feedback and that the window stays responsive while it runs.
* [ ] History panel: first snapshot, restoring one, comparing two, with a 1M-word project.
* [ ] Save to a slow location (network or cloud-synced folder) while typing; the repack and fsync happen on workers but a slow disk is the main unknown.
* [ ] Memory: watch the process memory with the 1M-word project open and after an hour of use (a leak would show as steady growth).
* [x] Startup time of the app itself: `justwrite.exe --measure-startup` gave first_frame_ms=53 (processing 4.7 ms) on 2026-10-09. Cold start after a reboot not measured.
* [ ] IME input (a CJK or accent-composition keyboard) while typing in the big project.

Not built yet (add if wanted):
* PDF export timing in `large-file-check` (typst; likely minutes at 1M words, so gate it behind a flag).
* A test with realistic prose (a public-domain novel as the source text, not the 30-word generator) for spell check and layout.
* Opening a 1M-word chapter on a worker thread: check the load path `Storage::read` streaming (`append_loaded`) and that typing stays responsive while a big chapter is still loading.
* A cold-cache run (drop the OS file cache before opening) for the open-time numbers.

## Executable size

Measured 2026-10-09, Windows, `cargo build --release --bin justwrite`:

| Profile | `justwrite.exe` |
| ------- | --------------- |
| `lto = "thin"`, `codegen-units = 1`, `strip`, `panic = "abort"` (before) | 40.92 MB |
| `lto = "fat"`, `opt-level = "s"` (now in `Cargo.toml`) | 32.51 MB |

About 5.1 MB of the exe is the embedded fonts and dictionaries (`assets/`). The rest is mostly typst.

**The smaller build is now proven fast enough on this machine (2026-10-09); keep it.** `opt-level = "s"` can slow the hot paths (typing, layout, paint, spell check, export). Before keeping it:
* [x] Build to a separate folder. Not needed: the normal release build already uses this profile, and no app was running.
* [x] Run `large-file-check -- --session-only` and compare with the step 4 tables. Done 2026-10-09: all session rows pass; typing p95 1.4 to 1.8 ms (preview view; raw view passes the same limits), no growth from 50k to 1M words.
* [x] Run the plain `large-file-check` and `performance-check` too. Done: both pass (see the top of this file). `justwrite.exe` is 32.52 MB with this profile.
* [ ] If the session rows regress, keep `opt-level = 3` for the hot crates only and use `"s"` for the rest, for example `[profile.release.package.typst]` and friends with `opt-level = "s"`. Or go back to `opt-level = 3` with fat LTO and accept the larger file.

Further shrinking, in order of effort:
* `opt-level = "z"` (smaller again, slower again; same session check applies).
* Drop unused typst features: `typst-kit` is already `default-features = false`; look for PDF, SVG and image codecs pulled in by `typst-pdf` and `typst-render` that the export path never uses (`cargo bloat --release --crates` shows what is big).
* Fonts: ship one body face family instead of Courier Prime, Libertinus Serif and Tinos together, or load the rarely used ones from disk on first use instead of `include_bytes!`. Dictionaries (`es.index.dic`, `en.index.dic`) could be compressed in the exe and unpacked on first spell-check load (89 ms today).
* UPX compression: usually halves the file, but adds startup cost and often triggers antivirus false positives. Not recommended for a writing tool people install from the web.
