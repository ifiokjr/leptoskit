#!/bin/bash
set -euo pipefail

simulator=${1:?Provide the identifier of an owned booted simulator}
application=${2:?Provide the compiled Controls.app path}
receipt=${3:?Provide a fresh receipt directory}
poll_interval_seconds=0.25

[[ ! -e "$receipt" ]]
mkdir -p "$receipt"

/usr/bin/xcrun simctl install "$simulator" "$application"
container=$(/usr/bin/xcrun simctl get_app_container \
	"$simulator" dev.leptoskit.join-controls data)

if [[ -f "$container/Documents/result.txt" ]]; then
	/bin/mv "$container/Documents/result.txt" "$receipt/previous-runtime.log"
fi

/usr/bin/xcrun simctl launch --terminate-running-process \
	"$simulator" dev.leptoskit.join-controls >"$receipt/launch.log"

for attempt in {1..120}; do
	result="$container/Documents/result.txt"

	if [[ -f "$result" ]] && /usr/bin/grep -q 'ALL CONTROLS COMPLETE' "$result"; then
		/bin/cp "$result" "$receipt/runtime.log"
		/usr/bin/xcrun simctl io "$simulator" screenshot "$receipt/screen.png"
		/bin/cat "$receipt/runtime.log"
		! /usr/bin/grep -q '^FAIL' "$receipt/runtime.log"
		exit $?
	fi

	/bin/sleep "$poll_interval_seconds"
done

printf '%s\n' 'Control run did not finish within 30 seconds' >&2
exit 1
