# Talpa — Claude context

System tray app that routes traffic for specific domains and IP subnets through an SSH tunnel.
Written in Rust. Currently supports **macOS only**; Windows support is planned.

See [CODESTYLE.md](CODESTYLE.md) for module layout, naming rules, and error-handling conventions.

## What the app does

1. Runs a local SOCKS5 proxy (`127.0.0.1:1080`).
2. Matches outgoing connections by domain pattern or IP/subnet.
3. Matched connections are forwarded through an upstream SOCKS5 (maintained via `ssh -D`).
4. Unmatched connections go direct.
5. Optionally runs a UDP DNS proxy that routes matched domains to an upstream DNS server.
6. Configures system proxy, env vars, and tool configs (`~/.npmrc`, `~/.gitconfig`, `~/.curlrc`) on start; rolls everything back on stop/quit.

## Build

```bash
cargo build --release          # binary at target/release/talpa
cargo test                     # unit tests (matcher logic)
```

### macOS app bundle

```bash
brew install librsvg            # required for icon conversion
./scripts/make-icns.sh          # assets/logo.svg → assets/AppIcon.icns
cargo install cargo-bundle
cargo bundle --release          # → target/release/bundle/osx/Talpa.app
```

## Key architecture decisions

- **tokio on background thread, winit on main thread** — macOS requires `NSApp` on the main thread; tokio runtime lives on a dedicated thread and communicates via `mpsc::sync_channel<Cmd>`.
- **Cmd::Reload(Arc<Config>)** — hot-reload without restart: stop current services, build new `Matcher` from updated config, restart.
- **SSH_ASKPASS** — password auth for SSH tunnel uses a temp script at `/tmp/talpa-askpass.sh` with `chmod 700`; `SSH_ASKPASS_REQUIRE=force` prevents tty prompts.
- **`core/` has no UI dependency** — `ui/` imports `core/`, never the other way around.
- **`ui/<platform>/`** — platform-specific UI code gated with `#[cfg(target_os = "...")]`. Currently only `ui/macos/` exists: `dialogs.rs` uses `osascript`, `menubar.rs` uses AppKit via `tray-icon` + `winit`. When adding Windows support, create `ui/windows/` with the same public interface.

## Config

Loaded from `config.toml` next to the binary (or first CLI arg). See `config.example.toml`.
All settings are also editable at runtime via the menu bar without restarting the app.

## CI / Release

- Push to `master` → runs tests → auto-creates a semver tag (conventional commits determine bump).
- Push of `v*` tag → builds `.app` bundle + `.dmg` → creates GitHub Release with both as assets.
- Workflow: [.github/workflows/release.yml](.github/workflows/release.yml)
