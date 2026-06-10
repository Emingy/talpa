//! Build script:
//!
//! 1. Reads the bundle name from `Cargo.toml`
//!    (`package.metadata.bundle.name`) and exposes it as the `BUNDLE_NAME`
//!    compile-time env var. Runs on every target.
//! 2. On Windows targets, embeds an application manifest that requests
//!    elevation (`requireAdministrator`), so launching the exe always triggers
//!    a UAC prompt — the tool needs admin for TUN creation and route/DNS
//!    changes. No-op on other targets.
//!
//! Cross-compiling from macOS/Linux requires the mingw `windres` for the target
//! (e.g. `x86_64-w64-mingw32-windres` from the `mingw-w64` package); winresource
//! locates it by the target prefix.

const ADMIN_MANIFEST: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<assembly xmlns="urn:schemas-microsoft-com:asm.v1" manifestVersion="1.0">
  <trustInfo xmlns="urn:schemas-microsoft-com:asm.v3">
    <security>
      <requestedPrivileges>
        <requestedExecutionLevel level="requireAdministrator" uiAccess="false"/>
      </requestedPrivileges>
    </security>
  </trustInfo>
</assembly>"#;

fn main() {
    emit_bundle_name();
    embed_windows_manifest();
}

/// Read `package.metadata.bundle.name` from `Cargo.toml` and expose it as the
/// `BUNDLE_NAME` compile-time env var.
fn emit_bundle_name() {
    let manifest: toml::Value = toml::from_str(
        &std::fs::read_to_string("Cargo.toml").expect("Cargo.toml not found"),
    )
    .expect("invalid Cargo.toml");

    let name = manifest
        .get("package")
        .and_then(|p| p.get("metadata"))
        .and_then(|m| m.get("bundle"))
        .and_then(|b| b.get("name"))
        .and_then(|n| n.as_str())
        .unwrap_or("Talpa");

    println!("cargo:rustc-env=BUNDLE_NAME={}", name);
}

/// On Windows targets, embed the admin-elevation manifest. No-op elsewhere.
fn embed_windows_manifest() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_manifest(ADMIN_MANIFEST);
        if let Err(e) = res.compile() {
            panic!("failed to embed Windows admin manifest: {e}");
        }
    }
}
