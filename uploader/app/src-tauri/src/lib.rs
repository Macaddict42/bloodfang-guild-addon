mod autostart;
mod baked;
mod config;

use guild_uploader_core as core;
use notify_debouncer_mini::{new_debouncer, DebouncedEventKind};
use serde::Serialize;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tauri::{
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Emitter, Manager, State,
};

struct AppState {
    prefs: Mutex<config::Prefs>,
    origin: Mutex<config::PrefsOrigin>,
    watching: Mutex<bool>,
    connect_cancel: Mutex<Option<Arc<AtomicBool>>>,
    autostart_lock: Mutex<()>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Boot {
    prefs: config::Prefs,
    origin: &'static str,
    site_configured: bool,
    site_host: String,
    prefs_readable: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct TokenStatus {
    has_token: bool,
    state: &'static str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct FolderView {
    resolved: Option<String>,
    source: &'static str,
    show_folder_ui: bool,
    choices: Vec<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ConnectOutcome {
    has_token: bool,
}

fn defaults_addon() -> String {
    option_env!("ADDON_NAME").unwrap_or("").to_string()
}

fn defaults_repo() -> String {
    option_env!("GITHUB_REPO").unwrap_or("").to_string()
}

fn origin_name(origin: config::PrefsOrigin) -> &'static str {
    match origin {
        config::PrefsOrigin::FirstInstall => "firstInstall",
        config::PrefsOrigin::ExistingFile => "existingFile",
        config::PrefsOrigin::Unreadable => "unreadable",
    }
}

fn folder_view(probe: core::FolderProbe) -> FolderView {
    let source = match probe.source {
        core::FolderSource::Saved => "saved",
        core::FolderSource::CanonicalForever => "canonicalForever",
        core::FolderSource::PlatformProbe => "platformProbe",
        core::FolderSource::None => "none",
    };
    FolderView {
        resolved: probe.resolved.map(|p| p.display().to_string()),
        source,
        show_folder_ui: probe.show_folder_ui,
        choices: probe.choices.into_iter().map(|p| p.display().to_string()).collect(),
    }
}

fn schedule_autostart(app: AppHandle) {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let _guard = match state.autostart_lock.lock() {
            Ok(g) => g,
            Err(_) => return,
        };
        let enabled = state.prefs.lock().map(|p| p.start_at_login).unwrap_or(false);
        if let Err(err) = autostart::apply(&app, enabled) {
            let _ = app.emit("autostart-failed", err);
        }
    });
}

fn writable(state: &AppState) -> Result<(), String> {
    let origin = state.origin.lock().map_err(|e| e.to_string())?;
    if *origin == config::PrefsOrigin::Unreadable {
        Err("Saved settings couldn't be read, so they were left unchanged.".into())
    } else {
        Ok(())
    }
}

#[tauri::command]
async fn get_boot(app: AppHandle, state: State<'_, AppState>) -> Result<Boot, String> {
    let origin = *state.origin.lock().map_err(|e| e.to_string())?;
    if origin == config::PrefsOrigin::FirstInstall {
        let mut prefs = state.prefs.lock().map_err(|e| e.to_string())?.clone();
        prefs.start_at_login = true;
        if prefs.addon_name.is_empty() {
            prefs.addon_name = defaults_addon();
        }
        if let Ok(site) = baked::site_url() {
            prefs.site_url = site;
        }
        config::save_prefs(&prefs)?;
        *state.prefs.lock().map_err(|e| e.to_string())? = prefs;
        *state.origin.lock().map_err(|e| e.to_string())? = config::PrefsOrigin::ExistingFile;
    }
    if origin != config::PrefsOrigin::Unreadable {
        schedule_autostart(app);
    }
    let prefs = state.prefs.lock().map_err(|e| e.to_string())?.clone();
    let origin_now = *state.origin.lock().map_err(|e| e.to_string())?;
    Ok(Boot {
        prefs,
        origin: origin_name(origin_now),
        site_configured: baked::site_url().is_ok(),
        site_host: baked::site_host(),
        prefs_readable: origin_now != config::PrefsOrigin::Unreadable,
    })
}

#[tauri::command]
async fn token_status() -> Result<TokenStatus, String> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    tauri::async_runtime::spawn_blocking(move || {
        let _ = tx.send(config::get_token());
    });
    let status = match tokio::time::timeout(Duration::from_secs(2), rx).await {
        Ok(Ok(Ok(Some(_)))) => TokenStatus { has_token: true, state: "present" },
        Ok(Ok(Ok(None))) => TokenStatus { has_token: false, state: "absent" },
        _ => TokenStatus { has_token: false, state: "unavailable" },
    };
    Ok(status)
}

#[tauri::command]
async fn probe_game_folder(state: State<'_, AppState>) -> Result<FolderView, String> {
    let saved = {
        let prefs = state.prefs.lock().map_err(|e| e.to_string())?;
        if prefs.game_folder.is_empty() { None } else { Some(prefs.game_folder.clone()) }
    };
    let work = tauri::async_runtime::spawn_blocking(move || {
        let path = saved.map(PathBuf::from);
        core::probe_game_folder(path.as_deref())
    });
    match tokio::time::timeout(Duration::from_millis(1500), work).await {
        Ok(Ok(probe)) => Ok(folder_view(probe)),
        _ => Ok(FolderView { resolved: None, source: "none", show_folder_ui: true, choices: vec![] }),
    }
}

#[tauri::command]
async fn connect_browser(state: State<'_, AppState>, switch_account: Option<bool>) -> Result<ConnectOutcome, String> {
    let site = baked::site_url()?;
    let prompt_login = switch_account.unwrap_or(false);
    let cancel = Arc::new(AtomicBool::new(false));
    {
        let mut slot = state.connect_cancel.lock().map_err(|e| e.to_string())?;
        if slot.is_some() {
            return Err("Sign-in is already in progress.".into());
        }
        *slot = Some(cancel.clone());
    }
    let cancel_for_call = cancel.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        core::browser_auth::connect_loopback(&site, Duration::from_secs(5 * 60), &cancel_for_call, prompt_login, |url| {
            opener::open(url).is_ok()
        })
    })
    .await
    .map_err(|e| e.to_string());
    if let Ok(mut slot) = state.connect_cancel.lock() {
        *slot = None;
    }
    match result {
        Ok(Ok(token)) => {
            config::set_token(&token)?;
            Ok(ConnectOutcome { has_token: true })
        }
        Ok(Err(err)) => Err(err.to_string()),
        Err(err) => Err(err),
    }
}

#[tauri::command]
async fn cancel_connect(state: State<'_, AppState>) -> Result<(), String> {
    let slot = state.connect_cancel.lock().map_err(|e| e.to_string())?;
    if let Some(flag) = slot.as_ref() {
        flag.store(true, Ordering::SeqCst);
    }
    Ok(())
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct SetupInput {
    game_folder: String,
    start_at_login: bool,
}

fn remember_folder(state: &AppState, folder: &str, only_if_empty: bool, start_at_login: Option<bool>) -> Result<(), String> {
    writable(state)?;
    let path = PathBuf::from(folder.trim());
    if !core::folder_acceptable(&path) {
        return Err("That folder doesn't look like a game client (need WTF, Interface/AddOns, or Wow.exe).".into());
    }
    let mut prefs = state.prefs.lock().map_err(|e| e.to_string())?;
    if only_if_empty && !prefs.game_folder.is_empty() {
        return Ok(());
    }
    prefs.game_folder = path.display().to_string();
    prefs.setup_done = true;
    if prefs.addon_name.is_empty() {
        prefs.addon_name = defaults_addon();
    }
    if let Ok(site) = baked::site_url() {
        prefs.site_url = site;
    }
    if let Some(enabled) = start_at_login {
        prefs.start_at_login = enabled;
    }
    config::save_prefs(&prefs)
}

#[tauri::command]
async fn accept_detected_folder(app: AppHandle, state: State<'_, AppState>, folder: String) -> Result<(), String> {
    remember_folder(&*state, &folder, true, None)?;
    schedule_autostart(app);
    Ok(())
}

#[tauri::command]
async fn save_setup(app: AppHandle, state: State<'_, AppState>, input: SetupInput) -> Result<(), String> {
    remember_folder(&*state, &input.game_folder, false, Some(input.start_at_login))?;
    schedule_autostart(app);
    Ok(())
}

#[tauri::command]
async fn set_start_at_login(app: AppHandle, state: State<'_, AppState>, enabled: bool) -> Result<(), String> {
    writable(&*state)?;
    {
        let mut prefs = state.prefs.lock().map_err(|e| e.to_string())?;
        prefs.start_at_login = enabled;
        if let Ok(site) = baked::site_url() {
            prefs.site_url = site;
        }
        config::save_prefs(&prefs)?;
    }
    schedule_autostart(app);
    Ok(())
}

#[tauri::command]
async fn clear_token() -> Result<TokenStatus, String> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    tauri::async_runtime::spawn_blocking(move || {
        let _ = tx.send(config::set_token(""));
    });
    match tokio::time::timeout(Duration::from_secs(2), rx).await {
        Ok(Ok(Ok(()))) => Ok(TokenStatus { has_token: false, state: "absent" }),
        Ok(Ok(Err(err))) => Err(err),
        _ => Err("Couldn't update the saved sign-in.".into()),
    }
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

fn do_upload(prefs: &config::Prefs) -> Result<UploadResult, core::UploadError> {
    let token = match config::get_token() {
        Ok(Some(token)) => token,
        Ok(None) => return Err(core::UploadError::BadToken),
        Err(err) => return Err(core::UploadError::Network(err)),
    };
    let site = baked::site_url().map_err(|err| core::UploadError::Network(err))?;
    let game = PathBuf::from(&prefs.game_folder);
    let files = core::saved_variables(&game, &prefs.addon_name);
    if files.is_empty() {
        return Err(core::UploadError::Network(format!(
            "No SavedVariables for {} yet. Log in with the addon, then /reload.",
            prefs.addon_name
        )));
    }
    let state_path = config::state_path(prefs);
    let report = core::run(&files, &site, &token, &state_path)?;
    let lines: Vec<String> = report
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
    let at = unix_now();
    let accepted = report.results.iter().filter(|(_, outcome)| core::done(outcome)).count();
    let status = if report.sent == 0 { "nothing new".into() } else { format!("sent {accepted}") };
    Ok(UploadResult { found: report.found, sent: accepted, lines, at, status })
}

fn unix_now() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    format!("{secs}")
}

#[tauri::command]
async fn upload_now(state: State<'_, AppState>) -> Result<UploadResult, String> {
    let prefs = state.prefs.lock().map_err(|e| e.to_string())?.clone();
    let result = tauri::async_runtime::spawn_blocking(move || do_upload(&prefs)).await.map_err(|e| e.to_string())?;
    match result {
        Ok(upload) => {
            let mut prefs = state.prefs.lock().map_err(|e| e.to_string())?;
            prefs.last_upload_at = Some(upload.at.clone());
            prefs.last_status = Some(upload.status.clone());
            prefs.last_sent_lines = Some(upload.lines.clone());
            config::save_prefs(&prefs)?;
            Ok(upload)
        }
        Err(err) if core::drops_saved_token(&err) => {
            let _ = config::set_token("");
            Err(err.to_string())
        }
        Err(err) => Err(err.to_string()),
    }
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

fn semver_gt(a: &str, b: &str) -> bool {
    match (semver::Version::parse(a), semver::Version::parse(b)) {
        (Ok(a), Ok(b)) => a > b,
        _ => a != b,
    }
}

#[tauri::command]
async fn check_addon(state: State<'_, AppState>) -> Result<AddonCheck, String> {
    let prefs = state.prefs.lock().map_err(|e| e.to_string())?.clone();
    let repo = defaults_repo();
    tauri::async_runtime::spawn_blocking(move || {
        let installed = if prefs.game_folder.is_empty() {
            None
        } else {
            core::installed_version(&core::addons_dir(PathBuf::from(&prefs.game_folder).as_path()), &prefs.addon_name).map(|v| v.to_string())
        };
        let latest = core::latest_addon(&repo, &prefs.addon_name).map_err(|e| e.to_string())?;
        let (tag, ver) = match &latest {
            Some(release) => (Some(release.tag.clone()), Some(release.version.to_string())),
            None => (None, None),
        };
        let mut update_available = match (&installed, &latest) {
            (Some(installed), Some(release)) => semver_gt(&release.version.to_string(), installed),
            (None, Some(_)) => true,
            _ => false,
        };
        if update_available {
            if let (Some(skip), Some(ver)) = (&prefs.skipped_addon_version, &ver) {
                if skip == ver {
                    update_available = false;
                }
            }
        }
        Ok(AddonCheck { installed, latest: ver, tag, update_available, game_running: false })
    })
    .await
    .map_err(|e| e.to_string())?
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct GameRunning {
    running: bool,
}

#[tauri::command]
async fn game_running() -> Result<GameRunning, String> {
    let running = match tokio::time::timeout(
        Duration::from_secs(3),
        tauri::async_runtime::spawn_blocking(core::game_seems_running),
    )
    .await
    {
        Ok(Ok(running)) => running,
        _ => false,
    };
    Ok(GameRunning { running })
}

#[tauri::command]
async fn update_addon(state: State<'_, AppState>) -> Result<String, String> {
    let prefs = state.prefs.lock().map_err(|e| e.to_string())?.clone();
    let repo = defaults_repo();
    let version = tauri::async_runtime::spawn_blocking(move || {
        let release = core::latest_addon(&repo, &prefs.addon_name).map_err(|e| e.to_string())?.ok_or("no addon release found on GitHub")?;
        core::install_addon(&release, PathBuf::from(&prefs.game_folder).as_path(), &prefs.addon_name, Some(&repo)).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())??;
    let mut prefs = state.prefs.lock().map_err(|e| e.to_string())?;
    prefs.last_addon_version = Some(version.clone());
    prefs.skipped_addon_version = None;
    config::save_prefs(&prefs)?;
    Ok(version)
}

#[tauri::command]
async fn skip_addon_version(state: State<'_, AppState>, version: String) -> Result<(), String> {
    let mut prefs = state.prefs.lock().map_err(|e| e.to_string())?;
    let version = version.trim();
    if version.is_empty() {
        return Err("No version to skip.".into());
    }
    prefs.skipped_addon_version = Some(version.to_string());
    config::save_prefs(&prefs)
}

#[tauri::command]
async fn start_watch(app: AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    {
        let mut flag = state.watching.lock().map_err(|e| e.to_string())?;
        if *flag {
            return Ok(());
        }
        *flag = true;
    }
    let prefs = state.prefs.lock().map_err(|e| e.to_string())?.clone();
    let listed = tauri::async_runtime::spawn_blocking(move || {
        core::saved_variables(&PathBuf::from(&prefs.game_folder), &prefs.addon_name)
    })
    .await;
    let files = match listed {
        Ok(files) => files,
        Err(err) => {
            if let Ok(mut flag) = state.watching.lock() {
                *flag = false;
            }
            return Err(err.to_string());
        }
    };
    if files.is_empty() {
        let mut flag = state.watching.lock().map_err(|e| e.to_string())?;
        *flag = false;
        return Ok(());
    }
    std::thread::spawn(move || {
        let (tx, rx) = std::sync::mpsc::channel();
        let mut debouncer = new_debouncer(Duration::from_millis(1500), tx).ok();
        if let Some(debouncer) = debouncer.as_mut() {
            for file in &files {
                let _ = debouncer.watcher().watch(file, notify::RecursiveMode::NonRecursive);
                if let Some(parent) = file.parent() {
                    let _ = debouncer.watcher().watch(parent, notify::RecursiveMode::NonRecursive);
                }
            }
        }
        for res in rx {
            if let Ok(events) = res {
                if events.iter().any(|event| event.kind == DebouncedEventKind::Any) {
                    let _ = app.emit("sv-changed", ());
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
async fn check_self_update() -> Result<SelfCheck, String> {
    let current = env!("CARGO_PKG_VERSION").to_string();
    let repo = defaults_repo();
    if repo.is_empty() {
        return Ok(SelfCheck { current, latest: None, update_available: false });
    }
    let current_for_check = current.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let latest = core::latest_app(&repo).map_err(|e| e.to_string())?;
        let ver = latest.as_ref().map(|release| release.version.to_string());
        let update_available = match (&ver, semver::Version::parse(&current_for_check)) {
            (Some(latest), Ok(current)) => semver::Version::parse(latest).map(|lv| lv > current).unwrap_or(false),
            _ => false,
        };
        Ok(SelfCheck { current: current_for_check, latest: ver, update_available })
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn pick_folder(app: AppHandle) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.dialog().file().set_title("Choose the game folder").pick_folder(move |picked| {
        let _ = tx.send(picked.and_then(|path| path.into_path().ok()).map(|path| path.display().to_string()));
    });
    rx.await.map_err(|e| e.to_string())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let loaded = config::load_prefs();
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            Some(vec!["--hidden".into()]),
        ))
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_single_instance::init(|app, _, _| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.show();
                let _ = window.set_focus();
            }
        }))
        .manage(AppState {
            prefs: Mutex::new(loaded.prefs),
            origin: Mutex::new(loaded.origin),
            watching: Mutex::new(false),
            connect_cancel: Mutex::new(None),
            autostart_lock: Mutex::new(()),
        })
        .invoke_handler(tauri::generate_handler![
            get_boot,
            token_status,
            probe_game_folder,
            connect_browser,
            cancel_connect,
            accept_detected_folder,
            save_setup,
            set_start_at_login,
            clear_token,
            upload_now,
            check_addon,
            game_running,
            update_addon,
            skip_addon_version,
            start_watch,
            check_self_update,
            pick_folder
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
                    "show" => {
                        if let Some(window) = app.get_webview_window("main") {
                            let _ = window.show();
                            let _ = window.set_focus();
                        }
                    }
                    "upload" => {
                        let _ = app.emit("tray-upload", ());
                    }
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = event {
                        let app = tray.app_handle();
                        if let Some(window) = app.get_webview_window("main") {
                            let _ = window.show();
                            let _ = window.set_focus();
                        }
                    }
                })
                .icon(app.default_window_icon().unwrap().clone())
                .build(app)?;
            if autostart::launched_hidden() {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.hide();
                }
            }
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running Guild Uploader");
}
