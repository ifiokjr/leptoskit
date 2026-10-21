#!/usr/bin/env bash
set -euo pipefail
mkdir -p "$HOME" "$XDG_RUNTIME_DIR" target/native-ui/linux target/native-ui/driver
chmod 700 "$XDG_RUNTIME_DIR"
cmake -S crates/leptoskit_testing/fixtures/linux -B target/native-ui/linux -G Ninja
cmake --build target/native-ui/linux
printf 'Leptoskit native file chooser evidence\n' >target/native-ui/linux/evidence.txt
export APPIUM_ARTIFACT_OUTPUT_PATH="$PWD/target/native-ui/driver"
exec selenium-webdriver-at-spi-run bash setup/scripts/native_linux_session.sh
