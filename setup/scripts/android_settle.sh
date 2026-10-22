#!/usr/bin/env bash
# Waits until an Android device's adb transport has settled after boot.
#
# Hosted emulators report boot completion while the system is still starting
# services, and the adb transport can drop ("device offline") during that
# window. Starting UI automation then fails in whichever command is in flight.
# This guard requires the device to stay booted, with a responsive package
# service, for an unbroken window before any test touches it. A drop restarts
# the window, so the job proceeds only once the transport has held.
set -euo pipefail

device="${1:?device serial}"
settle_seconds="${ANDROID_SETTLE_SECONDS:-30}"
deadline=$((SECONDS + ${ANDROID_SETTLE_TIMEOUT_SECONDS:-300}))
healthy_since=""

while [ "$SECONDS" -lt "$deadline" ]; do
	state="$(adb -s "$device" get-state 2>/dev/null || true)"
	probe=""
	if [ "$state" = device ]; then
		probe="$(adb -s "$device" shell 'getprop sys.boot_completed; getprop init.svc.bootanim; cmd package path android' 2>/dev/null | tr -d '\r' || true)"
	fi
	if [ "$(sed -n 1p <<<"$probe")" = 1 ] &&
		[ "$(sed -n 2p <<<"$probe")" = stopped ] &&
		[[ "$(sed -n 3p <<<"$probe")" == package:* ]]; then
		healthy_since="${healthy_since:-$SECONDS}"
		if [ $((SECONDS - healthy_since)) -ge "$settle_seconds" ]; then
			echo "Android device $device settled after ${SECONDS}s"
			exit 0
		fi
	else
		if [ -n "$healthy_since" ]; then
			echo "Android device $device dropped (state: ${state:-none}) after $((SECONDS - healthy_since))s healthy; waiting again" >&2
		fi
		healthy_since=""
		if [ "$state" = offline ]; then
			# An offline transport recovers only after the host reconnects it.
			adb -s "$device" reconnect >/dev/null 2>&1 || true
		fi
	fi
	sleep 2
done
echo "Android device $device did not settle within ${ANDROID_SETTLE_TIMEOUT_SECONDS:-300}s" >&2
exit 1
