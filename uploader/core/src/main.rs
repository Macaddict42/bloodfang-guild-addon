//! Command-line uploader, for testing and for members who'd rather not run the app.
//! Settings come from the environment only: UPLOADER_SITE (site base URL), UPLOADER_TOKEN, UPLOADER_ADDON
//! (addon folder name), and the game folder as the first argument.
use guild_uploader_core::*;
use std::path::PathBuf;
use std::process::exit;

fn env(k: &str) -> String { std::env::var(k).unwrap_or_else(|_| { eprintln!("Set {k}."); exit(2) }) }

fn main() {
    let Some(game) = std::env::args().nth(1).map(PathBuf::from) else {
        eprintln!("Usage: guild-uploader <game folder, e.g. \".../World of Warcraft/_classic_era_\">");
        exit(2)
    };
    let (site, token, addon) = (env("UPLOADER_SITE"), env("UPLOADER_TOKEN"), env("UPLOADER_ADDON"));
    let files = saved_variables(&game, &addon);
    if files.is_empty() { eprintln!("No SavedVariables for {addon} under {}. Log in once with the addon, then /reload.", game.display()); exit(1) }
    let state = std::env::var("UPLOADER_STATE").map(PathBuf::from).unwrap_or_else(|_| game.join("WTF").join(format!("{addon}-uploader-state.json")));
    match run(&files, &site, &token, &state) {
        Ok(r) => {
            println!("Found {} strings, sent {}.", r.found, r.sent);
            for (s, o) in &r.results {
                println!("  {} {}", o.ty.as_deref().unwrap_or("?"), if o.ok { "accepted".into() } else { o.error.clone().unwrap_or_default() });
                let _ = s;
            }
        }
        Err(e) => { eprintln!("Upload stopped: {e}"); exit(1) }
    }
}
