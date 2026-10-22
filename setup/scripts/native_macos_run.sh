#!/usr/bin/env bash
# Runs the macOS native journeys through Appium's Mac2 driver: the shared
# conformance journey on the AppKit fixture, then the dialog plugin's own
# fixture. Both apps are launched by bundle path, so each session sees only
# its fixture and the sheets it presents.
#
# The driver builds and starts its XCTest runner, which needs Xcode and an
# interactive session whose UI automation is allowed (see the native testing
# guide). Build the journeys first:
#
#   cargo build --locked -p leptoskit_testing --features native-ui \
#     --example native_conformance --example dialog_plugin_journey
#   cargo build --locked -p leptoskit_plugin_dialog --example native_fixture
set -euo pipefail
driver_port="${NATIVE_DRIVER_PORT:-14731}"
out="target/native-ui/macos"
mkdir -p "$out"
lab="$(bash setup/scripts/native_fixture_macos.sh)"

# The plugin fixture is a plain executable; XCTest launches bundles.
dialog="$PWD/$out/DialogFixture.app"
rm -rf "$dialog"
mkdir -p "$dialog/Contents/MacOS"
cp target/debug/examples/native_fixture "$dialog/Contents/MacOS/DialogFixture"
cat >"$dialog/Contents/Info.plist" <<'PLIST'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>CFBundleIdentifier</key><string>dev.leptoskit.dialogfixture</string>
<key>CFBundleName</key><string>Dialog Fixture</string>
<key>CFBundleExecutable</key><string>DialogFixture</string>
<key>CFBundleVersion</key><string>1</string>
<key>CFBundlePackageType</key><string>APPL</string>
<key>NSPrincipalClass</key><string>NSApplication</string>
<key>NSHighResolutionCapable</key><true/>
</dict></plist>
PLIST

server_pid=""
cleanup() {
	if [ -n "$server_pid" ]; then kill "$server_pid" 2>/dev/null || true; fi
}
trap cleanup EXIT
(
	unset APPIUM_HOME
	cd setup/native-ui
	exec node node_modules/appium/build/lib/main.js --address 127.0.0.1 --port "$driver_port" --use-drivers mac2 --log-no-colors
) >target/native-ui/appium-macos.log 2>&1 &
server_pid=$!
deadline=$((SECONDS + 60))
until curl -fsS "http://127.0.0.1:$driver_port/status" 2>/dev/null | jq -e '.value.ready == true' >/dev/null; do
	kill -0 "$server_pid"
	if [ "$SECONDS" -ge "$deadline" ]; then
		echo "Native driver startup timed out" >&2
		exit 1
	fi
	sleep 0.1
done
endpoint="http://127.0.0.1:$driver_port"
for decision in deny allow; do
	target/debug/examples/native_conformance macos "$endpoint" "$lab" desktop "$decision"
done
for decision in decline accept; do
	target/debug/examples/dialog_plugin_journey macos "$endpoint" "$dialog" "$decision"
done
