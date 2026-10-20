#!/usr/bin/env bash
set -euo pipefail
platform="${1:?ios or android}"
device="${2:-}"
driver_port="${NATIVE_DRIVER_PORT:-14731}"
mkdir -p target/native-ui
if [ "$platform" = ios ]; then
	driver=xcuitest
	app="$(bash setup/scripts/native_fixture_ios.sh)"
	if [ -z "$device" ]; then
		device="$(xcrun simctl list devices available --json | jq -er '[.devices[][] | select(.isAvailable and (.name | startswith("iPhone")))] | sort_by(.state != "Booted") | first | .udid')"
	fi
elif [ "$platform" = android ]; then
	driver=uiautomator2
	test -n "$device"
	app="$(bash setup/scripts/native_fixture_android.sh | tail -1)"
else
	echo 'Expected ios or android' >&2
	exit 2
fi
server_pid=""
cleanup() {
	if [ -n "$server_pid" ]; then kill "$server_pid" 2>/dev/null || true; fi
}
trap cleanup EXIT
(
	unset APPIUM_HOME
	cd setup/native-ui
	exec node node_modules/appium/build/lib/main.js --address 127.0.0.1 --port "$driver_port" --use-drivers "$driver" --log-no-colors
) >"target/native-ui/appium-$platform.log" 2>&1 &
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
kill -0 "$server_pid"
for decision in deny allow; do
	# Reset only our fixture's persisted permission state, never other apps.
	if [ "$platform" = ios ]; then
		if xcrun simctl get_app_container "$device" dev.leptoskit.capturelab app >/dev/null 2>&1; then
			xcrun simctl uninstall "$device" dev.leptoskit.capturelab
		fi
	else
		if adb -s "$device" shell pm path dev.leptoskit.capturelab | grep -q '^package:'; then
			adb -s "$device" uninstall dev.leptoskit.capturelab
		fi
	fi
	target/debug/examples/native_conformance "$platform" "http://127.0.0.1:$driver_port" "$app" "$device" "$decision"
done
