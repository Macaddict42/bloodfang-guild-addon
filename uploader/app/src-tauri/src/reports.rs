//! Automatic error reports. Every reported error is logged first; then, at most once an hour per
//! error code (remembered across restarts), the redacted recent log goes to the site in the
//! background. Signed in, the report carries the uploader token; before sign-in it goes anonymously.
use guild_uploader_core as core;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

static DEBOUNCE_LOCK: Mutex<()> = Mutex::new(());

/// Log `message` as an error under `code` and send a report if this code wasn't sent in the last hour.
pub fn report(code: &'static str, message: &str, addon_version: Option<String>) {
    core::log::error(code, message);
    let message = message.to_string();
    std::thread::spawn(move || send(code, &message, addon_version));
}

fn send(code: &'static str, message: &str, addon_version: Option<String>) {
    let Some(dir) = core::log::dir() else { return };
    let Ok(site) = crate::baked::site_url() else {
        core::log::warn("report", "this build has no site address; error report not sent");
        return;
    };
    {
        let _g = DEBOUNCE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let path = dir.join("report-debounce.json");
        let now = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        let mut d = core::report::Debounce::load(&path);
        if !d.allow(code, now) {
            core::log::info("report", &format!("{code} was reported within the last hour; not sending again"));
            return;
        }
        let _ = d.save(&path);
    }
    let token = crate::config::get_token().ok().flatten();
    let report = core::report::LogReport::new(code, message, env!("CARGO_PKG_VERSION"), addon_version, core::log::tail(core::report::MAX_LOG_BYTES));
    let who = if token.is_some() { "signed in" } else { "anonymous" };
    match core::report::send(&site, token.as_deref(), &report) {
        Ok(core::report::SendOutcome::Stored) => core::log::info("report", &format!("sent {code} report ({who}, {} log bytes)", report.log.len())),
        Ok(core::report::SendOutcome::Refused(status)) => core::log::warn("report", &format!("site refused the {code} report ({who}): HTTP {status}")),
        Err(e) => core::log::warn("report", &format!("couldn't send the {code} report ({who}): {e}")),
    }
}
