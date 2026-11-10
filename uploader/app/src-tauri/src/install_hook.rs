//! The Windows installer hook is the previous-install handoff. These checks
//! lock the registry identities and the data it must not touch.

#[cfg(test)]
mod tests {
    const HOOK: &str = include_str!("../windows/hooks.nsh");

    #[test]
    fn previous_install_is_removed_without_deleting_saved_data() {
        assert!(HOOK.contains("Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\Guild Uploader"));
        assert!(HOOK.contains("Software\\raidgroup\\Guild Uploader"));
        assert!(HOOK.contains("Software\\Microsoft\\Windows\\CurrentVersion\\Run"));
        assert!(HOOK.contains("guild-uploader-app"));
        assert!(HOOK.contains("--hidden"));
        assert!(HOOK.contains("/S"));
        assert!(!HOOK.contains("/UPDATE"));
        assert!(!HOOK.contains("RMDir /r"));
        assert!(!HOOK.contains("DeleteAppData"));
        assert!(!HOOK.contains("guild-uploader\\"));
        assert!(HOOK.contains("Delete \"$INSTDIR\\guild-uploader-verify-update.exe\""));
        assert!(HOOK.contains("NSIS_HOOK_PREINSTALL"));
        assert!(HOOK.contains("NSIS_HOOK_POSTINSTALL"));
        assert!(HOOK.contains("NSIS_HOOK_PREUNINSTALL"));
        assert!(HOOK.contains("Software\\Bloodfang Guild\\Bloodfang App"));
        assert!(HOOK.contains("Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\Bloodfang App"));
        assert!(HOOK.contains("$INSTDIR\\Bloodfang Uploader.exe"));
        assert!(HOOK.contains("DeleteRegValue HKCU \"Software\\Microsoft\\Windows\\CurrentVersion\\Run\" \"Bloodfang App\""));
        assert!(HOOK.contains("DeleteRegValue HKCU \"Software\\Microsoft\\Windows\\CurrentVersion\\Run\" \"guild-uploader-app\""));
    }
}
