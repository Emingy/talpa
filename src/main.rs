mod config;
mod core;
mod ui;
mod utils;

use std::path::PathBuf;
use std::sync::{mpsc, Arc};

use core::proxy_service::{Cmd, ProxyState};
use tracing_subscriber::EnvFilter;

fn default_config_path() -> PathBuf {
    if cfg!(debug_assertions) {
        return PathBuf::from("config.toml");
    }
    std::env::var("HOME")
        .map(|h| {
            PathBuf::from(h)
                .join("Library/Application Support")
                .join(env!("BUNDLE_NAME"))
                .join("config.toml")
        })
        .unwrap_or_else(|_| PathBuf::from("config.toml"))
}

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("talpa=info,warn")),
        )
        .init();

    let config_path = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(default_config_path);

    let config = Arc::new(config::Config::load_or_create(&config_path));
    let matcher = Arc::new(core::matcher::Matcher::new(&config.domains, &config.ips));
    let state = Arc::new(ProxyState::new());

    let (cmd_tx, cmd_rx) = tokio::sync::mpsc::channel::<Cmd>(4);
    let (done_tx, done_rx) = mpsc::sync_channel::<()>(1);

    {
        let config = config.clone();
        let matcher = matcher.clone();
        let state = state.clone();
        std::thread::spawn(move || {
            core::proxy_service::run_thread(config, matcher, state, cmd_rx, done_tx);
        });
    }

    ui::macos::menubar::TrayApp::run(config_path, config, state, cmd_tx, done_rx);
}
