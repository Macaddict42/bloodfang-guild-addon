//! Close hides the existing window when the tray exists. Exit is the menu item
//! that actually ends the process. A failed tray must not leave the window hidden.

pub const MENU_SHOW: &str = "Open";
pub const MENU_EXIT: &str = "Exit";

/// Hide on the window close button only while a tray can bring the window back
/// and the process is not already leaving. OS shutdown and Exit set `exiting`.
pub fn close_should_hide(tray_ready: bool, exiting: bool) -> bool {
    tray_ready && !exiting
}

/// `--hidden` may hide the window only when the tray can show it again.
pub fn hidden_launch_keeps_window_visible(tray_ready: bool) -> bool {
    !tray_ready
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn close_hides_only_while_the_tray_can_restore_the_same_window() {
        assert!(close_should_hide(true, false));
        assert!(!close_should_hide(false, false));
        assert!(!close_should_hide(true, true));
        assert!(!close_should_hide(false, true));
    }

    #[test]
    fn a_failed_tray_does_not_start_hidden() {
        assert!(hidden_launch_keeps_window_visible(false));
        assert!(!hidden_launch_keeps_window_visible(true));
    }

    #[test]
    fn menu_labels_are_open_and_exit() {
        assert_eq!(MENU_SHOW, "Open");
        assert_eq!(MENU_EXIT, "Exit");
        assert_ne!(MENU_EXIT, "Quit");
    }
}
