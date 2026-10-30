---
"release": patch
---

# Stop building HarmonyOS apps as desktop Linux

Rust reports HarmonyOS (`aarch64-unknown-linux-ohos`) as `target_os = "linux"`, so HarmonyOS builds compiled every plugin's desktop Linux code and dependencies, such as arboard, rfd, D-Bus, keyring, global-hotkey, and the GTK backend. The plugins and the `leptoskit` crate now check `not(target_env = "ohos")` wherever they mean desktop Linux. On HarmonyOS, the `os` plugin reports the platform as `harmonyos` instead of `linux`, and `shell` no longer compiles process spawning: it answers every call with `Unavailable`.
