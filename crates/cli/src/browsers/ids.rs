//! Identifiers shared by nm-host, `lull browsers install` and their tests (§14).

/// Native-messaging host name (§14).
pub const NM_HOST_NAME: &str = "io.github.rgrvlsk.lull";

/// Chromium extension id, pinned by the manifest `key` (ADR-0007).
// ponytail: placeholder until WP-R generates the key pair (needs the author's go-ahead);
// nm-host and the NM manifests both read this one constant.
pub const CHROMIUM_EXTENSION_ID: &str = "lullplaceholderlullplaceholderaa";

/// Firefox/Zen add-on id (v1.1, §9).
pub const FIREFOX_ADDON_ID: &str = "lull@rgrvlsk.github.io";
