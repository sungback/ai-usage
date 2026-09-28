#!/bin/bash
set -euo pipefail

echo "==> Building macOS Apple Silicon (ARM64)..."

# Ensure Rust aarch64 target is installed
rustup target add aarch64-apple-darwin

# Build release binary for Apple Silicon
echo "==> Compiling aarch64-apple-darwin..."
cargo build --release --target aarch64-apple-darwin

BINARY="target/aarch64-apple-darwin/release/ai-usage"

# Verify architecture
file "$BINARY"

# Construct macOS .app bundle
APP_DIR="target/AI Usage Monitor.app"
echo "==> Assembling application bundle at: $APP_DIR"
rm -rf "$APP_DIR"
mkdir -p "$APP_DIR/Contents/MacOS"
mkdir -p "$APP_DIR/Contents/Resources"

cp "$BINARY" "$APP_DIR/Contents/MacOS/ai-usage"
chmod +x "$APP_DIR/Contents/MacOS/ai-usage"
cp scripts/Info.plist "$APP_DIR/Contents/Info.plist"

# 버전 키는 여기 하드코딩하지 말고 Cargo.toml에서 주입한다 — 어긋나면 Finder와
# updater.rs의 is_version_newer가 서로 다른 버전을 읽는다. [package]로 한정하는 건,
# 안 하면 의존성 크레이트의 `version = "0.62"`가 섞여 0.62가 앱 버전으로 나오기 때문.
APP_VERSION=$(sed -n '/^\[package\]/,/^\[/s/^version *= *"\([^"]*\)".*/\1/p' Cargo.toml)
if [ -z "$APP_VERSION" ]; then
  echo "ERROR: could not read [package] version from Cargo.toml" >&2
  exit 1
fi
plutil -replace CFBundleVersion -string "$APP_VERSION" "$APP_DIR/Contents/Info.plist"
plutil -replace CFBundleShortVersionString -string "$APP_VERSION" "$APP_DIR/Contents/Info.plist"

# Generate icns if iconutil is available
if command -v iconutil >/dev/null 2>&1 && [ -f src/icons/256x256.png ]; then
    ICONSET_DIR="target/AppIcon.iconset"
    mkdir -p "$ICONSET_DIR"
    sips -z 16 16     src/icons/16x16.png   --out "$ICONSET_DIR/icon_16x16.png" 2>/dev/null || true
    sips -z 32 32     src/icons/32x32.png   --out "$ICONSET_DIR/icon_16x16@2x.png" 2>/dev/null || true
    sips -z 32 32     src/icons/32x32.png   --out "$ICONSET_DIR/icon_32x32.png" 2>/dev/null || true
    sips -z 64 64     src/icons/48x48.png   --out "$ICONSET_DIR/icon_32x32@2x.png" 2>/dev/null || true
    sips -z 128 128   src/icons/256x256.png --out "$ICONSET_DIR/icon_128x128.png" 2>/dev/null || true
    sips -z 256 256   src/icons/256x256.png --out "$ICONSET_DIR/icon_128x128@2x.png" 2>/dev/null || true
    sips -z 256 256   src/icons/256x256.png --out "$ICONSET_DIR/icon_256x256.png" 2>/dev/null || true
    sips -z 512 512   src/icons/256x256.png --out "$ICONSET_DIR/icon_256x256@2x.png" 2>/dev/null || true
    iconutil -c icns "$ICONSET_DIR" -o "$APP_DIR/Contents/Resources/AppIcon.icns" 2>/dev/null || true
    rm -rf "$ICONSET_DIR"
fi

# Ad-hoc code sign app bundle
echo "==> Code signing application bundle..."
codesign --force --deep --sign - "$APP_DIR"

# Package distribution ZIP archive
echo "==> Packaging macOS ZIP archive..."
rm -f target/ai-usage-macos-arm64.zip
(cd target && zip -r -y -q ai-usage-macos-arm64.zip "AI Usage Monitor.app")

# Package DMG with Applications symlink
echo "==> Packaging macOS DMG..."
DMG_STAGE="target/dmg_stage"
rm -rf "$DMG_STAGE"
mkdir -p "$DMG_STAGE"
cp -R "$APP_DIR" "$DMG_STAGE/"
ln -s /Applications "$DMG_STAGE/Applications"

rm -f target/ai-usage-macos-arm64.dmg
hdiutil create -volname "AI Usage Monitor" -srcfolder "$DMG_STAGE" -ov -format UDZO target/ai-usage-macos-arm64.dmg
rm -rf "$DMG_STAGE"

echo "==> Apple Silicon (ARM64) build completed successfully:"
echo "    - ARM64 binary: $BINARY"
echo "    - App bundle: $APP_DIR"
echo "    - Zip archive: target/ai-usage-macos-arm64.zip"
echo "    - DMG package: target/ai-usage-macos-arm64.dmg"
