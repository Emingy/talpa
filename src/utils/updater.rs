use std::sync::{Arc, Mutex};
use std::time::Duration;

pub fn spawn_update_check(repo_url: &'static str, latest: Arc<Mutex<Option<String>>>) {
    if repo_url.is_empty() {
        return;
    }
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(3));
        let api_url = github_api_url(repo_url);
        if let Ok(resp) = ureq::get(&api_url).set("User-Agent", "talpa").call() {
            if let Ok(json) = resp.into_json::<serde_json::Value>() {
                if let Some(tag) = json["tag_name"].as_str() {
                    *latest.lock().unwrap() = Some(tag.to_owned());
                }
            }
        }
    });
}

pub fn is_newer(current: &str, latest: &str) -> bool {
    match (parse_semver(current), parse_semver(latest)) {
        (Some(c), Some(l)) => l > c,
        _ => false,
    }
}

fn parse_semver(s: &str) -> Option<(u32, u32, u32)> {
    let s = s.trim_start_matches('v');
    let mut parts = s.splitn(3, '.');
    Some((
        parts.next()?.parse().ok()?,
        parts.next()?.parse().ok()?,
        parts.next()?.parse().ok()?,
    ))
}

fn github_api_url(repo_url: &str) -> String {
    let path = repo_url
        .trim_end_matches('/')
        .trim_start_matches("https://github.com/");
    format!("https://api.github.com/repos/{}/releases/latest", path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn newer_version_detected() {
        assert!(is_newer("1.0.0", "v1.0.1"));
        assert!(is_newer("1.0.0", "v1.1.0"));
        assert!(is_newer("1.0.0", "v2.0.0"));
    }

    #[test]
    fn same_or_older_not_newer() {
        assert!(!is_newer("1.0.1", "v1.0.1"));
        assert!(!is_newer("1.0.1", "v1.0.0"));
    }

    #[test]
    fn github_api_url_from_full_url() {
        assert_eq!(
            github_api_url("https://github.com/user/talpa"),
            "https://api.github.com/repos/user/talpa/releases/latest"
        );
    }
}
