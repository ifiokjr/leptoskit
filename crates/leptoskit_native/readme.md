<p align="center">
  <a href="https://ifiokjr.github.io/leptoskit/"><img src="https://raw.githubusercontent.com/ifiokjr/leptoskit/main/docs/src/assets/mark.svg" alt="leptoskit" width="64"></a>
</p>

# leptoskit_native

> The leptoskit native renderer: React Native's architecture, rewritten in Rust.

Apps describe their interface with shared Rust builders that return `Element` trees:

```rust
use leptoskit_native::components::{Button, Text, View};

let screen = View().class("gap-4 p-6 md:flex-row").children([
	Text("Hello, leptoskit").class("text-2xl font-bold"),
	Button("Continue").class("rounded-lg bg-indigo-500 px-4 py-2 text-white"),
]);
```

The [component catalog](https://ifiokjr.github.io/leptoskit/reference/components.html) has 15 primitive builders for containers, text, images, vector drawings, scrolling, and native controls, including controlled `Switch`, `Checkbox`, `Radio`, `Slider`, progress, `PasswordInput`, and activation-only `Pressable`. Tailwind expresses layout and visual composition. The 31 optional `components::helpers` builders include `Row`, `Column`, `Stack`, `Card`, and eager `ListView`/`GridView` compositions. Web and every native backend render real controls; AppKit is the most complete, and the UIKit, Android, GTK, and Win32 backends are younger.

The builders are plain functions. The `leptoskit` crate's `view!` macro expands rstml markup such as `<View class="p-6"><Text>"Hello"</Text></View>` into the same calls, through the `IntoView` and `Fragment` types in `leptoskit_native::view`. Each builder accepts built-in Tailwind through `.class` and `.try_class`. Classes append and override typed base styles regardless of builder order. `.class` panics with a diagnostic for unsupported input. `.try_class` returns `ClassError`. Control events propose typed values; application state owns the committed value. `Element::validate` and `NativeSurface::try_set_root` reject invalid control trees before replacing the UI.

Each commit resolves utility declarations, inherits text styles, diffs the tree against the committed tree, and emits a batch of mutations. A platform `NativeBackend` applies those mutations to widgets. [Yoga](https://github.com/ifiokjr/leptoskit/tree/main/crates/leptoskit_yoga) computes layout, and the surface pushes changed frames. Events bubble from widgets through handlers.

Native backends render platform widgets without a web view. AppKit implements the macOS backend. Other platforms need their own `NativeBackend` implementation. The headless backend records mutations for tests and runtime integration.

## The pipeline

1. `NativeSurface::set_root(&element_tree)` — the app commits a description.
2. The surface resolves classes against the viewport and dark-mode value, including inherited text props.
3. The surface compares the resolved tree with the previous commit and produces a `MutationList`.
4. The backend applies widget mutations. Yoga computes frames, and changed frames become `UpdateLayout` mutations.
5. `dispatch_event` invokes handlers from the target through its ancestors until handled.

`set_viewport` and `set_dark_mode` resolve the retained source tree again. Hidden subtrees do not receive hit tests or dispatched events. Clipped overflow excludes out-of-bounds hit targets. `overflow-scroll` on a `View` resolves to `ScrollView`.

## Backends

- `HeadlessBackend` records every mutation; it powers tests, snapshot suites, and the wasm host's recording mode.
- Platform backends implement `NativeBackend` and map mutations onto the platform widget toolkit. Text measurement plugs in through `TextMeasurer` so layout matches the platform's font metrics.

## Relationship to leptos

The optional `web` feature provides `leptoskit_native::web::render(Element)`, which returns a Leptos view with semantic controls and generated CSS. Wrap the call in a reactive closure when your screen reads signals. Leptos handles browser updates, server rendering, and hydration. Native `Element` is not itself a portable `IntoView` implementation.

The `leptoskit` crate re-exports the builders, renderer types, and optional web adapter. The wasm host drives a native surface from a code-pushed bundle.

See the [shared component guide](https://ifiokjr.github.io/leptoskit/guide/07-shared-components.html) and [CSS coverage reference](https://ifiokjr.github.io/leptoskit/reference/css-coverage.html) for defaults, supported utilities, and platform limits. Text size utilities set font size only. Native fonts expose four weights, and CSS features outside the supported subset return class diagnostics.
