# leptoskit_plugin_autostart

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
