# Build shared screens with Tailwind

Build your screen from native primitives such as `View`, `Text`, `Image`, `ScrollView`, and `Pressable`. These Rust functions return the same `Element` type on native and web. Tailwind expresses layouts and visual composition. The [component catalog](../reference/components.md) lists the 14 primitives and 31 optional helpers.

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

The built-in components are builder functions. Call `View()` and `Text("Hello")` directly. Your shared screen does not need HTML elements.

## Write the screen as markup

`leptoskit::prelude` also exports a `view!` macro and a `#[component]` attribute. `view!` parses [rstml](https://github.com/rs-tml/rstml) markup, the syntax leptos uses, and expands it into the same builder calls. The markup renders on the web and on every native backend:

```rust
use leptoskit::prelude::*;

#[component]
pub fn Counter(count: usize, on_increment: impl Fn() + Send + Sync + 'static) -> impl IntoView {
	view! {
		<View class="w-full gap-4 p-6 bg-slate-100 dark:bg-slate-950">
			<Text class="text-3xl font-bold text-indigo-500">"Count: " {count}</Text>
			<Button class="rounded-lg bg-indigo-500 px-4 py-2 text-white" on:press=on_increment>
				"Increment"
			</Button>
		</View>
	}
}

let screen = view! { <Counter count=0 on_increment=|| println!("increment") /> };
```

Each tag calls a component, each attribute calls the builder method of the same name, and `on:press` calls `on_press`. `#[component]` keeps the function's signature, so `Counter(0, handler)` still works in builder code. The [`view!` and `#[component]` reference](../reference/view.md) covers props, children, fragments, and errors.

Flutter-inspired compositions remain optional. Import them from `leptoskit::components::helpers` when useful; the default prelude keeps their functions out of your primary component vocabulary. A row can be `View().class("flex-row")`, and layered content can use a relative `View` with absolute children.

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

## Clear the status bar and home indicator

Phones draw system UI over the edges of the screen. Padding utilities with a `safe` value pad by the inset the host reports for each edge, and `safe-offset-*` adds a spacing value to that inset. The names follow the `tailwindcss-safe-area` plugin:

```rust
let screen = ScrollView().class("w-full h-full bg-white").child(
	View().class("min-h-full px-6 pt-safe-offset-4 pb-safe-offset-4 gap-6"),
);
```

`p`, `px`, `py`, `pt`, `pr`, `pb`, and `pl` accept both forms. `UIKitHost` reads the controller view's `safeAreaInsets`. Other native hosts call `NativeSurface::set_safe_area` with the edges covered by system UI, in logical pixels; on Android, those are the window insets for system bars and display cutouts. Desktop windows report no insets, so the padding is just the offset. In a browser, the utilities emit `env(safe-area-inset-*)`, which needs `viewport-fit=cover` in the page's viewport meta tag.

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

Set text input values and accessible names with the shared builders; see the [accessibility guide](12-accessibility.md) for the full ARIA vocabulary:

```rust
let form = View().class("gap-3 p-4").children([
	TextInput().value("Ada").placeholder("Your name").aria_label("Name").class("border border-slate-300 rounded-md p-2"),
	Button("Save").enabled(true).on_press(|| println!("save")),
]);
```

Register text changes with `.on_change(handler)`. The handler receives the proposed `&str`. `PasswordInput()` has the same single-line API with masked display.

An unstyled text input keeps the platform's text-field border, and its height leaves room for it: 28 units at the default font size. Once it has its own background, border, or corner radius, as above, it is as tall as its text plus its padding. A height class such as `h-10` always wins.

Use typed handlers for boolean and numeric controls. Keep the accepted value in your application state, then rebuild the screen after a change:

```rust
use leptoskit::prelude::*;

pub fn preferences(checked: bool, update: impl Fn(bool) + Send + Sync + 'static) -> Element {
	Card().class("gap-4").children([
		Text("Preferences").class("text-xl font-semibold"),
		ListTile(
			None,
			Text("Notifications"),
			Some(
				Switch(checked)
					.aria_label("Notifications")
					.on_toggle(update),
			),
		),
	])
}
```

`on_toggle` receives a proposed boolean. `on_value_change` receives a proposed slider value. Both renderers restore the committed value if the application rejects the proposal. Related radios share a validated `RadioGroup` and application-owned selection.

Use `Pressable("Accessible name")` for custom decorative button content. Register its action with `on_press`, then add text, images, and layout containers as children. Nested controls, scrolling descendants, and descendant event handlers are invalid. `Pressable` currently provides activation rather than React Native's full press lifecycle.

Use `ScrollView()` for a scroll container. `View().class("overflow-scroll")` also becomes a native scroll widget during resolution. AppKit supports both scroll axes. Native scroll event delivery is not implemented.

Use `ListView()` or `GridView(columns)` for eager collections that fill the remainder of a bounded column. Every item is built and laid out. They do not provide virtualization.

Add `overflow-hidden` when children must clip to a rounded container on both renderers:

```rust
let photo = View().class("size-24 rounded-lg overflow-hidden").child(
	Image("avatar.jpg").class("size-24").aria_label("Ada's avatar"),
);
```

AppKit image sources are local paths or `file://` URLs. A web image source is a browser URL. Supply an asset source that your host can resolve.

For supported utility families and their limits, use the [CSS coverage reference](../reference/css-coverage.md). For a complete shared-screen example, see [code reuse across platforms](../platforms/code-reuse.md).

## Persist shared application state

Enable the `storage` feature to use `leptoskit::hooks::use_async_storage` with persistent preferences. The browser host opens a `LocalStorage<T>` namespace. A native host opens an application-private data directory. Both implement the same `AsyncStorage<T>` contract and return the same reactive handle for shared screen code.

```rust
let initialize = owner.with(|| {
	leptoskit::hooks::use_async_storage(storage, "name", String::from("Guest"))
});
let preferences = initialize.await?;
```

Read `preferences.value` when constructing the shared tree. A button callback passes `(preferences.set_value)(new_name)` to the host's local executor. The hook updates the displayed value after persistence succeeds and returns errors without replacing the last successful value. Native file operations run on worker threads. Browser local storage completes immediately; custom async adapters can use IndexedDB or a host service.

The owner scopes the model across awaits. Saves, reloads, and removal serialize per handle. Disposing the owner while an operation is pending returns a typed disposal error. The host also supplies viewport dimensions; use `Viewport<f32>` to retain native Tailwind's fractional logical pixels.
