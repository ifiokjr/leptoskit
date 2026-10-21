#!/usr/bin/env bash
set -euo pipefail
for decision in deny allow; do
	target/native-ui/linux/CaptureLab >"target/native-ui/linux/$decision.log" 2>&1 &
	fixture_pid=$!
	trap 'kill "$fixture_pid" 2>/dev/null || true' EXIT
	target/debug/examples/native_conformance linux http://127.0.0.1:4723 Root desktop "$decision"
	kill "$fixture_pid"
	wait "$fixture_pid" || [ "$?" = 143 ]
	trap - EXIT
done
