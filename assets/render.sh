#!/bin/sh
# Regenerates the PNGs under assets/ from assets/src/*.svg.
set -eu
cd "$(dirname "$0")"
cargo build -q --release --manifest-path ../tools/render-assets/Cargo.toml
render=../tools/render-assets/target/release/render-assets

for state in done wait input; do
  "$render" "src/state-$state.svg" "state-$state.png" 256
done

# macOS iconset; `iconutil -c icns` on the host turns it into AppIcon.icns.
mkdir -p AppIcon.iconset
for size in 16 32 128 256 512; do
  "$render" src/icon.svg "AppIcon.iconset/icon_${size}x${size}.png" "$size"
  "$render" src/icon.svg "AppIcon.iconset/icon_${size}x${size}@2x.png" "$((size * 2))"
done
