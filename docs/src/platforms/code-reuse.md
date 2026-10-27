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

The [built-in primitives](../reference/components.md) are Rust builder functions. They are not `#[component]` functions or `<View>` tags. Each returns an `Element` with the common layout, props, and event vocabulary. Use `View` plus Tailwind for visual composition. Optional layout and presentation functions live under `components::helpers`. Interactive builders become real platform controls, or DOM controls on the web, and your app owns their values.

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

Swap `HeadlessBackend` for a platform backend to draw real widgets: `AppKitBackend`, `UIKitBackend`, `AndroidBackend`, `GtkBackend`, or `WindowsBackend`. The headless backend checks the tree, layout, and widget changes without drawing anything, which is what tests use. The apps `leptoskit new` generates create the surface for you.

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

The screen tree is shared. The host still owns its window, run loop, viewport, appearance updates, asset locations, and application lifecycle. Every native backend sends press, text change, submit, focus, blur, toggle, and value change events. Scroll events come from every native backend and the web, but not yet from the generated macOS, iOS, and Android apps, which also draw only nine components; see [platforms](readme.md#generated-apps-and-backend-crates). [Rive](../guide/20-rive.md) renders trusted vector files across the web and native hosts.

`NativeKind::Custom` and `NativeProps::extra` belong to platform integrations. The web renderer rejects custom native widgets. It also rejects children of `Image` and `TextInput`, which map to void HTML controls.

`Image` takes a source each platform must be able to load: a local path or `file://` URL on native platforms, a URL in the browser. A small function that picks the source per platform keeps screens shared.

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
