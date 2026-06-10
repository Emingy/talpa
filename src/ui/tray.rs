use anyhow::Result;
use tao::event_loop::EventLoopProxy;
use tray_icon::menu::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu};
use tray_icon::{TrayIcon, TrayIconBuilder};

use super::controller::{Controller, Status};
use super::UserEvent;
use crate::config;
use crate::update::UpdateStatus;

/// A 32×32 RGBA dot used as the tray icon on every platform; its color encodes
/// the pipeline [`Status`]. Kept non-template on macOS so the color shows.
fn status_icon(status: Status) -> tray_icon::Icon {
    const SIZE: i32 = 32;
    let (r, g, b) = match status {
        Status::Running => (46, 204, 113),    // green
        Status::Processing => (120, 190, 240), // light blue
        Status::Error => (231, 76, 60),       // red
        Status::Stopped => (149, 165, 166),   // grey
    };
    let c = (SIZE - 1) as f32 / 2.0;
    let radius = c - 1.0;
    let mut rgba = Vec::with_capacity((SIZE * SIZE * 4) as usize);
    for y in 0..SIZE {
        for x in 0..SIZE {
            let inside = {
                let dx = x as f32 - c;
                let dy = y as f32 - c;
                (dx * dx + dy * dy).sqrt() <= radius
            };
            let a = if inside { 255 } else { 0 };
            rgba.extend_from_slice(&[r, g, b, a]);
        }
    }
    tray_icon::Icon::from_rgba(rgba, SIZE as u32, SIZE as u32).expect("valid tray icon")
}

/// Outcome of handling a menu click, so the event loop knows when to quit.
#[derive(PartialEq)]
pub enum Action {
    None,
    Quit,
}

// The live menu and the item handles we compare click ids against / mutate.
// muda items are `Rc`-backed clones, so holding them here keeps them alive and
// lets us flip their enabled/checked state.
struct MenuModel {
    menu: Menu,
    // Held only to keep the Rc-backed item alive; its text never changes.
    _version: MenuItem,
    update: MenuItem,
    status: MenuItem,
    start: MenuItem,
    stop: MenuItem,
    open: MenuItem,
    logs: MenuItem,
    reload: MenuItem,
    quit: MenuItem,
    domains: Vec<(String, CheckMenuItem)>,
    _submenu: Submenu,
}

/// The status-bar UI: a tray icon plus the controller it drives.
pub struct Ui {
    tray: TrayIcon,
    controller: Controller,
    menu: MenuModel,
    /// Latest update-check outcome; `None` while the check is still in flight.
    /// Cached so it can be re-applied after a menu rebuild.
    update_status: Option<UpdateStatus>,
}

impl Ui {
    /// Must be called on the main thread after the event loop has initialised
    /// (macOS requires `NSApplication` to exist before the status item).
    pub fn new(config_path: String, proxy: EventLoopProxy<UserEvent>) -> Result<Self> {
        // A second proxy for the background update checker (the first is moved
        // into the controller).
        let updater_proxy = proxy.clone();
        let controller = Controller::new(config_path, proxy)?;
        let menu = Self::build_menu()?;
        let tray = TrayIconBuilder::new()
            .with_menu(Box::new(menu.menu.clone()))
            .with_icon(status_icon(Status::Stopped))
            .build()?;

        let ui = Self { tray, controller, menu, update_status: None };
        ui.refresh_state();

        // Check GitHub for a newer release; the result comes back as a
        // `UserEvent::UpdateChecked` handled on the main thread.
        crate::update::spawn_check(move |status| {
            let _ = updater_proxy.send_event(UserEvent::UpdateChecked(status));
        });

        Ok(ui)
    }

    fn build_menu() -> Result<MenuModel> {
        let menu = Menu::new();

        let version = MenuItem::new(format!("Talpa v{}", crate::update::CURRENT_VERSION), false, None);
        // Disabled until the background check resolves; becomes a clickable
        // "update available" button (or a status line) via `apply_update_status`.
        let update = MenuItem::new("Checking for updates…", false, None);
        let status = MenuItem::new("Status: stopped", false, None);
        let start = MenuItem::new("Start", true, None);
        let stop = MenuItem::new("Stop", true, None);
        let open = MenuItem::new("Open config…", true, None);
        let logs = MenuItem::new("Open logs…", true, None);
        let reload = MenuItem::new("Reload config", true, None);
        let quit = MenuItem::new("Quit", true, None);

        let submenu = Submenu::new("Domains", true);
        let mut domains = Vec::new();
        let cfg = config::config();
        if cfg.domains.is_empty() {
            submenu.append(&MenuItem::new("(none configured)", false, None))?;
        } else {
            for mask in &cfg.domains {
                let item = CheckMenuItem::new(mask, true, config::domain_enabled(mask), None);
                submenu.append(&item)?;
                domains.push((mask.clone(), item));
            }
        }

        menu.append(&version)?;
        menu.append(&update)?;
        menu.append(&PredefinedMenuItem::separator())?;
        menu.append(&status)?;
        menu.append(&PredefinedMenuItem::separator())?;
        menu.append(&start)?;
        menu.append(&stop)?;
        menu.append(&PredefinedMenuItem::separator())?;
        menu.append(&submenu)?;
        menu.append(&PredefinedMenuItem::separator())?;
        menu.append(&open)?;
        menu.append(&logs)?;
        menu.append(&reload)?;
        menu.append(&PredefinedMenuItem::separator())?;
        menu.append(&quit)?;

        Ok(MenuModel {
            menu,
            _version: version,
            update,
            status,
            start,
            stop,
            open,
            logs,
            reload,
            quit,
            domains,
            _submenu: submenu,
        })
    }

    /// Rebuilds the whole menu — used after a reload, since the domain list may
    /// have changed.
    fn rebuild_menu(&mut self) {
        match Self::build_menu() {
            Ok(model) => {
                self.tray.set_menu(Some(Box::new(model.menu.clone())));
                self.menu = model;
                self.refresh_state();
                // The fresh menu's update item starts in "checking" state; restore
                // whatever the last check found.
                self.apply_update_status();
            }
            Err(e) => log::error!("[UI] failed to rebuild menu: {e}"),
        }
    }

    /// Records the update-check result and reflects it in the menu.
    pub fn on_update_checked(&mut self, status: UpdateStatus) {
        match &status {
            UpdateStatus::Available { version, .. } => {
                log::info!("[UI] update available: v{version}")
            }
            UpdateStatus::UpToDate => log::info!("[UI] up to date"),
            UpdateStatus::Failed => log::info!("[UI] update check failed"),
        }
        self.update_status = Some(status);
        self.apply_update_status();
    }

    /// Sets the update menu item's label and enabled state from the cached
    /// [`UpdateStatus`]. Only [`UpdateStatus::Available`] makes it clickable.
    fn apply_update_status(&self) {
        let (text, enabled) = match &self.update_status {
            Some(UpdateStatus::Available { version, .. }) => {
                (format!("⬆ Update available: v{version}"), true)
            }
            Some(UpdateStatus::UpToDate) => ("✓ Up to date".to_owned(), false),
            Some(UpdateStatus::Failed) => ("Update check failed".to_owned(), false),
            None => ("Checking for updates…".to_owned(), false),
        };
        self.menu.update.set_text(text);
        self.menu.update.set_enabled(enabled);
    }

    /// Syncs item enabled/checked state, the status label, and the tray icon to
    /// the current pipeline [`Status`].
    fn refresh_state(&self) {
        let status = self.controller.status();
        // Start is offered when stopped or after an error (to retry); Stop only
        // while running. Both are disabled mid-operation.
        self.menu
            .start
            .set_enabled(matches!(status, Status::Stopped | Status::Error));
        self.menu.stop.set_enabled(status == Status::Running);
        self.menu.status.set_text(match status {
            Status::Stopped => "Status: stopped",
            Status::Processing => "Status: working…",
            Status::Running => "Status: running",
            Status::Error => "Status: error",
        });
        let _ = self.tray.set_icon(Some(status_icon(status)));
    }

    /// Settles the UI after a spawned start finishes.
    pub fn on_pipeline_started(&mut self, ok: bool) {
        self.controller.on_started(ok);
        self.refresh_state();
    }

    /// Settles the UI after a spawned stop finishes.
    pub fn on_pipeline_stopped(&mut self) {
        self.controller.on_stopped();
        self.refresh_state();
    }

    /// Dispatches a menu click. Returns [`Action::Quit`] when the app should exit.
    pub fn handle_menu(&mut self, event: &MenuEvent) -> Action {
        let id = event.id();

        if id == self.menu.update.id() {
            self.open_release();
        } else if id == self.menu.start.id() {
            self.controller.start();
            self.refresh_state();
        } else if id == self.menu.stop.id() {
            self.controller.stop();
            self.refresh_state();
        } else if id == self.menu.reload.id() {
            self.controller.reload();
            self.rebuild_menu();
        } else if id == self.menu.open.id() {
            self.open_config();
        } else if id == self.menu.logs.id() {
            self.open_logs();
        } else if id == self.menu.quit.id() {
            return Action::Quit;
        } else if let Some((mask, item)) = self.menu.domains.iter().find(|(_, it)| it.id() == id) {
            let enabled = item.is_checked();
            config::set_domain_enabled(mask, enabled);
            log::info!("[UI] domain {mask} {}", if enabled { "enabled" } else { "disabled" });
            if self.controller.is_running() {
                self.controller.reconcile_dns();
            }
        }

        Action::None
    }

    fn open_release(&self) {
        // Best-effort: open the release page for the available update.
        use crate::platform::{ShellOpen, Sys};
        if let Some(UpdateStatus::Available { url, .. }) = &self.update_status
            && let Err(e) = Sys::open_url(url)
        {
            log::error!("[UI] failed to open release page: {e}");
        }
    }

    fn open_config(&self) {
        // Best-effort: open the config file in the OS default handler.
        use crate::platform::{ShellOpen, Sys};
        if let Err(e) = Sys::open_path(self.controller.config_path()) {
            log::error!("[UI] failed to open config: {e}");
        }
    }

    fn open_logs(&self) {
        // Best-effort: open the log file in the OS default handler.
        use crate::platform::{ShellOpen, Sys};
        match crate::paths::log_file() {
            Some(path) => {
                if let Err(e) = Sys::open_path(&path.to_string_lossy()) {
                    log::error!("[UI] failed to open logs: {e}");
                }
            }
            None => log::error!("[UI] log file path unavailable"),
        }
    }

    /// Cleans up before the process exits. Uses the blocking teardown so routes
    /// and DNS state are reverted before `process::exit` (the async `stop` would
    /// not finish in time).
    pub fn shutdown(&mut self) {
        self.controller.shutdown();
    }
}
