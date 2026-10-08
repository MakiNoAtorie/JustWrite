use super::workspace::valid_relative;
use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use ropey::{Rope, RopeBuilder};
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    path::{Path, PathBuf},
    sync::Arc,
};

pub enum ReadEvent {
    Chunk(Rope),
    Complete,
    Error(io::Error),
}
pub type ReadSink = Arc<dyn Fn(ReadEvent) + Send + Sync>;
#[derive(Clone, Debug)]
pub struct Entry {
    pub path: PathBuf,
    pub directory: bool,
}

/// Every document operation, including watching, goes through this boundary.
pub trait Storage: Send + Sync {
    fn read(&self, path: &Path, sink: ReadSink);
    fn write(&self, path: &Path, text: &Rope) -> io::Result<()>;
    fn list(&self, path: &Path) -> io::Result<Vec<Entry>>;
    fn watch(&self, changed: Arc<dyn Fn(PathBuf) + Send + Sync>) -> io::Result<RecommendedWatcher>;
    fn rename(&self, from: &Path, to: &Path) -> io::Result<()>;
    fn remove(&self, path: &Path) -> io::Result<()>;
    fn create(&self, path: &Path, folder: bool) -> io::Result<()>;
    fn write_checked(&self, path: &Path, text: &Rope, expected: &Rope) -> io::Result<Option<Rope>> {
        let disk = read_complete(self, path)?;
        if disk != *expected {
            return Ok(Some(disk));
        }
        self.write(path, text)?;
        Ok(None)
    }
}

pub struct LocalStorage {
    root: PathBuf,
}
impl LocalStorage {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }
    fn resolve(&self, path: &Path) -> io::Result<PathBuf> {
        if !valid_relative(path) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "A workspace-relative path is required",
            ));
        }
        // Canonicalizing the parent also prevents symlinks from escaping the workspace.
        let root = self.root.canonicalize()?;
        let path: PathBuf = path
            .components()
            .filter_map(|c| match c {
                std::path::Component::Normal(name) => Some(name),
                _ => None,
            })
            .collect();
        if path.as_os_str().is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "The workspace root cannot be changed as a file",
            ));
        }
        let target = root.join(path);
        let parent = target
            .parent()
            .ok_or_else(|| io::Error::other("Missing parent folder"))?
            .canonicalize()?;
        if !parent.starts_with(&root) {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "Path leaves the workspace",
            ));
        }
        if target.exists() && !target.canonicalize()?.starts_with(&root) {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "Symlink leaves the workspace",
            ));
        }
        Ok(target)
    }
}
impl Storage for LocalStorage {
    fn read(&self, path: &Path, sink: ReadSink) {
        let storage = Self::new(self.root.clone());
        let path = path.to_path_buf();
        std::thread::spawn(move || {
            let result = storage
                .resolve(&path)
                .and_then(|target| stream_utf8(File::open(target)?, &*sink));
            match result {
                Ok(()) => sink(ReadEvent::Complete),
                Err(error) => sink(ReadEvent::Error(error)),
            }
        });
    }
    fn write(&self, path: &Path, text: &Rope) -> io::Result<()> {
        let target = self.resolve(path)?;
        let mut temporary = tempfile::NamedTempFile::new_in(
            target
                .parent()
                .ok_or_else(|| io::Error::other("Missing parent"))?,
        )?;
        text.write_to(temporary.as_file_mut())?;
        temporary.flush()?;
        temporary.as_file().sync_all()?;
        if target.exists() {
            fs::set_permissions(temporary.path(), fs::metadata(&target)?.permissions())?;
        }
        // A short-lived reader or antivirus handle can delay replacement on Windows.
        for attempt in 0..5 {
            match temporary.persist(&target) {
                Ok(_) => return Ok(()),
                Err(error)
                    if attempt < 4 && error.error.kind() == io::ErrorKind::PermissionDenied =>
                {
                    temporary = error.file;
                    std::thread::sleep(std::time::Duration::from_millis(10 << attempt));
                }
                Err(error) => return Err(error.error),
            }
        }
        unreachable!()
    }
    fn list(&self, path: &Path) -> io::Result<Vec<Entry>> {
        let directory = if valid_relative(path)
            && path.components().all(|c| c == std::path::Component::CurDir)
        {
            self.root.canonicalize()?
        } else {
            self.resolve(path)?
        };
        fs::read_dir(directory)?
            .map(|entry| {
                let entry = entry?;
                let path = path
                    .join(entry.file_name())
                    .components()
                    .filter_map(|c| match c {
                        std::path::Component::Normal(name) => Some(name),
                        _ => None,
                    })
                    .collect();
                Ok(Entry {
                    path,
                    directory: entry.file_type()?.is_dir(),
                })
            })
            .collect()
    }
    fn watch(&self, changed: Arc<dyn Fn(PathBuf) + Send + Sync>) -> io::Result<RecommendedWatcher> {
        let root = self.root.canonicalize()?;
        let base = root.clone();
        let mut watcher =
            notify::recommended_watcher(move |event: Result<notify::Event, notify::Error>| {
                if let Ok(event) = event {
                    if event.kind.is_modify() || event.kind.is_create() || event.kind.is_remove() {
                        for path in event.paths {
                            if let Ok(relative) = path.strip_prefix(&base) {
                                changed(relative.to_path_buf());
                            }
                        }
                    }
                }
            })
            .map_err(io::Error::other)?;
        watcher
            .watch(&root, RecursiveMode::Recursive)
            .map_err(io::Error::other)?;
        Ok(watcher)
    }
    fn rename(&self, from: &Path, to: &Path) -> io::Result<()> {
        let target = self.resolve(to)?;
        if target.exists() {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "A file with that name already exists",
            ));
        }
        fs::rename(self.resolve(from)?, target)
    }
    fn remove(&self, path: &Path) -> io::Result<()> {
        let target = self.resolve(path)?;
        if target.is_dir() {
            fs::remove_dir_all(target)
        } else {
            fs::remove_file(target)
        }
    }
    fn create(&self, path: &Path, folder: bool) -> io::Result<()> {
        let target = self.resolve(path)?;
        if folder {
            fs::create_dir(target)
        } else {
            OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(target)?
                .sync_all()
        }
    }
}

pub fn read_complete<S: Storage + ?Sized>(storage: &S, path: &Path) -> io::Result<Rope> {
    let (sender, receiver) = std::sync::mpsc::channel();
    storage.read(
        path,
        Arc::new(move |event| {
            let _ = sender.send(event);
        }),
    );
    let mut text = Rope::new();
    while let Ok(event) = receiver.recv() {
        match event {
            ReadEvent::Chunk(chunk) => text.append(chunk),
            ReadEvent::Complete => return Ok(text),
            ReadEvent::Error(error) => return Err(error),
        }
    }
    Err(io::Error::other("File loader stopped"))
}

fn stream_utf8(mut file: impl Read, sink: &dyn Fn(ReadEvent)) -> io::Result<()> {
    let mut bytes = vec![0; 8192];
    let mut pending = Vec::new();
    loop {
        let count = file.read(&mut bytes)?;
        if count == 0 {
            if !pending.is_empty() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "File ends with incomplete UTF-8",
                ));
            }
            return Ok(());
        }
        pending.extend_from_slice(&bytes[..count]);
        let valid = match std::str::from_utf8(&pending) {
            Ok(_) => pending.len(),
            Err(error) if error.error_len().is_none() => error.valid_up_to(),
            Err(_) => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "This file is not UTF-8",
                ));
            }
        };
        if valid > 0 {
            let mut builder = RopeBuilder::new();
            builder.append(std::str::from_utf8(&pending[..valid]).map_err(io::Error::other)?);
            sink(ReadEvent::Chunk(builder.finish()));
            pending.drain(..valid);
        }
        // Send the first screen early, then amortize the rest in 1 MiB batches.
        if bytes.len() < 1024 * 1024 {
            bytes.resize(1024 * 1024, 0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn streaming_utf8_and_atomic_replace() {
        let mut data = "a".repeat(8191);
        data.push_str("日本語\r\n");
        let loaded = std::sync::Mutex::new(String::new());
        stream_utf8(data.as_bytes(), &|event| {
            if let ReadEvent::Chunk(chunk) = event {
                loaded.lock().unwrap().push_str(&chunk.to_string());
            }
        })
        .unwrap();
        assert_eq!(*loaded.lock().unwrap(), data);
        assert!(stream_utf8(&[0xff][..], &|_| {}).is_err());
        let root = std::env::temp_dir().join(format!("jw-test-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        let store = LocalStorage::new(root.clone());
        store
            .write(Path::new("test.md"), &Rope::from_str("first"))
            .unwrap();
        store
            .write(Path::new("test.md"), &Rope::from_str(&data))
            .unwrap();
        assert_eq!(fs::read_to_string(root.join("test.md")).unwrap(), data);
        let conflict = store
            .write_checked(
                Path::new("test.md"),
                &Rope::from_str("wrong"),
                &Rope::from_str("first"),
            )
            .unwrap();
        assert_eq!(conflict.unwrap().to_string(), data);
        assert_eq!(fs::read_to_string(root.join("test.md")).unwrap(), data);
        assert!(store.create(Path::new("test.md"), false).is_err());
        let entries = store.list(Path::new(".")).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].path, Path::new("test.md"));
        assert!(store.remove(Path::new(".")).is_err());
        assert!(
            store
                .write(Path::new("../escape.md"), &Rope::new())
                .is_err()
        );
        fs::remove_file(root.join("test.md")).unwrap();
        fs::remove_dir(root).unwrap();
    }
}
