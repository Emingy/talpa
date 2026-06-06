pub fn dialog_input(title: &str, prompt: &str, default: &str) -> Option<String> {
    let script = format!(
        r#"display dialog "{p}" default answer "{d}" with title "{t}" buttons {{"Cancel","OK"}} default button "OK""#,
        p = prompt.replace('"', "\\\""),
        d = default.replace('"', "\\\""),
        t = title.replace('"', "\\\""),
    );
    let out = std::process::Command::new("osascript").args(["-e", &script]).output().ok()?;
    if !out.status.success() { return None; }
    String::from_utf8_lossy(&out.stdout)
        .split("text returned:").nth(1)
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

pub fn dialog_password(title: &str, prompt: &str) -> Option<String> {
    let script = format!(
        r#"display dialog "{p}" default answer "" with title "{t}" with hidden answer buttons {{"Cancel","OK"}} default button "OK""#,
        p = prompt.replace('"', "\\\""),
        t = title.replace('"', "\\\""),
    );
    let out = std::process::Command::new("osascript").args(["-e", &script]).output().ok()?;
    if !out.status.success() { return None; }
    String::from_utf8_lossy(&out.stdout)
        .split("text returned:").nth(1)
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

pub fn dialog_choose(items: &[String], title: &str, prompt: &str) -> Option<String> {
    let list = items.iter()
        .map(|d| format!(r#""{}""#, d.replace('"', "\\\"")))
        .collect::<Vec<_>>()
        .join(", ");
    let script = format!(
        r#"choose from list {{{l}}} with title "{t}" with prompt "{p}""#,
        l = list,
        t = title.replace('"', "\\\""),
        p = prompt.replace('"', "\\\""),
    );
    let out = std::process::Command::new("osascript").args(["-e", &script]).output().ok()?;
    if !out.status.success() { return None; }
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if s == "false" { None } else { Some(s) }
}
