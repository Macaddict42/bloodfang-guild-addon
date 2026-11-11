//! Rotating log file for support (`uploader.log`, then `uploader.1.log`, `uploader.2.log`).
//!
//! Each line starts with a UTC timestamp. Every line is redacted before it is written: uploader
//! tokens, bearer values, OAuth codes and verifiers, cookies, signatures and signed URL parameters
//! are replaced, and the user's home folder name is replaced with `<user>`. The log never needs
//! the raw value of any of these, so redacting at write time keeps them off disk entirely.
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

pub const FILE: &str = "uploader.log";
/// Rotate when the current file would pass this size.
pub const MAX_BYTES: u64 = 1024 * 1024;
/// Current file plus this many older ones.
pub const KEEP_OLD: usize = 2;

struct Sink {
    dir: PathBuf,
    max: u64,
    home: Option<String>,
}

static SINK: Mutex<Option<Sink>> = Mutex::new(None);

/// `%LOCALAPPDATA%\<product>\logs` on Windows, `~/Library/Logs/<product>` on macOS,
/// `$XDG_STATE_HOME`/`~/.local/state` `/<product>/logs` elsewhere.
pub fn default_dir(product: &str) -> Option<PathBuf> {
    #[cfg(windows)]
    {
        std::env::var_os("LOCALAPPDATA").map(|d| PathBuf::from(d).join(product).join("logs"))
    }
    #[cfg(target_os = "macos")]
    {
        std::env::var_os("HOME").map(|h| PathBuf::from(h).join("Library").join("Logs").join(product))
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        std::env::var_os("XDG_STATE_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local").join("state")))
            .map(|d| d.join(product).join("logs"))
    }
}

fn home_dir() -> Option<String> {
    #[cfg(windows)]
    let h = std::env::var("USERPROFILE").ok();
    #[cfg(not(windows))]
    let h = std::env::var("HOME").ok();
    h.filter(|h| h.len() > 3)
}

/// Start logging into `dir`. Safe to call again (the newest call wins).
pub fn init(dir: &Path) -> std::io::Result<()> {
    init_with(dir, MAX_BYTES, home_dir())
}

pub fn init_with(dir: &Path, max: u64, home: Option<String>) -> std::io::Result<()> {
    fs::create_dir_all(dir)?;
    let mut g = SINK.lock().unwrap_or_else(|e| e.into_inner());
    *g = Some(Sink { dir: dir.to_path_buf(), max, home });
    Ok(())
}

pub fn dir() -> Option<PathBuf> {
    SINK.lock().ok().and_then(|g| g.as_ref().map(|s| s.dir.clone()))
}

pub fn info(area: &str, msg: &str) { write("INFO", area, msg) }
pub fn warn(area: &str, msg: &str) { write("WARN", area, msg) }
pub fn error(area: &str, msg: &str) { write("ERROR", area, msg) }

fn write(level: &str, area: &str, msg: &str) {
    let mut g = SINK.lock().unwrap_or_else(|e| e.into_inner());
    let Some(sink) = g.as_mut() else { return };
    let text = redact_with(msg, sink.home.as_deref()).replace(['\r', '\n'], " ");
    let line = format!("{} {level} {area}: {text}\n", utc_now());
    let _ = append(&sink.dir, sink.max, line.as_bytes());
}

fn path_n(dir: &Path, n: usize) -> PathBuf {
    if n == 0 { dir.join(FILE) } else { dir.join(format!("uploader.{n}.log")) }
}

/// Append one line, rotating first if it would push the file past `max`.
fn append(dir: &Path, max: u64, line: &[u8]) -> std::io::Result<()> {
    let cur = path_n(dir, 0);
    let size = fs::metadata(&cur).map(|m| m.len()).unwrap_or(0);
    if size > 0 && size + line.len() as u64 > max {
        rotate(dir)?;
    }
    let mut f = OpenOptions::new().create(true).append(true).open(&cur)?;
    f.write_all(line)
}

/// `uploader.log` → `.1` → `.2`; the oldest falls off.
pub fn rotate(dir: &Path) -> std::io::Result<()> {
    let _ = fs::remove_file(path_n(dir, KEEP_OLD));
    for n in (0..KEEP_OLD).rev() {
        let from = path_n(dir, n);
        if from.exists() {
            fs::rename(&from, path_n(dir, n + 1))?;
        }
    }
    Ok(())
}

/// The last `max` bytes of the log (older file first if the current one is short), starting
/// at a line boundary. Already redacted, because every line was redacted when written.
pub fn tail(max: usize) -> String {
    let Some(dir) = dir() else { return String::new() };
    tail_in(&dir, max)
}

pub fn tail_in(dir: &Path, max: usize) -> String {
    let mut chunks: Vec<Vec<u8>> = Vec::new();
    let mut left = max;
    for n in 0..=KEEP_OLD {
        if left == 0 { break }
        let Ok(mut f) = File::open(path_n(dir, n)) else { continue };
        let len = f.metadata().map(|m| m.len()).unwrap_or(0) as usize;
        let take = len.min(left);
        if f.seek(SeekFrom::Start((len - take) as u64)).is_err() { continue }
        let mut buf = Vec::with_capacity(take);
        if f.take(take as u64).read_to_end(&mut buf).is_err() { continue }
        left -= buf.len();
        chunks.push(buf);
    }
    chunks.reverse();
    let all: Vec<u8> = chunks.concat();
    let mut text = String::from_utf8_lossy(&all).into_owned();
    if all.len() >= max {
        // Cut to the first full line.
        if let Some(i) = text.find('\n') { text = text[i + 1..].to_string(); }
    }
    text
}

/// `2026-11-11T01:58:07Z`.
pub fn utc_now() -> String {
    let secs = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    utc_stamp(secs)
}

pub fn utc_stamp(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    // Howard Hinnant's civil_from_days.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + if m <= 2 { 1 } else { 0 };
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z", rem / 3600, rem % 3600 / 60, rem % 60)
}

/// Redact with this machine's home folder.
pub fn redact(text: &str) -> String { redact_with(text, home_dir().as_deref()) }

const QUERY_KEYS: &[&str] = &[
    "code", "state", "token", "access_token", "refresh_token", "id_token", "code_verifier", "code_challenge",
    "client_secret", "password", "secret", "sig", "signature", "x-amz-signature", "x-amz-credential",
    "x-amz-security-token", "jwt", "key", "se", "sp", "sv", "skoid", "sktid",
];
const JSON_KEYS: &[&str] = &[
    "token", "access_token", "refresh_token", "id_token", "code", "code_verifier", "client_secret", "password",
    "secret", "cookie", "signature", "sig", "authorization",
];

/// Replace secrets and the home folder name. Pure; see the tests for each shape.
pub fn redact_with(text: &str, home: Option<&str>) -> String {
    let mut s = text.to_string();
    // Uploader tokens anywhere.
    s = replace_after(&s, "upl_", |c| c.is_ascii_alphanumeric() || c == '_' || c == '-', "upl_[redacted]", true);
    // Authorization / cookie headers: everything to the end of the value.
    for h in ["authorization:", "cookie:", "set-cookie:", "x-api-key:"] {
        s = replace_header(&s, h);
    }
    s = replace_word_value(&s, "bearer ");
    s = replace_word_value(&s, "basic ");
    for k in QUERY_KEYS {
        s = replace_kv(&s, &format!("{k}="), &['&', ' ', '"', '\'', '#', ')', ']', '>', ',', ';']);
    }
    for k in JSON_KEYS {
        s = replace_json_string(&s, k);
    }
    redact_home(&s, home)
}

fn is_word_start(s: &str, i: usize) -> bool {
    i == 0 || !s[..i].chars().next_back().map(|c| c.is_ascii_alphanumeric() || c == '_').unwrap_or(false)
}

/// Case-insensitive find of `needle` (ASCII) at or after `from`.
fn find_ci(hay: &str, needle: &str, from: usize) -> Option<usize> {
    let h = hay.as_bytes();
    let n = needle.as_bytes();
    if n.is_empty() || h.len() < n.len() { return None }
    (from..=h.len() - n.len()).find(|&i| h[i..i + n.len()].eq_ignore_ascii_case(n) && hay.is_char_boundary(i))
}

fn replace_after(s: &str, prefix: &str, keep: impl Fn(char) -> bool, with: &str, word: bool) -> String {
    let mut out = String::new();
    let mut i = 0;
    while let Some(at) = find_ci(s, prefix, i) {
        let end_prefix = at + prefix.len();
        let val_len: usize = s[end_prefix..].chars().take_while(|c| keep(*c)).map(|c| c.len_utf8()).sum();
        if (word && !is_word_start(s, at)) || val_len == 0 || s[end_prefix..end_prefix + val_len].starts_with("[redacted]") {
            out.push_str(&s[i..end_prefix]);
            i = end_prefix;
            continue;
        }
        out.push_str(&s[i..at]);
        out.push_str(with);
        i = end_prefix + val_len;
    }
    out.push_str(&s[i..]);
    out
}

fn replace_header(s: &str, header: &str) -> String {
    let mut out = String::new();
    let mut i = 0;
    while let Some(at) = find_ci(s, header, i) {
        let start = at + header.len();
        let end = s[start..].find(['\n', '\r']).map(|e| start + e).unwrap_or(s.len());
        out.push_str(&s[i..start]);
        out.push_str(" [redacted]");
        i = end;
    }
    out.push_str(&s[i..]);
    out
}

fn replace_word_value(s: &str, word: &str) -> String {
    let cap = format!("{}{}", word[..1].to_uppercase(), &word[1..]);
    replace_after(s, word, |c| !c.is_whitespace() && c != '"' && c != '\'' && c != ',', &format!("{cap}[redacted]"), true)
}

fn replace_kv(s: &str, key_eq: &str, stops: &[char]) -> String {
    let mut out = String::new();
    let mut i = 0;
    while let Some(at) = find_ci(s, key_eq, i) {
        let start = at + key_eq.len();
        // Only a parameter name: preceded by ?, &, space, quote or start.
        let ok_before = at == 0 || matches!(s[..at].chars().next_back(), Some('?' | '&' | ' ' | '"' | '\'' | '(' | ',' | ';'));
        let end = s[start..].find(|c: char| stops.contains(&c) || c.is_whitespace()).map(|e| start + e).unwrap_or(s.len());
        if !ok_before || end == start || s[start..].starts_with("[redacted]") {
            out.push_str(&s[i..start]);
            i = start;
            continue;
        }
        out.push_str(&s[i..start]);
        out.push_str("[redacted]");
        i = end;
    }
    out.push_str(&s[i..]);
    out
}

/// `"key": "value"` → `"key": "[redacted]"`.
fn replace_json_string(s: &str, key: &str) -> String {
    let needle = format!("\"{key}\"");
    let mut out = String::new();
    let mut i = 0;
    while let Some(at) = find_ci(s, &needle, i) {
        let after = at + needle.len();
        let rest = &s[after..];
        let trimmed = rest.trim_start();
        let Some(colon_rest) = trimmed.strip_prefix(':') else {
            out.push_str(&s[i..after]);
            i = after;
            continue;
        };
        let val = colon_rest.trim_start();
        let val_at = s.len() - val.len();
        if let Some(body) = val.strip_prefix('"') {
            let mut end = None;
            let mut esc = false;
            for (k, c) in body.char_indices() {
                if esc { esc = false; continue }
                if c == '\\' { esc = true; continue }
                if c == '"' { end = Some(k); break }
            }
            let Some(end) = end else { out.push_str(&s[i..after]); i = after; continue };
            out.push_str(&s[i..val_at]);
            out.push_str("\"[redacted]\"");
            i = val_at + 1 + end + 1;
        } else {
            out.push_str(&s[i..after]);
            i = after;
        }
    }
    out.push_str(&s[i..]);
    out
}

/// Replace the home folder, and any `Users\<name>` / `/Users/<name>` / `/home/<name>` segment.
fn redact_home(s: &str, home: Option<&str>) -> String {
    let mut out = s.to_string();
    if let Some(h) = home {
        let h = h.trim_end_matches(['/', '\\']);
        if h.len() > 3 {
            for variant in [h.to_string(), h.replace('\\', "/"), h.replace('/', "\\")] {
                while let Some(at) = find_ci(&out, &variant, 0) {
                    let name_at = variant.rfind(['/', '\\']).map(|p| p + 1).unwrap_or(0);
                    let replacement = format!("{}<user>", &variant[..name_at]);
                    out.replace_range(at..at + variant.len(), &replacement);
                }
            }
        }
    }
    for (prefix, seps) in [("users\\", &['\\', '/', '"', '\'', ' ', ';', ')'][..]), ("users/", &['/', '\\', '"', '\'', ' ', ';', ')'][..]), ("/home/", &['/', '"', '\'', ' ', ';', ')'][..])] {
        let mut i = 0;
        while let Some(at) = find_ci(&out, prefix, i) {
            let start = at + prefix.len();
            let end = out[start..].find(|c: char| seps.contains(&c)).map(|e| start + e).unwrap_or(out.len());
            let name = &out[start..end];
            if name.is_empty() || name == "<user>" || name.eq_ignore_ascii_case("public") || name.eq_ignore_ascii_case("shared") {
                i = start;
                continue;
            }
            out.replace_range(start..end, "<user>");
            i = start + "<user>".len();
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_codes_cookies_and_signatures_are_redacted() {
        let raw = concat!(
            "POST /api/uploader/ingest Authorization: Bearer upl_AbCdEfGhIjKlMnOpQrStUvWxYz0123456789_-abcde\n",
            "callback http://127.0.0.1:5123/cb?code=SECRETCODE123&state=st4te done\n",
            "token body {\"access_token\":\"upl_zzz\",\"refresh_token\": \"r3fr\\\"esh\", \"type\":\"bearer\"}\n",
            "Cookie: session=abc; other=def\n",
            "get https://objects.example/x.zip?X-Amz-Signature=deadbeef&X-Amz-Credential=AKIA/x&response=ok\n",
            "verifier code_verifier=vvvvv sig=0123abcd bearer abc.def.ghi\n",
            "site echoed upl_Qwertyuiop1234 in an error\n",
        );
        let out = redact_with(raw, None);
        for secret in ["AbCdEfGh", "SECRETCODE123", "st4te", "upl_zzz", "r3fr", "session=abc", "deadbeef", "AKIA", "vvvvv", "0123abcd", "abc.def.ghi", "Qwerty"] {
            assert!(!out.contains(secret), "{secret} leaked in:\n{out}");
        }
        assert!(out.contains("upl_[redacted]"));
        assert!(out.contains("response=ok"), "{out}");
        assert!(out.contains("\"type\":\"bearer\""), "{out}");
        assert!(out.contains("/api/uploader/ingest"));
        // Idempotent: redacting again changes nothing.
        assert_eq!(redact_with(&out, None), out);
    }

    #[test]
    fn home_folder_name_is_redacted_on_each_platform() {
        let win = redact_with(r"C:\Users\Jane Doe\AppData\Local\x and c:/users/Jane Doe/y", Some(r"C:\Users\Jane Doe"));
        assert!(!win.contains("Jane"), "{win}");
        assert!(win.contains(r"C:\Users\<user>\AppData\Local\x"), "{win}");
        let mac = redact_with("/Users/jdoe/Library/Logs/x and /Users/other/z", Some("/Users/jdoe"));
        assert!(!mac.contains("jdoe") && !mac.contains("other"), "{mac}");
        assert!(mac.contains("/Users/<user>/Library/Logs/x"));
        let lin = redact_with("/home/jdoe/.local/state", None);
        assert_eq!(lin, "/home/<user>/.local/state");
        // Game folders outside the home folder stay readable.
        let game = redact_with(r"C:\Program Files (x86)\World of Warcraft\_forever_", Some(r"C:\Users\Jane"));
        assert_eq!(game, r"C:\Program Files (x86)\World of Warcraft\_forever_");
        assert_eq!(redact_with(r"C:\Users\Public\Games", None), r"C:\Users\Public\Games");
    }

    #[test]
    fn utc_stamps() {
        assert_eq!(utc_stamp(0), "1970-01-01T00:00:00Z");
        assert_eq!(utc_stamp(1_794_362_287), "2026-11-11T01:58:07Z");
        assert_eq!(utc_stamp(951_782_400), "2000-02-29T00:00:00Z");
    }

    #[test]
    fn rotation_keeps_three_files_under_the_cap() {
        let dir = tempfile::tempdir().unwrap();
        let line = vec![b'x'; 99];
        let mut l = line.clone();
        l.push(b'\n');
        for _ in 0..50 {
            append(dir.path(), 1000, &l).unwrap();
        }
        let names: Vec<String> = {
            let mut v: Vec<String> = fs::read_dir(dir.path()).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
            v.sort();
            v
        };
        assert_eq!(names, vec!["uploader.1.log", "uploader.2.log", "uploader.log"]);
        for n in &names {
            let len = fs::metadata(dir.path().join(n)).unwrap().len();
            assert!(len <= 1000, "{n} is {len}");
            assert!(len > 0);
        }
        // 50 lines of 100 bytes in 1000-byte files: the newest 2..3 files' worth survive.
        let total: u64 = names.iter().map(|n| fs::metadata(dir.path().join(n)).unwrap().len()).sum();
        assert!(total <= 3000 && total >= 2100, "{total}");
    }

    #[test]
    fn tail_spans_files_and_starts_on_a_line() {
        let dir = tempfile::tempdir().unwrap();
        for i in 0..40 {
            append(dir.path(), 500, format!("line {i:03} {}\n", "y".repeat(40)).as_bytes()).unwrap();
        }
        let t = tail_in(dir.path(), 300);
        assert!(t.len() <= 300);
        assert!(t.starts_with("line "), "{t}");
        assert!(t.trim_end().ends_with(&format!("line 039 {}", "y".repeat(40))));
        let all = tail_in(dir.path(), 64 * 1024);
        assert!(all.starts_with("line "));
        assert!(all.contains("line 039"));
    }

    #[test]
    fn written_lines_are_redacted_and_stamped() {
        let dir = tempfile::tempdir().unwrap();
        init_with(dir.path(), MAX_BYTES, Some("/Users/jdoe".into())).unwrap();
        error("upload", "failed for /Users/jdoe/x with Bearer upl_abcdefghijklmnopqrstuvwxyz0123456789ABCDEFG\nsecond line");
        let text = fs::read_to_string(dir.path().join(FILE)).unwrap();
        assert!(!text.contains("jdoe") && !text.contains("abcdefghij"), "{text}");
        assert_eq!(text.lines().count(), 1);
        let first = text.lines().next().unwrap();
        assert!(first.len() > 21 && first.as_bytes()[19] == b'Z' && first.contains(" ERROR upload: "), "{first}");
        *SINK.lock().unwrap() = None;
    }
}
