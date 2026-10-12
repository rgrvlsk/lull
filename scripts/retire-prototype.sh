#!/bin/bash
# M1 cutover (WP1.6): retire the bash devserver-reaper prototype. Run by hand, once,
# right before `scripts/dev-launchagent.sh install`. Never run in CI.
set -euo pipefail

LABEL=local.devserver-reaper
launchctl bootout "gui/$(id -u)/$LABEL" 2>/dev/null && echo "booted out $LABEL" || echo "$LABEL was not loaded"
rm -f "$HOME/Library/LaunchAgents/$LABEL.plist"
rm -rf "$HOME/.local/state/devserver-reaper"
echo "removed its plist and state."
echo "Left in place: ~/.local/bin/devserver-reaper and ~/Library/Logs/devserver-reaper.log (delete by hand if unwanted)."
