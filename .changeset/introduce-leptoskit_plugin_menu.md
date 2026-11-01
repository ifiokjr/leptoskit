---
leptoskit_plugin_menu: breaking
---

# leptoskit_plugin_menu: native menus

`leptoskit_plugin_menu` is the native-host plugin for bounded native menu definitions with explicit desktop ownership.

Native hosts construct it explicitly; it is not a guest plugin and installs nothing by default. The desktop host owns the menu and installs it on its own windows; definitions are validated before any native allocation, and mobile and the browser expose no menu.
