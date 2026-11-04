#!/usr/bin/env bash
# Runs one shard of the test suite: `cargo test --locked --all-features` over
# the shard's packages, passing any further arguments to cargo.
#
#   setup/scripts/test-shard.sh workspace -- --test-threads=1
#   setup/scripts/test-shard.sh windows --no-run
#
# Shards:
#
#   compile-errors   the compile-fail tests of `view!` and the RPC macros
#   database         the Postgres-backed data layer
#   cli              the CLI
#   workspace        every other workspace package
#   windows, macos   the packages whose code or tests depend on that OS
#
# The workspace suite runs once, on Linux, as the first four shards in
# parallel, so no CI job compiles and tests the whole workspace. `workspace`
# takes every package the other Linux shards don't name, so a new crate is
# tested without editing this file. Windows and macOS run only their own
# shard: a package belongs in it when its code or tests change with the OS,
# such as a widget backend or a plugin with a per-OS implementation.
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$repo_root"

database=(
	leptoskit_auth
	leptoskit_db
	leptoskit_db_client_fixture
	leptoskit_db_macros
	leptoskit_db_pack_fixture
	leptoskit_db_schema
	leptoskit_db_studio_fixture
	leptoskit_idp
)
cli=(leptoskit_cli)
# The trybuild tests compile every fixture against `leptoskit` with all of
# its features, the slowest single test in the suite.
compile_errors=(leptoskit leptoskit_rpc)
windows=(
	gallery
	leptoskit_plugin_arguments
	leptoskit_plugin_autostart
	leptoskit_plugin_dialog
	leptoskit_plugin_document_picker
	leptoskit_plugin_filesystem
	leptoskit_plugin_keep_awake
	leptoskit_plugin_logging
	leptoskit_plugin_menu
	leptoskit_plugin_network
	leptoskit_plugin_shell
	leptoskit_plugin_single_instance
	leptoskit_plugin_sqlite
	leptoskit_plugin_store
	leptoskit_plugin_tray
	leptoskit_windows
	platform_probe
)
macos=(
	gallery
	leptoskit_appkit
	leptoskit_bridge_ffi
	leptoskit_plugin_arguments
	leptoskit_plugin_autostart
	leptoskit_plugin_battery
	leptoskit_plugin_clipboard
	leptoskit_plugin_device
	leptoskit_plugin_dialog
	leptoskit_plugin_document_picker
	leptoskit_plugin_global_shortcut
	leptoskit_plugin_haptics
	leptoskit_plugin_keep_awake
	leptoskit_plugin_linking
	leptoskit_plugin_localization
	leptoskit_plugin_menu
	leptoskit_plugin_network
	leptoskit_plugin_secure_store
	leptoskit_plugin_shell
	leptoskit_plugin_single_instance
	leptoskit_plugin_tray
	leptoskit_plugin_web_auth
	leptoskit_plugin_window_state
	leptoskit_testing
	observatory_example
	platform_probe
	postcard_example
)

shard="${1:-}"
shift || true
targets=()
skip=()
case "$shard" in
compile-errors)
	selected=("${compile_errors[@]}")
	targets=(--test ui)
	;;
database) selected=("${database[@]}") ;;
cli) selected=("${cli[@]}") ;;
workspace)
	selected=()
	while IFS= read -r package; do
		selected+=("$package")
	done < <(
		cargo metadata --locked --no-deps --format-version 1 |
			jq -r '.packages[].name' |
			grep -vxF -f <(printf '%s\n' "${database[@]}" "${cli[@]}") |
			sort
	)
	# `compile-errors` runs these.
	skip=(--skip compile_errors)
	;;
windows) selected=("${windows[@]}") ;;
macos) selected=("${macos[@]}") ;;
*)
	echo "usage: $0 compile-errors|database|cli|workspace|windows|macos [cargo test arguments...]" >&2
	exit 2
	;;
esac

packages=()
for package in "${selected[@]}"; do
	packages+=(--package "$package")
done

arguments=("$@")
if [ "${#skip[@]}" -gt 0 ]; then
	separated=false
	for argument in "${arguments[@]+"${arguments[@]}"}"; do
		if [ "$argument" = "--" ]; then separated=true; fi
	done
	if [ "$separated" = false ]; then arguments+=(--); fi
	arguments+=("${skip[@]}")
fi

set -x
exec cargo test --locked --all-features "${packages[@]}" "${targets[@]+"${targets[@]}"}" "${arguments[@]+"${arguments[@]}"}"
