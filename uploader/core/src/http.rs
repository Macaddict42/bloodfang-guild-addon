//! Every HTTP request the uploader makes goes through these agents.
//!
//! Certificates are checked by the operating system's own verifier (Windows CryptoAPI, macOS
//! Security framework), the same trust the browser uses. A root that antivirus HTTPS scanning or a
//! company proxy added to the system store is trusted here too, and Windows can fetch a missing
//! root on demand. Nothing is bundled: no trust store and no TLS runtime ships with the app.
use std::sync::{Arc, OnceLock};
use std::time::Duration;

/// Which certificate trust the agents use. Kept as a value so a test can pin the choice.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Trust {
    /// The operating system verifier and its trust store (rustls-platform-verifier).
    Os,
}

/// The trust every agent in this crate is built with.
pub const TRUST: Trust = Trust::Os;

pub const USER_AGENT: &str = concat!("guild-uploader/", env!("CARGO_PKG_VERSION"));

fn provider() -> Arc<rustls::crypto::CryptoProvider> {
    Arc::new(rustls::crypto::ring::default_provider())
}

/// TLS client settings that verify with the OS. `extra_roots` exists for tests, which stand in
/// for a root that antivirus software installed into the system store.
pub fn tls_config_with(extra_roots: Vec<rustls::pki_types::CertificateDer<'static>>) -> Result<Arc<rustls::ClientConfig>, String> {
    let provider = provider();
    let verifier = if extra_roots.is_empty() {
        rustls_platform_verifier::Verifier::new(provider.clone())
    } else {
        rustls_platform_verifier::Verifier::new_with_extra_roots(extra_roots, provider.clone())
    }
    .map_err(|e| format!("couldn't load the system certificate store: {e}"))?;
    let config = rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|e| format!("TLS setup failed: {e}"))?
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(verifier))
        .with_no_client_auth();
    Ok(Arc::new(config))
}

/// Built once per process; the system store is read on first use.
pub fn tls_config() -> Result<Arc<rustls::ClientConfig>, String> {
    static CONFIG: OnceLock<Result<Arc<rustls::ClientConfig>, String>> = OnceLock::new();
    CONFIG.get_or_init(|| tls_config_with(Vec::new())).clone()
}

/// Agent with the OS verifier and the given timeouts. Redirects follow ureq's default unless
/// `redirects` is set. If the system store can't be read at all the agent still builds (with
/// ureq's own settings) and the failure is logged, so a broken store shows up in the log.
pub fn agent(connect: Duration, read: Duration, overall: Duration, redirects: Option<u32>) -> ureq::Agent {
    let mut b = ureq::AgentBuilder::new()
        .timeout_connect(connect)
        .timeout_read(read)
        .timeout(overall)
        .user_agent(USER_AGENT);
    if let Some(n) = redirects {
        b = b.redirects(n);
    }
    match tls_config() {
        Ok(cfg) => b.tls_config(cfg).build(),
        Err(e) => {
            crate::log::error("tls", &e);
            b.build()
        }
    }
}

/// Common agent for JSON calls to the site: 5 s connect, 30 s overall.
pub fn site_agent() -> ureq::Agent {
    agent(Duration::from_secs(5), Duration::from_secs(30), Duration::from_secs(30), None)
}

/// Full error text: the error and every `source()` under it, without repeats.
/// `"https://x/: Connection Failed: tls connection init failed: invalid peer certificate: UnknownIssuer"`.
pub fn error_chain(err: &(dyn std::error::Error + 'static)) -> String {
    let mut out = err.to_string();
    let mut cur = err.source();
    while let Some(e) = cur {
        let s = e.to_string();
        if !s.is_empty() && !out.contains(&s) {
            out.push_str(": ");
            out.push_str(&s);
        }
        cur = e.source();
    }
    out
}

/// A ureq failure as one line: HTTP status and a short body, or the transport error chain.
pub fn describe(err: ureq::Error) -> String {
    match err {
        ureq::Error::Status(code, resp) => {
            let body: String = resp.into_string().unwrap_or_default().chars().take(300).collect();
            format!("HTTP {code} {}", body.trim())
        }
        ureq::Error::Transport(t) => error_chain(&t),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;

    #[test]
    fn agents_use_the_os_verifier() {
        assert_eq!(TRUST, Trust::Os);
        // The OS store loads on this machine (CI: Windows, macOS, Linux).
        assert!(tls_config().is_ok(), "{:?}", tls_config().err());
    }

    /// A local HTTPS server whose certificate chains to a private root, like an antivirus
    /// scanner re-signing traffic. The root is unknown to the bundled list and to the OS store,
    /// so the request fails with the real reason in the text. Once the root is trusted by the
    /// verifier (as when antivirus installs its root into the system store) the same request works.
    #[test]
    fn a_private_root_works_once_the_system_trusts_it() {
        let ca_key = rcgen::KeyPair::generate().unwrap();
        let mut ca_params = rcgen::CertificateParams::new(Vec::<String>::new()).unwrap();
        ca_params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
        ca_params.distinguished_name.push(rcgen::DnType::CommonName, "Test Scanner Root");
        let ca = ca_params.self_signed(&ca_key).unwrap();
        let leaf_key = rcgen::KeyPair::generate().unwrap();
        let leaf_params = rcgen::CertificateParams::new(vec!["localhost".to_string()]).unwrap();
        let leaf = leaf_params.signed_by(&leaf_key, &ca, &ca_key).unwrap();

        let server_cfg = Arc::new(
            rustls::ServerConfig::builder_with_provider(provider())
                .with_safe_default_protocol_versions()
                .unwrap()
                .with_no_client_auth()
                .with_single_cert(
                    vec![leaf.der().clone(), ca.der().clone()],
                    rustls::pki_types::PrivateKeyDer::Pkcs8(leaf_key.serialize_der().into()),
                )
                .unwrap(),
        );
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            for stream in listener.incoming().take(2) {
                let Ok(mut tcp) = stream else { continue };
                let cfg = server_cfg.clone();
                std::thread::spawn(move || {
                    let mut conn = rustls::ServerConnection::new(cfg).unwrap();
                    let mut tls = rustls::Stream::new(&mut conn, &mut tcp);
                    let mut buf = [0u8; 2048];
                    if tls.read(&mut buf).is_ok() {
                        let _ = tls.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok");
                        let _ = tls.flush();
                    }
                });
            }
        });
        let url = format!("https://localhost:{port}/");
        let t = Duration::from_secs(5);

        let plain = agent(t, t, t, None);
        let err = plain.get(&url).call().unwrap_err();
        let text = describe(err);
        assert!(text.to_lowercase().contains("certificate"), "{text}");

        // Windows' CryptoAPI chain engine doesn't anchor on in-process extra roots (it reports an
        // untrusted root), so the "now trusted" half runs where it is honoured; on Windows the
        // real path is a root in the system store, which this test can't install on a CI runner.
        if cfg!(windows) {
            return;
        }
        let trusted = ureq::AgentBuilder::new()
            .timeout(t)
            .tls_config(tls_config_with(vec![ca.der().clone()]).unwrap())
            .build();
        let body = trusted.get(&url).call().unwrap().into_string().unwrap();
        assert_eq!(body, "ok");
    }

    #[test]
    fn error_chain_keeps_every_cause_once() {
        #[derive(Debug)]
        struct E(&'static str, Option<Box<E>>);
        impl std::fmt::Display for E {
            fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result { f.write_str(self.0) }
        }
        impl std::error::Error for E {
            fn source(&self) -> Option<&(dyn std::error::Error + 'static)> { self.1.as_deref().map(|e| e as _) }
        }
        let e = E("connection failed", Some(Box::new(E("tls init failed", Some(Box::new(E("UnknownIssuer", None)))))));
        assert_eq!(error_chain(&e), "connection failed: tls init failed: UnknownIssuer");
    }
}
