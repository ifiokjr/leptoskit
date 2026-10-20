#!/usr/bin/env bash
# Builds the platform probe for Android (aarch64), pushes it to the first
# device, and runs it. Exits non-zero when any probe fails on device.
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$repo_root"

# Re-exec in a clean environment: the devenv shell's nix CC/CXX wrappers
# mis-link Android builds the same way they do iOS simulator builds.
if [ -z "${LEPTOSKIT_CLEAN_BUILD_ENV:-}" ]; then
	exec env -i \
		LEPTOSKIT_CLEAN_BUILD_ENV=1 \
		HOME="$HOME" \
		USER="${USER:-$(id -un)}" \
		TERM="${TERM:-dumb}" \
		ANDROID_HOME="${ANDROID_HOME:-$HOME/Library/Android/sdk}" \
		${ANDROID_NDK_HOME:+"ANDROID_NDK_HOME=$ANDROID_NDK_HOME"} \
		${ANDROID_NDK_VERSION:+"ANDROID_NDK_VERSION=$ANDROID_NDK_VERSION"} \
		PATH="$HOME/.cargo/bin:/usr/bin:/bin:/usr/sbin:/sbin" \
		bash "${BASH_SOURCE[0]}"
fi

# ANDROID_NDK_HOME is the NDK itself (the directory holding `toolchains/`), by
# the convention the Android Gradle plugin and cargo-ndk share. Without it, the
# NDK is picked from the SDK's side-by-side `ndk/` directory: the version
# ANDROID_NDK_VERSION names, or else the newest installed.
if [ -n "${ANDROID_NDK_HOME:-}" ]; then
	ndk="$ANDROID_NDK_HOME"
else
	ndk_versions="${ANDROID_HOME:-$HOME/Library/Android/sdk}/ndk"
	ndk_version="${ANDROID_NDK_VERSION:-$(ls "$ndk_versions" | sort -V | tail -1)}"
	ndk="$ndk_versions/$ndk_version"
fi
if [ ! -d "$ndk/toolchains/llvm/prebuilt" ]; then
	echo "no Android NDK at $ndk; set ANDROID_NDK_HOME to the NDK directory" >&2
	exit 1
fi
prebuilt="$(ls "$ndk/toolchains/llvm/prebuilt" | head -1)"
export CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER="$ndk/toolchains/llvm/prebuilt/$prebuilt/bin/aarch64-linux-android24-clang"
export PATH="$HOME/.cargo/bin:/usr/bin:/bin"

cargo build --release --package platform_probe --target aarch64-linux-android

adb="${ANDROID_HOME:-$HOME/Library/Android/sdk}/platform-tools/adb"
serial="$("$adb" devices | awk 'NR>1 && $2=="device" {print $1; exit}')"
if [ -z "$serial" ]; then
	echo "no Android device or emulator is ready" >&2
	exit 1
fi

"$adb" -s "$serial" push target/aarch64-linux-android/release/platform_probe /data/local/tmp/platform_probe >/dev/null
"$adb" -s "$serial" shell chmod +x /data/local/tmp/platform_probe
exec "$adb" -s "$serial" shell /data/local/tmp/platform_probe
