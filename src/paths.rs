//! Standard per-OS locations for the config file and log directory.
//!
//! | OS      | config dir                          | log dir                          |
//! |---------|-------------------------------------|----------------------------------|
//! | macOS   | `~/Library/Application Support/talpa` | `~/Library/Logs/talpa`   |
//! | Linux   | `~/.config/talpa`                | `~/.local/state/talpa`        |
//! | Windows | `%APPDATA%\talpa`                | `%LOCALAPPDATA%\talpa\logs`   |
//!
//! The tool runs elevated; on Unix, `sudo` sets `$HOME` to root's. We honour
//! `SUDO_USER` so files land in the human user's directories, not root's.

use std::path::PathBuf;

const APP: &str = "talpa";

/// Default config template written on first run when no config exists.
const DEFAULT_CONFIG: &str = include_str!("../config.example.yml");

/// Home directory of the human user, accounting for `sudo` on Unix.
#[cfg(unix)]
fn user_home() -> Option<PathBuf> {
    if let Ok(user) = std::env::var("SUDO_USER")
        && !user.is_empty()
        && user != "root"
    {
        let base = if cfg!(target_os = "macos") {
            format!("/Users/{user}")
        } else {
            format!("/home/{user}")
        };
        let home = PathBuf::from(base);
        if home.is_dir() {
            return Some(home);
        }
    }
    std::env::var_os("HOME").map(PathBuf::from)
}

#[cfg(target_os = "macos")]
fn config_dir() -> Option<PathBuf> {
    Some(user_home()?.join("Library/Application Support").join(APP))
}
#[cfg(target_os = "macos")]
fn log_dir_inner() -> Option<PathBuf> {
    Some(user_home()?.join("Library/Logs").join(APP))
}

#[cfg(target_os = "linux")]
fn config_dir() -> Option<PathBuf> {
    Some(user_home()?.join(".config").join(APP))
}
#[cfg(target_os = "linux")]
fn log_dir_inner() -> Option<PathBuf> {
    Some(user_home()?.join(".local/state").join(APP))
}

#[cfg(target_os = "windows")]
fn config_dir() -> Option<PathBuf> {
    Some(PathBuf::from(std::env::var_os("APPDATA")?).join(APP))
}
#[cfg(target_os = "windows")]
fn log_dir_inner() -> Option<PathBuf> {
    Some(PathBuf::from(std::env::var_os("LOCALAPPDATA")?).join(APP).join("logs"))
}

/// The standard config file path, falling back to `./config.yml` if the
/// platform directory can't be determined.
pub fn default_config_path() -> PathBuf {
    config_dir()
        .map(|d| d.join("config.yml"))
        .unwrap_or_else(|| PathBuf::from("config.yml"))
}

/// The log file path (`<log dir>/talpa.log`), or `None` if the directory
/// can't be resolved.
pub fn log_file() -> Option<PathBuf> {
    Some(log_dir_inner()?.join("talpa.log"))
}

/// Writes the default config template to `path` (creating parent dirs). Used on
/// first run when no config exists yet.
pub fn create_default_config(path: &std::path::Path) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, DEFAULT_CONFIG)
}
