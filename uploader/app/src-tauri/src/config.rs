//! Non-secret prefs on disk; token only in the OS credential store.
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

const SERVICE: &str = "guild-uploader";
const ACCOUNT: &str = "uploader-token";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrefsOrigin {
    FirstInstall,
    ExistingFile,
    Unreadable,
}

#[derive(Debug, Clone)]
pub struct LoadedPrefs {
    pub prefs: Prefs,
    pub origin: PrefsOrigin,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct Prefs {
    pub site_url: String,
    pub game_folder: String,
    pub addon_name: String,
    pub start_at_login: bool,
    pub last_upload_at: Option<String>,
    /// Last pass that looked for new data, whether or not it sent anything.
    #[serde(default)]
    pub last_checked_at: Option<String>,
    pub last_status: Option<String>,
    /// Plain outcome lines from the last upload (e.g. "gbank accepted"), for UI "what was sent".
    #[serde(default)]
    pub last_sent_lines: Option<Vec<String>>,
    pub last_addon_version: Option<String>,
    /// Latest addon version the user chose Skip this version for (SPEC §18b).
    #[serde(default)]
    pub skipped_addon_version: Option<String>,
    /// Uploader-app version the user chose Later for. Not an addon setting.
    #[serde(default)]
    pub deferred_app_version: Option<String>,
    pub setup_done: bool,
}

pub fn prefs_path() -> PathBuf {
    dirs::config_dir().unwrap_or_else(|| PathBuf::from(".")).join("guild-uploader").join("prefs.json")
}

pub fn load_prefs() -> LoadedPrefs {
    load_prefs_from(&prefs_path())
}

/// Missing file: first install, start at login on.
/// Present file: keep `startAtLogin`, including false. A missing key stays false.
/// Unreadable file: not a first install, and callers must not overwrite it or enable login start.
pub fn load_prefs_from(path: &std::path::Path) -> LoadedPrefs {
    if !path.exists() {
        return LoadedPrefs {
            origin: PrefsOrigin::FirstInstall,
            prefs: Prefs {
                start_at_login: true,
                addon_name: option_env!("ADDON_NAME").unwrap_or("").to_string(),
                ..Prefs::default()
            },
        };
    }
    match fs::read_to_string(path) {
        Ok(text) => match serde_json::from_str::<Prefs>(&text) {
            Ok(prefs) => LoadedPrefs { prefs, origin: PrefsOrigin::ExistingFile },
            Err(_) => unreadable(),
        },
        Err(_) => unreadable(),
    }
}

fn unreadable() -> LoadedPrefs {
    LoadedPrefs {
        origin: PrefsOrigin::Unreadable,
        prefs: Prefs { start_at_login: false, ..Prefs::default() },
    }
}

pub fn save_prefs(prefs: &Prefs) -> Result<(), String> {
    let p = prefs_path();
    if let Some(d) = p.parent() { fs::create_dir_all(d).map_err(|e| e.to_string())?; }
    let tmp = p.with_extension("tmp");
    fs::write(&tmp, serde_json::to_vec_pretty(prefs).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    fs::rename(tmp, p).map_err(|e| e.to_string())
}

pub fn state_path(prefs: &Prefs) -> PathBuf {
    dirs::data_local_dir().unwrap_or_else(|| PathBuf::from("."))
        .join("guild-uploader")
        .join(format!("{}-upload-state.json", prefs.addon_name))
}

pub fn set_token(token: &str) -> Result<(), String> {
    let e = keyring::Entry::new(SERVICE, ACCOUNT).map_err(|e| e.to_string())?;
    if token.is_empty() { let _ = e.delete_credential(); return Ok(()); }
    e.set_password(token).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_file_is_first_install_with_login_on() {
        let path = std::env::temp_dir().join(format!("gu-prefs-missing-{}", std::process::id()));
        let _ = fs::remove_file(&path);
        let loaded = load_prefs_from(&path);
        assert_eq!(loaded.origin, PrefsOrigin::FirstInstall);
        assert!(loaded.prefs.start_at_login);
        assert!(!loaded.prefs.setup_done);
    }

    #[test]
    fn existing_false_stays_false_and_corrupt_is_not_first_install() {
        let path = std::env::temp_dir().join(format!("gu-prefs-keep-{}", std::process::id()));
        fs::write(&path, r#"{"siteUrl":"https://old.example","gameFolder":"C:\\Program Files\\World of Warcraft\\_forever_","addonName":"Addon","startAtLogin":false,"lastUploadAt":null,"lastStatus":null,"lastAddonVersion":null,"setupDone":true}"#).unwrap();
        let loaded = load_prefs_from(&path);
        assert_eq!(loaded.origin, PrefsOrigin::ExistingFile);
        assert!(!loaded.prefs.start_at_login);
        assert!(loaded.prefs.setup_done);
        assert!(loaded.prefs.game_folder.contains("_forever_"));
        fs::write(&path, "{").unwrap();
        let bad = load_prefs_from(&path);
        assert_eq!(bad.origin, PrefsOrigin::Unreadable);
        assert!(!bad.prefs.start_at_login);
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn partial_file_keeps_login_off() {
        let path = std::env::temp_dir().join(format!("gu-prefs-partial-{}", std::process::id()));
        fs::write(&path, r#"{"gameFolder":"D:\\Games\\Wow","setupDone":true}"#).unwrap();
        let loaded = load_prefs_from(&path);
        assert_eq!(loaded.origin, PrefsOrigin::ExistingFile);
        assert!(!loaded.prefs.start_at_login);
        assert_eq!(loaded.prefs.game_folder, r"D:\Games\Wow");
        let _ = fs::remove_file(&path);
    }
}

pub fn get_token() -> Result<Option<String>, String> {
    let e = keyring::Entry::new(SERVICE, ACCOUNT).map_err(|e| e.to_string())?;
    match e.get_password() {
        Ok(p) if !p.is_empty() => Ok(Some(p)),
        Ok(_) | Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(e.to_string()),
    }
}
