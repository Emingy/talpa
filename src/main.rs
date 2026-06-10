// No console window on Windows — this is a tray app; logs go to a file instead.
#![cfg_attr(windows, windows_subsystem = "windows")]

mod config;
mod core;
mod paths;
mod platform;
mod ui;
mod update;

use std::path::PathBuf;

use config::Config;

fn main() -> anyhow::Result<()> {
    // On Unix, re-launch elevated via the platform's graphical auth prompt if not
    // already root (the tool needs root for TUN + routing/DNS). Windows uses the
    // embedded admin manifest (build.rs) instead.
    #[cfg(unix)]
    ensure_root();

    init_logging();

    // First CLI arg overrides the config path; otherwise use the standard
    // per-OS config location, creating a default template on first run.
    let config_path = match std::env::args().nth(1) {
        Some(arg) => PathBuf::from(arg),
        None => paths::default_config_path(),
    };
    if !config_path.exists() {
        match paths::create_default_config(&config_path) {
            Ok(()) => log::info!("[CONFIG] created default config at {}", config_path.display()),
            Err(e) => log::error!("[CONFIG] failed to create {}: {e}", config_path.display()),
        }
    }
    Config::load(&config_path)?;

    // Hands the main thread to the status-bar/tray event loop. Start/stop and
    // config management happen from the tray menu; this never returns.
    ui::run(config_path.to_string_lossy().into_owned());
    Ok(())
}

/// On Unix, ensures the process runs as root. If it doesn't, re-launches the
/// same binary (with the same args) through the platform's graphical auth prompt
/// (macOS: `osascript`; Linux: `pkexec`). `elevate` never returns — it replaces
/// this unprivileged launcher with the elevated instance and exits.
#[cfg(unix)]
fn ensure_root() {
    // SAFETY: `geteuid` takes no arguments and is always safe to call.
    if unsafe { libc::geteuid() } == 0 {
        return;
    }

    let Ok(exe) = std::env::current_exe() else {
        eprintln!("talpa: cannot determine own path to self-elevate; run with sudo.");
        std::process::exit(1);
    };

    elevate(&exe);
}

/// macOS: re-launch via osascript's "administrator privileges" prompt, then exit.
///
/// We **spawn osascript detached and exit immediately** rather than waiting on it.
/// `do shell script … with administrator privileges` blocks until the launched
/// command finishes, but the elevated instance is the long-running tray app — so
/// waiting here would keep this unprivileged launcher (and, when started from
/// Finder, its Dock icon) alive for the entire session. The orphaned osascript is
/// reparented to launchd and carries the elevated app. Output is discarded and the
/// path/args are shell- and AppleScript-escaped.
#[cfg(target_os = "macos")]
fn elevate(exe: &std::path::Path) -> ! {
    let mut cmd = sh_quote(&exe.to_string_lossy());
    for arg in std::env::args().skip(1) {
        cmd.push(' ');
        cmd.push_str(&sh_quote(&arg));
    }
    cmd.push_str(" >/dev/null 2>&1");

    let script = format!(
        "do shell script \"{}\" with administrator privileges",
        as_escape(&cmd)
    );
    match std::process::Command::new("osascript")
        .arg("-e")
        .arg(&script)
        .spawn()
    {
        Ok(_) => std::process::exit(0),
        Err(e) => {
            eprintln!("talpa: failed to launch elevation prompt: {e}");
            std::process::exit(1);
        }
    }
}

/// Linux: re-launch via `pkexec` (PolicyKit graphical prompt). pkexec scrubs the
/// environment, so the GUI/session variables the tray needs (display, D-Bus, XDG
/// runtime) are forwarded explicitly through `env`. Requires a PolicyKit agent in
/// the session; on Wayland the compositor may still refuse a root GUI client, in
/// which case `sudo` from a terminal is the fallback.
#[cfg(target_os = "linux")]
fn elevate(exe: &std::path::Path) -> ! {
    let mut cmd = std::process::Command::new("pkexec");
    cmd.arg("env");
    for var in [
        "DISPLAY",
        "WAYLAND_DISPLAY",
        "XAUTHORITY",
        "XDG_RUNTIME_DIR",
        "DBUS_SESSION_BUS_ADDRESS",
        "XDG_SESSION_TYPE",
    ] {
        if let Ok(val) = std::env::var(var) {
            cmd.arg(format!("{var}={val}"));
        }
    }
    cmd.arg(exe);
    cmd.args(std::env::args().skip(1));
    // pkexec runs the elevated process as its child; wait on it so this launcher
    // mirrors its lifetime (no Dock/taskbar entry exists for it on Linux anyway).
    match cmd.status() {
        Ok(s) if s.success() => std::process::exit(0),
        _ => {
            eprintln!(
                "talpa needs root privileges; elevation was cancelled or failed. Try running with sudo."
            );
            std::process::exit(1);
        }
    }
}

/// Wraps `s` in single quotes for the shell, escaping any embedded single quote.
#[cfg(target_os = "macos")]
fn sh_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

/// Escapes `\` and `"` so `s` can sit inside an AppleScript double-quoted string.
#[cfg(target_os = "macos")]
fn as_escape(s: &str) -> String {
    s.replace('\\', r"\\").replace('"', "\\\"")
}

/// Sets up logging to a file in the standard log directory, plus the terminal
/// where one exists (Unix). On Windows there is no console (see
/// `windows_subsystem`), so the file is the only sink. The file is truncated on
/// each startup, so every run begins with a fresh log.
fn init_logging() {
    use simplelog::{CombinedLogger, Config as LogConfig, LevelFilter, SharedLogger};

    let mut loggers: Vec<Box<dyn SharedLogger>> = Vec::new();

    if let Some(file_path) = paths::log_file() {
        if let Some(parent) = file_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(file) = std::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&file_path)
        {
            loggers.push(simplelog::WriteLogger::new(
                LevelFilter::Info,
                LogConfig::default(),
                file,
            ));
        }
    }

    #[cfg(not(windows))]
    loggers.push(simplelog::TermLogger::new(
        LevelFilter::Info,
        LogConfig::default(),
        simplelog::TerminalMode::Mixed,
        simplelog::ColorChoice::Auto,
    ));

    let _ = CombinedLogger::init(loggers);
}
