//! The phone client: one open project, raw text in and out, and the sync round of `ui/cloud.rs`
//! without a window. The logic is in `Session`; the shell calls the C functions at the end.
use crate::{
    cloud::{self, Ids, Synced},
    core::notes::{self, Marker},
    history::{History, merge},
    jw::{self, Node, Project, Section, project::OpenError},
};
use serde_json::{Value, json};
use std::{
    ffi::{CStr, CString, c_char},
    fs,
    path::{Path, PathBuf},
    sync::Mutex,
};

/// Rounds in one sync: a merge needs a second one to go up, and the cloud can change in between.
const ROUNDS: usize = 3;

pub struct Session {
    data: PathBuf,
    project: Project,
    history: History,
}

/// What a sync did.
#[derive(Debug, Default, PartialEq)]
pub struct Round {
    /// The text or the chapter list changed under the shell: load them again.
    pub changed: bool,
    /// For the author; empty when there is nothing to say.
    pub message: String,
}

fn text(error: impl ToString) -> String {
    error.to_string()
}

// The lock guards only this device's own copy, and iOS ends the app without a release: take it.
fn load(data: &Path, jw: &Path) -> Result<(Project, History), String> {
    let opened = Project::open(data, jw, true).map_err(|e| match e {
        OpenError::Invalid(message) => message,
        OpenError::Locked(conflict) => conflict.message(jw::time::now()),
    })?;
    let history = History::load(&opened.project.work);
    Ok((opened.project, history))
}

impl Session {
    /// Opens the project `name` of the cloud folder; the first time, it comes down.
    pub fn open(data: &Path, name: &str) -> Result<Session, String> {
        // The name comes from the server; it must stay inside the folder.
        if Path::new(name).file_name().and_then(|n| n.to_str()) != Some(name) {
            return Err("Cloud: that is not a file name".into());
        }
        let folder = data.join("projects");
        let jw = folder.join(name);
        if !jw.exists() {
            fs::create_dir_all(&folder).map_err(text)?;
            cloud::fetch(data, &Ids::default(), name, &jw)?;
        }
        let (project, history) = load(data, &jw)?;
        let mut session = Session { data: data.to_owned(), project, history };
        // No network: the project opens as it is, and the next sync brings it in line.
        let _ = session.sync();
        Ok(session)
    }

    /// Chapters and pages in reading order: id, title, section, part, words.
    pub fn tree(&self) -> Value {
        let rows = |section, name: &str| {
            let leaves = self.project.manifest.tree.leaves(section);
            leaves
                .into_iter()
                .map(|(node, part)| json!({ "id": node.id(), "title": node.title(), "section": name, "part": part.map(Node::title), "words": node.words() }))
                .collect::<Vec<_>>()
        };
        json!([rows(Section::Manuscript, "manuscript"), rows(Section::Notes, "notes")].concat())
    }

    /// The text of a chapter or page as the author types it, notes between `$$`.
    pub fn read(&self, id: &str) -> Result<String, String> {
        Ok(notes::to_display(&self.project.read_text(id).map_err(text)?, &Marker::default()).0)
    }
    /// Stores the text; returns its word count.
    pub fn write(&mut self, id: &str, display: &str) -> Result<u32, String> {
        self.project.write_text(id, &notes::to_neutral(display, &Marker::default()).0).map_err(text)
    }

    /// Adds a chapter at the end of the manuscript; returns its id.
    pub fn add_chapter(&mut self, title: &str) -> Result<String, String> {
        let title = title.trim();
        if title.is_empty() {
            return Err("A title cannot be empty".into());
        }
        let node = Node::chapter(title);
        let id = node.id().to_owned();
        let end = self.project.manifest.tree.manuscript.len();
        self.project.add(Section::Manuscript, &[], end, node)?;
        Ok(id)
    }
    pub fn rename(&mut self, id: &str, title: &str) -> Result<(), String> {
        self.project.rename(id, title)
    }

    /// Brings the project and its cloud file in line.
    pub fn sync(&mut self) -> Result<Round, String> {
        let now = jw::time::now();
        // Keeps the newest snapshot that both devices have recent; a merge compares against it.
        self.history.capture_auto(&self.project, "Written on the phone", now).map_err(text)?;
        let mut round = Round::default();
        for _ in 0..ROUNDS {
            if self.project.is_dirty() {
                self.project.pack(false).map_err(text)?;
            }
            let jw = self.project.path.clone();
            match cloud::sync(&self.data, &Ids::default(), &jw)? {
                Synced::NotLinked => return Err("Cloud: this project is not linked. Open it from the list again.".into()),
                Synced::Nothing | Synced::Uploaded => return Ok(round),
                Synced::Downloaded => {
                    let (project, history) = load(&self.data, &jw)?;
                    std::mem::replace(&mut self.project, project).abandon();
                    self.history = history;
                    round.changed = true;
                    return Ok(round);
                }
                Synced::Incoming(file, tag) => {
                    let merged = merge::merge_file(&mut self.history, &mut self.project, &file, now).and_then(|report| cloud::accept(&self.data, &jw, tag).map(|()| report));
                    let _ = fs::remove_file(&file);
                    let report = merged.map_err(|error| format!("Cloud: the other computer's version could not be merged ({error})"))?;
                    // An empty line: the cloud file is a copy of this project, and this one goes up over it.
                    if !report.line.is_empty() {
                        round.changed = true;
                        round.message = "Cloud: merged the other computer's changes.".into();
                        if !report.conflicts.is_empty() {
                            round.message.push_str(&format!(" Changed in the same place on both, your text kept: {}.", report.conflicts.join(", ")));
                        }
                        if report.lists_differ {
                            round.message.push_str(" The chapter list changed on both; yours was kept.");
                        }
                        round.message.push_str(&format!(" Its version is on the line “{}” in the history.", report.line));
                    }
                }
                Synced::Conflict(name) => {
                    round.message = format!("Cloud: another project has this name in the cloud. This one is now “{name}” there.");
                    return Ok(round);
                }
            }
        }
        Ok(round)
    }

    /// Syncs, then lets the project go. What did not go up goes at the next open.
    pub fn close(mut self) -> Result<(), String> {
        let synced = self.sync().map(drop);
        self.project.close().map_err(text)?;
        synced
    }
}

// ---- the C side --------------------------------------------------------------------------------
//
// Every function takes UTF-8 strings and returns a JSON string, `{"ok": …}` or `{"error": "…"}`,
// that the caller gives back to `jw_free`. `data` is the app's own folder for its files.

// ponytail: one open project. Make this a table of sessions when the shell shows two at once.
static SESSION: Mutex<Option<Session>> = Mutex::new(None);

fn reply(result: Result<Value, String>) -> *mut c_char {
    let value = match result {
        Ok(value) => json!({ "ok": value }),
        Err(error) => json!({ "error": error }),
    };
    // JSON text has no NUL byte.
    CString::new(value.to_string()).unwrap_or_default().into_raw()
}
/// # Safety
/// `pointer` is a NUL-terminated string or null.
unsafe fn string(pointer: *const c_char) -> Result<String, String> {
    if pointer.is_null() {
        return Err("A value is missing".into());
    }
    unsafe { CStr::from_ptr(pointer) }.to_str().map(str::to_owned).map_err(text)
}
fn session<T>(run: impl FnOnce(&mut Session) -> Result<T, String>) -> Result<T, String> {
    run(SESSION.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).as_mut().ok_or("No project is open")?)
}

/// # Safety
/// The arguments are NUL-terminated strings.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn jw_connect(data: *const c_char, server: *const c_char, user: *const c_char, password: *const c_char) -> *mut c_char {
    reply((|| unsafe { Ok(json!(cloud::connect_with_password(Path::new(&string(data)?), &string(server)?, &string(user)?, &string(password)?)?)) })())
}
/// # Safety
/// `data` is a NUL-terminated string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn jw_projects(data: *const c_char) -> *mut c_char {
    reply((|| unsafe { Ok(json!(cloud::list(Path::new(&string(data)?), &Ids::default())?)) })())
}
/// Opens a project and returns its chapters; the project that was open is closed.
/// # Safety
/// The arguments are NUL-terminated strings.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn jw_open(data: *const c_char, name: *const c_char) -> *mut c_char {
    reply((|| {
        let (data, name) = unsafe { (string(data)?, string(name)?) };
        let mut slot = SESSION.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(open) = slot.take() {
            let _ = open.close();
        }
        let session = slot.insert(Session::open(Path::new(&data), &name)?);
        Ok(session.tree())
    })())
}
#[unsafe(no_mangle)]
pub extern "C" fn jw_tree() -> *mut c_char {
    reply(session(|s| Ok(s.tree())))
}
/// # Safety
/// `id` is a NUL-terminated string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn jw_read(id: *const c_char) -> *mut c_char {
    reply(session(|s| Ok(json!(s.read(&unsafe { string(id) }?)?))))
}
/// Returns the word count.
/// # Safety
/// The arguments are NUL-terminated strings.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn jw_write(id: *const c_char, text: *const c_char) -> *mut c_char {
    reply(session(|s| unsafe { Ok(json!(s.write(&string(id)?, &string(text)?)?)) }))
}
/// Returns the id of the new chapter.
/// # Safety
/// `title` is a NUL-terminated string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn jw_add_chapter(title: *const c_char) -> *mut c_char {
    reply(session(|s| Ok(json!(s.add_chapter(&unsafe { string(title) }?)?))))
}
/// # Safety
/// The arguments are NUL-terminated strings.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn jw_rename(id: *const c_char, title: *const c_char) -> *mut c_char {
    reply(session(|s| unsafe { Ok(json!(s.rename(&string(id)?, &string(title)?)?)) }))
}
/// Returns `{"changed": bool, "message": "…"}`.
#[unsafe(no_mangle)]
pub extern "C" fn jw_sync() -> *mut c_char {
    reply(session(|s| s.sync().map(|round| json!({ "changed": round.changed, "message": round.message }))))
}
#[unsafe(no_mangle)]
pub extern "C" fn jw_close() -> *mut c_char {
    let open = SESSION.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).take();
    reply(open.map_or(Ok(()), Session::close).map(|()| Value::Null))
}
/// # Safety
/// `reply` came from a function of this file and is given back once.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn jw_free(reply: *mut c_char) {
    if !reply.is_null() {
        drop(unsafe { CString::from_raw(reply) });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{core::comments::CommentSet, history::Kind, jw::manifest::COMMENTS};
    use std::time::Duration;

    /// The desktop side of a sync with nothing to merge, as `ui/cloud.rs` does it.
    fn desktop_sync(data: &Path, project: &mut Project, history: &mut History) -> Synced {
        history.capture_auto(project, "Automatic snapshot", jw::time::now()).unwrap();
        if project.is_dirty() {
            project.pack(false).unwrap();
        }
        cloud::sync(data, &Ids::default(), &project.path).unwrap()
    }

    #[test]
    fn a_phone_and_a_desktop_write_the_same_project() {
        let base = cloud::tests::server();
        let (desk_data, phone_data, folder) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        cloud::tests::sign_in_to(&base, desk_data.path());
        cloud::tests::sign_in_to(&base, phone_data.path());

        // The desktop makes the project, with a comment on "Two", and sends it up.
        let jw = folder.path().join("Novel.jw");
        let mut desk = Project::create(desk_data.path(), &jw, "Novel").unwrap();
        let chapter = desk.manifest.tree.manuscript[0].id().to_owned();
        let first = "One.\n\nTwo.\n\nThree.\n";
        desk.write_text(&chapter, first).unwrap();
        let mut comments = CommentSet::default();
        let at = first.find("Two").unwrap();
        comments.add(&chapter, first, at..at + 3, "Check this", "", 0).unwrap();
        desk.work.write(COMMENTS, comments.to_json().as_bytes()).unwrap();
        let mut desk_history = History::load(&desk.work);
        desk_history.capture(&desk, Kind::Manual, "Start", jw::time::now()).unwrap();
        desk.pack(false).unwrap();
        cloud::link(desk_data.path(), &jw).unwrap();
        assert_eq!(cloud::sync(desk_data.path(), &Ids::default(), &jw).unwrap(), Synced::Uploaded);

        // The phone opens it and writes in the first paragraph.
        assert_eq!(cloud::list(phone_data.path(), &Ids::default()).unwrap(), ["Novel.jw"]);
        assert!(Session::open(phone_data.path(), "../Novel.jw").is_err());
        let mut phone = Session::open(phone_data.path(), "Novel.jw").unwrap();
        assert_eq!(phone.tree()[0]["id"], chapter.as_str());
        assert_eq!(phone.read(&chapter).unwrap(), first);
        std::thread::sleep(Duration::from_millis(15));
        assert_eq!(phone.write(&chapter, "One, written on the phone.\n\nTwo.\n\nThree.\n").unwrap(), 7);

        // Meanwhile the desktop writes in the third and is first in the cloud.
        desk.write_text(&chapter, "One.\n\nTwo.\n\nThree, at the desk.\n").unwrap();
        assert_eq!(desktop_sync(desk_data.path(), &mut desk, &mut desk_history), Synced::Uploaded);

        // The phone merges, and the result goes up.
        let round = phone.sync().unwrap();
        assert!(round.changed && round.message.starts_with("Cloud: merged the other computer's changes. Its version"), "{round:?}");
        let both = "One, written on the phone.\n\nTwo.\n\nThree, at the desk.\n";
        assert_eq!(phone.read(&chapter).unwrap(), both);
        assert_eq!(phone.sync().unwrap(), Round::default());

        // The desktop takes it, and its comment finds "Two" again.
        assert_eq!(desktop_sync(desk_data.path(), &mut desk, &mut desk_history), Synced::Downloaded);
        desk.abandon();
        let (mut desk, mut desk_history) = load(desk_data.path(), &jw).unwrap();
        assert_eq!(desk.read_text(&chapter).unwrap(), both);
        let mut comments = CommentSet::parse(&desk.work.read(COMMENTS).unwrap());
        comments.reanchor_item(&chapter, both);
        let anchor = comments.comments[0].anchor;
        assert_eq!((both.chars().skip(anchor.start).take(anchor.end - anchor.start).collect::<String>().as_str(), comments.comments[0].orphaned), ("Two", false));

        // A desktop change with nothing new on the phone comes down; a new chapter goes up.
        std::thread::sleep(Duration::from_millis(15));
        desk.write_text(&chapter, "All new.\n").unwrap();
        assert_eq!(desktop_sync(desk_data.path(), &mut desk, &mut desk_history), Synced::Uploaded);
        assert_eq!(phone.sync().unwrap(), Round { changed: true, message: String::new() });
        assert_eq!(phone.read(&chapter).unwrap(), "All new.\n");
        std::thread::sleep(Duration::from_millis(15));
        let second = phone.add_chapter(" Two ").unwrap();
        phone.rename(&second, "The second").unwrap();
        phone.write(&second, "Text $$ a note $$ here.\n").unwrap();
        assert_eq!(phone.read(&second).unwrap(), "Text $$ a note $$ here.\n");
        assert_eq!(phone.tree()[1], json!({ "id": second, "title": "The second", "section": "manuscript", "part": null, "words": 2 }));
        phone.close().unwrap();
        assert_eq!(desktop_sync(desk_data.path(), &mut desk, &mut desk_history), Synced::Downloaded);
    }

    /// The phone's path against a real Nextcloud server, with an app password from its web
    /// settings. Opens the named project of the cloud folder and reads it; changes nothing.
    /// `JUSTWRITE_TEST_NEXTCLOUD=https://… JUSTWRITE_TEST_USER=… JUSTWRITE_TEST_PASSWORD=… JUSTWRITE_TEST_PROJECT=Novel.jw cargo test --lib a_real_nextcloud_takes_the_phone -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn a_real_nextcloud_takes_the_phone() {
        let var = |name: &str| std::env::var(name).unwrap_or_else(|_| panic!("{name} is not set"));
        let data = tempfile::tempdir().unwrap();
        cloud::tests::no_secret_store();
        println!("{}", cloud::connect_with_password(data.path(), &var("JUSTWRITE_TEST_NEXTCLOUD"), &var("JUSTWRITE_TEST_USER"), &var("JUSTWRITE_TEST_PASSWORD")).unwrap());
        println!("{:?}", cloud::list(data.path(), &Ids::default()).unwrap());
        let mut phone = Session::open(data.path(), &var("JUSTWRITE_TEST_PROJECT")).unwrap();
        let tree = phone.tree();
        println!("{tree:#}");
        let text = phone.read(tree[0]["id"].as_str().unwrap()).unwrap();
        println!("{} characters in the first chapter", text.chars().count());
        assert_eq!(phone.sync().unwrap(), Round::default());
        phone.close().unwrap();
    }

    #[test]
    fn the_c_side_answers_in_json() {
        let answer = |pointer: *mut c_char| {
            let value: Value = serde_json::from_str(unsafe { CStr::from_ptr(pointer) }.to_str().unwrap()).unwrap();
            unsafe { jw_free(pointer) };
            value
        };
        assert_eq!(answer(jw_tree()), json!({ "error": "No project is open" }));
        assert_eq!(answer(unsafe { jw_read(std::ptr::null()) }), json!({ "error": "No project is open" }));
        assert_eq!(answer(jw_close()), json!({ "ok": null }));
    }
}
