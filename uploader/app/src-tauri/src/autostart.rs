//! Start-at-login. Windows writes a quoted Run value itself because the plugin's
//! value is unquoted and splits on spaces. macOS uses the plugin's LaunchAgent,
//! whose arguments are already separate strings.

use tauri::AppHandle;

pub fn launched_hidden() -> bool {
    std::env::args().any(|arg| arg == "--hidden")
}

#[cfg(target_os = "windows")]
const RUN_VALUE: &str = "guild-uploader-app";

pub fn apply(app: &AppHandle, enabled: bool) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        let _ = app;
        windows_run(enabled)
    }
    #[cfg(not(target_os = "windows"))]
    {
        use tauri_plugin_autostart::ManagerExt;
        let auto = app.autolaunch();
        if enabled { auto.enable() } else { auto.disable() }.map_err(|e| e.to_string())
    }
}

#[cfg(target_os = "windows")]
fn windows_run(enabled: bool) -> Result<(), String> {
    use winreg::enums::HKEY_CURRENT_USER;
    use winreg::RegKey;
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let (key, _) = hkcu
        .create_subkey("Software\\Microsoft\\Windows\\CurrentVersion\\Run")
        .map_err(|e| e.to_string())?;
    if !enabled {
        let _ = key.delete_value(RUN_VALUE);
        return Ok(());
    }
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let data = guild_uploader_core::quote_windows_run(&exe.display().to_string(), &["--hidden"])?;
    key.set_value(RUN_VALUE, &data).map_err(|e| e.to_string())?;
    let read: String = key.get_value(RUN_VALUE).map_err(|e| e.to_string())?;
    if !read.starts_with('"') {
        let _ = key.delete_value(RUN_VALUE);
        return Err("Start at login was not saved safely.".into());
    }
    Ok(())
}
