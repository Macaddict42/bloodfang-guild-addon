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

/// How long the background thread waits between checks. The window being hidden is not an input.
#[derive(Clone, Copy, Debug)]
pub struct Cadence {
    pub game_wait_ms: u64,
    pub discover_ms: u64,
    pub network_base_ms: u64,
    pub network_cap_ms: u64,
}

impl Cadence {
    pub fn production() -> Self {
        Self {
            game_wait_ms: 60_000,
            discover_ms: 30 * 60 * 1000,
            network_base_ms: 30_000,
            network_cap_ms: 30 * 60 * 1000,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    WaitForGame,
    Installed,
    Current,
    KeepSkipped,
    Network,
    Integrity,
}

pub fn outcome_from_notice(message: &str, ok: bool, retry: bool) -> Outcome {
    if message.starts_with("Game is running") {
        Outcome::WaitForGame
    } else if message.starts_with("Installed ") {
        Outcome::Installed
    } else if message.starts_with("This addon version was set aside") {
        Outcome::KeepSkipped
    } else if !ok && retry {
        Outcome::Network
    } else if !ok {
        Outcome::Integrity
    } else {
        Outcome::Current
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Due {
    /// A check is already running. The caller must not start another.
    Busy,
    Wait { ms: u64 },
    Check,
}

pub struct Scheduler {
    cadence: Cadence,
    next_at: u64,
    in_flight: bool,
    network_failures: u32,
    installs: u32,
}

impl Scheduler {
    pub fn new(cadence: Cadence) -> Self {
        Self { cadence, next_at: 0, in_flight: false, network_failures: 0, installs: 0 }
    }

    pub fn installs(&self) -> u32 {
        self.installs
    }

    /// `now` is a fake or monotonic millisecond clock. Visibility is not consulted.
    pub fn poll(&mut self, now: u64) -> Due {
        if self.in_flight {
            return Due::Busy;
        }
        if now < self.next_at {
            return Due::Wait { ms: self.next_at - now };
        }
        self.in_flight = true;
        Due::Check
    }

    pub fn finish(&mut self, now: u64, outcome: Outcome) {
        self.in_flight = false;
        let wait = match outcome {
            Outcome::WaitForGame => {
                self.network_failures = 0;
                self.cadence.game_wait_ms
            }
            Outcome::Network => {
                self.network_failures = self.network_failures.saturating_add(1);
                let shift = self.network_failures.saturating_sub(1).min(8);
                let scaled = self.cadence.network_base_ms.saturating_mul(1u64 << shift);
                scaled.min(self.cadence.network_cap_ms)
            }
            Outcome::Installed => {
                self.installs = self.installs.saturating_add(1);
                self.network_failures = 0;
                self.cadence.discover_ms
            }
            Outcome::Current | Outcome::KeepSkipped | Outcome::Integrity => {
                self.network_failures = 0;
                self.cadence.discover_ms
            }
        };
        self.next_at = now.saturating_add(wait);
    }
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

    fn test_cadence() -> Cadence {
        Cadence { game_wait_ms: 60_000, discover_ms: 1_800_000, network_base_ms: 30_000, network_cap_ms: 1_800_000 }
    }

    #[test]
    fn hidden_game_exit_installs_once_without_a_ui_call() {
        let cadence = test_cadence();
        let mut sched = Scheduler::new(cadence);
        let mut now = 0u64;
        let mut game_running = true;
        let mut checks = 0u32;
        let mut installs = 0u32;
        let window_visible = false;
        while installs == 0 && checks < 8 {
            match sched.poll(now) {
                Due::Busy => panic!("a second check started while one was in flight"),
                Due::Wait { ms } => now += ms,
                Due::Check => {
                    checks += 1;
                    assert!(!window_visible, "the tray-hidden window is not asked");
                    let outcome = if game_running {
                        if checks >= 3 {
                            game_running = false;
                        }
                        Outcome::WaitForGame
                    } else {
                        installs += 1;
                        Outcome::Installed
                    };
                    sched.finish(now, outcome);
                }
            }
        }
        assert!(checks >= 3, "the game stayed busy through more than the first retry");
        assert_eq!(installs, 1);
        assert_eq!(sched.installs(), 1);
        match sched.poll(now) {
            Due::Wait { ms } => assert!(ms >= cadence.discover_ms),
            other => panic!("expected to wait after the one install, got {other:?}"),
        }
    }

    #[test]
    fn a_second_check_does_not_overlap_and_a_new_release_is_found_later() {
        let cadence = test_cadence();
        let mut sched = Scheduler::new(cadence);
        assert!(matches!(sched.poll(0), Due::Check));
        assert!(matches!(sched.poll(0), Due::Busy));
        sched.finish(0, Outcome::Current);
        assert!(matches!(sched.poll(cadence.discover_ms - 1), Due::Wait { .. }));
        assert!(matches!(sched.poll(cadence.discover_ms), Due::Check));
        sched.finish(cadence.discover_ms, Outcome::Installed);
        assert_eq!(sched.installs(), 1);
        assert!(matches!(sched.poll(cadence.discover_ms), Due::Wait { .. }));
    }

    #[test]
    fn network_waits_grow_and_a_bad_archive_does_not() {
        let cadence = test_cadence();
        let mut sched = Scheduler::new(cadence);
        assert!(matches!(sched.poll(0), Due::Check));
        sched.finish(0, Outcome::Network);
        match sched.poll(0) {
            Due::Wait { ms } => assert_eq!(ms, cadence.network_base_ms),
            other => panic!("{other:?}"),
        }
        assert!(matches!(sched.poll(cadence.network_base_ms), Due::Check));
        sched.finish(cadence.network_base_ms, Outcome::Network);
        match sched.poll(cadence.network_base_ms) {
            Due::Wait { ms } => assert_eq!(ms, cadence.network_base_ms * 2),
            other => panic!("{other:?}"),
        }
        let mut integrity = Scheduler::new(cadence);
        assert!(matches!(integrity.poll(0), Due::Check));
        integrity.finish(0, Outcome::Integrity);
        match integrity.poll(0) {
            Due::Wait { ms } => assert_eq!(ms, cadence.discover_ms),
            other => panic!("{other:?}"),
        }
        assert_eq!(outcome_from_notice("Game is running. The addon update will wait until it closes.", true, true), Outcome::WaitForGame);
        assert_eq!(outcome_from_notice("Installed 0.1.2.", true, false), Outcome::Installed);
        assert_eq!(outcome_from_notice("the download didn't match SHA256SUMS; left the old addon in place", false, false), Outcome::Integrity);
        assert_eq!(outcome_from_notice("couldn't reach GitHub: offline", false, true), Outcome::Network);
    }
}
