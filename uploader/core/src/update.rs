//! Addon zip install from a GitHub release. Files only; never talks to the game process.

use semver::Version;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

#[derive(Debug, Clone)]
pub struct AddonRelease {
    pub tag: String,
    pub version: Version,
    pub zip_name: String,
    pub zip_url: String,
    pub sums_url: Option<String>,
    /// Detached signature of the archive or installer bytes, when the release has one.
    pub sig_url: Option<String>,
}

#[derive(Debug)]
pub enum UpdateError {
    Network(String),
    Parse(String),
    Checksum,
    Zip(String),
    Io(String),
}
impl std::fmt::Display for UpdateError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            UpdateError::Network(e) => write!(f, "couldn't reach GitHub: {e}"),
            UpdateError::Parse(e) => write!(f, "couldn't read the release: {e}"),
            UpdateError::Checksum => write!(f, "the download didn't match SHA256SUMS; left the old addon in place"),
            UpdateError::Zip(e) => write!(f, "couldn't unpack the addon: {e}"),
            UpdateError::Io(e) => write!(f, "{e}"),
        }
    }
}

#[derive(Deserialize)]
struct GhRelease { tag_name: String, assets: Vec<GhAsset> }
#[derive(Deserialize)]
struct GhAsset { name: String, browser_download_url: String }

/// Latest `addon-vX.Y.Z` release for `repo` (`owner/name`) whose zip starts with `addon_name-`.
pub fn latest_addon(repo: &str, addon_name: &str) -> Result<Option<AddonRelease>, UpdateError> {
    let url = format!("https://api.github.com/repos/{repo}/releases?per_page=30");
    let body = get(&url)?;
    let list: Vec<GhRelease> = serde_json::from_str(&body).map_err(|e| UpdateError::Parse(e.to_string()))?;
    let mut best: Option<AddonRelease> = None;
    for r in list {
        let Some(ver) = r.tag_name.strip_prefix("addon-v").and_then(|v| Version::parse(v).ok()) else { continue };
        let zip = r.assets.iter().find(|a| a.name == format!("{addon_name}-{ver}.zip") || a.name.starts_with(&format!("{addon_name}-")) && a.name.ends_with(".zip"));
        let Some(zip) = zip else { continue };
        let sums = r.assets.iter().find(|a| a.name == "SHA256SUMS").map(|a| a.browser_download_url.clone());
        let sig_url = signature_url(&r.assets, &zip.name);
        let cand = AddonRelease { tag: r.tag_name.clone(), version: ver.clone(), zip_name: zip.name.clone(), zip_url: zip.browser_download_url.clone(), sums_url: sums, sig_url };
        if best.as_ref().map(|b| &cand.version > &b.version).unwrap_or(true) { best = Some(cand); }
    }
    Ok(best)
}

fn http_agent(overall: Duration) -> ureq::Agent {
    let connect = Duration::from_secs(5).min(overall);
    ureq::AgentBuilder::new()
        .timeout_connect(connect)
        .timeout_read(overall)
        .timeout(overall)
        .build()
}

fn get(url: &str) -> Result<String, UpdateError> {
    get_with(url, &http_agent(Duration::from_secs(20)))
}

fn get_with(url: &str, agent: &ureq::Agent) -> Result<String, UpdateError> {
    agent
        .get(url)
        .set("User-Agent", "guild-uploader")
        .set("Accept", "application/vnd.github+json")
        .call()
        .map_err(|e| UpdateError::Network(e.to_string()))?
        .into_string()
        .map_err(|e| UpdateError::Network(e.to_string()))
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
    let r = http_agent(Duration::from_secs(30))
        .get(url)
        .set("User-Agent", "guild-uploader")
        .call()
        .map_err(|e| UpdateError::Network(e.to_string()))?
        .into_reader();
    let mut buf = vec![];
    r.take(64 * 1024 * 1024).read_to_end(&mut buf).map_err(|e| UpdateError::Network(e.to_string()))?;
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

/// Download, verify SHA-256, install into Interface/AddOns/<name>/, backup + rollback on failure.
/// Returns the installed version string. `attest_repo` enables best-effort `gh attestation verify`.
pub fn install_addon(rel: &AddonRelease, game_dir: &Path, addon_name: &str, attest_repo: Option<&str>) -> Result<String, UpdateError> {
    let zip_bytes = get_bytes(&rel.zip_url)?;
    let sums = match &rel.sums_url {
        Some(u) => get(u)?,
        None => return Err(UpdateError::Parse("release has no SHA256SUMS".into())),
    };
    let expect = sums_entry(&sums, &rel.zip_name).ok_or_else(|| UpdateError::Parse(format!("{} missing from SHA256SUMS", rel.zip_name)))?;
    if sha256_hex(&zip_bytes) != expect { return Err(UpdateError::Checksum); }

    let dest = addons_dir(game_dir).join(addon_name);
    let backup = addons_dir(game_dir).join(format!("{addon_name}.bak-uploader"));
    if backup.exists() { let _ = fs::remove_dir_all(&backup); }
    if dest.exists() {
        fs::rename(&dest, &backup).map_err(|e| UpdateError::Io(e.to_string()))?;
    }

    let tmp = tempfile::tempdir().map_err(|e| UpdateError::Io(e.to_string()))?;
    unpack_zip(&zip_bytes, tmp.path()).map_err(UpdateError::Zip)?;
    // Zip root is ADDON_NAME/
    let staged = tmp.path().join(addon_name);
    let from = if staged.is_dir() { staged } else {
        // single top-level dir
        fs::read_dir(tmp.path()).ok().and_then(|mut d| d.next().and_then(|e| e.ok()).map(|e| e.path()))
            .filter(|p| p.is_dir())
            .ok_or_else(|| UpdateError::Zip("zip has no addon folder".into()))?
    };
    fs::create_dir_all(addons_dir(game_dir)).map_err(|e| UpdateError::Io(e.to_string()))?;
    if let Err(e) = copy_dir(&from, &dest) {
        let _ = fs::remove_dir_all(&dest);
        if backup.exists() { let _ = fs::rename(&backup, &dest); }
        return Err(UpdateError::Io(e));
    }
    let _ = fs::remove_dir_all(&backup);

    if let Some(repo) = attest_repo {
        // Best-effort: write zip to a temp file and run gh if present.
        let zpath = tmp.path().join(&rel.zip_name);
        if fs::write(&zpath, &zip_bytes).is_ok() {
            let _ = Command::new("gh").args(["attestation", "verify", zpath.to_str().unwrap_or(""), "-R", repo]).status();
        }
    }
    
    Ok(rel.version.to_string())
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

/// Latest desktop app release (`uploader-app-vX.Y.Z`) if any.
pub fn latest_app(repo: &str) -> Result<Option<AddonRelease>, UpdateError> {
    let url = format!("https://api.github.com/repos/{repo}/releases?per_page=30");
    let body = get(&url)?;
    let list: Vec<GhRelease> = serde_json::from_str(&body).map_err(|e| UpdateError::Parse(e.to_string()))?;
    let mut best: Option<AddonRelease> = None;
    for r in list {
        let Some(ver) = r.tag_name.strip_prefix("uploader-app-v").and_then(|v| Version::parse(v).ok()) else { continue };
        let asset = {
            let lower = |a: &GhAsset| a.name.to_ascii_lowercase();
            #[cfg(target_os = "macos")]
            let prefer = r.assets.iter().find(|a| {
                let n = lower(a);
                n.ends_with(".dmg") && (n.contains("universal") || n.contains("macos"))
            }).or_else(|| r.assets.iter().find(|a| lower(a).ends_with(".dmg")));
            #[cfg(target_os = "windows")]
            let prefer = r.assets.iter().find(|a| {
                let n = lower(a);
                n.ends_with(".exe") && !n.contains("unst")
            });
            #[cfg(not(any(target_os = "macos", target_os = "windows")))]
            let prefer = r.assets.iter().find(|a| {
                let n = lower(a);
                n.ends_with(".dmg") || n.ends_with(".exe") || n.ends_with(".app.tar.gz")
            });
            prefer
        };
        let Some(asset) = asset else { continue };
        let sums = r.assets.iter().find(|a| a.name == "SHA256SUMS").map(|a| a.browser_download_url.clone());
        let sig_url = signature_url(&r.assets, &asset.name);
        let cand = AddonRelease {
            tag: r.tag_name.clone(),
            version: ver.clone(),
            zip_name: asset.name.clone(),
            zip_url: asset.browser_download_url.clone(),
            sums_url: sums,
            sig_url,
        };
        if best.as_ref().map(|b| &cand.version > &b.version).unwrap_or(true) { best = Some(cand); }
    }
    Ok(best)
}

/// Wait until `cmd` exits or `budget` elapses. On expiry the child is killed.
/// `None` means it did not finish in time (or could not be spawned).
pub fn command_output_within(mut cmd: Command, budget: Duration) -> Option<std::process::Output> {
    cmd.stdout(Stdio::piped()).stderr(Stdio::null());
    let mut child = cmd.spawn().ok()?;
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => return child.wait_with_output().ok(),
            Ok(None) if start.elapsed() >= budget => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(30)),
            Err(_) => return None,
        }
    }
}

/// Heuristic: a process named like the client is running (for the /reload warning only).
/// The check is killed if it exceeds 2 seconds. It never opens the game.
pub fn game_seems_running() -> bool {
    let budget = Duration::from_secs(2);
    #[cfg(target_os = "windows")]
    {
        let cmd = Command::new("tasklist");
        command_output_within(cmd, budget)
            .map(|o| {
                let s = String::from_utf8_lossy(&o.stdout).to_ascii_lowercase();
                s.contains("wow.exe") || s.contains("wowclassic.exe")
            })
            .unwrap_or(false)
    }
    #[cfg(not(target_os = "windows"))]
    {
        let mut cmd = Command::new("pgrep");
        cmd.args(["-if", "Wow|World of Warcraft"]);
        command_output_within(cmd, budget).map(|o| o.status.success()).unwrap_or(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;
    use std::io::Write;

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
    fn sums_parse() {
        let s = "abc123  Addon-0.1.1.zip\ndef  other\n";
        assert_eq!(sums_entry(s, "Addon-0.1.1.zip").as_deref(), Some("abc123"));
    }

    #[test]
    fn command_budget_kills_a_slow_child() {
        let start = Instant::now();
        let mut cmd = Command::new("sleep");
        cmd.arg("30");
        let out = command_output_within(cmd, Duration::from_millis(250));
        let elapsed = start.elapsed();
        assert!(out.is_none(), "sleep should be killed");
        assert!(elapsed < Duration::from_secs(2), "budget elapsed {elapsed:?}");
        eprintln!("MEASURE command_budget elapsed_ms={}", elapsed.as_millis());
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

    #[test]
    fn game_check_on_this_machine_is_bounded() {
        let start = Instant::now();
        let _running = game_seems_running();
        let elapsed = start.elapsed();
        assert!(elapsed < Duration::from_secs(3), "game check elapsed {elapsed:?}");
        eprintln!("MEASURE game_seems_running elapsed_ms={}", elapsed.as_millis());
    }
}
