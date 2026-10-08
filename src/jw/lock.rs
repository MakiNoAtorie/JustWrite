//! A lock file beside the `.jw` so two computers sharing a synced folder do not edit the same
//! project at once. It is advisory: a clear warning, not a guarantee.

use crate::platform;
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
};

/// A lock whose heartbeat is older than this belongs to a computer that has probably gone.
pub const STALE_SECONDS: i64 = 5 * 60;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LockInfo {
    pub host: String,
    pub user: String,
    pub pid: u32,
    pub since: i64,
    pub heartbeat: i64,
    pub app: String,
    /// Identifies one opening of the project; refreshing checks it is still ours.
    pub token: String,
}
#[derive(Clone, Debug)]
pub struct Conflict {
    pub holder: LockInfo,
    pub same_machine: bool,
    /// The holder has not been heard from for a long time.
    pub stale: bool,
}
impl Conflict {
    pub fn message(&self, now: i64) -> String {
        let who = if self.holder.user.is_empty() {
            self.holder.host.clone()
        } else {
            format!("{} on {}", self.holder.user, self.holder.host)
        };
        let seen = super::time::ago(self.holder.heartbeat, now);
        if self.same_machine {
            format!("This project is already open in another JustWrite window ({who}, last active {seen}).")
        } else if self.stale {
            format!("This project was opened by {who}, last active {seen}. That computer may have closed it without releasing it.")
        } else {
            format!("This project is open on {who} (last active {seen}). Editing it in two places can lose work.")
        }
    }
}

pub struct Lock {
    path: PathBuf,
    info: LockInfo,
}
pub enum Acquired {
    Held(Lock),
    Conflict(Conflict),
}

pub fn lock_path(jw: &Path) -> PathBuf {
    let name = jw.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    jw.with_file_name(format!(".{name}.lock"))
}
fn mine(now: i64) -> LockInfo {
    LockInfo {
        host: platform::machine_name(),
        user: platform::user_name(),
        pid: std::process::id(),
        since: now,
        heartbeat: now,
        app: format!("JustWrite {}", env!("CARGO_PKG_VERSION")),
        token: super::manifest::new_id(),
    }
}
fn write_new(path: &Path, info: &LockInfo) -> io::Result<()> {
    let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
    file.write_all(serde_json::to_string_pretty(info).map_err(io::Error::other)?.as_bytes())?;
    file.sync_all()
}
fn replace(path: &Path, info: &LockInfo) -> io::Result<()> {
    let directory = path.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new("."));
    let mut temporary = tempfile::Builder::new().prefix(".lock-").suffix(".tmp").tempfile_in(directory)?;
    temporary.write_all(serde_json::to_string_pretty(info).map_err(io::Error::other)?.as_bytes())?;
    super::archive::persist(temporary, path)
}
pub fn read(jw: &Path) -> Option<LockInfo> {
    serde_json::from_slice(&fs::read(lock_path(jw)).ok()?).ok()
}

impl Lock {
    /// Takes the lock. With `force` an existing lock is overwritten (the user chose to open the
    /// project anyway).
    pub fn acquire(jw: &Path, force: bool, now: i64) -> io::Result<Acquired> {
        let path = lock_path(jw);
        let info = mine(now);
        for _ in 0..3 {
            match write_new(&path, &info) {
                Ok(()) => return Ok(Acquired::Held(Self { path, info })),
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
                Err(e) => return Err(e),
            }
            let existing = fs::read(&path).ok().and_then(|b| serde_json::from_slice::<LockInfo>(&b).ok());
            let Some(existing) = existing else {
                // Unreadable lock: treat as left over.
                let _ = fs::remove_file(&path);
                continue;
            };
            let same_machine = existing.host == info.host;
            if same_machine && existing.pid == info.pid {
                replace(&path, &info)?;
                return Ok(Acquired::Held(Self { path, info }));
            }
            if same_machine && !platform::process_alive(existing.pid) {
                let _ = fs::remove_file(&path);
                continue;
            }
            if force {
                replace(&path, &info)?;
                return Ok(Acquired::Held(Self { path, info }));
            }
            let stale = now - existing.heartbeat > STALE_SECONDS;
            return Ok(Acquired::Conflict(Conflict { holder: existing, same_machine, stale }));
        }
        Err(io::Error::other("Cannot take the project lock"))
    }
    /// Shows that this computer is still here. Fails if someone else took the lock over.
    pub fn refresh(&mut self, now: i64) -> Result<(), LockInfo> {
        match fs::read(&self.path).ok().and_then(|b| serde_json::from_slice::<LockInfo>(&b).ok()) {
            Some(current) if current.token != self.info.token => return Err(current),
            None => {}
            Some(_) => {}
        }
        self.info.heartbeat = now;
        let _ = replace(&self.path, &self.info);
        Ok(())
    }
    pub fn release(self) {
        if let Some(current) = fs::read(&self.path).ok().and_then(|b| serde_json::from_slice::<LockInfo>(&b).ok()) {
            if current.token == self.info.token {
                let _ = fs::remove_file(&self.path);
            }
        }
    }
    pub fn info(&self) -> &LockInfo {
        &self.info
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn other_machine(jw: &Path, heartbeat: i64) {
        let info = LockInfo {
            host: "OTHER-PC".into(),
            user: "sam".into(),
            pid: 4242,
            since: heartbeat,
            heartbeat,
            app: "JustWrite".into(),
            token: "abc".into(),
        };
        fs::write(lock_path(jw), serde_json::to_string(&info).unwrap()).unwrap();
    }
    #[test]
    fn lock_is_taken_refreshed_and_released() {
        let dir = tempfile::tempdir().unwrap();
        let jw = dir.path().join("book.jw");
        let Acquired::Held(mut lock) = Lock::acquire(&jw, false, 1000).unwrap() else { panic!("expected the lock") };
        assert!(lock_path(&jw).exists());
        assert_eq!(read(&jw).unwrap().heartbeat, 1000);
        lock.refresh(1060).unwrap();
        assert_eq!(read(&jw).unwrap().heartbeat, 1060);
        // The same process may re-acquire (reopening in the same window).
        let Acquired::Held(again) = Lock::acquire(&jw, false, 1100).unwrap() else { panic!("same process") };
        // The old handle no longer owns the file, so releasing it leaves the new one in place.
        lock.release();
        assert!(lock_path(&jw).exists());
        again.release();
        assert!(!lock_path(&jw).exists());
    }
    #[test]
    fn another_computer_is_a_conflict_with_a_clear_message() {
        let dir = tempfile::tempdir().unwrap();
        let jw = dir.path().join("book.jw");
        other_machine(&jw, 1000);
        let Acquired::Conflict(c) = Lock::acquire(&jw, false, 1100).unwrap() else { panic!("expected conflict") };
        assert!(!c.same_machine && !c.stale);
        let message = c.message(1100);
        assert!(message.contains("sam on OTHER-PC") && message.contains("two places"), "{message}");
        // A long-silent holder is reported as stale but still needs the user's decision.
        let Acquired::Conflict(c) = Lock::acquire(&jw, false, 1000 + STALE_SECONDS + 1).unwrap() else { panic!() };
        assert!(c.stale && c.message(5000).contains("may have closed"));
        // Opening anyway takes the lock; the other computer's refresh then notices.
        let Acquired::Held(mine) = Lock::acquire(&jw, true, 1200).unwrap() else { panic!("forced") };
        assert_eq!(read(&jw).unwrap().host, platform::machine_name());
        mine.release();
    }
    #[test]
    fn losing_the_lock_is_reported_by_refresh() {
        let dir = tempfile::tempdir().unwrap();
        let jw = dir.path().join("book.jw");
        let Acquired::Held(mut lock) = Lock::acquire(&jw, false, 1).unwrap() else { panic!() };
        other_machine(&jw, 2);
        assert_eq!(lock.refresh(3).unwrap_err().host, "OTHER-PC");
        // Releasing a lock that is no longer ours leaves the new owner's file alone.
        lock.release();
        assert!(lock_path(&jw).exists());
    }
    #[test]
    fn a_dead_process_on_this_machine_is_a_stale_lock() {
        let dir = tempfile::tempdir().unwrap();
        let jw = dir.path().join("book.jw");
        let info = LockInfo {
            host: platform::machine_name(),
            user: String::new(),
            pid: u32::MAX - 7,
            since: 1,
            heartbeat: 1,
            app: String::new(),
            token: "x".into(),
        };
        fs::write(lock_path(&jw), serde_json::to_string(&info).unwrap()).unwrap();
        assert!(matches!(Lock::acquire(&jw, false, 10).unwrap(), Acquired::Held(_)));
    }
}
