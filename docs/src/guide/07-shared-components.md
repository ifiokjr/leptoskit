# Build shared screens with Tailwind

Build your screen from native primitives such as `View`, `Text`, `Image`, `ScrollView`, and `Pressable`. These Rust functions return the same `Element` type on native and web. Tailwind expresses layouts and visual composition. The [component catalog](../reference/components.md) lists the 15 primitives and 31 optional helpers.

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

A screen whose root is a `View` or `ScrollView` fills the window's width, as its `<div>` does in a browser, so `items-center` centres content on the screen on every platform. A button centres its label unless you align it with `text-left` or `text-right`.

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

`HeadlessBackend` lays the screen out without drawing it, which is useful in tests. A real host uses a platform backend instead, such as `AppKitBackend` on macOS or `UIKitBackend` on iOS. The apps `leptoskit new` generates do all of this for you; you only need a surface when you write your own host. Rebuild and commit the screen when your application state changes: native event callbacks update your state, and the host decides when to call `set_root` again.

## Render the same screen with Leptos

Enable the optional `web` feature:

```toml
[dependencies]
leptoskit = { version = "0.1", features = ["web"] }
leptos = "0.9.0-rc0"
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

Signals and the hooks in `leptoskit::hooks` (the `use` feature) also work in native apps on iOS, Android, macOS, Windows, and Linux. There is no reactive closure there: the app's Wasm runtime calls the view you pass to `leptoskit::guest::mount` again after every event, and the view reads each signal's current value. When state changes outside an event, such as in a plugin reply, call `leptoskit::guest::render()` afterwards. Add `reactive_graph = "0.2"` to the app's dependencies for the owner and the `Get` trait:

```rust
use leptoskit::hooks::use_toggle;
use leptoskit::prelude::*;
use reactive_graph::owner::Owner;
use reactive_graph::prelude::*;

let owner = Owner::new();
let switch = owner.with(|| use_toggle(false));
leptoskit::guest::mount(move || {
	// The view keeps the owner, and with it the hooks, alive.
	let _scope = &owner;
	let toggle = switch.toggle.clone();
	let label = if switch.value.get() { "On" } else { "Off" };
	Button(label).on_press(move || toggle())
});
```

## Update a value without rebuilding the screen

Re-rendering costs every element on the screen. A list of 1,000 cards is about 3,000 elements, so changing 100 labels that way rebuilds 3,000 to change 100. For a value that changes in place, bind the element to a signal:

```rust
use leptoskit::prelude::*;

let marked = RwSignal::new(false);

View()
	.class("flex-col gap-2 p-4")
	.child(Element::text_from(move || {
		if marked.get() {
			"Marked".to_owned()
		} else {
			"Not marked".to_owned()
		}
	}))
	.child(Button("Mark").on_press(move || marked.set(true)));
```

`Element::text_from` reads the signal once while it builds the element, so the first frame is right with no extra step, and `Element::background_from` does the same for a background colour.

The renderer then keeps the node. After writing a signal outside an event handler — in a plugin reply, a frame callback, or a scenario step — call `leptoskit::guest::refresh()` once. It reads each binding, sends only the elements whose value differs, and commits them together, so the cost follows the values that changed rather than the size of the screen.

On the web the same tree shows the value read at build time, and leptos re-runs the screen when the signal changes, so nothing else is needed there. `leptoskit::prelude` exports leptos's own `RwSignal`, so a screen shared with the browser keeps one signal type.

## Animate from app code

Some screens change on every frame: a counter that ticks, a progress value, or a list your code scrolls. A timer drifts against the display, so updates land between frames and the motion stutters. Wait for the display instead. `leptoskit::guest::next_frame` resolves once per display frame, like `requestAnimationFrame` in a browser, and whatever you render before awaiting it again reaches the screen on the next frame:

```rust
use leptoskit::guest::next_frame;

use crate::Counter;

/// Adds one to the counter on each of the next `frames` display frames.
async fn count_frames(counter: Counter, frames: u32) -> Result<(), leptoskit::guest::Error> {
	for _ in 0..frames {
		next_frame().await?;
		counter.increment();
		leptoskit::guest::render();
	}

	Ok(())
}
```

Run it on your app's executor, as you would any other async plugin call. The `Frame` it returns carries `timestamp_ns` and `period_ns`, the frame's time and the display's refresh period, for motion that should follow elapsed time rather than the number of frames. On iOS and macOS, frames come from `CADisplayLink`; on Android, from the `Choreographer`. Your render happens inside that frame's callback, so a render that fits in one frame never waits an extra one. The display link runs only while your code waits for a frame.

On Windows, Linux, and HarmonyOS, `next_frame` returns `leptoskit::guest::Error::Unavailable`, and so do apps built before leptoskit had a frame clock, which a code push update can still reach. Fall back to a timer there rather than treating it as a failure.

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

## Style the pressed state

A button that looks the same while you press it feels broken. Add `active:` utilities to a `Button` or `Pressable` to restyle it while it is held down:

```rust
use leptoskit::prelude::*;

pub fn add_button(increment: impl Fn() + Send + Sync + 'static) -> Element {
	Button("Add one")
		.class("rounded-lg bg-indigo-500 px-6 py-3 text-white active:bg-indigo-700")
		.on_press(increment)
}
```

This works on every platform except HarmonyOS. On the web, `active:` compiles to CSS `:active`, and the rendered root listens for touches so that iOS Safari applies it too. On macOS, iOS, Android, Linux, and Windows, the native control reports when the platform starts showing it as pressed, and leptoskit applies the `active:` utilities to that control until it is released. Nothing else re-renders, and the control keeps its widget, accessible name, and role. `active:` rules override responsive and dark-mode rules while they apply, and combine with them, as in `dark:active:bg-indigo-300`.

Each platform decides when a press starts and ends, the same way it does for its own highlight. On the web and macOS, a press lasts until the pointer is released, even after the pointer leaves the control. On iOS and Windows, it ends when the finger or pointer slides away and returns if it comes back. On Android, it ends when the finger slides away. Linux follows GTK's pressed state for the button. Keys show the pressed style where the platform draws one for them: while Space is held on Android and Windows and in most browsers, and briefly on Linux. On macOS and iOS, a key press or VoiceOver activates the control without showing its pressed style.

Controls without `active:` utilities still respond. Every `Pressable`, and every `Button` with its own background, border, or corner radius, dims to 70 percent of its opacity while held. A `Button` without any of those keeps the platform's own pressed highlight instead. Browser buttons have no native chrome, so they always dim. Any `active:` utility replaces the dim; to keep a control unchanged while pressed, repeat a resting value, as in `bg-indigo-500 active:bg-indigo-500`.

`active:` is only valid on `Button` and `Pressable`, the controls whose presses every platform reports. On any other element, `.class` panics and `.try_class` returns an error. Inside a `Pressable`, put the pressed styles on the `Pressable` itself. Text color is inherited, so `active:text-white` on the `Pressable` also recolors any child text that sets no color of its own. To check pressed styles in a test, `WidgetTester::hold` holds a control down and `release` lets it go; see [Testing](08-testing.md).

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

Set text input values and accessible names with the shared builders; see the [accessibility guide](15-accessibility.md) for the full ARIA vocabulary:

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
use leptoskit::components::helpers::Card;
use leptoskit::components::helpers::ListTile;
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

Use `ScrollView()` for a scroll container. `View().class("overflow-scroll")` also becomes a native scroll widget during resolution. Scroll views scroll on both axes. To react to scrolling, register a handler with `.on(NativeEventSlot::Scroll, handler)`: the web and every native backend report the scroll offset. The generated macOS, iOS, and Android apps don't report scroll events yet.

Use `ListView()` or `GridView(columns)` for eager collections that fill the remainder of a bounded column. Every item is built and laid out. For long lists, `VirtualList` builds only the items on screen; see [virtualized lists](17-virtualized-lists.md).

Add `overflow-hidden` when children must clip to a rounded container on both renderers:

```rust
let photo = View().class("size-24 rounded-lg overflow-hidden").child(
	Image("avatar.jpg").class("size-24").aria_label("Ada's avatar"),
);
```

Native platforms load images from local files only: a path or a `file://` URL, plus `resource://` names with the iOS backend and `android.resource://` URIs on Android. In the browser, the source is a URL. Give each platform a source it can load.

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
