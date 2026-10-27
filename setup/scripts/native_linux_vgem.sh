#!/usr/bin/env bash
# Loads the vgem kernel module so the isolated Linux desktop can allocate
# software OpenGL buffers.
#
# Hosted runner kernels ship vgem only in their 71 MB extra-modules package,
# and the Azure Ubuntu mirror has crawled through that download for half an
# hour. So vgem, and any module it needs that the base kernel lacks, is kept
# in a directory per kernel ($NATIVE_VGEM_CACHE, restored by CI), and the
# package is fetched only when the runner's kernel changes. That fetch moves
# to the next mirror whenever it falls below 1 MiB/s, and the package is
# checked against apt's hash before anything is taken from it.
#
# A kernel with no extra-modules package, a download no mirror completes, or a
# module that will not load fails here by name rather than as a missing
# /dev/dri device later.
set -euo pipefail
kernel="$(uname -r)"
package="linux-modules-extra-$kernel"
modules="${NATIVE_VGEM_CACHE:-$HOME/.cache/leptoskit-vgem}/$kernel"
attempts="${NATIVE_APT_ATTEMPTS:-3}"

# Refreshes apt's lists when they don't know the package yet. An unrelated
# preinstalled repository or a mirror mid-sync fails updates intermittently,
# so the update is attempted again after a pause.
ensure_package_known() {
	local attempt
	apt-cache show "$package" >/dev/null 2>&1 && return 0
	for attempt in $(seq 1 "$attempts"); do
		if sudo apt-get -o Acquire::Retries=5 update; then
			if apt-cache show "$package" >/dev/null 2>&1; then
				return 0
			fi
			echo "Kernel $kernel has no $package package; this runner image cannot provide vgem" >&2
			return 1
		fi
		if [ "$attempt" -lt "$attempts" ]; then
			echo "apt update attempt $attempt of $attempts failed; retrying" >&2
			sleep $((attempt * ${NATIVE_APT_RETRY_DELAY:-30}))
		fi
	done
	echo "Could not update apt's lists to find $package after $attempts attempts" >&2
	return 1
}

# Downloads the package into $1, trying its mirror and then the main archive
# and kernel.org's mirror, abandoning any transfer slower than 1 MiB/s for 20
# seconds, and verifies it against the hash apt's lists record.
download_package() {
	local deb="$1" uri="" hash="" algorithm digest tool path url round
	read -r uri _ _ hash < <(apt-get download --print-uris "$package" | tr -d "'")
	if [ -z "$uri" ] || [ -z "$hash" ]; then
		echo "apt gave no download address and hash for $package" >&2
		return 1
	fi
	algorithm="${hash%%:*}"
	digest="${hash#*:}"
	tool="$(checksum_tool "$algorithm")"
	path="${uri#*/ubuntu/}"
	for round in 1 2; do
		for url in "$uri" "https://archive.ubuntu.com/ubuntu/$path" "https://mirrors.edge.kernel.org/ubuntu/$path"; do
			echo "Fetching $package from $url (round $round)" >&2
			if curl --fail --location --silent --show-error --connect-timeout 15 \
				--speed-limit 1048576 --speed-time 20 --output "$deb" "$url"; then
				if echo "$digest  $deb" | "$tool" --check --status; then
					return 0
				fi
				echo "$url served a $package that does not match apt's $algorithm" >&2
			fi
		done
	done
	echo "No mirror delivered $package for kernel $kernel" >&2
	return 1
}

# The coreutils program that checks an apt hash field such as SHA512.
checksum_tool() {
	case "$1" in
	SHA512) echo sha512sum ;;
	SHA256) echo sha256sum ;;
	*)
		echo "apt recorded an unexpected $1 hash for $package" >&2
		return 1
		;;
	esac
}

# Copies vgem and every module it depends on that the base kernel lacks out of
# the extracted package tree $1 into the per-kernel directory.
collect_modules() {
	local tree="$1/lib/modules/$kernel" pending=(vgem) name file dependency dependencies
	mkdir -p "$modules"
	while [ "${#pending[@]}" -gt 0 ]; do
		name="${pending[0]}"
		pending=("${pending[@]:1}")
		# Module names use underscores; some of their files use dashes.
		file="$(find "$tree" \( -name "$name.ko*" -o -name "${name//_/-}.ko*" \) -print -quit)"
		if [ -z "$file" ]; then
			echo "$package has no $name module" >&2
			return 1
		fi
		cp "$file" "$modules/"
		IFS=',' read -r -a dependencies <<<"$(modinfo -F depends "$file")"
		for dependency in "${dependencies[@]}"; do
			if [ -n "$dependency" ] && ! modinfo "$dependency" >/dev/null 2>&1 &&
				[ -z "$(find "$modules" \( -name "$dependency.ko*" -o -name "${dependency//_/-}.ko*" \) -print -quit)" ]; then
				pending+=("$dependency")
			fi
		done
	done
}

# Fetches the package once for this kernel into the scratch directory $work
# and keeps only the modules vgem needs.
fetch_modules() {
	ensure_package_known
	download_package "$work/$package.deb"
	dpkg-deb --extract "$work/$package.deb" "$work/tree"
	collect_modules "$work/tree"
}

if ! modinfo vgem >/dev/null 2>&1; then
	shopt -s nullglob
	kept=("$modules"/vgem.ko*)
	shopt -u nullglob
	if [ "${#kept[@]}" -eq 0 ]; then
		work="$(mktemp -d)"
		trap 'rm -rf "$work"' EXIT
		fetch_modules
		# Tells CI to keep the modules for the next run on this kernel.
		if [ -n "${GITHUB_OUTPUT:-}" ]; then
			echo "fetched=true" >>"$GITHUB_OUTPUT"
		fi
	else
		echo "Using the vgem modules kept for kernel $kernel" >&2
	fi
	sudo install -D --mode=0644 --target-directory="/lib/modules/$kernel/updates/leptoskit-vgem" "$modules"/*
	sudo depmod "$kernel"
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
