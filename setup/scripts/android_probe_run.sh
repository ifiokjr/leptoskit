#!/usr/bin/env bash
# Builds the platform probe for Android (aarch64), pushes it to the first
# device, and runs it. Exits non-zero when any probe fails on device.
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$repo_root"

ndk_root="${ANDROID_NDK_HOME:-$HOME/Library/Android/sdk/ndk}"
ndk_version="${ANDROID_NDK_VERSION:-$(ls "$ndk_root" | sort -V | tail -1)}"
prebuilt="$(ls "$ndk_root/$ndk_version/toolchains/llvm/prebuilt" | head -1)"
export CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER="$ndk_root/$ndk_version/toolchains/llvm/prebuilt/$prebuilt/bin/aarch64-linux-android24-clang"
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
