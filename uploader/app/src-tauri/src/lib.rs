mod addon_auto;
mod app_update;
mod autostart;
mod baked;
mod config;
mod install_hook;
mod tray_life;
mod upload_gate;

use guild_uploader_core as core;

/// SHA-256 of the raw public key compiled into this build, when one was configured.
pub fn compiled_update_key_sha256() -> Option<String> {
    app_update::trust_from_build().public_key.as_ref().map(|key| core::sha256_hex(key))
}

pub struct InstallerCheck {
    pub installed: bool,
    pub restarted: bool,
    pub message: String,
}

/// Production Update now path. The install hook refuses to start a process.
pub fn check_installer_bytes(version: &str, installer: &[u8], sha256_hex: &str, signature: &[u8]) -> InstallerCheck {
    let result = app_update::decide(
        app_update::Choice::UpdateNow,
        &app_update::trust_from_build(),
        || {
            Ok(app_update::Fetched {
                version: version.to_string(),
                installer: installer.to_vec(),
                sha256_hex: sha256_hex.to_string(),
                signature: signature.to_vec(),
            })
        },
        |_| Err("installation is not authorized".into()),
    );
    InstallerCheck { installed: result.installed, restarted: result.restarted, message: result.message }
}
use notify_debouncer_mini::new_debouncer;
use serde::Serialize;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tauri::{
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Emitter, Manager, State, WindowEvent,
};

struct AppState {
    prefs: Mutex<config::Prefs>,
    origin: Mutex<config::PrefsOrigin>,
    watching: Mutex<bool>,
    connect_cancel: Mutex<Option<Arc<AtomicBool>>>,
    autostart_lock: Mutex<()>,
    upload_seq: upload_gate::UploadSeq,
    upload_flight: upload_gate::Flight,
    addon_started: AtomicBool,
    addon_stop: Arc<AtomicBool>,
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

fn do_upload(prefs: &config::Prefs) -> Result<upload_gate::SavedUpload, core::UploadError> {
    let token = match config::get_token() {
        Ok(Some(token)) => token,
        Ok(None) => return Err(core::UploadError::BadToken),
        Err(err) => return Err(core::UploadError::Network(err)),
    };
    let site = baked::site_url().map_err(|err| core::UploadError::Network(err))?;
    upload_gate::upload_saved_variables(&prefs.game_folder, &prefs.addon_name, &site, &token, &config::state_path(prefs))
}

fn upload_result(upload: &upload_gate::SavedUpload) -> UploadResult {
    UploadResult {
        found: upload.found,
        sent: upload.sent,
        lines: upload.lines.clone(),
        at: upload.at.clone(),
        status: upload.status.clone(),
    }
}

fn stored_result(prefs: &config::Prefs) -> UploadResult {
    UploadResult {
        found: 0,
        sent: 0,
        lines: prefs.last_sent_lines.clone().unwrap_or_default(),
        at: prefs.last_upload_at.clone().unwrap_or_default(),
        status: prefs.last_status.clone().unwrap_or_else(|| "nothing new".into()),
    }
}

fn notice_ok(upload: &upload_gate::SavedUpload) -> WatchNotice {
    WatchNotice {
        ok: true,
        found: upload.found,
        sent: upload.sent,
        lines: upload.lines.clone(),
        at: upload.at.clone(),
        status: upload.status.clone(),
        error: String::new(),
    }
}

fn notice_err(message: String) -> WatchNotice {
    WatchNotice { ok: false, found: 0, sent: 0, lines: Vec::new(), at: String::new(), status: String::new(), error: message }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WatchNotice {
    ok: bool,
    found: usize,
    sent: usize,
    lines: Vec<String>,
    at: String,
    status: String,
    error: String,
}

/// Upload from the watcher thread or the tray menu. Neither path asks whether
/// the window is visible. Hiding the window does not stop this thread.
fn perform_upload(app: &AppHandle) -> Result<(), String> {
    let state = app.state::<AppState>();
    if !state.upload_flight.acquire() {
        return Ok(());
    }
    loop {
        let outcome = upload_once(app);
        if !state.upload_flight.take_repeat() {
            return outcome;
        }
    }
}

fn upload_once(app: &AppHandle) -> Result<(), String> {
    let state = app.state::<AppState>();
    let (prefs, seq) = {
        let prefs = state.prefs.lock().map_err(|e| e.to_string())?.clone();
        (prefs, state.upload_seq.note_start())
    };
    let mut attempt = 0;
    let result = loop {
        match do_upload(&prefs) {
            Ok(upload) => break Ok(upload),
            Err(err) if upload_gate::transient_retry(&err, attempt) => {
                attempt += 1;
                std::thread::sleep(upload_gate::retry_pause(attempt));
            }
            Err(err) => break Err(err),
        }
    };
    let step = {
        let mut prefs = state.prefs.lock().map_err(|e| e.to_string())?;
        match upload_gate::finish_upload(&mut prefs, &state.upload_seq, seq, result) {
            upload_gate::Finish::Stale => None,
            upload_gate::Finish::Applied(upload) => {
                config::save_prefs(&prefs)?;
                Some(notice_ok(&upload))
            }
            upload_gate::Finish::Failed { drop_token, message } => {
                if drop_token {
                    let _ = config::set_token("");
                }
                Some(notice_err(message))
            }
        }
    };
    let Some(notice) = step else { return Ok(()) };
    let failed = !notice.ok;
    let message = notice.error.clone();
    let _ = app.emit("sv-changed", &notice);
    if failed { Err(message) } else { Ok(()) }
}

#[tauri::command]
async fn upload_now(state: State<'_, AppState>) -> Result<UploadResult, String> {
    let (prefs, seq) = {
        let prefs = state.prefs.lock().map_err(|e| e.to_string())?.clone();
        (prefs, state.upload_seq.note_start())
    };
    let result = tauri::async_runtime::spawn_blocking(move || do_upload(&prefs)).await.map_err(|e| e.to_string())?;
    let mut prefs = state.prefs.lock().map_err(|e| e.to_string())?;
    match upload_gate::finish_upload(&mut prefs, &state.upload_seq, seq, result) {
        upload_gate::Finish::Applied(upload) => {
            config::save_prefs(&prefs)?;
            Ok(upload_result(&upload))
        }
        upload_gate::Finish::Stale => Ok(stored_result(&prefs)),
        upload_gate::Finish::Failed { drop_token, message } => {
            if drop_token {
                let _ = config::set_token("");
            }
            Err(message)
        }
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
        let game_running = core::game_seems_running();
        Ok(AddonCheck { installed, latest: ver, tag, update_available, game_running })
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
    let listed_folder = prefs.game_folder.clone();
    let listed_addon = prefs.addon_name.clone();
    let listed = tauri::async_runtime::spawn_blocking(move || {
        core::saved_variables(&PathBuf::from(&listed_folder), &listed_addon)
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
    let game = PathBuf::from(&prefs.game_folder);
    let watch_paths = if files.is_empty() { saved_variable_dirs(&game) } else { files.clone() };
    spawn_addon_auto(app.clone());
    if watch_paths.is_empty() {
        let mut flag = state.watching.lock().map_err(|e| e.to_string())?;
        *flag = false;
        return Ok(());
    }
    let backlog = !files.is_empty();
    std::thread::spawn(move || {
        let (tx, rx) = std::sync::mpsc::channel();
        let mut debouncer = new_debouncer(Duration::from_millis(1500), tx).ok();
        if let Some(debouncer) = debouncer.as_mut() {
            for path in &watch_paths {
                let _ = debouncer.watcher().watch(path, notify::RecursiveMode::NonRecursive);
                if path.is_file() {
                    if let Some(parent) = path.parent() {
                        let _ = debouncer.watcher().watch(parent, notify::RecursiveMode::NonRecursive);
                    }
                }
            }
        }
        if backlog {
            let _ = perform_upload(&app);
        }
        for res in rx {
            if let Ok(events) = res {
                if upload_gate::debounced_change_should_upload(events.iter().map(|event| event.kind)) {
                    let _ = perform_upload(&app);
                }
            }
        }
    });
    Ok(())
}

fn saved_variable_dirs(game: &std::path::Path) -> Vec<PathBuf> {
    let accounts = game.join("WTF").join("Account");
    let mut dirs = Vec::new();
    if let Ok(read) = std::fs::read_dir(accounts) {
        for entry in read.flatten() {
            let saved = entry.path().join("SavedVariables");
            if saved.is_dir() {
                dirs.push(saved);
            }
        }
    }
    dirs
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AddonAutoNotice {
    ok: bool,
    retry: bool,
    game_running: bool,
    installed: String,
    latest: String,
    message: String,
}

fn spawn_addon_auto(app: AppHandle) {
    let state = app.state::<AppState>();
    if state.addon_started.swap(true, Ordering::SeqCst) {
        return;
    }
    let stop = Arc::clone(&state.addon_stop);
    std::thread::spawn(move || {
        let mut sched = addon_auto::Scheduler::new(addon_auto::Cadence::production());
        let mut now = 0u64;
        loop {
            if stop.load(Ordering::SeqCst) {
                break;
            }
            match sched.poll(now) {
                addon_auto::Due::Busy => break,
                addon_auto::Due::Wait { ms } => {
                    sleep_cancellable(Duration::from_millis(ms), &stop);
                    now = now.saturating_add(ms);
                }
                addon_auto::Due::Check => {
                    let notice = addon_auto_once(&app);
                    let outcome = addon_auto::outcome_from_notice(&notice.message, notice.ok, notice.retry);
                    let _ = app.emit("addon-auto", &notice);
                    sched.finish(now, outcome);
                }
            }
        }
    });
}

fn sleep_cancellable(total: Duration, stop: &AtomicBool) {
    let step = Duration::from_secs(1);
    let mut left = total;
    while left > Duration::ZERO {
        if stop.load(Ordering::SeqCst) {
            return;
        }
        let slice = if left > step { step } else { left };
        std::thread::sleep(slice);
        left = left.saturating_sub(slice);
    }
}

fn addon_auto_once(app: &AppHandle) -> AddonAutoNotice {
    let state = app.state::<AppState>();
    let prefs = match state.prefs.lock() {
        Ok(prefs) => prefs.clone(),
        Err(err) => {
            return AddonAutoNotice {
                ok: false,
                retry: false,
                game_running: false,
                installed: String::new(),
                latest: String::new(),
                message: err.to_string(),
            }
        }
    };
    let repo = defaults_repo();
    let game_running = core::game_seems_running();
    let latest = if repo.is_empty() { Ok(None) } else { core::latest_addon(&repo, &prefs.addon_name) };
    let (latest_ver, release) = match latest {
        Ok(release) => {
            let ver = release.as_ref().map(|item| item.version.to_string()).unwrap_or_default();
            (ver, release)
        }
        Err(err) => {
            return AddonAutoNotice {
                ok: false,
                retry: true,
                game_running,
                installed: prefs.last_addon_version.clone().unwrap_or_default(),
                latest: String::new(),
                message: err.to_string(),
            }
        }
    };
    let installed = if prefs.game_folder.is_empty() {
        None
    } else {
        core::installed_version(&core::addons_dir(PathBuf::from(&prefs.game_folder).as_path()), &prefs.addon_name).map(|v| v.to_string())
    };
    let update_available = match (&installed, &release) {
        (Some(installed), Some(release)) => semver_gt(&release.version.to_string(), installed),
        (None, Some(_)) => true,
        _ => false,
    };
    let skipped = prefs.skipped_addon_version.as_deref() == Some(latest_ver.as_str()) && !latest_ver.is_empty();
    let step = addon_auto::addon_auto_step(update_available, game_running, skipped);
    let installed_text = installed.clone().unwrap_or_default();
    match step {
        addon_auto::AddonAuto::Current => AddonAutoNotice {
            ok: true,
            retry: false,
            game_running,
            installed: installed_text,
            latest: latest_ver.clone(),
            message: if latest_ver.is_empty() { "Couldn't check for an addon update.".into() } else { "Up to date.".into() },
        },
        addon_auto::AddonAuto::KeepSkipped => AddonAutoNotice {
            ok: true,
            retry: false,
            game_running,
            installed: installed_text,
            latest: latest_ver,
            message: "This addon version was set aside earlier. It stays as it is.".into(),
        },
        addon_auto::AddonAuto::WaitForGame => AddonAutoNotice {
            ok: true,
            retry: true,
            game_running: true,
            installed: installed_text,
            latest: latest_ver,
            message: "Game is running. The addon update will wait until it closes.".into(),
        },
        addon_auto::AddonAuto::Install => {
            let Some(release) = release else {
                return AddonAutoNotice {
                    ok: true,
                    retry: false,
                    game_running,
                    installed: installed_text,
                    latest: latest_ver,
                    message: "Up to date.".into(),
                };
            };
            match core::install_addon(&release, PathBuf::from(&prefs.game_folder).as_path(), &prefs.addon_name, Some(&repo)) {
                Ok(version) => {
                    if let Ok(mut prefs) = state.prefs.lock() {
                        prefs.last_addon_version = Some(version.clone());
                        prefs.skipped_addon_version = None;
                        let _ = config::save_prefs(&prefs);
                    }
                    AddonAutoNotice {
                        ok: true,
                        retry: false,
                        game_running,
                        installed: version.clone(),
                        latest: version,
                        message: format!("Installed {latest_ver}."),
                    }
                }
                Err(err) => AddonAutoNotice {
                    ok: false,
                    retry: addon_auto::addon_failure_retries(matches!(err, core::UpdateError::Network(_)), 0),
                    game_running,
                    installed: installed_text,
                    latest: latest_ver,
                    message: err.to_string(),
                },
            }
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SelfCheck {
    current: String,
    latest: Option<String>,
    update_available: bool,
    offer: bool,
}

#[tauri::command]
async fn check_self_update(state: State<'_, AppState>) -> Result<SelfCheck, String> {
    let current = env!("CARGO_PKG_VERSION").to_string();
    let deferred = state.prefs.lock().map_err(|e| e.to_string())?.deferred_app_version.clone();
    let repo = defaults_repo();
    if repo.is_empty() {
        return Ok(SelfCheck { current, latest: None, update_available: false, offer: false });
    }
    let current_for_check = current.clone();
    let found = tauri::async_runtime::spawn_blocking(move || -> Result<(String, Option<String>, bool), String> {
        let latest = core::latest_app(&repo).map_err(|e| e.to_string())?;
        let ver = latest.as_ref().map(|release| release.version.to_string());
        let update_available = match (&ver, semver::Version::parse(&current_for_check)) {
            (Some(latest), Ok(current)) => semver::Version::parse(latest).map(|lv| lv > current).unwrap_or(false),
            _ => false,
        };
        Ok((current_for_check, ver, update_available))
    })
    .await
    .map_err(|e| e.to_string())?;
    let (current, ver, update_available) = found?;
    let offer = app_update::should_offer(&current, ver.as_deref(), deferred.as_deref());
    Ok(SelfCheck { current, latest: ver, update_available, offer })
}

#[tauri::command]
async fn defer_app_update(state: State<'_, AppState>, version: String) -> Result<(), String> {
    let version = version.trim();
    if version.is_empty() {
        return Err("No app version to defer.".into());
    }
    let mut prefs = state.prefs.lock().map_err(|e| e.to_string())?;
    prefs.deferred_app_version = Some(version.to_string());
    config::save_prefs(&prefs)
}

#[derive(Serialize)]
struct AppUpdateGate {
    installed: bool,
    message: String,
}

#[tauri::command]
async fn accept_app_update() -> Result<AppUpdateGate, String> {
    let repo = defaults_repo();
    let result = tauri::async_runtime::spawn_blocking(move || {
        app_update::decide(
            app_update::Choice::UpdateNow,
            &app_update::trust_from_build(),
            || app_update::fetch_published(&repo),
            app_update::launch_verified_installer,
        )
    })
    .await
    .map_err(|e| e.to_string())?;
    Ok(AppUpdateGate { installed: result.installed, message: result.message })
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

struct TrayLife {
    tray_ready: AtomicBool,
    exiting: AtomicBool,
}

fn restore_main(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}

fn install_tray(app: &tauri::App) -> Result<(), String> {
    let show = MenuItem::with_id(app, "show", tray_life::MENU_SHOW, true, None::<&str>).map_err(|e| e.to_string())?;
    let exit = MenuItem::with_id(app, "exit", tray_life::MENU_EXIT, true, None::<&str>).map_err(|e| e.to_string())?;
    let menu = Menu::with_items(app, &[&show, &exit]).map_err(|e| e.to_string())?;
    let icon = app.default_window_icon().cloned().ok_or_else(|| "This build has no window icon.".to_string())?;
    let mut tray = TrayIconBuilder::new()
        .menu(&menu)
        .icon(icon)
        .icon_as_template(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "exit" => {
                if let Some(life) = app.try_state::<TrayLife>() {
                    life.exiting.store(true, Ordering::SeqCst);
                }
                app.exit(0);
            }
            "show" => restore_main(app),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = event {
                restore_main(tray.app_handle());
            }
        });
    if let Some(name) = app.config().product_name.as_deref() {
        tray = tray.tooltip(name);
    }
    tray.build(app).map_err(|e| e.to_string())?;
    Ok(())
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
            restore_main(app);
        }))
        .manage(AppState {
            prefs: Mutex::new(loaded.prefs),
            origin: Mutex::new(loaded.origin),
            watching: Mutex::new(false),
            connect_cancel: Mutex::new(None),
            autostart_lock: Mutex::new(()),
            upload_seq: upload_gate::UploadSeq::default(),
            upload_flight: upload_gate::Flight::default(),
            addon_started: AtomicBool::new(false),
            addon_stop: Arc::new(AtomicBool::new(false)),
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
            defer_app_update,
            accept_app_update,
            pick_folder
        ])
        .setup(|app| {
            let tray_ready = match install_tray(app) {
                Ok(()) => true,
                Err(err) => {
                    eprintln!("tray icon was not created: {err}");
                    false
                }
            };
            app.manage(TrayLife {
                tray_ready: AtomicBool::new(tray_ready),
                exiting: AtomicBool::new(false),
            });
            if autostart::launched_hidden() {
                if let Some(window) = app.get_webview_window("main") {
                    if tray_life::hidden_launch_keeps_window_visible(tray_ready) {
                        let _ = window.show();
                        let _ = window.set_focus();
                    } else {
                        let _ = window.hide();
                    }
                }
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                let life = window.try_state::<TrayLife>();
                let tray_ready = life.as_ref().map(|life| life.tray_ready.load(Ordering::SeqCst)).unwrap_or(false);
                let exiting = life.as_ref().map(|life| life.exiting.load(Ordering::SeqCst)).unwrap_or(false);
                if tray_life::close_should_hide(tray_ready, exiting) {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
        })
        .build(tauri::generate_context!())
        .expect("the desktop app failed to start")
        .run(|app, event| {
            if let tauri::RunEvent::ExitRequested { .. } = event {
                if let Some(life) = app.try_state::<TrayLife>() {
                    life.exiting.store(true, Ordering::SeqCst);
                }
                if let Some(state) = app.try_state::<AppState>() {
                    state.addon_stop.store(true, Ordering::SeqCst);
                }
            }
        });
}
