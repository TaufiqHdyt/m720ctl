#!/bin/bash
# Build, sign, and install m720ctl as a launchd login agent.
#
# Signing: by default the binary is ad-hoc signed. macOS ties an ad-hoc
# binary's Accessibility/Input Monitoring grant to its exact hash, so a
# *changed* binary needs re-granting. To keep the grant across rebuilds, sign
# with a real identity from your keychain (an Apple Development cert or a
# self-signed code-signing cert):
#   M720CTL_SIGN_IDENTITY="Apple Development: you@example.com" ./dist/install.sh
set -euo pipefail

LABEL=com.user.m720ctl
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
BIN_DIR="$HOME/.local/bin"
BIN="$BIN_DIR/m720ctl"
PLIST="$HOME/Library/LaunchAgents/$LABEL.plist"
LOG="$HOME/Library/Logs/m720ctl.log"
DOMAIN="gui/$(id -u)"
IDENTITY="${M720CTL_SIGN_IDENTITY:--}"

[[ "$(uname -s)/$(uname -m)" == "Darwin/arm64" ]] || { echo "Apple Silicon macOS only" >&2; exit 1; }

echo "==> Building"
cargo build --release --manifest-path "$ROOT/Cargo.toml"

echo "==> Signing (identity: $IDENTITY)"
STAGED="$ROOT/target/release/m720ctl.signed"
cp "$ROOT/target/release/m720ctl" "$STAGED"
codesign --sign "$IDENTITY" --force --identifier "$LABEL" "$STAGED"

mkdir -p "$BIN_DIR" "$(dirname "$PLIST")" "$(dirname "$LOG")"
if launchctl print "$DOMAIN/$LABEL" >/dev/null 2>&1; then
    echo "==> Stopping running agent"
    launchctl bootout "$DOMAIN/$LABEL" 2>/dev/null || launchctl unload "$PLIST" 2>/dev/null || true
fi

CHANGED=1
if [[ -f "$BIN" ]] && cmp -s "$STAGED" "$BIN"; then
    CHANGED=0
    echo "==> Binary unchanged; existing permission grants still apply"
else
    echo "==> Installing $BIN"
    # Replace (not overwrite in place) so the kernel's signature cache sees a new file.
    rm -f "$BIN"
    cp "$STAGED" "$BIN"
fi

echo "==> Installing $PLIST"
sed -e "s|__BIN__|$BIN|g" -e "s|__LOG__|$LOG|g" "$ROOT/dist/$LABEL.plist" > "$PLIST"
plutil -lint -s "$PLIST"

echo "==> Loading agent"
launchctl bootstrap "$DOMAIN" "$PLIST" 2>/dev/null || launchctl load -w "$PLIST"

cat <<MSG

Installed. Logs: $LOG
MSG
if [[ $CHANGED == 1 ]]; then
    cat <<MSG
Grant permissions (System Settings -> Privacy & Security):
  * Accessibility
  * Input Monitoring
Turn on 'm720ctl' in both lists; use '+' and pick $BIN if it isn't listed.
If an old 'm720ctl' entry is already there, remove it with '-' and add it again
(a rebuilt ad-hoc binary doesn't match the old grant).
The agent waits and starts working once both are granted; no restart needed.
Opening the settings panes...
MSG
    open "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility" || true
    open "x-apple.systempreferences:com.apple.preference.security?Privacy_ListenEvent" || true
fi
echo "Status: launchctl print $DOMAIN/$LABEL"
