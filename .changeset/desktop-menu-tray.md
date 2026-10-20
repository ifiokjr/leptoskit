---
"leptoskit_plugin_menu": minor
"leptoskit_plugin_tray": minor
---

# Add host-owned desktop menus and tray icons

Add independent `menu` and `tray` crates built on muda and tray-icon, without a Tauri application. Menu definitions are bounded and validated before native allocation. macOS hosts attach an explicit application-menu lease and Windows hosts an HWND menu lease; dropping a lease restores the previous menu. Activations flow through a bounded event channel that the host installs explicitly.

The tray crate owns up to 16 icons on macOS/Windows and one KSNI icon on Linux, with validated RGBA images, optional menus, coalesced bounded events and ordered teardown. Linux deliberately avoids GTK; standalone Linux menu bars, standard roles, context menus and guest registration are not provided.
