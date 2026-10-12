#!/bin/bash
# Development only (WP1.6): run the daemon from this checkout as a per-user LaunchAgent
# until the Homebrew formula exists. Deleted by WP-R. Never run in CI.
#
#   scripts/dev-launchagent.sh install     build release, write the plist, bootstrap it
#   scripts/dev-launchagent.sh uninstall   bootout and remove the plist
set -euo pipefail

LABEL=io.github.rgrvlsk.lull.daemon
PLIST="$HOME/Library/LaunchAgents/$LABEL.plist"
REPO=$(cd "$(dirname "$0")/.." && pwd)
BIN="$REPO/target/release/lull"
LOGS="$HOME/Library/Logs/lull"
DOMAIN="gui/$(id -u)"

case "${1:-}" in
install)
  cargo build --release --manifest-path "$REPO/Cargo.toml"
  mkdir -p "$LOGS" "$(dirname "$PLIST")"
  launchctl bootout "$DOMAIN/$LABEL" 2>/dev/null || true
  cat > "$PLIST" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>$LABEL</string>
  <key>ProgramArguments</key><array><string>$BIN</string><string>daemon</string></array>
  <key>RunAtLoad</key><true/>
  <key>KeepAlive</key><true/>
  <key>ProcessType</key><string>Background</string>
  <key>StandardOutPath</key><string>$LOGS/daemon.log</string>
  <key>StandardErrorPath</key><string>$LOGS/daemon.log</string>
</dict>
</plist>
PLIST
  launchctl bootstrap "$DOMAIN" "$PLIST"
  echo "loaded $LABEL → $BIN daemon (logs: $LOGS/daemon.log)"
  ;;
uninstall)
  launchctl bootout "$DOMAIN/$LABEL" 2>/dev/null || true
  rm -f "$PLIST"
  echo "removed $LABEL"
  ;;
*)
  echo "usage: $0 install|uninstall" >&2
  exit 2
  ;;
esac
