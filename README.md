# Just a markdown editor

A portable native Rust Markdown editor for Windows 10/11. It uses winit, cosmic-text and softbuffer to draw on the CPU, with no webview or GUI framework.

## Run

Double-click `dist/Just a markdown editor.exe`, or pass a Markdown file:

```powershell
& './dist/Just a markdown editor.exe' './examples/welcome.md'
```

Everything is reachable with the mouse. The menu bar (File, Edit, Format, View, Help) lists every command with its shortcut and also opens from the keyboard: hold **Alt** to see the underlined letters, press **Alt+F** for File, then use the arrow keys, **Enter** and **Esc**. Right-click opens context menus in the editor, the folder tree and on tabs. The toolbar lights up the formatting at the cursor (bold, italic, lists, heading level), and resting on any button shows a tooltip with its shortcut.

**Ctrl+F** opens a find bar above the text with a live match count, previous/next buttons (**Enter** / **Shift+Enter**), a match-case toggle and every match highlighted; the Replace button expands a replace row. **Ctrl+click** opens a link. **Ctrl+K** wraps the selection as a link and selects the address for typing. **Settings** (Edit menu, **Ctrl+,**) is a dialog with a Light/Dark/System theme, accent swatches, font pickers that preview each font, steppers and toggles. **Edit > Keyboard shortcuts** remaps a command by clicking it and pressing the new keys. Drag the dividers between split panes to resize them, or drag a tab to any pane edge to split there. **File > Open recent folder** reopens earlier workspaces.

**Ctrl+G** goes to a line, **Word count** (View menu) counts the selection or the whole file, and **File > Export as HTML** writes `name.html` beside the Markdown file. Other shortcuts: **Ctrl+O** opens a workspace, **Ctrl+P** or **Ctrl+Shift+L** opens files, **Ctrl+Shift+P** lists commands, **F11** enters focus mode and **Esc** exits.

The session (tabs, splits, panel sizes, recent folders, window position and size) is saved a few seconds after each change, so a crash loses almost nothing.

The editor includes inline preview, clickable tasks, Unicode and composed-input handling, operation-based undo, tabs and nested splits, tab dragging, a folder tree, a collapsible outline, fuzzy navigation, file find/replace and workspace search. Pipe tables are drawn as a grid (monospace, aligned columns, bold header); the row holding the cursor shows its source so you can edit it. Autosave runs after a 700 ms pause; **Ctrl+S** saves immediately. Saving uses atomic replacement and checks for external changes. When a conflict appears, **Ctrl+S** keeps your edits and **Ctrl+R** loads the external version; both commands can be remapped.

`settings.toml` and `session.toml` are created beside the executable. Keep the program in a writable folder. Settings are human-editable TOML and reload when changed. Session state restores the workspace, tabs, splits and cursors, including files saved outside the workspace. Unsaved drafts ask to be saved or discarded on close. Markdown files are the only document storage.

## Build and check

Install stable Rust and Visual Studio C++ Build Tools, then run:

```powershell
./build.ps1 -Test -Benchmark
```

This workspace has a project-local Rust toolchain under `.tools`; the build script detects it. Rust is not needed to run the executable.

The 18 tests cover Unicode and line endings, undo and saved-state tracking, parsing and incremental indexes, shortcut remapping, storage boundaries, split layout, search (case-insensitive by default), menus and config. A hidden native-window test exercises composition events, tab moves, autosave, external-change conflicts, folder-tree updates and session restore against isolated temporary files. It never opens the user's settings or documents.

The release benchmark uses 1 MiB, 100 MiB and 10 MiB single-line fixtures at 1100x760 and 2560x1440. Each case includes a full block index and measures 220 warmed edit and scroll samples through viewport parsing, shaping, CPU rasterization and framebuffer copy. `performance-results.json` records results and fails the gate above 5 ms per edit or a 60 Hz scroll budget. OS presentation, background scanning, display latency and truly cold disk caches are outside that benchmark.

`just-a-markdown-editor --measure-startup` presents a native first frame, prints process-internal startup timing and exits. Idle uses an event-driven wait loop, with no animation or continuous redraw timer.

## Implementation limits

Tables ignore the `:--:` alignment markers, are drawn only when the whole grid fits the pane width (wider ones stay as source) and pad by character count, so wide CJK text may misalign. Preview is bounded to physical lines and 512-character chunks. Inline formatting crossing those boundaries and Setext headings do not have complete CommonMark styling; the original source stays intact. Fenced code state and ATX headings are indexed across the document. Large-file processing measurements are machine-specific and do not certify every possible document or frame.

The platform module includes macOS paths and Cmd shortcuts, but this build has only been compiled and tested on Windows. Actual Japanese/Chinese/Korean OS input-method interaction still needs manual validation; automated tests cover preedit/commit handling. The app icon remains an open item from the spec. Images, math, PDF export, sync and the other excluded v1 features are omitted.
