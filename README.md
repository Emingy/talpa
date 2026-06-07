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
| Windows | — |
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
- Automatic update check on launch — notifies in the menu when a new version is available

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

| Mode | Config path |
|---|---|
| Installed app | `~/Library/Application Support/Talpa/config.toml` (created automatically on first launch) |
| Local development (`cargo run`) | `config.toml` in the project directory |

To use a custom path, pass it as the first argument: `./talpa /path/to/config.toml`

See [config.example.toml](config.example.toml) for all available options. All fields except `listen` and `upstream` are optional.

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

## Releasing

```bash
./scripts/release.sh patch   # or minor / major / 1.2.3
git push && git push origin v<version>
```

The script bumps the version in `Cargo.toml`, commits, and creates a `v*` tag.
Pushing the tag triggers CI: builds `.app` + `.dmg` and publishes a GitHub Release with auto-generated changelog.

---

## How it works

1. App starts with proxy **stopped**
2. Click **Start** in the menu bar to bring up the SSH tunnel and SOCKS5 listener
3. All TCP connections to matched domains/IPs are forwarded through the upstream proxy; everything else connects directly
4. DNS queries for matched domains are forwarded to `upstream_dns`; others go to `fallback_dns`
5. Click **Stop** or **Quit** — proxy stops, all system settings are rolled back automatically
