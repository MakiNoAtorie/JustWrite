# iOS port plan

Research result for `main`, commit `bb95429` (2026-10-11), which holds cloud saving. Nothing was built or run
for this document; items marked **verify** are facts about third-party crates or tools that the
first phase must confirm.

## Status (2026-10-11, branch `ios-port`)

Everything that does not need a Mac is written. The next step needs the Mac; see
"With the Mac" below.

| Part | State |
| ---- | ----- |
| Phase 0, library for iOS | Done. Compiled and linked for `aarch64-apple-ios` in the CI. |
| Phase 1, `src/mobile.rs` | Done. Tested on Windows against the local test server (two devices, merge, download, comment anchor, no network, new connection). |
| Real Nextcloud run | Done on 2026-10-11 against the real server, with an app password: connect, a test project up, the phone opens it, writes and syncs, the desktop side takes the text, the test project is removed. `tools/nextcloud-phone-check.ps1` asks for the password and runs it. The cloud folder of the account was empty: save a project to the cloud from the desktop before the phone is used. |
| Phase 2, the shell | Written in `ios/` (about 380 lines of Swift, `project.yml`, `build-rust.sh`). It compiles and links in the CI. **Never run**: no simulator, no device. |
| CI | The repository is public since 2026-10-11, so the jobs run. The `ios` job in `.github/workflows/ci.yml` builds the app for the device without signing and passes: the Rust library compiles for `aarch64-apple-ios` (`ring` included), the Swift files compile, the app links. The `Performance gate` step fails on the Windows and macOS runners: the editor drawing times at 2560x1440 are over the limits there. It passes on this computer and the editor code is the same as on `main`; the shared runners are slower. |

Changes to the first design, all in the text below: `keyring` needs its own store on iOS; no
`crate-type` line; one open project in a global, not a pointer; the note marker is always `$$`
(the marker is an app setting of the desktop, not part of the project); the phone keeps its
`.jw` files in `projects/` inside the data folder; the project list also works with no network.

## Scope

A personal iPhone client for one user. It does one thing: write chapter text in a project that
syncs with the desktop through the Nextcloud connection that already works.

* **In:** connect to Nextcloud, pick a project, list chapters and notes, edit raw text (`*`, `**`,
  `$$` shown as typed), add and rename a chapter, word count, sync, merge.
* **Out:** export, import, preview, concealed markers, review comments, history screens, splits,
  tabs, menus, settings forms, spelling from `zspell`, OneDrive and Google Drive, App Store.

## Verdict

**Small and low risk: about 2 to 3 weeks.** Do not port the desktop interface. Build a thin native
shell (SwiftUI, one list and one system text view) over the Rust library.

The earlier draft of this plan advised against a native editor, because it would have meant
writing concealed markers, note styling and comment highlights a second time. Raw text removes that
reason. A system text view then gives, with no code: the soft keyboard, autocorrect, accents,
dictation, selection handles, undo, system spelling and smart punctuation. The hard problems of the
earlier draft (text input through winit, touch mapping, CPU rendering on the device, a phone layout
for the desktop interface) are gone.

No existing iOS editor can stand in: a `.jw` is a ZIP, and the merge needs the project history.

## What is reused, what is left out

| Module | Lines | iOS |
| ------ | ----- | --- |
| `jw` (project, archive, working copy, journal, lock) | ~2,100 | Reused as it is. |
| `history` (store, `merge.rs`) | ~1,400 | Reused. `merge_file` is the cloud merge and has no interface code. |
| `cloud.rs` | 1,163 | Reused: `list`, `fetch`, `link`, `sync`, `accept`. Sign-in needs a small addition (below). |
| `core/notes.rs` | ~900 | Reused: `to_display` / `to_neutral` between the stored note characters and `$$`, and `word_count`. |
| `ui`, `render`, `export`, `import`, `fonts`, `core/spell`, `core/storage` | ~17,000 | Not compiled for iOS. |
| `platform` | ~290 | Split: `secret` and `machine_name` stay; dialogs, clipboard, fonts, window code are desktop only. |

Dependencies not compiled for iOS: `winit`, `softbuffer`, `cosmic-text`, `typst*`, `rfd`, `arboard`,
`notify`, `zspell`. That also removes most of the 34 MB binary.

## Design

```
SwiftUI shell (~300-400 lines)
  ConnectView      server, user, app password (once)
  ProjectsView     names from the cloud folder
  ChaptersView     manuscript and notes, word counts, "+"
  EditorView       TextEditor with the raw text
        |  C ABI, about 10 functions, strings and JSON
Rust static library (this crate, iOS target)
  src/mobile.rs    session: open, read, write, sync, merge (~250 lines, new)
  jw, history, cloud, core/notes   (existing)
```

### The Rust side: `src/mobile.rs`

One session in a global behind a lock: one project is open at a time. The functions that reach
the cloud take the data folder from the caller (the app's `Library/Application Support`), as
`Project::open` and `cloud::sync` already do, so `platform::data_directory` is not needed. The
`.jw` files of the phone are in `projects/` in that folder.

| Function | Does |
| -------- | ---- |
| `connect(data, server, user, password)` | New `cloud::connect_with_password`: stores the secret, makes the `JustWrite` folder, writes `cloud.json`. |
| `projects(data)` | `cloud::list` and the projects already on the phone. With no network: those alone. |
| `open(data, name)` | `cloud::fetch` if the phone does not hold the file (else `cloud::link`, for the case of a new connection), `Project::open` with `force`, then `sync`; a sync that fails does not stop the open. Closes the project that was open. |
| `tree()` | JSON: id, title, section, words. |
| `read(id)` | `read_text` then `notes::to_display` with the `$$` marker. |
| `write(id, text)` | `notes::to_neutral` then `write_text`; returns the word count. |
| `add_chapter`, `rename` | `Project::add`, `Project::rename`. |
| `sync()` | The round below; returns what happened so the shell can reload the text or show a message. |
| `close()` | `sync`, then `Project::close`. |

**The sync round** is the logic of `ui/cloud.rs:150-260` without the window:

1. Take an automatic snapshot if the text changed since the last one (`History::capture_auto`). This
   keeps the newest snapshot both devices have recent, which is what `merge_file` compares against.
   Then remove old snapshots by the project's rule (`prune`), as the desktop does.
2. `Project::pack`, then `cloud::sync`.
3. `Downloaded`: open the project again. `Incoming(file, tag)`: `merge_file`, `cloud::accept`, pack,
   sync again (three rounds at most). `Conflict(name)`: report it.

The desktop keeps its own copy of this logic for now. Share one function only if the two start to
drift.

Plain C ABI (`extern "C"`, strings in, one `jw_free`). Every answer is a JSON string, `{"ok": …}`
or `{"error": "…"}`. The header is `ios/Sources/justwrite.h`. Ten functions do not justify a
binding generator; move to UniFFI if the surface grows.

### Sign-in

`nextcloud_login` (`cloud.rs:393`) opens a browser and polls for three minutes; on iOS the app is
suspended while the browser is in front. For one user the short path is an **app password** made
once in the Nextcloud web settings and pasted into `ConnectView`. `connect_with_password` is about
15 lines: the account record, `store_secret` and the `MKCOL` call are already there. It does not
take the old app password back on the server: that password is the user's.

### The shell

* The editor is a `TextEditor` bound to a string. On pause in typing (two seconds) and on leaving
  the chapter: `write`.
* The editor takes no text while a sync runs: a merge can replace the text of the open chapter. A
  dead network ends the sync after 15 seconds (`timeout_resolve`, `timeout_connect`).
* All calls into the library run one at a time on one background queue.
* App goes to the background: `write`, then `sync`, inside a background task (iOS gives about 30 s).
  If the upload does not finish, the next round sends it; if the desktop changed the project in
  between, that is the merge case the code already handles.
* App comes to the front: `sync`. Nothing is unsaved at that point, so a download is safe; reload the
  open chapter if the round says the text changed.
* A merge that kept the local paragraph over the cloud one shows the same message as the desktop.
* The same code runs on an iPad with no extra work; no iPad layout is planned.

## Things to check (each is small)

| Item | Why | If it fails |
| ---- | --- | ----------- |
| `keyring` 4 on iOS | `platform::secret` holds the app password. Checked: `keyring` has no default store on iOS, so `platform::secret` uses `apple-native-keyring-store` (`protected`) through `keyring-core` there. It compiles; the Keychain call itself is proven only on a device (**verify** in Phase 2). | Keychain call in Swift (about 20 lines), password passed to `connect` each launch. |
| `ureq` with `rustls` + `platform-verifier` on iOS | All cloud traffic. Compiles and links for iOS in the CI. The certificate check itself needs a device (**verify**). | Switch the iOS build to bundled web roots. |
| `jiff` `tz-system` on iOS | Names of cloud lines and conflict files use local time. Compiles for iOS. The shell sets `TZ` to the zone name at launch; that `jiff` then finds the zone data on a device is not proven (**verify**). | Accept UTC. |
| `platform::process_alive` | On iOS it cannot run `kill`, so it answers "alive". iOS kills apps often, so a stale lock from the app itself would warn on every open. | Done: `mobile.rs` always opens with `force`; the lock only guards the app's own private copy. |
| `platform::machine_name` | Falls through to "this computer". | Done: "iPhone" on iOS. Used in the lock and in conflict file names. |
| Comments after a phone edit | The phone does not show comments, but its edits move the text they point at. Read in the code: the desktop finds the anchors again when a chapter tab opens (`ui/project.rs:692`) and in a merge (`history/restore.rs:35`, called from `merge.rs`). The Phase 1 test proves it for a phone edit and a merge. | One `reanchor_item` call in `write`. |
| Size of the `.jw` | The whole file, history included, goes up on every sync, on mobile data. Measured on generated projects: 0.54 MB at 100,000 words, 5.3 MB at 1,000,000; each sync after an edit adds one copy of the edited chapter (about 19 KB for 4,400 words) until `prune` removes it. | Nothing now. Measure with the real project first. |

## The build problem

iOS apps are built and signed with Xcode on a Mac. This is the only real obstacle from a Windows
machine. Options, best first:

1. **A Mac (owned, borrowed, or rented by the hour).** Needed for a few days in Phase 2, and for a
   short rebuild after that. A free Apple ID signs a build that runs 7 days; the paid programme
   (99 USD per year) makes it 1 year.
2. **No Mac:** build an unsigned `.ipa` on the GitHub Actions macOS runner (the workflow already has
   one), then sign and install it from Windows with a sideload tool and a free Apple ID, renewed every
   7 days (**verify**). It works for installing; it is a poor way to debug, because there is no
   console and no simulator. Acceptable only because most of the logic is tested on Windows first.

A third path came up and is in use: the `ios` job of the CI builds the app for the device, not
signed, on the GitHub macOS runner. It proves the build with no Mac. It cannot sign, install or
run the app.

Phases 0 and 1 are done and tested on Windows.

### With the Mac

1. Once: `rustup target add aarch64-apple-ios aarch64-apple-ios-sim`, `brew install xcodegen`.
2. `cd ios && xcodegen`, open `JustWrite.xcodeproj`, choose the team under Signing.
3. Build. The build step `build-rust.sh` makes `libjustwrite.a` for the device or the simulator.
   The device build is proven in the CI. The simulator build (`aarch64-apple-ios-sim`) is not.
   The app has never run: expect faults in how the screens behave, not in the build.
4. On the device, the **verify** rows above: the Keychain keeps the app password after a restart,
   HTTPS reaches the server, cloud line names show local time.
5. The three checks of Phase 2.

## Phases

### Phase 0. The library compiles for iOS (done)

* In `lib.rs`, compile `ui`, `render`, `export`, `import`, `fonts` only when the target is not iOS.
  Move their dependencies under `[target.'cfg(not(target_os = "ios"))'.dependencies]`.
* Split `platform`; let the compiler name what else must be gated (expected: `core/storage`,
  `core/spell`, `core/config`).
* Gated in the end: `ui`, `render`, `export`, `import`, `fonts`, and in `core`: `config`, `spell`,
  `storage`, `workspace`. `platform/desktop.rs` holds the window, font, dialog and clipboard code.
* The CI has an `ios` job that builds the whole app (see "The build problem"). It passes.

### Phase 1. `mobile.rs` (done)

* The session, the sync round, `connect_with_password`, the C ABI. No `crate-type` line: it would make every
  desktop release build link the library twice. The shell build runs
  `cargo rustc --lib --release --target aarch64-apple-ios --crate-type staticlib`.
* Tests run on the desktop against the local test server that `cloud.rs` tests already use: open,
  write, sync, a second "device" writes, sync gives a merge; and the comment anchor check.
* The run against the real Nextcloud server is done (see Status): `tools/nextcloud-phone-check.ps1`.
  The phone needs no new logic, only a screen.
* Timings: `cargo test --profile fast --lib phone_timings -- --ignored --nocapture` (CSV, also
  appended to `bench_data/phone-timings.csv`). On the Windows desktop, with the local test server
  as the cloud, median in ms for 100,000 words / 1,000,000 words: first open 59 / 450, open again
  4 / 20, read 2 / 2, write 3 / 3, sync with nothing to do 4 / 15, sync after an edit 29 / 96,
  sync with a merge 229 / 1,604. A phone is slower and adds the network; nothing here needs work.
  The one change from this pass is `prune` in the sync round.

### Phase 2. The shell (written, builds in the CI, never run; needs the Mac)

* Written: `ios/project.yml` (XcodeGen makes the Xcode project), `ios/build-rust.sh`, and in
  `ios/Sources`: `Core.swift` (the calls), `JustWriteApp.swift` (connect, project list),
  `Project.swift` (chapter list, editor, background and foreground hooks). The Keychain is
  reached from Rust; no Swift code for it unless the **verify** row fails.
* To do: "With the Mac" above.
* Done when: write a paragraph on the phone, lock it, open the desktop, the paragraph is there; and
  the same in the other direction; and both edited while the phone was offline gives one merge
  message and no lost text.

### Phase 3. Use it for a week

Add only what gets in the way. Candidates, none planned: a toolbar key for `$$` and `*`, a font
choice, a dark theme check, the Spanish dialogue dash, a manual snapshot button.

## What not to do

* Do not run the desktop interface on the phone (winit, softbuffer, cosmic-text).
* Do not write a second implementation of the format, the sync decision or the merge in Swift.
* Do not sync through the Files app or the Nextcloud app's file provider: `cloud.rs` already does
  the transfer with version checks, and the file provider gives no such guarantee.
