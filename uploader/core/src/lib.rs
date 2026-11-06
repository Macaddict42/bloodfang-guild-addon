//! Uploader core (SPEC §19). Files only: reads `WTF/Account/*/SavedVariables/<addon>.lua` and posts the
//! signed export strings it finds to the site's `/api/uploader/ingest`. It never reads, writes or talks to
//! the game process, and it never edits SavedVariables.
pub mod browser_auth;
pub mod detect;
pub mod launch;
pub mod update;
use regex::Regex;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

pub use detect::{candidates, folder_acceptable, looks_like_game_dir, probe_explicit, probe_game_folder, FolderProbe, FolderSource};
pub use launch::quote_windows_run;
pub use update::{latest_addon, latest_app, install_addon, installed_version, addons_dir, game_seems_running, AddonRelease, UpdateError};

/// Types the site's uploader endpoint accepts.
pub const TYPES: &[&str] = &["gbank", "recipes"];
const BATCH: usize = 20;

/// Every `!BF:<type>:v1:` string for an accepted type, in file order, without repeats.
pub fn extract(text: &str) -> Vec<String> {
    let re = Regex::new(r"!BF:(gbank|recipes):v1:[A-Za-z0-9()]+").unwrap();
    let mut seen = BTreeSet::new();
    re.find_iter(text).map(|m| m.as_str().to_string()).filter(|s| seen.insert(s.clone())).collect()
}

/// SavedVariables files for `addon` under a game folder such as `.../World of Warcraft/_classic_era_`.
pub fn saved_variables(game_dir: &Path, addon: &str) -> Vec<PathBuf> {
    let mut out = vec![];
    let Ok(accounts) = fs::read_dir(game_dir.join("WTF").join("Account")) else { return out };
    for a in accounts.flatten() {
        let f = a.path().join("SavedVariables").join(format!("{addon}.lua"));
        if f.is_file() { out.push(f) }
    }
    out.sort();
    out
}

pub fn digest(s: &str) -> String { Sha256::digest(s.as_bytes()).iter().map(|b| format!("{b:02x}")).collect() }

/// Hashes of strings already handled (accepted or duplicate), so restarts don't resend them.
#[derive(Default, Serialize, Deserialize)]
pub struct State { pub sent: BTreeSet<String> }
impl State {
    pub fn load(p: &Path) -> State { fs::read_to_string(p).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default() }
    /// Atomic: write a temp file, then rename over the old one.
    pub fn save(&self, p: &Path) -> std::io::Result<()> {
        if let Some(d) = p.parent() { fs::create_dir_all(d)? }
        let tmp = p.with_extension("tmp");
        fs::write(&tmp, serde_json::to_vec(self).unwrap())?;
        fs::rename(tmp, p)
    }
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct Outcome { #[serde(rename = "type")] pub ty: Option<String>, pub ok: bool, pub error: Option<String> }

#[derive(Debug)]
pub enum UploadError { BadToken, RateLimited, Http(u16, String), Network(String) }
impl std::fmt::Display for UploadError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            UploadError::BadToken => write!(f, "the site rejected the uploader token; make a new one on the site's Uploader tokens page"),
            UploadError::RateLimited => write!(f, "the site asked us to slow down; try again in a minute"),
            UploadError::Http(c, b) => write!(f, "the site answered {c}: {b}"),
            UploadError::Network(e) => write!(f, "couldn't reach the site: {e}"),
        }
    }
}

/// A string is finished when the site accepted it or already had it.
pub fn done(o: &Outcome) -> bool { o.ok || o.error.as_deref() == Some("duplicate") }

pub fn post(site: &str, token: &str, strings: &[String]) -> Result<Vec<Outcome>, UploadError> {
    let url = format!("{}/api/uploader/ingest", site.trim_end_matches('/'));
    let res = ureq::post(&url)
        .set("Authorization", &format!("Bearer {token}"))
        .timeout(std::time::Duration::from_secs(30))
        .send_json(serde_json::json!({ "strings": strings }));
    match res {
        Ok(r) => {
            #[derive(Deserialize)] struct Body { results: Vec<Outcome> }
            r.into_json::<Body>().map(|b| b.results).map_err(|e| UploadError::Network(e.to_string()))
        }
        Err(ureq::Error::Status(401, _)) => Err(UploadError::BadToken),
        Err(ureq::Error::Status(403, r)) => {
            let body = r.into_string().unwrap_or_default();
            if body.contains("\"forbidden\"") {
                Err(UploadError::Http(403, "this account can't upload. Officer, GM, or Webmaster is required".into()))
            } else {
                Err(UploadError::Http(403, body.chars().take(200).collect()))
            }
        }
        Err(ureq::Error::Status(429, _)) => Err(UploadError::RateLimited),
        Err(ureq::Error::Status(c, r)) => Err(UploadError::Http(c, r.into_string().unwrap_or_default().chars().take(200).collect())),
        Err(e) => Err(UploadError::Network(e.to_string())),
    }
}

pub struct Report { pub found: usize, pub sent: usize, pub results: Vec<(String, Outcome)> }

/// One pass: read files, skip what's been handled, upload the rest in batches, remember what finished.
pub fn run(files: &[PathBuf], site: &str, token: &str, state_path: &Path) -> Result<Report, UploadError> {
    let mut state = State::load(state_path);
    let mut all = vec![];
    for f in files { if let Ok(t) = fs::read_to_string(f) { all.extend(extract(&t)) } }
    let found = all.len();
    let todo: Vec<String> = all.into_iter().filter(|s| !state.sent.contains(&digest(s))).collect();
    let mut results = vec![];
    for chunk in todo.chunks(BATCH) {
        let out = post(site, token, chunk)?;
        for (s, o) in chunk.iter().zip(out) {
            if done(&o) { state.sent.insert(digest(s)); }
            results.push((s.clone(), o));
        }
        let _ = state.save(state_path);
    }
    Ok(Report { found, sent: todo.len(), results })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn extracts_only_known_types_once() {
        let sv = "X = {\n [\"gbank\"] = { s = \"!BF:gbank:v1:ab(c)9\" },\n y = \"!BF:link:v1:zz\", z = \"!BF:gbank:v1:ab(c)9\", r = \"!BF:recipes:v1:Q\" }";
        assert_eq!(extract(sv), vec!["!BF:gbank:v1:ab(c)9", "!BF:recipes:v1:Q"]);
    }
    #[test]
    fn duplicate_counts_as_done() {
        assert!(done(&Outcome { ty: None, ok: false, error: Some("duplicate".into()) }));
        assert!(!done(&Outcome { ty: None, ok: false, error: Some("bad_sig".into()) }));
    }
    #[test]
    fn state_roundtrip() {
        let p = std::env::temp_dir().join(format!("gu-state-{}.json", std::process::id()));
        let mut s = State::default(); s.sent.insert("a".into()); s.save(&p).unwrap();
        assert!(State::load(&p).sent.contains("a")); let _ = fs::remove_file(p);
    }
}
