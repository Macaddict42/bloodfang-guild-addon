//! Non-secret prefs on disk; token only in the OS credential store.
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

const SERVICE: &str = "guild-uploader";
const ACCOUNT: &str = "uploader-token";

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Prefs {
    pub site_url: String,
    pub game_folder: String,
    pub addon_name: String,
    pub start_at_login: bool,
    pub last_upload_at: Option<String>,
    pub last_status: Option<String>,
    pub last_addon_version: Option<String>,
    pub setup_done: bool,
}

pub fn prefs_path() -> PathBuf {
    dirs::config_dir().unwrap_or_else(|| PathBuf::from(".")).join("guild-uploader").join("prefs.json")
}

pub fn load_prefs() -> Prefs {
    let p = prefs_path();
    fs::read_to_string(&p).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_else(|| Prefs {
        site_url: option_env!("SITE_URL").unwrap_or("").to_string(),
        addon_name: option_env!("ADDON_NAME").unwrap_or("").to_string(), 
        ..Prefs::default()
    })
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

pub fn get_token() -> Result<Option<String>, String> {
    let e = keyring::Entry::new(SERVICE, ACCOUNT).map_err(|e| e.to_string())?;
    match e.get_password() {
        Ok(p) if !p.is_empty() => Ok(Some(p)),
        Ok(_) | Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(e.to_string()),
    }
}
