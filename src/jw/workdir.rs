//! The unpacked working copy of a project, with a change journal.
//!
//! Edits go to files in the working copy as they happen, each written atomically and noted in the
//! journal. The `.jw` is repacked from it on save, on close and every few minutes. After a crash
//! the journal shows that the working copy holds changes the `.jw` does not, and they are
//! recovered instead of lost.

use super::archive::{self, PackState, Previous, Stamp};
use super::manifest::{MANIFEST, Manifest, new_id};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    fs::{self, OpenOptions},
    io::{self, Write},
    path::{Component, Path, PathBuf},
};

const JOURNAL: &str = ".journal";
const PACKED: &str = ".packed.json";
const ORIGIN: &str = ".origin.json";

#[derive(Serialize, Deserialize)]
struct Origin {
    path: PathBuf,
}

#[derive(Debug)]
pub enum PackError {
    /// The `.jw` was changed by something else since this program last wrote it.
    ExternalChange,
    Io(io::Error),
}
impl std::fmt::Display for PackError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ExternalChange => write!(f, "The .jw file was changed outside JustWrite since it was last saved"),
            Self::Io(e) => write!(f, "{e}"),
        }
    }
}
impl From<io::Error> for PackError {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}

#[derive(Clone, Debug)]
pub struct WorkDir {
    root: PathBuf,
}

/// Only paths inside the project are allowed.
pub fn valid(rel: &str) -> bool {
    !rel.is_empty()
        && Path::new(rel).components().all(|c| matches!(c, Component::Normal(_)))
        && !rel.starts_with('.')
        && !rel.contains("/.")
        && !rel.contains('\\')
}

impl WorkDir {
    pub fn at(root: PathBuf) -> Self {
        Self { root }
    }
    pub fn root(&self) -> &Path {
        &self.root
    }
    pub fn path(&self, rel: &str) -> io::Result<PathBuf> {
        if !valid(rel) {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, format!("Not a project path: {rel}")));
        }
        Ok(self.root.join(rel))
    }
    pub fn read(&self, rel: &str) -> io::Result<Vec<u8>> {
        fs::read(self.path(rel)?)
    }
    pub fn read_string(&self, rel: &str) -> io::Result<String> {
        String::from_utf8(self.read(rel)?).map_err(|_| io::Error::new(io::ErrorKind::InvalidData, format!("{rel} is not UTF-8")))
    }
    pub fn exists(&self, rel: &str) -> bool {
        self.path(rel).is_ok_and(|p| p.exists())
    }
    /// Atomically replaces a file and notes the change in the journal.
    pub fn write(&self, rel: &str, bytes: &[u8]) -> io::Result<()> {
        let target = self.path(rel)?;
        let parent = target.parent().ok_or_else(|| io::Error::other("no parent"))?;
        fs::create_dir_all(parent)?;
        let mut temporary = tempfile::Builder::new().prefix(".w-").suffix(".tmp").tempfile_in(parent)?;
        temporary.write_all(bytes)?;
        temporary.as_file().sync_all()?;
        archive::persist(temporary, &target)?;
        self.journal("write", rel);
        Ok(())
    }
    /// Writes a content-addressed file once; an existing file of that name is already correct.
    pub fn write_once(&self, rel: &str, bytes: &[u8]) -> io::Result<bool> {
        if self.path(rel)?.exists() {
            return Ok(false);
        }
        self.write(rel, bytes)?;
        Ok(true)
    }
    /// Appends to a file (creating it); for logs that only grow. A write cut short by a crash
    /// leaves at most one broken last line.
    pub fn append(&self, rel: &str, bytes: &[u8]) -> io::Result<()> {
        let target = self.path(rel)?;
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut file = OpenOptions::new().create(true).append(true).open(target)?;
        file.write_all(bytes)?;
        self.journal("write", rel);
        Ok(())
    }
    pub fn list(&self, rel: &str) -> io::Result<Vec<String>> {
        let mut names = Vec::new();
        for entry in fs::read_dir(self.path(rel)?)? {
            names.push(entry?.file_name().to_string_lossy().into_owned());
        }
        names.sort();
        Ok(names)
    }
    pub fn remove(&self, rel: &str) -> io::Result<()> {
        let target = self.path(rel)?;
        let result = if target.is_dir() { fs::remove_dir_all(&target) } else { fs::remove_file(&target) };
        match result {
            Ok(()) => {
                self.journal("remove", rel);
                Ok(())
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e),
        }
    }
    pub fn journal(&self, op: &str, rel: &str) {
        let line = serde_json::json!({ "t": super::time::now(), "op": op, "path": rel });
        if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(self.root.join(JOURNAL)) {
            let _ = writeln!(file, "{line}");
        }
    }
    /// Files changed since the last pack, according to the journal.
    pub fn dirty(&self) -> BTreeSet<String> {
        let mut dirty = BTreeSet::new();
        let Ok(text) = fs::read_to_string(self.root.join(JOURNAL)) else { return dirty };
        for line in text.lines() {
            // A line cut off by a crash is ignored.
            let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else { continue };
            match value["op"].as_str() {
                Some("pack") => dirty.clear(),
                Some("write" | "remove") => {
                    if let Some(path) = value["path"].as_str() {
                        dirty.insert(path.to_owned());
                    }
                }
                _ => {}
            }
        }
        dirty
    }
    pub fn is_dirty(&self) -> bool {
        !self.dirty().is_empty()
    }
    pub fn pack_state(&self) -> Option<PackState> {
        serde_json::from_slice(&fs::read(self.root.join(PACKED)).ok()?).ok()
    }
    fn save_pack_state(&self, state: &PackState) -> io::Result<()> {
        fs::write(self.root.join(PACKED), serde_json::to_vec(state).map_err(io::Error::other)?)
    }
    fn origin(&self) -> Option<PathBuf> {
        serde_json::from_slice::<Origin>(&fs::read(self.root.join(ORIGIN)).ok()?).ok().map(|o| o.path)
    }
    fn set_origin(&self, path: &Path) -> io::Result<()> {
        fs::write(self.root.join(ORIGIN), serde_json::to_vec(&Origin { path: path.to_owned() }).map_err(io::Error::other)?)
    }

    /// Repacks the `.jw`. Fails with [`PackError::ExternalChange`] if the file on disk is not the
    /// one this working copy last wrote, unless `overwrite` is set.
    pub fn pack(&self, target: &Path, overwrite: bool) -> Result<(), PackError> {
        let _writing = archive::WRITING.lock();
        let previous_state = self.pack_state();
        if !overwrite && target.exists() {
            if let Some(state) = &previous_state {
                if archive::stamp(target).ok() != Some(state.jw) && self.origin().as_deref() == Some(target) {
                    return Err(PackError::ExternalChange);
                }
            }
        }
        let dirty = self.dirty();
        let reuse = previous_state.as_ref().filter(|_| target.exists() && self.origin().as_deref() == Some(target));
        let state = archive::pack(
            &self.root,
            target,
            reuse.map(|state| Previous { jw: target, state, dirty: &dirty }),
        )?;
        self.save_pack_state(&state)?;
        self.set_origin(target)?;
        // The journal starts over: everything in it is now in the archive.
        fs::write(self.root.join(JOURNAL), format!("{}\n", serde_json::json!({ "t": super::time::now(), "op": "pack" })))?;
        Ok(())
    }
}

/// What opening found in a working copy left behind by an earlier session.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Recovered {
    /// Files holding changes that were never packed into the `.jw`.
    pub unsaved: BTreeSet<String>,
    /// The `.jw` also changed on disk meanwhile (another computer, a sync).
    pub also_changed_outside: bool,
}

pub struct Opened {
    pub work: WorkDir,
    pub manifest: Manifest,
    pub recovered: Option<Recovered>,
    /// The `.jw` had the same project id as another file open on this computer; it was given a
    /// new identity so the two do not share a working copy.
    pub duplicate: bool,
}

fn work_root(data: &Path, id: &str) -> PathBuf {
    data.join("work").join(id)
}
fn same_file(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    }
}

/// Opens (or recovers) the working copy for `jw` below the application data folder `data`.
pub fn open(data: &Path, jw: &Path) -> Result<Opened, String> {
    let mut manifest = archive::read_manifest(jw)?;
    let mut root = work_root(data, &manifest.id);
    let mut duplicate = false;
    if root.exists() {
        let work = WorkDir::at(root.clone());
        let origin = work.origin();
        if let Some(origin) = origin.as_ref().filter(|o| !same_file(o, jw)) {
            if origin.exists() && !work.is_dirty() || origin.exists() && work.is_dirty() {
                // Another file with this identity is (or was) open here: this one is a copy.
                duplicate = true;
                manifest.id = new_id();
                root = work_root(data, &manifest.id);
            }
        }
        if !duplicate {
            let dirty = work.dirty();
            let now = archive::stamp(jw).map_err(|e| e.to_string())?;
            let packed = work.pack_state();
            let changed_outside = packed.as_ref().is_none_or(|p| p.jw != now);
            if !dirty.is_empty() {
                work.set_origin(jw).map_err(|e| e.to_string())?;
                let manifest = Manifest::parse(&work.read(MANIFEST).map_err(|e| e.to_string())?)?;
                return Ok(Opened {
                    work,
                    manifest,
                    recovered: Some(Recovered { unsaved: dirty, also_changed_outside: changed_outside }),
                    duplicate: false,
                });
            }
            if !changed_outside {
                work.set_origin(jw).map_err(|e| e.to_string())?;
                let manifest = Manifest::parse(&work.read(MANIFEST).map_err(|e| e.to_string())?)?;
                return Ok(Opened { work, manifest, recovered: None, duplicate: false });
            }
            // The file changed while nothing here was pending: take the file's version.
            fs::remove_dir_all(&root).map_err(|e| e.to_string())?;
        }
    }
    let state = archive::unpack(jw, &root).map_err(|e| format!("Cannot unpack {}: {e}", jw.display()))?;
    let work = WorkDir::at(root);
    if duplicate {
        work.write(MANIFEST, manifest.to_json().as_bytes()).map_err(|e| e.to_string())?;
    } else {
        fs::write(work.root.join(PACKED), serde_json::to_vec(&state).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
        fs::write(work.root.join(JOURNAL), format!("{}\n", serde_json::json!({ "t": super::time::now(), "op": "pack" }))).map_err(|e| e.to_string())?;
    }
    work.set_origin(jw).map_err(|e| e.to_string())?;
    Ok(Opened { work, manifest, recovered: None, duplicate })
}

/// A new working copy for a project that has no `.jw` yet.
pub fn create(data: &Path, manifest: &Manifest) -> io::Result<WorkDir> {
    let root = work_root(data, &manifest.id);
    fs::create_dir_all(&root)?;
    let work = WorkDir::at(root);
    work.write(MANIFEST, manifest.to_json().as_bytes())?;
    Ok(work)
}

/// Working copies whose `.jw` no longer exists and that hold no unsaved changes can be deleted.
pub fn clean_unused(data: &Path) -> usize {
    let mut removed = 0;
    let Ok(entries) = fs::read_dir(data.join("work")) else { return 0 };
    for entry in entries.flatten() {
        let work = WorkDir::at(entry.path());
        if !work.is_dirty() && work.origin().is_none_or(|o| !o.exists()) && fs::remove_dir_all(entry.path()).is_ok() {
            removed += 1;
        }
    }
    removed
}

pub fn stamp_of(path: &Path) -> Option<Stamp> {
    archive::stamp(path).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make(data: &Path, jw: &Path) -> (WorkDir, Manifest) {
        let manifest = Manifest::new("Novel");
        let work = create(data, &manifest).unwrap();
        work.write("chapters/one.txt", b"first draft").unwrap();
        work.pack(jw, false).unwrap();
        (work, manifest)
    }

    #[test]
    fn paths_are_confined_to_the_project() {
        let dir = tempfile::tempdir().unwrap();
        let work = WorkDir::at(dir.path().to_owned());
        for bad in ["", "../x", "/abs", ".journal", "a/../../b", "a\\b", "chapters/.hidden"] {
            assert!(work.write(bad, b"x").is_err(), "{bad:?}");
        }
        work.write("chapters/ok.txt", b"x").unwrap();
        assert_eq!(work.read_string("chapters/ok.txt").unwrap(), "x");
        assert!(work.dirty().contains("chapters/ok.txt"));
    }

    #[test]
    fn reopening_a_clean_project_reuses_the_working_copy() {
        let data = tempfile::tempdir().unwrap();
        let out = tempfile::tempdir().unwrap();
        let jw = out.path().join("book.jw");
        let (work, manifest) = make(data.path(), &jw);
        assert!(!work.is_dirty());
        let opened = open(data.path(), &jw).unwrap();
        assert!(opened.recovered.is_none() && !opened.duplicate);
        assert_eq!(opened.manifest.id, manifest.id);
        assert_eq!(opened.work.read_string("chapters/one.txt").unwrap(), "first draft");
    }

    #[test]
    fn a_crash_leaves_changes_that_are_recovered() {
        let data = tempfile::tempdir().unwrap();
        let out = tempfile::tempdir().unwrap();
        let jw = out.path().join("book.jw");
        let (work, _) = make(data.path(), &jw);
        // Typing after the last pack, then the app dies without packing.
        work.write("chapters/one.txt", b"second draft, never packed").unwrap();
        work.write("chapters/two.txt", b"a new chapter").unwrap();
        drop(work);
        let opened = open(data.path(), &jw).unwrap();
        let recovered = opened.recovered.expect("recovery expected");
        assert!(recovered.unsaved.contains("chapters/one.txt") && recovered.unsaved.contains("chapters/two.txt"));
        assert!(!recovered.also_changed_outside);
        assert_eq!(opened.work.read_string("chapters/one.txt").unwrap(), "second draft, never packed");
        // Packing makes the recovered state permanent and clears the journal.
        opened.work.pack(&jw, false).unwrap();
        assert!(!opened.work.is_dirty());
        let fresh = tempfile::tempdir().unwrap();
        let again = open(fresh.path(), &jw).unwrap();
        assert_eq!(again.work.read_string("chapters/two.txt").unwrap(), "a new chapter");
    }

    #[test]
    fn a_torn_journal_line_is_ignored() {
        let data = tempfile::tempdir().unwrap();
        let out = tempfile::tempdir().unwrap();
        let jw = out.path().join("book.jw");
        let (work, _) = make(data.path(), &jw);
        work.write("chapters/one.txt", b"x").unwrap();
        let mut file = OpenOptions::new().append(true).open(work.root().join(JOURNAL)).unwrap();
        write!(file, "{{\"op\":\"write\",\"pa").unwrap();
        assert_eq!(work.dirty().len(), 1);
    }

    #[test]
    fn a_file_changed_elsewhere_is_picked_up_when_nothing_is_pending() {
        let data = tempfile::tempdir().unwrap();
        let other = tempfile::tempdir().unwrap();
        let out = tempfile::tempdir().unwrap();
        let jw = out.path().join("book.jw");
        let (_, manifest) = make(data.path(), &jw);
        // Another computer edits and saves the same project.
        let theirs = open(other.path(), &jw).unwrap();
        theirs.work.write("chapters/one.txt", b"edited elsewhere").unwrap();
        std::thread::sleep(std::time::Duration::from_millis(15));
        theirs.work.pack(&jw, false).unwrap();
        let opened = open(data.path(), &jw).unwrap();
        assert!(opened.recovered.is_none());
        assert_eq!(opened.manifest.id, manifest.id);
        assert_eq!(opened.work.read_string("chapters/one.txt").unwrap(), "edited elsewhere");
    }

    #[test]
    fn conflicting_changes_are_reported_not_merged() {
        let data = tempfile::tempdir().unwrap();
        let other = tempfile::tempdir().unwrap();
        let out = tempfile::tempdir().unwrap();
        let jw = out.path().join("book.jw");
        let (work, _) = make(data.path(), &jw);
        work.write("chapters/one.txt", b"my unsaved edit").unwrap();
        let theirs = open(other.path(), &jw).unwrap();
        theirs.work.write("chapters/one.txt", b"their edit").unwrap();
        std::thread::sleep(std::time::Duration::from_millis(15));
        theirs.work.pack(&jw, false).unwrap();
        // Packing from here must not silently overwrite their file.
        assert!(matches!(work.pack(&jw, false), Err(PackError::ExternalChange)));
        let opened = open(data.path(), &jw).unwrap();
        let recovered = opened.recovered.unwrap();
        assert!(recovered.also_changed_outside);
        assert_eq!(opened.work.read_string("chapters/one.txt").unwrap(), "my unsaved edit");
        // The user can choose to overwrite.
        opened.work.pack(&jw, true).unwrap();
    }

    #[test]
    fn a_copied_file_gets_its_own_identity() {
        let data = tempfile::tempdir().unwrap();
        let out = tempfile::tempdir().unwrap();
        let jw = out.path().join("book.jw");
        let copy = out.path().join("book copy.jw");
        let (_, manifest) = make(data.path(), &jw);
        fs::copy(&jw, &copy).unwrap();
        let opened = open(data.path(), &copy).unwrap();
        assert!(opened.duplicate);
        assert_ne!(opened.manifest.id, manifest.id);
        // The original is untouched.
        let original = open(data.path(), &jw).unwrap();
        assert_eq!(original.manifest.id, manifest.id);
        assert!(!original.duplicate);
    }

    #[test]
    fn unused_working_copies_are_cleaned_up_but_unsaved_ones_kept() {
        let data = tempfile::tempdir().unwrap();
        let out = tempfile::tempdir().unwrap();
        let jw = out.path().join("book.jw");
        let (work, _) = make(data.path(), &jw);
        fs::remove_file(&jw).unwrap();
        work.write("chapters/one.txt", b"unsaved").unwrap();
        assert_eq!(clean_unused(data.path()), 0);
        work.pack(&out.path().join("elsewhere.jw"), false).unwrap();
        fs::remove_file(out.path().join("elsewhere.jw")).unwrap();
        assert_eq!(clean_unused(data.path()), 1);
    }
}
