---
"release": minor
---

# Show buttons as pressed while they are held

A styled button showed no change while you pressed it, on the web and on every native platform. `Button` and `Pressable` now accept `active:` utilities, such as `active:bg-indigo-700 active:opacity-80`, and apply them while the control is held down: through CSS `:active` on the web, and through each platform's own pressed state on macOS, iOS, Android, Linux, and Windows. The web adapter's rendered root now listens passively for touches, because iOS Safari applies `:active` to a touched element only where the page listens for them. Only the held control is restyled, in place, keeping its accessible name and role. `active:` on any other element is a `ClassError`, since native platforms report presses only for controls.

Controls without `active:` utilities now dim to 70 percent of their opacity while held: every `Pressable`, and every `Button` with its own background, border, or corner radius. Buttons that keep their native chrome keep the platform's own highlight instead. The Windows backend no longer darkens held pressables, or buttons with their own background, border, or corners, on top of that, and Android buttons in non-scrolling containers show their pressed state on touch down rather than after the tap delay.

Backends report holds as the new `NativeEvent::PressIn` and `NativeEvent::PressOut`. `WasmHost::dispatch` handles them on the host without entering the guest, and other hosts pass them to the new `NativeSurface::press_in` and `NativeSurface::press_out`. `StyleContext::active` resolves a tree with its controls pressed, `lk_app_event` accepts kinds 8 and 9 for the generated apps' renderers, and `WidgetTester::hold` and `release` let widget tests check pressed styles. HarmonyOS buttons keep ArkUI's own press effect and don't apply `active:` yet.
