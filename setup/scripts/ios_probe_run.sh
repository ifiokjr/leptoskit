#!/usr/bin/env bash
# Builds the platform probe for the iOS simulator, installs, and runs it.
# Exits non-zero when any probe fails on device.
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$repo_root"

# Cross-compiling needs Xcode's clang and an environment the nix/devenv
# shell would otherwise poison: its CC/CXX wrappers mis-link simulator
# builds and leave proc-macro dylibs un-loadable. Re-exec in a clean
# environment — exactly the one a plain terminal provides.
if [ -z "${LEPTOSKIT_CLEAN_BUILD_ENV:-}" ]; then
	clang_dir="$(dirname "$(xcrun --find clang)")"
	exec env -i \
		LEPTOSKIT_CLEAN_BUILD_ENV=1 \
		HOME="$HOME" \
		USER="${USER:-$(id -un)}" \
		TERM="${TERM:-dumb}" \
		PATH="$HOME/.cargo/bin:$clang_dir:/usr/bin:/bin:/usr/sbin:/sbin" \
		bash "${BASH_SOURCE[0]}"
fi

# Host build scripts link against the macOS SDK while the target link
# needs the simulator SDK; SDKROOT covers the host and per-target
# rustflags carry the simulator sysroot.
export SDKROOT="$(xcrun --sdk macosx --show-sdk-path)"
export CARGO_TARGET_AARCH64_APPLE_IOS_SIM_RUSTFLAGS="\
-C link-arg=-isysroot$(xcrun --sdk iphonesimulator --show-sdk-path)"

rustup target add aarch64-apple-ios-sim >/dev/null 2>&1 || true
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
xcrun simctl bootstatus "$udid" -b

xcrun simctl install "$udid" "$app"
# `simctl launch` does not propagate the app's exit status, so success is
# judged from the probe's own verdict lines.
log="$(mktemp)"
xcrun simctl launch --console-pty "$udid" dev.leptoskit.platform-probe | tee "$log"
grep -q "platform probe: all probes passed on ios" "$log"
if grep -q "FAIL " "$log"; then
	echo "a probe failed on the iOS simulator" >&2
	exit 1
fi
