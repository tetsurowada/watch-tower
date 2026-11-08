#!/bin/sh
# Runs the spike once through launchd, as watch-tower will run in production.
# Usage: ./launchd-test.sh <folder>
set -eu
cd "$(dirname "$0")"
FOLDER=${1:?usage: ./launchd-test.sh <folder>}
APP_BIN="$PWD/target/WatchTowerSpike.app/Contents/MacOS/macos-notify"
LABEL=dev.watch-tower.spike.launchd
PLIST="$PWD/target/$LABEL.plist"
LOG=/tmp/watch-tower-spike.log

cat > "$PLIST" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>$LABEL</string>
  <key>ProgramArguments</key>
  <array>
    <string>$APP_BIN</string>
    <string>$FOLDER</string>
    <string>via launchd</string>
  </array>
  <key>RunAtLoad</key><true/>
  <key>StandardErrorPath</key><string>$LOG</string>
</dict>
</plist>
PLIST

: > "$LOG"
launchctl bootout "gui/$(id -u)/$LABEL" 2>/dev/null || true
launchctl bootstrap "gui/$(id -u)" "$PLIST"
echo "started; click the notification within 120s. log: $LOG"
tail -f "$LOG" &
TAIL=$!
sleep 2
while launchctl print "gui/$(id -u)/$LABEL" 2>/dev/null | grep -q 'state = running'; do
  sleep 1
done
sleep 1
kill "$TAIL" 2>/dev/null || true
launchctl bootout "gui/$(id -u)/$LABEL" 2>/dev/null || true
