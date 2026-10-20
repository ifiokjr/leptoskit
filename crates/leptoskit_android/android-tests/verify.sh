#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/../../.."
exec cargo run --locked --package leptoskit_android --example verify_device
