//! Update now downloads and starts an installer only after the signature check.
//! This build has no trusted public key, so the command stops before any download.
//! A published file alone does not pass: the key has to be present at compile time
//! as GUILD_UPLOADER_UPDATE_PUBKEY (64 hex characters). No private key is stored here.

use guild_uploader_core as core;
use ring::signature::{UnparsedPublicKey, ED25519};

pub const NO_TRUST_KEY: &str = "This build has no trusted update key, and no verified installer is published. Nothing was downloaded or restarted.";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Choice {
    Later,
    Dismiss,
    UpdateNow,
}

pub struct Trust {
    pub public_key: Option<Vec<u8>>,
}

pub struct Fetched {
    pub version: String,
    pub installer: Vec<u8>,
    pub sha256_hex: String,
    pub signature: Vec<u8>,
}

pub enum FetchFail {
    Network(String),
    MissingMetadata(String),
}

pub struct AcceptResult {
    pub installed: bool,
    pub restarted: bool,
    pub message: String,
}

pub fn trust_from_build() -> Trust {
    Trust { public_key: decode_public_key(option_env!("GUILD_UPLOADER_UPDATE_PUBKEY").unwrap_or("")) }
}

pub fn should_offer(current: &str, latest: Option<&str>, deferred: Option<&str>) -> bool {
    match latest {
        Some(latest) if version_gt(latest, current) && deferred != Some(latest) => true,
        _ => false,
    }
}

fn version_gt(latest: &str, current: &str) -> bool {
    match (semver::Version::parse(latest), semver::Version::parse(current)) {
        (Ok(latest), Ok(current)) => latest > current,
        _ => false,
    }
}

fn decode_hex(text: &str) -> Option<Vec<u8>> {
    let text = text.trim();
    if text.is_empty() || text.len() % 2 != 0 || !text.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(text.len() / 2);
    for i in (0..bytes.len()).step_by(2) {
        let hi = hex_nibble(bytes[i])?;
        let lo = hex_nibble(bytes[i + 1])?;
        out.push((hi << 4) | lo);
    }
    Some(out)
}

pub fn decode_public_key(hex_key: &str) -> Option<Vec<u8>> {
    let out = decode_hex(hex_key)?;
    if out.len() == 32 { Some(out) } else { None }
}

fn hex_nibble(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn signature_bytes(raw: &[u8]) -> Option<Vec<u8>> {
    if raw.len() == 64 {
        return Some(raw.to_vec());
    }
    if raw.len() == 128 && raw.iter().all(|b| b.is_ascii_hexdigit()) {
        let out = decode_hex(std::str::from_utf8(raw).ok()?)?;
        if out.len() == 64 { return Some(out); }
    }
    None
}

fn signature_ok(public_key: &[u8], message: &[u8], signature: &[u8]) -> bool {
    if public_key.len() != 32 {
        return false;
    }
    let Some(signature) = signature_bytes(signature) else { return false };
    UnparsedPublicKey::new(&ED25519, public_key).verify(message, &signature).is_ok()
}

fn refused(message: impl Into<String>) -> AcceptResult {
    AcceptResult { installed: false, restarted: false, message: message.into() }
}

/// Later and dismiss do not fetch. Update now fetches, checks the hash and signature, then calls `install`.
pub fn decide<F, I>(choice: Choice, trust: &Trust, fetch: F, install: I) -> AcceptResult
where
    F: FnOnce() -> Result<Fetched, FetchFail>,
    I: FnOnce(&[u8]) -> Result<(), String>,
{
    match choice {
        Choice::Later | Choice::Dismiss => refused("The running app was left as it is. Nothing was downloaded or restarted."),
        Choice::UpdateNow => accept_verified(trust, fetch, install),
    }
}

pub fn accept_verified<F, I>(trust: &Trust, fetch: F, install: I) -> AcceptResult
where
    F: FnOnce() -> Result<Fetched, FetchFail>,
    I: FnOnce(&[u8]) -> Result<(), String>,
{
    let Some(key) = trust.public_key.as_deref() else {
        return refused(NO_TRUST_KEY);
    };
    if key.len() != 32 {
        return refused(NO_TRUST_KEY);
    }
    let fetched = match fetch() {
        Ok(fetched) => fetched,
        Err(FetchFail::Network(err)) => return refused(format!("Couldn't reach the update. Nothing was downloaded or restarted. {err}")),
        Err(FetchFail::MissingMetadata(err)) => return refused(format!("The update is missing required files. Nothing was installed or restarted. {err}")),
    };
    let actual = core::sha256_hex(&fetched.installer);
    if actual != fetched.sha256_hex.to_ascii_lowercase() {
        return refused("The download didn't match SHA256SUMS. Nothing was installed or restarted.");
    }
    if !signature_ok(key, &fetched.installer, &fetched.signature) {
        return refused("The installer signature did not match the trusted key. Nothing was installed or restarted.");
    }
    match install(&fetched.installer) {
        Ok(()) => AcceptResult {
            installed: true,
            restarted: false,
            message: format!("Verified {}. The installer was started. This app was not restarted by the updater.", fetched.version),
        },
        Err(err) => refused(format!("The installer was verified and was not started. {err}")),
    }
}

pub fn fetch_published(repo: &str) -> Result<Fetched, FetchFail> {
    if repo.is_empty() {
        return Err(FetchFail::MissingMetadata("no update repository is configured".into()));
    }
    let release = match core::latest_app(repo) {
        Ok(release) => release,
        Err(core::UpdateError::Network(err)) => return Err(FetchFail::Network(err)),
        Err(err) => return Err(FetchFail::MissingMetadata(err.to_string())),
    };
    let Some(release) = release else {
        return Err(FetchFail::MissingMetadata("no uploader release is published".into()));
    };
    let Some(sums_url) = release.sums_url.clone() else {
        return Err(FetchFail::MissingMetadata("release has no SHA256SUMS".into()));
    };
    let Some(sig_url) = release.sig_url.clone() else {
        return Err(FetchFail::MissingMetadata("release has no signature".into()));
    };
    let installer = match core::download_bytes(&release.zip_url) {
        Ok(bytes) => bytes,
        Err(core::UpdateError::Network(err)) => return Err(FetchFail::Network(err)),
        Err(err) => return Err(FetchFail::MissingMetadata(err.to_string())),
    };
    let sums = match core::download_text(&sums_url) {
        Ok(text) => text,
        Err(core::UpdateError::Network(err)) => return Err(FetchFail::Network(err)),
        Err(err) => return Err(FetchFail::MissingMetadata(err.to_string())),
    };
    let signature = match core::download_bytes(&sig_url) {
        Ok(bytes) => bytes,
        Err(core::UpdateError::Network(err)) => return Err(FetchFail::Network(err)),
        Err(err) => return Err(FetchFail::MissingMetadata(err.to_string())),
    };
    let sha256_hex = core::sums_entry(&sums, &release.zip_name).ok_or_else(|| {
        FetchFail::MissingMetadata(format!("{} missing from SHA256SUMS", release.zip_name))
    })?;
    Ok(Fetched { version: release.version.to_string(), installer, sha256_hex, signature })
}

/// macOS replaces the bundle itself and restarts, so the Windows wording doesn't fit.
pub fn mac_installed_message(verified: &str) -> String {
    let version = verified.strip_prefix("Verified ").and_then(|r| r.split(". ").next()).unwrap_or("the update");
    format!("Verified {version}. The app was updated and is restarting.")
}

/// Writes the already-verified bytes and starts that file. Tests do not call this with real bytes.
pub fn launch_verified_installer(bytes: &[u8]) -> Result<(), String> {
    if bytes.is_empty() {
        return Err("The verified installer was empty, so it was not started.".into());
    }
    let path = std::env::temp_dir().join("guild-uploader-update.bin");
    std::fs::write(&path, bytes).map_err(|e| e.to_string())?;
    // CREATE_NO_WINDOW suppresses a console. A GUI installer still shows its own windows.
    guild_uploader_core::background_command(&path).spawn().map_err(|e| format!("{e}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ring::rand::SystemRandom;
    use ring::signature::{Ed25519KeyPair, KeyPair};

    #[test]
    fn mac_message_names_the_version() {
        assert_eq!(mac_installed_message("Verified 0.1.3. The installer was started. This app was not restarted by the updater."), "Verified 0.1.3. The app was updated and is restarting.");
    }

    #[test]
    fn later_hides_that_version_and_a_newer_one_asks_again() {
        assert!(should_offer("0.1.0", Some("0.2.0"), None));
        assert!(!should_offer("0.1.0", Some("0.2.0"), Some("0.2.0")));
        assert!(should_offer("0.1.0", Some("0.3.0"), Some("0.2.0")));
        assert!(!should_offer("0.1.0", Some("0.1.0"), None));
        assert!(!should_offer("0.1.0", None, None));
    }

    #[test]
    fn compiled_trust_fails_closed_or_rejects_tampering() {
        assert!(NO_TRUST_KEY.contains("no trusted update key"));
        let trust = trust_from_build();
        match trust.public_key.clone() {
            None => {
                let mut fetches = 0;
                let result = decide(
                    Choice::UpdateNow,
                    &trust,
                    || { fetches += 1; Err(FetchFail::Network("must not fetch".into())) },
                    |_| panic!("install must not run"),
                );
                assert!(!result.installed);
                assert!(!result.restarted);
                assert!(result.message.contains("no trusted update key"));
                assert_eq!(fetches, 0);
            }
            Some(key) => {
                assert_eq!(key.len(), 32);
                if let Ok(configured) = std::env::var("GUILD_UPLOADER_UPDATE_PUBKEY") {
                    assert_eq!(decode_public_key(&configured).as_deref(), Some(key.as_slice()));
                }
                let installer = b"private-test-payload";
                let mut installs = 0;
                let bad_sig = decide(
                    Choice::UpdateNow,
                    &trust,
                    || Ok(Fetched {
                        version: "0.1.0-private".into(),
                        installer: installer.to_vec(),
                        sha256_hex: core::sha256_hex(installer),
                        signature: vec![0u8; 64],
                    }),
                    |_| { installs += 1; Ok(()) },
                );
                assert!(!bad_sig.installed);
                assert!(!bad_sig.restarted);
                assert!(bad_sig.message.contains("signature"));
                assert_eq!(installs, 0);
                let bad_sha = decide(
                    Choice::UpdateNow,
                    &trust,
                    || Ok(Fetched {
                        version: "0.1.0-private".into(),
                        installer: installer.to_vec(),
                        sha256_hex: "00".into(),
                        signature: vec![0u8; 64],
                    }),
                    |_| { installs += 1; Ok(()) },
                );
                assert!(!bad_sha.installed);
                assert!(bad_sha.message.contains("SHA256SUMS"));
                assert_eq!(installs, 0);
            }
        }
    }

    fn fixture() -> (Vec<u8>, Fetched) {
        let rng = SystemRandom::new();
        let pkcs8 = Ed25519KeyPair::generate_pkcs8(&rng).unwrap();
        let pair = Ed25519KeyPair::from_pkcs8(pkcs8.as_ref()).unwrap();
        let installer = b"fake-installer-not-executed".to_vec();
        let signature = pair.sign(&installer).as_ref().to_vec();
        let fetched = Fetched {
            version: "0.2.0".into(),
            sha256_hex: core::sha256_hex(&installer),
            installer,
            signature,
        };
        (pair.public_key().as_ref().to_vec(), fetched)
    }

    #[test]
    fn later_and_dismiss_do_not_download_or_install() {
        let (key, fetched) = fixture();
        let trust = Trust { public_key: Some(key) };
        for choice in [Choice::Later, Choice::Dismiss] {
            let mut fetched_calls = 0;
            let mut install_calls = 0;
            let result = decide(
                choice,
                &trust,
                || {
                    fetched_calls += 1;
                    Ok(Fetched { version: fetched.version.clone(), installer: fetched.installer.clone(), sha256_hex: fetched.sha256_hex.clone(), signature: fetched.signature.clone() })
                },
                |_| {
                    install_calls += 1;
                    Ok(())
                },
            );
            assert!(!result.installed);
            assert!(!result.restarted);
            assert_eq!(fetched_calls, 0);
            assert_eq!(install_calls, 0);
        }
    }

    #[test]
    fn update_now_installs_only_after_the_signature_matches() {
        let (key, fetched) = fixture();
        let trust = Trust { public_key: Some(key.clone()) };
        let mut installed: Option<Vec<u8>> = None;
        let result = decide(
            Choice::UpdateNow,
            &trust,
            || Ok(Fetched { version: fetched.version.clone(), installer: fetched.installer.clone(), sha256_hex: fetched.sha256_hex.clone(), signature: fetched.signature.clone() }),
            |bytes| {
                installed = Some(bytes.to_vec());
                Ok(())
            },
        );
        assert!(result.installed);
        assert!(!result.restarted);
        assert_eq!(installed.as_deref(), Some(fetched.installer.as_slice()));
        assert!(result.message.contains("not restarted"));

        let mut install_calls = 0;
        let bad_sig = decide(
            Choice::UpdateNow,
            &trust,
            || {
                let mut signature = fetched.signature.clone();
                signature[0] ^= 0xff;
                Ok(Fetched { version: fetched.version.clone(), installer: fetched.installer.clone(), sha256_hex: fetched.sha256_hex.clone(), signature })
            },
            |_| { install_calls += 1; Ok(()) },
        );
        assert!(!bad_sig.installed);
        assert!(bad_sig.message.contains("signature"));
        assert_eq!(install_calls, 0);

        let bad_sha = decide(
            Choice::UpdateNow,
            &trust,
            || Ok(Fetched { version: fetched.version.clone(), installer: fetched.installer.clone(), sha256_hex: "00".into(), signature: fetched.signature.clone() }),
            |_| { install_calls += 1; Ok(()) },
        );
        assert!(!bad_sha.installed);
        assert!(bad_sha.message.contains("SHA256SUMS"));
        assert_eq!(install_calls, 0);
    }

    #[test]
    fn missing_metadata_network_and_absent_key_do_not_install() {
        let (key, fetched) = fixture();
        let trust = Trust { public_key: Some(key) };
        let mut install_calls = 0;
        let missing = decide(Choice::UpdateNow, &trust, || Err(FetchFail::MissingMetadata("release has no signature".into())), |_| { install_calls += 1; Ok(()) });
        assert!(!missing.installed);
        assert!(missing.message.contains("missing"));
        let network = decide(Choice::UpdateNow, &trust, || Err(FetchFail::Network("offline".into())), |_| { install_calls += 1; Ok(()) });
        assert!(!network.installed);
        assert!(network.message.contains("Nothing was downloaded"));
        let mut fetch_calls = 0;
        let no_key = decide(
            Choice::UpdateNow,
            &Trust { public_key: None },
            || { fetch_calls += 1; Ok(Fetched { version: fetched.version.clone(), installer: fetched.installer.clone(), sha256_hex: fetched.sha256_hex.clone(), signature: fetched.signature.clone() }) },
            |_| { install_calls += 1; Ok(()) },
        );
        assert!(!no_key.installed);
        assert!(no_key.message.contains("no trusted update key"));
        assert_eq!(fetch_calls, 0);
        assert_eq!(install_calls, 0);
        assert!(launch_verified_installer(&[]).is_err());
    }
}
