---
leptoskit_cli: breaking
---

# leptoskit_cli: the command line

`leptoskit_cli` is the `leptoskit` and `lk` command: scaffold a project, run every platform, build, serve, generate routes and translations, manage code push, lint, format, and inspect toolchains.

Every workflow starts here. The binary is a thin wrapper over the library, so the same code drives `leptoskit dev`, CI builds, native host preparation, and release checks. Generated projects call it by name; tooling calls the library directly.
