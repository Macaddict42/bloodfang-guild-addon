//! Addon zip install from a GitHub release. Files only; never talks to the game process.
//! The newest addon is looked up on the site first (`/api/addon/latest`, which carries the zip's
//! SHA-256) and on GitHub's API only when the site can't answer.

use semver::Version;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct AddonRelease {
    pub tag: String,
    pub version: Version,
    pub zip_name: String,
    pub zip_url: String,
    pub sums_url: Option<String>,
    /// Detached signature of the archive or installer bytes, when the release has one.
    pub sig_url: Option<String>,
    /// SHA-256 of the zip when the source states it (the site does); otherwise SHA256SUMS is used.
    pub sha256: Option<String>,
    /// Where this release was found: "site" or "GitHub".
    pub source: &'static str,
}

#[derive(Debug)]
pub enum UpdateError {
    Network(String),
    Parse(String),
    Checksum,
    Zip(String),
    Io(String),
    /// A file stayed in use (the game holds it open) after the retries; the old addon was put back.
    Locked(String),
}
impl std::fmt::Display for UpdateError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            UpdateError::Network(e) => write!(f, "{e}"),
            UpdateError::Parse(e) => write!(f, "couldn't read the release: {e}"),
            UpdateError::Checksum => write!(f, "the download didn't match its SHA-256 checksum; left the old addon in place"),
            UpdateError::Zip(e) => write!(f, "couldn't unpack the addon: {e}"),
            UpdateError::Io(e) => write!(f, "{e}"),
            UpdateError::Locked(e) => write!(f, "a file was in use and couldn't be replaced: {e}; left the old addon in place"),
        }
    }
}

#[derive(Deserialize)]
struct GhRelease { tag_name: String, assets: Vec<GhAsset> }
#[derive(Deserialize)]
struct GhAsset { name: String, browser_download_url: String }

#[derive(Deserialize)]
struct SiteLatest { version: String, tag: String, #[serde(default)] assets: Vec<SiteAsset> }
#[derive(Deserialize)]
struct SiteAsset { name: String, #[serde(default)] sha256: Option<String>, url: String }

fn zip_matches(name: &str, addon_name: &str, ver: &Version) -> bool {
    name == format!("{addon_name}-{ver}.zip") || name.starts_with(&format!("{addon_name}-")) && name.ends_with(".zip")
}

/// Read the site's `/api/addon/latest` body. Only an https zip with a 64-hex SHA-256 is used.
pub fn parse_site_latest(body: &str, addon_name: &str) -> Result<Option<AddonRelease>, UpdateError> {
    let r: SiteLatest = serde_json::from_str(body).map_err(|e| UpdateError::Parse(format!("site answer: {e}")))?;
    let ver = Version::parse(r.version.trim()).map_err(|e| UpdateError::Parse(format!("site version {:?}: {e}", r.version)))?;
    let Some(a) = r.assets.iter().find(|a| zip_matches(&a.name, addon_name, &ver)) else { return Ok(None) };
    let sha = a.sha256.as_deref().map(|h| h.trim().to_ascii_lowercase()).filter(|h| h.len() == 64 && h.chars().all(|c| c.is_ascii_hexdigit()));
    let Some(sha) = sha else { return Err(UpdateError::Parse(format!("site lists {} without a SHA-256", a.name))) };
    if !a.url.starts_with("https://") {
        return Err(UpdateError::Parse(format!("site lists {} at a non-https address", a.name)));
    }
    Ok(Some(AddonRelease { tag: r.tag, version: ver, zip_name: a.name.clone(), zip_url: a.url.clone(), sums_url: None, sig_url: None, sha256: Some(sha), source: "site" }))
}

/// Newest addon release: the site first, then GitHub. Each attempt and failure is logged.
/// Fails only when neither source answered, with both reasons in the error.
pub fn latest_addon_any(site: Option<&str>, repo: &str, addon_name: &str) -> Result<Option<AddonRelease>, UpdateError> {
    let mut site_err = None;
    if let Some(site) = site.filter(|s| !s.is_empty()) {
        let url = format!("{}/api/addon/latest", site.trim_end_matches('/'));
        match get_with(&url, &http_agent(Duration::from_secs(20))) {
            Ok(body) => match parse_site_latest(&body, addon_name) {
                Ok(Some(rel)) => {
                    crate::log::info("addon", &format!("site lists {} ({})", rel.version, rel.zip_name));
                    return Ok(Some(rel));
                }
                Ok(None) => {
                    crate::log::warn("addon", &format!("site lists no {addon_name} zip; trying GitHub"));
                    site_err = Some(format!("the site lists no {addon_name} zip"));
                }
                Err(e) => {
                    crate::log::warn("addon", &format!("site answer unusable: {e}; trying GitHub"));
                    site_err = Some(e.to_string());
                }
            },
            Err(e) => {
                crate::log::warn("addon", &format!("site check failed: {e}; trying GitHub"));
                site_err = Some(e.to_string());
            }
        }
    }
    if repo.is_empty() {
        return match site_err {
            Some(e) => Err(UpdateError::Network(e)),
            None => Ok(None),
        };
    }
    match latest_addon(repo, addon_name) {
        Ok(found) => {
            match &found {
                Some(rel) => crate::log::info("addon", &format!("GitHub lists {} ({})", rel.version, rel.zip_name)),
                None => crate::log::warn("addon", &format!("GitHub lists no addon-v* release with a {addon_name} zip")),
            }
            Ok(found)
        }
        Err(e) => {
            crate::log::error("addon", &format!("GitHub check failed: {e}"));
            Err(match site_err {
                Some(s) => UpdateError::Network(format!("{s}; GitHub: {e}")),
                None => e,
            })
        }
    }
}

/// Latest `addon-vX.Y.Z` release for `repo` (`owner/name`) whose zip starts with `addon_name-`.
pub fn latest_addon(repo: &str, addon_name: &str) -> Result<Option<AddonRelease>, UpdateError> {
    let url = format!("https://api.github.com/repos/{repo}/releases?per_page=30");
    let body = get(&url)?;
    let list: Vec<GhRelease> = serde_json::from_str(&body).map_err(|e| UpdateError::Parse(e.to_string()))?;
    let mut best: Option<AddonRelease> = None;
    for r in list {
        let Some(ver) = r.tag_name.strip_prefix("addon-v").and_then(|v| Version::parse(v).ok()) else { continue };
        let zip = r.assets.iter().find(|a| zip_matches(&a.name, addon_name, &ver));
        let Some(zip) = zip else { continue };
        let sums = r.assets.iter().find(|a| a.name == "SHA256SUMS").map(|a| a.browser_download_url.clone());
        let sig_url = signature_url(&r.assets, &zip.name);
        let cand = AddonRelease { tag: r.tag_name.clone(), version: ver.clone(), zip_name: zip.name.clone(), zip_url: zip.browser_download_url.clone(), sums_url: sums, sig_url, sha256: None, source: "GitHub" };
        if best.as_ref().map(|b| &cand.version > &b.version).unwrap_or(true) { best = Some(cand); }
    }
    Ok(best)
}

fn http_agent(overall: Duration) -> ureq::Agent {
    let connect = Duration::from_secs(5).min(overall);
    crate::http::agent(connect, overall, overall, None)
}

fn host_of(url: &str) -> &str {
    url.split("://").nth(1).unwrap_or(url).split(['/', '?']).next().unwrap_or("")
}

fn network(url: &str, err: ureq::Error) -> UpdateError {
    UpdateError::Network(format!("couldn't reach {}: {}", host_of(url), crate::http::describe(err)))
}

fn get(url: &str) -> Result<String, UpdateError> {
    get_with(url, &http_agent(Duration::from_secs(20)))
}

fn get_with(url: &str, agent: &ureq::Agent) -> Result<String, UpdateError> {
    let mut req = agent.get(url);
    if host_of(url) == "api.github.com" {
        req = req.set("Accept", "application/vnd.github+json");
    } else {
        req = req.set("Accept", "application/json");
    }
    req.call()
        .map_err(|e| network(url, e))?
        .into_string()
        .map_err(|e| UpdateError::Network(format!("couldn't read the answer from {}: {}", host_of(url), crate::http::error_chain(&e))))
}

fn signature_url(assets: &[GhAsset], asset_name: &str) -> Option<String> {
    let expected = format!("{asset_name}.sig");
    assets.iter().find(|a| a.name == expected).map(|a| a.browser_download_url.clone())
}

pub fn download_text(url: &str) -> Result<String, UpdateError> {
    get(url)
}

pub fn download_bytes(url: &str) -> Result<Vec<u8>, UpdateError> {
    get_bytes(url)
}

fn get_bytes(url: &str) -> Result<Vec<u8>, UpdateError> {
    // Big downloads (the app update is ~15 MB): stop if nothing arrives for 30 s, give up after 10 min.
    let r = crate::http::agent(Duration::from_secs(5), Duration::from_secs(30), Duration::from_secs(600), None)
        .get(url)
        .call()
        .map_err(|e| network(url, e))?
        .into_reader();
    let mut buf = vec![];
    r.take(64 * 1024 * 1024).read_to_end(&mut buf).map_err(|e| UpdateError::Network(format!("download from {} broke off: {}", host_of(url), crate::http::error_chain(&e))))?;
    Ok(buf)
}

/// SHA-256 hex of bytes.
pub fn sha256_hex(bytes: &[u8]) -> String { Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect() }

/// Find `<hex>  <name>` (two spaces, GNU coreutils style) in SHA256SUMS text.
pub fn sums_entry(sums: &str, name: &str) -> Option<String> {
    for line in sums.lines() {
        let line = line.trim();
        if let Some((hash, file)) = line.split_once("  ").or_else(|| line.split_once(" *")) {
            if file.trim() == name { return Some(hash.trim().to_lowercase()); }
        }
    }
    None
}

/// Read `## Version: x.y.z` from an installed addon `.toc`.
pub fn installed_version(addons_dir: &Path, addon_name: &str) -> Option<Version> {
    let toc = addons_dir.join(addon_name).join(format!("{addon_name}.toc"));
    let text = fs::read_to_string(toc).ok()?;
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("## Version:") {
            return Version::parse(rest.trim()).ok();
        }
    }
    None
}

pub fn addons_dir(game_dir: &Path) -> PathBuf { game_dir.join("Interface").join("AddOns") }

/// Download, verify SHA-256, install into Interface/AddOns/<name>/ in place (see `install_zip_with`),
/// backup + rollback on failure.
/// Returns the installed version string. The archive is checked against the SHA-256 the site
/// gave or, for a GitHub release, against SHA256SUMS, in this process.
pub fn install_addon(rel: &AddonRelease, game_dir: &Path, addon_name: &str) -> Result<String, UpdateError> {
    let zip_bytes = get_bytes(&rel.zip_url)?;
    let expect = match (&rel.sha256, &rel.sums_url) {
        (Some(h), _) => h.clone(),
        (None, Some(u)) => {
            let sums = get(u)?;
            sums_entry(&sums, &rel.zip_name).ok_or_else(|| UpdateError::Parse(format!("{} missing from SHA256SUMS", rel.zip_name)))?
        }
        (None, None) => return Err(UpdateError::Parse("release has no SHA256SUMS".into())),
    };
    let actual = sha256_hex(&zip_bytes);
    if actual != expect {
        crate::log::error("addon", &format!("checksum mismatch for {}: expected {expect}, got {actual} ({} bytes)", rel.zip_name, zip_bytes.len()));
        return Err(UpdateError::Checksum);
    }
    install_zip_bytes(&zip_bytes, rel, game_dir, addon_name)
}

fn install_zip_bytes(zip_bytes: &[u8], rel: &AddonRelease, game_dir: &Path, addon_name: &str) -> Result<String, UpdateError> {
    install_zip_with(zip_bytes, rel, game_dir, addon_name, &Retry::production(), &|from: &Path, to: &Path| fs::rename(from, to))
}

/// How often a file the game holds open is retried before the install is rolled back.
#[derive(Clone, Copy, Debug)]
pub struct Retry {
    pub attempts: u32,
    pub base_ms: u64,
}

impl Retry {
    pub fn production() -> Self {
        // 6 tries over about 6 s (0.2, 0.4, 0.8, 1.6, 3.2 s apart).
        Self { attempts: 6, base_ms: 200 }
    }
}

/// A file another program holds open: Windows sharing or lock violation, or access denied while
/// the file is in use. These are worth retrying; anything else fails at once.
fn is_lock_error(e: &std::io::Error) -> bool {
    if e.kind() == std::io::ErrorKind::PermissionDenied {
        return true;
    }
    #[cfg(windows)]
    {
        // ERROR_SHARING_VIOLATION, ERROR_LOCK_VIOLATION
        if matches!(e.raw_os_error(), Some(32) | Some(33)) {
            return true;
        }
    }
    false
}

type RenameFn<'a> = &'a dyn Fn(&Path, &Path) -> std::io::Result<()>;

/// Copy `src` next to `target` under a temporary name, then rename it over `target`.
/// The rename is retried while the target is locked. The temporary file is removed on failure.
fn place_file(src: &Path, target: &Path, retry: &Retry, rename: RenameFn) -> std::io::Result<()> {
    let dir = target.parent().ok_or_else(|| std::io::Error::other("file has no folder"))?;
    fs::create_dir_all(dir)?;
    let name = target.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    let tmp = dir.join(format!(".{name}.uploader-tmp"));
    let _ = fs::remove_file(&tmp);
    fs::copy(src, &tmp)?;
    let mut attempt = 0u32;
    loop {
        match rename(&tmp, target) {
            Ok(()) => return Ok(()),
            Err(e) if is_lock_error(&e) && attempt + 1 < retry.attempts => {
                std::thread::sleep(Duration::from_millis(retry.base_ms.saturating_mul(1u64 << attempt.min(10))));
                attempt += 1;
            }
            Err(e) => {
                let _ = fs::remove_file(&tmp);
                return Err(e);
            }
        }
    }
}

/// Every file under `root`, as paths relative to it.
fn list_files(root: &Path) -> std::io::Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    let mut stack = vec![PathBuf::new()];
    while let Some(rel) = stack.pop() {
        for ent in fs::read_dir(root.join(&rel))? {
            let ent = ent?;
            let child = rel.join(ent.file_name());
            if ent.file_type()?.is_dir() {
                stack.push(child);
            } else {
                out.push(child);
            }
        }
    }
    out.sort();
    Ok(out)
}

/// Install order: every other file first, `.toc` files last, so the game never sees a new `.toc`
/// that points at files not written yet.
fn install_order(mut files: Vec<PathBuf>) -> Vec<PathBuf> {
    let is_toc = |p: &PathBuf| p.extension().map(|e| e.eq_ignore_ascii_case("toc")).unwrap_or(false);
    files.sort_by_key(|p| (is_toc(p), p.clone()));
    files
}

/// Install that is safe while the game is running: the game may hold addon files open and reads
/// them again on /reload or a zone change.
/// - The archive is unpacked into a staging folder in `Interface/` (same volume as AddOns, never
///   scanned for addons).
/// - The live folder is copied to a backup; it is never renamed or removed.
/// - Each file is replaced in place (temporary file, then rename), `.toc` last. Files the new
///   version no longer ships are left in place (the `.toc` no longer loads them).
/// - A file that stays locked after the retries rolls back every file already replaced and
///   returns `UpdateError::Locked`.
fn install_zip_with(zip_bytes: &[u8], rel: &AddonRelease, game_dir: &Path, addon_name: &str, retry: &Retry, rename: RenameFn) -> Result<String, UpdateError> {
    let addons = addons_dir(game_dir);
    let interface = addons.parent().map(Path::to_path_buf).unwrap_or_else(|| game_dir.to_path_buf());
    fs::create_dir_all(&addons).map_err(|e| UpdateError::Io(format!("couldn't create {}: {e}", addons.display())))?;
    let dest = addons.join(addon_name);
    // Earlier versions kept the backup inside AddOns; it is our own folder, so tidy it away.
    let legacy = addons.join(format!("{addon_name}.bak-uploader"));
    if legacy.exists() {
        let _ = fs::remove_dir_all(&legacy);
    }

    let stage = tempfile::Builder::new()
        .prefix(&format!(".{addon_name}.uploader-stage-"))
        .tempdir_in(&interface)
        .map_err(|e| UpdateError::Io(format!("couldn't make a staging folder in {}: {e}", interface.display())))?;
    unpack_zip(zip_bytes, stage.path()).map_err(UpdateError::Zip)?;
    // Zip root is ADDON_NAME/ (or a single top-level folder).
    let staged = stage.path().join(addon_name);
    let from = if staged.is_dir() {
        staged
    } else {
        fs::read_dir(stage.path())
            .ok()
            .and_then(|mut d| d.next().and_then(|e| e.ok()).map(|e| e.path()))
            .filter(|p| p.is_dir())
            .ok_or_else(|| UpdateError::Zip("zip has no addon folder".into()))?
    };
    let files = list_files(&from).map_err(|e| UpdateError::Zip(format!("couldn't read the unpacked addon: {e}")))?;
    if files.is_empty() {
        return Err(UpdateError::Zip("zip has no addon files".into()));
    }

    let had_dest = dest.is_dir();
    let backup = interface.join(format!(".{addon_name}.uploader-backup"));
    if backup.exists() {
        let _ = fs::remove_dir_all(&backup);
    }
    if had_dest {
        copy_dir(&dest, &backup).map_err(|e| {
            let _ = fs::remove_dir_all(&backup);
            UpdateError::Io(format!("couldn't back up the old addon ({}): {e}; it was left as it is", dest.display()))
        })?;
    }

    let mut done: Vec<PathBuf> = Vec::new();
    for rel_path in install_order(files) {
        if let Err(e) = place_file(&from.join(&rel_path), &dest.join(&rel_path), retry, rename) {
            let locked = is_lock_error(&e);
            let restored = roll_back(&dest, &backup, had_dest, &done, retry, rename);
            let _ = fs::remove_dir_all(&backup);
            let what = format!("{} ({e})", dest.join(&rel_path).display());
            crate::log::warn("addon", &format!("install stopped at {what}; rolled back {} file(s){}", done.len(), if restored { "" } else { " (some could not be restored)" }));
            return Err(if locked {
                UpdateError::Locked(what)
            } else {
                UpdateError::Io(format!("couldn't copy the addon into {}: {what}", dest.display()))
            });
        }
        done.push(rel_path);
    }
    let _ = fs::remove_dir_all(&backup);
    Ok(rel.version.to_string())
}

/// Put back every file in `done` from the backup (or remove it when the old version didn't have it).
/// Returns false when a file couldn't be restored.
fn roll_back(dest: &Path, backup: &Path, had_dest: bool, done: &[PathBuf], retry: &Retry, rename: RenameFn) -> bool {
    if !had_dest {
        return fs::remove_dir_all(dest).is_ok() || !dest.exists();
    }
    let mut ok = true;
    // Undo in reverse, so a new .toc goes first.
    for rel_path in done.iter().rev() {
        let old = backup.join(rel_path);
        let target = dest.join(rel_path);
        if old.is_file() {
            if place_file(&old, &target, retry, rename).is_err() {
                ok = false;
            }
        } else if fs::remove_file(&target).is_err() && target.exists() {
            ok = false;
        }
    }
    ok
}

fn zip_name_stays_inside(name: &Path) -> bool {
    !name.as_os_str().is_empty()
        && !name.is_absolute()
        && name.components().all(|c| matches!(c, std::path::Component::Normal(_) | std::path::Component::CurDir))
}

fn unpack_zip(bytes: &[u8], dest: &Path) -> Result<(), String> {
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes)).map_err(|e| e.to_string())?;
    for i in 0..archive.len() {
        let mut file = archive.by_index(i).map_err(|e| e.to_string())?;
        let raw_name = file.name().to_string();
        let name = Path::new(&raw_name);
        if !zip_name_stays_inside(name) {
            return Err("addon archive tried to leave the addon folder".into());
        }
        let out = dest.join(name);
        if file.is_dir() {
            fs::create_dir_all(&out).map_err(|e| e.to_string())?;
        } else {
            if let Some(p) = out.parent() { fs::create_dir_all(p).map_err(|e| e.to_string())?; }
            let mut f = fs::File::create(&out).map_err(|e| e.to_string())?;
            std::io::copy(&mut file, &mut f).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

fn copy_dir(src: &Path, dst: &Path) -> Result<(), String> {
    fs::create_dir_all(dst).map_err(|e| e.to_string())?;
    for ent in fs::read_dir(src).map_err(|e| e.to_string())? {
        let ent = ent.map_err(|e| e.to_string())?;
        let to = dst.join(ent.file_name());
        if ent.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            copy_dir(&ent.path(), &to)?;
        } else {
            fs::copy(ent.path(), to).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

/// The update asset for `os`: the NSIS setup .exe on Windows; on macOS the `.app.tar.gz` archive
/// (an update replaces the .app with it; a .dmg can't be run as an installer).
pub fn pick_app_asset(names: &[&str], os: &str) -> Option<usize> {
    let lower: Vec<String> = names.iter().map(|n| n.to_ascii_lowercase()).collect();
    match os {
        "windows" => lower.iter().position(|n| n.ends_with(".exe") && !n.contains("unst")),
        "macos" => lower.iter().position(|n| n.ends_with(".app.tar.gz")),
        _ => lower.iter().position(|n| n.ends_with(".app.tar.gz") || n.ends_with(".exe")),
    }
}

/// Latest desktop app release (`uploader-app-vX.Y.Z`) if any.
pub fn latest_app(repo: &str) -> Result<Option<AddonRelease>, UpdateError> {
    let url = format!("https://api.github.com/repos/{repo}/releases?per_page=30");
    let body = get(&url)?;
    let list: Vec<GhRelease> = serde_json::from_str(&body).map_err(|e| UpdateError::Parse(e.to_string()))?;
    let mut best: Option<AddonRelease> = None;
    for r in list {
        let Some(ver) = r.tag_name.strip_prefix("uploader-app-v").and_then(|v| Version::parse(v).ok()) else { continue };
        let names: Vec<&str> = r.assets.iter().map(|a| a.name.as_str()).collect();
        let Some(idx) = pick_app_asset(&names, std::env::consts::OS) else { continue };
        let asset = &r.assets[idx];
        let sums = r.assets.iter().find(|a| a.name == "SHA256SUMS").map(|a| a.browser_download_url.clone());
        let sig_url = signature_url(&r.assets, &asset.name);
        let cand = AddonRelease {
            tag: r.tag_name.clone(),
            version: ver.clone(),
            zip_name: asset.name.clone(),
            zip_url: asset.browser_download_url.clone(),
            sums_url: sums,
            sig_url,
            sha256: None,
            source: "GitHub",
        };
        if best.as_ref().map(|b| &cand.version > &b.version).unwrap_or(true) { best = Some(cand); }
    }
    Ok(best)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::net::TcpListener;
    use std::time::Instant;

    #[test]
    fn zip_parent_dir_is_rejected() {
        use std::io::Write;
        use zip::write::FileOptions;
        let mut cursor = std::io::Cursor::new(Vec::new());
        {
            let mut writer = zip::ZipWriter::new(&mut cursor);
            writer.start_file("../outside.txt", FileOptions::default()).unwrap();
            writer.write_all(b"no").unwrap();
            writer.finish().unwrap();
        }
        let dir = tempfile::tempdir().unwrap();
        let err = unpack_zip(cursor.get_ref(), dir.path()).unwrap_err();
        assert!(err.contains("leave the addon folder"));
        assert!(!dir.path().join("outside.txt").exists());
        assert!(!dir.path().parent().unwrap().join("outside.txt").exists());
    }

    #[test]
    fn mac_updates_use_the_app_archive_and_windows_the_setup_exe() {
        let names = [
            "guild-uploader-app-0.1.3-macos-Example_Uploader_0.1.3_universal.dmg",
            "guild-uploader-app-0.1.3-macos-Example_Uploader_0.1.3_universal.dmg.sig",
            "guild-uploader-app-0.1.3-macos-Example_Uploader.app.tar.gz",
            "guild-uploader-app-0.1.3-macos-Example_Uploader.app.tar.gz.sig",
            "guild-uploader-app-0.1.3-windows-Example_Uploader_0.1.3_x64-setup.exe",
            "guild-uploader-app-0.1.3-windows-Example_Uploader_0.1.3_x64-setup.exe.sig",
            "SHA256SUMS",
        ];
        assert_eq!(pick_app_asset(&names, "macos"), Some(2));
        assert_eq!(pick_app_asset(&names, "windows"), Some(4));
        assert_eq!(pick_app_asset(&names[..2], "macos"), None, "a .dmg alone is not an update");
    }

    #[test]
    fn site_latest_is_used_with_its_hash() {
        let body = r#"{"version":"0.2.1","tag":"addon-v0.2.1","publishedAt":"2026-11-10T21:10:14.000Z","assets":[{"name":"Example-0.2.1.zip","sizeBytes":51492,"sha256":"80E43DF26C1BD1C322CF12EBA2F47AF987B715A5C4DAB6A5DF57921845A3711F","url":"https://github.com/o/r/releases/download/addon-v0.2.1/Example-0.2.1.zip"}]}"#;
        let rel = parse_site_latest(body, "Example").unwrap().unwrap();
        assert_eq!(rel.version.to_string(), "0.2.1");
        assert_eq!(rel.source, "site");
        assert_eq!(rel.sha256.as_deref(), Some("80e43df26c1bd1c322cf12eba2f47af987b715a5c4dab6a5df57921845a3711f"));
        assert!(parse_site_latest(body, "Other").unwrap().is_none());
        let no_hash = body.replace("\"sha256\":\"80E43DF26C1BD1C322CF12EBA2F47AF987B715A5C4DAB6A5DF57921845A3711F\",", "");
        assert!(parse_site_latest(&no_hash, "Example").is_err());
        let http = body.replace("https://github.com", "http://github.com");
        assert!(parse_site_latest(&http, "Example").is_err());
    }

    /// Serve fixed answers by path; count hits.
    fn serve_paths(routes: Vec<(&'static str, u16, Vec<u8>)>) -> (String, std::sync::Arc<std::sync::Mutex<Vec<String>>>) {
        use std::io::Read;
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", l.local_addr().unwrap());
        let hits = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let log = hits.clone();
        std::thread::spawn(move || {
            for c in l.incoming() {
                let Ok(mut c) = c else { continue };
                let mut buf = [0u8; 4096];
                let n = c.read(&mut buf).unwrap_or(0);
                let req = String::from_utf8_lossy(&buf[..n]).to_string();
                let path = req.split_whitespace().nth(1).unwrap_or("").to_string();
                log.lock().unwrap().push(path.clone());
                let (code, body) = routes.iter().find(|r| path.starts_with(r.0)).map(|r| (r.1, r.2.clone())).unwrap_or((404, b"{}".to_vec()));
                let _ = write!(c, "HTTP/1.1 {code} X\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
                let _ = c.write_all(&body);
            }
        });
        (base, hits)
    }

    fn addon_zip(name: &str, version: &str) -> Vec<u8> {
        use zip::write::FileOptions;
        let mut cursor = std::io::Cursor::new(Vec::new());
        {
            let mut w = zip::ZipWriter::new(&mut cursor);
            w.start_file(format!("{name}/{name}.toc"), FileOptions::default()).unwrap();
            write!(w, "## Title: {name}\n## Version: {version}\n").unwrap();
            w.finish().unwrap();
        }
        cursor.into_inner()
    }

    #[test]
    fn site_check_falls_back_and_reports_both_reasons() {
        // Site down and no GitHub repo: the error names the site and the real cause.
        let err = latest_addon_any(Some("http://127.0.0.1:9"), "", "Example").unwrap_err().to_string();
        assert!(err.contains("couldn't reach 127.0.0.1:9"), "{err}");
        assert!(err.to_lowercase().contains("connect"), "{err}");
        // Site answers 404 and there is no repo to fall back to: the error carries the status.
        let (site, hits) = serve_paths(vec![]);
        let err = latest_addon_any(Some(&site), "", "Example").unwrap_err().to_string();
        assert!(err.contains("HTTP 404"), "{err}");
        assert_eq!(hits.lock().unwrap().as_slice(), ["/api/addon/latest"]);
    }

    #[test]
    fn install_from_site_hash_and_reject_a_mismatch() {
        let zip = addon_zip("Example", "0.2.1");
        let (base, _) = serve_paths(vec![("/Example-0.2.1.zip", 200, zip.clone())]);
        let game = tempfile::tempdir().unwrap();
        let old = addons_dir(game.path()).join("Example");
        fs::create_dir_all(&old).unwrap();
        fs::write(old.join("Example.toc"), "## Version: 0.2.0\n").unwrap();
        let mut rel = AddonRelease { tag: "addon-v0.2.1".into(), version: Version::parse("0.2.1").unwrap(), zip_name: "Example-0.2.1.zip".into(), zip_url: format!("{base}/Example-0.2.1.zip"), sums_url: None, sig_url: None, sha256: Some("0".repeat(64)), source: "site" };
        assert!(matches!(install_addon(&rel, game.path(), "Example"), Err(UpdateError::Checksum)));
        assert_eq!(installed_version(&addons_dir(game.path()), "Example").unwrap().to_string(), "0.2.0", "old addon left in place");
        rel.sha256 = Some(sha256_hex(&zip));
        assert_eq!(install_addon(&rel, game.path(), "Example").unwrap(), "0.2.1");
        assert_eq!(installed_version(&addons_dir(game.path()), "Example").unwrap().to_string(), "0.2.1");
    }

    #[test]
    fn a_broken_zip_restores_the_old_addon() {
        let game = tempfile::tempdir().unwrap();
        let old = addons_dir(game.path()).join("Example");
        fs::create_dir_all(&old).unwrap();
        fs::write(old.join("Example.toc"), "## Version: 0.2.0\n").unwrap();
        let rel = AddonRelease { tag: "t".into(), version: Version::parse("0.2.1").unwrap(), zip_name: "Example-0.2.1.zip".into(), zip_url: String::new(), sums_url: None, sig_url: None, sha256: None, source: "site" };
        assert!(matches!(install_zip_bytes(b"not a zip", &rel, game.path(), "Example"), Err(UpdateError::Zip(_))));
        assert_eq!(installed_version(&addons_dir(game.path()), "Example").unwrap().to_string(), "0.2.0");
    }

    fn zip_of(files: &[(&str, &str)]) -> Vec<u8> {
        use zip::write::FileOptions;
        let mut cursor = std::io::Cursor::new(Vec::new());
        {
            let mut w = zip::ZipWriter::new(&mut cursor);
            for (name, body) in files {
                w.start_file(*name, FileOptions::default()).unwrap();
                w.write_all(body.as_bytes()).unwrap();
            }
            w.finish().unwrap();
        }
        cursor.into_inner()
    }

    fn old_addon(game: &Path) -> PathBuf {
        let old = addons_dir(game).join("Example");
        fs::create_dir_all(old.join("libs")).unwrap();
        fs::write(old.join("Example.toc"), "## Version: 0.2.0\nCore.lua\nGone.lua\n").unwrap();
        fs::write(old.join("Core.lua"), "old core").unwrap();
        fs::write(old.join("Gone.lua"), "removed in 0.2.1").unwrap();
        fs::write(old.join("libs").join("Lib.lua"), "old lib").unwrap();
        old
    }

    fn new_zip() -> Vec<u8> {
        zip_of(&[
            ("Example/Example.toc", "## Version: 0.2.1\nCore.lua\nNew.lua\n"),
            ("Example/Core.lua", "new core"),
            ("Example/New.lua", "added in 0.2.1"),
            ("Example/libs/Lib.lua", "new lib"),
        ])
    }

    fn rel_021() -> AddonRelease {
        AddonRelease { tag: "addon-v0.2.1".into(), version: Version::parse("0.2.1").unwrap(), zip_name: "Example-0.2.1.zip".into(), zip_url: String::new(), sums_url: None, sig_url: None, sha256: None, source: "site" }
    }

    fn only_example_left(game: &Path) {
        let mut addons: Vec<String> = fs::read_dir(addons_dir(game)).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().to_string()).collect();
        addons.sort();
        assert_eq!(addons, ["Example"], "no staging, backup or temporary folder in AddOns");
        let interface: Vec<String> = fs::read_dir(game.join("Interface")).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().to_string()).collect();
        assert_eq!(interface, ["AddOns"], "staging folder and backup were cleaned up");
    }

    fn no_tmp_files(dir: &Path) {
        for f in list_files(dir).unwrap() {
            assert!(!f.to_string_lossy().contains("uploader-tmp"), "left {f:?}");
        }
    }

    #[test]
    fn in_place_install_keeps_the_folder_writes_toc_last_and_leaves_removed_files() {
        let game = tempfile::tempdir().unwrap();
        let dest = old_addon(game.path());
        #[cfg(unix)]
        let ino = { use std::os::unix::fs::MetadataExt; fs::metadata(&dest).unwrap().ino() };
        let order = std::sync::Mutex::new(Vec::new());
        let rename = |from: &Path, to: &Path| {
            order.lock().unwrap().push(to.strip_prefix(&dest).unwrap().to_path_buf());
            assert!(from.parent() == to.parent(), "the temporary file sits next to its target");
            fs::rename(from, to)
        };
        let fast = Retry { attempts: 3, base_ms: 1 };
        assert_eq!(install_zip_with(&new_zip(), &rel_021(), game.path(), "Example", &fast, &rename).unwrap(), "0.2.1");
        let order = order.into_inner().unwrap();
        assert_eq!(order.len(), 4);
        assert_eq!(order.last().unwrap(), Path::new("Example.toc"), ".toc is written last: {order:?}");
        assert_eq!(fs::read_to_string(dest.join("Core.lua")).unwrap(), "new core");
        assert_eq!(fs::read_to_string(dest.join("New.lua")).unwrap(), "added in 0.2.1");
        assert_eq!(fs::read_to_string(dest.join("libs/Lib.lua")).unwrap(), "new lib");
        assert_eq!(fs::read_to_string(dest.join("Gone.lua")).unwrap(), "removed in 0.2.1", "files the new version dropped stay");
        assert_eq!(installed_version(&addons_dir(game.path()), "Example").unwrap().to_string(), "0.2.1");
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            assert_eq!(fs::metadata(&dest).unwrap().ino(), ino, "the live folder was not renamed or recreated");
        }
        only_example_left(game.path());
        no_tmp_files(&dest);
    }

    #[test]
    fn a_file_that_stays_locked_rolls_every_file_back() {
        let game = tempfile::tempdir().unwrap();
        let dest = old_addon(game.path());
        let tries = std::sync::atomic::AtomicU32::new(0);
        // New.lua sorts after Core.lua and libs/Lib.lua sorts last of the non-.toc files: lock it,
        // so Core.lua and New.lua are already replaced when the install stops.
        let rename = |from: &Path, to: &Path| {
            if to.ends_with("libs/Lib.lua") {
                tries.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                return Err(std::io::Error::from(std::io::ErrorKind::PermissionDenied));
            }
            fs::rename(from, to)
        };
        let fast = Retry { attempts: 4, base_ms: 1 };
        let err = install_zip_with(&new_zip(), &rel_021(), game.path(), "Example", &fast, &rename).unwrap_err();
        assert!(matches!(err, UpdateError::Locked(_)), "{err}");
        assert!(err.to_string().contains("left the old addon in place"), "{err}");
        assert_eq!(tries.load(std::sync::atomic::Ordering::SeqCst), 4, "retried up to the limit");
        assert_eq!(fs::read_to_string(dest.join("Core.lua")).unwrap(), "old core", "replaced file restored");
        assert_eq!(fs::read_to_string(dest.join("libs/Lib.lua")).unwrap(), "old lib");
        assert!(!dest.join("New.lua").exists(), "file the old version didn't have was removed");
        assert_eq!(fs::read_to_string(dest.join("Gone.lua")).unwrap(), "removed in 0.2.1");
        assert_eq!(installed_version(&addons_dir(game.path()), "Example").unwrap().to_string(), "0.2.0", "old .toc never replaced");
        only_example_left(game.path());
        no_tmp_files(&dest);
    }

    #[test]
    fn a_short_lock_is_retried_and_the_install_finishes() {
        let game = tempfile::tempdir().unwrap();
        let dest = old_addon(game.path());
        let tries = std::sync::atomic::AtomicU32::new(0);
        let rename = |from: &Path, to: &Path| {
            if to.ends_with("Core.lua") && tries.fetch_add(1, std::sync::atomic::Ordering::SeqCst) < 2 {
                return Err(std::io::Error::from(std::io::ErrorKind::PermissionDenied));
            }
            fs::rename(from, to)
        };
        let fast = Retry { attempts: 6, base_ms: 1 };
        assert_eq!(install_zip_with(&new_zip(), &rel_021(), game.path(), "Example", &fast, &rename).unwrap(), "0.2.1");
        assert_eq!(tries.load(std::sync::atomic::Ordering::SeqCst), 3);
        assert_eq!(fs::read_to_string(dest.join("Core.lua")).unwrap(), "new core");
        only_example_left(game.path());
    }

    #[test]
    fn other_errors_are_not_retried_and_a_first_install_is_removed_on_failure() {
        let game = tempfile::tempdir().unwrap();
        let tries = std::sync::atomic::AtomicU32::new(0);
        let rename = |from: &Path, to: &Path| {
            if to.ends_with("New.lua") {
                tries.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                return Err(std::io::Error::other("disk full"));
            }
            fs::rename(from, to)
        };
        let fast = Retry { attempts: 6, base_ms: 1 };
        let err = install_zip_with(&new_zip(), &rel_021(), game.path(), "Example", &fast, &rename).unwrap_err();
        assert!(matches!(err, UpdateError::Io(_)), "{err}");
        assert_eq!(tries.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert!(!addons_dir(game.path()).join("Example").exists(), "a half first install is removed");
        // And a clean first install works with the real rename.
        assert_eq!(install_zip_bytes(&new_zip(), &rel_021(), game.path(), "Example").unwrap(), "0.2.1");
        only_example_left(game.path());
    }

    #[test]
    fn an_old_backup_folder_in_addons_is_tidied() {
        let game = tempfile::tempdir().unwrap();
        old_addon(game.path());
        let legacy = addons_dir(game.path()).join("Example.bak-uploader");
        fs::create_dir_all(&legacy).unwrap();
        fs::write(legacy.join("Example.toc"), "## Version: 0.1.0\n").unwrap();
        assert_eq!(install_zip_bytes(&new_zip(), &rel_021(), game.path(), "Example").unwrap(), "0.2.1");
        only_example_left(game.path());
    }

    #[test]
    fn sums_parse() {
        let s = "abc123  Addon-0.1.1.zip\ndef  other\n";
        assert_eq!(sums_entry(s, "Addon-0.1.1.zip").as_deref(), Some("abc123"));
    }

    #[test]
    fn http_read_timeout_returns_without_hanging() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            if let Ok((mut sock, _)) = listener.accept() {
                let _ = sock.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\n\r\n");
                std::thread::sleep(Duration::from_secs(30));
            }
        });
        let start = Instant::now();
        let result = get_with(&format!("http://127.0.0.1:{port}/slow"), &http_agent(Duration::from_millis(400)));
        let elapsed = start.elapsed();
        assert!(result.is_err());
        assert!(elapsed < Duration::from_secs(3), "http elapsed {elapsed:?}");
        eprintln!("MEASURE http_read_timeout elapsed_ms={}", elapsed.as_millis());
    }

}
