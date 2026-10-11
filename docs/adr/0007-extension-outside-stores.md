# The extension ships outside the stores: unpacked on Chromium, unlisted signed XPI on Firefox

v1 does not list the extension on the Chrome Web Store or AMO. The cask carries both
builds. For Chromium browsers, `lull browsers install` copies the extension to a stable
directory (`~/Library/Application Support/lull/extension/chromium`) and guides the user
through "Load unpacked". The manifest's `key` pins the extension id, so the native
messaging manifest's `allowed_origins` is known in advance; the browser re-reads the
directory at startup, so a cask update plus a browser restart updates it. For Firefox and
Zen, which only install signed add-ons, the XPI is signed through AMO's **unlisted**
channel (`web-ext sign --channel unlisted`), keeps the fixed id `lull@rgrvlsk.github.io`,
and declares a `browser_specific_settings.gecko.update_url` pointing at GitHub releases, so
it still auto-updates.

## Considered Options

- **Listed on CWS and AMO.** Easiest for users and auto-updating everywhere, but it puts
  store review and listing upkeep on the release path of a tool whose extension only talks
  to its own native host.

## Consequences

- Chromium users must enable Developer mode, and see whatever developer-mode warning the
  browser shows; Chromium updates need a browser restart after `brew upgrade`.
- The extension signing key (Chromium `key` private half, AMO API credentials) is a
  maintainer secret kept out of the repo.
- Listing on the stores later stays possible: the Chromium id can be kept by uploading the
  same key.
