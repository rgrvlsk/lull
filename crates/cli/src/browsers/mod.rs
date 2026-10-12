//! WP2.3 — `lull browsers install|uninstall` (contract §9), and the identifiers the
//! native-messaging host validates callers against.

use std::process::ExitCode;

/// Native-messaging host name (§14).
#[allow(dead_code)] // used by WP2.1/WP2.3; drop once they land
pub const NM_HOST_NAME: &str = "io.github.rgrvlsk.lull";

/// Chromium extension id, pinned by the manifest `key` (ADR-0007).
// ponytail: placeholder until WP-R generates the key pair (needs the author's go-ahead);
// nm-host and the NM manifests both read this one constant.
#[allow(dead_code)] // used by WP2.1/WP2.3; drop once they land
pub const CHROMIUM_EXTENSION_ID: &str = "lullplaceholderlullplaceholderaa";

/// Firefox/Zen add-on id (v1.1, §9).
#[allow(dead_code)] // used by WP2.1/WP2.3; drop once they land
pub const FIREFOX_ADDON_ID: &str = "lull@rgrvlsk.github.io";

pub fn run(_args: Vec<String>) -> ExitCode {
    eprintln!("lull browsers: not implemented yet");
    ExitCode::FAILURE
}

/// `lull doctor` lines `(label, ok, text)` for each detected browser's NM manifest.
pub fn doctor_lines() -> Vec<(String, bool, String)> {
    Vec::new()
}
