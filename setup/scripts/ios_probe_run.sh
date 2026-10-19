#!/usr/bin/env bash
# Builds the platform probe for the iOS simulator, installs, and runs it.
# Exits non-zero when any probe fails on device.
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$repo_root"

CLANG_DIR="$(dirname "$(xcrun --find clang)")"
export PATH="$HOME/.cargo/bin:$CLANG_DIR:/usr/bin:/bin"
export SDKROOT="$(xcrun --sdk iphonesimulator --show-sdk-path)"

cargo build --release --package platform_probe --target aarch64-apple-ios-sim

app="target/platform_probe.app"
rm -rf "$app"
mkdir -p "$app"
cp target/aarch64-apple-ios-sim/release/platform_probe "$app/platform_probe"
cat >"$app/Info.plist" <<'PLIST'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>CFBundleExecutable</key><string>platform_probe</string>
	<key>CFBundleIdentifier</key><string>dev.leptoskit.platform-probe</string>
	<key>CFBundleName</key><string>platform_probe</string>
	<key>CFBundlePackageType</key><string>APPL</string>
	<key>CFBundleShortVersionString</key><string>0.1</string>
	<key>CFBundleVersion</key><string>1</string>
	<key>MinimumOSVersion</key><string>14.0</string>
</dict>
</plist>
PLIST

udid="$(xcrun simctl list devices available | grep -m1 -oE '[0-9A-F-]{36}.*(Booted)' | grep -oE '[0-9A-F-]{36}' || true)"
if [ -z "$udid" ]; then
	udid="$(xcrun simctl list devices available | grep -m1 -oE '[0-9A-F-]{36}')"
	xcrun simctl boot "$udid"
fi

xcrun simctl install "$udid" "$app"
exec xcrun simctl launch --console-pty "$udid" dev.leptoskit.platform-probe
