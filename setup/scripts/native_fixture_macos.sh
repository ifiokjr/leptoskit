#!/usr/bin/env bash
# Builds the AppKit conformance fixture as an application bundle, which the
# Mac2 driver launches by path, and places the file its open panel selects
# beside the bundle.
set -euo pipefail
root="$(cd "$(dirname "$0")/../.." && pwd)"
out_dir="$root/target/native-ui/macos"
out="$out_dir/CaptureLab.app"
source_dir="$root/crates/leptoskit_testing/fixtures/macos"
rm -rf "$out"
mkdir -p "$out/Contents/MacOS"
cp "$source_dir/Info.plist" "$out/Contents/Info.plist"
xcrun swiftc -target "$(uname -m)-apple-macos13.0" "$source_dir/main.swift" -o "$out/Contents/MacOS/CaptureLab"
printf 'Leptoskit native file chooser evidence\n' >"$out_dir/evidence.txt"
printf '%s\n' "$out"
