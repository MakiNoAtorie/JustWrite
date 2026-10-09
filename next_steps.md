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
