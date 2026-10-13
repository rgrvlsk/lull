//! Open `lull ui` from the foreground daemon in a new tab of the terminal it runs in:
//! Ghostty 1.3+ and iTerm2 get a tab, Terminal.app a window (it has no scriptable tabs),
//! through AppleScript. Other terminals get the command to run instead.

use std::process::Command;

/// What a new UI would open in, for the daemon's banner; `None` when we cannot script it.
pub fn target() -> Option<&'static str> {
    match std::env::var("TERM_PROGRAM").ok()?.as_str() {
        "ghostty" => Some("a new Ghostty tab"),
        "iTerm.app" => Some("a new iTerm2 tab"),
        "Apple_Terminal" => Some("a new Terminal window"),
        _ => None,
    }
}

/// The shell command that opens the UI against this daemon (`LULL_DIR` included).
pub fn command() -> String {
    let exe = std::env::current_exe().map_or("lull".into(), |p| p.display().to_string());
    match std::env::var("LULL_DIR") {
        Ok(dir) => format!("LULL_DIR={} {} ui", sh(&dir), sh(&exe)),
        Err(_) => format!("{} ui", sh(&exe)),
    }
}

/// What to tell a person to type: plain `lull` unless this daemon uses its own `LULL_DIR`.
pub fn typed() -> String {
    match std::env::var_os("LULL_DIR") {
        Some(_) => command(),
        None => "lull".into(),
    }
}

/// Open the UI; `Ok` says where, `Err` says what to do instead.
pub fn open() -> Result<&'static str, String> {
    let place = target().ok_or_else(|| format!("run `{}` in another tab", typed()))?;
    let script = match place {
        // `command` runs through the shell; the tab closes when the UI quits.
        "a new Ghostty tab" => format!(
            "tell application \"Ghostty\" to new tab in front window with configuration {{command:{}}}",
            applescript(&command())
        ),
        "a new iTerm2 tab" => format!(
            "tell application \"iTerm\"\ntell current window\n\
             set t to (create tab with default profile)\n\
             tell current session of t to write text {}\nend tell\nend tell",
            applescript(&command())
        ),
        _ => format!(
            "tell application \"Terminal\" to do script {}",
            applescript(&command())
        ),
    };
    let out = Command::new("/usr/bin/osascript")
        .args(["-e", &script])
        .output()
        .map_err(|e| e.to_string())?;
    if out.status.success() {
        return Ok(place);
    }
    let err = String::from_utf8_lossy(&out.stderr);
    if err.contains("-1743") {
        return Err(
            "macOS did not let lull control the terminal; allow it in System Settings › \
                    Privacy & Security › Automation"
                .into(),
        );
    }
    Err(format!(
        "could not open the UI ({}); run `{}`",
        err.trim(),
        command()
    ))
}

/// Single-quoted for `sh`.
fn sh(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

/// A double-quoted AppleScript string literal.
fn applescript(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', r"\\").replace('"', r#"\""#))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quoting_survives_quotes_and_backslashes() {
        assert_eq!(sh("/a b/it's"), r"'/a b/it'\''s'");
        assert_eq!(applescript(r#"say "hi" \ bye"#), r#""say \"hi\" \\ bye""#);
    }
}
