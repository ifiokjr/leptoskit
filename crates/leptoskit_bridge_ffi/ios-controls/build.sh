#!/bin/bash
set -euo pipefail

controls=$(cd "$(dirname "$0")" && pwd)
source_root=$(cd "$controls/.." && pwd)
output=${1:?Provide a fresh absolute output directory}
swift_language_version=6

[[ "$output" = /* && ! -e "$output/Controls.app" ]]
mkdir -p "$output/Controls.app" "$output/temporary" "$output/modules"
export TMPDIR="$output/temporary" CLANG_MODULE_CACHE_PATH="$output/modules"

compiler=$(/usr/bin/xcrun --find swiftc)
sdk=$(/usr/bin/xcrun --sdk iphonesimulator --show-sdk-path)

"$compiler" --version

/usr/bin/xcrun clang -target arm64-apple-ios17.0-simulator \
	-isysroot "$sdk" -I "$source_root/include" \
	-c "$controls/NativeFixture.c" -o "$output/fixture.o"

/usr/bin/xcrun clang -target arm64-apple-ios17.0-simulator \
	-isysroot "$sdk" -I "$source_root/include" \
	-c "$controls/UnusedBoundary.c" -o "$output/unused.o"

"$compiler" -emit-library -static -emit-module -module-name LeptosKitBridge \
	-swift-version "$swift_language_version" -target arm64-apple-ios17.0-simulator -sdk "$sdk" \
	-I "$source_root/include" "$source_root"/apple/*.swift \
	-o "$output/libLeptosKitBridge.a" \
	-emit-module-path "$output/LeptosKitBridge.swiftmodule"

"$compiler" -swift-version "$swift_language_version" -target arm64-apple-ios17.0-simulator -sdk "$sdk" \
	-I "$source_root/include" -I "$output" -L "$output" -lLeptosKitBridge \
	"$source_root"/mobile/*.swift "$controls/UIKitControls.swift" \
	"$output/fixture.o" "$output/unused.o" -o "$output/Controls.app/Controls"

cat >"$output/Controls.app/Info.plist" <<'PLIST'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>CFBundleIdentifier</key><string>dev.leptoskit.join-controls</string>
<key>CFBundleExecutable</key><string>Controls</string>
<key>CFBundleName</key><string>Controls</string>
<key>CFBundleVersion</key><string>1</string>
<key>CFBundlePackageType</key><string>APPL</string>
<key>LSRequiresIPhoneOS</key><true/>
<key>UILaunchScreen</key><dict/>
</dict></plist>
PLIST

/usr/bin/codesign --force --sign - "$output/Controls.app"
