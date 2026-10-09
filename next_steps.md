# Next steps / working memory

This file doubles as the running memory for the large-file performance work. Update it at the end of every step.

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

### Step 2: timing benchmarks and speed tests (TODO, waiting for go-ahead)

Plan:
* Extend `src/bin/performance-check.rs`, or add a bin or test next to it, that loads `bench_data/*.jw` (skip with a clear message if missing).
* Keep output tiny: one line per measurement, plus a JSON report. No per-iteration prints.
* Cover: open and unpack, `Manifest::parse`, read every chapter, word counts, comment load and mapping, search, history capture and compare, Markdown export, edit and scroll latency at the end of a big chapter.
* Give each a limit in ms, with a pass/fail exit code.

### Step 3: run the benchmarks on the generated files (TODO, after step 2)

* Record the results in this file under a new "Results" section, and note any regressions or bottlenecks found.
