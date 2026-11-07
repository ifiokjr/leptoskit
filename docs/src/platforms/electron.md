# Electron concepts in leptoskit

Electron's main-process APIs are a useful checklist of desktop responsibilities. leptoskit does not embed Chromium or Node; it maps those responsibilities onto runtime plugins and host-owned desktop services so the same app tree can still render as native widgets.

Use this page when you know the Electron API and need the leptoskit equivalent. The [plugin support matrix](plugin-inventory.md) says what each crate implements on each platform.

## Map

| Electron concept | leptoskit API | Notes |
| --- | --- | --- |
| `app` identity / version | `leptoskit_plugin_application` | Host packaging metadata, not the guest crate version. |
| `process.argv` | `leptoskit_plugin_arguments` | Immutable snapshot; host chooses guest disclosure. |
| `requestSingleInstanceLock` | `leptoskit_plugin_single_instance` | Lock plus second-launch `notify_running` / `poll_activation`. Focus is still host-owned. |
| `app.setLoginItemSettings` | `leptoskit_plugin_autostart` | Host-configured identity and absolute executable. |
| `BrowserWindow` bounds / fullscreen | `leptoskit_plugin_window_state` | Portable placement; owned capture/restore on macOS first. |
| `Menu` | `leptoskit_plugin_menu` | Application/window menus; roles and context menus remain. |
| `Tray` | `leptoskit_plugin_tray` | Host-owned icons and menus on macOS/Windows/Linux. |
| `globalShortcut` | `leptoskit_plugin_global_shortcut` | macOS/Windows/X11; Wayland unavailable. |
| `dialog` | `leptoskit_plugin_dialog` | Open/save/message dialogs with a parent window. |
| `Notification` | `leptoskit_plugin_notifications` | Immediate local posts; scheduling/actions remain. |
| `clipboard` | `leptoskit_plugin_clipboard` | Text today; images/HTML remain. |
| `shell.openExternal` / `openPath` | `leptoskit_plugin_linking`, `leptoskit_plugin_shell` | Linking opens validated URLs; shell runs scoped child processes. |
| `powerMonitor` | `leptoskit_plugin_power_monitor` | Host-pushed lifecycle events; Windows idle-time queries. |
| `powerSaveBlocker` | `leptoskit_plugin_keep_awake` | Display-idle inhibition leases. |
| Custom protocols / deep links / file opens | `leptoskit_plugin_deep_link` | Inbound queue + association descriptors; packaging still declares schemes/types. |
| Outbound `https` / `mailto` / `tel` | `leptoskit_plugin_linking` | Default policy rejects arbitrary custom schemes. |
| `autoUpdater` | `leptoskit_plugin_updater` | Native executable hooks via a host `Backend`. Not Wasm code push. |
| Guest UI over-the-air updates | `leptoskit_codepush` | Signed Wasm bundles, separate from native installers. |
| `utilityProcess` / `child_process` | `leptoskit_plugin_shell` | Bounded, scoped, cancellable children — no Node compatibility. |
| `safeStorage` | `leptoskit_plugin_secure_store` | OS credential entries, not a general encrypt facade. |
| Desktop capture / screenshots | `leptoskit_plugin_screen_capture` | Mobile secure-window / observation today; desktop pixel capture remains. |

## First-wave desktop lifecycle

A typical desktop host wires the pieces like this:

1. Acquire `InstanceGuard`. If that returns `None`, call `notify_running` and exit.
2. Construct `PowerMonitor`, `DeepLinks`, menus, tray, and shortcuts on the UI thread after the event loop exists.
3. Push OS open-URL / open-file / suspend / resume events into those queues as the platform delivers them.
4. Optionally register bridge plugins so the Wasm guest can drain events.
5. Supply an `updater::Backend` when the packaged app has a native update feed; use `UnavailableBackend` until then.

```rust,ignore
use leptoskit_plugin_deep_link::DeepLinks;
use leptoskit_plugin_power_monitor::Event;
use leptoskit_plugin_power_monitor::PowerMonitor;
use leptoskit_plugin_single_instance::Activation;
use leptoskit_plugin_single_instance::InstanceGuard;
use leptoskit_plugin_single_instance::notify_running;
use leptoskit_plugin_updater::UnavailableBackend;
use leptoskit_plugin_updater::Updater;

let Some(guard) = InstanceGuard::acquire(&runtime_dir, "dev.example.app")? else {
	notify_running(
		&runtime_dir,
		"dev.example.app",
		&Activation::new(std::env::args_os().next().unwrap(), std::env::args_os().skip(1).collect())?,
	)?;
	return Ok(());
};

let mut power = PowerMonitor::new();
power.push(Event::Resume);

let mut links = DeepLinks::new(["example".into()])?;
links.push_url("example://home")?;

let updater = Updater::new(UnavailableBackend);
let _ = (guard, power, links, updater);
```

## What this wave does not claim

- Writing Info.plist URL types, Windows registry verbs, or `.desktop` MimeType lines for you.
- macOS/Linux idle-time backends, logind suspend observers, or generated host event-loop wiring.
- A shipping update feed, delta encoder, or notarized installer pipeline inside `updater`.
- Desktop screenshot/recording capture, rich clipboard, notification action callbacks, or Wayland global shortcuts.
- Chromium `webContents`, BrowserView, session partitions, or Node integration — those are intentionally out of scope for a native-widget toolkit.

See [Where things stand](../readme.md#where-things-stand) and the matrix rows for each crate for the live gap list.
