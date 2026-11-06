# Changelog

All notable changes to this project are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.2.0](https://github.com/leptoskit/leptoskit/releases/tag/v0.2.0) (2026-11-05)

### Breaking changes

#### leptoskit_plugin_shell: child processes

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_plugin_shell` is the leptoskit plugin for scoped, bounded, cancellable desktop child processes built from host-defined command capabilities, such as running a bundled tool with `--version`.

Add `leptoskit_plugin_shell` as a direct dependency, select the `shell` plugin in `native.plugins`, and declare each command under `[native.plugin-options.shell.commands]`: an absolute executable and working directory, the allowed argument slots, an explicit environment, output limits, and a deadline. Guests enable the `guest` feature and call `guest::ShellClient::execute` with a command name and arguments; they never send a shell command string. Commands run on macOS, Linux, and Windows, and cancellation, timeouts, and shutdown clean up the child's process group or Windows Job Object. The generated Windows and Linux apps don't register runtime plugins yet, so there a host constructs `Shell` itself; mobile and the browser have no process backend.

### Fixes

- **Correct README claims about generated renderers, accessibility, runtime plugin registration, password screens, RPC macros, and interpreter updates.** Share crate summaries with Rust documentation and derive MSRV badges from Cargo through MDT templates, with a documentation drift check. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #515](https://github.com/leptoskit/leptoskit/pull/515)
