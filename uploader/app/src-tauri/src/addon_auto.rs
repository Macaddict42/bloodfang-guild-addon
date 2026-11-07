//! Whether an addon release may be installed without asking.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AddonAuto {
    /// Newer than what is installed, the game is not running, and this version was not set aside.
    Install,
    /// A newer archive exists, but the game process is running.
    WaitForGame,
    /// The user previously set this exact version aside.
    KeepSkipped,
    Current,
}

pub fn addon_auto_step(update_available: bool, game_running: bool, skipped_matches_latest: bool) -> AddonAuto {
    if !update_available {
        return AddonAuto::Current;
    }
    if skipped_matches_latest {
        return AddonAuto::KeepSkipped;
    }
    if game_running {
        return AddonAuto::WaitForGame;
    }
    AddonAuto::Install
}

/// Network checks may be repeated. A checksum or unpack failure must not.
pub fn addon_failure_retries(network: bool, attempt: u32) -> bool {
    network && attempt < 3
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_addon_installs_without_a_prompt() {
        assert_eq!(addon_auto_step(true, false, false), AddonAuto::Install);
    }

    #[test]
    fn a_running_game_defers_and_a_skipped_version_stays() {
        assert_eq!(addon_auto_step(true, true, false), AddonAuto::WaitForGame);
        assert_eq!(addon_auto_step(true, false, true), AddonAuto::KeepSkipped);
        assert_eq!(addon_auto_step(false, true, false), AddonAuto::Current);
    }

    #[test]
    fn only_a_network_problem_retries() {
        assert!(addon_failure_retries(true, 0));
        assert!(!addon_failure_retries(true, 3));
        assert!(!addon_failure_retries(false, 0));
    }
}
