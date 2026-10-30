#!/usr/bin/env bash
# Builds setup/harmonyos_plugins, which depends on every plugin crate, for
# arm64 HarmonyOS with the SDK's clang, the way the CLI builds a HarmonyOS
# host. Needs OHOS_SDK_HOME (a public SDK root containing 12/native) and the
# aarch64-unknown-linux-ohos Rust target.
set -euo pipefail
ndk="${OHOS_SDK_HOME:?set OHOS_SDK_HOME to the HarmonyOS SDK root}/12/native"
tools="$ndk/llvm/bin"
sysroot="$ndk/sysroot"
flags="--target=aarch64-linux-ohos --sysroot=$sysroot -D__MUSL__"
export CARGO_TARGET_AARCH64_UNKNOWN_LINUX_OHOS_LINKER="$tools/clang"
export CARGO_TARGET_AARCH64_UNKNOWN_LINUX_OHOS_RUSTFLAGS="-C link-arg=--target=aarch64-linux-ohos -C link-arg=--sysroot=$sysroot"
export CC_aarch64_unknown_linux_ohos="$tools/clang"
export CXX_aarch64_unknown_linux_ohos="$tools/clang++"
export AR_aarch64_unknown_linux_ohos="$tools/llvm-ar"
export CFLAGS_aarch64_unknown_linux_ohos="$flags"
export CXXFLAGS_aarch64_unknown_linux_ohos="$flags"
exec cargo build --locked -p harmonyos_plugins --target aarch64-unknown-linux-ohos
