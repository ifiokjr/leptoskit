#!/usr/bin/env bash
# Runs inside the driver's nested KWin Wayland session, on its private D-Bus
# session bus: the conformance journeys on the Qt fixture, then the dialog
# plugin's own journeys on its GTK fixture. The plugin's file chooser is
# rendered by xdg-desktop-portal's KDE backend and its message box by zenity,
# so the portal must own its names on that bus before any dialog is presented.
set -euo pipefail
out=target/native-ui/linux
mkdir -p "$out"
export GDK_BACKEND=wayland

bus_names() {
	dbus-send --session --print-reply=literal --dest=org.freedesktop.DBus \
		/org/freedesktop/DBus org.freedesktop.DBus.ListNames
}

# The session bus starts before the compositor, so its activation environment
# has no Wayland display: the portal that kwin's own startup registration
# auto-activates on it sees every KDE backend activation die on the xcb
# platform and caches that failure. Hand the bus this session's display
# environment, replace that portal with one started from the session itself,
# and wait for both the portal and its KDE backend to own their names.
dbus-update-activation-environment WAYLAND_DISPLAY QT_QPA_PLATFORM GDK_BACKEND
pkill -f xdg-desktop-portal || true
sleep 1

# The packages place the portal and its KDE backend in arch-specific
# directories, so resolve each from the D-Bus service that activates it.
portal_kde=$(sed -n 's/^Exec=//p' \
	/usr/share/dbus-1/services/org.freedesktop.impl.portal.desktop.kde.service)
portal=$(sed -n 's/^Exec=//p' \
	/usr/share/dbus-1/services/org.freedesktop.portal.Desktop.service)
: "${portal_kde:?missing the KDE portal backend service}" \
	"${portal:?missing the portal service}"
"$portal_kde" >"$out/portal-kde.log" 2>&1 &
"$portal" >"$out/portal.log" 2>&1 &
portal_ready=""
for _ in $(seq 1 60); do
	if names="$(bus_names)" &&
		grep -Fq org.freedesktop.portal.Desktop <<<"$names" &&
		grep -Fq org.freedesktop.impl.portal.desktop.kde <<<"$names"; then
		portal_ready=1
		break
	fi
	sleep 0.5
done
if [ -z "$portal_ready" ]; then
	echo "xdg-desktop-portal and its KDE backend never owned the session bus" >&2
	exit 1
fi

for decision in deny allow; do
	target/native-ui/linux/CaptureLab >"$out/$decision.log" 2>&1 &
	fixture_pid=$!
	trap 'kill "$fixture_pid" 2>/dev/null || true' EXIT
	target/debug/examples/native_conformance linux http://127.0.0.1:4723 Root desktop "$decision"
	kill "$fixture_pid"
	wait "$fixture_pid" || [ "$?" = 143 ]
	trap - EXIT
done

# The dialog plugin's fixture presents its file chooser in the directory that
# holds its own executable, where the runner leaves the evidence file.
printf 'Leptoskit dialog plugin evidence\n' >target/debug/examples/evidence.txt
for decision in decline accept; do
	target/debug/examples/native_fixture >"$out/dialog-$decision.log" 2>&1 &
	fixture_pid=$!
	trap 'kill "$fixture_pid" 2>/dev/null || true' EXIT
	target/debug/examples/dialog_plugin_journey linux http://127.0.0.1:4723 Root "$decision"
	kill "$fixture_pid"
	wait "$fixture_pid" || [ "$?" = 143 ]
	trap - EXIT
done
