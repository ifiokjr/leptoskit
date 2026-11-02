<h1 align="center">leptoskit_plugin_single_instance</h1>

<p align="center">
  <strong>leptoskit_plugin_single_instance</strong> is the native-host plugin for explicit process-lifetime instance locks.
</p>

<br>

<p align="center">
  <a href="#overview"><strong>Overview</strong></a> ·
  <a href="#how-it-fits"><strong>How it fits</strong></a> ·
  <a href="#use-it"><strong>Use it</strong></a> ·
  <a href="#learn-more"><strong>Learn more</strong></a> ·
  <a href="https://docs.rs/leptoskit_plugin_single_instance"><strong>Docs</strong></a> ·
  <a href="https://github.com/ifiokjr/leptoskit"><strong>Repository</strong></a>
</p>

<br>

<p align="center">
  <a href="https://crates.io/crates/leptoskit_plugin_single_instance"><img alt="crates.io" src="https://img.shields.io/crates/v/leptoskit_plugin_single_instance.svg"></a>
  <a href="https://docs.rs/leptoskit_plugin_single_instance"><img alt="docs.rs" src="https://docs.rs/leptoskit_plugin_single_instance/badge.svg"></a>
  <a href="https://github.com/ifiokjr/leptoskit/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/ifiokjr/leptoskit/actions/workflows/ci.yml/badge.svg"></a>
  <a href="https://github.com/ifiokjr/leptoskit/blob/main/Cargo.toml"><img alt="MSRV" src="https://img.shields.io/badge/MSRV-1.95.0-blue.svg"></a>
  <a href="https://opensource.org/license/unlicense"><img alt="License: Unlicense" src="https://img.shields.io/badge/license-Unlicense-blue.svg"></a>
</p>

<br>

## Overview

A native host startup plugin for mutual exclusion on macOS, Linux, and Windows. Call `InstanceGuard::acquire(private_directory, application_identity)` before constructing the app. `Some(guard)` means this process owns the identity; `None` means another cooperating process already holds it. Keep the guard alive for the application lifetime. OS lock ownership is released on guard drop or process exit, including a crash.

This implements the locking part of [Electron's single-instance lifecycle](https://www.electronjs.org/docs/latest/api/app#apprequestsingleinstancelockadditionaldata). Unlike the full [Tauri single-instance plugin](https://v2.tauri.app/plugin/single-instance/), it does **not** transfer launch arguments, emit activation events, or focus the original window. Activation handoff needs a bounded, authenticated per-user transport and is not implemented by this crate.

The caller supplies an existing absolute directory on a local filesystem. On Unix it must be owned by the effective user with no group/other permissions, and symlink/hard-link lock files are rejected. On Windows the caller must use an existing directory with a private user ACL, typically below LocalAppData; the crate does not edit ACLs. All app instances must choose the same identity and directory. Never unlink lock files while instances might be running: retained files prevent split-inode locks. The directory is a trusted host capability, not a guest-controlled path. Network filesystems and malicious processes with the same user's access are outside this lock's contract.

No guest bridge methods or automatic constructor are exposed because the host must decide whether to construct the application at all. Mobile and browser builds export no instance guard. The implementation uses [fs4](https://github.com/al8n/fs4) for OS advisory locks. Tests use temporary directories and separate child processes, including exclusion and reacquisition.

## How it fits

Native hosts construct it explicitly; it is not a guest plugin and installs nothing by default. Call `InstanceGuard::acquire` before constructing the app and keep the guard alive; macOS, Linux, and Windows release the OS lock on drop or process exit, including a crash.

## Use it

```toml
# Host-only: construct it explicitly; there is no `native.plugins` entry.
[dependencies]
leptoskit_plugin_single_instance = "0.1"
```

## Learn more

- [Crate documentation](https://docs.rs/leptoskit_plugin_single_instance)
- [leptoskit documentation](https://ifiokjr.github.io/leptoskit/)
- [Repository](https://github.com/ifiokjr/leptoskit)
- [Plugin inventory](https://ifiokjr.github.io/leptoskit/platforms/plugin-inventory.html)
