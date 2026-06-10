# talpa

Cross-platform tool (macOS / Linux / Windows) that routes traffic for specific domains through a SOCKS5 proxy via a userspace TUN device. All OS-specific operations live behind the platform layer (see **Platform layer** below).

## What it does

1. **Proxy** — opens an SSH SOCKS5 tunnel to a remote host (default `user@remote-host`, local port `1080`)
2. **Tunnel** — creates a TUN device (default `10.0.0.2/24`), runs a userspace TCP/IP stack (ipstack), proxies accepted TCP connections through SOCKS5. Also installs always-on `static_routes` at startup.
3. **DNS Server** — listens on `127.0.0.1:53` (**both UDP and TCP**), intercepts queries matching the configured `domains`, resolves them via SOCKS5, adds per-IP host routes through the TUN for resolved IPs. TCP is required because resolvers (notably the Windows DNS client) fall back to DNS-over-TCP when a UDP answer is truncated (TC bit, responses > 512 bytes); a missing TCP listener makes that retry hit a closed port and fail the lookup with a connection reset. Both listeners share `Handler::resolve`.

All addresses, ports, the SSH target, TUN parameters, static routes, and domain masks are loaded from a YAML config (see **Configuration** below). The values above are the built-in defaults used when a field — or the whole file — is absent.

## Startup order

`main()` loads the config, then hands the **main thread** to the status-bar/tray UI (`ui::run`). The proxy pipeline is **not** started automatically — the user starts/stops it from the tray menu. Stages start **sequentially**, each only after the previous is actually ready; `start()` returns `Ok(())` once initialized and spawns background tasks:

```
Config::load() → ui::run() ─┐
                            ├─ [menu: Start] → Proxy::start() → Tunnel::start() → DnsServer::start()
                            └─ [menu: Stop]  → DnsServer::stop() → Tunnel::stop() → Proxy::stop()
```

`Proxy::start()` blocks until the SOCKS port accepts a TCP connection (`wait_until_ready`, 15s timeout) before returning, so TUN and DNS only come up against a live proxy. If any stage fails, `Controller::start()` cancels the token and tears the rest down in reverse.

Shutdown is via the menu's **Quit** item: `Controller::stop()` (reverse order) → `process::exit(0)`.

## Menu bar UI (`src/ui/`)

A status-bar/tray app built on `tray-icon` + `tao`. The tao event loop owns the main thread (macOS requires the `NSStatusItem` there; on Linux the tray needs GTK); a manually-built multi-thread tokio `Runtime` lives in `Controller` and the menu handlers drive it via `block_on`. muda menu items are `Rc`-backed (`!Send`), so the `Ui` and all item handles live entirely on the main thread inside the event-loop closure; menu clicks are forwarded from muda's handler into the loop as `UserEvent`s via an `EventLoopProxy`. Status indication: a rendered colored-dot icon (`status_icon`, green=running / grey=stopped) on every platform — kept non-template on macOS so the color shows.

- `ui/mod.rs` — `run()`: builds the event loop, forwards `MenuEvent`s, creates the tray on `StartCause::Init`.
- `ui/controller.rs` — `Controller`: owns the tokio runtime + running/stopped state; `start`/`stop`/`reload`/`reconcile_dns`.
- `ui/tray.rs` — `Ui`: builds the menu and dispatches clicks. Menu: Status (disabled label) · Start · Stop · **Domains ▸** (a checkable item per `domains` entry) · Open config… · Open logs… · Reload config · Quit.

**Domain toggles:** runtime-only overlay (`config::set_domain_enabled` / `domain_enabled`), not written back to YAML — `Reload config` re-reads the file and clears the overlay. Toggling while running calls `DnsServer::reconcile()` to apply/clear the matching split-DNS entry (via `platform::Sys`) and flush. `match_domain` skips disabled masks.

**Config reload:** `Config::reload` re-reads the file and swaps the live `Arc<Config>`. If the pipeline was running, `Controller::reload` restarts it so TUN/DNS-bind changes take effect; the menu is rebuilt afterward in case the domain list changed.

Requires elevation like the rest of the tool (TUN + routing/DNS); the tray runs in the same elevated process. On macOS the app runs as an accessory (`ActivationPolicy::Accessory`, set in `ui::run` before the loop starts) so it shows only in the status bar — no Dock icon or app-switcher entry. Windows/Linux create no window here, so there's no taskbar entry either.

## Architecture

```
DNS query (matched domain)
  └─ DnsServer (127.0.0.1:53)
       ├─ forward_via_socks → upstream DNS (<upstream-dns>:53 via SOCKS5)
       ├─ extract resolved IPs + TTL
       └─ Tunnel::add_route(ip, ttl) → platform::Sys::add_route (host route via the TUN)

TCP connection to routed IP
  └─ utunX (TUN device, 10.0.0.2)
       └─ ipstack::IpStack::accept() → IpStackTcpStream
            └─ Socks5Stream::connect(127.0.0.1:1080, original_dst)
                 └─ copy_bidirectional
```

## Platform layer (`src/platform/`)

Every OS-specific system operation is expressed as a trait in `platform/mod.rs` and implemented once per OS in `macos.rs` / `linux.rs` / `windows.rs`. The active backend is selected at compile time and re-exported as `platform::Sys`; the rest of the code calls `platform::Sys::*` and never shells out to an OS command directly. Traits use native `async fn` in traits (edition 2024) with static dispatch through `Sys` — no `async-trait`/`dyn`.

- `RouteManager` — `add_route`/`del_route`/`route_present` + `configure_tun` + `ipstack_packet_information`. macOS: BSD `route`. Linux: `ip route` (via `dev <iface>`). Windows: PowerShell `*-NetRoute`.
- `DnsConfigurator` — `apply`/`clear`/`flush_cache` (surgical, per-domain split-DNS). macOS: `/etc/resolver/<base>`. Linux: `systemd-resolved` (`resolvectl dns/domain`). Windows: NRPT (`*-DnsClientNrptRule`).
- `ProcessControl::terminate_pid` — `kill -TERM` (Unix) / `taskkill` (Windows).
- `ShellOpen::open_path` / `open_url` — open a file (`open -t` / `xdg-open` / `cmd /c start`) or a URL (`open` / `xdg-open` / `cmd /c start`) in the default handler.

`TunHandle { name, gateway, address }` carries the live interface so routing and (on Linux) split-DNS can target it. `Tunnel::start` reads the device name (`tun::AbstractDevice::tun_name`), stores it in the `ACTIVE_TUN` static (cleared on `Tunnel::stop`); `Tunnel::active_tun()` is the read accessor used by the routing helpers and `DnsServer`.

## Key design decisions

**PI header:** `tun 0.8` normalises the 4-byte AF/PI header per OS (macOS `packet_information=true` strips it; Linux defaults to `false` with `IFF_NO_PI`). `ipstack` must therefore use `packet_information=false` to avoid double-handling — driven by `platform::Sys::ipstack_packet_information()` (`false` on all current targets).

**DNS override resilience (surgical split-DNS):** Resolution for the configured `domains` is pointed at our local server per-domain, not globally, and is reversible. macOS: `/etc/resolver/<base>` files (checked before global DNS, so a VPN cannot override matched domains). Linux: `systemd-resolved` routing-only domains (`~base`) bound to the TUN link. Windows: NRPT rules. Disabled masks are cleared on `apply`.

**Route priority:** Host routes (`/32`) always win over a VPN's default route (`/0`) via longest-prefix-match. A background task (`revalidate_routes`, every 10s) re-adds any routes that a VPN may have flushed — both DNS-derived and static — through `platform::Sys::route_present`/`add_route`.

**Static vs DNS routes:** `tun.static_routes` (single IP or CIDR) are added once at startup and never expire — they are revalidated but not TTL-managed. DNS-derived host routes carry a TTL timer.

**TTL-based route expiry:** Each DNS-derived route has a TTL timer (`AbortHandle`). On re-resolution the timer is refreshed. Minimum TTL is `tun.min_ttl_secs` (default 60s). On expiry, `platform::Sys::del_route` is called automatically.

**SSH child monitoring:** `Proxy::start()` stores only the PID (not `Child`) in a static. `stop()` takes the PID and sends SIGTERM. If ssh dies on its own, the background watcher logs the error but does not crash the process. After spawning ssh, `start()` calls `wait_until_ready` — it polls the SOCKS addr with `TcpStream::connect` (200ms interval, 15s timeout) and bails early if the PID has already cleared (ssh died), so the next stages only run against a live proxy.

**Cancellation token:** A single `CancellationToken` (created per `start()`) is cloned to both `Tunnel::start` and `DnsServer::start`. The TUN accept loop / route-revalidation task and the DNS `recv_from` loop each `select!` on `shutdown.cancelled()` and break on cancel — this is what frees the `:53` UDP socket on Stop (otherwise the recv loop would keep the port bound and the next Start would hit `Address already in use`). `Controller::stop()` cancels the token first, then runs the per-stage `stop()`s in reverse.

## Domain matching (`domains` in config; matching logic in `src/core/dns/utils.rs`)

| Pattern | Matches |
|---|---|
| `**.example.com` | `example.com` and any depth: `a.b.c.example.com` |
| `*.example.com` | exactly one level: `api.example.com`, not `a.b.example.com` |
| `example.com` | exact match only |

## Configuration

Defined in `src/config.rs` (struct `Config`), loaded at startup into a swappable `OnceLock<RwLock<Arc<Config>>>`; access anywhere via `config::config()` (returns a cheap `Arc` snapshot). The UI can `Config::reload()` to swap in a fresh parse at runtime. Parsed from YAML (`serde` + `serde_yaml`).

- Path: first CLI arg, else the **standard per-OS config dir** (`src/paths.rs`): macOS `~/Library/Application Support/talpa/config.yml`, Linux `~/.config/talpa/config.yml`, Windows `%APPDATA%\talpa\config.yml`. On Unix `SUDO_USER` is honoured so files land in the human user's home, not root's.
- On first run, if the chosen config file is missing, `main()` writes a default template there (the embedded `config.example.yml`, via `paths::create_default_config`) and then loads it. Partial files merge over defaults (`#[serde(default)]`); unknown keys are rejected (`deny_unknown_fields`).
- `config.yml` is gitignored (holds real hosts/domains). `config.example.yml` is the checked-in, depersonalized template documenting every field; it is `include_str!`-embedded as the first-run default. The config test parses it.
- Defaults live in the `Default` impls in `src/config.rs` and are depersonalized placeholders.
- Sections: `ssh.target`, `socks.port` (SOCKS addr derived as `127.0.0.1:<port>` via `Socks::addr()`), `tun.{address,gateway,netmask,mtu,min_ttl_secs,static_routes}`, `dns.{listen_addr,upstream_addr,upstream_socks_addr}`, `domains`.

## Run

```bash
# macOS / Linux
sudo ./target/debug/talpa                  # uses ./config.yml (or defaults if absent)
sudo ./target/debug/talpa /path/to/cfg.yml # explicit config path

# Windows (elevated PowerShell / cmd)
.\target\debug\talpa.exe
```

Requires elevated privileges for TUN creation and route/DNS changes (root via `sudo` on macOS/Linux, Administrator on Windows). An OpenSSH client and a key for `ssh.target` in the agent are needed on all platforms. Launching opens a status-bar/tray menu (see **Menu bar UI**); use **Start** to bring the pipeline up.

**Logging:** `main()` initialises a file logger (`log` + `simplelog`) writing to `talpa.log` in the standard per-OS log dir (`src/paths.rs`): macOS `~/Library/Logs/talpa`, Linux `~/.local/state/talpa`, Windows `%LOCALAPPDATA%\talpa\logs`. On Unix it also logs to the terminal. The Windows binary is built with `#![cfg_attr(windows, windows_subsystem = "windows")]` — no console window — so the file is the only sink there. All code logs via `log::info!`/`log::error!` (the old `[TAG]` message prefixes are kept).

Per-OS runtime requirements:
- **Linux:** `systemd-resolved` (split-DNS) and GTK3 + `libayatana-appindicator` (tray). `iproute2` (`ip`) for routing.
- **Windows:** PowerShell (routing + NRPT split-DNS). `wintun.dll` is embedded in the binary and extracted to a temp file at startup (see `src/platform/windows.rs`); the x64 DLL must be placed at `assets/wintun.dll` before building for Windows (see `assets/README.md`).
- **macOS:** no extra packages.

## Dependencies

- `tun 0.8.10` — TUN device, cross-platform (macOS utun / Linux tun / Windows wintun), async
- `ipstack 0` — userspace TCP/IP stack
- `tokio-socks 0.5` — SOCKS5 client
- `hickory-{server,proto,resolver} 0.26` — DNS parsing
- `tokio-util 0.7` — `CancellationToken`
- `serde 1` + `serde_yaml 0.9` — YAML config
- `tray-icon 0.19` + `tao 0.30` — status-bar/tray menu and event loop
- `log 0.4` + `simplelog 0.12` — file + terminal logging
