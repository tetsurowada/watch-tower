#!/bin/sh
# Builds WatchTowerSpike.app and signs it ad hoc. Run on the macOS host.
set -eu
cd "$(dirname "$0")"

# Override to start from a clean notification permission record.
BUNDLE_ID=${BUNDLE_ID:-dev.watch-tower.spike}

# Keep macOS build artifacts apart from the devcontainer's Linux ones.
export CARGO_TARGET_DIR=target/macos
cargo build --release

APP=target/WatchTowerSpike.app
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS"
cp "$CARGO_TARGET_DIR/release/macos-notify" "$APP/Contents/MacOS/macos-notify"
cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleIdentifier</key><string>$BUNDLE_ID</string>
  <key>CFBundleName</key><string>WatchTowerSpike</string>
  <key>CFBundleExecutable</key><string>macos-notify</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleVersion</key><string>1</string>
  <key>CFBundleShortVersionString</key><string>0.1</string>
  <key>LSMinimumSystemVersion</key><string>13.0</string>
  <key>LSUIElement</key><true/>
</dict>
</plist>
PLIST

codesign --force --sign - "$APP"
codesign --verify --verbose "$APP"
echo "built $APP ($BUNDLE_ID)"
