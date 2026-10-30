#!/usr/bin/env bash
# Builds the pinned driver's WebDriverAgent for one simulator ahead of the
# first session. native_conformance launches this build instead of compiling
# the runner itself when NATIVE_WDA_DERIVED_DATA names the derived data
# directory. The arguments mirror the driver's own build-for-testing command.
set -euo pipefail
device="${1:?simulator udid}"
derived_data="${2:?derived data directory}"
project=setup/native-ui/node_modules/appium-xcuitest-driver/node_modules/appium-webdriveragent/WebDriverAgent.xcodeproj
if [ ! -d "$project" ]; then
	echo "WebDriverAgent project not found at $project; install setup/native-ui first" >&2
	exit 1
fi
runtime="$(xcrun simctl list devices --json | jq -er --arg udid "$device" '
	.devices | to_entries[] | select(any(.value[]; .udid == $udid)) | .key')"
# The driver targets the device's own major.minor iOS version.
deployment_target="$(xcrun simctl list runtimes --json | jq -er --arg runtime "$runtime" '
	.runtimes[] | select(.identifier == $runtime) | .version | split(".")[:2] | join(".")')"
# xcodebuild resolves the destination once, before it compiles anything, and
# gives up if that one lookup misses the device. On a hosted runner the first
# boot mounts the device's runtime, and while CoreSimulator registers it there
# are moments when it reports no simulators at all: every preinstalled device
# vanishes, not just the one this job created. A build that starts in that
# window dies with "Unable to find a device matching the provided destination
# specifier" and lists only placeholders (main, 2026-10-29, run 40859821954:
# the build began 4m43s after `simctl boot` and still saw none, and `simctl
# bootstatus` reported that same device booted 19s later). Wait until
# xcodebuild's own destination list names the UDID instead of handing it one it
# cannot resolve, then build once. This still overlaps the job's cargo build,
# which is why the build runs here rather than inside the first test session.
destination_timeout="${NATIVE_WDA_DESTINATION_TIMEOUT:-600}"
wait_started=$SECONDS
while :; do
	destinations="$(xcodebuild -showdestinations -project "$project" -scheme WebDriverAgentRunner 2>&1 || true)"
	if grep -Fq "id:$device" <<<"$destinations"; then
		break
	fi
	if [ "$((SECONDS - wait_started))" -ge "$destination_timeout" ]; then
		echo "xcodebuild did not list simulator $device as a destination within ${destination_timeout} seconds" >&2
		echo "xcodebuild: $(xcodebuild -version | tr '\n' ' ')" >&2
		printf '%s\n' "$destinations" >&2
		xcrun simctl list devices available >&2 || true
		xcrun simctl list runtimes >&2 || true
		exit 1
	fi
	sleep 5
done
echo "xcodebuild listed simulator $device as a destination after $((SECONDS - wait_started)) seconds"
xcodebuild build-for-testing \
	-project "$project" \
	-scheme WebDriverAgentRunner \
	-destination "id=$device" \
	-derivedDataPath "$derived_data" \
	"IPHONEOS_DEPLOYMENT_TARGET=$deployment_target" \
	GCC_TREAT_WARNINGS_AS_ERRORS=0 \
	COMPILER_INDEX_STORE_ENABLE=NO
