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
use leptoskit::hooks::{use_posture, Posture};

let posture = RwSignal::new(Posture::default());
let fold = use_posture(posture);
```

Then keep the signal up to date. The plugin's `watch` future reports the current posture and every change after it, until you drop it. In a native build, set the signal and render after each change, and poll the future from your app's executor, the one its `leptoskit_poll` export drives:

```rust
let watching = leptoskit_plugin_posture::guest::DevicePosture::new()?.watch(move |current| {
	posture.set(current.clone());
	leptoskit::guest::render();
});
```

In a browser build, `leptoskit_plugin_posture::web::watch` takes the same callback, and leptos re-renders on its own.

## Lay out around the fold

A `Posture` answers the common questions, and `use_posture` offers each as a signal of the same name:

| Question                 | True when                                                                                      |
| ------------------------ | ---------------------------------------------------------------------------------------------- |
| `fold == Fold::Closed`   | The phone is folded shut and your app is on the cover screen.                                  |
| `fold == Fold::HalfOpen` | The phone is partly open.                                                                      |
| `fold == Fold::Open`     | The phone is open flat.                                                                        |
| `is_tabletop()`          | It is half open with the fold running side to side, like a laptop: the top half is the screen. |
| `is_book()`              | It is half open with the fold running top to bottom, held like a book.                         |

Each hinge has an `angle` in degrees where the platform measures it, from 0 shut to 180 flat. `panes()` is the window split around each fold. A pane is a region in fractions of the window, so it maps straight onto percentage sizes. A window with no fold across it is one pane.

This screen shows a video in the top half and its controls in the bottom half when a flip phone stands on a table, and the two stacked normally otherwise:

```rust
fn screen(posture: &Posture) -> Element {
	let panes = posture.panes();
	if posture.is_tabletop() && panes.len() == 2 {
		let fold = 1.0 - panes[0].height - panes[1].height;
		View()
			.class("h-screen")
			.child(player().class(format!("h-[{}%]", panes[0].height * 100.0)))
			.child(View().class(format!("h-[{}%]", fold * 100.0)))
			.child(controls().class(format!("h-[{}%]", panes[1].height * 100.0)))
	} else {
		View().class("h-screen").child(player()).child(controls())
	}
}
```

The angle is also an input in its own right. A game can treat the hinge like a lever, or a reader can turn the page as the phone closes.

## What each platform reports

- **iOS:** an iPhone Duo on iOS 27.1 reports whether it is closed, half open, or open, the hinge angle, and where the fold crosses your window. Older iPhones report a phone that does not fold. Build with Xcode 27.1 or later: an app built with an older SDK cannot see the hinge and reports a phone that does not fold.
- **Android:** book folds, flips, and trifolds report each fold that crosses your window, with its position and whether it hides content. When the phone is folded onto its cover screen, or your app is in split screen, no fold crosses the window; the fold state then comes from the hinge angle sensor, on phones that have one. The angle is reported when one fold crosses the window.
- **Browser:** Chrome and Edge report whether the device is half open and where the fold is. Browsers report no angle, and a closed phone looks the same as an open one.
- **HarmonyOS:** book folds, wide folds, and trifolds report each hinge's state, where each fold crosses your window, and, from HarmonyOS 5.0 (API 12), each hinge's angle. Folded onto the cover screen or in split screen, no fold crosses the window, and you get the fold state alone. Flip phones say they do not fold, so they report unknown. This hasn't been checked on a HarmonyOS emulator or device yet; see [HarmonyOS](../platforms/harmonyos.md#foldable-phones).
- **macOS, Windows, and Linux:** the plugin is unavailable.

Apps created before this release declare which configuration changes their Android activity handles itself. Add `screenLayout|smallestScreenSize` to `android:configChanges` in `android/app/src/main/AndroidManifest.xml` so folding resizes the app instead of restarting it.
