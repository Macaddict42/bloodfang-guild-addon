//! The only approval dialog is for a newer uploader app.
//! There is no verified installer to run: GitHub releases are not published from
//! this unsigned pull-request build, and nothing here downloads or restarts.

pub const VERIFIED_INSTALL_MISSING: &str = "This build can't install an app update. No verified installer is published, so nothing was downloaded or restarted.";

pub fn should_offer(current: &str, latest: Option<&str>, deferred: Option<&str>) -> bool {
    match latest {
        Some(latest) if version_gt(latest, current) && deferred != Some(latest) => true,
        _ => false,
    }
}

/// Later remembers that version. Update now does not install or restart.
pub fn update_now_installs() -> bool {
    false
}

fn version_gt(latest: &str, current: &str) -> bool {
    match (semver::Version::parse(latest), semver::Version::parse(current)) {
        (Ok(latest), Ok(current)) => latest > current,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn later_hides_that_version_and_a_newer_one_asks_again() {
        assert!(should_offer("0.1.0", Some("0.2.0"), None));
        assert!(!should_offer("0.1.0", Some("0.2.0"), Some("0.2.0")));
        assert!(should_offer("0.1.0", Some("0.3.0"), Some("0.2.0")));
        assert!(!should_offer("0.1.0", Some("0.1.0"), None));
        assert!(!should_offer("0.1.0", None, None));
    }

    #[test]
    fn update_now_does_not_install_without_a_verified_installer() {
        assert!(!update_now_installs());
        assert!(VERIFIED_INSTALL_MISSING.contains("nothing was downloaded"));
        assert!(VERIFIED_INSTALL_MISSING.contains("restarted"));
    }
}
