//! Uploader core (SPEC §19). Files only: reads `WTF/Account/*/SavedVariables/<addon>.lua` and posts the
//! signed export strings it finds to the site's `/api/uploader/ingest`. It never reads, writes or talks to
//! the game process, and it never edits SavedVariables.
pub mod browser_auth;
pub mod detect;
pub mod launch;
pub mod process;
pub mod ranks;
pub mod update;
use regex::Regex;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

pub use detect::{candidates, folder_acceptable, looks_like_game_dir, probe_explicit, probe_game_folder, FolderProbe, FolderSource};
pub use launch::quote_windows_run;
pub use process::{background_command, game_seems_running, is_game_process, CREATE_NO_WINDOW};
pub use ranks::{newest_guild_ranks, parse_guild_block, post_ranks, GuildRanks, RanksMark, RanksOutcome};
pub use update::{latest_addon, latest_app, install_addon, installed_version, addons_dir, download_bytes, download_text, sha256_hex, sums_entry, AddonRelease, UpdateError};

/// Types the site's uploader endpoint accepts.
pub const TYPES: &[&str] = &["gbank", "recipes", "ranks"];
const BATCH: usize = 20;

/// Every `!BF:<type>:v1:` string for an accepted type, in file order, without repeats.
pub fn extract(text: &str) -> Vec<String> {
    let re = Regex::new(r"!BF:(gbank|recipes|ranks):v1:[A-Za-z0-9()]+").unwrap();
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
#[serde(default)]
pub struct State {
    pub sent: BTreeSet<String>,
    /// Last guild rank-name block the site gave a final answer for (`ranks.rs`).
    pub ranks_last: Option<RanksMark>,
}
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
/// A per-string refusal (including guild-bank permission) stays unfinished and
/// does not discard the saved token.
pub fn done(o: &Outcome) -> bool { o.ok || o.error.as_deref() == Some("duplicate") }

/// Only a no-longer-member `401 {"error":"bad_token"}` drops the saved token.
pub fn drops_saved_token(err: &UploadError) -> bool {
    matches!(err, UploadError::BadToken)
}

/// Map an ingest HTTP status and body. `401` with `bad_token` is the dead-member
/// case. A guild-bank permission failure arrives as HTTP 200 `results`, so this
/// does not rewrite HTTP 403 into a batch-wide role sentence.
pub fn interpret_ingest(status: u16, body: &str) -> Result<Vec<Outcome>, UploadError> {
    match status {
        200 => {
            #[derive(Deserialize)] struct Body { results: Vec<Outcome> }
            let parsed = serde_json::from_str::<Body>(body).map_err(|e| UploadError::Network(e.to_string()))?;
            Ok(parsed.results.into_iter().map(redact_outcome).collect())
        }
        401 => {
            #[derive(Deserialize)] struct ErrBody { error: String }
            match serde_json::from_str::<ErrBody>(body) {
                Ok(b) if b.error == "bad_token" => Err(UploadError::BadToken),
                _ => Err(UploadError::Http(401, redact_uploader_token(body))),
            }
        }
        429 => Err(UploadError::RateLimited),
        code => Err(UploadError::Http(code, redact_uploader_token(body))),
    }
}

fn redact_outcome(mut o: Outcome) -> Outcome {
    if let Some(error) = o.error.take() {
        o.error = Some(redact_uploader_token(&error));
    }
    o
}

/// Drop an `upl_` secret if a site error body echoes one. The UI shows this string.
fn redact_uploader_token(body: &str) -> String {
    let mut out = String::new();
    let mut rest = body;
    while let Some(i) = rest.find("upl_") {
        out.push_str(&rest[..i]);
        out.push_str("upl_…");
        rest = &rest[i + 4..];
        let skip = rest.find(|c: char| c.is_whitespace() || c == '"' || c == '\'' || c == ',' || c == '}').unwrap_or(rest.len());
        rest = &rest[skip..];
    }
    out.push_str(rest);
    out.chars().take(200).collect()
}

pub fn post(site: &str, token: &str, strings: &[String]) -> Result<Vec<Outcome>, UploadError> {
    let url = format!("{}/api/uploader/ingest", site.trim_end_matches('/'));
    let res = ureq::post(&url)
        .set("Authorization", &format!("Bearer {token}"))
        .timeout(std::time::Duration::from_secs(30))
        .send_json(serde_json::json!({ "strings": strings }));
    match res {
        Ok(r) => interpret_ingest(200, &r.into_string().unwrap_or_default()),
        Err(ureq::Error::Status(code, r)) => interpret_ingest(code, &r.into_string().unwrap_or_default()),
        Err(e) => Err(UploadError::Network(e.to_string())),
    }
}

#[derive(Debug)]
pub struct Report {
    pub found: usize,
    pub sent: usize,
    pub results: Vec<(String, Outcome)>,
    /// Rank names, when a block was sent on this pass.
    pub ranks: Option<RanksOutcome>,
}

impl Report {
    /// Whether this pass sent anything to the site.
    pub fn requested(&self) -> bool { self.sent > 0 || self.ranks.is_some() }
}

/// Where to look for the guild rank-name block: the addon folder name and the configured guild
/// (empty accepts any guild name; the site checks it).
pub struct RanksSource<'a> { pub addon: &'a str, pub guild: &'a str }

/// One pass: read files, skip what's been handled, upload the rest in batches, remember what finished.
pub fn run(files: &[PathBuf], site: &str, token: &str, state_path: &Path) -> Result<Report, UploadError> {
    run_all(files, site, token, state_path, None)
}

/// `run`, then the newest guild rank-name block when `ranks` is given and the site hasn't answered
/// for that exact block yet. A dead token stops the pass like an ingest `401 bad_token`.
pub fn run_all(files: &[PathBuf], site: &str, token: &str, state_path: &Path, ranks: Option<RanksSource>) -> Result<Report, UploadError> {
    let mut report = run_strings(files, site, token, state_path)?;
    let Some(src) = ranks else { return Ok(report) };
    let Some(block) = newest_guild_ranks(files, src.addon, src.guild) else { return Ok(report) };
    let mark = RanksMark { t: block.t, hash: block.hash() };
    let mut state = State::load(state_path);
    if state.ranks_last.as_ref() == Some(&mark) { return Ok(report) }
    let outcome = post_ranks(site, token, &block);
    if outcome == RanksOutcome::BadToken { return Err(UploadError::BadToken) }
    if outcome.is_final() {
        state.ranks_last = Some(mark);
        let _ = state.save(state_path);
    }
    report.ranks = Some(outcome);
    Ok(report)
}

fn run_strings(files: &[PathBuf], site: &str, token: &str, state_path: &Path) -> Result<Report, UploadError> {
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
    Ok(Report { found, sent: todo.len(), results, ranks: None })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn extracts_only_known_types_once() {
        let sv = "X = {\n [\"gbank\"] = { s = \"!BF:gbank:v1:ab(c)9\" },\n y = \"!BF:link:v1:zz\", z = \"!BF:gbank:v1:ab(c)9\", r = \"!BF:recipes:v1:Q\" }";
        assert_eq!(extract(sv), vec!["!BF:gbank:v1:ab(c)9", "!BF:recipes:v1:Q"]);
        let with_ranks = "o = { [\"ranks\"] = { s = \"!BF:ranks:v1:Zz9\" } }";
        assert_eq!(extract(with_ranks), vec!["!BF:ranks:v1:Zz9"]);
        assert!(TYPES.contains(&"ranks"));
    }
    #[test]
    fn duplicate_counts_as_done() {
        assert!(done(&Outcome { ty: None, ok: false, error: Some("duplicate".into()) }));
        assert!(!done(&Outcome { ty: None, ok: false, error: Some("bad_sig".into()) }));
    }
    #[test]
    fn bad_token_401_drops_the_saved_token() {
        let err = interpret_ingest(401, r#"{"error":"bad_token"}"#).unwrap_err();
        assert!(drops_saved_token(&err));
        assert!(!err.to_string().contains("upl_"));
    }

    #[test]
    fn other_401_does_not_pretend_the_token_is_dead() {
        let err = interpret_ingest(401, r#"{"error":"nope","token":"upl_secret"}"#).unwrap_err();
        assert!(!drops_saved_token(&err));
        assert!(!err.to_string().contains("upl_secret"));
        assert!(err.to_string().contains("upl_…"));
    }

    #[test]
    fn http_200_keeps_a_bank_refusal_on_that_string() {
        let body = r#"{"results":[{"type":"gbank","ok":false,"error":"site_refused"},{"type":"recipes","ok":true}]}"#;
        let out = interpret_ingest(200, body).unwrap();
        assert_eq!(out.len(), 2);
        assert!(!done(&out[0]));
        assert_eq!(out[0].error.as_deref(), Some("site_refused"));
        assert!(done(&out[1]));
        assert_eq!(out[1].ty.as_deref(), Some("recipes"));
    }

    #[test]
    fn http_403_is_not_rewritten_as_an_officer_batch() {
        let err = interpret_ingest(403, r#"{"error":"forbidden","token":"upl_secret"}"#).unwrap_err();
        assert!(!drops_saved_token(&err));
        let text = err.to_string();
        assert!(!text.contains("Officer"));
        assert!(!text.contains("Webmaster"));
        assert!(!text.contains("upl_secret"));
        assert!(text.contains("403"));
    }
    #[test]
    fn http_errors_do_not_keep_an_uploader_token() {
        let shown = redact_uploader_token(r#"{"error":"bad","token":"upl_supersecretvalue"}"#);
        assert!(!shown.contains("supersecret"));
        assert!(shown.contains("upl_…"));
    }
    /// Answers each request with the next (status, body) and records request lines and bodies.
    fn serve(answers: Vec<(u16, &'static str)>) -> (String, std::sync::Arc<std::sync::Mutex<Vec<String>>>) {
        use std::io::{Read, Write};
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let site = format!("http://{}", l.local_addr().unwrap());
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let log = seen.clone();
        std::thread::spawn(move || {
            for (code, body) in answers {
                let Ok((mut c, _)) = l.accept() else { return };
                let mut buf = Vec::new();
                let mut chunk = [0u8; 4096];
                loop {
                    let n = c.read(&mut chunk).unwrap_or(0);
                    if n == 0 { break }
                    buf.extend_from_slice(&chunk[..n]);
                    let text = String::from_utf8_lossy(&buf).to_string();
                    if let Some(h) = text.find("\r\n\r\n") {
                        let len = text[..h].lines().find_map(|l| l.to_ascii_lowercase().strip_prefix("content-length:").map(|v| v.trim().parse::<usize>().unwrap_or(0))).unwrap_or(0);
                        if buf.len() >= h + 4 + len { break }
                    }
                }
                log.lock().unwrap().push(String::from_utf8_lossy(&buf).to_string());
                let _ = write!(c, "HTTP/1.1 {code} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
            }
        });
        (site, seen)
    }

    fn two_account_shape(dir: &Path, t1: i64, t2: i64) -> Vec<PathBuf> {
        for (acct, t) in [("300769113#1", t1), ("300769113#2", t2)] {
            let sv = dir.join("WTF").join("Account").join(acct).join("SavedVariables");
            fs::create_dir_all(&sv).unwrap();
            fs::write(sv.join("Example.lua"), crate::ranks::tests::fixture(t)).unwrap();
        }
        saved_variables(dir, "Example")
    }

    #[test]
    fn empty_chars_and_outbox_still_send_rank_names_once() {
        let dir = tempfile::tempdir().unwrap();
        let files = two_account_shape(dir.path(), 1794100000, 1794100500);
        let state = dir.path().join("state.json");
        let (site, seen) = serve(vec![(200, r#"{"stored":"stored"}"#)]);
        let src = || Some(RanksSource { addon: "Example", guild: "Example" });
        let r = run_all(&files, &site, "upl_test", &state, src()).unwrap();
        assert_eq!((r.found, r.sent), (0, 0));
        assert_eq!(r.ranks, Some(RanksOutcome::Stored("stored".into())));
        assert!(r.requested());
        {
            let req = seen.lock().unwrap()[0].clone();
            assert!(req.starts_with("POST /api/uploader/ranks "));
            assert!(req.contains("Authorization: Bearer upl_test") || req.contains("authorization: Bearer upl_test"));
            let body: serde_json::Value = serde_json::from_str(&req[req.find("\r\n\r\n").unwrap() + 4..]).unwrap();
            assert_eq!(body, serde_json::json!({ "guild": "Example", "t": 1794100500, "ranks": ["Guild Master", "Officer", "Veteran", "Member", "Initiate"] }));
        }
        // Unchanged block: no request at all, so nothing to report as sent.
        let again = run_all(&files, &site, "upl_test", &state, src()).unwrap();
        assert!(again.ranks.is_none());
        assert!(!again.requested());
        assert_eq!(seen.lock().unwrap().len(), 1);
    }

    #[test]
    fn officer_refusal_is_final_for_that_block_and_a_new_capture_retries() {
        let dir = tempfile::tempdir().unwrap();
        let state = dir.path().join("state.json");
        let files = two_account_shape(dir.path(), 10, 20);
        let (site, seen) = serve(vec![(403, r#"{"error":"forbidden"}"#), (404, "{}"), (200, r#"{"stored":"unchanged"}"#)]);
        let src = || Some(RanksSource { addon: "Example", guild: "Example" });
        let r = run_all(&files, &site, "upl_test", &state, src()).unwrap();
        assert_eq!(r.ranks.unwrap().line(), format!("ranks {}", ranks::NEEDS_OFFICER));
        assert!(run_all(&files, &site, "upl_test", &state, src()).unwrap().ranks.is_none());
        // /reload writes a newer t: try again. 404 is not final, so the next pass retries it.
        let files = two_account_shape(dir.path(), 10, 30);
        assert!(!run_all(&files, &site, "upl_test", &state, src()).unwrap().ranks.unwrap().is_final());
        assert!(run_all(&files, &site, "upl_test", &state, src()).unwrap().ranks.unwrap().is_final());
        assert_eq!(seen.lock().unwrap().len(), 3);
    }

    #[test]
    fn rank_names_with_a_dead_token_stop_the_pass() {
        let dir = tempfile::tempdir().unwrap();
        let files = two_account_shape(dir.path(), 1, 2);
        let (site, _) = serve(vec![(401, r#"{"error":"bad_token"}"#)]);
        let err = run_all(&files, &site, "upl_x", &dir.path().join("s.json"), Some(RanksSource { addon: "Example", guild: "" })).unwrap_err();
        assert!(drops_saved_token(&err));
    }

    #[test]
    fn plain_run_never_sends_rank_names() {
        let dir = tempfile::tempdir().unwrap();
        let files = two_account_shape(dir.path(), 1, 2);
        let r = run(&files, "http://127.0.0.1:9", "upl_x", &dir.path().join("s.json")).unwrap();
        assert!(!r.requested());
    }

    #[test]
    fn old_state_files_still_load() {
        let p = std::env::temp_dir().join(format!("gu-old-state-{}.json", std::process::id()));
        fs::write(&p, r#"{"sent":["a"]}"#).unwrap();
        let s = State::load(&p);
        assert!(s.sent.contains("a") && s.ranks_last.is_none());
        let _ = fs::remove_file(p);
    }

    #[test]
    fn state_roundtrip() {
        let p = std::env::temp_dir().join(format!("gu-state-{}.json", std::process::id()));
        let mut s = State::default(); s.sent.insert("a".into()); s.save(&p).unwrap();
        assert!(State::load(&p).sent.contains("a")); let _ = fs::remove_file(p);
    }
}
