//! WP2.3 — `lull browsers install|uninstall` (contract §9).

use std::process::ExitCode;

pub fn run(_args: Vec<String>) -> ExitCode {
    eprintln!("lull browsers: not implemented yet");
    ExitCode::FAILURE
}

/// `lull doctor` lines `(label, ok, text)` for each detected browser's NM manifest.
pub fn doctor_lines() -> Vec<(String, bool, String)> {
    Vec::new()
}
