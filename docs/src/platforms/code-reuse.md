# Code reuse across platforms

A shared screen returns an `Element` tree built with leptoskit components. Native surfaces and the optional Leptos web renderer consume the same tree. Application code can share its screen construction, state, and callbacks without a second HTML implementation.

## One screen function

```rust
use leptoskit::prelude::*;

pub fn counter(count: usize, increment: impl Fn() + Send + Sync + 'static) -> Element {
	View()
		.class("w-full gap-4 p-6 md:flex-row")
		.child(Text(format!("Count: {count}")).class("text-3xl font-bold"))
		.child(
			Button("Increment")
				.class("rounded-lg bg-indigo-500 px-4 py-2 text-white")
				.on_press(increment),
		)
}
```

The [built-in components](../reference/components.md) are Rust builder functions. They are not `#[component]` functions or `<View>` tags. Each returns an `Element` with the common layout, props, and event vocabulary. Layout wrappers and presentation builders compose existing elements. Interactive builders use real AppKit or DOM controls with application-owned values.

The built-in class compiler interprets the supported Tailwind subset. Native resolution produces Yoga styles and widget props. Web rendering generates CSS for the same declarations. [CSS coverage](../reference/css-coverage.md) records the supported behavior and limits.

## Native rendering

The native host commits the shared screen and forwards platform events:

```rust
use leptoskit::native::HeadlessBackend;
use leptoskit::prelude::*;

let mut surface = NativeSurface::new(HeadlessBackend::new());
surface.set_viewport(390.0, 844.0);
surface.set_root(&counter(0, || println!("increment")));
```

Use `AppKitBackend` to render native macOS widgets. Other native platforms need a `NativeBackend` implementation. The headless backend validates the tree, layout, and mutation stream without a widget toolkit.

The host rebuilds the tree when application state changes. The surface retains unresolved classes and updates responsive layout when the host calls `set_viewport`. `set_dark_mode` updates dark variants. Repeated environment values do not trigger a new commit.

## Web rendering

Enable `leptoskit`'s optional `web` feature and select the appropriate Leptos rendering feature for your target. Render the same function from a reactive closure:

```rust
use leptos::prelude::Get;
use leptos::prelude::IntoView;
use leptos::prelude::RwSignal;
use leptos::prelude::Update;

pub fn browser_root() -> impl IntoView {
	let count = RwSignal::new(0usize);
	move || {
		leptoskit::web::render(counter(count.get(), move || {
			count.update(|count| *count += 1)
		}))
	}
}
```

The web renderer maps the shared controls to DOM elements and connects the shared event handlers. Leptos handles DOM reconciliation, server rendering, and hydration. Browser media queries update responsive classes without rebuilding the tree. Yoga computes native frames. The browser computes web layout. Font metrics and unsupported CSS behavior can differ between them.

## Platform boundaries

The screen tree is shared. The host still owns its window, run loop, viewport, appearance updates, asset locations, and application lifecycle. Native event support depends on the backend. AppKit currently sends press, text change, submit, focus, and blur events. It does not deliver scroll callbacks.

`NativeKind::Custom` and `NativeProps::extra` belong to platform integrations. The web renderer rejects custom native widgets. It also rejects children of `Image` and `TextInput`, which map to void HTML controls.

`Image` accepts a source string understood by the backend. AppKit loads local paths and `file://` URLs. The browser loads browser URLs. A shared asset catalogue can provide the source appropriate to the host.

## Shared layers

| Layer                             | Shared implementation                     |
| --------------------------------- | ----------------------------------------- |
| Components and callbacks          | `leptoskit::components` and `Element`     |
| Tailwind parsing and declarations | `leptoskit_native::tailwind`              |
| Native layout and reconciliation  | `leptoskit_yoga` and `leptoskit_native`   |
| Browser rendering                 | Optional `leptoskit::web`, using Leptos   |
| Native wasm runtime and updates   | `leptoskit_wasm` and `leptoskit_codepush` |
| Configuration                     | `leptoskit_config`                        |

The [shared component guide](../guide/07-shared-components.md) covers class validation, typed base styles, controls, and scrolling. The [platform probe](https://github.com/ifiokjr/leptoskit/tree/main/examples/platform_probe) exercises native runtime and update contracts. It does not substitute for a platform widget backend or CSS conformance tests.
