#!/usr/bin/env bash
# Build the small native Android fixture without a Gradle dependency graph.
set -euo pipefail
root="$(cd "$(dirname "$0")/../.." && pwd)"
sdk="${ANDROID_HOME:?Set ANDROID_HOME to the Android SDK}"
tools="$sdk/build-tools/${ANDROID_BUILD_TOOLS:-35.0.0}"
platform="$sdk/platforms/android-35/android.jar"
out="$root/target/native-ui/android"
source_dir="$root/crates/leptoskit_testing/fixtures/android"
mkdir -p "$out/classes" "$out/dex"
javac -source 8 -target 8 -classpath "$platform" -d "$out/classes" "$source_dir/CaptureActivity.java"
"$tools/d8" --lib "$platform" --min-api 33 --output "$out/dex" "$out/classes/dev/leptoskit/capturelab/CaptureActivity.class"
"$tools/aapt" package -f -M "$source_dir/AndroidManifest.xml" -I "$platform" -F "$out/unsigned.apk"
(cd "$out/dex" && "$tools/aapt" add "$out/unsigned.apk" classes.dex)
"$tools/zipalign" -f 4 "$out/unsigned.apk" "$out/aligned.apk"
if [ ! -f "$out/fixture.keystore" ]; then
	keytool -genkeypair -keystore "$out/fixture.keystore" -storepass android -keypass android -alias fixture -dname 'CN=Leptoskit Test Fixture' -keyalg RSA -validity 3650 -noprompt
fi
"$tools/apksigner" sign --ks "$out/fixture.keystore" --ks-pass pass:android --key-pass pass:android --out "$out/CaptureLab.apk" "$out/aligned.apk"
printf '%s\n' "$out/CaptureLab.apk"
