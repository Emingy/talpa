use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

use tray_icon::menu::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu};
use tray_icon::{TrayIcon, TrayIconBuilder};
use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::platform::macos::{ActivationPolicy, EventLoopBuilderExtMacOS};
use winit::window::WindowId;

use tracing::error;

use crate::core::proxy_service::{Cmd, ProxyState};
use crate::ui::macos::dialogs::{dialog_choose, dialog_input, dialog_password};
use crate::utils::config_helpers::{ensure_dns, ensure_sp, ensure_tunnel};
use crate::utils::icons::circle_icon;
use crate::utils::updater;

pub struct TrayApp {
    config_path: PathBuf,
    config: Arc<crate::config::Config>,
    state: Arc<ProxyState>,
    cmd_tx: tokio::sync::mpsc::Sender<Cmd>,
    done_rx: mpsc::Receiver<()>,
    tray: Option<TrayIcon>,
    last_status: Option<u8>, // 0=stopped, 1=degraded, 2=running

    // ── top-level items ──
    status_item: MenuItem,
    toggle_item: MenuItem,
    quit_item: MenuItem,

    // ── domains ──
    add_domain_item: MenuItem,
    remove_domain_item: MenuItem,

    // ── IP rules ──
    add_ip_item: MenuItem,
    remove_ip_item: MenuItem,

    // ── SSH tunnel ──
    ssh_host_item: MenuItem,
    ssh_user_item: MenuItem,
    ssh_password_item: MenuItem,
    ssh_clear_pw_item: MenuItem,
    ssh_ssh_port_item: MenuItem,
    ssh_tunnel_port_item: MenuItem,

    // ── system proxy ──
    sp_enable_item: CheckMenuItem,
    sp_npm_item: CheckMenuItem,
    sp_git_item: CheckMenuItem,
    sp_curl_item: CheckMenuItem,

    // ── DNS ──
    dns_enable_item: CheckMenuItem,
    dns_listen_item: MenuItem,
    dns_upstream_item: MenuItem,
    dns_fallback_item: MenuItem,

    // ── upstream ──
    up_addr_item: MenuItem,
    up_port_item: MenuItem,

    // ── about ──
    version_item: MenuItem,
    update_item: MenuItem,
    latest_version: Arc<Mutex<Option<String>>>,
    last_latest: Option<String>,
}

impl TrayApp {
    pub fn new(
        config_path: PathBuf,
        config: Arc<crate::config::Config>,
        state: Arc<ProxyState>,
        cmd_tx: tokio::sync::mpsc::Sender<Cmd>,
        done_rx: mpsc::Receiver<()>,
    ) -> Self {
        let t = config.ssh_tunnel.as_ref();
        let sp = config.system_proxy.as_ref();
        let dns = config.dns.as_ref();

        let app = Self {
            config_path,
            state,
            cmd_tx,
            done_rx,
            config: config.clone(),
            tray: None,
            last_status: None,

            status_item: MenuItem::new("○ Proxy: Stopped", false, None),
            toggle_item: MenuItem::new("Start", true, None),
            quit_item: MenuItem::new("Quit", true, None),

            add_domain_item: MenuItem::new("Add domain...", true, None),
            remove_domain_item: MenuItem::new("Remove domain...", true, None),

            add_ip_item: MenuItem::new("Add IP/subnet...", true, None),
            remove_ip_item: MenuItem::new("Remove IP/subnet...", true, None),

            ssh_host_item:        MenuItem::new("", true, None),
            ssh_user_item:        MenuItem::new("", true, None),
            ssh_password_item:    MenuItem::new("", true, None),
            ssh_clear_pw_item:    MenuItem::new("Clear password", t.and_then(|t| t.password.as_ref()).is_some(), None),
            ssh_ssh_port_item:    MenuItem::new("", true, None),
            ssh_tunnel_port_item: MenuItem::new("", true, None),

            sp_enable_item: CheckMenuItem::new("Enable system proxy", true, sp.map(|s| s.enabled).unwrap_or(false),      None),
            sp_npm_item:    CheckMenuItem::new("  Configure npm",      true, sp.map(|s| s.configure_npm).unwrap_or(true), None),
            sp_git_item:    CheckMenuItem::new("  Configure git",      true, sp.map(|s| s.configure_git).unwrap_or(true), None),
            sp_curl_item:   CheckMenuItem::new("  Configure curl",     true, sp.map(|s| s.configure_curl).unwrap_or(true), None),

            dns_enable_item:   CheckMenuItem::new("Enable DNS proxy", true, dns.is_some(), None),
            dns_listen_item:   MenuItem::new("", dns.is_some(), None),
            dns_upstream_item: MenuItem::new("", dns.is_some(), None),
            dns_fallback_item: MenuItem::new("", dns.is_some(), None),

            up_addr_item: MenuItem::new("", true, None),
            up_port_item: MenuItem::new("", true, None),

            version_item: MenuItem::new(
                format!("Version: {}", env!("CARGO_PKG_VERSION")),
                false,
                None,
            ),
            update_item: MenuItem::new("", false, None),
            latest_version: Arc::new(Mutex::new(None)),
            last_latest: None,
        };
        updater::spawn_update_check(env!("CARGO_PKG_REPOSITORY"), app.latest_version.clone());
        app.sync_item_texts();
        app
    }

    pub fn run(config_path: PathBuf, config: Arc<crate::config::Config>, state: Arc<ProxyState>, cmd_tx: tokio::sync::mpsc::Sender<Cmd>, done_rx: mpsc::Receiver<()>) {
        let event_loop = EventLoop::builder()
            .with_activation_policy(ActivationPolicy::Accessory)
            .build()
            .unwrap();
        let mut app = TrayApp::new(config_path, config, state, cmd_tx, done_rx);
        event_loop.run_app(&mut app).unwrap();
    }

    fn sync_item_texts(&self) {
        let t = self.config.ssh_tunnel.as_ref();
        let dns = self.config.dns.as_ref();
        let has_pw = t.and_then(|t| t.password.as_ref()).is_some();

        self.ssh_host_item.set_text(format!("Host: {}", t.map(|t| t.host.as_str()).unwrap_or("-")));
        self.ssh_user_item.set_text(format!("User: {}", t.map(|t| t.user.as_str()).unwrap_or("-")));
        self.ssh_password_item.set_text(if has_pw { "Password: ••••  (change...)" } else { "Set password..." });
        self.ssh_clear_pw_item.set_enabled(has_pw);
        self.ssh_ssh_port_item.set_text(format!("SSH port: {}", t.map(|t| t.ssh_port).unwrap_or(22)));
        self.ssh_tunnel_port_item.set_text(format!("Tunnel port: {}", t.map(|t| t.local_port).unwrap_or(10808)));

        let dns_on = dns.is_some();
        self.dns_listen_item.set_enabled(dns_on);
        self.dns_upstream_item.set_enabled(dns_on);
        self.dns_fallback_item.set_enabled(dns_on);
        self.dns_listen_item.set_text(format!("Listen port: {}", dns.map(|d| d.listen_port).unwrap_or(5300)));
        self.dns_upstream_item.set_text(format!("Upstream DNS: {}", dns.map(|d| d.upstream_dns.as_str()).unwrap_or("8.8.8.8:53")));
        self.dns_fallback_item.set_text(format!("Fallback DNS: {}", dns.map(|d| d.fallback_dns.as_str()).unwrap_or("8.8.8.8:53")));

        self.up_addr_item.set_text(format!("Address: {}", self.config.upstream.addr));
        self.up_port_item.set_text(format!("Port: {}", self.config.upstream.port));
    }

    fn build_menu(&self) -> Menu {
        let domain_sub = Submenu::new(format!("Domains ({})", self.config.domains.len()), true);
        for d in &self.config.domains {
            let _ = domain_sub.append(&MenuItem::new(d, false, None));
        }
        let _ = domain_sub.append(&PredefinedMenuItem::separator());
        let _ = domain_sub.append(&self.add_domain_item);
        let _ = domain_sub.append(&self.remove_domain_item);

        let ip_sub = Submenu::new(format!("IP Rules ({})", self.config.ips.len()), true);
        for ip in &self.config.ips {
            let _ = ip_sub.append(&MenuItem::new(ip, false, None));
        }
        let _ = ip_sub.append(&PredefinedMenuItem::separator());
        let _ = ip_sub.append(&self.add_ip_item);
        let _ = ip_sub.append(&self.remove_ip_item);

        let ssh_sub = Submenu::new("SSH Tunnel", true);
        let _ = ssh_sub.append_items(&[
            &self.ssh_host_item,
            &self.ssh_user_item,
            &self.ssh_ssh_port_item,
            &self.ssh_tunnel_port_item,
            &PredefinedMenuItem::separator(),
            &self.ssh_password_item,
            &self.ssh_clear_pw_item,
        ]);

        let sp_sub = Submenu::new("System Proxy", true);
        let _ = sp_sub.append_items(&[
            &self.sp_enable_item,
            &PredefinedMenuItem::separator(),
            &self.sp_npm_item,
            &self.sp_git_item,
            &self.sp_curl_item,
        ]);

        let dns_sub = Submenu::new("DNS Proxy", true);
        let _ = dns_sub.append_items(&[
            &self.dns_enable_item,
            &PredefinedMenuItem::separator(),
            &self.dns_listen_item,
            &self.dns_upstream_item,
            &self.dns_fallback_item,
        ]);

        let up_sub = Submenu::new("Upstream SOCKS5", true);
        let _ = up_sub.append_items(&[&self.up_addr_item, &self.up_port_item]);

        let settings_sub = Submenu::new("Settings", true);
        let _ = settings_sub.append_items(&[&ssh_sub, &sp_sub, &dns_sub, &up_sub]);

        let menu = Menu::new();
        let _ = menu.append_items(&[
            &self.status_item,
            &PredefinedMenuItem::separator(),
            &self.toggle_item,
            &PredefinedMenuItem::separator(),
            &domain_sub,
            &ip_sub,
            &settings_sub,
            &PredefinedMenuItem::separator(),
            &self.version_item,
        ]);
        if self.update_item.is_enabled() {
            let _ = menu.append(&self.update_item);
        }
        let _ = menu.append_items(&[&PredefinedMenuItem::separator(), &self.quit_item]);
        menu
    }

    fn refresh_menu(&self) {
        self.sync_item_texts();
        if let Some(tray) = &self.tray {
            tray.set_menu(Some(Box::new(self.build_menu())));
        }
    }

    fn update_config<F: FnOnce(&mut crate::config::Config)>(&mut self, f: F) {
        let mut c = (*self.config).clone();
        f(&mut c);
        if let Err(e) = c.save(&self.config_path) {
            error!("save config: {}", e);
            return;
        }
        let arc = Arc::new(c);
        self.config = arc.clone();
        let _ = self.cmd_tx.try_send(Cmd::Reload(arc));
        self.refresh_menu();
    }

    // ── event loop ────────────────────────────────────────────────────────────

    fn tick(&mut self, event_loop: &ActiveEventLoop) {
        while let Ok(ev) = MenuEvent::receiver().try_recv() {
            let id = ev.id().clone();
            self.dispatch(id, event_loop);
        }
        self.sync_status_icon();
        self.sync_update_item();
    }

    fn sync_update_item(&mut self) {
        let latest = self.latest_version.lock().unwrap().clone();
        if latest == self.last_latest { return; }
        self.last_latest = latest.clone();
        if let Some(ref tag) = latest {
            if updater::is_newer(env!("CARGO_PKG_VERSION"), tag) {
                self.update_item.set_text(format!("↑ Update available: {}", tag));
                self.update_item.set_enabled(true);
                self.refresh_menu();
            }
        }
    }

    fn sync_status_icon(&mut self) {
        let running    = self.state.running.load(Ordering::Relaxed);
        let connecting = self.state.connecting.load(Ordering::Relaxed);
        let tunnel_req = self.state.tunnel_required.load(Ordering::Relaxed);
        let tunnel_up  = self.state.tunnel_up.load(Ordering::Relaxed);
        let status: u8 = if running {
            if tunnel_req && !tunnel_up { 2 } else { 3 }
        } else if connecting {
            1
        } else {
            0
        };
        if self.last_status == Some(status) { return; }
        self.last_status = Some(status);
        if let Some(tray) = &self.tray {
            let (label, toggle, color) = match status {
                3 => ("● Proxy: Running",      "Stop",  [52u8,  199, 89]),
                2 => ("● Proxy: Tunnel down",  "Stop",  [255u8, 149,  0]),
                1 => ("◌ Proxy: Connecting…",  "Stop",  [0u8,   122, 255]),
                _ => ("○ Proxy: Stopped",      "Start", [142u8, 142, 147]),
            };
            let _ = tray.set_icon(Some(circle_icon(color)));
            self.status_item.set_text(label);
            self.toggle_item.set_text(toggle);
        }
    }

    // ── dispatch (pure router) ────────────────────────────────────────────────

    fn dispatch(&mut self, id: tray_icon::menu::MenuId, event_loop: &ActiveEventLoop) {
        if id == self.toggle_item.id()          { self.do_toggle(); return; }
        if id == self.quit_item.id()            { self.do_quit(event_loop); return; }
        if id == self.add_domain_item.id()      { self.do_add_domain(); return; }
        if id == self.remove_domain_item.id()   { self.do_remove_domain(); return; }
        if id == self.add_ip_item.id()          { self.do_add_ip(); return; }
        if id == self.remove_ip_item.id()       { self.do_remove_ip(); return; }
        if id == self.ssh_host_item.id()        { self.do_ssh_host(); return; }
        if id == self.ssh_user_item.id()        { self.do_ssh_user(); return; }
        if id == self.ssh_ssh_port_item.id()    { self.do_ssh_port(); return; }
        if id == self.ssh_tunnel_port_item.id() { self.do_tunnel_port(); return; }
        if id == self.ssh_password_item.id()    { self.do_ssh_password(); return; }
        if id == self.ssh_clear_pw_item.id()    { self.do_ssh_clear_pw(); return; }
        if id == self.sp_enable_item.id()       { self.do_sp_enable(); return; }
        if id == self.sp_npm_item.id()          { self.do_sp_npm(); return; }
        if id == self.sp_git_item.id()          { self.do_sp_git(); return; }
        if id == self.sp_curl_item.id()         { self.do_sp_curl(); return; }
        if id == self.dns_enable_item.id()      { self.do_dns_enable(); return; }
        if id == self.dns_listen_item.id()      { self.do_dns_listen(); return; }
        if id == self.dns_upstream_item.id()    { self.do_dns_upstream(); return; }
        if id == self.dns_fallback_item.id()    { self.do_dns_fallback(); return; }
        if id == self.up_addr_item.id()         { self.do_upstream_addr(); return; }
        if id == self.up_port_item.id()         { self.do_upstream_port(); return; }
        if id == self.update_item.id()          { self.do_open_update(); }
    }

    // ── action handlers ───────────────────────────────────────────────────────

    fn do_toggle(&mut self) {
        let active = self.state.running.load(Ordering::Relaxed)
            || self.state.connecting.load(Ordering::Relaxed);
        let _ = self.cmd_tx.try_send(if active { Cmd::Stop } else { Cmd::Start });
    }

    fn do_quit(&mut self, event_loop: &ActiveEventLoop) {
        let _ = self.cmd_tx.try_send(Cmd::Quit);
        let _ = self.done_rx.recv_timeout(Duration::from_secs(5));
        event_loop.exit();
    }

    fn do_add_domain(&mut self) {
        if let Some(d) = dialog_input("Add Domain", "Enter domain pattern (e.g. **.example.com):", "") {
            let d = d.trim().to_string();
            if !d.is_empty() && !self.config.domains.contains(&d) {
                self.update_config(|c| c.domains.push(d.clone()));
            }
        }
    }

    fn do_remove_domain(&mut self) {
        let domains = self.config.domains.clone();
        if domains.is_empty() { return; }
        if let Some(sel) = dialog_choose(&domains, "Remove Domain", "Select domain to remove:") {
            self.update_config(|c| c.domains.retain(|d| *d != sel));
        }
    }

    fn do_add_ip(&mut self) {
        if let Some(v) = dialog_input("Add IP Rule", "Enter IP address or subnet (e.g. 10.0.0.0/8 or 1.2.3.4):", "") {
            let v = v.trim().to_string();
            if !v.is_empty() && !self.config.ips.contains(&v) {
                self.update_config(|c| c.ips.push(v.clone()));
            }
        }
    }

    fn do_remove_ip(&mut self) {
        let ips = self.config.ips.clone();
        if ips.is_empty() { return; }
        if let Some(sel) = dialog_choose(&ips, "Remove IP Rule", "Select IP/subnet to remove:") {
            self.update_config(|c| c.ips.retain(|ip| *ip != sel));
        }
    }

    fn do_ssh_host(&mut self) {
        let cur = self.config.ssh_tunnel.as_ref().map(|t| t.host.clone()).unwrap_or_default();
        if let Some(v) = dialog_input("SSH Host", "Enter SSH host:", &cur) {
            self.update_config(|c| ensure_tunnel(c).host = v);
        }
    }

    fn do_ssh_user(&mut self) {
        let cur = self.config.ssh_tunnel.as_ref().map(|t| t.user.clone()).unwrap_or_default();
        if let Some(v) = dialog_input("SSH User", "Enter SSH username:", &cur) {
            self.update_config(|c| ensure_tunnel(c).user = v);
        }
    }

    fn do_ssh_port(&mut self) {
        let cur = self.config.ssh_tunnel.as_ref().map(|t| t.ssh_port.to_string()).unwrap_or_else(|| "22".into());
        if let Some(v) = dialog_input("SSH Port", "Enter SSH port:", &cur) {
            if let Ok(p) = v.parse::<u16>() {
                self.update_config(|c| ensure_tunnel(c).ssh_port = p);
            }
        }
    }

    fn do_tunnel_port(&mut self) {
        let cur = self.config.ssh_tunnel.as_ref().map(|t| t.local_port.to_string()).unwrap_or_else(|| "10808".into());
        if let Some(v) = dialog_input("Tunnel Port", "Enter local tunnel port:", &cur) {
            if let Ok(p) = v.parse::<u16>() {
                self.update_config(|c| {
                    let t = ensure_tunnel(c);
                    t.local_port = p;
                    c.upstream.port = p;
                });
            }
        }
    }

    fn do_ssh_password(&mut self) {
        let prompt = format!(
            "Password for {}@{}:",
            self.config.ssh_tunnel.as_ref().map(|t| t.user.as_str()).unwrap_or("user"),
            self.config.ssh_tunnel.as_ref().map(|t| t.host.as_str()).unwrap_or("host"),
        );
        if let Some(pw) = dialog_password("SSH Password", &prompt) {
            self.update_config(|c| ensure_tunnel(c).password = Some(pw));
        }
    }

    fn do_ssh_clear_pw(&mut self) {
        self.update_config(|c| {
            if let Some(t) = &mut c.ssh_tunnel { t.password = None; }
        });
    }

    fn do_sp_enable(&mut self) {
        let on = self.sp_enable_item.is_checked();
        self.update_config(|c| {
            c.system_proxy = if on {
                Some(c.system_proxy.clone().unwrap_or_default())
            } else {
                None
            };
        });
    }

    fn do_sp_npm(&mut self) {
        let v = self.sp_npm_item.is_checked();
        self.update_config(|c| ensure_sp(c).configure_npm = v);
    }

    fn do_sp_git(&mut self) {
        let v = self.sp_git_item.is_checked();
        self.update_config(|c| ensure_sp(c).configure_git = v);
    }

    fn do_sp_curl(&mut self) {
        let v = self.sp_curl_item.is_checked();
        self.update_config(|c| ensure_sp(c).configure_curl = v);
    }

    fn do_dns_enable(&mut self) {
        let on = self.dns_enable_item.is_checked();
        self.update_config(|c| {
            c.dns = if on { Some(c.dns.clone().unwrap_or_default()) } else { None };
        });
    }

    fn do_dns_listen(&mut self) {
        let cur = self.config.dns.as_ref().map(|d| d.listen_port.to_string()).unwrap_or_else(|| "5300".into());
        if let Some(v) = dialog_input("DNS Listen Port", "Enter local DNS listen port:", &cur) {
            if let Ok(p) = v.parse::<u16>() {
                self.update_config(|c| ensure_dns(c).listen_port = p);
            }
        }
    }

    fn do_dns_upstream(&mut self) {
        let cur = self.config.dns.as_ref().map(|d| d.upstream_dns.clone()).unwrap_or_else(|| "8.8.8.8:53".into());
        if let Some(v) = dialog_input("Upstream DNS", "DNS for matched domains (host:port):", &cur) {
            self.update_config(|c| ensure_dns(c).upstream_dns = v);
        }
    }

    fn do_dns_fallback(&mut self) {
        let cur = self.config.dns.as_ref().map(|d| d.fallback_dns.clone()).unwrap_or_else(|| "8.8.8.8:53".into());
        if let Some(v) = dialog_input("Fallback DNS", "DNS for everything else (host:port):", &cur) {
            self.update_config(|c| ensure_dns(c).fallback_dns = v);
        }
    }

    fn do_upstream_addr(&mut self) {
        let cur = self.config.upstream.addr.clone();
        if let Some(v) = dialog_input("Upstream Address", "Upstream SOCKS5 address:", &cur) {
            self.update_config(|c| c.upstream.addr = v);
        }
    }

    fn do_open_update(&self) {
        let repo = env!("CARGO_PKG_REPOSITORY");
        if !repo.is_empty() {
            let _ = std::process::Command::new("open")
                .arg(format!("{}/releases/latest", repo))
                .spawn();
        }
    }

    fn do_upstream_port(&mut self) {
        let cur = self.config.upstream.port.to_string();
        if let Some(v) = dialog_input("Upstream Port", "Upstream SOCKS5 port:", &cur) {
            if let Ok(p) = v.parse::<u16>() {
                self.update_config(|c| c.upstream.port = p);
            }
        }
    }
}

// ── winit ApplicationHandler ──────────────────────────────────────────────────

impl ApplicationHandler for TrayApp {
    fn resumed(&mut self, _: &ActiveEventLoop) {
        if self.tray.is_some() { return; }
        let menu = self.build_menu();
        self.tray = Some(
            TrayIconBuilder::new()
                .with_menu(Box::new(menu))
                .with_icon(circle_icon([142, 142, 147]))
                .with_tooltip("Talpa")
                .build()
                .unwrap(),
        );
    }

    fn window_event(&mut self, _: &ActiveEventLoop, _: WindowId, _: WindowEvent) {}

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        self.tick(event_loop);
        event_loop.set_control_flow(ControlFlow::WaitUntil(
            Instant::now() + Duration::from_millis(300),
        ));
    }
}
