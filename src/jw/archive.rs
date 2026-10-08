//! Packing a working copy into a `.jw` ZIP and unpacking it again.
//!
//! The ZIP is plain and recoverable with any zip tool. Packing is atomic: the archive is written
//! next to the target and renamed over it. Entries that did not change since the last pack are
//! copied from the old archive without being recompressed.

use super::manifest::{MANIFEST, Manifest};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File},
    io::{self, Read, Write},
    path::{Path, PathBuf},
};
use zip::{CompressionMethod, ZipArchive, ZipWriter, write::SimpleFileOptions};

/// Size and modification time, enough to notice that a file changed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Stamp {
    pub len: u64,
    pub mtime_ms: i64,
}
pub fn stamp(path: &Path) -> io::Result<Stamp> {
    let meta = fs::metadata(path)?;
    let mtime_ms = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_millis() as i64);
    Ok(Stamp { len: meta.len(), mtime_ms })
}

/// What the last pack wrote, so the next one can skip unchanged files.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct PackState {
    pub jw: Stamp,
    pub files: BTreeMap<String, Stamp>,
}

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

pub fn read_manifest(jw: &Path) -> Result<Manifest, String> {
    let file = File::open(jw).map_err(|e| format!("Cannot open {}: {e}", jw.display()))?;
    let mut archive = ZipArchive::new(file).map_err(|e| format!("{} is not a valid .jw file: {e}", jw.display()))?;
    let mut entry = archive.by_name(MANIFEST).map_err(|_| "This .jw file has no manifest.json".to_string())?;
    let mut bytes = Vec::new();
    entry.read_to_end(&mut bytes).map_err(|e| e.to_string())?;
    Manifest::parse(&bytes)
}

/// Extracts `jw` into `dest`, which must not exist yet or be empty.
pub fn unpack(jw: &Path, dest: &Path) -> io::Result<PackState> {
    let mut archive = ZipArchive::new(File::open(jw)?).map_err(|e| invalid(e.to_string()))?;
    fs::create_dir_all(dest)?;
    let mut files = BTreeMap::new();
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).map_err(|e| invalid(e.to_string()))?;
        // `enclosed_name` rejects absolute paths and `..` (zip slip).
        let Some(relative) = entry.enclosed_name() else {
            return Err(invalid(format!("Unsafe path in archive: {}", entry.name())));
        };
        let target = dest.join(&relative);
        if entry.is_dir() {
            fs::create_dir_all(&target)?;
            continue;
        }
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut out = File::create(&target)?;
        io::copy(&mut entry, &mut out)?;
        out.flush()?;
        drop(out);
        files.insert(entry.name().to_owned(), stamp(&target)?);
    }
    Ok(PackState { jw: stamp(jw)?, files })
}

/// Every project file under `work`, as `/`-separated names. Hidden files (the journal and other
/// bookkeeping) and temporary files are not part of the project.
pub fn project_files(work: &Path) -> io::Result<Vec<String>> {
    fn walk(dir: &Path, prefix: &str, out: &mut Vec<String>) -> io::Result<()> {
        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') || name.ends_with(".tmp") {
                continue;
            }
            let path = format!("{prefix}{name}");
            if entry.file_type()?.is_dir() {
                walk(&entry.path(), &format!("{path}/"), out)?;
            } else {
                out.push(path);
            }
        }
        Ok(())
    }
    let mut out = Vec::new();
    walk(work, "", &mut out)?;
    // The manifest first, then the rest in a stable order.
    out.sort_by_key(|p| (p != MANIFEST, p.clone()));
    Ok(out)
}

fn zip_time(mtime_ms: i64) -> zip::DateTime {
    jiff::Timestamp::from_millisecond(mtime_ms)
        .ok()
        .map(|t| t.to_zoned(jiff::tz::TimeZone::UTC).datetime())
        .and_then(|d| {
            zip::DateTime::from_date_and_time(
                d.year().max(1980) as u16,
                d.month() as u8,
                d.day() as u8,
                d.hour() as u8,
                d.minute() as u8,
                d.second() as u8,
            )
            .ok()
        })
        .unwrap_or_default()
}
fn already_compressed(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    [".jpg", ".jpeg", ".png", ".webp", ".gif", ".zip"].iter().any(|e| lower.ends_with(e))
}

/// What a pack may reuse from the archive written last time.
pub struct Previous<'a> {
    pub jw: &'a Path,
    pub state: &'a PackState,
    /// Files written since that pack, whatever their stamps say.
    pub dirty: &'a BTreeSet<String>,
}

/// Writes `work` as a `.jw` at `target`, atomically.
pub fn pack(work: &Path, target: &Path, previous: Option<Previous>) -> io::Result<PackState> {
    let directory = target.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new("."));
    let temporary = tempfile::Builder::new().prefix(".jw-").suffix(".tmp").tempfile_in(directory)?;
    let mut old = previous
        .as_ref()
        .and_then(|p| File::open(p.jw).ok())
        .and_then(|f| ZipArchive::new(f).ok());
    let mut state = PackState::default();
    {
        let mut writer = ZipWriter::new(io::BufWriter::new(temporary.as_file().try_clone()?));
        for name in project_files(work)? {
            let path = work.join(&name);
            let now = stamp(&path)?;
            // History blobs are named by their hash, so a name that exists is a file that matches.
            let immutable = name.starts_with("history/blobs/");
            let reusable = previous.as_ref().is_some_and(|p| {
                !p.dirty.contains(&name) && (immutable || p.state.files.get(&name) == Some(&now))
            });
            let mut copied = false;
            if reusable {
                if let Some(archive) = old.as_mut() {
                    if let Some(index) = archive.index_for_name(&name) {
                        let entry = archive.by_index_raw(index).map_err(|e| invalid(e.to_string()))?;
                        writer.raw_copy_file(entry).map_err(|e| invalid(e.to_string()))?;
                        copied = true;
                    }
                }
            }
            if !copied {
                let method = if already_compressed(&name) { CompressionMethod::Stored } else { CompressionMethod::Deflated };
                let options = SimpleFileOptions::default().compression_method(method).last_modified_time(zip_time(now.mtime_ms));
                writer.start_file(&name, options).map_err(|e| invalid(e.to_string()))?;
                let mut file = File::open(&path)?;
                io::copy(&mut file, &mut writer)?;
            }
            state.files.insert(name, now);
        }
        let mut inner = writer.finish().map_err(|e| invalid(e.to_string()))?;
        inner.flush()?;
    }
    temporary.as_file().sync_all()?;
    // Check what was written before it replaces anything.
    {
        let mut check = ZipArchive::new(File::open(temporary.path())?).map_err(|e| invalid(e.to_string()))?;
        if check.len() != state.files.len() {
            return Err(invalid("The archive written does not match the project"));
        }
        let mut entry = check.by_name(MANIFEST).map_err(|e| invalid(e.to_string()))?;
        let mut bytes = Vec::new();
        entry.read_to_end(&mut bytes)?;
        Manifest::parse(&bytes).map_err(invalid)?;
    }
    persist(temporary, target)?;
    state.jw = stamp(target)?;
    Ok(state)
}

/// Replaces `target` with the temporary file; a sync client or virus scanner holding the target
/// open for a moment is waited out.
pub fn persist(mut temporary: tempfile::NamedTempFile, target: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        // Temporary files are private; the saved file keeps the mode of the one it replaces, or
        // the usual one for a new file.
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(target).map_or(0o644, |m| m.permissions().mode() & 0o777);
        let _ = temporary.as_file().set_permissions(std::fs::Permissions::from_mode(mode));
    }
    for attempt in 0..6 {
        match temporary.persist(target) {
            Ok(_) => return Ok(()),
            Err(error) if attempt < 5 && matches!(error.error.kind(), io::ErrorKind::PermissionDenied | io::ErrorKind::Other) => {
                temporary = error.file;
                std::thread::sleep(std::time::Duration::from_millis(20 << attempt));
            }
            Err(error) => return Err(error.error),
        }
    }
    unreachable!()
}

/// Lists the entries of a `.jw`; used by tests and diagnostics.
pub fn entries(jw: &Path) -> io::Result<Vec<String>> {
    let archive = ZipArchive::new(File::open(jw)?).map_err(|e| invalid(e.to_string()))?;
    Ok(archive.file_names().map(str::to_owned).collect())
}

pub fn temporary_sibling(target: &Path, tag: &str) -> PathBuf {
    let name = target.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    target.with_file_name(format!(".{name}.{tag}.tmp"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project(dir: &Path) {
        fs::create_dir_all(dir.join("chapters")).unwrap();
        fs::create_dir_all(dir.join("history/blobs")).unwrap();
        fs::write(dir.join(MANIFEST), Manifest::new("T").to_json()).unwrap();
        fs::write(dir.join("chapters/a.txt"), "Chapter one text\n".repeat(100)).unwrap();
        fs::write(dir.join("history/blobs/abc"), "blob").unwrap();
        fs::write(dir.join(".journal"), "not part of the project").unwrap();
    }

    #[test]
    fn pack_and_unpack_round_trip_with_a_plain_zip() {
        let work = tempfile::tempdir().unwrap();
        let out = tempfile::tempdir().unwrap();
        project(work.path());
        let jw = out.path().join("book.jw");
        let state = pack(work.path(), &jw, None).unwrap();
        let names = entries(&jw).unwrap();
        assert_eq!(names.len(), 3);
        assert!(!names.iter().any(|n| n.contains("journal")));
        assert!(state.files.contains_key("chapters/a.txt"));
        assert_eq!(read_manifest(&jw).unwrap().title, "T");
        // No temporary files are left beside the target.
        let leftovers: Vec<_> = fs::read_dir(out.path()).unwrap().map(|e| e.unwrap().file_name()).collect();
        assert_eq!(leftovers.len(), 1);
        let restored = tempfile::tempdir().unwrap();
        let unpacked = unpack(&jw, &restored.path().join("w")).unwrap();
        assert_eq!(fs::read_to_string(restored.path().join("w/chapters/a.txt")).unwrap(), "Chapter one text\n".repeat(100));
        assert_eq!(unpacked.files.len(), 3);
    }

    #[test]
    fn unchanged_entries_are_copied_and_changed_ones_rewritten() {
        let work = tempfile::tempdir().unwrap();
        let out = tempfile::tempdir().unwrap();
        project(work.path());
        let jw = out.path().join("book.jw");
        let first = pack(work.path(), &jw, None).unwrap();
        fs::write(work.path().join("chapters/b.txt"), "new chapter").unwrap();
        fs::write(work.path().join("chapters/a.txt"), "edited").unwrap();
        // Even with a stamp that claims nothing changed, a dirty file is rewritten.
        let mut stale = first.clone();
        stale.files.insert("chapters/a.txt".into(), stamp(&work.path().join("chapters/a.txt")).unwrap());
        let dirty: BTreeSet<String> = ["chapters/a.txt".to_string()].into();
        let second = pack(work.path(), &jw, Some(Previous { jw: &jw.clone(), state: &stale, dirty: &dirty })).unwrap();
        let mut archive = ZipArchive::new(File::open(&jw).unwrap()).unwrap();
        let mut text = String::new();
        archive.by_name("chapters/a.txt").unwrap().read_to_string(&mut text).unwrap();
        assert_eq!(text, "edited");
        assert_eq!(archive.len(), 4);
        assert!(second.files.contains_key("chapters/b.txt"));
        // Deleted files disappear from the archive.
        fs::remove_file(work.path().join("chapters/b.txt")).unwrap();
        pack(work.path(), &jw, Some(Previous { jw: &jw.clone(), state: &second, dirty: &BTreeSet::new() })).unwrap();
        assert_eq!(entries(&jw).unwrap().len(), 3);
    }

    #[test]
    fn archives_with_unsafe_paths_are_refused() {
        let out = tempfile::tempdir().unwrap();
        let jw = out.path().join("evil.jw");
        let mut writer = ZipWriter::new(File::create(&jw).unwrap());
        writer.start_file("../escape.txt", SimpleFileOptions::default()).unwrap();
        writer.write_all(b"x").unwrap();
        writer.finish().unwrap();
        let dest = out.path().join("dest");
        assert!(unpack(&jw, &dest).is_err());
        assert!(!out.path().join("escape.txt").exists());
    }

    #[test]
    fn a_failed_pack_leaves_the_old_file_alone() {
        let work = tempfile::tempdir().unwrap();
        let out = tempfile::tempdir().unwrap();
        project(work.path());
        let jw = out.path().join("book.jw");
        pack(work.path(), &jw, None).unwrap();
        let before = fs::read(&jw).unwrap();
        // A manifest that cannot be parsed makes the check after writing fail.
        fs::write(work.path().join(MANIFEST), "not json").unwrap();
        assert!(pack(work.path(), &jw, None).is_err());
        assert_eq!(fs::read(&jw).unwrap(), before);
        assert_eq!(fs::read_dir(out.path()).unwrap().count(), 1);
    }
}
