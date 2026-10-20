#!/usr/bin/env bash
# Builds the benchmark fixture workspace under target/bench-fixture.
set -euo pipefail
root="$(git rev-parse --show-toplevel)"
fixture="$root/target/bench-fixture"
mkdir -p "$fixture/crates/fixture_client/src" "$fixture/crates/fixture_server/src"
cat >"$fixture/Cargo.toml" <<'EOF'
[workspace]
members = ["crates/*"]
resolver = "2"
EOF
cat >"$fixture/leptoskit.toml" <<'EOF'
[app]
name = "fixture_server"
version = "0.1.0"
default_platform = "web"
EOF
cat >"$fixture/crates/fixture_client/Cargo.toml" <<'EOF'
[package]
name = "fixture_client"
version = "0.1.0"
edition = "2021"

[lib]
crate-type = ["cdylib"]

[dependencies]
wasm-bindgen = "=0.2.129"
EOF
cat >"$fixture/crates/fixture_client/src/lib.rs" <<'EOF'
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
pub fn greet(name: &str) -> String {
    format!("hello, {name}!")
}
EOF
cat >"$fixture/crates/fixture_server/Cargo.toml" <<'EOF'
[package]
name = "fixture_server"
version = "0.1.0"
edition = "2021"
EOF
cat >"$fixture/crates/fixture_server/src/main.rs" <<'EOF'
fn main() {
    println!("server");
}
EOF
echo "$fixture"
