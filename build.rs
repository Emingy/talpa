fn main() {
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
