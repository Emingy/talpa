<p align="center">
    <img alt="Logo" src="assets/logo.svg" width=150/>
</p>
<h1 align="center" style="border-bottom: none;">Talpa</h1>
<h6 align="center">System tray app — routes traffic for specific domains and IPs through an SSH SOCKS5 tunnel via a userspace TUN device.</h6>
<p align="center">
  <a href="LICENSE">
    <img alt="License" src="https://img.shields.io/npm/l/@emingy/core">
  </a>
</p>

---

## Platform support

| Platform | Status |
|---|---|
| macOS | ✅ Supported — prebuilt `.app` / `.dmg` |
| Linux | ✅ Supported — prebuilt binary tarball |
| Windows | ✅ Supported — prebuilt binary zip |

Every OS-specific operation (routing, split-DNS, TUN setup) lives behind a platform layer (`src/platform/`) implemented per OS. Each release publishes artifacts for all three.

---

## Features

- Routes traffic for matched domains and IP subnets through a SOCKS5 proxy over an SSH tunnel
- Userspace **TUN device** + TCP/IP stack (`ipstack`) — no system-wide proxy settings are touched
- **Surgical split-DNS**: matched domains are resolved by a local DNS server and routed per-domain, fully reversible (macOS `/etc/resolver`, Linux `systemd-resolved`, Windows NRPT) — a VPN's default DNS can't override matched domains
- Local DNS proxy on `127.0.0.1:53` (UDP **and** TCP) — matched domains resolved over SOCKS, everything else via the upstream resolver
- Per-IP **host routes** (`/32`) for resolved addresses always win over a VPN's default route; a background task re-adds any routes a VPN flushes
- TTL-based route expiry for DNS-derived routes; always-on `static_routes` (single IP or CIDR)
- Auto-manages the SSH tunnel (`ssh -D`) — waits until the SOCKS port is live before bringing up TUN/DNS, monitors the child, tears down cleanly
- Domain pattern matching: `*.example.com` (one level), `**.example.com` (any depth), exact
- Controlled entirely from the tray menu — Start/Stop, per-domain toggles, open config/logs, reload config

---

## Build

```bash
cargo build --release
```

Binary: `target/release/talpa`

### macOS app bundle

```bash
# 1. Generate AppIcon.icns from assets/logo.svg (requires librsvg)
brew install librsvg
./scripts/make-icns.sh

# 2. Build the .app bundle
cargo install cargo-bundle
cargo bundle --release
```

App: `target/release/bundle/osx/Talpa.app`

Requirements: Rust stable. On Windows, place the x64 `wintun.dll` at `assets/wintun.dll` before building (see [assets/README.md](assets/README.md)) — it is embedded into the binary.

---

## Run

The tool creates a TUN device and changes routing/DNS, so it needs elevated privileges. An OpenSSH client and a key for `ssh.target` loaded in the SSH agent are required on all platforms.

```bash
# macOS / Linux
sudo ./target/release/talpa                  # uses the per-OS config path (or defaults)
sudo ./target/release/talpa /path/to/cfg.yml # explicit config path

# Windows (elevated PowerShell / cmd)
.\target\release\talpa.exe
```

Launching opens the tray menu with the proxy **stopped** — use **Start** to bring the pipeline up.

Per-OS runtime requirements:
- **Linux:** `systemd-resolved` (split-DNS), GTK3 + `libayatana-appindicator` (tray), `iproute2` (routing)
- **Windows:** PowerShell (routing + NRPT split-DNS); `wintun.dll` is embedded and extracted at startup
- **macOS:** no extra packages

---

## Configuration

Config is YAML. On first run, if the file is missing, a default template (the contents of [config.example.yml](config.example.yml)) is written to the standard per-OS location and loaded.

| Platform | Config path |
|---|---|
| macOS | `~/Library/Application Support/talpa/config.yml` |
| Linux | `~/.config/talpa/config.yml` |
| Windows | `%APPDATA%\talpa\config.yml` |

To use a custom path, pass it as the first argument: `sudo ./talpa /path/to/config.yml`

Sections: `ssh.target`, `socks.port`, `tun.{address,gateway,netmask,mtu,min_ttl_secs,static_routes}`, `dns.{listen_addr,upstream_addr,upstream_socks_addr}`, `domains`. Any omitted field merges over the built-in defaults; see [config.example.yml](config.example.yml) for every option. Edit, then **Reload config** from the tray menu.

---

## Domain patterns

| Pattern | Matches |
|---|---|
| `**.example.com` | `example.com`, `a.example.com`, `a.b.example.com` |
| `*.example.com` | `a.example.com` only (one level) |
| `example.com` | exact match only |

---

## Static routes

`tun.static_routes` are added once at startup and never expire (revalidated, but not TTL-managed):

| Pattern | Matches |
|---|---|
| `1.2.3.4` | exact address |
| `10.0.0.0/8` | entire subnet (CIDR) |

---

## Releasing

```bash
./scripts/release.sh patch   # or minor / major / 1.2.3
git push && git push origin v<version>
```

The script bumps the version in `Cargo.toml`, commits, and creates a `v*` tag.
Pushing the tag triggers CI: builds the macOS `.app`/`.dmg`, the Linux tarball, and the Windows zip, then publishes a GitHub Release with auto-generated changelog.

---

## How it works

1. App starts with the proxy **stopped**
2. Click **Start** — stages come up sequentially: SSH SOCKS5 tunnel (waits until the SOCKS port accepts a connection) → TUN device + ipstack → local DNS server
3. DNS queries for matched domains are resolved over SOCKS and each resolved IP gets a host route through the TUN; non-matched queries go to the upstream resolver
4. TCP connections to routed IPs enter the TUN, are picked up by the userspace stack, and proxied through SOCKS5 to their original destination; everything else connects directly
5. Click **Stop** or **Quit** — stages tear down in reverse and all routing/DNS changes are rolled back automatically

## Status indicators

| Icon | Label | Meaning |
|---|---|---|
| ○ | Proxy: Stopped | Not running |
| ● (blue) | Proxy: Connecting… | Starting up, waiting for SSH tunnel |
| ● (orange) | Proxy: Tunnel down | Running, but SSH tunnel failed to connect |
| ● (green) | Proxy: Running | All systems up |

Clicking **Stop** during *Connecting* cancels startup immediately.
