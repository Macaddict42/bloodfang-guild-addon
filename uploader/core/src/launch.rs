//! Quote a Windows Run-key command so a path with spaces stays one program.

/// `"C:\Program Files\Guild Uploader\app.exe" --hidden`
/// The executable is quoted. Arguments must not contain spaces or quotes.
pub fn quote_windows_run(exe: &str, args: &[&str]) -> Result<String, String> {
    if exe.is_empty() || exe.contains('"') {
        return Err("can't register start at login for this install path".into());
    }
    let mut out = format!("\"{exe}\"");
    for arg in args {
        if arg.is_empty() || arg.contains('"') || arg.contains(' ') || arg.contains('\t') {
            return Err("can't register start at login for this install path".into());
        }
        out.push(' ');
        out.push_str(arg);
    }
    if !out.starts_with('"') {
        return Err("can't register start at login for this install path".into());
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spaces_in_the_exe_path_stay_inside_quotes() {
        let line = quote_windows_run(r"C:\Program Files\Guild Uploader\guild-uploader-app.exe", &["--hidden"]).unwrap();
        assert_eq!(line, r#""C:\Program Files\Guild Uploader\guild-uploader-app.exe" --hidden"#);
        assert!(line.starts_with('"'));
        assert!(!line.starts_with(r"C:\Program "));
    }

    #[test]
    fn a_quote_in_the_path_is_rejected() {
        assert!(quote_windows_run(r#"C:\bad"path\app.exe"#, &["--hidden"]).is_err());
    }
}
