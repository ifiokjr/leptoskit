#!/usr/bin/env bash
set -euo pipefail
root="$(cd "$(dirname "$0")/../.." && pwd)"
out="$root/target/native-ui/CaptureLab.app"
source_dir="$root/crates/leptoskit_testing/fixtures/ios"
mkdir -p "$out"
cp "$source_dir/Info.plist" "$out/Info.plist"
xcrun --sdk iphonesimulator swiftc -target "$(uname -m)-apple-ios17.0-simulator" -sdk "$(xcrun --sdk iphonesimulator --show-sdk-path)" "$source_dir/main.swift" -o "$out/CaptureLab"
printf '%s\n' "$out"
