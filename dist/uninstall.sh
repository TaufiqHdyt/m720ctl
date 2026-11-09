#!/bin/bash
# Stop and remove the m720ctl login agent and binary. Leaves the config
# (~/.config/m720ctl) and log (~/Library/Logs/m720ctl.log) in place.
set -euo pipefail

LABEL=com.user.m720ctl
BIN="$HOME/.local/bin/m720ctl"
PLIST="$HOME/Library/LaunchAgents/$LABEL.plist"
DOMAIN="gui/$(id -u)"

if launchctl print "$DOMAIN/$LABEL" >/dev/null 2>&1; then
    echo "==> Stopping agent"
    launchctl bootout "$DOMAIN/$LABEL" 2>/dev/null || launchctl unload "$PLIST" 2>/dev/null || true
fi
rm -f "$PLIST" && echo "==> Removed $PLIST"
rm -f "$BIN" && echo "==> Removed $BIN"

cat <<MSG

Done. Optionally also remove:
  * the 'm720ctl' entries in System Settings -> Privacy & Security ->
    Accessibility and Input Monitoring
  * ~/.config/m720ctl and ~/Library/Logs/m720ctl.log
MSG
