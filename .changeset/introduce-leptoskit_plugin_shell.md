---
leptoskit_plugin_shell: breaking
---

# leptoskit_plugin_shell: child processes

`leptoskit_plugin_shell` is the native-host plugin for scoped, bounded, cancellable desktop child processes built from host-defined command capabilities.

Native hosts construct it explicitly; it is not a guest plugin and installs nothing by default. macOS, Linux, and Windows run only absolute executables with fixed argument slots and an explicit environment; the host never accepts a shell command string from a guest.
