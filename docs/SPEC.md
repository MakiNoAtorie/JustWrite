# JustWrite: Build Spec

A native, ultra-light, cross-platform desktop app for writing fiction manuscripts. Windows first,
macOS before 1.0. It is forked from the architecture of *Just a markdown editor*: winit +
cosmic-text + softbuffer CPU rendering with no webview, a rope buffer, operation-based undo, IME
handling, an incremental block index that parses the viewport only, an event-driven idle loop,
atomic saving, autosave, session restore, a settings file, remappable shortcuts, menus, tabs and
splits, focus mode, word count, the performance gate (5 ms per edit, 60 Hz scrolling) and
`--measure-startup`. The Windows-only dialog code is replaced by `rfd`.

## Project structure

* A tree with **Manuscript** (parts and chapters: add, rename, delete, drag to reorder; only this
  is exported) and **Notes & research** (pages).
* One tab per chapter or page; splits are allowed.
* Word counts per chapter and in total, excluding notes. Optional project and session goals.

## Prose formatting

* `*italic*` and `**bold**` with concealed markers; a scene-break line (`###newscene###`, `#`, `***`, …) shows as a
  configurable symbol; a `## Name ##` line names a scene and is left out of exports and word counts.
  Names and hash breaks are locked in the text unless the setting "Edit scene names in the text" is
  on. No headings inside chapters.
* Auto-typography that can be switched off: smart quotes, `--` to an em dash, the Spanish dialogue
  dash.
* A per-project language (English or Spanish) drives spelling, hyphenation and the quote style.

## Notes layer

* A configurable marker (default `$$`): a line note, an inline note `a $$ n $$ b`, a block note
  (lines holding only `$$$`), and a literal `\$$`.
* Notes are stored in a neutral form, so changing the marker updates existing notes.
* A shortcut turns the line, selection or paragraph into a note and back.
* Notes are grey by default; colour and style are settings, and a toggle hides them.

## Review comments

* A highlight in the text plus a side panel. Each comment has a body, a timestamp, a status
  (open or resolved), a tag (plot, continuity, prose, research or custom) and replies. There is no
  author field.
* Filter by status, tag and chapter; clicking a comment jumps to the text.
* Stored outside the text as anchored ranges that follow edits, with the quoted text and its
  context. A comment whose text is deleted becomes *orphaned*; it is never lost.

## Snapshots

* Manual snapshots with a message; optional automatic ones on a timer, when closing and on a
  manual save.
* A History tab in the side panel: the chapters changed since the newest snapshot, and the
  snapshots of the line with compare, restore, rename and delete.
* Content-addressed blobs; restore (whole project or one chapter) is non-destructive and creates a
  new snapshot.
* A compare view between any two snapshots or a snapshot and the current text, with notes and
  comments that can be hidden.
* Named lines that can be switched and compared, never merged.
* Pruning keeps every manual snapshot and thins out old automatic ones.
* Trimming on request: one snapshot per hour, day or week stays, or none; manual snapshots and
  safety copies only when asked. The newest snapshot of a line always stays.

## File format `.jw`

A standard ZIP with `manifest.json` (versioned), the chapter and page text, `comments.json`,
`history/`, `export-presets.json`, `book.json` and optional assets such as the cover. The app edits an
unpacked working copy in its data folder with a change journal and repacks atomically on save, on
close and every few minutes. A crash is recovered from the journal. A lock file warns against
opening the project on two machines. The `.jw` extension is associated with the app on Windows
and macOS. See [FORMAT.md](FORMAT.md).

## Import

`.docx`, `.md` and `.txt` into a new or existing project, split by heading or a chosen pattern,
with a comment marker (for example lines starting with `#`) converted to notes.

## Export

A dialog with:

* **Scope:** the current chapter, selected chapters, the whole manuscript, or any snapshot.
* **Content:** notes stripped (with an option to include them); comments off, as DOCX comments or
  as an appendix; clean whitespace.
* **Matter:** title page, table of contents, chapter titles, the scene-break symbol, the
  chapter-start style, dedication, acknowledgments, about-the-author and copyright pages.
* **Formats:** txt, md, DOCX, PDF and EPUB 3.

Every layout value can be chosen and saved as a preset. Built-in presets:

1. **Standard manuscript:** 12 pt Times New Roman or Courier, double spaced, 1 inch margins,
   ragged right, 0.5 inch indent, a title page with contact details and a rounded word count, a
   running header (surname / short title / page), chapters starting about a third down the page, a
   centred `#` for scene breaks, italics or underline.
2. **Print book:** a typeset PDF; trims 5×8, 5.25×8, 5.5×8.5, 6×9 or custom; mirrored margins with a
   gutter; justified and hyphenated text; running heads and page numbers; chapter-opening styles
   with optional drop caps; chapters start on right-hand pages with blank pages inserted; optional
   bleed; fonts embedded.
3. **EPUB 3:** cover, metadata, navigation, a CSS theme; it must validate.
4. **Plain / Markdown.**

Publishing metadata lives on a *Book details* page. Open-licensed book fonts are bundled, and
system fonts can be used.

## Preview

A paged preview of any export preset, drawn in the app from the same typesetting output as the
PDF. Editor display options: font, size, line height, column width, light and dark themes (the
dark one warm), focus mode and optional typewriter scrolling.

## Settings

Notes marker and style, hide notes, project language, auto-typography, autosave and
auto-snapshot intervals, snapshot pruning, editor appearance, the default export preset and
shortcuts.

## Cloud

Optional and off until a service is connected (File, Connect to cloud…): Nextcloud by WebDAV,
OneDrive (app folder) or Google Drive (files the app made). One service at a time. The local `.jw`
stays the master copy and the app works offline as before.

* The whole `.jw` goes to a `JustWrite` folder in the service, one file per project.
* Upload: after each save of the `.jw`; a linked project is saved 20 s after the writing stops,
  and when the project closes.
* Download: when a linked project opens, when the window gets the focus, and every 60 s. A newer
  cloud version replaces the local file only when nothing local is unsaved.
* Never overwrite: an upload states the cloud version it replaces.
* When both computers changed the project, the cloud version is merged in through the history.
  It is kept whole on a new line (`Cloud <date>`), with the snapshots only that computer had, and
  the text here is saved in a safety snapshot first. The two are compared with the newest snapshot
  both have: a chapter changed on one side is taken from that side; a chapter changed on both is
  put together paragraph by paragraph; where both changed the same paragraph the text here stays
  and the author is told. A chapter list changed on both sides stays as it is here, with the
  chapters added there at its end. Review comments, book details and presets stay as they are
  here. The result is recorded as a snapshot and uploaded.
* A cloud file of the same name that is another project is not touched: the local project goes
  up as `<name> (conflict, <computer>, <date>).jw`.
* The sign-in (app password or refresh token) is in the operating system's secret store. OneDrive
  and Google Drive need the client id of an app registration, in Settings.

## Non-goals

No JustWrite account or server, no collaboration; no plugin system; no merging between lines
(the merge of a cloud version is the one exception).

## Build notes

Keep the editor core, the `.jw` format, history and the exporters as separate modules. The format
carries a version field. The performance gates apply to every new feature, including large
manuscripts and deep history.
