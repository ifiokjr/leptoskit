<h1 align="center">leptoskit_plugin_menu</h1>

<p align="center">
  <strong>leptoskit_plugin_menu</strong> is the native-host plugin for bounded native menu definitions with explicit desktop ownership.
</p>

<br>

<p align="center">
  <a href="#overview"><strong>Overview</strong></a> ·
  <a href="#how-it-fits"><strong>How it fits</strong></a> ·
  <a href="#use-it"><strong>Use it</strong></a> ·
  <a href="#learn-more"><strong>Learn more</strong></a> ·
  <a href="https://docs.rs/leptoskit_plugin_menu"><strong>Docs</strong></a> ·
  <a href="https://github.com/ifiokjr/leptoskit"><strong>Repository</strong></a>
</p>

<br>

<p align="center">
  <a href="https://crates.io/crates/leptoskit_plugin_menu"><img alt="crates.io" src="https://img.shields.io/crates/v/leptoskit_plugin_menu.svg"></a>
  <a href="https://docs.rs/leptoskit_plugin_menu"><img alt="docs.rs" src="https://docs.rs/leptoskit_plugin_menu/badge.svg"></a>
  <a href="https://github.com/ifiokjr/leptoskit/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/ifiokjr/leptoskit/actions/workflows/ci.yml/badge.svg"></a>
  <a href="https://github.com/ifiokjr/leptoskit/blob/main/Cargo.toml"><img alt="MSRV" src="https://img.shields.io/badge/MSRV-1.95.0-blue.svg"></a>
  <a href="https://opensource.org/license/unlicense"><img alt="License: Unlicense" src="https://img.shields.io/badge/license-Unlicense-blue.svg"></a>
</p>

<br>

## Overview

Bounded native menu definitions and explicit desktop ownership, inspired by [Electron Menu](https://www.electronjs.org/docs/latest/api/menu) and [Tauri window menus](https://v2.tauri.app/learn/window-menu/). Uses [muda 0.21](https://docs.rs/muda/0.21.2/muda/) directly, without a Tauri application.

## Supported behavior

| Platform                | Menu model                                                  | Presentation                                                                   |
| ----------------------- | ----------------------------------------------------------- | ------------------------------------------------------------------------------ |
| macOS                   | Native commands, checks, separators, submenus, accelerators | Explicit application-menu lease; tray integration                              |
| Windows                 | Native commands, checks, separators, submenus, accelerators | Explicit HWND menu lease; tray integration                                     |
| Linux                   | GTK-free snapshot model, including live state changes       | KSNI tray integration through the tray crate; standalone menu bars unavailable |
| iOS / Android / browser | Definition validation only                                  | Unavailable; use the app's own UI                                              |

`supports_menu_bar()` reports standalone menu-bar availability. Desktop APIs are exported only on macOS, Windows, and Linux. There is no automatic native factory or guest registration: menus require an explicit UI-thread host and event loop. No global window lookup, detached UI worker, or implicit app activation occurs.

Definitions allow at most 256 total nodes, eight submenu levels, 64-byte IDs, 256-byte labels, and 64-byte accelerator strings. IDs must be unique throughout the tree. Accelerators are parsed before any native tree is allocated. For example, `CmdOrCtrl+S` maps to the platform's primary command modifier. Root application-menu items on macOS must be submenus; tray menus can be flat. Native mnemonic and keyboard-layout behavior follows muda and the platform. Predefined application roles, radio groups, icons, and standalone context-menu presentation are not implemented by this wrapper.

## Host integration

Create all menus on the host's UI thread. AppKit checks this at construction; `DesktopMenu` is not `Send` or `Sync` on any desktop platform. A host typically retains it in `Rc<DesktopMenu>` and gives the same owner to a tray.

Muda's callback is process-wide and can be installed only once. It silently ignores later installation attempts and cannot report the existing owner. This crate therefore never installs that callback for you. If another component already owns it, forward events from that callback into `EventSender` instead. Install it before the first native activation; otherwise muda initializes its unbounded default channel. Do not poll that upstream channel as a substitute for bounded forwarding. Keep one host event channel alive for the process lifetime, or retain a host dispatcher that can switch its downstream sender.

The callback is nonblocking, safe to invoke from the KSNI thread, and has no UI side effects. It drops newest activations when full and counts them through `take_dropped()` so the host can surface overload. Counts saturate. For a sleeping event loop, send a separate coalesced wake signal after forwarding. Do not create an unbounded wake/event queue. IDs are namespaced per native menu; dispatching an event to the wrong menu does nothing. Commands disabled while queued, including commands below disabled submenus, are ignored. Check state is read at dispatch time, so multiple queued clicks can report the same current state.

`native_menu()` exists for tray integration. Retain the wrapper alongside the native clone, and do not mutate its native structure or IDs; that bypasses the wrapper's validated bounds and routing. State methods (`set_enabled`, `set_checked`, `set_label`) preserve the native tree and update KSNI snapshots.

## Presentation and cleanup

On macOS, explicitly obtain the host's `NSApplication` on its main thread, then call `macos::ApplicationMenu::attach(menu, &application)`. Retain the returned lease. Only one lease can own the process's application-menu slot. Drop restores the saved menu only if the slot still contains this lease's menu; unrelated replacement menus are preserved. This affects this process only, not the user's other applications. The crate does not activate the app or create a window.

On Windows, `windows::WindowMenu::attach(menu, hwnd)` is unsafe because the host must keep its owned HWND and previous HMENU alive until the lease is dropped. Attach and drop on that window's thread, drop before destroying the window, and do not attach another muda menu or modify its native handles during the lease. The lease removes its native subclass and restores the previous menu at teardown. The host must call `translate_accelerator(&message)` before `TranslateMessage` and `DispatchMessage`; if it returns true, do not dispatch the message again. This connects native accelerators to the host's existing message loop.

Linux builds deliberately disable GTK3 and GTK4. The maintained snapshot backend feeds a KSNI tray without introducing deprecated GTK3 dependencies. It does not make standalone window menus available. A later GTK4 host integration needs an explicit GTK window and lifecycle, rather than a hidden GTK application here.

## Verification and limits

Pure tests cover duplicate IDs, labels, depth/node limits, and bounded callback overflow. The macOS main-thread integration test creates only its own invisible process menu, dispatches an AppKit item action, verifies state/event routing, checks ownership contention, and proves previous-menu restoration and unrelated replacement preservation. It does not open a visible menu or claim a real keyboard/input journey. Windows and Linux native code are cross-compiled; Windows message-loop behavior and a running Linux StatusNotifier host still need platform runtime tests. Browser/mobile builds validate definitions but export no native menu objects.

The Windows-only test creates a hidden HWND, sends `WM_COMMAND` to its menu subclass, checks event routing and disabled-command handling, and verifies previous-menu restoration before destroying its owned resources. It is included for Windows test runners and has been cross-compiled, not executed on this macOS development host.

## How it fits

Native hosts construct it explicitly; it is not a guest plugin and installs nothing by default. The desktop host owns the menu and installs it on its own windows; definitions are validated before any native allocation, and mobile and the browser expose no menu.

## Use it

```rust,no_run
# #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
# fn example() -> Result<(), leptoskit_plugin_menu::Error> {
use leptoskit_plugin_menu::{Definition, DesktopMenu, EventChannel, Item, muda};

let events = EventChannel::new(64)?;
let sender = events.sender();
// Exactly once, before any menu or tray can emit an activation:
muda::MenuEvent::set_event_handler(Some(move |event| sender.forward(event)));

let menu = DesktopMenu::new(Definition::new(vec![Item::Submenu {
    id: "file".into(),
    label: "File".into(),
    enabled: true,
    items: vec![Item::Command {
        id: "save".into(),
        label: "Save".into(),
        enabled: true,
        accelerator: Some("CmdOrCtrl+S".into()),
    }],
}])?)?;

// During each host event-loop turn, drain a bounded batch:
for _ in 0..64 {
    let Some(native) = events.try_recv() else { break };
    if let Some(event) = menu.handle_event(&native) {
        // Route event.id to the application's own command handler.
        assert_eq!(event.id, "save");
    }
}
menu.set_enabled("save", false)?;
# Ok(())
# }
```

## Learn more

- [Crate documentation](https://docs.rs/leptoskit_plugin_menu)
- [leptoskit documentation](https://ifiokjr.github.io/leptoskit/)
- [Repository](https://github.com/ifiokjr/leptoskit)
- [Plugin inventory](https://ifiokjr.github.io/leptoskit/platforms/plugin-inventory.html)
