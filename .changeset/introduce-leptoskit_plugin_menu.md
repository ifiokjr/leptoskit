---
leptoskit_plugin_menu: major
---

# leptoskit_plugin_menu: native menus

`leptoskit_plugin_menu` is the native-host plugin for bounded native menu definitions with explicit desktop ownership: commands, checkboxes, separators, submenus, and accelerators such as `CmdOrCtrl+S`.

Native hosts construct it explicitly on their UI thread; it is not a guest plugin and installs nothing by default. macOS attaches it as the application menu and Windows attaches it to a window the host owns; Linux has no standalone menu bar, so there it only feeds tray menus. Definitions are validated before any native allocation, and iOS, Android, and the browser only validate definitions and show no menu.
