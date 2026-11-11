//! Automatic error reports: the redacted recent log plus a few facts, posted to the site's
//! `/api/uploader/logs`. Signed-in reports carry the uploader token; before sign-in they go
//! anonymously and the site rate-limits them. Each error code is sent at most once an hour.
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

/// Most log text one report carries.
pub const MAX_LOG_BYTES: usize = 64 * 1024;
/// Per error code.
pub const DEBOUNCE_SECS: u64 = 3600;

/// Error codes the app reports. The site accepts `[a-z0-9_]{1,40}`.
pub mod code {
    pub const ADDON_CHECK: &str = "addon_check";
    pub const ADDON_INSTALL: &str = "addon_install";
    pub const UPLOAD: &str = "upload";
    pub const APP_UPDATE_CHECK: &str = "app_update_check";
    pub const APP_UPDATE_INSTALL: &str = "app_update_install";
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LogReport {
    pub error_code: String,
    /// Short, redacted error text.
    pub message: String,
    pub app_version: String,
    pub os: String,
    pub addon_version: Option<String>,
    /// Redacted log tail, at most `MAX_LOG_BYTES`.
    pub log: String,
}

impl LogReport {
    pub fn new(error_code: &str, message: &str, app_version: &str, addon_version: Option<String>, log: String) -> Self {
        LogReport {
            error_code: error_code.to_string(),
            message: crate::log::redact(message).chars().take(500).collect(),
            app_version: app_version.to_string(),
            os: format!("{} {}", std::env::consts::OS, std::env::consts::ARCH),
            addon_version,
            log: clamp_tail(&crate::log::redact(&log), MAX_LOG_BYTES),
        }
    }
}

/// Keep the newest `max` bytes, starting on a character (and, if cut, a line) boundary.
pub fn clamp_tail(s: &str, max: usize) -> String {
    if s.len() <= max { return s.to_string() }
    let mut start = s.len() - max;
    while !s.is_char_boundary(start) { start += 1 }
    let cut = &s[start..];
    match cut.find('\n') { Some(i) if i + 1 < cut.len() => cut[i + 1..].to_string(), _ => cut.to_string() }
}

/// When each error code was last sent (unix seconds), kept on disk so a restart doesn't resend.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Debounce {
    #[serde(default)]
    pub last: BTreeMap<String, u64>,
}

impl Debounce {
    pub fn load(p: &Path) -> Debounce {
        fs::read_to_string(p).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
    }
    pub fn save(&self, p: &Path) -> std::io::Result<()> {
        if let Some(d) = p.parent() { fs::create_dir_all(d)? }
        let tmp = p.with_extension("tmp");
        fs::write(&tmp, serde_json::to_vec(self).unwrap_or_default())?;
        fs::rename(tmp, p)
    }
    /// True (and remembered) when `code` wasn't sent in the last hour. A clock that moved
    /// backwards counts as "sent just now" rather than allowing a burst.
    pub fn allow(&mut self, code: &str, now: u64) -> bool {
        if let Some(&t) = self.last.get(code) {
            if now < t.saturating_add(DEBOUNCE_SECS) && now + DEBOUNCE_SECS > t {
                return false;
            }
        }
        self.last.insert(code.to_string(), now);
        true
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum SendOutcome {
    Stored,
    /// The site refused or limited it; not retried.
    Refused(u16),
}

/// POST the report. `token` is the uploader token when signed in.
pub fn send(site: &str, token: Option<&str>, report: &LogReport) -> Result<SendOutcome, String> {
    let url = format!("{}/api/uploader/logs", site.trim_end_matches('/'));
    let mut req = crate::http::site_agent().post(&url).set("Content-Type", "application/json");
    if let Some(t) = token.filter(|t| !t.is_empty()) {
        req = req.set("Authorization", &format!("Bearer {t}"));
    }
    match req.send_json(report) {
        Ok(_) => Ok(SendOutcome::Stored),
        Err(ureq::Error::Status(code, _)) => Ok(SendOutcome::Refused(code)),
        Err(e) => Err(crate::http::describe(e)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_report_per_code_per_hour() {
        let mut d = Debounce::default();
        assert!(d.allow(code::UPLOAD, 10_000));
        assert!(!d.allow(code::UPLOAD, 10_000 + 60));
        assert!(!d.allow(code::UPLOAD, 10_000 + DEBOUNCE_SECS - 1));
        assert!(d.allow(code::ADDON_CHECK, 10_100), "another code is independent");
        assert!(d.allow(code::UPLOAD, 10_000 + DEBOUNCE_SECS));
        assert!(!d.allow(code::UPLOAD, 10_000 + DEBOUNCE_SECS + 5));
    }

    #[test]
    fn debounce_survives_a_restart_and_a_clock_jump() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("report-debounce.json");
        let mut d = Debounce::default();
        assert!(d.allow(code::ADDON_INSTALL, 50_000));
        d.save(&p).unwrap();
        let mut again = Debounce::load(&p);
        assert!(!again.allow(code::ADDON_INSTALL, 50_100));
        // Clock set back by 10 minutes: still suppressed.
        assert!(!again.allow(code::ADDON_INSTALL, 49_400));
        // Far in the past (clock reset): allowed, so a broken clock can't silence reports forever.
        assert!(again.allow(code::ADDON_INSTALL, 1_000));
        assert!(Debounce::load(&dir.path().join("missing.json")).last.is_empty());
    }

    #[test]
    fn report_is_capped_and_redacted() {
        let big = format!("{}\nBearer upl_abcdefghijklmnopqrstuvwxyz0123456789ABCDEFG tail\n", "a".repeat(100_000));
        let r = LogReport::new(code::UPLOAD, "failed with upl_abcdefghijklmnopqrstuvwxyz0123456789ABCDEFG", "0.1.3", Some("0.2.1".into()), big);
        assert!(r.log.len() <= MAX_LOG_BYTES);
        assert!(r.log.ends_with("tail\n"));
        assert!(!r.log.contains("abcdefghij") && !r.message.contains("abcdefghij"));
        let json = serde_json::to_value(&r).unwrap();
        for k in ["errorCode", "message", "appVersion", "os", "addonVersion", "log"] {
            assert!(json.get(k).is_some(), "{k}");
        }
        assert_eq!(clamp_tail("é1\né2\n", 4), "é2\n");
    }

    #[test]
    fn send_posts_json_with_or_without_the_token() {
        use std::io::{Read, Write};
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let site = format!("http://{}", l.local_addr().unwrap());
        let h = std::thread::spawn(move || {
            let mut seen = vec![];
            for code in [201, 429] {
                let (mut c, _) = l.accept().unwrap();
                let mut buf = vec![0u8; 8192];
                let mut got = Vec::new();
                loop {
                    let n = c.read(&mut buf).unwrap();
                    got.extend_from_slice(&buf[..n]);
                    let t = String::from_utf8_lossy(&got).to_string();
                    if let Some(h) = t.find("\r\n\r\n") {
                        let len = t[..h].lines().find_map(|l| l.to_ascii_lowercase().strip_prefix("content-length:").map(|v| v.trim().parse::<usize>().unwrap())).unwrap_or(0);
                        if got.len() >= h + 4 + len { break }
                    }
                }
                seen.push(String::from_utf8_lossy(&got).to_string());
                let _ = write!(c, "HTTP/1.1 {code} X\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{{}}");
            }
            seen
        });
        let r = LogReport::new(code::ADDON_CHECK, "couldn't reach", "0.1.3", None, "line\n".into());
        assert_eq!(send(&site, Some("upl_x"), &r).unwrap(), SendOutcome::Stored);
        assert_eq!(send(&site, None, &r).unwrap(), SendOutcome::Refused(429));
        let seen = h.join().unwrap();
        assert!(seen[0].starts_with("POST /api/uploader/logs "));
        assert!(seen[0].to_ascii_lowercase().contains("authorization: bearer upl_x"));
        assert!(!seen[1].to_ascii_lowercase().contains("authorization"));
        assert!(seen[1].contains("\"errorCode\":\"addon_check\""));
    }
}
