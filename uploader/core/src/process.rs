//! Child processes and the game-running check.
//!
//! The desktop app does not launch a user-installed CLI. Game detection reads
//! the OS process list in this process. The only helper that starts a program
//! is `background_command`, used for the installer the user already accepted.
//! On Windows that helper sets CREATE_NO_WINDOW so a console is not created.
//! It does not hide a GUI program's own windows.

use std::ffi::OsStr;
use std::process::{Command, Stdio};

/// `CREATE_NO_WINDOW` from the Windows process creation flags.
pub const CREATE_NO_WINDOW: u32 = 0x0800_0000;

pub fn background_command(program: impl AsRef<OsStr>) -> Command {
    let mut cmd = Command::new(program);
    cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd
}

/// True when a label from the process list looks like the game client.
/// This never opens or signals that process.
pub fn is_game_process(label: &str) -> bool {
    let n = label.to_ascii_lowercase();
    n.contains("wow.exe") || n.contains("wowclassic.exe") || n.contains("world of warcraft")
}

pub fn game_seems_running() -> bool {
    process_labels().iter().any(|label| is_game_process(label))
}

#[cfg(windows)]
fn process_labels() -> Vec<String> {
    use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
    };
    let mut names = Vec::new();
    unsafe {
        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snap == INVALID_HANDLE_VALUE {
            return names;
        }
        let mut entry: PROCESSENTRY32W = std::mem::zeroed();
        entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
        if Process32FirstW(snap, &mut entry) != 0 {
            loop {
                let len = entry.szExeFile.iter().position(|c| *c == 0).unwrap_or(entry.szExeFile.len());
                if let Ok(name) = String::from_utf16(&entry.szExeFile[..len]) {
                    if !name.is_empty() {
                        names.push(name);
                    }
                }
                if Process32NextW(snap, &mut entry) == 0 {
                    break;
                }
            }
        }
        let _ = CloseHandle(snap);
    }
    names
}

/// macOS has no /proc. libproc (part of libSystem, called in this process) lists process ids and
/// each process's short name and executable path, the same names Activity Monitor shows. No CLI
/// such as `ps` is started, and the game process itself is never opened or signalled.
#[cfg(target_os = "macos")]
fn process_labels() -> Vec<String> {
    use std::os::raw::{c_int, c_void};
    const PATH_MAX: usize = 4096; // PROC_PIDPATHINFO_MAXSIZE is 4 * MAXPATHLEN
    let mut names = Vec::new();
    // First call with a null buffer returns the current process count.
    let count = unsafe { libc::proc_listallpids(std::ptr::null_mut(), 0) };
    if count <= 0 {
        return names;
    }
    // Leave room for processes started between the two calls.
    let cap = count as usize + 64;
    let mut pids: Vec<c_int> = vec![0; cap];
    let bytes = (cap * std::mem::size_of::<c_int>()) as c_int;
    let n = unsafe { libc::proc_listallpids(pids.as_mut_ptr() as *mut c_void, bytes) };
    if n <= 0 {
        return names;
    }
    pids.truncate((n as usize).min(cap));
    let mut buf = vec![0u8; PATH_MAX];
    for pid in pids.into_iter().filter(|p| *p > 0) {
        let len = unsafe { libc::proc_name(pid, buf.as_mut_ptr() as *mut c_void, buf.len() as u32) };
        if len > 0 {
            let name = String::from_utf8_lossy(&buf[..len as usize]).trim().to_string();
            if !name.is_empty() {
                names.push(name);
            }
        }
        let len = unsafe { libc::proc_pidpath(pid, buf.as_mut_ptr() as *mut c_void, buf.len() as u32) };
        if len > 0 {
            let path = String::from_utf8_lossy(&buf[..len as usize]).trim().to_string();
            if !path.is_empty() {
                names.push(path);
            }
        }
    }
    names
}

#[cfg(not(any(windows, target_os = "macos")))]
fn process_labels() -> Vec<String> {
    let mut names = Vec::new();
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return names;
    };
    for entry in entries.flatten() {
        let Some(pid) = entry.file_name().to_str().map(str::to_string) else { continue };
        if !pid.bytes().all(|b| b.is_ascii_digit()) {
            continue;
        }
        let path = entry.path();
        if let Ok(comm) = std::fs::read_to_string(path.join("comm")) {
            let comm = comm.trim();
            if !comm.is_empty() {
                names.push(comm.to_string());
            }
        }
        if let Ok(bytes) = std::fs::read(path.join("cmdline")) {
            let text = String::from_utf8_lossy(&bytes).replace('\0', " ");
            let text = text.trim();
            if !text.is_empty() {
                names.push(text.to_string());
            }
        }
    }
    names
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    #[test]
    fn game_image_names_match_and_this_app_does_not() {
        assert!(is_game_process("Wow.exe"));
        assert!(is_game_process("WowClassic.exe"));
        assert!(is_game_process("C:\\Games\\World of Warcraft\\Wow.exe"));
        assert!(is_game_process("World of Warcraft"));
        assert!(is_game_process("/Applications/World of Warcraft/_classic_/World of Warcraft Classic.app/Contents/MacOS/World of Warcraft Classic"));
        assert!(!is_game_process("guild-uploader-app.exe"));
        assert!(!is_game_process("Bloodfang Uploader.exe"));
        assert!(!is_game_process("gh.exe"));
        assert!(!is_game_process("tasklist.exe"));
        assert!(!is_game_process("explorer.exe"));
    }

    #[test]
    fn create_no_window_is_the_windows_flag() {
        assert_eq!(CREATE_NO_WINDOW, 0x0800_0000);
    }

    #[test]
    fn game_check_on_this_machine_is_bounded() {
        let start = Instant::now();
        let _running = game_seems_running();
        // The list itself must work on every desktop OS (it was empty on macOS before 0.1.4).
        assert!(!process_labels().is_empty(), "this test's own process is in the list");
        let elapsed = start.elapsed();
        assert!(elapsed < std::time::Duration::from_secs(3), "game check elapsed {elapsed:?}");
    }
}
