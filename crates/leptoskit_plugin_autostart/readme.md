<h1 align="center">leptoskit_plugin_autostart</h1>

<p align="center">
  <strong>leptoskit_plugin_autostart</strong> is the native-host plugin for explicit per-user desktop login registration with a validated identity and command.
</p>

<br>

<p align="center">
  <a href="#overview"><strong>Overview</strong></a> ·
  <a href="#how-it-fits"><strong>How it fits</strong></a> ·
  <a href="#use-it"><strong>Use it</strong></a> ·
  <a href="#learn-more"><strong>Learn more</strong></a> ·
  <a href="https://docs.rs/leptoskit_plugin_autostart"><strong>Docs</strong></a> ·
  <a href="https://github.com/ifiokjr/leptoskit"><strong>Repository</strong></a>
</p>

<br>

<p align="center">
  <a href="https://crates.io/crates/leptoskit_plugin_autostart"><img alt="crates.io" src="https://img.shields.io/crates/v/leptoskit_plugin_autostart.svg"></a>
  <a href="https://docs.rs/leptoskit_plugin_autostart"><img alt="docs.rs" src="https://docs.rs/leptoskit_plugin_autostart/badge.svg"></a>
  <a href="https://github.com/ifiokjr/leptoskit/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/ifiokjr/leptoskit/actions/workflows/ci.yml/badge.svg"></a>
  <a href="https://github.com/ifiokjr/leptoskit/blob/main/Cargo.toml"><img alt="MSRV" src="https://img.shields.io/badge/MSRV-1.95.0-blue.svg"></a>
  <a href="https://opensource.org/license/unlicense"><img alt="License: Unlicense" src="https://img.shields.io/badge/license-Unlicense-blue.svg"></a>
</p>

<br>

## Overview

Explicit per-user desktop login registration, configured by the native host. `Autostart::new(identity, absolute_executable, arguments)` only validates the configuration. Call `enable`, `disable`, or `is_registered` from Rust, or register the constructed `Autostart` with the bridge and grant the corresponding methods to use `guest::AutostartClient`.

| Platform          | Registration                                                                           |
| ----------------- | -------------------------------------------------------------------------------------- |
| macOS             | `~/Library/LaunchAgents/<identity>.plist`, with argument array and RunAtLoad           |
| Linux             | `$XDG_CONFIG_HOME/autostart/<identity>.desktop`, with correctly escaped Exec arguments |
| Windows           | Current user's `Software\Microsoft\Windows\CurrentVersion\Run` registry value          |
| iOS, Android, web | Unavailable                                                                            |

The identity must be stable and unique, such as a reverse DNS application ID. The executable must be an existing regular file, not an application directory. No shell is involved. Windows command lines use the standard CRT quoting rules and must fit the Run key's 260 UTF-16-unit command limit. Linux rejects executable paths containing `=`, as required by the desktop entry specification.

Registration reports the exact entry owned by this configuration, not whether OS policy or user startup preferences allow it to run. This does not modify Windows StartupApproved settings, bootstrap a LaunchAgent in the current login, or bypass desktop policy. A changed or unrelated entry is a conflict and is never overwritten or removed. Disable the old configuration before replacing its executable or arguments. Operations target the user's trusted configuration directory; hostile concurrent modification by the same user is out of scope. File creation is atomic and refuses to replace an existing entry or symlink.

Native registration needs app-specific configuration, so the crate intentionally has no automatic runtime constructor. Do not grant startup modification to an untrusted guest. Tests exercise temporary files and serialization; they never change the developer's login settings. Actual reboot/login behavior still needs packaged-app validation on each desktop OS.

References: [Tauri autostart](https://v2.tauri.app/plugin/autostart/), [Electron app lifecycle](https://www.electronjs.org/docs/latest/api/app), [Apple LaunchAgents](https://developer.apple.com/library/archive/documentation/MacOSX/Conceptual/BPSystemStartup/Chapters/CreatingLaunchdJobs.html), [XDG autostart](https://specifications.freedesktop.org/autostart/latest/), [desktop Exec quoting](https://specifications.freedesktop.org/desktop-entry/latest/exec-variables.html), [Windows Run keys](https://learn.microsoft.com/en-us/windows/win32/setupapi/run-and-runonce-registry-keys).

## How it fits

Native hosts construct it explicitly; it is not a guest plugin and installs nothing by default. Constructing the manager never enables login startup; registration is an explicit call, and mobile, browser, and unsupported desktops return `Unavailable`.

## Use it

```toml
# Host-only: construct it explicitly; there is no `native.plugins` entry.
[dependencies]
leptoskit_plugin_autostart = "0.1"
```

## Learn more

- [Crate documentation](https://docs.rs/leptoskit_plugin_autostart)
- [leptoskit documentation](https://ifiokjr.github.io/leptoskit/)
- [Repository](https://github.com/ifiokjr/leptoskit)
- [Plugin inventory](https://ifiokjr.github.io/leptoskit/platforms/plugin-inventory.html)
