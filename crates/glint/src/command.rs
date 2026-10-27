//! Run-box commands: ">cmd /k ver" in the search box, and the commands
//! bound to custom shortcuts. Parsing is shared; launching is per platform.

/// Splits a command line into the program and its arguments, the way the
/// Run dialog reads it: a quoted first part, or everything up to the first
/// space. `%VAR%` references are expanded.
pub fn split(line: &str) -> Option<(String, String)> {
    let line = expand_vars(line.trim());
    if line.is_empty() {
        return None;
    }
    let (program, args) = if let Some(rest) = line.strip_prefix('"') {
        match rest.split_once('"') {
            Some((p, a)) => (p.to_string(), a.trim().to_string()),
            None => (rest.to_string(), String::new()),
        }
    } else {
        match line.split_once(char::is_whitespace) {
            Some((p, a)) => (p.to_string(), a.trim().to_string()),
            None => (line.clone(), String::new()),
        }
    };
    Some((program, args))
}

/// Replaces `%NAME%` with the environment variable, leaving unknown ones.
fn expand_vars(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(start) = rest.find('%') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        match after.find('%') {
            Some(end) if end > 0 && !after[..end].contains(char::is_whitespace) => {
                let name = &after[..end];
                match std::env::var(name) {
                    Ok(v) => out.push_str(&v),
                    Err(_) => {
                        out.push('%');
                        out.push_str(name);
                        out.push('%');
                    }
                }
                rest = &after[end + 1..];
            }
            _ => {
                out.push('%');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// The text after the ">" that turns the search box into a run box.
pub fn from_query(query: &str) -> Option<&str> {
    query.trim_start().strip_prefix('>').map(str::trim)
}

/// Runs `line`, as administrator when `admin` is set (Windows asks for
/// consent unless Glint itself runs elevated). False if it couldn't start.
#[cfg(windows)]
pub fn run(line: &str, admin: bool) -> bool {
    let Some((program, args)) = split(line) else { return false };
    match crate::win::shell::execute(&program, &args, admin) {
        Ok(()) => true,
        // Windows Terminal is missing on older Windows 10: use PowerShell.
        Err(not_found) if not_found && matches!(program.to_ascii_lowercase().as_str(), "wt" | "wt.exe") => {
            crate::win::shell::execute("powershell.exe", "", admin).is_ok()
        }
        Err(_) => false,
    }
}

#[cfg(not(windows))]
pub fn run(line: &str, admin: bool) -> bool {
    eprintln!("run {line:?} admin={admin}");
    split(line).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits() {
        assert_eq!(split("cmd /k ver"), Some(("cmd".into(), "/k ver".into())));
        assert_eq!(split(r#""C:\Program Files\app.exe" -x"#), Some((r"C:\Program Files\app.exe".into(), "-x".into())));
        assert_eq!(split("  notepad  "), Some(("notepad".into(), String::new())));
        assert_eq!(split(""), None);
        std::env::set_var("GLINT_TEST_DIR", "C:\\tmp");
        assert_eq!(split("explorer %GLINT_TEST_DIR%"), Some(("explorer".into(), "C:\\tmp".into())));
        assert_eq!(split("echo 50% done"), Some(("echo".into(), "50% done".into())));
        assert_eq!(from_query(" >cmd"), Some("cmd"));
        assert_eq!(from_query("cmd"), None);
    }
}
