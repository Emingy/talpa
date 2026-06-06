# Code Style

## Module structure

```
src/
  main.rs              — entry point only: init logging, load config, spawn thread, run UI
  config.rs            — Config struct and all sub-structs; Config::load / Config::save
  core/                — proxy engine, no UI dependencies
    matcher.rs         — domain pattern and IP/subnet matching
    socks5.rs          — SOCKS5 server
    dns.rs             — UDP DNS proxy
    tunnel.rs          — SSH tunnel management
    system_proxy.rs    — system proxy configuration (currently macOS: networksetup, launchctl, tool configs)
    proxy_service.rs   — lifecycle: starts/stops core services, handles Cmd channel
  ui/                  — UI layer, depends on core but core must not depend on ui
    macos/             — macOS UI (compiled only on target_os = "macos")
      menubar.rs       — TrayApp struct and ApplicationHandler impl
      dialogs.rs       — osascript dialog helpers
    windows/           — Windows UI (planned, not yet implemented)
  utils/               — shared helpers with no business logic
    icons.rs           — tray icon rendering
    config_helpers.rs  — ensure_tunnel / ensure_sp / ensure_dns
```

## Single Responsibility

- Each function does one thing. If a name requires "and", split it.
- `dispatch()` is a pure router — no business logic, only `if id == ... { self.do_*(); return; }`.
- Each menu action lives in its own `do_*` method on `TrayApp`.
- Helper functions go in `utils/`; they must not import from `core/` or `ui/`.

## Naming

- Action handlers on `TrayApp`: `do_<noun>_<verb>` or `do_<verb>_<noun>` — e.g. `do_add_domain`, `do_ssh_host`.
- Public entry points for subsystems: `run(config, ...)` or `run_thread(...)`.
- Config mutators: `ensure_<section>(c: &mut Config) -> &mut SectionConfig` — create the section if absent, return mutable ref.

## Error handling

- At system boundaries (file I/O, network): propagate with `?` or log + exit in `main`-path code.
- In UI handlers: log the error with `tracing::error!`, do not panic or `unwrap`.
- In core async tasks: log with `tracing::error!` inside `tokio::spawn`, never let tasks silently die.
- `unwrap()` is acceptable only when the invariant is guaranteed at construction time (e.g. `Icon::from_rgba` with a known-good buffer).

## Comments

- Write no comments by default.
- Add a comment only when the **why** is non-obvious: a hidden constraint, a subtle invariant, a workaround.
- Never describe what the code does — well-named identifiers already do that.

## Imports

- Within `core/`, refer to sibling modules as `crate::core::<module>`.
- `config` is crate-root: always `crate::config`.
- Never import `ui` from `core` — the dependency arrow is one-way: `ui → core`, `core → config`.
