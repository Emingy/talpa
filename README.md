<p align="center">
    <img alt="Logo" src="assets/logo.svg" width=150/>
</p>
<h1 align="center" style="border-bottom: none;">Talpa</h1>
<h6 align="center">System tray app — routes traffic for specific domains and IPs through an SSH tunnel.</h6>
<p align="center">
  <a href="LICENSE">
    <img alt="License" src="https://img.shields.io/npm/l/@emingy/core">
  </a>
</p>

---

## Platform support

| Platform | Status |
|---|---|
| macOS | ✅ Supported |
| Windows | 🔜 Planned |
| Linux | — |
---

## Features

- Routes traffic for matched domains and IP subnets through an upstream SOCKS5 proxy
- Domain pattern matching: `*.example.com` (one level), `**.example.com` (any depth), exact
- IP/subnet rules: single address (`1.2.3.4`) or CIDR range (`10.0.0.0/8`)
- Auto-manages SSH tunnel (`ssh -D`) — restarts on failure, supports password auth
- Configures macOS system proxy via `networksetup`
- Sets `ALL_PROXY` / `HTTP_PROXY` / `HTTPS_PROXY` via `launchctl` (no terminal restart needed)
- Configures `~/.npmrc`, `~/.gitconfig`, `~/.curlrc`
- DNS proxy: matched domains resolved via upstream DNS, everything else via fallback
- All settings editable from the menu bar — no config file editing required

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

Requirements: Rust stable.

---

## Configuration

Config file is `config.toml` next to the binary (or pass path as first argument).

See [config.example.toml](config.example.toml) for a full annotated example.

All fields except `domains`, `listen`, and `upstream` are optional.

---

## Domain patterns

| Pattern | Matches |
|---|---|
| `**.example.com` | `example.com`, `a.example.com`, `a.b.example.com` |
| `*.example.com` | `a.example.com` only (one level) |
| `example.com` | exact match only |

---

## IP patterns

| Pattern | Matches |
|---|---|
| `1.2.3.4` | exact address |
| `10.0.0.0/8` | entire subnet (CIDR) |
| `::1` | exact IPv6 address |
| `fd00::/8` | IPv6 subnet |

---

## How it works

1. App starts with proxy **stopped**
2. Click **Start** in the menu bar to bring up the SSH tunnel and SOCKS5 listener
3. All TCP connections to matched domains/IPs are forwarded through the upstream proxy; everything else connects directly
4. DNS queries for matched domains are forwarded to `upstream_dns`; others go to `fallback_dns`
5. Click **Stop** or **Quit** — proxy stops, all system settings are rolled back automatically
