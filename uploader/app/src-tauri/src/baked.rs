//! Site address baked at compile time from `SITE_URL`. No host is written in this file.

pub fn site_url() -> Result<String, String> {
    let raw = option_env!("SITE_URL").unwrap_or("");
    guild_uploader_core::browser_auth::parse_site_url(raw).map_err(|_| {
        "This build has no site address. Connect and upload stay off until it is rebuilt with one.".into()
    })
}

pub fn site_host() -> String {
    site_url().map(|url| guild_uploader_core::browser_auth::site_host(&url)).unwrap_or_default()
}
