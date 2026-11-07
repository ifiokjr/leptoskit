# Watch UIs

<Callout kind="warning" title="Experimental">

Wear OS and watchOS support is early. The shared API, host stubs, and validation budgets are in place; packaging, simulator/device install, and live tile or complication updates are not. Use this to design watch surfaces, not to ship.

</Callout>

A phone screen is the wrong starting point for a watch. Wear OS tiles, watchOS glances, and face complications are tiny ambient surfaces with their own lifecycles. leptoskit gives them a dedicated API under `leptoskit::watch` that still reuses Tailwind styling and the shared primitives (`View`, `Text`, `Button`, …) inside a hard layout budget.

## The problem it solves

If you mount a phone file route on a watch, you get unreadably small chrome, widgets watches do not have (password fields, long scrollers), and no place for tiles or complications. `WatchApp` describes the surfaces a watch actually has:

| Surface             | Role                                | Typical budget        |
| ------------------- | ----------------------------------- | --------------------- |
| `WatchPage`         | Interactive watch app page          | ~32 nodes, short text |
| `WatchTile`         | Wear OS tile / watchOS accessory    | ~12 nodes             |
| `WatchGlance`       | Smart Stack / glance preview + body | ~8 nodes              |
| `WatchComplication` | Face slot                           | ~4 nodes              |

Navigation is crown- and swipe-sized: `Single`, `Stack`, `Pager`, or `ListDetail` — not the phone file router.

## Example

```rust
use leptoskit::prelude::*;
use leptoskit::watch::*;

fn steps_wear() -> WatchApp {
	WatchApp::new("steps", WatchKind::WearOs)
		.navigation(WatchNavigation::Stack)
		.page(
			WatchPage::new(
				"home",
				view! {
					<View class="items-center justify-center gap-2 p-2">
						<Text class="text-sm">"Steps"</Text>
						<Text class="text-2xl font-bold">"8,432"</Text>
						<Button>"Open"</Button>
					</View>
				},
			)
			.title("Steps"),
		)
		.tile(WatchTile::new(
			"steps-tile",
			view! { <Text>"8.4k"</Text> },
		))
		.complication(WatchComplication::new(
			"steps-face",
			ComplicationFamily::CircularSmall,
			view! { <Text>"8k"</Text> },
		))
}
```

The same vocabulary works for watchOS with `WatchKind::WatchOs` and `WatchGlance` instead of (or beside) tiles. Content still uses leptoskit elements and classes; the container enforces the budget before a host presents it.

## Platforms

| Platform        | Feature   | Host crate          | Status             |
| --------------- | --------- | ------------------- | ------------------ |
| Android Wear OS | `wearos`  | `leptoskit_wearos`  | Experimental stubs |
| Apple watchOS   | `watchos` | `leptoskit_watchos` | Experimental stubs |

Enable the shared API with the `watch` feature, or a platform feature that pulls it in:

```toml
leptoskit = { version = "0.3", features = ["wearos"] }
# or
leptoskit = { version = "0.3", features = ["watchos"] }
```

Create a starter:

```sh
leptoskit new steps --platform wearos
# or
leptoskit new steps --platform watchos
```

`leptoskit native prepare wearos` (or `watchos`) writes the host stubs. `leptoskit native build` for either platform stops with an experimental notice until packaging lands.

## What is refused on purpose

Watch surfaces reject phone-oriented widgets such as `ScrollView`, `TextInput`, `PasswordInput`, `Checkbox`, `Radio`, `Slider`, and `Rive`. Deep or wordy trees fail validation against the surface budget. That is intentional: a complication is not a settings screen.

## Permissions and entitlements

Documented beside each platform page:

- [Wear OS](wearos.md): watch feature flag, tile service, complication data source.
- [watchOS](watchos.md): Watch App target, WidgetKit, complication push, App Groups.
