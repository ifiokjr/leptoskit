<h1 align="center">leptoskit_plugin_shell</h1>

<p align="center">
  <strong>leptoskit_plugin_shell</strong> is the native-host plugin for scoped, bounded, cancellable desktop child processes built from host-defined command capabilities.
</p>

<br>

<p align="center">
  <a href="#overview"><strong>Overview</strong></a> ·
  <a href="#how-it-fits"><strong>How it fits</strong></a> ·
  <a href="#use-it"><strong>Use it</strong></a> ·
  <a href="#learn-more"><strong>Learn more</strong></a> ·
  <a href="https://docs.rs/leptoskit_plugin_shell"><strong>Docs</strong></a> ·
  <a href="https://github.com/ifiokjr/leptoskit"><strong>Repository</strong></a>
</p>

<br>

<p align="center">
  <a href="https://crates.io/crates/leptoskit_plugin_shell"><img alt="crates.io" src="https://img.shields.io/crates/v/leptoskit_plugin_shell.svg"></a>
  <a href="https://docs.rs/leptoskit_plugin_shell"><img alt="docs.rs" src="https://docs.rs/leptoskit_plugin_shell/badge.svg"></a>
  <a href="https://github.com/ifiokjr/leptoskit/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/ifiokjr/leptoskit/actions/workflows/ci.yml/badge.svg"></a>
  <a href="https://github.com/ifiokjr/leptoskit/blob/main/Cargo.toml"><img alt="MSRV" src="https://img.shields.io/badge/MSRV-1.95.0-blue.svg"></a>
  <a href="https://opensource.org/license/unlicense"><img alt="License: Unlicense" src="https://img.shields.io/badge/license-Unlicense-blue.svg"></a>
</p>

<br>

## Overview

Scoped asynchronous child processes for macOS, Linux, and Windows. The host constructs `Shell` with a named map of `CommandPolicy` values and a limit of 1–64 concurrent children. Construction validates configuration and starts no process.

Each scope fixes an absolute executable, absolute working directory, argument slots, exact environment, output limits, and deadline. Executable and directory paths are canonicalized. Arguments must match an exact value, an enumerated value, or a bounded regular expression matching the entire argument. Missing and extra arguments are denied. The environment is cleared before installing the configured values, standard input is closed, and no PATH lookup or shell string interpolation occurs. Windows batch scripts are rejected because Windows executes them through `cmd.exe`. The host must not expose an interpreter or broad argument pattern that defeats its intended capability policy. Paths must remain under trusted host control; canonicalization is not protection against a same-user attacker replacing an executable later.

`Shell::spawn(&request)` returns an owned `RunningChild`. `wait().await` yields binary stdout/stderr, the normal exit code when available, and an explicit `Finish` reason. Nonzero exits are ordinary unsuccessful results. Cancellation, timeout, and output limits return bounded partial output with `success = false`. Requests are limited to 128 arguments and 64 KiB. Combined capture is limited to 4 MiB. Runtime limits include execution and pipe draining, so inherited pipes cannot hang capture indefinitely.

`RunningChild::cancel_and_wait()` kills and reaps the direct child. `cancellation()` provides an idempotent signal suitable for bridge cancellation. Dropping the owner or aborting its wait future signals the supervising task, which keeps the child and concurrency permit until cleanup finishes. Keep the Tokio runtime, with I/O and time enabled, alive until explicit `wait` or `Cancellation::closed` completes during shutdown. Forced runtime destruction falls back to Tokio's kill-on-drop and best-effort reaping; it cannot provide the same completion guarantee. Descendant process groups/Windows Job Objects are not managed. Scope commands accordingly: this is direct-child supervision, not a process-tree sandbox.

For generated desktop apps, add this crate to the host and guest dependencies, enable `guest` on the guest dependency, and select `"shell"` in `native.plugins`. The CLI enables `async-host` only for the native build and installs `from_config` using the host-only options below. Command names and argument slots are explicit capabilities; the factory never invents an allowlist. Missing scopes, unknown fields, invalid paths or limits, and configuration over 64 KiB are rejected before execution. Selecting this plugin on native mobile returns `Unavailable` during app creation.

An argument slot is `{ kind = "exact", value = "..." }`, `{ kind = "one-of", values = ["...", "..."] }`, or `{ kind = "pattern", expression = "...", max_bytes = 64 }`. Pattern slots still match the entire argument. `arguments` is required, including an explicit empty array for commands without arguments. Environment defaults to empty; capture/deadline defaults match `Limits`, and the default child limit is four.

The guest resolves `guest::ShellClient::new()` once and calls `execute(&Request)`. Requests and replies retain the existing binary protocol. Bridge cancellation signals the supervising child and waits for kill/reap before releasing call capacity; dropping an invocation future also retains cleanup ownership. App shutdown cancels every remaining child and waits for cleanup. A canceled guest call reports `Cancelled`; direct `RunningChild` callers can still receive partial output with `Finish::Cancelled`. Timeout/output-limit replies retain partial output and their explicit finish reason. The bridge reserves the largest configured encoded reply, including framing, before admitting execution.

Custom hosts can still use `Shell::dispatch`, forward cancellation, await the returned `RunningChild`, and complete the bridge on its owning thread. There is no synchronous `Plugin` implementation.

The binary request is command name, argument count, and argument strings. A response contains finish reason, success flag, exit-code presence/code bits, stdout, and stderr. Strict decoders reject malformed, oversized, or inconsistent results. Mobile, HarmonyOS, and web have no process backend (`is_available()` is false); guest resolution of an uninstalled shell capability returns `Unavailable`. URL opening belongs to the linking plugin, not child execution.

Tests spawn only the local Rust test executable, never shell utilities or network commands. They cover argument denial, full regex matching, nonzero exits, non-UTF-8 output, environment clearing, capacity, cancellation, dropped owners/futures, deadlines, both output limits, strict factory options, and shutdown. Unix adapter tests additionally use `waitpid` to verify the supervisor already reaped the child before completion.

```sh
cargo test -p leptoskit_plugin_shell --all-features
cargo clippy -p leptoskit_plugin_shell --all-features --all-targets -- -D warnings
cargo check -p leptoskit_plugin_shell --target wasm32-unknown-unknown --features guest
```

References: [Tauri shell scopes](https://v2.tauri.app/plugin/shell/), [Electron utility processes](https://www.electronjs.org/docs/latest/api/utility-process), [Tokio child lifecycle](https://docs.rs/tokio/latest/tokio/process/struct.Child.html), [Tokio kill-on-drop limits](https://docs.rs/tokio/latest/tokio/process/struct.Command.html#method.kill_on_drop).

## How it fits

Native hosts construct it explicitly; it is not a guest plugin and installs nothing by default. macOS, Linux, and Windows run only absolute executables with fixed argument slots and an explicit environment; the host never accepts a shell command string from a guest.

## Use it

```toml
[native.plugin-options.shell]
max_children = 4

[native.plugin-options.shell.commands.version]
executable = "/absolute/path/to/tool"
directory = "/absolute/trusted/directory"
arguments = [{ kind = "exact", value = "--version" }]
environment = {}
stdout_bytes = 262144
stderr_bytes = 262144
timeout_ms = 30000
```

## Learn more

- [Crate documentation](https://docs.rs/leptoskit_plugin_shell)
- [leptoskit documentation](https://ifiokjr.github.io/leptoskit/)
- [Repository](https://github.com/ifiokjr/leptoskit)
- [Plugin inventory](https://ifiokjr.github.io/leptoskit/platforms/plugin-inventory.html)
