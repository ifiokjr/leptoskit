---
leptoskit_plugin_menu: minor
---

# Builder-friendly native menus

`leptoskit_plugin_menu` adds chained constructors (`Menu`, `MenuBar`, `Submenu`, `Command`, `Check`, `Separator`) that build the same validated definitions hosts already attach. On macOS, `ApplicationMenu::install` builds and leases the AppKit menu bar in one step; Windows gains the matching `WindowMenu::install` helper. Linux still has no standalone menu bar and exposes `linux::attach_menu_bar` as an explicit `Unavailable` stub; tray / status-item menus keep sharing `DesktopMenu` with `leptoskit_plugin_tray`. Docs cover the macOS path and the Windows/Linux parity gaps.
