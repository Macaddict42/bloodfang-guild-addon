use std::env;
use std::fs;
use std::process::ExitCode;

fn main() -> ExitCode {
    let mut args = env::args().skip(1);
    let mut payload = None;
    let mut sha = None;
    let mut signature = None;
    let mut version = "0.1.0-private".to_string();
    while let Some(arg) = args.next() {
        let value = args.next();
        match arg.as_str() {
            "--payload" => payload = value,
            "--sha256" => sha = value,
            "--signature" => signature = value,
            "--version" => version = value.unwrap_or_default(),
            "--show-trust" => {
                match guild_uploader_app_lib::compiled_update_key_sha256() {
                    Some(fingerprint) => {
                        println!("trust_present=true");
                        println!("trust_sha256={fingerprint}");
                        return ExitCode::SUCCESS;
                    }
                    None => {
                        println!("trust_present=false");
                        return ExitCode::from(3);
                    }
                }
            }
            _ => return usage(),
        }
    }
    let (Some(payload), Some(sha), Some(signature)) = (payload, sha, signature) else { return usage() };
    let installer = match fs::read(&payload) {
        Ok(bytes) => bytes,
        Err(err) => {
            eprintln!("payload: {err}");
            return ExitCode::from(64);
        }
    };
    let signature = match fs::read(&signature) {
        Ok(bytes) => bytes,
        Err(err) => {
            eprintln!("signature: {err}");
            return ExitCode::from(64);
        }
    };
    match guild_uploader_app_lib::compiled_update_key_sha256() {
        Some(fingerprint) => println!("trust_sha256={fingerprint}"),
        None => {
            println!("trust_present=false");
            return ExitCode::from(3);
        }
    }
    let result = guild_uploader_app_lib::check_installer_bytes(&version, &installer, &sha, &signature);
    let verified = result.message.contains("was verified");
    println!("verified={verified}");
    println!("installed={}", result.installed);
    println!("restarted={}", result.restarted);
    println!("message={}", result.message);
    if verified && !result.installed && !result.restarted {
        ExitCode::SUCCESS
    } else if result.installed || result.restarted {
        ExitCode::from(4)
    } else {
        ExitCode::from(2)
    }
}

fn usage() -> ExitCode {
    eprintln!("usage: updater --show-trust");
    eprintln!("       updater --payload FILE --sha256 HEX --signature FILE [--version TEXT]");
    ExitCode::from(64)
}
