#!/usr/bin/env bash
# Runs clippy with every warning denied: rustc's, clippy's, and cargo's own,
# such as an unused dependency.
#
#   setup/scripts/clippy.sh                       the host, and the file routes example
#   setup/scripts/clippy.sh aarch64-apple-ios     the workspace for another target
#   setup/scripts/clippy.sh wasm32-unknown-unknown
#
# Code behind `cfg(target_os = …)` or `cfg(target_arch = "wasm32")` only
# compiles, and so only lints, for that target. CI runs this on each
# platform's runner with the targets it can check, so no platform's code goes
# unlinted.
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$repo_root"

# Git for Windows puts its own MSYS perl first on the path, and that perl
# lacks the modules OpenSSL's Configure needs to build `openssl-sys` with the
# `vendored` feature. Strawberry Perl, which the Windows runner also has,
# does. This is a no-op elsewhere, where the directory does not exist.
if [ -d /c/Strawberry/perl/bin ]; then
	export PATH="/c/Strawberry/perl/bin:$PATH"
fi

# `build.warnings` fails the run on any warning without changing rustc's
# flags, so it reuses the build cache, unlike `-D warnings`.
# `--keep-going` lints every package even after one fails, so a run reports
# every warning at once.
deny=(--locked --keep-going --config 'build.warnings="deny"')

host() {
	echo "clippy: host"
	cargo clippy "${deny[@]}" --workspace --all-features --all-targets
	echo "clippy: examples/file_routes"
	cargo clippy "${deny[@]}" --manifest-path examples/file_routes/Cargo.toml --all-targets
	echo "clippy: devtools"
	cargo clippy "${deny[@]}" --manifest-path devtools/Cargo.toml --all-targets
}

# The workspace with every feature on cannot build for wasm: features unify
# across packages and pull in host-only dependencies. The packages with
# browser or guest code are linted in groups instead, each with the widest
# set of features and targets that builds for wasm.
#
# Every feature, every target.
wasm_full=(
	gallery leptoskit_bench_counter leptoskit_bench_guest leptoskit_bridge leptoskit_extension
	leptoskit_l10n
	leptoskit_plugin_accelerometer leptoskit_plugin_application leptoskit_plugin_autostart
	leptoskit_plugin_barometer leptoskit_plugin_battery leptoskit_plugin_brightness
	leptoskit_plugin_crypto leptoskit_plugin_dialog leptoskit_plugin_document_picker
	leptoskit_plugin_global_shortcut leptoskit_plugin_gyroscope leptoskit_plugin_http
	leptoskit_plugin_image_manipulator leptoskit_plugin_light_sensor
	leptoskit_plugin_local_authentication leptoskit_plugin_magnetometer leptoskit_plugin_menu
	leptoskit_plugin_os leptoskit_plugin_passkeys leptoskit_plugin_pedometer
	leptoskit_plugin_screen_capture leptoskit_plugin_secure_store leptoskit_plugin_sharing
	leptoskit_plugin_shell leptoskit_plugin_single_instance leptoskit_plugin_sqlite
	leptoskit_plugin_store_review leptoskit_plugin_tracking_transparency leptoskit_plugin_tray
	leptoskit_plugin_web_auth leptoskit_plugin_websocket leptoskit_plugin_window_state leptoskit_query
	leptoskit_rive leptoskit_router leptoskit_testing_library plugin_guest_probe plugin_replies_probe
)
# Every feature, the library only: their tests need the host.
wasm_library=(
	leptoskit_native leptoskit_plugin_arguments leptoskit_plugin_clipboard
	leptoskit_plugin_device leptoskit_plugin_filesystem leptoskit_plugin_haptics
	leptoskit_plugin_keep_awake leptoskit_plugin_linking leptoskit_plugin_localization
	leptoskit_plugin_location leptoskit_plugin_logging leptoskit_plugin_network
	leptoskit_plugin_notifications leptoskit_plugin_permissions
	leptoskit_plugin_screen_orientation leptoskit_plugin_speech leptoskit_sensor
	leptoskit_testing leptoskit_use
)
# Default features, the library only: some of their features need the host.
wasm_default_library=(
	hooks_guest_probe islands_example leptoskit leptoskit_auth leptoskit_codepush leptoskit_db
	leptoskit_plugin_store leptoskit_tui
)
# Default features, every target: posture's `guest` feature leaves out the
# browser module, and with it the only use of the browser dependencies, and
# opencode's features build its native runner.
wasm_default=(leptoskit_plugin_posture opencode_example)

wasm() {
	wasm_group "every feature, every target" "${wasm_full[@]/#/--package=}" --all-features --all-targets
	wasm_group "every feature, library" "${wasm_library[@]/#/--package=}" --all-features --lib
	wasm_group "default features, library" "${wasm_default_library[@]/#/--package=}" --lib
	wasm_group "default features, every target" "${wasm_default[@]/#/--package=}" --all-targets
	# The DevTools interface is its own workspace, and a browser client only;
	# its `extension` feature builds the browser DevTools panel instead.
	wasm_group "the DevTools interface" --manifest-path devtools/Cargo.toml --lib
	wasm_group "the DevTools panel" --manifest-path devtools/Cargo.toml --lib --features extension
	# The DevTools web agent and its hooks; the `host` agent is native only.
	wasm_group "the DevTools web agent" --package=leptoskit_devtools --no-default-features --features web,query,form --lib
	# The RPC client, once per way it reaches the server: its server features
	# can't build for wasm, and `web` and `guest` pick different sockets.
	wasm_group "leptoskit_rpc in the browser" --package=leptoskit_rpc --no-default-features --features web --lib
	wasm_group "leptoskit_rpc in a guest" --package=leptoskit_rpc --no-default-features --features guest --lib
	# Huddle's app, once in the browser and once as the native apps' guest:
	# its `web` and `native` features pick different clients.
	wasm_group "huddle in the browser" --package=huddle_app --features csr --lib
	wasm_group "huddle in a guest" --package=huddle_app --features native --lib
}

wasm_group() {
	local description="$1"
	shift
	echo "clippy: wasm32-unknown-unknown, $description"
	cargo clippy "${deny[@]}" --target wasm32-unknown-unknown "$@"
}

if [ "$#" -eq 0 ]; then
	host
	exit 0
fi

# Dependencies with C sources (SQLite, ring, the tree-sitter grammars) need a
# C compiler for the target. The NDK's clang is Android's.
android_compiler() {
	local target="$1"
	# GitHub's Ubuntu runners name their newest NDK ANDROID_NDK_LATEST_HOME.
	ANDROID_NDK_HOME="${ANDROID_NDK_HOME:-${ANDROID_NDK_LATEST_HOME:-}}"
	if [ -z "$ANDROID_NDK_HOME" ]; then
		echo "clippy: set ANDROID_NDK_HOME to lint $target" >&2
		exit 1
	fi
	local host
	case "$(uname -s)" in
	Darwin) host=darwin-x86_64 ;;
	*) host=linux-x86_64 ;;
	esac
	local toolchain="$ANDROID_NDK_HOME/toolchains/llvm/prebuilt/$host"
	local variable="${target//-/_}"
	export "CC_$variable=$toolchain/bin/${target}24-clang"
	# The native Rive runtime builds C++.
	export "CXX_$variable=$toolchain/bin/${target}24-clang++"
	export "AR_$variable=$toolchain/bin/llvm-ar"
	# OpenSSL archives its static libraries with the toolchain's ranlib, and
	# derives the command from the target triple unless this is set, which names
	# a binary the runner does not have.
	export "RANLIB_$variable=$toolchain/bin/llvm-ranlib"
	# bindgen (QuickJS, in the native benchmark) reads the headers too.
	export "BINDGEN_EXTRA_CLANG_ARGS_$variable=--sysroot=$toolchain/sysroot"
}

for target in "$@"; do
	case "$target" in
	wasm32-unknown-unknown) wasm ;;
	*-linux-android)
		android_compiler "$target"
		echo "clippy: $target"
		cargo clippy "${deny[@]}" --workspace --all-features --all-targets --target "$target"
		;;
	*)
		echo "clippy: $target"
		cargo clippy "${deny[@]}" --workspace --all-features --all-targets --target "$target"
		;;
	esac
done
