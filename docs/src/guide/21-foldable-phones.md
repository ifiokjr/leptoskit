# Foldable phones

A foldable phone changes shape while your app is running. Someone opens a Pixel Fold or an iPhone Duo and your app jumps from a narrow cover screen to a tablet-sized one. They half-close a Galaxy Z Flip and stand it on a table, and the bottom half of the screen becomes a stand. They open a Galaxy Z TriFold or a Huawei Mate XT one panel at a time. Your app keeps running through all of it: on iOS, Android, and in the browser, folding resizes the app instead of restarting it, so Tailwind's responsive classes and your app's state carry through.

Size alone does not tell you how the phone is folded. A half-open book fold is as wide as an open one, but content that runs across the crease is hard to read. The `posture` plugin tells you how the device is folded and where each fold crosses your window.

## Read the posture

Add the plugin to your app and to each native host, as with any [runtime plugin](../platforms/plugins.md), and grant it:

```toml
[native]
plugins = ["posture"]
```

Keep the posture in a signal and hand it to `use_posture`, from `leptoskit::hooks`, which turns it into memoized signals for your screens:

```rust
use leptos::prelude::*;
use leptoskit::hooks::Posture;
use leptoskit::hooks::use_posture;

let posture = RwSignal::new(Posture::default());
let fold = use_posture(posture);
```

Then keep the signal up to date. The plugin's `watch` future reports the current posture and every change after it, until you drop it. That is how you react to fold-driven configuration changes: folding updates WindowManager (Android), hinge geometry (iOS), or viewport segments (browsers), and `watch` delivers the new `Posture`. In a native build, set the signal and render after each change, and poll the future from your app's executor, the one its `leptoskit_poll` export drives:

```rust
let watching = leptoskit_plugin_posture::guest::DevicePosture::new()?.watch(move |current| {
	posture.set(current.clone());
	leptoskit::guest::render();
});
```

In a browser build, `leptoskit_plugin_posture::web::watch` takes the same callback, and leptos re-renders on its own.

Folding also changes the window size. Combine `use_posture` with `use_window_size` when a layout depends on both. On Android, the generated activity already lists `orientation|screenSize|screenLayout|smallestScreenSize` in `android:configChanges`, so the activity stays alive while the view resizes and the posture plugin reports the new fold.

## Lay out around the fold

A `Posture` answers the common questions, and `use_posture` offers each as a signal of the same name:

| Question                 | True when                                                                                      |
| ------------------------ | ---------------------------------------------------------------------------------------------- |
| `fold == Fold::Closed`   | The phone is folded shut and your app is on the cover screen.                                  |
| `fold == Fold::HalfOpen` | The phone is partly open.                                                                      |
| `fold == Fold::Open`     | The phone is open flat.                                                                        |
| `is_tabletop()`          | It is half open with the fold running side to side, like a laptop: the top half is the screen. |
| `is_book()`              | It is half open with the fold running top to bottom, held like a book.                         |
| `is_dual_pane()`         | `panes()` returns more than one region: a separating fold crosses the window.                  |
| `layout_mode()`          | `Tabletop`, `Book`, `DualPane`, or `Single`, in that priority order.                           |

Each hinge has an `angle` in degrees where the platform measures it, from 0 shut to 180 flat, and a `separating` flag when the platform says whether the hinge splits the window into independent areas. `panes()` is the window split around each separating fold. A pane is a region in fractions of the window, so it maps straight onto percentage sizes. A window with no separating fold across it is one pane — including a fully open continuous foldable whose crease does not separate the display.

### Tabletop (flip phones)

This screen shows a video in the top half and its controls in the bottom half when a flip phone stands on a table, and the two stacked normally otherwise:

```rust
#[component]
fn Screen(posture: &Posture) -> Element {
	let panes = posture.panes();
	if posture.is_tabletop() && panes.len() == 2 {
		let fold = 1.0 - panes[0].height - panes[1].height;
		view! {
			<View class="h-screen">
				{player().class(format!("h-[{}%]", panes[0].height * 100.0))}
				<View class=format!("h-[{}%]", fold * 100.0) />
				{controls().class(format!("h-[{}%]", panes[1].height * 100.0))}
			</View>
		}
	} else {
		view! { <View class="h-screen">{player()} {controls()}</View> }
	}
}
```

### Dual-pane (book folds and dual-screen)

When `is_dual_pane()` is true and the mode is not tabletop, put a list beside its detail (or any two columns) using the pane widths:

```rust
#[component]
fn Mail(posture: &Posture) -> Element {
	let panes = posture.panes();
	if posture.is_dual_pane() && panes.len() >= 2 {
		view! {
			<View class="flex-row h-screen">
				{inbox().class(format!("w-[{}%]", panes[0].width * 100.0))}
				{message().class(format!("w-[{}%]", panes[1].width * 100.0))}
			</View>
		}
	} else {
		view! { <View class="h-screen">{inbox()}</View> }
	}
}
```

`layout_mode()` is the short form of those checks: match on `Tabletop`, `Book`, `DualPane`, or `Single` when several screens share one posture signal.

The angle is also an input in its own right. A game can treat the hinge like a lever, or a reader can turn the page as the phone closes.

## What each platform reports

- **iOS:** an iPhone Duo on iOS 27.1 reports whether it is closed, half open, or open, the hinge angle, and where the fold crosses your window. Older iPhones report a phone that does not fold. Build with Xcode 27.1 or later: an app built with an older SDK cannot see the hinge and reports a phone that does not fold.
- **Android:** book folds, flips, and trifolds report each fold that crosses your window, with its position, whether it hides content, and whether it separates the window into independent panes (`FoldingFeature.isSeparating`). A fully open continuous foldable still reports its crease, but with `separating = false`, so `panes()` stays one pane and dual-pane layouts stay off until the device is half open or truly dual-screen. When the phone is folded onto its cover screen, or your app is in split screen, no fold crosses the window; the fold state then comes from the hinge angle sensor, on phones that have one. The angle is reported when one fold crosses the window. See [Android](../platforms/android.md#foldable-phones).
- **Browser:** Chrome and Edge report whether the device is half open and where the fold is. Browsers report no angle, and a closed phone looks the same as an open one.
- **HarmonyOS:** book folds, wide folds, and trifolds report each hinge's state, where each fold crosses your window, and, from HarmonyOS 5.0 (API 12), each hinge's angle. Folded onto the cover screen or in split screen, no fold crosses the window, and you get the fold state alone. Flip phones say they do not fold, so they report unknown. This hasn't been checked on a HarmonyOS emulator or device yet; see [HarmonyOS](../platforms/harmonyos.md#foldable-phones).
- **macOS, Windows, and Linux:** the plugin is unavailable.

Apps created before the foldable release declare which configuration changes their Android activity handles itself. Add `screenLayout|smallestScreenSize` to `android:configChanges` in `android/app/src/main/AndroidManifest.xml` so folding resizes the app instead of restarting it. New projects already include those flags with `orientation|screenSize`.

## Related: wide windows without a hinge

iPad Stage Manager, Android multi-window, and desktop free resizing change the window size without reporting a hinge. There is no fold posture to watch; use `use_window_size` (and Tailwind breakpoints) for a two-column layout when the window is wide enough. The dual-pane helpers above stay for real folds: same percentage-pane idea, different signal.
