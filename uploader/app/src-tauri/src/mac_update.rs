#![cfg_attr(not(target_os = "macos"), allow(dead_code))]
//! macOS Update now: the release's `.app.tar.gz` (already SHA-256 and Ed25519 checked by
//! `app_update::accept_verified`) replaces the running `.app` in place, then the app restarts.
//! A `.dmg` can't be "run" as an installer, so it is not used for updates.
//!
//! The swap is two renames in the bundle's own folder, so it is atomic per step and is undone if
//! the second rename fails. The functions here are plain file operations and are tested on every
//! platform; only `current_bundle` and the restart are macOS-specific.
use std::fs;
use std::io::Read;
use std::path::{Component, Path, PathBuf};

/// The `.app` that contains `exe` (`X.app/Contents/MacOS/<bin>`).
pub fn bundle_of(exe: &Path) -> Option<PathBuf> {
    let macos = exe.parent()?;
    let contents = macos.parent()?;
    let app = contents.parent()?;
    let ok = macos.file_name()? == "MacOS" && contents.file_name()? == "Contents" && app.extension()? == "app";
    ok.then(|| app.to_path_buf())
}

/// The running app's bundle.
pub fn current_bundle() -> Result<PathBuf, String> {
    let exe = std::env::current_exe().map_err(|e| format!("couldn't find the running app: {e}"))?;
    let exe = exe.canonicalize().unwrap_or(exe);
    bundle_of(&exe).ok_or_else(|| "the app isn't running from an .app bundle, so it can't replace itself".to_string())
}

fn entry_path_ok(p: &Path) -> bool {
    !p.as_os_str().is_empty() && !p.is_absolute() && p.components().all(|c| matches!(c, Component::Normal(_) | Component::CurDir))
}

/// Unpack `archive` (gzip tar holding one `<Name>.app`) into `dest`. Returns the unpacked `.app`.
/// Refuses absolute or `..` paths, links that point outside the archive, and anything other than
/// one top-level `.app` that has `Contents/Info.plist` and a `Contents/MacOS` binary.
pub fn unpack_app(archive: &[u8], dest: &Path) -> Result<PathBuf, String> {
    let gz = flate2::read::GzDecoder::new(archive);
    let mut tar = tar::Archive::new(gz.take(512 * 1024 * 1024));
    tar.set_preserve_permissions(true);
    tar.set_overwrite(false);
    let mut top: Option<PathBuf> = None;
    for entry in tar.entries().map_err(|e| format!("the update archive is unreadable: {e}"))? {
        let mut entry = entry.map_err(|e| format!("the update archive is unreadable: {e}"))?;
        let path = entry.path().map_err(|e| format!("bad path in the update archive: {e}"))?.into_owned();
        if !entry_path_ok(&path) {
            return Err(format!("the update archive has an unsafe path: {}", path.display()));
        }
        let first = path.components().find_map(|c| match c { Component::Normal(n) => Some(PathBuf::from(n)), _ => None });
        let Some(first) = first else { continue };
        if first.extension().map(|e| e != "app").unwrap_or(true) {
            return Err(format!("the update archive has {} outside the app", path.display()));
        }
        match &top {
            None => top = Some(first.clone()),
            Some(t) if *t != first => return Err("the update archive holds more than one app".into()),
            _ => {}
        }
        let kind = entry.header().entry_type();
        if kind.is_symlink() || kind.is_hard_link() {
            let target = entry.link_name().map_err(|e| e.to_string())?.ok_or("link without a target")?.into_owned();
            let base = path.parent().unwrap_or(Path::new(""));
            if target.is_absolute() || !stays_inside(base, &target) {
                return Err(format!("the update archive has a link that leaves the app: {}", path.display()));
            }
        } else if !(kind.is_file() || kind.is_dir()) {
            return Err(format!("the update archive has an unexpected entry: {}", path.display()));
        }
        if !entry.unpack_in(dest).map_err(|e| format!("couldn't unpack {}: {e}", path.display()))? {
            return Err(format!("the update archive has an unsafe path: {}", path.display()));
        }
    }
    let app = dest.join(top.ok_or("the update archive is empty")?);
    if !app.join("Contents").join("Info.plist").is_file() {
        return Err("the update archive's app has no Info.plist".into());
    }
    let has_bin = fs::read_dir(app.join("Contents").join("MacOS")).map(|mut d| d.any(|e| e.map(|e| e.path().is_file()).unwrap_or(false))).unwrap_or(false);
    if !has_bin {
        return Err("the update archive's app has no program in Contents/MacOS".into());
    }
    Ok(app)
}

/// `base` joined with a relative `target` never climbs above the archive root.
fn stays_inside(base: &Path, target: &Path) -> bool {
    let mut depth: i64 = base.components().filter(|c| matches!(c, Component::Normal(_))).count() as i64;
    for c in target.components() {
        match c {
            Component::Normal(_) => depth += 1,
            Component::ParentDir => {
                depth -= 1;
                if depth < 1 { return false }
            }
            Component::CurDir => {}
            _ => return false,
        }
    }
    true
}

/// Replace `app` with the app in `archive`. Unpacks next to `app` (same volume), moves the old
/// bundle aside, moves the new one in, and deletes the old one. Any failure leaves `app` as it was.
pub fn replace_bundle(app: &Path, archive: &[u8]) -> Result<(), String> {
    let parent = app.parent().ok_or("the app has no parent folder")?;
    let name = app.file_name().ok_or("the app has no name")?.to_string_lossy().into_owned();
    let stage = parent.join(format!(".{name}.update-{}", std::process::id()));
    let _ = fs::remove_dir_all(&stage);
    fs::create_dir(&stage).map_err(|e| format!("couldn't write next to the app in {} ({e}). Move the app to a folder you can change, or install the update from the .dmg", parent.display()))?;
    let result = (|| {
        let fresh = unpack_app(archive, &stage)?;
        let old = parent.join(format!(".{name}.old-{}", std::process::id()));
        let _ = fs::remove_dir_all(&old);
        fs::rename(app, &old).map_err(|e| format!("couldn't move the running app aside: {e}"))?;
        if let Err(e) = fs::rename(&fresh, app) {
            let _ = fs::rename(&old, app);
            return Err(format!("couldn't put the new app in place, so the old one was put back: {e}"));
        }
        let _ = fs::remove_dir_all(&old);
        Ok(())
    })();
    let _ = fs::remove_dir_all(&stage);
    result
}

/// The production install step for macOS: replace the running bundle.
pub fn install_verified_archive(bytes: &[u8]) -> Result<(), String> {
    if bytes.is_empty() {
        return Err("the verified update was empty".into());
    }
    let app = current_bundle()?;
    guild_uploader_core::log::info("app-update", &format!("replacing {}", app.display()));
    replace_bundle(&app, bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn archive(entries: &[(&str, Option<&[u8]>, Option<&str>)]) -> Vec<u8> {
        let mut out = Vec::new();
        {
            let gz = flate2::write::GzEncoder::new(&mut out, flate2::Compression::fast());
            let mut b = tar::Builder::new(gz);
            for (path, data, link) in entries {
                let mut h = tar::Header::new_gnu();
                if let Some(target) = link {
                    h.set_entry_type(tar::EntryType::Symlink);
                    h.set_size(0);
                    h.set_mode(0o755);
                    b.append_link(&mut h, path, target).unwrap();
                } else if let Some(d) = data {
                    h.set_size(d.len() as u64);
                    h.set_mode(0o755);
                    h.set_cksum();
                    b.append_data(&mut h, path, *d).unwrap();
                } else {
                    h.set_entry_type(tar::EntryType::Directory);
                    h.set_size(0);
                    h.set_mode(0o755);
                    h.set_cksum();
                    b.append_data(&mut h, path, &[][..]).unwrap();
                }
            }
            b.into_inner().unwrap().finish().unwrap();
        }
        out
    }

    fn good(version: &[u8]) -> Vec<u8> {
        archive(&[
            ("Example.app/Contents/Info.plist", Some(b"<plist/>"), None),
            ("Example.app/Contents/MacOS/Example", Some(version), None),
            ("Example.app/Contents/Resources/icon.icns", Some(b"i"), None),
        ])
    }

    fn installed(dir: &Path, version: &[u8]) -> PathBuf {
        let app = dir.join("Example.app");
        fs::create_dir_all(app.join("Contents/MacOS")).unwrap();
        fs::write(app.join("Contents/Info.plist"), b"<plist/>").unwrap();
        fs::write(app.join("Contents/MacOS/Example"), version).unwrap();
        app
    }

    #[test]
    fn bundle_is_found_from_the_executable() {
        assert_eq!(bundle_of(Path::new("/Applications/Example.app/Contents/MacOS/Example")), Some(PathBuf::from("/Applications/Example.app")));
        assert_eq!(bundle_of(Path::new("/usr/local/bin/example")), None);
    }

    #[test]
    fn replaces_the_bundle_and_cleans_up() {
        let dir = tempfile::tempdir().unwrap();
        let app = installed(dir.path(), b"old");
        fs::write(app.join("Contents/MacOS/stale"), b"x").unwrap();
        replace_bundle(&app, &good(b"new")).unwrap();
        assert_eq!(fs::read(app.join("Contents/MacOS/Example")).unwrap(), b"new");
        assert!(!app.join("Contents/MacOS/stale").exists(), "old files don't linger");
        let left: Vec<_> = fs::read_dir(dir.path()).unwrap().map(|e| e.unwrap().file_name()).collect();
        assert_eq!(left, vec![std::ffi::OsString::from("Example.app")]);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(app.join("Contents/MacOS/Example")).unwrap().permissions().mode();
            assert!(mode & 0o111 != 0, "program stays executable");
        }
    }

    #[test]
    fn bad_archives_leave_the_app_untouched() {
        let cases: Vec<Vec<u8>> = vec![
            b"not gzip".to_vec(),
            archive(&[("Example.app/Contents/MacOS/Example", Some(b"new"), None)]),
            archive(&[("Example.app/Contents/Info.plist", Some(b"p"), None), ("Other.app/Contents/MacOS/x", Some(b"x"), None)]),
            archive(&[("Example.app/Contents/Info.plist", Some(b"p"), None), ("Example.app/Contents/MacOS/Example", Some(b"n"), None), ("Example.app/Contents/l", None, Some("/etc/passwd"))]),
            archive(&[("Example.app/Contents/Info.plist", Some(b"p"), None), ("Example.app/Contents/MacOS/Example", Some(b"n"), None), ("Example.app/l", None, Some("../../outside"))]),
            archive(&[("loose.txt", Some(b"x"), None)]),
        ];
        for (i, bytes) in cases.iter().enumerate() {
            let dir = tempfile::tempdir().unwrap();
            let app = installed(dir.path(), b"old");
            assert!(replace_bundle(&app, bytes).is_err(), "case {i} was accepted");
            assert_eq!(fs::read(app.join("Contents/MacOS/Example")).unwrap(), b"old", "case {i}");
            let left: Vec<_> = fs::read_dir(dir.path()).unwrap().map(|e| e.unwrap().file_name()).collect();
            assert_eq!(left.len(), 1, "case {i} left {left:?}");
        }
    }

    #[test]
    fn links_inside_the_app_are_kept() {
        let dir = tempfile::tempdir().unwrap();
        let app = installed(dir.path(), b"old");
        let a = archive(&[
            ("Example.app/Contents/Info.plist", Some(b"<plist/>"), None),
            ("Example.app/Contents/MacOS/Example", Some(b"new"), None),
            ("Example.app/Contents/Frameworks/X.framework/Versions/A/X", Some(b"lib"), None),
            ("Example.app/Contents/Frameworks/X.framework/X", None, Some("Versions/A/X")),
        ]);
        replace_bundle(&app, &a).unwrap();
        #[cfg(unix)]
        assert_eq!(fs::read(app.join("Contents/Frameworks/X.framework/X")).unwrap(), b"lib");
    }
}
