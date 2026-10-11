//! The phone client: one open project, raw text in and out, and the sync round of `ui/cloud.rs`
//! without a window. The logic is in `Session`; the shell calls the C functions at the end.
use crate::{
    cloud::{self, Ids, Synced},
    core::notes::{self, Marker},
    history::{History, merge, prune},
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
/// Where the device keeps its `.jw` files, inside the data folder.
const PROJECTS: &str = "projects";

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

/// The projects of the cloud folder and those already on this device. With no network, the
/// second ones alone, so the author can go on writing.
pub fn projects(data: &Path) -> Result<Vec<String>, String> {
    let here = fs::read_dir(data.join(PROJECTS)).into_iter().flatten().flatten().filter_map(|entry| entry.file_name().into_string().ok()).filter(|name| name.ends_with(".jw"));
    let mut names: Vec<String> = here.collect();
    match cloud::list(data, &Ids::default()) {
        Ok(cloud) => names.extend(cloud),
        Err(error) if names.is_empty() => return Err(error),
        Err(_) => {}
    }
    names.sort();
    names.dedup();
    Ok(names)
}

impl Session {
    /// Opens the project `name` of the cloud folder and brings it in line with the cloud.
    pub fn open(data: &Path, name: &str) -> Result<Session, String> {
        let mut session = Session::load(data, name)?;
        // No network: the project opens as it is, and the next sync brings it in line.
        let _ = session.sync();
        Ok(session)
    }
    /// Opens the project as it is on the device; the first time, it comes down.
    pub fn load(data: &Path, name: &str) -> Result<Session, String> {
        // The name comes from the server; it must stay inside the folder.
        if Path::new(name).file_name().and_then(|n| n.to_str()) != Some(name) {
            return Err("Cloud: that is not a file name".into());
        }
        let folder = data.join(PROJECTS);
        let jw = folder.join(name);
        if jw.exists() {
            // A new connection drops every link. Linked again, the first sync merges the two.
            let _ = cloud::link(data, &jw);
        } else {
            fs::create_dir_all(&folder).map_err(text)?;
            cloud::fetch(data, &Ids::default(), name, &jw)?;
        }
        let (project, history) = load(data, &jw)?;
        Ok(Session { data: data.to_owned(), project, history })
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
        if self.history.capture_auto(&self.project, "Written on the phone", now).map_err(text)?.is_some() {
            // The project's own rule for old snapshots, as on the desktop: the file goes up whole.
            let _ = prune::prune(&mut self.history, &self.project, now);
        }
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
    reply((|| unsafe { Ok(json!(projects(Path::new(&string(data)?))?)) })())
}
/// Opens a project and returns its chapters; the project that was open is closed.
/// # Safety
/// The arguments are NUL-terminated strings.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn jw_open(data: *const c_char, name: *const c_char) -> *mut c_char {
    reply((|| {
        let (data, name) = unsafe { (string(data)?, string(name)?) };
        let mut slot = SESSION.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        // The list shows at once: the project that is open stays open, and the shell syncs after.
        if let Some(open) = slot.take_if(|open| open.project.path != Path::new(&data).join(PROJECTS).join(&name)) {
            let _ = open.close();
        }
        let session = match &mut *slot {
            Some(session) => session,
            empty => empty.insert(Session::load(Path::new(&data), &name)?),
        };
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
        assert_eq!(projects(phone_data.path()).unwrap(), ["Novel.jw"]);
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
        // With no network the phone still lists what it holds.
        cloud::tests::sign_in_to("http://127.0.0.1:9", phone_data.path());
        assert_eq!(projects(phone_data.path()).unwrap(), ["Novel.jw"]);
        // Connected again, the project it holds is linked again and writing goes on.
        cloud::tests::sign_in_to(&base, phone_data.path());
        let mut phone = Session::open(phone_data.path(), "Novel.jw").unwrap();
        std::thread::sleep(Duration::from_millis(15));
        phone.write(&second, "After the new connection.
").unwrap();
        assert_eq!(phone.sync().unwrap().message, "");
        phone.close().unwrap();
        assert_eq!(desktop_sync(desk_data.path(), &mut desk, &mut desk_history), Synced::Downloaded);
        desk.abandon();
        assert_eq!(load(desk_data.path(), &jw).unwrap().0.read_text(&second).unwrap(), "After the new connection.
");
    }

    /// The phone's path against a real Nextcloud server, with an app password from its web
    /// settings. With `JUSTWRITE_TEST_PROJECT` it opens that project of the cloud folder and reads
    /// it; nothing changes. Without it, a small test project goes up, the phone writes in it, the
    /// desktop side takes the result, and the test project is removed from the server.
    /// `tools\nextcloud-phone-check.ps1` asks for the password and runs this.
    #[test]
    #[ignore]
    fn a_real_nextcloud_takes_the_phone() {
        let var = |name: &str| std::env::var(name).unwrap_or_else(|_| panic!("{name} is not set"));
        let data = tempfile::tempdir().unwrap();
        cloud::tests::no_secret_store();
        println!("{}", cloud::connect_with_password(data.path(), &var("JUSTWRITE_TEST_NEXTCLOUD"), &var("JUSTWRITE_TEST_USER"), &var("JUSTWRITE_TEST_PASSWORD")).unwrap());
        println!("in the cloud folder: {:?}", projects(data.path()).unwrap());
        if let Ok(name) = std::env::var("JUSTWRITE_TEST_PROJECT") {
            let mut phone = Session::open(data.path(), &name).unwrap();
            let tree = phone.tree();
            println!("{tree:#}");
            let text = phone.read(tree[0]["id"].as_str().unwrap()).unwrap();
            println!("{} characters in the first chapter", text.chars().count());
            assert_eq!(phone.sync().unwrap(), Round::default());
            return phone.close().unwrap();
        }

        // The desktop side has the same connection in a data folder of its own.
        let (desk_data, folder) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        fs::copy(data.path().join("cloud.json"), desk_data.path().join("cloud.json")).unwrap();
        let name = format!("JustWrite phone test {}.jw", jw::new_id());
        let jw = folder.path().join(&name);
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let mut desk = Project::create(desk_data.path(), &jw, "Phone test").unwrap();
            let chapter = desk.manifest.tree.manuscript[0].id().to_owned();
            desk.write_text(&chapter, "From the desktop.\n").unwrap();
            let mut desk_history = History::load(&desk.work);
            desk_history.capture(&desk, Kind::Manual, "Start", jw::time::now()).unwrap();
            desk.pack(false).unwrap();
            cloud::link(desk_data.path(), &jw).unwrap();
            assert_eq!(cloud::sync(desk_data.path(), &Ids::default(), &jw).unwrap(), Synced::Uploaded);
            println!("uploaded {name}");

            assert!(projects(data.path()).unwrap().contains(&name));
            let mut phone = Session::open(data.path(), &name).unwrap();
            assert_eq!(phone.read(&chapter).unwrap(), "From the desktop.\n");
            std::thread::sleep(Duration::from_millis(15));
            let written = "From the desktop.\n\nFrom the phone, with a $$ note $$.\n";
            assert_eq!(phone.write(&chapter, written).unwrap(), 8);
            assert_eq!(phone.sync().unwrap(), Round::default());
            phone.close().unwrap();
            println!("the phone wrote and synced");

            assert_eq!(desktop_sync(desk_data.path(), &mut desk, &mut desk_history), Synced::Downloaded);
            desk.abandon();
            let (desk, _) = load(desk_data.path(), &jw).unwrap();
            assert_eq!(notes::to_display(&desk.read_text(&chapter).unwrap(), &Marker::default()).0, written);
            desk.abandon();
            println!("the desktop side has the phone's text");
        }));
        println!("removing {name}: HTTP {}", cloud::tests::remove(data.path(), &name));
        if let Err(panic) = outcome {
            std::panic::resume_unwind(panic);
        }
    }

    // tool: phone timings (open, read, write, sync, merge) on a generated project, CSV | run: cargo test --profile fast --lib phone_timings -- --ignored --nocapture
    /// The cloud is the local test server, so the times are the work on the device without the
    /// network. `JUSTWRITE_BENCH` names the project; the default is `bench_data/bench_100k.jw`
    /// from `tools/gen_bench_jw.py`. Rows also go to `bench_data/phone-timings.csv`.
    #[test]
    #[ignore]
    fn phone_timings() {
        use std::{io::Write, time::Instant};
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let source = std::env::var("JUSTWRITE_BENCH").map(PathBuf::from).unwrap_or_else(|_| root.join("bench_data/bench_100k.jw"));
        let base = cloud::tests::server();
        let (desk_data, folder) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        cloud::tests::sign_in_to(&base, desk_data.path());
        let jw = folder.path().join("Bench.jw");
        fs::copy(&source, &jw).unwrap();
        let (mut desk, mut desk_history) = load(desk_data.path(), &jw).unwrap();
        desk_history.capture(&desk, Kind::Manual, "Start", jw::time::now()).unwrap();
        desk.pack(false).unwrap();
        cloud::link(desk_data.path(), &jw).unwrap();
        assert_eq!(cloud::sync(desk_data.path(), &Ids::default(), &jw).unwrap(), Synced::Uploaded);
        let size_start = fs::metadata(&jw).unwrap().len();

        let pause = || std::thread::sleep(Duration::from_millis(15));
        let ms = |from: Instant| from.elapsed().as_secs_f64() * 1000.;
        let mut rows: Vec<(&str, Vec<f64>)> = Vec::new();

        let mut first = Vec::new();
        for _ in 0..5 {
            let data = tempfile::tempdir().unwrap();
            cloud::tests::sign_in_to(&base, data.path());
            let from = Instant::now();
            let phone = Session::open(data.path(), "Bench.jw").unwrap();
            first.push(ms(from));
            phone.close().unwrap();
        }
        rows.push(("open_first", first));

        let phone_data = tempfile::tempdir().unwrap();
        cloud::tests::sign_in_to(&base, phone_data.path());
        Session::open(phone_data.path(), "Bench.jw").unwrap().close().unwrap();
        let mut again = Vec::new();
        for _ in 0..10 {
            let from = Instant::now();
            let phone = Session::open(phone_data.path(), "Bench.jw").unwrap();
            again.push(ms(from));
            phone.close().unwrap();
        }
        rows.push(("open_again", again));

        let mut phone = Session::open(phone_data.path(), "Bench.jw").unwrap();
        let tree = phone.tree();
        let mut chapters: Vec<(u64, String)> = tree.as_array().unwrap().iter().filter(|item| item["section"] == "manuscript").map(|item| (item["words"].as_u64().unwrap(), item["id"].as_str().unwrap().to_owned())).collect();
        chapters.sort();
        let (words, mine) = chapters.pop().unwrap();
        let (_, theirs) = chapters.pop().unwrap();
        let total: u64 = words + chapters.iter().map(|c| c.0).sum::<u64>();

        let (mut read, mut write, mut idle, mut edited, mut merged) = (Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new());
        let mut text = String::new();
        for round in 0..20 {
            let from = Instant::now();
            text = phone.read(&mine).unwrap();
            read.push(ms(from));
            text.push_str(&format!("

Round {round} of the timing."));
            let from = Instant::now();
            assert!(phone.write(&mine, &text).unwrap() as u64 > words);
            write.push(ms(from));
        }
        for round in 0..10 {
            pause();
            phone.write(&mine, &format!("{text}

Sync {round}.")).unwrap();
            let from = Instant::now();
            assert_eq!(phone.sync().unwrap(), Round::default());
            edited.push(ms(from));
            let from = Instant::now();
            assert_eq!(phone.sync().unwrap(), Round::default());
            idle.push(ms(from));
        }
        assert_eq!(desktop_sync(desk_data.path(), &mut desk, &mut desk_history), Synced::Downloaded);
        desk.abandon();
        (desk, desk_history) = load(desk_data.path(), &jw).unwrap();
        for round in 0..5 {
            pause();
            let their_text = desk.read_text(&theirs).unwrap();
            desk.write_text(&theirs, &format!("{their_text}

Desk {round}.")).unwrap();
            assert_eq!(desktop_sync(desk_data.path(), &mut desk, &mut desk_history), Synced::Uploaded);
            phone.write(&mine, &format!("{text}

Merge {round}.")).unwrap();
            let from = Instant::now();
            assert!(phone.sync().unwrap().changed);
            merged.push(ms(from));
            pause();
            assert_eq!(desktop_sync(desk_data.path(), &mut desk, &mut desk_history), Synced::Downloaded);
            desk.abandon();
            (desk, desk_history) = load(desk_data.path(), &jw).unwrap();
        }
        assert!(desk.read_text(&mine).unwrap().ends_with("Merge 4.") && desk.read_text(&theirs).unwrap().ends_with("Desk 4."));
        rows.extend([("read", read), ("write", write), ("sync_idle", idle), ("sync_after_edit", edited), ("sync_merge", merged)]);
        let size_end = fs::metadata(&jw).unwrap().len();

        let commit = std::process::Command::new("git").args(["rev-parse", "--short", "HEAD"]).output().ok().and_then(|o| String::from_utf8(o.stdout).ok()).unwrap_or_default();
        let lead = format!("{},{},{}", jw::time::now(), commit.trim(), if cfg!(debug_assertions) { "debug" } else { "optimized" });
        let params = format!("{} {total}w chapter {words}w", source.file_name().unwrap().to_string_lossy());
        let mut out = String::new();
        for (name, mut times) in rows {
            times.sort_by(f64::total_cmp);
            let at = |q: f64| times[((times.len() as f64 * q).ceil() as usize).clamp(1, times.len()) - 1];
            out.push_str(&format!("{lead},{name},{params},{},ms,{:.1},{:.1},{:.1},{:.1},{:.1}
", times.len(), at(0.5), at(0.95), at(0.99), times[0], times[times.len() - 1]));
        }
        for (name, bytes) in [("jw_size_start", size_start), ("jw_size_after_15_syncs", size_end)] {
            out.push_str(&format!("{lead},{name},{params},1,bytes,{bytes},{bytes},{bytes},{bytes},{bytes}
"));
        }
        let header = "run_id,commit,build,name,params,n,unit,median,p95,p99,min,max
";
        print!("{header}{out}");
        let log = root.join("bench_data/phone-timings.csv");
        let fresh = !log.exists();
        if let Ok(mut file) = fs::OpenOptions::new().create(true).append(true).open(log) {
            let _ = file.write_all(format!("{}{out}", if fresh { header } else { "" }).as_bytes());
        }
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
