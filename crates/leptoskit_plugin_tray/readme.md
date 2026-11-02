<h1 align="center">leptoskit_plugin_tray</h1>

<p align="center">
  <strong>leptoskit_plugin_tray</strong> is the native-host plugin for explicitly owned native desktop tray icons with bounded events.
</p>

<br>

<p align="center">
  <a href="#overview"><strong>Overview</strong></a> ·
  <a href="#how-it-fits"><strong>How it fits</strong></a> ·
  <a href="#use-it"><strong>Use it</strong></a> ·
  <a href="#learn-more"><strong>Learn more</strong></a> ·
  <a href="https://docs.rs/leptoskit_plugin_tray"><strong>Docs</strong></a> ·
  <a href="https://github.com/ifiokjr/leptoskit"><strong>Repository</strong></a>
</p>

<br>

<p align="center">
  <a href="https://crates.io/crates/leptoskit_plugin_tray"><img alt="crates.io" src="https://img.shields.io/crates/v/leptoskit_plugin_tray.svg"></a>
  <a href="https://docs.rs/leptoskit_plugin_tray"><img alt="docs.rs" src="https://docs.rs/leptoskit_plugin_tray/badge.svg"></a>
  <a href="https://github.com/ifiokjr/leptoskit/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/ifiokjr/leptoskit/actions/workflows/ci.yml/badge.svg"></a>
  <a href="https://github.com/ifiokjr/leptoskit/blob/main/Cargo.toml"><img alt="MSRV" src="https://img.shields.io/badge/MSRV-1.95.0-blue.svg"></a>
  <a href="https://opensource.org/license/unlicense"><img alt="License: Unlicense" src="https://img.shields.io/badge/license-Unlicense-blue.svg"></a>
</p>

<br>

## Overview

`leptoskit_plugin_tray` uses maintained `tray-icon` 0.26 and `muda` 0.21 through `leptoskit_plugin_menu`; Linux uses the GTK-free KSNI backend, with no GTK3/AppIndicator fallback or advisory exemptions. There is no Tauri application dependency.

The native application explicitly calls `TrayHost::install(waker)` after starting its event loop. This owns tray-icon's process-global callback; no import, default constructor, guest plugin registration, or background startup creates UI. Install before any other tray-icon consumer calls its event APIs: its callback is write-once and cannot report a previous third-party installation. Do not combine competing tray-icon owners in one process. Only one LeptosKit tray host can exist at a time; a later owner is allowed after teardown.

The host owns native tray resources and returns opaque IDs. `remove` purges the ID and queued events before dropping the native icon. Dropping the host closes the queue before releasing all icons and menus. Late native events for removed IDs are ignored. `wake` runs under the event-delivery lock and must only post a nonblocking wake to the host loop; it must not call back into this owner, UI, or a Wasm guest. Drop waits for an in-progress wake to finish. A panicking wake is caught and reported by the next `EventBatch`, never unwound through native code.

`RgbaIcon::new` validates exact row-major RGBA bytes and 1–256 pixel dimensions before native allocation. Tooltip length is capped at 127 UTF-16 units, title at 256 UTF-8 bytes, and control characters are rejected. Images are not decoded from filenames or untrusted compressed data. macOS can render an alpha template; other systems reject that option explicitly. No stable GUID/autosave identity is installed, so tests do not change permanent shell placement preferences.

Events use a fixed 64-slot queue. Adjacent pointer moves for the same icon coalesce; click order is preserved. Full queues drop new events and report `lost`; consumers must not assume press/release pairs remain complete after loss. Wakeups coalesce until drain. IDs are never reused, including across successive host owners. Native resources are capped at 16 icons on macOS/Windows and one on Linux. The Linux cap avoids competing upstream menu-change watcher receivers.

Menus are optional `Rc<leptoskit_plugin_menu::DesktopMenu>` owners. The tray retains the wrapper, preserving native items and ID mappings. Menu activation uses that crate's separate bounded event channel, which the application installs explicitly. The tray does not replace or steal muda's menu callback. Call `set_menu` to replace/remove a menu; item updates use the menu wrapper. A retained external menu remains usable after its tray is removed.

Platform contracts:

- **macOS:** create the host on the main thread while `NSApplication` is running. All operations/drop remain on that thread. Bounds, hover events, and raw button transitions follow AppKit. Showing a native context menu may consume button events. Title/template icons are supported.
- **Windows:** create and retain the owner on the native message-loop thread and keep pumping messages. Title/template requests return Unavailable. The library cannot prove that the host is pumping its loop; this is an application responsibility.
- **Linux:** uses ksni 0.3 over the pure-Rust zbus D-Bus implementation, so building needs no system libdbus. Running requires a session D-Bus and a desktop with a StatusNotifierItem host; creating a tray fails when no StatusNotifierWatcher is available. KSNI owns its service thread; no GTK event loop is required. Success means the D-Bus service/request was accepted, not that an icon is actually visible. A desktop may hide icons, ignore tooltip/title presentation, or have no tray host. Events represent left/middle _activation_, not raw click transitions; the desktop handles right-click menus. Bounds and coordinates are `None`, never invented zeros. Windowless service teardown joins native workers and may wait for D-Bus cleanup. This wrapper does not claim a bounded native teardown duration or observed visibility.
- **Browser/mobile:** host installation returns Unavailable. No fake tray or notification substitution and no guest API without an explicit safe host UI context.

Unit tests cover RGBA/text bounds, queue ordering/overflow/coalescing, removed-ID rejection, closed queues, and caught callback panic. `tests/native.rs` runs in the 3-OS test matrix. On macOS (a temporary AppKit status item) and Windows (a hidden Shell notification icon) it creates real icons, updates every property, fills capacity, removes them and tears down an owner holding a live icon. On Linux it checks that a refused session D-Bus returns an error without retaining an icon. Whether a desktop actually displays an icon is not observed.

Design references: [Electron Tray](https://www.electronjs.org/docs/latest/api/tray), [Tauri system tray](https://v2.tauri.app/learn/system-tray/), [tray-icon native requirements](https://docs.rs/tray-icon/latest/tray_icon/), [KSNI](https://docs.rs/ksni/latest/ksni/).

## How it fits

Native hosts construct it explicitly; it is not a guest plugin and installs nothing by default. The host calls `TrayHost::install` after its event loop starts and owns removal; Linux uses the GTK-free KSNI backend, and mobile and the browser have no tray.

## Use it

```toml
# Host-only: construct it explicitly; there is no `native.plugins` entry.
[dependencies]
leptoskit_plugin_tray = "0.1"
```

## Learn more

- [Crate documentation](https://docs.rs/leptoskit_plugin_tray)
- [leptoskit documentation](https://ifiokjr.github.io/leptoskit/)
- [Repository](https://github.com/ifiokjr/leptoskit)
- [Plugin inventory](https://ifiokjr.github.io/leptoskit/platforms/plugin-inventory.html)
