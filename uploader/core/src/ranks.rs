//! Guild rank names from the addon's SavedVariables (`<Addon>DB.guild = { name, t, ranks = { ... } }`).
//! The addon writes that block on every character in a guild, linked or not. The uploader sends the
//! newest one to `POST /api/uploader/ranks` with its uploader token; the site decides whether the
//! signed-in member may set the names (Guild Master or officer). Files only: nothing is written back.
//!
//! The reader is a small bounded parser for the Lua table constructors the game writes. It caps the
//! file size, nesting depth and value count, and returns `None` on anything malformed. It never panics.
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

/// Larger SavedVariables files are skipped (export strings make them big, but not this big).
pub const MAX_FILE_BYTES: u64 = 16 * 1024 * 1024;
const MAX_DEPTH: usize = 16;
const MAX_VALUES: usize = 250_000;
/// Same limits as the addon (Guild.lua MAX_RANKS, MAX_LEN * 4 bytes) and the site.
pub const MAX_RANKS: usize = 10;
const MAX_RANK_BYTES: usize = 160;
const MAX_GUILD_BYTES: usize = 96;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct GuildRanks {
    pub guild: String,
    /// Capture time in Unix seconds, as written by the addon.
    pub t: i64,
    pub ranks: Vec<String>,
}

impl GuildRanks {
    /// Stable hash of the guild name and names, for "already sent" checks.
    pub fn hash(&self) -> String {
        let mut h = Sha256::new();
        h.update(self.guild.as_bytes());
        for r in &self.ranks {
            h.update([0u8]);
            h.update(r.as_bytes());
        }
        h.finalize().iter().map(|b| format!("{b:02x}")).collect()
    }
}

// ---------------------------------------------------------------- Lua values

#[derive(Debug, Clone, PartialEq)]
enum Val {
    Str(String),
    Num(f64),
    Bool(bool),
    Nil,
    Table(Vec<(Key, Val)>),
}

#[derive(Debug, Clone, PartialEq)]
enum Key {
    Name(String),
    Num(f64),
    Pos(usize),
}

struct Parser<'a> {
    s: &'a [u8],
    i: usize,
    values: usize,
}

impl<'a> Parser<'a> {
    fn new(s: &'a str) -> Self { Parser { s: s.as_bytes(), i: 0, values: 0 } }

    fn peek(&self) -> Option<u8> { self.s.get(self.i).copied() }
    fn at(&self, off: usize) -> Option<u8> { self.s.get(self.i + off).copied() }

    /// Long bracket level at the cursor: `[[` → 0, `[==[` → 2. Does not move.
    fn long_level(&self) -> Option<usize> {
        if self.peek() != Some(b'[') { return None; }
        let mut j = 1;
        while self.at(j) == Some(b'=') { j += 1; }
        if self.at(j) == Some(b'[') { Some(j - 1) } else { None }
    }

    fn long_body(&mut self, level: usize) -> Option<String> {
        self.i += level + 2;
        if self.peek() == Some(b'\r') { self.i += 1; }
        if self.peek() == Some(b'\n') { self.i += 1; }
        let start = self.i;
        while self.i < self.s.len() {
            if self.s[self.i] == b']' {
                let mut j = 1;
                while self.at(j) == Some(b'=') { j += 1; }
                if j - 1 == level && self.at(j) == Some(b']') {
                    let body = String::from_utf8_lossy(&self.s[start..self.i]).into_owned();
                    self.i += level + 2;
                    return Some(body);
                }
            }
            self.i += 1;
        }
        None
    }

    fn skip_ws(&mut self) -> Option<()> {
        loop {
            match self.peek() {
                Some(b' ' | b'\t' | b'\r' | b'\n' | 0x0b | 0x0c) => self.i += 1,
                Some(b'-') if self.at(1) == Some(b'-') => {
                    self.i += 2;
                    if let Some(level) = self.long_level() {
                        self.long_body(level)?;
                    } else {
                        while let Some(c) = self.peek() {
                            if c == b'\n' { break; }
                            self.i += 1;
                        }
                    }
                }
                _ => return Some(()),
            }
        }
    }

    fn name(&mut self) -> Option<String> {
        let start = self.i;
        match self.peek() {
            Some(c) if c == b'_' || c.is_ascii_alphabetic() => self.i += 1,
            _ => return None,
        }
        while let Some(c) = self.peek() {
            if c == b'_' || c.is_ascii_alphanumeric() { self.i += 1 } else { break }
        }
        Some(String::from_utf8_lossy(&self.s[start..self.i]).into_owned())
    }

    fn string(&mut self) -> Option<String> {
        let q = self.peek()?;
        self.i += 1;
        let mut out: Vec<u8> = Vec::new();
        loop {
            let c = self.peek()?;
            self.i += 1;
            if c == q { break; }
            if c == b'\n' { return None; }
            if c != b'\\' { out.push(c); continue; }
            let e = self.peek()?;
            self.i += 1;
            match e {
                b'n' => out.push(b'\n'),
                b't' => out.push(b'\t'),
                b'r' => out.push(b'\r'),
                b'a' => out.push(7),
                b'b' => out.push(8),
                b'f' => out.push(12),
                b'v' => out.push(11),
                b'\\' | b'"' | b'\'' => out.push(e),
                b'\n' => out.push(b'\n'),
                b'\r' => { if self.peek() == Some(b'\n') { self.i += 1; } out.push(b'\n') }
                b'x' => {
                    let h = std::str::from_utf8(self.s.get(self.i..self.i + 2)?).ok()?;
                    out.push(u8::from_str_radix(h, 16).ok()?);
                    self.i += 2;
                }
                b'z' => { while matches!(self.peek(), Some(b' ' | b'\t' | b'\r' | b'\n')) { self.i += 1; } }
                d if d.is_ascii_digit() => {
                    let mut n = u32::from(d - b'0');
                    for _ in 0..2 {
                        match self.peek() {
                            Some(x) if x.is_ascii_digit() => { n = n * 10 + u32::from(x - b'0'); self.i += 1; }
                            _ => break,
                        }
                    }
                    out.push(u8::try_from(n).ok()?);
                }
                _ => return None,
            }
        }
        Some(String::from_utf8_lossy(&out).into_owned())
    }

    fn number(&mut self) -> Option<f64> {
        let start = self.i;
        if matches!(self.peek(), Some(b'-' | b'+')) { self.i += 1; }
        while let Some(c) = self.peek() {
            let exp_sign = matches!(c, b'+' | b'-') && matches!(self.s.get(self.i.wrapping_sub(1)), Some(b'e' | b'E' | b'p' | b'P'));
            if c.is_ascii_alphanumeric() || c == b'.' || exp_sign { self.i += 1 } else { break }
        }
        let text = std::str::from_utf8(&self.s[start..self.i]).ok()?.to_ascii_lowercase();
        let (neg, body) = match text.strip_prefix('-') { Some(b) => (true, b), None => (false, text.strip_prefix('+').unwrap_or(&text)) };
        let v = if let Some(hex) = body.strip_prefix("0x") {
            if hex.is_empty() || hex.len() > 15 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) { return None; }
            i64::from_str_radix(hex, 16).ok()? as f64
        } else if body == "inf" || body == "nan" || body.is_empty() {
            return None;
        } else {
            body.parse::<f64>().ok()?
        };
        if !v.is_finite() { return None; }
        Some(if neg { -v } else { v })
    }

    fn value(&mut self, depth: usize) -> Option<Val> {
        self.values += 1;
        if self.values > MAX_VALUES || depth > MAX_DEPTH { return None; }
        self.skip_ws()?;
        match self.peek()? {
            b'{' => self.table(depth),
            b'"' | b'\'' => self.string().map(Val::Str),
            b'[' => { let level = self.long_level()?; self.long_body(level).map(Val::Str) }
            c if c.is_ascii_digit() || c == b'-' || c == b'+' || c == b'.' => self.number().map(Val::Num),
            _ => match self.name()?.as_str() {
                "true" => Some(Val::Bool(true)),
                "false" => Some(Val::Bool(false)),
                "nil" => Some(Val::Nil),
                _ => None,
            },
        }
    }

    fn table(&mut self, depth: usize) -> Option<Val> {
        self.i += 1; // {
        let mut out = Vec::new();
        let mut pos = 0usize;
        loop {
            self.skip_ws()?;
            match self.peek()? {
                b'}' => { self.i += 1; return Some(Val::Table(out)); }
                b'[' if self.long_level().is_none() => {
                    self.i += 1;
                    let k = match self.value(depth + 1)? {
                        Val::Str(s) => Key::Name(s),
                        Val::Num(n) => Key::Num(n),
                        Val::Bool(_) | Val::Nil | Val::Table(_) => return None,
                    };
                    self.skip_ws()?;
                    if self.peek()? != b']' { return None; }
                    self.i += 1;
                    self.skip_ws()?;
                    if self.peek()? != b'=' { return None; }
                    self.i += 1;
                    let v = self.value(depth + 1)?;
                    out.push((k, v));
                }
                c if c == b'_' || c.is_ascii_alphabetic() => {
                    let save = self.i;
                    let n = self.name()?;
                    self.skip_ws()?;
                    if self.peek() == Some(b'=') && self.at(1) != Some(b'=') {
                        self.i += 1;
                        let v = self.value(depth + 1)?;
                        out.push((Key::Name(n), v));
                    } else {
                        self.i = save;
                        let v = self.value(depth + 1)?;
                        pos += 1;
                        out.push((Key::Pos(pos), v));
                    }
                }
                _ => {
                    let v = self.value(depth + 1)?;
                    pos += 1;
                    out.push((Key::Pos(pos), v));
                }
            }
            self.skip_ws()?;
            match self.peek()? {
                b',' | b';' => self.i += 1,
                b'}' => {}
                _ => return None,
            }
        }
    }

    /// Top level of a SavedVariables file: `Name = value` statements.
    fn chunk(&mut self) -> Option<Vec<(String, Val)>> {
        let mut out = Vec::new();
        loop {
            self.skip_ws()?;
            if self.peek().is_none() { return Some(out); }
            if self.peek() == Some(b';') { self.i += 1; continue; }
            let n = self.name()?;
            self.skip_ws()?;
            if self.peek()? != b'=' { return None; }
            self.i += 1;
            let v = self.value(0)?;
            out.push((n, v));
        }
    }
}

fn field<'v>(t: &'v [(Key, Val)], name: &str) -> Option<&'v Val> {
    t.iter().find(|(k, _)| matches!(k, Key::Name(n) if n == name)).map(|(_, v)| v)
}

fn clean(s: &str, max: usize) -> Option<String> {
    let s = s.trim();
    if s.is_empty() || s.len() > max || s.chars().any(|c| c.is_control()) { return None; }
    Some(s.to_string())
}

/// `{ name = "...", t = 123, ranks = { "...", ... } }` → `GuildRanks`, or `None`.
fn guild_from(v: &Val) -> Option<GuildRanks> {
    let Val::Table(t) = v else { return None };
    let Val::Str(name) = field(t, "name")? else { return None };
    let guild = clean(name, MAX_GUILD_BYTES)?;
    let Val::Num(time) = field(t, "t")? else { return None };
    if time.fract() != 0.0 || *time <= 0.0 || *time > 9.0e15 { return None; }
    let Val::Table(list) = field(t, "ranks")? else { return None };
    if list.is_empty() || list.len() > MAX_RANKS { return None; }
    let mut slots: Vec<Option<String>> = vec![None; list.len()];
    for (k, v) in list {
        let idx = match k {
            Key::Pos(p) => *p,
            Key::Num(n) if n.fract() == 0.0 && *n >= 1.0 && *n <= MAX_RANKS as f64 => *n as usize,
            _ => return None,
        };
        let Val::Str(s) = v else { return None };
        let slot = slots.get_mut(idx.checked_sub(1)?)?;
        if slot.is_some() { return None; }
        *slot = Some(clean(s, MAX_RANK_BYTES)?);
    }
    let ranks = slots.into_iter().collect::<Option<Vec<_>>>()?;
    Some(GuildRanks { guild, t: *time as i64, ranks })
}

/// The guild block in one SavedVariables file's text. Looks in `<addon>DB` first, then any other
/// top-level table. `None` when the file can't be read as Lua or holds no valid block.
pub fn parse_guild_block(text: &str, addon: &str) -> Option<GuildRanks> {
    let top = Parser::new(text).chunk()?;
    let db = format!("{addon}DB");
    let preferred = top.iter().filter(|(n, _)| !addon.is_empty() && *n == db);
    let rest = top.iter().filter(|(n, _)| addon.is_empty() || *n != db);
    preferred.chain(rest).find_map(|(_, v)| match v {
        Val::Table(t) => field(t, "guild").and_then(guild_from),
        _ => None,
    })
}

fn read_capped(p: &Path) -> Option<String> {
    let f = fs::File::open(p).ok()?;
    if f.metadata().ok()?.len() > MAX_FILE_BYTES { return None; }
    let mut buf = Vec::new();
    f.take(MAX_FILE_BYTES + 1).read_to_end(&mut buf).ok()?;
    if buf.len() as u64 > MAX_FILE_BYTES { return None; }
    Some(String::from_utf8_lossy(&buf).into_owned())
}

fn same_guild(a: &str, b: &str) -> bool { a.trim().to_lowercase() == b.trim().to_lowercase() }

/// Newest block (largest `t`) across the given SavedVariables files whose guild name matches `guild`.
/// An empty `guild` accepts any name; the site still checks it.
pub fn newest_guild_ranks(files: &[PathBuf], addon: &str, guild: &str) -> Option<GuildRanks> {
    files
        .iter()
        .filter_map(|f| read_capped(f))
        .filter_map(|text| parse_guild_block(&text, addon))
        .filter(|g| guild.trim().is_empty() || same_guild(&g.guild, guild))
        .max_by_key(|g| g.t)
}

// ---------------------------------------------------------------- sending

/// Last block the site answered for with a final result, so an unchanged block is not sent again.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RanksMark {
    pub t: i64,
    pub hash: String,
}

/// What happened to the rank names on one pass. `line` uses the per-string line format
/// (`ranks accepted` or `ranks <reason>`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RanksOutcome {
    /// HTTP 200. `stored` is the site's `stored` field as text.
    Stored(String),
    /// A final refusal for this block (403, 400). Not retried until the block changes.
    Refused(String),
    /// Worth trying again on a later pass (404, 429, 5xx, network).
    Retry(String),
    /// `401 bad_token`: the saved token is dead.
    BadToken,
}

pub const NEEDS_OFFICER: &str = "Rank names need a Guild Master or officer character linked";

impl RanksOutcome {
    pub fn line(&self) -> String {
        match self {
            RanksOutcome::Stored(_) => "ranks accepted".into(),
            RanksOutcome::Refused(m) | RanksOutcome::Retry(m) => format!("ranks {m}"),
            RanksOutcome::BadToken => "ranks the site rejected the uploader token".into(),
        }
    }
    /// Whether the site gave a final answer for this block.
    pub fn is_final(&self) -> bool { matches!(self, RanksOutcome::Stored(_) | RanksOutcome::Refused(_)) }
}

/// Map a `/api/uploader/ranks` answer. Contract: 200 `{stored}`, 401, 403, 400.
pub fn interpret_ranks(status: u16, body: &str) -> RanksOutcome {
    #[derive(Deserialize)]
    struct Err { error: Option<String> }
    let err = || serde_json::from_str::<Err>(body).ok().and_then(|e| e.error).filter(|e| e.len() <= 60 && e.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'));
    match status {
        200 => {
            let stored = serde_json::from_str::<serde_json::Value>(body).ok().and_then(|v| v.get("stored").cloned());
            RanksOutcome::Stored(match stored {
                Some(serde_json::Value::String(s)) => s,
                Some(serde_json::Value::Bool(true)) => "stored".into(),
                Some(serde_json::Value::Bool(false)) => "unchanged".into(),
                _ => "accepted".into(),
            })
        }
        401 if err().as_deref() == Some("bad_token") => RanksOutcome::BadToken,
        401 => RanksOutcome::Retry("the site didn't accept this sign-in for rank names".into()),
        403 => RanksOutcome::Refused(NEEDS_OFFICER.into()),
        400 => RanksOutcome::Refused(match err() {
            Some(e) => format!("the site couldn't use these rank names ({e})"),
            None => "the site couldn't use these rank names".into(),
        }),
        404 => RanksOutcome::Retry("the site doesn't take rank names yet".into()),
        429 => RanksOutcome::Retry("the site asked us to slow down".into()),
        code => RanksOutcome::Retry(format!("the site answered {code} for rank names")),
    }
}

/// `POST {site}/api/uploader/ranks` with `{guild, t, ranks}` and the Bearer uploader token.
pub fn post_ranks(site: &str, token: &str, g: &GuildRanks) -> RanksOutcome {
    let url = format!("{}/api/uploader/ranks", site.trim_end_matches('/'));
    let res = ureq::post(&url)
        .set("Authorization", &format!("Bearer {token}"))
        .set("User-Agent", "guild-uploader")
        .timeout(std::time::Duration::from_secs(30))
        .send_json(serde_json::json!({ "guild": g.guild, "t": g.t, "ranks": g.ranks }));
    match res {
        Ok(r) => interpret_ranks(200, &r.into_string().unwrap_or_default()),
        Err(ureq::Error::Status(code, r)) => interpret_ranks(code, &r.into_string().unwrap_or_default()),
        Err(_) => RanksOutcome::Retry("couldn't reach the site for rank names".into()),
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Shape of a real SavedVariables file as the game writes it, with an empty chars and outbox.
    pub(crate) fn fixture(t: i64) -> String {
        format!(
            "\nExampleDB = {{\n\t[\"chars\"] = {{\n\t}},\n\t[\"settings\"] = {{\n\t\t[\"location\"] = true,\n\t}},\n\t[\"guild\"] = {{\n\t\t[\"t\"] = {t},\n\t\t[\"name\"] = \"Example\",\n\t\t[\"ranks\"] = {{\n\t\t\t\"Guild Master\", -- [1]\n\t\t\t\"Officer\", -- [2]\n\t\t\t\"Veteran\", -- [3]\n\t\t\t\"Member\", -- [4]\n\t\t\t\"Initiate\", -- [5]\n\t\t}},\n\t}},\n\t[\"outbox\"] = {{\n\t}},\n}}\n"
        )
    }

    fn five() -> Vec<String> { ["Guild Master", "Officer", "Veteran", "Member", "Initiate"].map(String::from).to_vec() }

    #[test]
    fn reads_the_real_file_shape() {
        let g = parse_guild_block(&fixture(1794112233), "Example").unwrap();
        assert_eq!(g, GuildRanks { guild: "Example".into(), t: 1794112233, ranks: five() });
    }

    #[test]
    fn reads_the_short_form_and_explicit_indexes() {
        let a = "ExampleDB = {guild = {name='Example', t=5, ranks={'Guild Master','Officer','Veteran','Member','Initiate'}}, chars = {}, outbox = {}}";
        assert_eq!(parse_guild_block(a, "Example").unwrap().ranks, five());
        let b = "ExampleDB = { [\"guild\"] = { [\"name\"] = \"Example\", [\"t\"] = 7, [\"ranks\"] = { [2] = \"Officer\", [1] = \"Guild Master\" } } }";
        assert_eq!(parse_guild_block(b, "Example").unwrap().ranks, vec!["Guild Master", "Officer"]);
    }

    #[test]
    fn escapes_comments_and_long_strings_do_not_confuse_it() {
        let s = "-- header\nExampleDB = { --[==[ block } ]==]\n [\"outbox\"] = { [\"gbank\"] = { s = \"!BF:gbank:v1:a}b\\\"c\", x = [[ } ]] } },\n [\"guild\"] = { name = \"Ex\\97mple\", t = 1.0e3, ranks = { \"R\\x61nk\" } } }";
        let g = parse_guild_block(s, "Example").unwrap();
        assert_eq!(g, GuildRanks { guild: "Example".into(), t: 1000, ranks: vec!["Rank".into()] });
    }

    #[test]
    fn malformed_or_oversized_input_is_none_not_a_panic() {
        let bad = [
            "", "ExampleDB", "ExampleDB = {", "ExampleDB = { guild = { name = 'x', t = 1, ranks = { 'a' ",
            "ExampleDB = { guild = { name = '', t = 1, ranks = { 'a' } } }",
            "ExampleDB = { guild = { name = 'B', t = -1, ranks = { 'a' } } }",
            "ExampleDB = { guild = { name = 'B', t = 1.5, ranks = { 'a' } } }",
            "ExampleDB = { guild = { name = 'B', t = 1, ranks = { } } }",
            "ExampleDB = { guild = { name = 'B', t = 1, ranks = { '  ' } } }",
            "ExampleDB = { guild = { name = 'B', t = 1, ranks = { 'a', 3 } } }",
            "ExampleDB = { guild = { name = 'B', t = 1, ranks = { [1] = 'a', [3] = 'c' } } }",
            "ExampleDB = { guild = { name = 'B', t = 1, ranks = { 'a', 'b', 'c', 'd', 'e', 'f', 'g', 'h', 'i', 'j', 'k' } } }",
            "ExampleDB = { guild = { name = 'B', t = 0x, ranks = { 'a' } } }",
            "ExampleDB = { guild = { name = 'unterminated, t = 1 } }",
            "ExampleDB = { guild = { name = 'B\\9999', t = 1, ranks = { 'a' } } }",
            "\u{0}\u{1}\u{ff}{{{{",
        ];
        for s in bad { assert!(parse_guild_block(s, "Example").is_none(), "{s:?}"); }
        let long = format!("ExampleDB = {{ guild = {{ name = 'B', t = 1, ranks = {{ '{}' }} }} }}", "x".repeat(MAX_RANK_BYTES + 1));
        assert!(parse_guild_block(&long, "Example").is_none());
        let deep = format!("ExampleDB = {}{}", "{".repeat(10_000), "}".repeat(10_000));
        assert!(parse_guild_block(&deep, "Example").is_none());
        let wide = format!("ExampleDB = {{ {} guild = {{ name = 'B', t = 1, ranks = {{ 'a' }} }} }}", "1,".repeat(MAX_VALUES + 10));
        assert!(parse_guild_block(&wide, "Example").is_none());
        // Truncated at every byte of a real file: never panics.
        let f = fixture(1794112233);
        for i in 0..f.len() { let _ = parse_guild_block(&f[..i], "Example"); }
    }

    #[test]
    fn addon_db_wins_over_other_tables() {
        let s = "OtherDB = { guild = { name = 'Other', t = 9, ranks = { 'x' } } }\nExampleDB = { guild = { name = 'Example', t = 1, ranks = { 'y' } } }";
        assert_eq!(parse_guild_block(s, "Example").unwrap().guild, "Example");
        assert_eq!(parse_guild_block(s, "").unwrap().guild, "Other");
    }

    fn two_accounts(t1: i64, t2: i64) -> (tempfile::TempDir, Vec<PathBuf>) {
        let dir = tempfile::tempdir().unwrap();
        for (acct, t) in [("300769113#1", t1), ("300769113#2", t2)] {
            let sv = dir.path().join("WTF").join("Account").join(acct).join("SavedVariables");
            fs::create_dir_all(&sv).unwrap();
            fs::write(sv.join("Example.lua"), fixture(t)).unwrap();
        }
        let files = crate::saved_variables(dir.path(), "Example");
        (dir, files)
    }

    #[test]
    fn two_accounts_pick_the_newest_capture_for_the_configured_guild() {
        let (_d, files) = two_accounts(1794100000, 1794100500);
        assert_eq!(files.len(), 2);
        let g = newest_guild_ranks(&files, "Example", "Example").unwrap();
        assert_eq!((g.t, g.ranks.clone()), (1794100500, five()));
        assert_eq!(newest_guild_ranks(&files, "Example", " example ").unwrap().t, 1794100500);
        assert!(newest_guild_ranks(&files, "Example", "Another Guild").is_none());
        assert_eq!(newest_guild_ranks(&files, "Example", "").unwrap().t, 1794100500);
    }

    #[test]
    fn a_newer_block_for_another_guild_is_ignored() {
        let (d, mut files) = two_accounts(1794100000, 1794100500);
        let sv = d.path().join("WTF").join("Account").join("3").join("SavedVariables");
        fs::create_dir_all(&sv).unwrap();
        fs::write(sv.join("Example.lua"), fixture(1794199999).replace("\"Example\"", "\"Other\"")).unwrap();
        files.push(sv.join("Example.lua"));
        assert_eq!(newest_guild_ranks(&files, "Example", "Example").unwrap().t, 1794100500);
    }

    #[test]
    fn oversized_files_are_skipped() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("Example.lua");
        let f = fs::File::create(&p).unwrap();
        f.set_len(MAX_FILE_BYTES + 1).unwrap();
        assert!(newest_guild_ranks(&[p], "Example", "Example").is_none());
    }

    #[test]
    fn hash_changes_with_names_not_time() {
        let a = parse_guild_block(&fixture(1), "Example").unwrap();
        let b = parse_guild_block(&fixture(2), "Example").unwrap();
        assert_eq!(a.hash(), b.hash());
        let c = parse_guild_block(&fixture(1).replace("Veteran", "Raider"), "Example").unwrap();
        assert_ne!(a.hash(), c.hash());
    }

    #[test]
    fn site_answers_map_to_clear_outcomes() {
        assert_eq!(interpret_ranks(200, r#"{"stored":"stored"}"#), RanksOutcome::Stored("stored".into()));
        assert_eq!(interpret_ranks(200, r#"{"stored":false}"#), RanksOutcome::Stored("unchanged".into()));
        assert_eq!(interpret_ranks(401, r#"{"error":"bad_token"}"#), RanksOutcome::BadToken);
        assert!(!interpret_ranks(401, "{}").is_final());
        let r = interpret_ranks(403, r#"{"error":"forbidden"}"#);
        assert_eq!(r.line(), format!("ranks {NEEDS_OFFICER}"));
        assert!(r.is_final());
        assert_eq!(interpret_ranks(400, r#"{"error":"other_guild"}"#), RanksOutcome::Refused("the site couldn't use these rank names (other_guild)".into()));
        assert_eq!(interpret_ranks(400, r#"{"error":"upl_abc def"}"#), RanksOutcome::Refused("the site couldn't use these rank names".into()));
        assert!(!interpret_ranks(404, "").is_final());
        assert!(!interpret_ranks(500, "").is_final());
        assert_eq!(RanksOutcome::Stored("x".into()).line(), "ranks accepted");
    }
}
