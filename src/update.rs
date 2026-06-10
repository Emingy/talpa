//! GitHub release update check.
//!
//! On startup the UI calls [`spawn_check`], which queries the GitHub "latest
//! release" API for the repository declared in `Cargo.toml` (`repository`),
//! compares its tag against the running version, and reports the outcome back
//! through a callback. The UI turns an [`UpdateStatus::Available`] into a
//! clickable menu item that opens the release page.

use std::time::Duration;

/// The running version (`CARGO_PKG_VERSION`), e.g. `1.2.0`.
pub const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");

/// The repository URL from `Cargo.toml` (`package.repository`), or empty if unset.
const REPOSITORY: &str = env!("CARGO_PKG_REPOSITORY");

/// Result of an update check.
#[derive(Clone, Debug)]
pub enum UpdateStatus {
    /// A newer release exists; `url` is its GitHub release page.
    Available { version: String, url: String },
    /// The running version is the latest.
    UpToDate,
    /// The check could not be completed (network/parse error, no repo set).
    Failed,
}

/// Runs the update check on a background thread and invokes `on_done` with the
/// result. Never blocks the caller.
pub fn spawn_check<F>(on_done: F)
where
    F: FnOnce(UpdateStatus) + Send + 'static,
{
    std::thread::spawn(move || on_done(check()));
}

/// Performs the blocking HTTP check against the GitHub API.
fn check() -> UpdateStatus {
    if REPOSITORY.is_empty() {
        return UpdateStatus::Failed;
    }

    let api_url = github_api_url(REPOSITORY);
    let resp = match ureq::get(&api_url)
        .set("User-Agent", "talpa")
        .set("Accept", "application/vnd.github+json")
        .timeout(Duration::from_secs(10))
        .call()
    {
        Ok(resp) => resp,
        Err(e) => {
            log::warn!("[UPDATE] check failed: {e}");
            return UpdateStatus::Failed;
        }
    };

    let json = match resp.into_json::<serde_json::Value>() {
        Ok(json) => json,
        Err(e) => {
            log::warn!("[UPDATE] could not parse release JSON: {e}");
            return UpdateStatus::Failed;
        }
    };

    let Some(tag) = json["tag_name"].as_str() else {
        log::warn!("[UPDATE] release JSON has no tag_name");
        return UpdateStatus::Failed;
    };

    if !is_newer(CURRENT_VERSION, tag) {
        return UpdateStatus::UpToDate;
    }

    // Prefer the release page from the API; fall back to the latest-release URL.
    let url = json["html_url"]
        .as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| format!("{}/releases/latest", REPOSITORY.trim_end_matches('/')));

    UpdateStatus::Available {
        version: tag.trim_start_matches('v').to_owned(),
        url,
    }
}

/// Whether `latest` is a strictly higher semver than `current` (both may carry a
/// leading `v`).
fn is_newer(current: &str, latest: &str) -> bool {
    match (parse_semver(current), parse_semver(latest)) {
        (Some(c), Some(l)) => l > c,
        _ => false,
    }
}

/// Parses `[v]MAJOR.MINOR.PATCH` into a comparable tuple.
fn parse_semver(s: &str) -> Option<(u32, u32, u32)> {
    let s = s.trim_start_matches('v');
    let mut parts = s.splitn(3, '.');
    Some((
        parts.next()?.parse().ok()?,
        parts.next()?.parse().ok()?,
        // Drop any pre-release/build suffix on the patch component.
        parts
            .next()?
            .split(['-', '+'])
            .next()?
            .parse()
            .ok()?,
    ))
}

/// Turns `https://github.com/owner/repo` into the latest-release API endpoint.
fn github_api_url(repo_url: &str) -> String {
    let path = repo_url
        .trim_end_matches('/')
        .trim_start_matches("https://github.com/")
        .trim_start_matches("http://github.com/");
    format!("https://api.github.com/repos/{path}/releases/latest")
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
        assert!(!is_newer("2.0.0", "v1.9.9"));
    }

    #[test]
    fn patch_suffix_is_ignored() {
        assert_eq!(parse_semver("1.2.3-rc1"), Some((1, 2, 3)));
        assert!(!is_newer("1.2.3", "v1.2.3-rc1"));
    }

    #[test]
    fn github_api_url_from_full_url() {
        assert_eq!(
            github_api_url("https://github.com/Emingy/talpa"),
            "https://api.github.com/repos/Emingy/talpa/releases/latest"
        );
    }
}
