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
- **Update check** — `utils/updater.rs` spawns a background thread 3 seconds after launch, fetches the latest GitHub release via the API, and compares with `CARGO_PKG_VERSION`. If a newer version is found, a clickable menu item appears that opens the releases page. The GitHub repo URL comes from `[package].repository` in `Cargo.toml`.

## Config

- **Debug builds** (`cargo run`): loads `config.toml` from the current directory.
- **Release builds** (`.app`): loads `~/Library/Application Support/<bundle-name>/config.toml`; created with defaults on first launch.
- Pass a custom path as the first CLI arg to override: `./talpa /path/to/config.toml`.

The bundle name in the path comes from `[package.metadata.bundle].name` in `Cargo.toml`, read at compile time by `build.rs`.
See `config.example.toml` for all available options. All settings are also editable at runtime via the menu bar.

## CI / Release

- Push to `master` → runs tests + clippy.
- Push a `v*` tag manually → builds `.app` bundle (ad-hoc signed) + `.dmg` → creates GitHub Release with changelog from conventional commits.
- Workflow: [.github/workflows/release.yml](.github/workflows/release.yml)
