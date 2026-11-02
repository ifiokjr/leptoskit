---
leptoskit_plugin_single_instance: breaking
---

# leptoskit_plugin_single_instance: single-instance locks

`leptoskit_plugin_single_instance` is the native-host plugin for explicit process-lifetime instance locks.

Native hosts construct it explicitly; it is not a guest plugin and installs nothing by default. Call `InstanceGuard::acquire` before constructing the app and keep the guard alive; macOS, Linux, and Windows release the OS lock on drop or process exit, including a crash.
