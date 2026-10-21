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
logcat_pid=""
cleanup() {
	if [ -n "$server_pid" ]; then kill "$server_pid" 2>/dev/null || true; fi
	if [ -n "$logcat_pid" ]; then kill "$logcat_pid" 2>/dev/null || true; fi
}
trap cleanup EXIT
if [ "$platform" = android ]; then
	# On hosted runners the adb transport intermittently drops ("device
	# offline") while a freshly booted emulator is still starting services,
	# failing whichever driver command is in flight. Wait for the transport to
	# hold, then install every package the driver would otherwise install
	# itself, one at a time. The session skips its own installs and logcat
	# (see the native_conformance example), so no concurrent adb work starts
	# before the first test command. Logcat is retained for any remaining
	# disconnect.
	adb -s "$device" logcat -v threadtime >"target/native-ui/logcat-android.log" 2>&1 &
	logcat_pid=$!
	bash setup/scripts/android_settle.sh "$device"
	server_apks=()
	while IFS= read -r apk; do server_apks+=("$apk"); done < <(
		find setup/native-ui/node_modules -path '*/appium-uiautomator2-server/apks/*.apk' | sort
	)
	if [ "${#server_apks[@]}" -ne 2 ]; then
		echo "Expected two UiAutomator2 server packages, found ${#server_apks[@]}" >&2
		exit 1
	fi
	settings_apks=()
	while IFS= read -r apk; do settings_apks+=("$apk"); done < <(
		find setup/native-ui/node_modules -path '*/io.appium.settings/apks/settings_apk-debug.apk' | sort -u
	)
	if [ "${#settings_apks[@]}" -ne 1 ]; then
		echo "Expected one Appium Settings package, found ${#settings_apks[@]}" >&2
		exit 1
	fi
	for apk in "${server_apks[@]}"; do
		adb -s "$device" install -r --no-incremental "$apk" >/dev/null
	done
	# The driver grants the Settings app its runtime permissions on install.
	adb -s "$device" install -r -g --no-incremental "${settings_apks[0]}" >/dev/null
fi
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
