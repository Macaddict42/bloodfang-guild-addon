mod config;

use guild_uploader_core as core;
use notify_debouncer_mini::{new_debouncer, DebouncedEventKind};
use serde::Serialize;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;
use tauri::{Emitter, 
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Manager, State,
};

struct AppState {
    prefs: Mutex<config::Prefs>,
    watching: Mutex<bool>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Snapshot {
    prefs: config::Prefs,
    has_token: bool,
    candidates: Vec<String>,
    addon_installed: Option<String>,
    game_running: bool,
    defaults: Defaults,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Defaults {
    site_url: String,
    addon_name: String,
    github_repo: String,
}

fn defaults() -> Defaults {
    Defaults {
        site_url: option_env!("SITE_URL").unwrap_or("").to_string(),
        addon_name: option_env!("ADDON_NAME").unwrap_or("").to_string(),
        github_repo: option_env!("GITHUB_REPO").unwrap_or("").to_string(),
    }
}

#[tauri::command]
fn get_snapshot(state: State<AppState>) -> Result<Snapshot, String> {
    let prefs = state.prefs.lock().map_err(|e| e.to_string())?.clone();
    let has_token = config::get_token()?.is_some();
    let candidates = core::candidates().into_iter().map(|p| p.display().to_string()).collect();
    let addon_installed = if prefs.game_folder.is_empty() {
        None
    } else {
        core::installed_version(&core::addons_dir(PathBuf::from(&prefs.game_folder).as_path()), &prefs.addon_name)
            .map(|v| v.to_string())
    };
    Ok(Snapshot {
        prefs,
        has_token,
        candidates,
        addon_installed,
        game_running: core::game_seems_running(),
        defaults: defaults(),
    })
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct SetupInput {
    site_url: String,
    token: String,
    game_folder: String,
    start_at_login: bool,
}

#[tauri::command]
fn save_setup(state: State<AppState>, input: SetupInput, app: AppHandle) -> Result<Snapshot, String> {
    if input.token.is_empty() && config::get_token()?.is_none() {
        return Err("paste an uploader token from the site (Settings → Uploader tokens)".into());
    }
    let game = PathBuf::from(input.game_folder.trim());
    if !core::looks_like_game_dir(&game) {
        return Err("that folder doesn't look like a game client (need WTF or Interface/AddOns)".into());
    }
    if !input.token.is_empty() {
        if !input.token.starts_with("upl_") { return Err("token should start with upl_".into()); }
        config::set_token(input.token.trim())?;
    }
    let mut prefs = state.prefs.lock().map_err(|e| e.to_string())?;
    prefs.site_url = input.site_url.trim().trim_end_matches('/').to_string();
    prefs.game_folder = game.display().to_string();
    if prefs.addon_name.is_empty() { prefs.addon_name = defaults().addon_name; }
    prefs.start_at_login = input.start_at_login;
    prefs.setup_done = true;
    config::save_prefs(&prefs)?;
    let enable = prefs.start_at_login;
    drop(prefs);
    use tauri_plugin_autostart::ManagerExt;
    let auto = app.autolaunch();
    let _ = if enable { auto.enable() } else { auto.disable() };
    let _ = app.emit("prefs-changed", ());
    get_snapshot(state)
}

#[tauri::command]
fn clear_token(state: State<AppState>) -> Result<Snapshot, String> {
    config::set_token("")?;
    get_snapshot(state)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct UploadResult {
    found: usize,
    sent: usize,
    lines: Vec<String>,
    at: String,
    status: String,
}

fn do_upload(prefs: &config::Prefs) -> Result<UploadResult, String> {
    let token = config::get_token()?.ok_or("no uploader token stored — open Setup")?;
    let game = PathBuf::from(&prefs.game_folder);
    let files = core::saved_variables(&game, &prefs.addon_name);
    if files.is_empty() {
        return Err(format!("no SavedVariables for {} yet — log in with the addon, then /reload", prefs.addon_name));
    }
    let state_path = config::state_path(prefs);
    let report = core::run(&files, &prefs.site_url, &token, &state_path).map_err(|e| e.to_string())?;
    let lines: Vec<String> = report.results.iter().map(|(_, o)| {
        format!("{} {}", o.ty.as_deref().unwrap_or("?"), if o.ok { "accepted".into() } else { o.error.clone().unwrap_or_default() })
    }).collect();
    let at = chrono_lite_now();
    let status = if report.sent == 0 { "nothing new".into() } else { format!("sent {}", report.sent) };
    Ok(UploadResult { found: report.found, sent: report.sent, lines, at, status })
}

fn chrono_lite_now() -> String {
    // RFC3339-ish UTC without extra crate
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    format!("{secs}")
}

#[tauri::command]
fn upload_now(state: State<AppState>) -> Result<UploadResult, String> {
    let mut prefs = state.prefs.lock().map_err(|e| e.to_string())?;
    let r = do_upload(&prefs)?;
    prefs.last_upload_at = Some(r.at.clone());
    prefs.last_status = Some(r.status.clone());
    config::save_prefs(&prefs)?;
    Ok(r)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AddonCheck {
    installed: Option<String>,
    latest: Option<String>,
    tag: Option<String>,
    update_available: bool,
    game_running: bool,
}

#[tauri::command]
fn check_addon(state: State<AppState>) -> Result<AddonCheck, String> {
    let prefs = state.prefs.lock().map_err(|e| e.to_string())?.clone();
    let d = defaults();
    let installed = if prefs.game_folder.is_empty() { None } else {
        core::installed_version(&core::addons_dir(PathBuf::from(&prefs.game_folder).as_path()), &prefs.addon_name).map(|v| v.to_string())
    };
    let latest = core::latest_addon(&d.github_repo, &prefs.addon_name).map_err(|e| e.to_string())?;
    let (tag, ver) = match &latest {
        Some(r) => (Some(r.tag.clone()), Some(r.version.to_string())),
        None => (None, None),
    };
    let update_available = match (&installed, &latest) {
        (Some(i), Some(r)) => semver_gt(&r.version.to_string(), i),
        (None, Some(_)) => true,
        _ => false,
    };
    Ok(AddonCheck { installed, latest: ver, tag, update_available, game_running: core::game_seems_running() })
}

fn semver_gt(a: &str, b: &str) -> bool {
    match (semver::Version::parse(a), semver::Version::parse(b)) {
        (Ok(a), Ok(b)) => a > b,
        _ => a != b,
    }
}

#[tauri::command]
fn update_addon(state: State<AppState>) -> Result<String, String> {
    let mut prefs = state.prefs.lock().map_err(|e| e.to_string())?;
    let d = defaults();
    let rel = core::latest_addon(&d.github_repo, &prefs.addon_name).map_err(|e| e.to_string())?
        .ok_or("no addon release found on GitHub")?;
    let ver = core::install_addon(&rel, PathBuf::from(&prefs.game_folder).as_path(), &prefs.addon_name, Some(&d.github_repo))
        .map_err(|e| e.to_string())?;
    prefs.last_addon_version = Some(ver.clone());
    config::save_prefs(&prefs)?;
    Ok(ver)
}

#[tauri::command]
fn start_watch(app: AppHandle, state: State<AppState>) -> Result<(), String> {
    let mut flag = state.watching.lock().map_err(|e| e.to_string())?;
    if *flag { return Ok(()); }
    *flag = true;
    let prefs = state.prefs.lock().map_err(|e| e.to_string())?.clone();
    let game = PathBuf::from(&prefs.game_folder);
    let addon = prefs.addon_name.clone();
    let files = core::saved_variables(&game, &addon);
    if files.is_empty() { return Ok(()); }
    let app2 = app.clone();
    std::thread::spawn(move || {
        let (tx, rx) = std::sync::mpsc::channel();
        let mut debouncer = new_debouncer(Duration::from_millis(1500), tx).ok();
        if let Some(d) = debouncer.as_mut() {
            for f in &files {
                let _ = d.watcher().watch(f, notify::RecursiveMode::NonRecursive);
                if let Some(parent) = f.parent() {
                    let _ = d.watcher().watch(parent, notify::RecursiveMode::NonRecursive);
                }
            }
        }
        for res in rx {
            if let Ok(events) = res {
                if events.iter().any(|e| e.kind == DebouncedEventKind::Any) {
                    let _ = app2.emit("sv-changed", ());
                }
            }
        }
    });
    Ok(())
}


#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SelfCheck {
    current: String,
    latest: Option<String>,
    update_available: bool,
}

#[tauri::command]
fn check_self_update() -> Result<SelfCheck, String> {
    let current = env!("CARGO_PKG_VERSION").to_string();
    let d = defaults();
    if d.github_repo.is_empty() {
        return Ok(SelfCheck { current, latest: None, update_available: false });
    }
    // Reuse addon release fetch pattern for uploader-app-v* tags via ureq through core helper
    let latest = core::latest_app(&d.github_repo).map_err(|e| e.to_string())?;
    let ver = latest.as_ref().map(|r| r.version.to_string());
    let update_available = match (&ver, semver::Version::parse(&current)) {
        (Some(l), Ok(c)) => semver::Version::parse(l).map(|lv| lv > c).unwrap_or(false),
        _ => false,
    };
    Ok(SelfCheck { current, latest: ver, update_available })
}


#[tauri::command]
async fn pick_folder(app: AppHandle) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;
    let (tx, rx) = std::sync::mpsc::channel();
    app.dialog().file().set_title("Choose the game folder").pick_folder(move |f| {
        let _ = tx.send(f.and_then(|p| p.into_path().ok()).map(|path| path.display().to_string()));
    });
    Ok(rx.recv().map_err(|e| e.to_string())?)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let prefs = config::load_prefs();
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_autostart::init(tauri_plugin_autostart::MacosLauncher::LaunchAgent, Some(vec!["--hidden".into()])))
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_single_instance::init(|app, _, _| {
            if let Some(w) = app.get_webview_window("main") { let _ = w.set_focus(); }
        }))
        .manage(AppState { prefs: Mutex::new(prefs), watching: Mutex::new(false) })
        .invoke_handler(tauri::generate_handler![
            get_snapshot, save_setup, clear_token, upload_now, check_addon, update_addon, start_watch, check_self_update, pick_folder
        ])
        .setup(|app| {
            let show = MenuItem::with_id(app, "show", "Open", true, None::<&str>)?;
            let upload = MenuItem::with_id(app, "upload", "Upload now", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&show, &upload, &quit])?;
            let _tray = TrayIconBuilder::new()
                .menu(&menu)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "quit" => app.exit(0),
                    "show" => { if let Some(w) = app.get_webview_window("main") { let _ = w.show(); let _ = w.set_focus(); } }
                    "upload" => { let _ = app.emit("tray-upload", ()); }
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = event {
                        let app = tray.app_handle();
                        if let Some(w) = app.get_webview_window("main") { let _ = w.show(); let _ = w.set_focus(); }
                    }
                })
                .icon(app.default_window_icon().unwrap().clone())
                .build(app)?;
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running Guild Uploader");
}
