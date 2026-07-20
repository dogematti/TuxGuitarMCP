#!/bin/sh
# Rebuild the plugin, reinstall it, restart TuxGuitar, wait for the bridge.
set -eu
APP="${TUXGUITAR_APP:-/Applications/tuxguitar-2.0.1-macosx-swt-cocoa-x86_64.app}"
cd "$(dirname "$0")/.."
( cd tuxguitar-mcp-bridge && mvn -q package -DskipTests )
cp tuxguitar-mcp-bridge/target/tuxguitar-mcp-bridge.jar "$APP/Contents/MacOS/share/plugins/"
osascript -e "tell application \"$(basename "$APP" .app)\" to quit" >/dev/null 2>&1 || true
i=0; while pgrep -f "$(basename "$APP" .app)" >/dev/null && [ $i -lt 15 ]; do sleep 1; i=$((i+1)); done
pgrep -f "$(basename "$APP" .app)" >/dev/null && { echo "force-killing TuxGuitar (unsaved changes discarded)"; pkill -9 -f "$(basename "$APP" .app)"; sleep 1; }
rm -f "$HOME/.tuxguitar-mcp/bridge.json"
open -a "$APP"
i=0; while [ ! -f "$HOME/.tuxguitar-mcp/bridge.json" ] && [ $i -lt 30 ]; do sleep 1; i=$((i+1)); done
[ -f "$HOME/.tuxguitar-mcp/bridge.json" ] && echo "bridge up" || { echo "bridge did not come up"; exit 1; }
