//! A project in memory: its manifest, book details, working copy and lock, with the operations
//! the application performs on it.

use super::{
    book::BookDetails,
    lock::{Acquired, Conflict, Lock},
    manifest::{ASSETS, BOOK, Manifest, MANIFEST, Node, Section, new_id},
    time,
    workdir::{self, PackError, Recovered, WorkDir},
};
use crate::core::notes;
use std::{
    fs, io,
    path::{Path, PathBuf},
};

pub enum OpenError {
    Invalid(String),
    /// Someone else has the project open; open again with `force` to take it over.
    Locked(Conflict),
}
pub struct Opened {
    pub project: Project,
    pub recovered: Option<Recovered>,
    pub duplicate: bool,
}

pub struct Project {
    pub work: WorkDir,
    pub manifest: Manifest,
    pub book: BookDetails,
    /// The `.jw` file.
    pub path: PathBuf,
    lock: Option<Lock>,
    data: PathBuf,
    /// Manuscript words when this session began.
    pub baseline_words: u64,
}

fn io_message(e: io::Error) -> String {
    e.to_string()
}

impl Project {
    /// Creates a project with one empty chapter and saves it to `jw`.
    pub fn create(data: &Path, jw: &Path, title: &str) -> Result<Project, String> {
        let mut manifest = Manifest::new(title);
        // Off in projects made before the setting existed, so those keep their behaviour.
        manifest.settings.snapshots.on_save = true;
        let first = Node::chapter("Chapter 1");
        manifest.tree.manuscript.push(first.clone());
        let work = workdir::create(data, &manifest).map_err(io_message)?;
        if let Some(file) = first.file() {
            work.write(&file, b"").map_err(io_message)?;
        }
        let lock = match Lock::acquire(jw, false, time::now()).map_err(io_message)? {
            Acquired::Held(lock) => Some(lock),
            Acquired::Conflict(c) => return Err(c.message(time::now())),
        };
        let mut project = Project {
            work,
            manifest,
            book: BookDetails::default(),
            path: jw.to_owned(),
            lock,
            data: data.to_owned(),
            baseline_words: 0,
        };
        project.save_book().map_err(io_message)?;
        project.pack(true).map_err(|e| e.to_string())?;
        Ok(project)
    }

    pub fn open(data: &Path, jw: &Path, force: bool) -> Result<Opened, OpenError> {
        let lock = match Lock::acquire(jw, force, time::now()).map_err(|e| OpenError::Invalid(e.to_string()))? {
            Acquired::Held(lock) => lock,
            Acquired::Conflict(c) => return Err(OpenError::Locked(c)),
        };
        let opened = match workdir::open(data, jw) {
            Ok(o) => o,
            Err(e) => {
                lock.release();
                return Err(OpenError::Invalid(e));
            }
        };
        let book = opened.work.read(BOOK).map(|b| BookDetails::parse(&b)).unwrap_or_default();
        let baseline_words = opened.manifest.tree.manuscript_words();
        Ok(Opened {
            project: Project {
                work: opened.work,
                manifest: opened.manifest,
                book,
                path: jw.to_owned(),
                lock: Some(lock),
                data: data.to_owned(),
                baseline_words,
            },
            recovered: opened.recovered,
            duplicate: opened.duplicate,
        })
    }

    pub fn title(&self) -> &str {
        &self.manifest.title
    }
    pub fn file_of(&self, id: &str) -> Option<String> {
        self.manifest.tree.find(id)?.file()
    }
    /// The id of the chapter or page stored in `file`.
    pub fn id_of_file(&self, file: &str) -> Option<String> {
        let stem = Path::new(file).file_stem()?.to_str()?;
        self.manifest.tree.find(stem).filter(|n| n.file().as_deref() == Some(file)).map(|n| n.id().to_owned())
    }

    pub fn save_manifest(&mut self) -> io::Result<()> {
        self.manifest.modified = time::now();
        self.work.write(MANIFEST, self.manifest.to_json().as_bytes())
    }
    pub fn save_book(&self) -> io::Result<()> {
        self.work.write(BOOK, self.book.to_json().as_bytes())
    }

    pub fn read_text(&self, id: &str) -> io::Result<String> {
        let file = self.file_of(id).ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "No such chapter or page"))?;
        self.work.read_string(&file)
    }
    /// Stores the neutral text of a chapter or page and updates its word count.
    pub fn write_text(&mut self, id: &str, neutral: &str) -> io::Result<u32> {
        let file = self.file_of(id).ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "No such chapter or page"))?;
        self.work.write(&file, neutral.as_bytes())?;
        let words = notes::word_count(neutral) as u32;
        self.set_words(id, words)?;
        Ok(words)
    }
    /// Records a word count; the manifest is only rewritten when it changed.
    pub fn set_words(&mut self, id: &str, count: u32) -> io::Result<()> {
        let changed = match self.manifest.tree.find_mut(id) {
            Some(Node::Chapter { words, .. } | Node::Page { words, .. }) if *words != count => {
                *words = count;
                true
            }
            _ => false,
        };
        if changed { self.save_manifest() } else { Ok(()) }
    }

    /// Adds a chapter, part or page and creates its empty file.
    pub fn add(&mut self, section: Section, parent: &[usize], index: usize, node: Node) -> Result<Vec<usize>, String> {
        let file = node.file();
        let path = self.manifest.tree.insert(section, parent, index, node)?;
        if let Some(file) = file {
            self.work.write(&file, b"").map_err(io_message)?;
        }
        self.save_manifest().map_err(io_message)?;
        Ok(path)
    }
    pub fn rename(&mut self, id: &str, title: &str) -> Result<(), String> {
        let title = title.trim();
        if title.is_empty() {
            return Err("A title cannot be empty".into());
        }
        self.manifest.tree.find_mut(id).ok_or("No such item")?.set_title(title);
        self.save_manifest().map_err(io_message)
    }
    pub fn move_node(&mut self, section: Section, from: &[usize], parent: &[usize], index: usize) -> Result<(), String> {
        self.manifest.tree.move_node(section, from, parent, index)?;
        self.save_manifest().map_err(io_message)
    }
    /// Removes a chapter, page or part (with everything in it) and its files. The history keeps
    /// the text.
    pub fn delete(&mut self, id: &str) -> Result<Node, String> {
        let (section, path) = self.manifest.tree.locate(id).ok_or("No such item")?;
        let node = self.manifest.tree.remove(section, &path).ok_or("No such item")?;
        fn files(node: &Node, out: &mut Vec<String>) {
            out.extend(node.file());
            node.children().iter().for_each(|c| files(c, out));
        }
        let mut doomed = Vec::new();
        files(&node, &mut doomed);
        for file in doomed {
            self.work.remove(&file).map_err(io_message)?;
        }
        self.save_manifest().map_err(io_message)?;
        Ok(node)
    }
    /// Re-reads every chapter and page to refresh the word counts.
    pub fn recount(&mut self) -> io::Result<()> {
        let mut ids = Vec::new();
        for section in [Section::Manuscript, Section::Notes] {
            ids.extend(self.manifest.tree.leaves(section).into_iter().map(|(n, _)| n.id().to_owned()));
        }
        let mut changed = false;
        for id in ids {
            let words = notes::word_count(&self.read_text(&id).unwrap_or_default()) as u32;
            if let Some(Node::Chapter { words: w, .. } | Node::Page { words: w, .. }) = self.manifest.tree.find_mut(&id) {
                changed |= *w != words;
                *w = words;
            }
        }
        if changed { self.save_manifest() } else { Ok(()) }
    }

    pub fn manuscript_words(&self) -> u64 {
        self.manifest.tree.manuscript_words()
    }
    /// Words added to the manuscript since this session began.
    pub fn session_words(&self) -> u64 {
        self.manuscript_words().saturating_sub(self.baseline_words)
    }

    /// Copies an image into the project as the cover.
    pub fn set_cover(&mut self, image: &Path) -> Result<(), String> {
        let extension = image.extension().and_then(|e| e.to_str()).unwrap_or("jpg").to_ascii_lowercase();
        if !["jpg", "jpeg", "png"].contains(&extension.as_str()) {
            return Err("The cover must be a JPEG or PNG image".into());
        }
        let bytes = fs::read(image).map_err(io_message)?;
        if bytes.len() > 20 * 1024 * 1024 {
            return Err("The cover image is larger than 20 MB".into());
        }
        if !self.book.cover.is_empty() {
            let _ = self.work.remove(&self.book.cover);
        }
        let name = format!("{ASSETS}/cover.{extension}");
        self.work.write(&name, &bytes).map_err(io_message)?;
        self.book.cover = name;
        self.save_book().map_err(io_message)
    }
    pub fn cover_bytes(&self) -> Option<Vec<u8>> {
        (!self.book.cover.is_empty()).then(|| self.work.read(&self.book.cover).ok()).flatten()
    }

    /// Repacks the `.jw` from the working copy.
    pub fn pack(&mut self, overwrite: bool) -> Result<(), PackError> {
        self.work.pack(&self.path, overwrite)
    }
    pub fn is_dirty(&self) -> bool {
        self.work.is_dirty()
    }
    /// Shows other computers that this one is still here.
    pub fn heartbeat(&mut self) -> Result<(), String> {
        if let Some(lock) = &mut self.lock {
            lock.refresh(time::now()).map_err(|holder| {
                format!("{} on {} has taken over this project", holder.user, holder.host)
            })?;
        }
        Ok(())
    }
    /// Packs and releases the lock.
    pub fn close(mut self) -> Result<(), PackError> {
        let result = if self.is_dirty() { self.pack(false) } else { Ok(()) };
        if let Some(lock) = self.lock.take() {
            lock.release();
        }
        result
    }
    /// Gives up the lock without packing (the user discarded the project's changes or lost it).
    pub fn abandon(mut self) {
        if let Some(lock) = self.lock.take() {
            lock.release();
        }
    }

    /// Saves a copy to `target` as an independent project and continues working in the copy.
    pub fn save_as(&mut self, target: &Path) -> Result<(), String> {
        let new_id = new_id();
        let new_root = self.data.join("work").join(&new_id);
        fn copy(from: &Path, to: &Path) -> io::Result<()> {
            fs::create_dir_all(to)?;
            for entry in fs::read_dir(from)? {
                let entry = entry?;
                let name = entry.file_name();
                if name.to_string_lossy().starts_with('.') {
                    continue;
                }
                if entry.file_type()?.is_dir() {
                    copy(&entry.path(), &to.join(name))?;
                } else {
                    fs::copy(entry.path(), to.join(name))?;
                }
            }
            Ok(())
        }
        copy(self.work.root(), &new_root).map_err(io_message)?;
        let lock = match Lock::acquire(target, false, time::now()).map_err(io_message)? {
            Acquired::Held(lock) => lock,
            Acquired::Conflict(c) => {
                let _ = fs::remove_dir_all(&new_root);
                return Err(c.message(time::now()));
            }
        };
        let mut manifest = self.manifest.clone();
        manifest.id = new_id;
        let work = WorkDir::at(new_root);
        work.write(MANIFEST, manifest.to_json().as_bytes()).map_err(io_message)?;
        if let Err(e) = work.pack(target, true) {
            lock.release();
            return Err(e.to_string());
        }
        if let Some(old) = self.lock.take() {
            old.release();
        }
        self.work = work;
        self.manifest = manifest;
        self.path = target.to_owned();
        self.lock = Some(lock);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dirs() -> (tempfile::TempDir, tempfile::TempDir) {
        (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap())
    }

    #[test]
    fn a_new_project_is_a_readable_zip_with_one_chapter() {
        let (data, out) = dirs();
        let jw = out.path().join("novel.jw");
        let project = Project::create(data.path(), &jw, "My Novel").unwrap();
        let names = super::super::archive::entries(&jw).unwrap();
        assert!(names.contains(&"manifest.json".into()) && names.contains(&"book.json".into()));
        assert_eq!(project.manifest.tree.manuscript.len(), 1);
        assert!(names.iter().any(|n| n.starts_with("chapters/")));
        project.close().unwrap();
        // The lock is released on close.
        assert!(!super::super::lock::lock_path(&jw).exists());
    }

    #[test]
    fn editing_the_tree_keeps_files_and_word_counts_in_step() {
        let (data, out) = dirs();
        let jw = out.path().join("novel.jw");
        let mut p = Project::create(data.path(), &jw, "Novel").unwrap();
        let first = p.manifest.tree.manuscript[0].id().to_owned();
        let neutral = notes::to_neutral("One two three $$ not counted\nfour\n", &notes::Marker::default()).0;
        assert_eq!(p.write_text(&first, &neutral).unwrap(), 4);
        assert_eq!(p.manuscript_words(), 4);
        assert_eq!(p.session_words(), 4);
        let part = p.add(Section::Manuscript, &[], 1, Node::part("Act I")).unwrap();
        let second = p.add(Section::Manuscript, &part, 0, Node::chapter("Chapter 2")).unwrap();
        let second_id = p.manifest.tree.get(Section::Manuscript, &second).unwrap().id().to_owned();
        p.write_text(&second_id, "five six").unwrap();
        let page = p.add(Section::Notes, &[], 0, Node::page("Characters")).unwrap();
        let page_id = p.manifest.tree.get(Section::Notes, &page).unwrap().id().to_owned();
        p.write_text(&page_id, "lots of notes words here").unwrap();
        assert_eq!(p.manuscript_words(), 6);
        assert_eq!(p.manifest.tree.notes_words(), 5);
        assert!(p.add(Section::Manuscript, &[], 0, Node::page("nope")).is_err());
        // Renaming and moving.
        p.rename(&second_id, "The Storm").unwrap();
        assert!(p.rename(&second_id, "  ").is_err());
        p.move_node(Section::Manuscript, &second, &[], 0).unwrap();
        assert_eq!(p.manifest.tree.manuscript[0].title(), "The Storm");
        // Deleting a part removes its chapters' files; the text is gone from the working copy.
        let part_id = p.manifest.tree.manuscript[2].id().to_owned();
        assert!(p.manifest.tree.manuscript[2].is_part());
        let file = p.file_of(&second_id).unwrap();
        p.delete(&second_id).unwrap();
        assert!(!p.work.exists(&file));
        p.delete(&part_id).unwrap();
        assert_eq!(p.manifest.tree.manuscript.len(), 1);
        assert_eq!(p.id_of_file(&p.file_of(&first).unwrap()).as_deref(), Some(first.as_str()));
        p.recount().unwrap();
        assert_eq!(p.manuscript_words(), 4);
        p.close().unwrap();
    }

    #[test]
    fn a_saved_project_opens_on_another_computer_with_everything() {
        let (data, out) = dirs();
        let jw = out.path().join("novel.jw");
        let mut p = Project::create(data.path(), &jw, "Novel").unwrap();
        let id = p.manifest.tree.manuscript[0].id().to_owned();
        p.write_text(&id, "Hello there.").unwrap();
        p.book.legal_name = "Jane Doe".into();
        p.book.isbn = "123".into();
        p.save_book().unwrap();
        p.manifest.settings.language = crate::core::typography::Language::Spanish;
        p.save_manifest().unwrap();
        p.close().unwrap();

        let elsewhere = tempfile::tempdir().unwrap();
        let Ok(opened) = Project::open(elsewhere.path(), &jw, false) else { panic!("open") };
        assert!(opened.recovered.is_none());
        assert_eq!(opened.project.read_text(&id).unwrap(), "Hello there.");
        assert_eq!(opened.project.book.legal_name, "Jane Doe");
        assert_eq!(opened.project.manifest.settings.language, crate::core::typography::Language::Spanish);
        assert_eq!(opened.project.baseline_words, 2);
        opened.project.close().unwrap();
    }

    #[test]
    fn a_second_open_warns_and_can_be_forced() {
        let (data, out) = dirs();
        let other_data = tempfile::tempdir().unwrap();
        let jw = out.path().join("novel.jw");
        let p = Project::create(data.path(), &jw, "Novel").unwrap();
        // Pretend the lock belongs to another computer.
        let mut info = super::super::lock::read(&jw).unwrap();
        info.host = "OTHER".into();
        info.user = "sam".into();
        fs::write(super::super::lock::lock_path(&jw), serde_json::to_string(&info).unwrap()).unwrap();
        match Project::open(other_data.path(), &jw, false) {
            Err(OpenError::Locked(c)) => assert!(c.message(time::now()).contains("sam on OTHER")),
            _ => panic!("expected a lock conflict"),
        }
        assert!(Project::open(other_data.path(), &jw, true).is_ok());
        // The first window finds out when it next checks in.
        let mut p = p;
        assert!(p.heartbeat().is_err());
    }

    #[test]
    fn crash_recovery_keeps_unpacked_chapters() {
        let (data, out) = dirs();
        let jw = out.path().join("novel.jw");
        let mut p = Project::create(data.path(), &jw, "Novel").unwrap();
        let id = p.manifest.tree.manuscript[0].id().to_owned();
        p.write_text(&id, "typed before the crash").unwrap();
        // The process dies: no pack, no lock release. The same-process lock is simply re-taken.
        drop(p);
        let Ok(opened) = Project::open(data.path(), &jw, false) else { panic!("open") };
        let recovered = opened.recovered.expect("recovered");
        assert!(recovered.unsaved.iter().any(|f| f.starts_with("chapters/")));
        assert_eq!(opened.project.read_text(&id).unwrap(), "typed before the crash");
    }

    #[test]
    fn save_as_makes_an_independent_copy() {
        let (data, out) = dirs();
        let jw = out.path().join("novel.jw");
        let copy = out.path().join("draft two.jw");
        let mut p = Project::create(data.path(), &jw, "Novel").unwrap();
        let id = p.manifest.tree.manuscript[0].id().to_owned();
        p.write_text(&id, "shared start").unwrap();
        p.pack(false).unwrap();
        let old_id = p.manifest.id.clone();
        p.save_as(&copy).unwrap();
        assert_ne!(p.manifest.id, old_id);
        p.write_text(&id, "only in the copy").unwrap();
        p.close().unwrap();
        let original = Project::open(data.path(), &jw, false).ok().unwrap();
        assert_eq!(original.project.read_text(&id).unwrap(), "shared start");
        let copied = Project::open(data.path(), &copy, false).ok().unwrap();
        assert_eq!(copied.project.read_text(&id).unwrap(), "only in the copy");
    }

    #[test]
    fn cover_images_are_stored_in_assets() {
        let (data, out) = dirs();
        let jw = out.path().join("novel.jw");
        let mut p = Project::create(data.path(), &jw, "Novel").unwrap();
        let image = out.path().join("c.png");
        fs::write(&image, b"\x89PNG fake").unwrap();
        p.set_cover(&image).unwrap();
        assert_eq!(p.book.cover, "assets/cover.png");
        assert_eq!(p.cover_bytes().unwrap(), b"\x89PNG fake");
        assert!(p.set_cover(&out.path().join("c.exe")).is_err());
        p.pack(false).unwrap();
        assert!(super::super::archive::entries(&jw).unwrap().contains(&"assets/cover.png".to_string()));
    }
}
