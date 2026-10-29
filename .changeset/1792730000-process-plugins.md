---
"release": minor
---

# Plugins can be separate programs anyone can write

A `[[plugins]]` entry with a `command` — or an executable named `leptoskit-plugin-<name>` on PATH — is now a program leptoskit runs once per hook, with a versioned JSON envelope on standard input and a JSON reply on standard output. Any language works, nothing is linked into the CLI, and a failed program fails the hook loudly by name.

The program answers `describe` with its CLI namespace, supported platforms, and commands; `generate` with the files it wrote; the build hooks with extra cargo features, environment, and rustflags; and its own `plugin:<name> <command>` invocations with an exit code. `leptoskit generate tool-plugin <dir>` scaffolds a working Rust program implementing the protocol, ready for `cargo install --path`.
