# Code reuse across platforms

The economics of a cross-platform toolkit live in one question: how much of the app do you write once? For leptoskit the answer is almost all of it — everything except the shell that boots your app on each OS.

## The layering

```
your app
├── components, pages, routing            ← write once
├── server functions (`leptoskit_fns`)    ← write once
├── styles (leptoskit_yoga `Style`)       ← write once
└── platform shells                       ← ~50 lines each, mostly config
```

### Components: written once

A component returns [`Element`](../../crates/leptoskit_native) values on native and leptos views on the web. The idiomatic pattern keeps the component's _state and logic_ in plain Rust and puts only the _tree construction_ behind a per-platform view layer:

```rust
// One model, shared by every platform.
pub struct CounterModel {
	pub count: RwSignal<u32>,
}

impl CounterModel {
	pub fn new() -> Self {
		Self {
			count: RwSignal::new(0),
		}
	}

	pub fn increment(&self) {
		self.count.update(|count| *count += 1);
	}
}

// The web view: leptos components.
pub fn counter_web(model: CounterModel) -> impl IntoView {
	view! {
		<div>
			<span>{move || model.count.get()}
			<button on:click=move |_| model.increment()>"Increment"</button>
		</div>
	}
}

// The native view: the same state, driving native widgets.
pub fn counter_native(model: CounterModel) -> Element {
	Element::view()
		.child(Element::text({
			let count = model.count;
			format!("Count: {}", count.get_untracked())
		}))
		.child(
			Element::button("Increment").on(NativeEventSlot::Press, move |_| {
				model.increment();
				EventOutcome::Handled
			}),
		)
}
```

The model has no platform types in it; each view is a thin translation. Server functions are the strongest form of reuse — the exact same `#[server]` call runs from the browser, from SSR, and from a native shell, because the transport is HTTP on every platform.

### Styles: written once

Native styles are [`leptoskit_yoga::Style`](../../crates/leptoskit_yoga) values — flexbox and CSS grid, the same properties the web uses. A style written for the native side reads naturally to anyone who has written CSS, and the layout numbers agree between platforms because the engine is the same Rust code everywhere:

```rust
pub fn card() -> Style {
	Style {
		display: Display::Flex,
		flex_direction: FlexDirection::Column,
		padding: leptoskit_yoga::Rect::all_edges(
			Dimension::Points(0.0),
			Dimension::Points(16.0),
			Dimension::Points(0.0),
			Dimension::Points(16.0),
		),
		gap: leptoskit_yoga::Rect {
			top: Dimension::Points(8.0),
			..Default::default()
		},
		..Style::default()
	}
}
```

Grid templates, `fr` tracks, `minmax`, alignment — all shared.

### Platform shells: written per platform, and small

The shell does three things, and only three things:

1. **Boot the runtime.** Create a [`WasmHost`](../../crates/leptoskit_wasm) over the platform's `NativeBackend` and the app's wasm bundle.
2. **Pump the run loop.** Forward touch, key, and scroll events through the host's dispatch; the wasm side rebuilds and the renderer commits the diff.
3. **Check for updates.** Run [`CodePushClient`](../../crates/leptoskit_codepush) on launch (and on demand) against your release server; a new interface version downloads as a delta, verifies, and stages for the next launch.

Everything past those three responsibilities is shared Rust that already runs on every target — the `platform_probe` suite is the proof: the same binary source validates the wasm ABI runtime, renderer, layout, and a delta update on desktop, iOS, and Android.

## Where the seams are

Be honest about the two places platform reality shows through:

- **Widget vocabulary.** `NativeKind` covers the common widgets (view, text, image, scroll view, text input, button). Platform-specific widgets ride in as `NativeKind::Custom("name")` with props in `NativeProps::extra`; the shell maps them to the platform's widget. Keep `Custom` usage wrapped in a small named constructor so the rest of the app stays portable.
- **Input and lifecycle events.** The event set in `NativeEvent` is the common denominator. Platform-unique events arrive as `Custom` slots the same way widgets do.

## Over-the-air updates: the payoff

Because the interface ships as a wasm bundle, a platform shell approved by an app store never contains your UI logic — it contains the runtime. Shipping a new interface is a release on a channel:

```bash
leptoskit build --release --platform native
leptoskit codepush release --channel production
```

Clients on the previous version receive a binary delta (measured in the probe: a bundle differing only in its marker bytes transfers as a fraction of the full container), verify its hash, and stage it. The next launch runs the new interface — no store review, instant rollback by shipping the older version again.

## What is shared, concretely

| Layer         | Crate                                | Shared across                            |
| ------------- | ------------------------------------ | ---------------------------------------- |
| Layout        | `leptoskit_yoga`                     | all platforms, byte-for-byte same engine |
| Renderer      | `leptoskit_native`                   | all native platforms                     |
| Runtime       | `leptoskit_wasm`                     | all native platforms                     |
| Updates       | `leptoskit_codepush`                 | all native platforms                     |
| Server        | `leptoskit_axum` + leptos server fns | web, SSR, and every native client        |
| Configuration | `leptoskit_config`                   | one `leptoskit.toml` for everything      |
