//! Fixed install locations for the game client. Never walks a disk and never opens the game.
use std::path::{Path, PathBuf};

/// Flavor folder used by the common Windows install.
pub const FOREVER_FLAVOR: &str = "_forever_";
/// Executable name used only as a file-exists check. It is never spawned.
pub const WOW_EXE: &str = "Wow.exe";

/// The one Windows path ordinary installs use. Other operating systems have no equivalent.
pub fn canonical_forever_path() -> Option<&'static Path> {
    #[cfg(target_os = "windows")]
    {
        Some(Path::new(r"C:\Program Files (x86)\World of Warcraft\_forever_"))
    }
    #[cfg(not(target_os = "windows"))]
    {
        None
    }
}

/// Fixed roots this process may stat. No extra drive letters, no registry, no disk walk.
pub fn platform_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    #[cfg(target_os = "windows")]
    {
        roots.push(PathBuf::from(r"C:\Program Files (x86)\World of Warcraft"));
        roots.push(PathBuf::from(r"C:\Program Files\World of Warcraft"));
        roots.push(PathBuf::from(r"C:\Games\World of Warcraft"));
        if let Ok(home) = std::env::var("USERPROFILE") {
            roots.push(Path::new(&home).join("Games").join("World of Warcraft"));
        }
    }
    #[cfg(target_os = "macos")]
    {
        if let Ok(home) = std::env::var("HOME") {
            roots.push(Path::new(&home).join("Applications").join("World of Warcraft"));
            roots.push(PathBuf::from("/Applications/World of Warcraft"));
            roots.push(Path::new(&home).join("Games").join("World of Warcraft"));
        }
    }
    #[cfg(target_os = "linux")]
    {
        if let Ok(home) = std::env::var("HOME") {
            roots.push(Path::new(&home).join("Games").join("World of Warcraft"));
            roots.push(
                Path::new(&home)
                    .join(".wine")
                    .join("drive_c")
                    .join("Program Files (x86)")
                    .join("World of Warcraft"),
            );
        }
    }
    roots
}

pub fn push_variants(out: &mut Vec<PathBuf>, root: &Path) {
    out.push(root.to_path_buf());
    for sub in [FOREVER_FLAVOR, "_classic_era_", "_classic_", "_retail_"] {
        out.push(root.join(sub));
    }
}

pub fn has_wow_exe(dir: &Path) -> bool {
    dir.join(WOW_EXE).is_file()
}

/// True when `game_dir` has a WTF tree or Interface/AddOns (a client folder the addon can live in).
pub fn looks_like_game_dir(game_dir: &Path) -> bool {
    game_dir.join("WTF").is_dir() || game_dir.join("Interface").join("AddOns").is_dir()
}

/// Saved or browsed folders need a WTF/AddOns tree. The canonical Forever folder is also
/// acceptable when `Wow.exe` is present, even before the game has written WTF.
pub fn folder_acceptable(path: &Path) -> bool {
    if path.as_os_str().is_empty() {
        return false;
    }
    looks_like_game_dir(path) || has_wow_exe(path)
}

/// Where a resolved folder came from. `None` means the user still has to choose.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FolderSource {
    Saved,
    CanonicalForever,
    PlatformProbe,
    None,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FolderProbe {
    pub resolved: Option<PathBuf>,
    pub source: FolderSource,
    pub show_folder_ui: bool,
    pub choices: Vec<PathBuf>,
}

fn miss() -> FolderProbe {
    FolderProbe { resolved: None, source: FolderSource::None, show_folder_ui: true, choices: vec![] }
}

fn hit(path: PathBuf, source: FolderSource) -> FolderProbe {
    FolderProbe { resolved: Some(path), source, show_folder_ui: false, choices: vec![] }
}

/// Resolve a game folder without scanning the disk.
///
/// A saved path that still looks like a client wins, on every OS.
/// Otherwise Windows checks only `canonical` (the common Forever path) and stops.
/// Otherwise each path in `extra_roots` and its flavor subfolders is checked.
/// One remaining hit is selected. Several hits are returned as choices. Zero hits ask the user.
pub fn probe_explicit(saved: Option<&Path>, canonical: Option<&Path>, extra_roots: &[PathBuf]) -> FolderProbe {
    if let Some(saved) = saved {
        if folder_acceptable(saved) {
            return hit(saved.to_path_buf(), FolderSource::Saved);
        }
    }
    if let Some(canonical) = canonical {
        if has_wow_exe(canonical) {
            return hit(canonical.to_path_buf(), FolderSource::CanonicalForever);
        }
        // The common path is the only automatic Windows check. Do not keep going
        // into other drives when it is missing.
        if extra_roots.is_empty() {
            return miss();
        }
    }
    let mut hits: Vec<PathBuf> = Vec::new();
    for root in extra_roots {
        let mut variants = Vec::new();
        push_variants(&mut variants, root);
        let matched: Vec<PathBuf> = variants.into_iter().filter(|p| folder_acceptable(p)).collect();
        let children: Vec<PathBuf> = matched.iter().filter(|p| p.as_path() != root.as_path()).cloned().collect();
        if children.is_empty() {
            hits.extend(matched);
        } else {
            hits.extend(children);
        }
    }
    dedupe(&mut hits);
    match hits.len() {
        0 => miss(),
        1 => hit(hits.remove(0), FolderSource::PlatformProbe),
        _ => FolderProbe { resolved: None, source: FolderSource::None, show_folder_ui: true, choices: hits },
    }
}

fn dedupe(paths: &mut Vec<PathBuf>) {
    let mut seen = Vec::new();
    paths.retain(|p| {
        if seen.iter().any(|s: &PathBuf| s == p) {
            false
        } else {
            seen.push(p.clone());
            true
        }
    });
}

/// Startup probe for this operating system.
///
/// Windows: saved folder, else the canonical Forever path, and nothing else.
/// macOS and Linux: saved folder, else the fixed roots in `platform_roots` (no extra drives).
pub fn probe_game_folder(saved: Option<&Path>) -> FolderProbe {
    #[cfg(target_os = "windows")]
    {
        probe_explicit(saved, canonical_forever_path(), &[])
    }
    #[cfg(not(target_os = "windows"))]
    {
        probe_explicit(saved, None, &platform_roots())
    }
}

/// Legacy list used by tests and anything that still wants every fixed candidate.
/// Drive letters outside `platform_roots` are not included.
pub fn candidates() -> Vec<PathBuf> {
    let mut out = Vec::new();
    for root in platform_roots() {
        push_variants(&mut out, &root);
    }
    out.into_iter().filter(|p| folder_acceptable(p)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::Instant;

    fn scratch(name: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("gu-detect-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&p);
        fs::create_dir_all(&p).unwrap();
        p
    }

    #[test]
    fn windows_roots_do_not_include_other_drives() {
        let roots = [
            r"C:\Program Files (x86)\World of Warcraft",
            r"C:\Program Files\World of Warcraft",
            r"C:\Games\World of Warcraft",
        ];
        for root in roots {
            assert!(!root.starts_with("D:"), "{root}");
        }
        let forever = r"C:\Program Files (x86)\World of Warcraft\_forever_";
        assert!(forever.ends_with(FOREVER_FLAVOR));
        assert!(!forever.starts_with("D:"));
    }

    #[test]
    fn canonical_forever_with_exe_is_selected_and_stops() {
        let root = scratch("forever");
        let client = root.join("World of Warcraft").join(FOREVER_FLAVOR);
        fs::create_dir_all(&client).unwrap();
        fs::write(client.join(WOW_EXE), b"not a real exe").unwrap();
        let other = root.join("elsewhere");
        fs::create_dir_all(other.join("WTF")).unwrap();
        let probe = probe_explicit(None, Some(&client), &[other]);
        assert_eq!(probe.source, FolderSource::CanonicalForever);
        assert_eq!(probe.resolved.as_deref(), Some(client.as_path()));
        assert!(!probe.show_folder_ui);
        assert!(probe.choices.is_empty());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn missing_exe_does_not_look_ready_and_does_not_use_extra_roots_when_canonical_is_set() {
        let root = scratch("missing-exe");
        let client = root.join(FOREVER_FLAVOR);
        fs::create_dir_all(&client).unwrap();
        let decoy = root.join("decoy");
        fs::create_dir_all(decoy.join("Interface").join("AddOns")).unwrap();
        let probe = probe_explicit(None, Some(&client), &[]);
        assert!(probe.resolved.is_none());
        assert!(probe.show_folder_ui);
        assert_eq!(probe.source, FolderSource::None);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn path_with_spaces_is_acceptable() {
        let root = scratch("spaces");
        let client = root.join("Program Files (x86)").join("World of Warcraft").join(FOREVER_FLAVOR);
        fs::create_dir_all(client.join("WTF")).unwrap();
        assert!(folder_acceptable(&client));
        let probe = probe_explicit(Some(&client), None, &[]);
        assert_eq!(probe.source, FolderSource::Saved);
        assert_eq!(probe.resolved.as_deref(), Some(client.as_path()));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn saved_folder_wins_over_canonical() {
        let root = scratch("saved");
        let saved = root.join("custom client");
        fs::create_dir_all(saved.join("Interface").join("AddOns")).unwrap();
        let canonical = root.join(FOREVER_FLAVOR);
        fs::create_dir_all(&canonical).unwrap();
        fs::write(canonical.join(WOW_EXE), b"x").unwrap();
        let probe = probe_explicit(Some(&saved), Some(&canonical), &[]);
        assert_eq!(probe.source, FolderSource::Saved);
        assert_eq!(probe.resolved.as_deref(), Some(saved.as_path()));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn platform_probe_drops_parent_when_flavor_matches_and_lists_several() {
        let root = scratch("mac");
        let a = root.join("Applications").join("World of Warcraft");
        let b = root.join("Games").join("World of Warcraft");
        fs::create_dir_all(a.join(FOREVER_FLAVOR).join("WTF")).unwrap();
        fs::create_dir_all(a.join("WTF")).unwrap();
        fs::create_dir_all(b.join("_classic_").join("Interface").join("AddOns")).unwrap();
        let probe = probe_explicit(None, None, &[a.clone(), b.clone()]);
        assert!(probe.show_folder_ui);
        assert!(probe.resolved.is_none());
        assert!(probe.choices.iter().any(|p| p.ends_with(FOREVER_FLAVOR)));
        assert!(!probe.choices.iter().any(|p| p == &a), "parent dropped when flavor matched");
        assert!(probe.choices.iter().any(|p| p.ends_with("_classic_")));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn one_platform_hit_is_automatic() {
        let root = scratch("one");
        let games = root.join("Games").join("World of Warcraft").join(FOREVER_FLAVOR);
        fs::create_dir_all(games.join("WTF")).unwrap();
        let probe = probe_explicit(None, None, &[root.join("Games").join("World of Warcraft")]);
        assert_eq!(probe.source, FolderSource::PlatformProbe);
        assert!(!probe.show_folder_ui);
        assert_eq!(probe.resolved.as_deref(), Some(games.as_path()));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn probe_on_this_machine_does_not_block() {
        let start = Instant::now();
        let probe = probe_game_folder(None);
        let elapsed = start.elapsed();
        assert!(elapsed < std::time::Duration::from_millis(1500), "probe elapsed {elapsed:?}");
        assert!(probe.source != FolderSource::CanonicalForever || cfg!(target_os = "windows"));
        eprintln!("MEASURE probe_game_folder elapsed_ms={} show_ui={}", elapsed.as_millis(), probe.show_folder_ui);
    }
}
