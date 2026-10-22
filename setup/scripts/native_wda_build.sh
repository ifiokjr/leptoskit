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
xcodebuild build-for-testing \
	-project "$project" \
	-scheme WebDriverAgentRunner \
	-destination "id=$device" \
	-derivedDataPath "$derived_data" \
	"IPHONEOS_DEPLOYMENT_TARGET=$deployment_target" \
	GCC_TREAT_WARNINGS_AS_ERRORS=0 \
	COMPILER_INDEX_STORE_ENABLE=NO
