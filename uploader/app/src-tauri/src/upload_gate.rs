//! Shared upload result for the button, the tray, and the file watcher.
//! The watcher calls this from its own thread. Nothing here listens for a UI event.

use guild_uploader_core as core;
use notify_debouncer_mini::DebouncedEventKind;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::config::Prefs;

pub struct UploadSeq {
    next: AtomicU64,
    applied_success: AtomicU64,
}

impl Default for UploadSeq {
    fn default() -> Self {
        Self { next: AtomicU64::new(0), applied_success: AtomicU64::new(0) }
    }
}

impl UploadSeq {
    pub fn note_start(&self) -> u64 {
        self.next.fetch_add(1, Ordering::SeqCst) + 1
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SavedUpload {
    pub found: usize,
    pub sent: usize,
    pub lines: Vec<String>,
    pub counts: Vec<crate::config::SentCount>,
    pub at: String,
    pub status: String,
    /// Whether this pass sent anything. Only then is it the "Last upload"; otherwise it is "Last checked".
    pub requested: bool,
}

#[derive(Debug)]
pub enum Finish {
    Applied(SavedUpload),
    /// A newer successful upload already wrote the prefs. Leave them alone.
    Stale,
    Failed { drop_token: bool, message: String },
}

/// A settled file change is worth an upload. Continuous writes are not a second trigger.
/// Window visibility is not an input.
pub fn debounced_change_should_upload(kinds: impl IntoIterator<Item = DebouncedEventKind>) -> bool {
    kinds.into_iter().any(|kind| kind == DebouncedEventKind::Any)
}

pub fn upload_saved_variables(
    game_folder: &str,
    addon_name: &str,
    guild_name: &str,
    site: &str,
    token: &str,
    state_path: &Path,
) -> Result<SavedUpload, core::UploadError> {
    let files = core::saved_variables(&PathBuf::from(game_folder), addon_name);
    if files.is_empty() {
        core::log::warn("upload", &format!("no SavedVariables for {addon_name} under {game_folder}"));
        return Err(core::UploadError::NoSavedVariables(addon_name.to_string()));
    }
    core::log::info("upload", &format!("checking {} SavedVariables file(s)", files.len()));
    let ranks = core::RanksSource { addon: addon_name, guild: guild_name };
    Ok(saved_from_report(core::run_all(&files, site, token, state_path, Some(ranks))?))
}

pub fn saved_from_report(report: core::Report) -> SavedUpload {
    let requested = report.requested();
    let mut lines: Vec<String> = report
        .results
        .iter()
        .map(|(_, outcome)| {
            format!(
                "{} {}",
                outcome.ty.as_deref().unwrap_or("?"),
                if outcome.ok { "accepted".to_string() } else { outcome.error.clone().unwrap_or_default() }
            )
        })
        .collect();
    let mut accepted = report.results.iter().filter(|(_, outcome)| core::done(outcome)).count();
    if let Some(ranks) = &report.ranks {
        lines.push(ranks.line());
        if matches!(ranks, core::RanksOutcome::Stored(_)) {
            accepted += 1;
        }
    }
    let status = if !requested { "nothing new".into() } else { format!("sent {accepted}") };
    SavedUpload {
        found: report.found,
        sent: accepted,
        lines,
        counts: counts_from_report(&report),
        at: unix_now(),
        status,
        requested,
    }
}


fn counts_from_report(report: &core::Report) -> Vec<crate::config::SentCount> {
    use std::collections::BTreeMap;
    let mut by_ty: BTreeMap<&str, usize> = BTreeMap::new();
    for (raw, outcome) in &report.results {
        if !core::done(outcome) {
            continue;
        }
        if let Some((ty, n)) = core::envelope_count(raw) {
            *by_ty.entry(ty).or_insert(0) += n;
        }
    }
    if matches!(report.ranks, Some(core::RanksOutcome::Stored(_))) {
        if let Some(n) = report.ranks_count {
            by_ty.insert("ranks", n);
        }
    }
    const ORDER: &[&str] = &["ranks", "gbank", "recipes"];
    let mut out = Vec::new();
    for ty in ORDER {
        if let Some(n) = by_ty.remove(ty) {
            out.push(crate::config::SentCount { ty: (*ty).into(), n });
        }
    }
    for (ty, n) in by_ty {
        out.push(crate::config::SentCount { ty: ty.into(), n });
    }
    out
}

/// Apply one finished upload. Callers hold the prefs lock for the whole call so the
/// status, time, and lines stay one record. An older success cannot replace a newer one.
/// A failure does not rewrite a stored success. A bad token drops the saved token only
/// when no newer success has already been kept.
pub fn finish_upload(prefs: &mut Prefs, seq: &UploadSeq, started: u64, result: Result<SavedUpload, core::UploadError>) -> Finish {
    let applied = seq.applied_success.load(Ordering::SeqCst);
    if started < applied {
        return Finish::Stale;
    }
    match result {
        Ok(upload) => {
            if upload.requested {
                prefs.last_upload_at = Some(upload.at.clone());
                prefs.last_sent_lines = Some(upload.lines.clone());
                prefs.last_sent_counts = Some(upload.counts.clone());
                prefs.last_status = Some(upload.status.clone());
            } else if prefs.last_status.is_none() {
                prefs.last_status = Some(upload.status.clone());
            }
            prefs.last_checked_at = Some(upload.at.clone());
            seq.applied_success.store(started, Ordering::SeqCst);
            Finish::Applied(upload)
        }
        Err(err) => Finish::Failed {
            drop_token: core::drops_saved_token(&err),
            message: err.to_string(),
        },
    }
}

/// Network and rate-limit failures may be tried again. A bad token, an HTTP
/// refusal, and a per-string result are not retried.
pub fn transient_retry(err: &core::UploadError, attempt: u32) -> bool {
    attempt < 3 && matches!(err, core::UploadError::Network(_) | core::UploadError::RateLimited)
}

pub fn retry_pause(attempt: u32) -> std::time::Duration {
    std::time::Duration::from_millis(200 * u64::from(attempt.max(1))).min(std::time::Duration::from_secs(1))
}

/// One upload runs at a time. A trigger that arrives during it is remembered
/// and runs once more, instead of stacking parallel uploads.
pub struct Flight {
    inner: std::sync::Mutex<FlightState>,
}

struct FlightState {
    running: bool,
    pending: bool,
}

impl Default for Flight {
    fn default() -> Self {
        Self { inner: std::sync::Mutex::new(FlightState { running: false, pending: false }) }
    }
}

impl Flight {
    pub fn acquire(&self) -> bool {
        let mut guard = self.inner.lock().unwrap_or_else(|err| err.into_inner());
        if guard.running {
            guard.pending = true;
            false
        } else {
            guard.running = true;
            true
        }
    }

    /// `true` when a trigger arrived while this upload was running.
    pub fn take_repeat(&self) -> bool {
        let mut guard = self.inner.lock().unwrap_or_else(|err| err.into_inner());
        if guard.pending {
            guard.pending = false;
            true
        } else {
            guard.running = false;
            false
        }
    }
}

fn unix_now() -> String {
    let secs = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    format!("{secs}")
}

/// Errors worth an automatic log report. Waiting for SavedVariables (the addon hasn't run yet) and
/// the site asking us to slow down are expected states, so they are only logged.
pub fn reportable(err: &core::UploadError) -> bool {
    !matches!(err, core::UploadError::NoSavedVariables(_) | core::UploadError::RateLimited)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_saved_variables_is_not_called_a_network_failure() {
        let dir = tempfile::tempdir().unwrap();
        let err = upload_saved_variables(&dir.path().display().to_string(), "Addon", "", "http://127.0.0.1:9", "upl_x", &dir.path().join("s.json")).unwrap_err();
        let text = err.to_string();
        assert!(!text.to_lowercase().contains("reach the site"), "{text}");
        assert!(text.contains("SavedVariables for Addon"), "{text}");
        assert!(!reportable(&err));
        assert!(reportable(&core::UploadError::Network("x".into())));
        assert!(reportable(&core::UploadError::Http(500, "x".into())));
    }
    use std::fs;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};
    use std::thread;

    fn prefs_with_success() -> Prefs {
        Prefs {
            game_folder: "C:\\Games\\Wow\\_forever_".into(),
            addon_name: "Addon".into(),
            start_at_login: true,
            last_upload_at: Some("100".into()),
            last_status: Some("sent 2".into()),
            last_sent_lines: Some(vec!["recipes accepted".into(), "gbank accepted".into()]),
            ..Prefs::default()
        }
    }

    fn saved(status: &str, lines: &[&str]) -> SavedUpload {
        SavedUpload {
            found: lines.len(),
            sent: 1,
            lines: lines.iter().map(|s| (*s).to_string()).collect(),
            counts: vec![],
            at: "200".into(),
            status: status.into(),
            requested: true,
        }
    }

    #[test]
    fn a_pass_that_sends_nothing_is_last_checked_not_last_upload() {
        let seq = UploadSeq::default();
        let started = seq.note_start();
        let mut prefs = prefs_with_success();
        let quiet = SavedUpload { found: 0, sent: 0, lines: vec![], counts: vec![], at: "300".into(), status: "nothing new".into(), requested: false };
        assert!(matches!(finish_upload(&mut prefs, &seq, started, Ok(quiet)), Finish::Applied(_)));
        assert_eq!(prefs.last_upload_at.as_deref(), Some("100"));
        assert_eq!(prefs.last_checked_at.as_deref(), Some("300"));
        assert_eq!(prefs.last_status.as_deref(), Some("sent 2"));
        assert_eq!(prefs.last_sent_lines.as_deref(), Some(["recipes accepted".to_string(), "gbank accepted".to_string()].as_slice()));
    }

    #[test]
    fn rank_names_count_as_an_upload_with_a_clear_line() {
        let stored = core::Report { found: 0, sent: 0, results: vec![], ranks: Some(core::RanksOutcome::Stored("stored".into())), ranks_count: Some(5) };
        let up = saved_from_report(stored);
        assert!(up.requested);
        assert_eq!((up.status.as_str(), up.lines.clone()), ("sent 1", vec!["ranks accepted".to_string()]));
        assert_eq!(up.counts, vec![crate::config::SentCount { ty: "ranks".into(), n: 5 }]);
        let refused = core::Report { found: 0, sent: 0, results: vec![], ranks: Some(core::RanksOutcome::Refused(core::ranks::NEEDS_OFFICER.into())), ranks_count: None };
        let up = saved_from_report(refused);
        assert_eq!(up.status, "sent 0");
        assert_eq!(up.lines, vec!["ranks Rank names need a Guild Master or officer character linked".to_string()]);
        let none = saved_from_report(core::Report { found: 0, sent: 0, results: vec![], ranks: None, ranks_count: None });
        assert!(!none.requested);
        assert_eq!(none.status, "nothing new");
    }

    #[test]
    fn two_account_files_send_rank_names_through_the_app_path() {
        let dir = tempfile_dir("gu-ranks");
        for acct in ["300769113#1", "300769113#2"] {
            let file = dir.join("WTF").join("Account").join(acct).join("SavedVariables").join("Example.lua");
            fs::create_dir_all(file.parent().unwrap()).unwrap();
            fs::write(file, "ExampleDB = {\n\t[\"chars\"] = {\n\t},\n\t[\"guild\"] = {\n\t\t[\"t\"] = 1794112233,\n\t\t[\"name\"] = \"Example\",\n\t\t[\"ranks\"] = {\n\t\t\t\"Guild Master\", -- [1]\n\t\t\t\"Officer\", -- [2]\n\t\t\t\"Veteran\", -- [3]\n\t\t\t\"Member\", -- [4]\n\t\t\t\"Initiate\", -- [5]\n\t\t},\n\t},\n\t[\"outbox\"] = {\n\t},\n}\n").unwrap();
        }
        let state = dir.join("state.json");
        let fixture = serve(403, r#"{"error":"forbidden"}"#);
        let up = upload_saved_variables(&dir.display().to_string(), "Example", "Example", &fixture.site, "upl_fixture_not_secret", &state).unwrap();
        let request = fixture.seen.recv_timeout(std::time::Duration::from_secs(5)).unwrap();
        assert!(request.starts_with("POST /api/uploader/ranks "));
        assert!(request.contains("\"ranks\":[\"Guild Master\",\"Officer\",\"Veteran\",\"Member\",\"Initiate\"]"));
        assert!(up.requested);
        assert_eq!(up.lines, vec![format!("ranks {}", core::ranks::NEEDS_OFFICER)]);
        let _ = fs::remove_dir_all(&dir);
    }

    fn tempfile_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn transient_errors_retry_and_auth_or_http_do_not() {
        assert!(transient_retry(&core::UploadError::Network("offline".into()), 0));
        assert!(transient_retry(&core::UploadError::Network("offline".into()), 2));
        assert!(!transient_retry(&core::UploadError::Network("offline".into()), 3));
        assert!(transient_retry(&core::UploadError::RateLimited, 1));
        assert!(!transient_retry(&core::UploadError::BadToken, 0));
        assert!(!transient_retry(&core::UploadError::Http(403, "no".into()), 0));
        assert!(!transient_retry(&core::UploadError::NoSavedVariables("Addon".into()), 0));
        assert!(!transient_retry(&core::UploadError::Local("keychain".into()), 0));
        assert!(retry_pause(1) <= std::time::Duration::from_secs(1));
    }

    #[test]
    fn a_second_trigger_waits_instead_of_overlapping() {
        let flight = Flight::default();
        assert!(flight.acquire());
        assert!(!flight.acquire());
        assert!(flight.take_repeat());
        assert!(!flight.take_repeat());
        assert!(flight.acquire());
    }

    #[test]
    fn settled_change_uploads_and_a_continuous_write_does_not() {
        assert!(debounced_change_should_upload([DebouncedEventKind::Any]));
        assert!(!debounced_change_should_upload([DebouncedEventKind::AnyContinuous]));
        assert!(!debounced_change_should_upload([]));
    }

    #[test]
    fn newer_success_wins_whichever_thread_locks_first() {
        let seq = UploadSeq::default();
        let older = seq.note_start();
        let newer = seq.note_start();
        let prefs = Arc::new(Mutex::new(Prefs::default()));
        let seq = Arc::new(seq);
        thread::scope(|scope| {
            let prefs_b = Arc::clone(&prefs);
            let seq_b = Arc::clone(&seq);
            scope.spawn(move || {
                let mut prefs = prefs_b.lock().unwrap();
                let finish = finish_upload(&mut prefs, &seq_b, newer, Ok(saved("sent 9", &["gbank accepted"])));
                assert!(matches!(finish, Finish::Applied(_)));
            });
            let prefs_a = Arc::clone(&prefs);
            let seq_a = Arc::clone(&seq);
            scope.spawn(move || {
                let mut prefs = prefs_a.lock().unwrap();
                let _ = finish_upload(&mut prefs, &seq_a, older, Ok(saved("sent 1", &["recipes accepted"])));
            });
        });
        let prefs = prefs.lock().unwrap();
        assert_eq!(prefs.last_status.as_deref(), Some("sent 9"));
        assert_eq!(prefs.last_sent_lines.as_deref(), Some(["gbank accepted".to_string()].as_slice()));
        assert_eq!(prefs.last_upload_at.as_deref(), Some("200"));
    }

    #[test]
    fn stale_failure_does_not_drop_the_token_or_rewrite_a_newer_success() {
        let seq = UploadSeq::default();
        let older = seq.note_start();
        let newer = seq.note_start();
        let mut prefs = prefs_with_success();
        let applied = finish_upload(&mut prefs, &seq, newer, Ok(saved("sent 4", &["recipes accepted"])));
        assert!(matches!(applied, Finish::Applied(_)));
        let stale = finish_upload(&mut prefs, &seq, older, Err(core::UploadError::BadToken));
        assert!(matches!(stale, Finish::Stale));
        assert_eq!(prefs.last_status.as_deref(), Some("sent 4"));
        assert_eq!(prefs.last_sent_lines.as_deref(), Some(["recipes accepted".to_string()].as_slice()));
        assert!(prefs.start_at_login);
        assert_eq!(prefs.game_folder, "C:\\Games\\Wow\\_forever_");
    }

    #[test]
    fn bad_token_keeps_the_last_success_and_asks_to_drop_the_token() {
        let seq = UploadSeq::default();
        let started = seq.note_start();
        let mut prefs = prefs_with_success();
        let finish = finish_upload(&mut prefs, &seq, started, Err(core::UploadError::BadToken));
        match finish {
            Finish::Failed { drop_token, message } => {
                assert!(drop_token);
                assert!(message.contains("rejected the uploader token"));
                assert!(!message.contains("upl_"));
            }
            other => panic!("expected token drop, got {other:?}"),
        }
        assert_eq!(prefs.last_status.as_deref(), Some("sent 2"));
        assert_eq!(
            prefs.last_sent_lines.as_deref(),
            Some(["recipes accepted".to_string(), "gbank accepted".to_string()].as_slice())
        );
    }

    #[test]
    fn other_failure_does_not_drop_the_token_or_clear_a_success() {
        let seq = UploadSeq::default();
        let started = seq.note_start();
        let mut prefs = prefs_with_success();
        let finish = finish_upload(
            &mut prefs,
            &seq,
            started,
            Err(core::UploadError::Http(403, "forbidden".into())),
        );
        match finish {
            Finish::Failed { drop_token, message } => {
                assert!(!drop_token);
                assert!(message.contains("403"));
            }
            other => panic!("expected ordinary failure, got {other:?}"),
        }
        assert_eq!(prefs.last_status.as_deref(), Some("sent 2"));
    }

    #[test]
    fn per_string_refusal_is_stored_and_keeps_the_token() {
        let seq = UploadSeq::default();
        let started = seq.note_start();
        let mut prefs = prefs_with_success();
        let upload = saved("sent 1", &["gbank site_refused", "recipes accepted"]);
        let finish = finish_upload(&mut prefs, &seq, started, Ok(upload));
        match finish {
            Finish::Applied(saved) => {
                assert_eq!(saved.status, "sent 1");
                assert!(saved.lines.iter().any(|line| line.contains("site_refused")));
            }
            other => panic!("expected stored refusal, got {other:?}"),
        }
        assert_eq!(prefs.last_status.as_deref(), Some("sent 1"));
        assert!(prefs.last_sent_lines.as_ref().unwrap().iter().any(|line| line == "recipes accepted"));
    }

    struct Fixture {
        site: String,
        seen: std::sync::mpsc::Receiver<String>,
    }

    fn serve(status: u16, body: &'static str) -> Fixture {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let (tx, rx) = std::sync::mpsc::channel();
        thread::spawn(move || {
            let (mut sock, _) = listener.accept().unwrap();
            sock.set_read_timeout(Some(std::time::Duration::from_secs(5))).unwrap();
            let mut buf = Vec::new();
            let mut tmp = [0u8; 2048];
            loop {
                match sock.read(&mut tmp) {
                    Ok(0) => break,
                    Ok(n) => {
                        buf.extend_from_slice(&tmp[..n]);
                        if let Some(end) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                            let headers = String::from_utf8_lossy(&buf[..end]);
                            let len = headers
                                .lines()
                                .find_map(|line| {
                                    let rest = line.split_once(':')?;
                                    if rest.0.eq_ignore_ascii_case("content-length") {
                                        rest.1.trim().parse::<usize>().ok()
                                    } else {
                                        None
                                    }
                                })
                                .unwrap_or(0);
                            if buf.len() >= end + 4 + len {
                                break;
                            }
                        }
                    }
                    Err(_) => break,
                }
            }
            let _ = tx.send(String::from_utf8_lossy(&buf).to_string());
            let resp = format!(
                "HTTP/1.1 {status} OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = sock.write_all(resp.as_bytes());
        });
        Fixture { site: format!("http://127.0.0.1:{port}"), seen: rx }
    }

    fn write_saved(dir: &Path, lua: &str) {
        let file = dir.join("WTF").join("Account").join("Test").join("SavedVariables").join("Addon.lua");
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(file, lua).unwrap();
    }

    #[test]
    fn file_change_posts_and_stores_without_a_ui_listener() {
        let dir = std::env::temp_dir().join(format!("gu-watch-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        write_saved(&dir, "x = \"!BF:recipes:v1:Ab9\"\n");
        let state = std::env::temp_dir().join(format!("gu-watch-state-{}.json", std::process::id()));
        let _ = fs::remove_file(&state);
        let fixture = serve(200, r#"{"results":[{"type":"recipes","ok":true}]}"#);
        assert!(debounced_change_should_upload([DebouncedEventKind::Any]));
        let uploaded = upload_saved_variables(
            &dir.display().to_string(),
            "Addon",
            "",
            &fixture.site,
            "upl_fixture_not_secret",
            &state,
        )
        .unwrap();
        let request = fixture.seen.recv_timeout(std::time::Duration::from_secs(5)).unwrap();
        assert!(request.starts_with("POST /api/uploader/ingest "));
        assert!(request.contains("upl_fixture_not_secret"));
        assert!(request.contains("!BF:recipes:v1:Ab9"));
        let seq = UploadSeq::default();
        let started = seq.note_start();
        let mut prefs = Prefs { game_folder: dir.display().to_string(), addon_name: "Addon".into(), ..Prefs::default() };
        let finish = finish_upload(&mut prefs, &seq, started, Ok(uploaded));
        assert!(matches!(finish, Finish::Applied(_)));
        assert_eq!(prefs.last_status.as_deref(), Some("sent 1"));
        assert_eq!(prefs.last_sent_lines.as_deref(), Some(["recipes accepted".to_string()].as_slice()));
        let stored = core::State::load(&state);
        assert!(stored.sent.contains(&core::digest("!BF:recipes:v1:Ab9")));
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_file(&state);
    }

    #[test]
    fn per_string_http_refusal_stays_unfinished_and_keeps_the_token_decision() {
        let dir = std::env::temp_dir().join(format!("gu-mixed-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        write_saved(&dir, "a = \"!BF:gbank:v1:Ab9\"\nb = \"!BF:recipes:v1:Cd8\"\n");
        let state = std::env::temp_dir().join(format!("gu-mixed-state-{}.json", std::process::id()));
        let _ = fs::remove_file(&state);
        let fixture = serve(
            200,
            r#"{"results":[{"type":"gbank","ok":false,"error":"site_refused"},{"type":"recipes","ok":true}]}"#,
        );
        let uploaded = upload_saved_variables(&dir.display().to_string(), "Addon", "", &fixture.site, "upl_fixture_not_secret", &state).unwrap();
        assert_eq!(uploaded.status, "sent 1");
        assert!(uploaded.lines.iter().any(|line| line == "gbank site_refused"));
        assert!(uploaded.lines.iter().any(|line| line == "recipes accepted"));
        let seq = UploadSeq::default();
        let started = seq.note_start();
        let mut prefs = prefs_with_success();
        let finish = finish_upload(&mut prefs, &seq, started, Ok(uploaded));
        assert!(matches!(finish, Finish::Applied(_)));
        assert_eq!(prefs.last_status.as_deref(), Some("sent 1"));
        let stored = core::State::load(&state);
        assert!(stored.sent.contains(&core::digest("!BF:recipes:v1:Cd8")));
        assert!(!stored.sent.contains(&core::digest("!BF:gbank:v1:Ab9")));
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_file(&state);
    }

    #[test]
    fn http_bad_token_does_not_replace_the_last_success() {
        let dir = std::env::temp_dir().join(format!("gu-token-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        write_saved(&dir, "a = \"!BF:recipes:v1:Ab9\"\n");
        let state = std::env::temp_dir().join(format!("gu-token-state-{}.json", std::process::id()));
        let _ = fs::remove_file(&state);
        let fixture = serve(401, r#"{"error":"bad_token","token":"upl_fixture_not_secret"}"#);
        let err = upload_saved_variables(&dir.display().to_string(), "Addon", "", &fixture.site, "upl_fixture_not_secret", &state).unwrap_err();
        assert!(core::drops_saved_token(&err));
        assert!(!err.to_string().contains("upl_fixture_not_secret"));
        let seq = UploadSeq::default();
        let started = seq.note_start();
        let mut prefs = prefs_with_success();
        let finish = finish_upload(&mut prefs, &seq, started, Err(err));
        match finish {
            Finish::Failed { drop_token, .. } => assert!(drop_token),
            other => panic!("expected drop, got {other:?}"),
        }
        assert_eq!(prefs.last_status.as_deref(), Some("sent 2"));
        assert_eq!(
            prefs.last_sent_lines.as_deref(),
            Some(["recipes accepted".to_string(), "gbank accepted".to_string()].as_slice())
        );
        assert!(core::State::load(&state).sent.is_empty());
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_file(&state);
    }
}
