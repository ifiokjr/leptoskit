#!/usr/bin/env bash
# Loads the vgem kernel module so the isolated Linux desktop can allocate
# software OpenGL buffers, installing the running kernel's extra modules when
# the runner image lacks them.
#
# apt on hosted runners intermittently fails on an unrelated preinstalled
# repository or on an Ubuntu mirror mid-sync. Each fetch is retried, and the
# whole update and install is attempted again after a pause. A kernel with no
# extra-modules package, or a module that will not load, fails here by name
# rather than as a missing /dev/dri device later.
set -euo pipefail
kernel="$(uname -r)"
package="linux-modules-extra-$kernel"
attempts="${NATIVE_APT_ATTEMPTS:-3}"

install_extra_modules() {
	local attempt
	for attempt in $(seq 1 "$attempts"); do
		if sudo apt-get -o Acquire::Retries=5 update; then
			if ! apt-cache show "$package" >/dev/null 2>&1; then
				echo "Kernel $kernel has no $package package; this runner image cannot provide vgem" >&2
				return 1
			fi
			if sudo apt-get -o Acquire::Retries=5 install -y "$package"; then
				return 0
			fi
		fi
		if [ "$attempt" -lt "$attempts" ]; then
			echo "apt attempt $attempt of $attempts failed; retrying" >&2
			sleep $((attempt * ${NATIVE_APT_RETRY_DELAY:-30}))
		fi
	done
	echo "Could not install $package for kernel $kernel after $attempts attempts" >&2
	return 1
}

if ! modinfo vgem >/dev/null 2>&1; then
	install_extra_modules
fi
if ! sudo modprobe vgem; then
	echo "Could not load the vgem module on kernel $kernel" >&2
	exit 1
fi
if ! sudo udevadm settle --timeout=60; then
	echo "udev did not settle within 60 seconds after loading vgem" >&2
	exit 1
fi
shopt -s nullglob
devices=(/dev/dri/*)
if [ "${#devices[@]}" -eq 0 ]; then
	echo "vgem loaded on kernel $kernel but created no /dev/dri device" >&2
	exit 1
fi
sudo chmod a+rw "${devices[@]}"
