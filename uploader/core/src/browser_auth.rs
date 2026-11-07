//! Loopback authorization-code + PKCE sign-in.
//!
//! The system browser reuses the site's existing Discord session. This module
//! binds `127.0.0.1`, receives a one-time code, and exchanges it over HTTPS.
//! The uploader token is returned to the caller and is never put in a URL.
//!
//! The live site does not implement these routes yet (they 404). A missing
//! route or any unexpected response fails closed: this function returns `Err`
//! and the caller must not store a token.
//!
//! Site contract the parent has to add (nothing here is treated as deployed):
//!
//! - `GET /api/uploader/oauth/authorize` with no query, `Accept: application/json`,
//!   returns `400` `{"error":"invalid_request"}` and creates no row. Any HTML
//!   or `404` means "not deployed".
//! - The same path with `response_type=code`, `client_id=guild-uploader`,
//!   `redirect_uri` (http, host exactly `127.0.0.1`, port 1024-65535, path
//!   `/callback`, no query or fragment), `state`, `code_challenge`,
//!   `code_challenge_method=S256`, `scope=ingest`.
//!   No session: `302` to `/signin?callbackUrl=` the same-origin authorize URL.
//!   Who may consent is the site's token rule (Trial and up can upload their own
//!   character strings; Officer, GM, and Webmaster stay required for guild bank
//!   and raid ingest). Do not refuse every non-officer here.
//!   Optional `prompt=login` (no other prompt value): sign out the current
//!   Auth.js session first, then continue to Discord sign-in. `callbackUrl`
//!   must be this same-origin authorize URL with `prompt` removed. Do not
//!   redirect to the loopback during sign-out, and do not log the code,
//!   verifier, state, or token. Absent `prompt` reuses the current session.
//!   Session allowed to mint a token: consent page, not an automatic approval.
//!   Consent `POST /api/uploader/oauth/consent` with the Auth.js CSRF token.
//!   Deny redirects to `{redirect_uri}?error=access_denied&state=`.
//!   Approve redirects to `{redirect_uri}?code=&state=` only. Code is 32 bytes,
//!   one use, 60 seconds. Transaction lives at most 5 minutes.
//! - `POST /api/uploader/oauth/token` JSON, no cookies:
//!   `grant_type=authorization_code`, `client_id`, `code`, `redirect_uri`,
//!   `code_verifier`, `state`. Success `200` `{"token":"upl_…","token_type":"Bearer"}`.
//!   The server checks PKCE, state, redirect, one-time use, and the role again.
//!   `400 {"error":"invalid_grant"}`, `403 {"error":"forbidden"}`.
//! - Ingest stays `POST /api/uploader/ingest` with `Authorization: Bearer`.
//!   `401 {"error":"bad_token"}` revokes the local copy. `403 {"error":"forbidden"}`
//!   does not. No token in a URL, a log line, or the installer.

use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::net::{IpAddr, SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

pub const CLIENT_ID: &str = "guild-uploader";
const AUTHORIZE_PATH: &str = "/api/uploader/oauth/authorize";
const TOKEN_PATH: &str = "/api/uploader/oauth/token";

#[derive(Debug, PartialEq, Eq)]
pub enum AuthError {
    NotDeployed,
    Cancelled,
    Expired,
    Denied,
    Forbidden,
    InvalidGrant,
    Browser,
    Bind,
    BadResponse,
    Network,
}

impl std::fmt::Display for AuthError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        let s = match self {
            AuthError::NotDeployed => "Sign-in isn't available on this site yet. Nothing was saved.",
            AuthError::Cancelled => "Sign-in cancelled. Nothing was saved.",
            AuthError::Expired => "Sign-in expired. Try again.",
            AuthError::Denied => "Sign-in was declined. Nothing was saved.",
            AuthError::Forbidden => "Sign in again with a Discord account that has the right role.",
            AuthError::InvalidGrant => "Sign-in expired or was already used. Try again.",
            AuthError::Browser => "Couldn't open the browser. Nothing was saved.",
            AuthError::Bind => "Couldn't listen for the browser on this computer. Nothing was saved.",
            AuthError::BadResponse => "The site's sign-in response was not usable. Nothing was saved.",
            AuthError::Network => "Couldn't reach the site to sign in. Nothing was saved.",
        };
        f.write_str(s)
    }
}

/// Compile-time / operator site URL. HTTPS only, no userinfo, no path required.
pub fn parse_site_url(raw: &str) -> Result<String, ()> {
    let raw = raw.trim().trim_end_matches('/');
    let Some(rest) = raw.strip_prefix("https://") else { return Err(()) };
    if !origin_shape(rest) {
        return Err(());
    }
    Ok(raw.to_string())
}

fn origin_shape(rest: &str) -> bool {
    if rest.is_empty() || rest.contains('@') || rest.contains('\\') || rest.contains(' ') || rest.contains('#') {
        return false;
    }
    let host = rest.split('/').next().unwrap_or("");
    let host_only = host.split(':').next().unwrap_or("");
    !host_only.is_empty() && host_only != "localhost"
}

fn site_for_connect(site: &str) -> Result<String, AuthError> {
    let site = site.trim().trim_end_matches('/');
    if parse_site_url(site).is_ok() {
        return Ok(site.to_string());
    }
    // Local tests only. Production uses parse_site_url, which rejects http.
    if let Some(rest) = site.strip_prefix("http://") {
        if (rest.starts_with("127.0.0.1:") || rest == "127.0.0.1") && origin_shape(rest) {
            return Ok(site.to_string());
        }
    }
    Err(AuthError::NotDeployed)
}

pub fn site_host(url: &str) -> String {
    url.trim_start_matches("https://")
        .trim_start_matches("http://")
        .split('/')
        .next()
        .unwrap_or("")
        .to_string()
}

fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(5))
        .timeout_read(Duration::from_secs(10))
        .timeout(Duration::from_secs(15))
        .redirects(0)
        .build()
}

fn random_bytes(n: usize) -> Result<Vec<u8>, AuthError> {
    let mut buf = vec![0u8; n];
    getrandom::getrandom(&mut buf).map_err(|_| AuthError::BadResponse)?;
    Ok(buf)
}

fn b64url(data: &[u8]) -> String {
    const T: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::new();
    let mut i = 0;
    while i + 3 <= data.len() {
        let n = ((data[i] as u32) << 16) | ((data[i + 1] as u32) << 8) | (data[i + 2] as u32);
        out.push(T[((n >> 18) & 63) as usize] as char);
        out.push(T[((n >> 12) & 63) as usize] as char);
        out.push(T[((n >> 6) & 63) as usize] as char);
        out.push(T[(n & 63) as usize] as char);
        i += 3;
    }
    if i < data.len() {
        let rem = data.len() - i;
        let mut n = (data[i] as u32) << 16;
        if rem == 2 {
            n |= (data[i + 1] as u32) << 8;
        }
        out.push(T[((n >> 18) & 63) as usize] as char);
        out.push(T[((n >> 12) & 63) as usize] as char);
        if rem == 2 {
            out.push(T[((n >> 6) & 63) as usize] as char);
        }
    }
    out
}

fn pkce_challenge(verifier: &str) -> String {
    b64url(&Sha256::digest(verifier.as_bytes()))
}

fn pct(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

fn pct_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = &s[i + 1..i + 3];
            if let Ok(v) = u8::from_str_radix(hex, 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn ct_eq(a: &str, b: &str) -> bool {
    let ab = a.as_bytes();
    let bb = b.as_bytes();
    let mut diff = ab.len() ^ bb.len();
    let n = ab.len().max(bb.len());
    for i in 0..n {
        let x = if i < ab.len() { ab[i] } else { 0 };
        let y = if i < bb.len() { bb[i] } else { 0 };
        diff |= (x ^ y) as usize;
    }
    diff == 0
}

fn preflight(site: &str) -> Result<(), AuthError> {
    let url = format!("{site}{AUTHORIZE_PATH}");
    match agent().get(&url).set("Accept", "application/json").set("User-Agent", "guild-uploader").call() {
        Ok(_) => Err(AuthError::NotDeployed),
        Err(ureq::Error::Status(400, resp)) => {
            let text = resp.into_string().unwrap_or_default();
            if text.len() > 1024 {
                return Err(AuthError::NotDeployed);
            }
            #[derive(serde::Deserialize)]
            struct Body { error: String }
            match serde_json::from_str::<Body>(&text) {
                Ok(body) if body.error == "invalid_request" => Ok(()),
                _ => Err(AuthError::NotDeployed),
            }
        }
        Err(ureq::Error::Status(_, _)) => Err(AuthError::NotDeployed),
        Err(_) => Err(AuthError::Network),
    }
}

/// SO_EXCLUSIVEADDRUSE, set before bind. socket2 0.5 does not wrap this option.
#[cfg(windows)]
fn exclusive_addr_use(socket: &socket2::Socket) -> Result<(), AuthError> {
    use std::os::windows::io::AsRawSocket;
    #[link(name = "ws2_32")]
    extern "system" {
        fn setsockopt(s: usize, level: i32, optname: i32, optval: *const i8, optlen: i32) -> i32;
    }
    let on: i32 = 1;
    let rc = unsafe {
        setsockopt(
            socket.as_raw_socket() as usize,
            0xffff, // SOL_SOCKET
            -5,     // SO_EXCLUSIVEADDRUSE
            &on as *const i32 as *const i8,
            4,
        )
    };
    if rc == 0 { Ok(()) } else { Err(AuthError::Bind) }
}

fn bind_loopback() -> Result<TcpListener, AuthError> {
    let socket = socket2::Socket::new(socket2::Domain::IPV4, socket2::Type::STREAM, None).map_err(|_| AuthError::Bind)?;
    #[cfg(windows)]
    exclusive_addr_use(&socket)?;
    let addr: SocketAddr = "127.0.0.1:0".parse().map_err(|_| AuthError::Bind)?;
    socket.bind(&addr.into()).map_err(|_| AuthError::Bind)?;
    socket.listen(16).map_err(|_| AuthError::Bind)?;
    socket.set_nonblocking(true).map_err(|_| AuthError::Bind)?;
    Ok(TcpListener::from(socket))
}

const CLOSE_HTML: &str = "<!DOCTYPE html><html><head><meta charset=\"utf-8\"><title>Guild Uploader</title></head><body><p>You can close this tab.</p></body></html>";

fn write_html(stream: &mut TcpStream, status: &str) {
    let body = CLOSE_HTML;
    let resp = format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nCache-Control: no-store\r\nReferrer-Policy: no-referrer\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.write_all(resp.as_bytes());
}

fn read_request(stream: &mut TcpStream) -> Option<String> {
    stream.set_read_timeout(Some(Duration::from_secs(2))).ok()?;
    let mut buf = [0u8; 8192];
    let n = stream.read(&mut buf).ok()?;
    if n == 0 || n == buf.len() {
        return None;
    }
    String::from_utf8(buf[..n].to_vec()).ok()
}

struct Callback {
    code: Option<String>,
    error: Option<String>,
    state: Option<String>,
    leak: bool,
    is_callback: bool,
}

fn parse_callback(target: &str) -> Callback {
    let mut cb = Callback { code: None, error: None, state: None, leak: false, is_callback: false };
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    if path != "/callback" {
        return cb;
    }
    cb.is_callback = true;
    for pair in query.split('&') {
        if pair.is_empty() {
            continue;
        }
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        let key = pct_decode(k);
        let val = pct_decode(v);
        if key == "token" || key == "access_token" || val.starts_with("upl_") {
            cb.leak = true;
        }
        match key.as_str() {
            "code" => cb.code = Some(val),
            "error" => cb.error = Some(val),
            "state" => cb.state = Some(val),
            _ => {}
        }
    }
    cb
}

fn accept_code(
    listener: &TcpListener,
    expected_state: &str,
    deadline: Instant,
    cancel: &AtomicBool,
) -> Result<String, AuthError> {
    let mut mismatches = 0u32;
    loop {
        if cancel.load(Ordering::SeqCst) {
            return Err(AuthError::Cancelled);
        }
        if Instant::now() >= deadline {
            return Err(AuthError::Expired);
        }
        match listener.accept() {
            Ok((mut stream, peer)) => {
                if peer.ip() != IpAddr::from([127, 0, 0, 1]) {
                    continue;
                }
                let Some(text) = read_request(&mut stream) else {
                    mismatches += 1;
                    if mismatches >= 8 {
                        return Err(AuthError::BadResponse);
                    }
                    continue;
                };
                let first = text.lines().next().unwrap_or("");
                let mut parts = first.split_whitespace();
                let method = parts.next().unwrap_or("");
                let target = parts.next().unwrap_or("");
                if method != "GET" {
                    write_html(&mut stream, "404 Not Found");
                    continue;
                }
                let cb = parse_callback(target);
                write_html(&mut stream, "200 OK");
                if !cb.is_callback {
                    continue;
                }
                if cb.leak {
                    return Err(AuthError::BadResponse);
                }
                let state_ok = cb.state.as_deref().map(|s| ct_eq(s, expected_state)).unwrap_or(false);
                if !state_ok {
                    mismatches += 1;
                    if mismatches >= 8 {
                        return Err(AuthError::BadResponse);
                    }
                    continue;
                }
                if cb.error.as_deref() == Some("access_denied") {
                    return Err(AuthError::Denied);
                }
                if let Some(code) = cb.code {
                    if code.is_empty() || code.len() > 256 || code.contains("upl_") {
                        return Err(AuthError::BadResponse);
                    }
                    return Ok(code);
                }
                return Err(AuthError::BadResponse);
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(40));
            }
            Err(_) => return Err(AuthError::Bind),
        }
    }
}

fn exchange(site: &str, code: &str, redirect_uri: &str, verifier: &str, state: &str) -> Result<String, AuthError> {
    let url = format!("{site}{TOKEN_PATH}");
    let body = serde_json::json!({
        "grant_type": "authorization_code",
        "client_id": CLIENT_ID,
        "code": code,
        "redirect_uri": redirect_uri,
        "code_verifier": verifier,
        "state": state,
    });
    let response = agent()
        .post(&url)
        .set("Accept", "application/json")
        .set("User-Agent", "guild-uploader")
        .send_json(body);
    match response {
        Ok(resp) => {
            let text = resp.into_string().unwrap_or_default();
            if text.len() > 4096 {
                return Err(AuthError::BadResponse);
            }
            #[derive(serde::Deserialize)]
            struct Body { token: String, #[serde(default)] token_type: String }
            let parsed: Body = serde_json::from_str(&text).map_err(|_| AuthError::BadResponse)?;
            if !parsed.token_type.is_empty() && parsed.token_type != "Bearer" {
                return Err(AuthError::BadResponse);
            }
            if !token_shape(&parsed.token) {
                return Err(AuthError::BadResponse);
            }
            Ok(parsed.token)
        }
        Err(ureq::Error::Status(403, _)) => Err(AuthError::Forbidden),
        Err(ureq::Error::Status(400, _)) => Err(AuthError::InvalidGrant),
        Err(ureq::Error::Status(404, _)) => Err(AuthError::NotDeployed),
        Err(ureq::Error::Status(_, _)) => Err(AuthError::BadResponse),
        Err(_) => Err(AuthError::Network),
    }
}

fn token_shape(token: &str) -> bool {
    token.starts_with("upl_")
        && token.len() > 4
        && token.len() <= 512
        && !token.chars().any(|c| c.is_whitespace())
}

/// Open the system browser via `open_browser` and return the `upl_` token.
/// `open_browser` returns false when the browser could not be opened.
/// The closure receives the authorize URL. It must not be logged by the caller.
pub fn connect_loopback(
    site: &str,
    overall: Duration,
    cancel: &AtomicBool,
    prompt_login: bool,
    mut open_browser: impl FnMut(&str) -> bool,
) -> Result<String, AuthError> {
    if cancel.load(Ordering::SeqCst) {
        return Err(AuthError::Cancelled);
    }
    let site = site_for_connect(site)?;
    preflight(&site)?;
    let listener = bind_loopback()?;
    let port = listener.local_addr().map_err(|_| AuthError::Bind)?.port();
    if port < 1024 {
        return Err(AuthError::Bind);
    }
    let verifier = b64url(&random_bytes(32)?);
    let challenge = pkce_challenge(&verifier);
    let state = b64url(&random_bytes(32)?);
    let redirect_uri = format!("http://127.0.0.1:{port}/callback");
    let mut authorize = format!(
        "{site}{AUTHORIZE_PATH}?response_type=code&client_id={CLIENT_ID}&redirect_uri={}&state={}&code_challenge={}&code_challenge_method=S256&scope=ingest",
        pct(&redirect_uri),
        pct(&state),
        pct(&challenge),
    );
    if prompt_login {
        authorize.push_str("&prompt=login");
    }
    if authorize.contains("upl_") || authorize.contains(&verifier) || authorize.matches("prompt=").count() > 1 {
        return Err(AuthError::BadResponse);
    }
    if prompt_login != authorize.ends_with("&prompt=login") {
        return Err(AuthError::BadResponse);
    }
    if !open_browser(&authorize) {
        return Err(AuthError::Browser);
    }
    let deadline = Instant::now() + overall;
    let code = accept_code(&listener, &state, deadline, cancel)?;
    drop(listener);
    exchange(&site, &code, &redirect_uri, &verifier, &state)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    use std::thread;

    #[test]
    fn pkce_rfc7636_vector() {
        let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        assert_eq!(pkce_challenge(verifier), "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM");
    }

    #[test]
    fn b64url_omits_padding() {
        assert_eq!(b64url(b"hello"), "aGVsbG8");
    }

    #[test]
    fn parse_site_url_allows_https_only() {
        assert!(parse_site_url("https://example.com").is_ok());
        assert!(parse_site_url("https://example.com/").is_ok());
        assert!(parse_site_url("http://example.com").is_err());
        assert!(parse_site_url("https://user:pass@example.com").is_err());
        assert!(parse_site_url("https://localhost").is_err());
        assert!(parse_site_url("").is_err());
    }

    fn json_response(code: u16, reason: &str, body: &str) -> Vec<u8> {
        format!(
            "HTTP/1.1 {code} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
        .into_bytes()
    }

    struct Mock {
        base: String,
        hits: Arc<std::sync::atomic::AtomicUsize>,
        challenge: Arc<Mutex<String>>,
    }

    fn spawn_mock(mode: &'static str) -> Mock {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(false).unwrap();
        let port = listener.local_addr().unwrap().port();
        let hits = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let challenge = Arc::new(Mutex::new(String::new()));
        let hits2 = hits.clone();
        let challenge2 = challenge.clone();
        thread::spawn(move || {
            for _ in 0..24 {
                let Ok((mut sock, _)) = listener.accept() else { break };
                sock.set_read_timeout(Some(Duration::from_secs(2))).ok();
                let mut buf = Vec::new();
                let mut tmp = [0u8; 1024];
                loop {
                    let n = sock.read(&mut tmp).unwrap_or(0);
                    if n == 0 {
                        break;
                    }
                    buf.extend_from_slice(&tmp[..n]);
                    if let Some(split) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                        let header = String::from_utf8_lossy(&buf[..split]).to_ascii_lowercase();
                        let length = header
                            .lines()
                            .find_map(|line| line.strip_prefix("content-length:").map(|v| v.trim().parse::<usize>().ok()))
                            .flatten()
                            .unwrap_or(0);
                        if buf.len() >= split + 4 + length || length == 0 {
                            break;
                        }
                    }
                    if buf.len() > 16384 {
                        break;
                    }
                }
                let text = String::from_utf8_lossy(&buf).to_string();
                let first = text.lines().next().unwrap_or("").to_string();
                let mut parts = first.split_whitespace();
                let method = parts.next().unwrap_or("");
                let target = parts.next().unwrap_or("");
                let body = text.split("\r\n\r\n").nth(1).unwrap_or("");
                let resp = if method == "GET" && target == AUTHORIZE_PATH {
                    if mode == "missing" {
                        b"HTTP/1.1 404 Not Found\r\nContent-Type: text/html\r\nContent-Length: 15\r\nConnection: close\r\n\r\n<!DOCTYPE html>".to_vec()
                    } else {
                        json_response(400, "Bad Request", r#"{"error":"invalid_request"}"#)
                    }
                } else if method == "POST" && target.starts_with(TOKEN_PATH) {
                    hits2.fetch_add(1, Ordering::SeqCst);
                    let req: serde_json::Value = serde_json::from_str(body).unwrap_or(serde_json::Value::Null);
                    let verifier = req.get("code_verifier").and_then(|v| v.as_str()).unwrap_or("");
                    let expect = challenge2.lock().unwrap().clone();
                    let pkce_ok = !expect.is_empty() && pkce_challenge(verifier) == expect;
                    let shape_ok = req.get("grant_type").and_then(|v| v.as_str()) == Some("authorization_code")
                        && req.get("client_id").and_then(|v| v.as_str()) == Some(CLIENT_ID)
                        && pkce_ok;
                    if mode == "forbid" {
                        json_response(403, "Forbidden", r#"{"error":"forbidden"}"#)
                    } else if mode == "badtoken" {
                        json_response(200, "OK", r#"{"token":"not-a-token","token_type":"Bearer"}"#)
                    } else if mode == "used" || !shape_ok {
                        json_response(400, "Bad Request", r#"{"error":"invalid_grant"}"#)
                    } else {
                        json_response(200, "OK", r#"{"token":"upl_from_the_site","token_type":"Bearer"}"#)
                    }
                } else {
                    json_response(404, "Not Found", r#"{"error":"nope"}"#)
                };
                let _ = sock.write_all(&resp);
            }
        });
        Mock { base: format!("http://127.0.0.1:{port}"), hits, challenge }
    }

    fn drive_browser(challenge_slot: Arc<Mutex<String>>, query: String) -> impl FnMut(&str) -> bool {
        move |url: &str| {
            assert!(!url.contains("upl_"), "token leaked into authorize url");
            assert!(!url.contains("code_verifier"), "verifier leaked into authorize url");
            assert!(url.contains("code_challenge_method=S256"));
            assert!(!url.contains("prompt="), "same-session sign-in must not send prompt");
            assert!(url.contains("127.0.0.1"));
            let challenge = url.split("code_challenge=").nth(1).unwrap().split('&').next().unwrap().to_string();
            *challenge_slot.lock().unwrap() = pct_decode(&challenge);
            let redirect = url.split("redirect_uri=").nth(1).unwrap().split('&').next().unwrap();
            let redirect = pct_decode(redirect);
            let state = url.split("state=").nth(1).unwrap().split('&').next().unwrap();
            let state = pct_decode(&state);
            let target = if query == "use-state" {
                format!("{redirect}?code=abc123&state={state}")
            } else if query == "denied" {
                format!("{redirect}?error=access_denied&state={state}")
            } else if query == "leak" {
                format!("{redirect}?access_token=upl_secret&state={state}")
            } else if query == "wrong-state" {
                format!("{redirect}?code=abc123&state=not-the-state")
            } else {
                format!("{redirect}?{query}")
            };
            let stripped = target.trim_start_matches("http://").to_string();
            // Another thread, because this closure returns before the listener
            // accepts. Retry: a single connect was late on a busy macOS runner
            // and the test then hit its deadline.
            thread::spawn(move || {
                let Some((host, path)) = stripped.split_once('/') else { return };
                let Ok(addr) = host.parse::<std::net::SocketAddr>() else { return };
                let req = format!("GET /{path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n");
                let start = Instant::now();
                while start.elapsed() < Duration::from_secs(4) {
                    if let Ok(mut stream) = TcpStream::connect_timeout(&addr, Duration::from_millis(200)) {
                        let _ = stream.set_write_timeout(Some(Duration::from_secs(2)));
                        let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
                        if stream.write_all(req.as_bytes()).is_ok() {
                            let mut buf = [0u8; 1024];
                            let _ = stream.read(&mut buf);
                            return;
                        }
                    }
                    thread::sleep(Duration::from_millis(20));
                }
            });
            true
        }
    }

    #[test]
    fn missing_route_does_not_open_a_browser_or_return_a_token() {
        let mock = spawn_mock("missing");
        let mut opened = false;
        let err = connect_loopback(&mock.base, Duration::from_secs(5), &AtomicBool::new(false), false, |_| {
            opened = true;
            true
        })
        .unwrap_err();
        assert_eq!(err, AuthError::NotDeployed);
        assert!(!opened);
        assert_eq!(mock.hits.load(Ordering::SeqCst), 0);
        assert!(!err.to_string().contains("upl_"));
    }

    #[test]
    fn success_stores_nothing_by_itself_and_returns_upl_token() {
        let mock = spawn_mock("ok");
        let token = connect_loopback(
            &mock.base,
            Duration::from_secs(5),
            &AtomicBool::new(false), false,
            drive_browser(mock.challenge.clone(), "use-state".into()),
        )
        .unwrap();
        assert_eq!(token, "upl_from_the_site");
        assert_eq!(mock.hits.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn role_denial_returns_forbidden_and_no_token() {
        let mock = spawn_mock("forbid");
        let err = connect_loopback(
            &mock.base,
            Duration::from_secs(5),
            &AtomicBool::new(false), false,
            drive_browser(mock.challenge.clone(), "use-state".into()),
        )
        .unwrap_err();
        assert_eq!(err, AuthError::Forbidden);
        assert!(!err.to_string().contains("upl_"));
        assert!(!err.to_string().contains("Officer"));
    }

    #[test]
    fn invalid_grant_is_distinct_from_denial() {
        let mock = spawn_mock("used");
        let err = connect_loopback(
            &mock.base,
            Duration::from_secs(5),
            &AtomicBool::new(false), false,
            drive_browser(mock.challenge.clone(), "use-state".into()),
        )
        .unwrap_err();
        assert_eq!(err, AuthError::InvalidGrant);
    }

    #[test]
    fn non_upl_body_is_rejected() {
        let mock = spawn_mock("badtoken");
        let err = connect_loopback(
            &mock.base,
            Duration::from_secs(5),
            &AtomicBool::new(false), false,
            drive_browser(mock.challenge.clone(), "use-state".into()),
        )
        .unwrap_err();
        assert_eq!(err, AuthError::BadResponse);
        assert!(!format!("{err}").contains("not-a-token"));
    }

    #[test]
    fn user_decline_does_not_call_the_token_endpoint() {
        let mock = spawn_mock("ok");
        let err = connect_loopback(
            &mock.base,
            Duration::from_secs(5),
            &AtomicBool::new(false), false,
            drive_browser(mock.challenge.clone(), "denied".into()),
        )
        .unwrap_err();
        assert_eq!(err, AuthError::Denied);
        assert_eq!(mock.hits.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn token_in_the_callback_fails_closed() {
        let mock = spawn_mock("ok");
        let err = connect_loopback(
            &mock.base,
            Duration::from_secs(5),
            &AtomicBool::new(false), false,
            drive_browser(mock.challenge.clone(), "leak".into()),
        )
        .unwrap_err();
        assert_eq!(err, AuthError::BadResponse);
        assert_eq!(mock.hits.load(Ordering::SeqCst), 0);
        assert!(!err.to_string().contains("upl_secret"));
    }

    #[test]
    fn cancel_returns_quickly() {
        let mock = spawn_mock("ok");
        let cancel = Arc::new(AtomicBool::new(false));
        let flag = cancel.clone();
        thread::spawn(move || {
            thread::sleep(Duration::from_millis(120));
            flag.store(true, Ordering::SeqCst);
        });
        let start = Instant::now();
        let err = connect_loopback(&mock.base, Duration::from_secs(30), &cancel, false, |_| true).unwrap_err();
        let elapsed = start.elapsed();
        assert_eq!(err, AuthError::Cancelled);
        assert!(elapsed < Duration::from_secs(2), "cancel elapsed {elapsed:?}");
        eprintln!("MEASURE auth_cancel elapsed_ms={}", elapsed.as_millis());
    }

    #[test]
    fn missing_or_unsafe_site_does_not_open_a_browser() {
        for site in ["", "http://evil.example", "https://user:pass@example.com", "https://localhost"] {
            let mut opened = false;
            let err = connect_loopback(site, Duration::from_secs(2), &AtomicBool::new(false), false, |_| {
                opened = true;
                true
            })
            .unwrap_err();
            assert!(!opened, "{site}");
            assert_eq!(err, AuthError::NotDeployed, "{site}");
            assert!(!err.to_string().contains("upl_"));
        }
    }

    #[test]
    fn mismatched_state_does_not_call_the_token_endpoint() {
        let mock = spawn_mock("ok");
        let err = connect_loopback(
            &mock.base,
            Duration::from_millis(400),
            &AtomicBool::new(false), false,
            drive_browser(mock.challenge.clone(), "wrong-state".into()),
        )
        .unwrap_err();
        assert_eq!(err, AuthError::Expired);
        assert_eq!(mock.hits.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn loopback_port_is_not_shared() {
        let listener = bind_loopback().unwrap();
        let port = listener.local_addr().unwrap().port();
        let second = std::net::TcpListener::bind(("127.0.0.1", port));
        assert!(second.is_err(), "a second listener bound the loopback port");
        assert_eq!(listener.local_addr().unwrap().ip(), IpAddr::from([127, 0, 0, 1]));
    }

    #[test]
    fn callback_path_must_be_exact_and_a_token_value_is_a_leak() {
        let ok = parse_callback("/callback?code=abc&state=xyz");
        assert!(ok.is_callback && !ok.leak && ok.code.as_deref() == Some("abc"));
        let other = parse_callback("/callback/extra?code=abc");
        assert!(!other.is_callback);
        let leak = parse_callback("/callback?access_token=upl_nope&state=xyz");
        assert!(leak.leak);
    }

    #[test]
    fn different_account_sends_prompt_login_and_no_token() {
        let mock = spawn_mock("ok");
        let err = connect_loopback(&mock.base, Duration::from_secs(5), &AtomicBool::new(false), true, |url| {
            assert!(url.ends_with("&prompt=login"));
            assert_eq!(url.matches("prompt=").count(), 1);
            assert!(!url.contains("upl_"));
            assert!(!url.contains("code_verifier"));
            assert!(url.contains("redirect_uri="));
            false
        })
        .unwrap_err();
        assert_eq!(err, AuthError::Browser);
        assert_eq!(mock.hits.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn expiry_returns_within_the_deadline() {
        let mock = spawn_mock("ok");
        let start = Instant::now();
        let err = connect_loopback(&mock.base, Duration::from_millis(250), &AtomicBool::new(false), false, |_| true).unwrap_err();
        let elapsed = start.elapsed();
        assert_eq!(err, AuthError::Expired);
        assert!(elapsed < Duration::from_secs(2), "expiry elapsed {elapsed:?}");
        eprintln!("MEASURE auth_expiry elapsed_ms={}", elapsed.as_millis());
    }
}
