//! Common WoW: Forever / client install folders. Never touches a running process.
use std::path::{Path, PathBuf};

/// Candidate game folders that look like a Forever (or classic-era) client with a WTF tree.
pub fn candidates() -> Vec<PathBuf> {
    let mut out = vec![];
    #[cfg(target_os = "windows")]
    {
        for root in [
            r"C:\Program Files (x86)\World of Warcraft",
            r"C:\Program Files\World of Warcraft",
            r"C:\Games\World of Warcraft",
            r"D:\World of Warcraft",
            r"D:\Games\World of Warcraft",
        ] {
            push_variants(&mut out, Path::new(root));
        }
        if let Ok(home) = std::env::var("USERPROFILE") {
            push_variants(&mut out, &Path::new(&home).join("Games").join("World of Warcraft"));
        }
    }
    #[cfg(target_os = "macos")]
    {
        if let Ok(home) = std::env::var("HOME") {
            push_variants(&mut out, &Path::new(&home).join("Applications").join("World of Warcraft"));
            push_variants(&mut out, Path::new("/Applications/World of Warcraft"));
            push_variants(&mut out, &Path::new(&home).join("Games").join("World of Warcraft"));
        }
    }
    #[cfg(target_os = "linux")]
    {
        if let Ok(home) = std::env::var("HOME") {
            push_variants(&mut out, &Path::new(&home).join("Games").join("World of Warcraft"));
            push_variants(&mut out, &Path::new(&home).join(".wine").join("drive_c").join("Program Files (x86)").join("World of Warcraft"));
        }
    }
    out.into_iter().filter(|p| p.join("WTF").is_dir() || p.join("Interface").is_dir()).collect()
}

fn push_variants(out: &mut Vec<PathBuf>, root: &Path) {
    // Forever and classic-era flavours live as subfolders; also accept the root itself.
    out.push(root.to_path_buf());
    for sub in ["_classic_era_", "_classic_", "_retail_", "WTF"] {
        let p = root.join(sub);
        if sub != "WTF" { out.push(p); }
    }
}

/// True when `game_dir` has Interface/AddOns or WTF (a plausible client folder).
pub fn looks_like_game_dir(game_dir: &Path) -> bool {
    game_dir.join("WTF").is_dir() || game_dir.join("Interface").join("AddOns").is_dir()
}
