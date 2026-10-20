# Build shared screens with Tailwind

Build your screen from `View`, `Text`, `Button`, `Image`, `ScrollView`, and `TextInput`. These Rust functions return the same `Element` type on native and web.

```rust
use leptoskit::prelude::*;

pub fn counter(count: usize, increment: impl Fn() + Send + Sync + 'static) -> Element {
	View()
		.class("w-full gap-4 p-6 bg-slate-100 dark:bg-slate-950")
		.child(Text(format!("Count: {count}")).class("text-3xl font-bold text-indigo-500"))
		.child(
			Button("Increment")
				.class("rounded-lg bg-indigo-500 px-4 py-2 text-white")
				.on_press(increment),
		)
}
```

The built-in components are builder functions. Call `View()` and `Text("Hello")` directly. They are not Leptos `#[component]` functions and do not use `<View>` tags. Your shared screen does not need HTML elements.

Tailwind utilities are built into leptoskit. You do not install Tailwind, scan source files, or run a Node build. The compiler parses classes into typed declarations. Native rendering applies those declarations to Yoga and widget props. The optional web renderer emits CSS from the same declarations.

## Commit the screen on native

Create a surface and set its viewport before the first commit:

```rust
use leptoskit::native::HeadlessBackend;
use leptoskit::prelude::*;

let mut surface = NativeSurface::new(HeadlessBackend::new());
surface.set_viewport(390.0, 844.0);
surface.set_root(&counter(0, || println!("increment")));
```

Use `AppKitBackend` in a macOS host. Rebuild and commit the screen when your application state changes. Native event callbacks update your state, and the host decides when to call `set_root` again.

## Render the same screen with Leptos

Enable the optional `web` feature:

```toml
[dependencies]
leptoskit = { version = "0.1", features = ["web"] }
leptos = "0.8"
```

Select Leptos's `csr`, `hydrate`, or `ssr` feature for your browser or server target. Wrap `web::render` in a reactive closure when the screen reads signals:

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

`counter` is the same function used by the native surface. The web renderer creates semantic DOM controls and attaches the existing event handlers. Leptos owns DOM updates, server rendering, and hydration.

## Combine typed styles and classes

Use typed styles for values that come from application data. Classes override the typed base for properties they set, regardless of builder call order:

```rust
use leptoskit::prelude::*;

let card = View()
	.class("w-20 p-4")
	.with_style(|style| style.width = Dimension::Points(200.0))
	.class("rounded-lg bg-white");
```

The rendered width is 80 logical pixels. Each `.class(...)` call appends utilities. Duplicate tokens are removed. Class string order does not determine precedence. Broad spacing utilities precede axes and individual edges, responsive rules override base rules, and important utilities win. Avoid competing utilities for the same property and variant. Equal-rank conflicts use deterministic lexical order, which is not an upstream Tailwind compatibility guarantee.

Use a trailing `!` for an important utility, such as `p-4!`. The older leading form, `!p-4`, is also accepted.

Keep the source tree when you need responsive behavior. `NativeSurface` retains the source internally and resolves it again after viewport or appearance changes. `Element::resolved` returns a snapshot with its classes removed and is intended for renderer integration.

## Update responsive styles and dark mode

Use the default `sm`, `md`, `lg`, `xl`, and `2xl` variants. `max-*` variants select widths below a breakpoint. Variants can be combined into a range:

```rust
let panel = View().class("p-4 md:max-xl:p-8 xl:flex-row dark:bg-slate-950");

surface.set_root(&panel);
surface.set_viewport(900.0, 600.0);
surface.set_dark_mode(true);
```

On native, the host supplies the viewport in logical pixels and calls `set_dark_mode` when appearance changes. The browser uses generated width media queries and `prefers-color-scheme`. Browser resizing does not require rebuilding the screen.

The [CSS coverage reference](../reference/css-coverage.md#defaults-and-units) lists breakpoint values, units, and component defaults.

## Validate dynamic classes

Use `.class(...)` for class strings you control. Invalid or unsupported tokens cause a panic with the class and its reason. Use `.try_class(...)` for dynamic input:

```rust
use leptoskit::prelude::*;

pub fn panel(classes: &str) -> Result<Element, ClassError> {
	View().try_class(classes)
}

let error = panel("md:hover:bg-red-500").expect_err("hover styles are unsupported");
assert_eq!(error.class, "md:hover:bg-red-500");
```

The compiler validates inactive variants too. An unsupported `2xl:shadow-lg` returns an error even in a narrow viewport. It rejects unknown utilities, invalid values, unsupported variants, and empty breakpoint ranges.

## Use controls and scrolling

Set text input values and accessibility labels with the shared builders:

```rust
let form = View().class("gap-3 p-4").children([
	TextInput().value("Ada").placeholder("Your name").accessibility_label("Name").class("border border-slate-300 rounded-md p-2"),
	Button("Save").enabled(true).on_press(|| println!("save")),
]);
```

Register text changes with `.on(NativeEventSlot::Change, handler)`. The handler receives `NativeEvent::Change { value }` and returns an `EventOutcome`.

Use `ScrollView()` for a scroll container. `View().class("overflow-scroll")` also becomes a native scroll widget during resolution. AppKit supports both scroll axes. Native scroll event delivery is not implemented.

Add `overflow-hidden` when children must clip to a rounded container on both renderers:

```rust
let photo = View().class("size-24 rounded-lg overflow-hidden").child(
	Image("avatar.jpg").class("size-24").accessibility_label("Ada's avatar"),
);
```

AppKit image sources are local paths or `file://` URLs. A web image source is a browser URL. Supply an asset source that your host can resolve.

For supported utility families and their limits, use the [CSS coverage reference](../reference/css-coverage.md). For a complete shared-screen example, see [code reuse across platforms](../platforms/code-reuse.md).
