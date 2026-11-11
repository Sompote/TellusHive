#!/bin/bash
# Builds the release downloads into dist/:
#   TellusHive-<version>-macos-universal.zip   TellusHive.app (Apple silicon + Intel)
#   TellusHive-<version>-windows-x64.zip       tellushive.exe
# Needs the rustup targets aarch64-apple-darwin, x86_64-apple-darwin, x86_64-pc-windows-gnu
# and mingw-w64 (brew install mingw-w64) for the Windows build.
set -euo pipefail
cd "$(dirname "$0")/.."
VERSION=$(grep -m1 '^version' Cargo.toml | cut -d'"' -f2)
OUT=dist
rm -rf "$OUT" && mkdir -p "$OUT"
export MACOSX_DEPLOYMENT_TARGET=11.0

echo "== macOS universal $VERSION"
cargo build --release -p tellushive --target aarch64-apple-darwin
cargo build --release -p tellushive --target x86_64-apple-darwin
APP="$OUT/TellusHive.app"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
lipo -create -output "$APP/Contents/MacOS/tellushive" \
  target/aarch64-apple-darwin/release/tellushive target/x86_64-apple-darwin/release/tellushive
ICONSET=$(mktemp -d)/AppIcon.iconset
mkdir -p "$ICONSET"
for s in 16 32 128 256 512; do
  sips -z $s $s brand/tellushive_icon_1024.png --out "$ICONSET/icon_${s}x${s}.png" >/dev/null
  sips -z $((s * 2)) $((s * 2)) brand/tellushive_icon_1024.png --out "$ICONSET/icon_${s}x${s}@2x.png" >/dev/null
done
iconutil -c icns "$ICONSET" -o "$APP/Contents/Resources/AppIcon.icns"
cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>TellusHive</string>
  <key>CFBundleDisplayName</key><string>TellusHive</string>
  <key>CFBundleIdentifier</key><string>io.github.sompote.tellushive</string>
  <key>CFBundleVersion</key><string>$VERSION</string>
  <key>CFBundleShortVersionString</key><string>$VERSION</string>
  <key>CFBundleExecutable</key><string>tellushive</string>
  <key>CFBundleIconFile</key><string>AppIcon</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>LSMinimumSystemVersion</key><string>11.0</string>
  <key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
PLIST
# ad-hoc signature: Apple silicon refuses to start an unsigned binary
codesign --force --deep --sign - "$APP"
(cd "$OUT" && ditto -c -k --keepParent TellusHive.app "TellusHive-$VERSION-macos-universal.zip")

echo "== Windows x64 $VERSION"
cargo build --release -p tellushive --target x86_64-pc-windows-gnu
WIN="$OUT/TellusHive-$VERSION-windows-x64"
mkdir -p "$WIN"
cp target/x86_64-pc-windows-gnu/release/tellushive.exe "$WIN/"
cp README.md LICENSE THIRD_PARTY_NOTICES.md SAFETY.md "$WIN/"
(cd "$OUT" && zip -qr "TellusHive-$VERSION-windows-x64.zip" "TellusHive-$VERSION-windows-x64")

shasum -a 256 "$OUT"/*.zip | tee "$OUT/SHA256SUMS.txt"
