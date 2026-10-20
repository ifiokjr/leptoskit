---
"release": minor
---

# Ship prebuilt leptoskit binaries on the GitHub release

The publish workflow now builds the `leptoskit` CLI for eight targets — linux (gnu and musl) on both architectures, macOS on both architectures, and windows-msvc on both architectures — and attaches the archives and their sha256 and sha512 checksums to the release, with build-provenance attestations verified before anything is published. The release only becomes public after the binaries are attached and every crate is on crates.io, so a published version always has installable artifacts.

The CLI crate carries `cargo-binstall` metadata resolving to those archives, so `cargo binstall leptoskit_cli` installs the prebuilt binary instead of compiling from source.
