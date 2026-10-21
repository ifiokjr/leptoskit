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
SDKROOT="$(xcrun --sdk macosx --show-sdk-path)"
simulator_sdk="$(xcrun --sdk iphonesimulator --show-sdk-path)"
export SDKROOT
export CARGO_TARGET_AARCH64_APPLE_IOS_SIM_RUSTFLAGS="-C link-arg=-isysroot$simulator_sdk"

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
# judged from the probe's own verdict line. Right after a slow boot the
# simulator can run the app before the console attaches, and the launch
# returns with none of its output. That is the simulator, not the probe, so
# a launch without a verdict is retried; a verdict, pass or fail, is final.
log="$(mktemp)"
verdict=""
for attempt in 1 2 3; do
	# SpringBoard can also deny the launch request after the probe has run
	# and printed its verdict, so the launch's exit status must not end the
	# script under `set -e`; only the verdict decides.
	xcrun simctl launch --console-pty --terminate-running-process \
		"$udid" dev.leptoskit.platform-probe | tee "$log" || true
	# The console is a PTY, so its lines end in CRLF.
	verdict="$(tr -d '\r' <"$log" | grep -m1 '^platform probe: ' || true)"
	[ -n "$verdict" ] && break
	echo "launch $attempt produced no verdict; relaunching" >&2
	sleep 5
done

if [ -z "$verdict" ]; then
	echo "the platform probe never reported a verdict on the iOS simulator" >&2
	exit 1
fi
if [ "$verdict" != "platform probe: all probes passed on ios" ]; then
	echo "a probe failed on the iOS simulator" >&2
	exit 1
fi
