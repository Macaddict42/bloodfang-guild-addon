//! Golden HTTP fixtures (contract/golden/http, contract/SPEC.md section 7).
//! Requests are generated from the code that sends them and must match the committed files byte for byte;
//! `WRITE_GOLDEN=1 cargo test -p guild-uploader-core golden` writes them (for a NEW case only, SPEC 5.2).
//! Responses are written by hand from SPEC 4.2; this test proves the uploader reads each one as the SPEC says.
use crate::ranks::{interpret_ranks, ranks_body, GuildRanks, RanksOutcome, NEEDS_OFFICER};
use crate::report::{log_outcome, LogReport, SendOutcome};
use crate::{ingest_body, interpret_ingest, UploadError};
use serde_json::{json, Value};
use std::path::PathBuf;

fn root() -> PathBuf { PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../contract/golden") }

fn string(name: &str) -> String {
    let p = root().join("strings").join(name);
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display())).trim_end().to_string()
}

fn request(method: &str, path: &str, auth: bool, body: Value) -> String {
    let mut v = json!({ "method": method, "path": path, "auth": if auth { "Bearer <uploader token>" } else { "none" } });
    if !body.is_null() { v["body"] = body; }
    serde_json::to_string_pretty(&v).unwrap() + "\n"
}

fn requests() -> Vec<(String, String)> {
    let strings = vec![string("gbank.v1.txt"), string("recipes.v1.txt"), string("ranks.v1.txt")];
    let ingest = request("POST", "/api/uploader/ingest", true, ingest_body(&strings));
    let ranks = GuildRanks { guild: "Example Guild".into(), t: 1793836800, ranks: ["Guild Master", "Officer", "Raid Lead", "Guild Member", "Trial", "Alt"].iter().map(|s| s.to_string()).collect() };
    let ranks = request("POST", "/api/uploader/ranks", true, ranks_body(&ranks));
    let log = LogReport { error_code: "upload".into(), message: "couldn't reach the site".into(), app_version: "0.1.4".into(), os: "windows x86_64".into(), addon_version: Some("0.2.1".into()), log: "2026-11-13T12:00:00Z INFO upload start\n".into() };
    let log_v = serde_json::to_value(&log).unwrap();
    let mut out = vec![];
    for case in ["ok-mixed-results", "bad-token", "rate-limited", "too-large", "bad-request"] {
        out.push((format!("ingest/{case}.request.json"), ingest.clone()));
    }
    for case in ["stored", "older", "not-officer", "bad-ranks", "bad-token", "rate-limited"] {
        out.push((format!("ranks/{case}.request.json"), ranks.clone()));
    }
    for (case, auth) in [("stored-token", true), ("stored-anonymous", false), ("bad-error-code", true), ("rate-limited", true)] {
        out.push((format!("logs/{case}.request.json"), request("POST", "/api/uploader/logs", auth, log_v.clone())));
    }
    for case in ["ok", "not-published"] {
        out.push((format!("addon-latest/{case}.request.json"), request("GET", "/api/addon/latest", false, Value::Null)));
    }
    out
}

#[test]
fn golden_requests_match_what_the_uploader_sends() {
    let write = std::env::var("WRITE_GOLDEN").is_ok();
    let mut bad = vec![];
    for (rel, text) in requests() {
        let p = root().join("http").join(&rel);
        if write {
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(&p, &text).unwrap();
        } else {
            match std::fs::read_to_string(&p) {
                Ok(t) if t == text => {}
                Ok(_) => bad.push(format!("differs http/{rel}")),
                Err(_) => bad.push(format!("missing http/{rel}")),
            }
        }
    }
    assert!(bad.is_empty(), "golden requests differ from contract/golden (a changed request needs a new case, SPEC 5.2):\n{}", bad.join("\n"));
}

fn response(rel: &str) -> (u16, String) {
    let p = root().join("http").join(rel);
    let v: Value = serde_json::from_str(&std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))).unwrap();
    (v["status"].as_u64().unwrap() as u16, v["body"].to_string())
}

#[test]
fn golden_ingest_responses_read_as_the_spec_says() {
    let (s, b) = response("ingest/ok-mixed-results.response.json");
    let r = interpret_ingest(s, &b).unwrap();
    let got: Vec<_> = r.iter().map(|o| (o.ty.clone().unwrap_or_default(), o.ok, o.error.clone())).collect();
    assert_eq!(got, vec![("gbank".into(), true, None), ("recipes".into(), false, Some("bad_sig".into())), ("ranks".into(), false, Some("duplicate".into()))]);
    assert!(crate::done(&r[2]), "duplicate counts as done");
    let (s, b) = response("ingest/bad-token.response.json");
    assert!(matches!(interpret_ingest(s, &b), Err(UploadError::BadToken)));
    let (s, b) = response("ingest/rate-limited.response.json");
    assert!(matches!(interpret_ingest(s, &b), Err(UploadError::RateLimited)));
    let (s, b) = response("ingest/too-large.response.json");
    assert!(matches!(interpret_ingest(s, &b), Err(UploadError::Http(413, _))));
    let (s, b) = response("ingest/bad-request.response.json");
    assert!(matches!(interpret_ingest(s, &b), Err(UploadError::Http(400, _))));
}

#[test]
fn golden_ranks_responses_read_as_the_spec_says() {
    let read = |c: &str| { let (s, b) = response(&format!("ranks/{c}.response.json")); interpret_ranks(s, &b) };
    assert_eq!(read("stored"), RanksOutcome::Stored("stored".into()));
    // SPEC 4.2: {stored:false} means the block is older than the stored one; today the uploader calls it "unchanged".
    assert_eq!(read("older"), RanksOutcome::Stored("unchanged".into()));
    assert_eq!(read("not-officer"), RanksOutcome::Refused(NEEDS_OFFICER.into()));
    assert_eq!(read("bad-ranks"), RanksOutcome::Refused("the site couldn't use these rank names (bad_ranks)".into()));
    assert_eq!(read("bad-token"), RanksOutcome::BadToken);
    assert!(matches!(read("rate-limited"), RanksOutcome::Retry(_)));
}

#[test]
fn golden_logs_and_latest_responses_read_as_the_spec_says() {
    for (c, want) in [("stored-token", SendOutcome::Stored), ("stored-anonymous", SendOutcome::Stored), ("bad-error-code", SendOutcome::Refused(400)), ("rate-limited", SendOutcome::Refused(429))] {
        assert_eq!(log_outcome(response(&format!("logs/{c}.response.json")).0), want, "{c}");
    }
    let (s, b) = response("addon-latest/ok.response.json");
    assert_eq!(s, 200);
    let rel = crate::update::parse_site_latest(&b, "Guildaddon").unwrap().expect("a usable release");
    assert_eq!((rel.version.to_string(), rel.zip_name.as_str(), rel.source), ("0.2.1".into(), "Guildaddon-0.2.1.zip", "site"));
    assert_eq!(response("addon-latest/not-published.response.json").0, 404);
}
