#!/usr/bin/env bash
# Converts the raw LLVM profile of an instrumented device run into an lcov
# tracefile that coverage_gate can merge with the host run.
#
#   export_device_coverage.sh <profile.profraw> <instrumented binary> <output.lcov>
#
# The device verifiers (`verify_simulator --coverage`, `verify_device
# --coverage`) leave the profile and the binary that wrote it. Source paths are
# written relative to the repository, because the device job runs on a
# different runner, and so under a different checkout path, than the job that
# gates coverage. Toolchain and dependency sources are dropped.
set -euo pipefail

if [ "$#" -ne 3 ]; then
	echo "usage: $0 <profile.profraw> <instrumented binary> <output.lcov>" >&2
	exit 2
fi
profile="$1"
object="$2"
output="$3"

# An absent or empty profile means the instrumented run never flushed, which
# must fail here rather than reach the gate as a report with nothing in it.
if [ ! -s "$profile" ]; then
	echo "no LLVM profile at $profile" >&2
	exit 1
fi

# The LLVM tools must match the compiler that instrumented the binary, so they
# come from the pinned toolchain's llvm-tools component.
sysroot="$(rustc --print sysroot)"
host="$(rustc -vV | sed -n 's/^host: //p')"
tools="$sysroot/lib/rustlib/$host/bin"
root="$(git rev-parse --show-toplevel)"
scratch="$(mktemp -d)"
trap 'rm -rf "$scratch"' EXIT

mkdir -p "$(dirname "$output")"
"$tools/llvm-profdata" merge -sparse "$profile" -o "$scratch/profile.profdata"
"$tools/llvm-cov" export --format=lcov --instr-profile "$scratch/profile.profdata" "$object" \
	--ignore-filename-regex '(\.cargo/(registry|git)/|/rustc/|\.rustup/)' >"$scratch/absolute.lcov"
sed "s|^SF:$root/|SF:|" "$scratch/absolute.lcov" >"$output"

if ! grep -q '^SF:[^/]' "$output"; then
	echo "$output names no repository source" >&2
	exit 1
fi
echo "wrote $output ($(grep -c '^SF:' "$output") files)"
