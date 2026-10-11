# The `.jw` project format (version 1)

A `.jw` file is a standard ZIP archive. Any ZIP tool can open it, and everything inside is plain
text or JSON, so a project can always be read without JustWrite.

```
manifest.json            version, title, the project tree, settings
book.json                book details: author, publisher, ISBN, dedication, ...
comments.json            review comments with their anchors
export-presets.json      export presets the author saved
chapters/<id>.txt        the text of each manuscript chapter
pages/<id>.txt           the text of each note page
assets/cover.jpg|png     the cover image (optional)
history/index.jsonl      one line per snapshot
history/lines.json       named lines and the current one
history/snap/<id>.json   what each snapshot holds (tree, hashes)
history/blobs/<blake3>   the text of every version, stored once
```

## Version and compatibility

`manifest.json` carries `format` (`"justwrite"`) and `format_version` (an integer, `1` today). A
program that finds a `format_version` higher than it knows refuses to open the file instead of
risking damage; unknown fields in the JSON files are ignored and kept.

## Text files

Text is UTF-8. Paragraphs are separated by a line break; there is no markup other than:

* `*italic*` and `**bold**`
* a scene break is a line that holds only `#`, `***`, `* * *` or similar symbols
* a scene may have a name: a line `# The storm` directly under its break. The editor shows it bold
  and centred; exports leave it out
* **notes** are stored with private-use characters, independent of the marker the author types:

  | Character | Meaning |
  | --------- | ------- |
  | U+E000 | starts a note that runs to the end of the line |
  | U+E001 … U+E002 | an inline note |
  | U+E003 | alone on a line, opens a block note |
  | U+E004 | alone on a line, closes it |

  The same table is written into `manifest.json` under `note_tokens`. A literal marker such as
  `$$` that the author wants as text is stored as the plain characters; changing the note marker in
  the settings never rewrites a stored file.

## The project tree

`manifest.json` lists the manuscript and the notes as trees. A `part` holds chapters (or, in the
notes, pages); a `chapter` or `page` names its text file by its id. Only the manuscript is exported.
Each entry keeps its word count so the tree can show totals without reading the files.

## Review comments

`comments.json` stores, for each comment: the chapter or page, the anchor (character offsets in
the stored text), the quoted text with up to 40 characters of context on each side, the body, a
timestamp, a status (`open` or `resolved`), a tag, replies, and an `orphaned` flag. While a chapter is
open the anchors follow every edit; if the commented text is deleted the comment is marked
orphaned and kept. When a chapter changes outside the program the anchor is found again from the
quote and its context.

## History

Every snapshot stores the tree and the hash of each chapter's text; the text itself lives once in
`history/blobs/`, named by its BLAKE3 hash, so a snapshot of an unchanged chapter costs nothing.
Snapshot kinds are `manual`, `auto`, `safety` (taken before a restore, switch or deletion) and
`restore`. A restore never removes anything: it saves what was there, applies the older text, and
records the restore as a new snapshot. Lines (named threads of history) are listed in
`history/lines.json`; they are switched, never merged.

## Working copy, saving and crash recovery

JustWrite edits an unpacked working copy under its data folder (`work/<project id>/`). Each
change to a file is written atomically and noted in `.journal`. The `.jw` is repacked from the
working copy when you save, when you close the project and every few minutes, by writing a temporary file next to it and
renaming it over the original. After a crash the journal shows that the working copy holds changes
the `.jw` does not, and they are recovered when the project is opened again.

A hidden lock file next to the project (`.<name>.jw.lock`) records which computer has it open and
is refreshed every minute. Opening a project that is locked elsewhere shows a clear warning; the
author can still choose to open it.
