//! Everything that differs between operating systems lives here.
use std::path::{Path, PathBuf};

#[cfg(not(target_os = "ios"))]
mod desktop;
#[cfg(not(target_os = "ios"))]
pub use desktop::*;
#[cfg(not(target_os = "ios"))]
use keyring::Entry;
#[cfg(target_os = "ios")]
use keyring_core::Entry;

pub fn primary_name() -> &'static str {
    if cfg!(target_os = "macos") { "Cmd" } else { "Ctrl" }
}

pub fn default_sans() -> &'static str {
    if cfg!(windows) {
        "Segoe UI"
    } else if cfg!(target_os = "macos") {
        "Helvetica Neue"
    } else {
        "DejaVu Sans"
    }
}
pub fn default_mono() -> &'static str {
    if cfg!(windows) {
        "Consolas"
    } else if cfg!(target_os = "macos") {
        "Menlo"
    } else {
        "DejaVu Sans Mono"
    }
}

pub fn workspace_file(path: &Path) -> Option<(PathBuf, PathBuf)> {
    let absolute = path.canonicalize().ok()?;
    Some((absolute.parent()?.to_path_buf(), PathBuf::from(absolute.file_name()?)))
}
pub fn save_destination(path: &Path) -> Option<PathBuf> {
    Some(path.parent()?.canonicalize().ok()?.join(path.file_name()?))
}

/// Per-user application data: settings, session, project working copies.
/// `JUSTWRITE_DATA` overrides it (tests, portable installs).
pub fn data_directory() -> PathBuf {
    let path = if let Some(path) = std::env::var_os("JUSTWRITE_DATA") {
        PathBuf::from(path)
    } else if cfg!(windows) {
        std::env::var_os("APPDATA")
            .map(|p| PathBuf::from(p).join("JustWrite"))
            .unwrap_or_else(exe_directory)
    } else if cfg!(target_os = "macos") {
        std::env::var_os("HOME")
            .map(|p| PathBuf::from(p).join("Library/Application Support/JustWrite"))
            .unwrap_or_else(exe_directory)
    } else {
        std::env::var_os("XDG_DATA_HOME")
            .map(|p| PathBuf::from(p).join("justwrite"))
            .or_else(|| std::env::var_os("HOME").map(|p| PathBuf::from(p).join(".local/share/justwrite")))
            .unwrap_or_else(exe_directory)
    };
    let _ = std::fs::create_dir_all(&path);
    path
}
fn exe_directory() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(Path::to_path_buf))
        .unwrap_or_else(|| PathBuf::from("."))
}

/// Opens a web or mail link in the default app. Other schemes are refused.
pub fn open_url(url: &str) -> bool {
    let allowed = ["http://", "https://", "mailto:"].iter().any(|s| url.to_lowercase().starts_with(s));
    if !allowed || url.chars().any(char::is_control) {
        return false;
    }
    open_with_system(url)
}
/// Opens a file (an exported PDF, say) in its default app.
pub fn open_path(path: &Path) -> bool {
    open_with_system(&path.to_string_lossy())
}
fn open_with_system(target: &str) -> bool {
    use std::process::{Command, Stdio};
    let mut command = if cfg!(windows) {
        let mut c = Command::new("rundll32");
        c.args(["url.dll,FileProtocolHandler", target]);
        c
    } else if cfg!(target_os = "macos") {
        let mut c = Command::new("open");
        c.arg(target);
        c
    } else {
        let mut c = Command::new("xdg-open");
        c.arg(target);
        c
    };
    command.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn().is_ok()
}

/// An entry in the operating system's secret store (Credential Manager, Keychain, Secret Service).
pub fn secret(name: &str) -> Result<Entry, String> {
    #[cfg(target_os = "ios")]
    {
        static STORE: std::sync::OnceLock<Result<(), String>> = std::sync::OnceLock::new();
        STORE
            .get_or_init(|| {
                let store = apple_native_keyring_store::protected::Store::new().map_err(|e| e.to_string())?;
                keyring_core::set_default_store(store);
                Ok(())
            })
            .clone()?;
    }
    Entry::new(crate::APP_NAME, name).map_err(|e| format!("The system's secret store is not available: {e}"))
}

/// A stable name for this computer, shown when two machines open the same project.
pub fn machine_name() -> String {
    if cfg!(target_os = "ios") {
        return "iPhone".into();
    }
    for key in ["COMPUTERNAME", "HOSTNAME", "HOST"] {
        if let Some(name) = std::env::var_os(key).filter(|n| !n.is_empty()) {
            return name.to_string_lossy().into_owned();
        }
    }
    if let Ok(name) = std::fs::read_to_string("/etc/hostname") {
        let name = name.trim();
        if !name.is_empty() {
            return name.to_owned();
        }
    }
    std::process::Command::new("hostname")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "this computer".into())
}
pub fn user_name() -> String {
    ["USERNAME", "USER"]
        .iter()
        .find_map(|k| std::env::var_os(k))
        .map(|v| v.to_string_lossy().into_owned())
        .unwrap_or_default()
}
/// Best-effort liveness check used to tell a stale lock file from a live one on this machine.
pub fn process_alive(pid: u32) -> bool {
    if cfg!(target_os = "linux") {
        return Path::new(&format!("/proc/{pid}")).exists();
    }
    use std::process::{Command, Stdio};
    let output = if cfg!(windows) {
        Command::new("tasklist").args(["/FI", &format!("PID eq {pid}"), "/NH"]).stderr(Stdio::null()).output()
    } else {
        Command::new("kill").args(["-0", &pid.to_string()]).stderr(Stdio::null()).output()
    };
    match output {
        Ok(o) if cfg!(windows) => String::from_utf8_lossy(&o.stdout).contains(&pid.to_string()),
        Ok(o) => o.status.success(),
        Err(_) => true,
    }
}

/// Registers `.jw` with the shell for the current user. macOS does this through the app bundle's
/// Info.plist (see `packaging/macos`); Linux through the `.desktop` file in `packaging/linux`.
pub fn register_file_type() -> Result<String, String> {
    #[cfg(windows)]
    {
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        let exe = exe.to_string_lossy().into_owned();
        let entries: [(&str, &str); 4] = [
            (r"HKCU\Software\Classes\.jw", "JustWrite.Project"),
            (r"HKCU\Software\Classes\JustWrite.Project", "JustWrite project"),
            (r"HKCU\Software\Classes\JustWrite.Project\DefaultIcon", &format!("\"{exe}\",0")),
            (r"HKCU\Software\Classes\JustWrite.Project\shell\open\command", &format!("\"{exe}\" \"%1\"")),
        ];
        for (key, value) in entries {
            let status = std::process::Command::new("reg")
                .args(["add", key, "/ve", "/d", value, "/f"])
                .output()
                .map_err(|e| e.to_string())?;
            if !status.status.success() {
                return Err(String::from_utf8_lossy(&status.stderr).into_owned());
            }
        }
        return Ok("Registered .jw files for this user".into());
    }
    #[allow(unreachable_code)]
    Err("File types are registered by the installer or app bundle on this system".into())
}
