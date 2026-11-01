# `view!` and `#[component]`

`leptoskit::prelude` exports two macros for writing shared screens as markup:

- `view!` parses [rstml](https://github.com/rs-tml/rstml) markup and expands it into builder calls. A view written with it renders on the web and on the iOS, Android, macOS, Windows, and GTK backends, because it produces the same `Element` tree as the builders.
- `#[component]` turns a function into a component that `view!` calls with named props. The function keeps its signature, so builder code can still call it positionally.

```rust
use leptoskit::prelude::*;

/// A settings row with a trailing control.
#[component]
pub fn SettingRow(
	/// The row's label.
	#[prop(into)]
	title: String,
	/// Shown under the title when set.
	#[prop(optional, into)]
	detail: Option<String>,
	children: Children,
) -> impl IntoView {
	view! {
		<View class="flex-row gap-3 items-center px-4 h-12">
			<View class="flex-1">
				<Text class="text-base">{title}</Text>
				{detail.map(
					|detail| view! { <Text class="text-sm text-slate-500">{detail}</Text> },
				)}
			</View>
			{children}
		</View>
	}
}

#[component]
pub fn Settings(wifi: bool, toggle: impl Fn(bool) + Send + Sync + 'static) -> Element {
	view! {
		<ScrollView class="flex-1">
			<SettingRow title="Wi-Fi" detail="Home 5G">
				<Switch checked=wifi accessibility_label="Wi-Fi" on:toggle=toggle />
			</SettingRow>
		</ScrollView>
	}
}
```

## How markup expands

Every tag is a component, and every component has a type with the same name as its function. Built-in components such as `View` and `Switch` declare one, and so does `#[component]`. A tag calls that type's `builder()`, then one method per attribute, then `build()`:

```rust
// view! { <Button class="px-4" enabled=false on:press=save>"Save"</Button>
// }
Button::builder()
	.class("px-4")
	.enabled(false)
	.on_press(save)
	.children(Fragment::new().with("Save"))
	.build()
```

Types and functions live in separate namespaces in Rust, so `use leptoskit::prelude::Button` imports both. Each method call carries the span of the attribute it came from. A wrong attribute name or value type is reported on that attribute, and rust-analyzer completes attribute names and expressions inside the markup.

| Markup                       | Meaning                                                                                                       |
| ---------------------------- | ------------------------------------------------------------------------------------------------------------- |
| `<View>…</View>`, `<View />` | Calls the component `View`. A tag can be a path, such as `<helpers::Card>`.                                   |
| `class="p-4"`                | Calls `.class("p-4")`. Tailwind classes work as they do with the builders. Repeat `class` to append more.     |
| `name=value`, `name={value}` | Calls `.name(value)`.                                                                                         |
| `name`                       | Calls `.name(true)`, as in `<Switch checked />`.                                                              |
| `border=(1.0, color)`        | A tuple spreads into several arguments: `.border(1.0, color)`. Write `{(a, b)}` to pass a tuple as one value. |
| `on:press=handler`           | Calls `.on_press(handler)`. Any `on:event` calls `on_event`, such as `on:toggle` and `on:value_change`.       |
| `"text"`                     | A text child.                                                                                                 |
| `{expression}`               | A child: an element, a string or number, an `Option`, a `Vec`, a `Fragment`, or a closure.                    |
| `<>…</>`                     | A `Fragment`: siblings without a parent.                                                                      |

`view!` with one root tag returns an `Element`. Markup with several roots, or a `<>…</>` root, returns a `Fragment`. Collect an iterator of views with `.collect::<Fragment>()`.

### Built-in components

The built-ins take their values as attributes that name `Element` builder methods:

| Tag                                                   | Attributes in place of builder arguments |
| ----------------------------------------------------- | ---------------------------------------- |
| `<Text>"Hello " {name}</Text>`                        | Text children become the label.          |
| `<Button>"Save"</Button>`                             | Text children become the label.          |
| `<Image source=url />`                                | `source`                                 |
| `<Switch checked=on />`                               | `checked`; also `<Checkbox>`             |
| `<Radio radio_group=group checked />`                 | `radio_group`, `checked`                 |
| `<Slider slider=state />`                             | `slider`                                 |
| `<LinearProgressIndicator progress=p />`              | `progress`                               |
| `<ActivityIndicator running />`                       | `running`                                |
| `<Pressable accessibility_label="Open">…</Pressable>` | `accessibility_label` names the action.  |

`View`, `ScrollView`, `TextInput`, and `PasswordInput` take no arguments. The helpers `Row`, `Column`, `Wrap`, `Center`, `Spacer`, `ClipRect`, `Divider`, `VerticalDivider`, `Card`, and `ListView` are tags too. Helpers that take elements as arguments, such as `ListTile`, stay builder calls: `{ListTile(None, title, Some(control))}`.

A `Text` or `Button` treats plain text children as its content, so `<Text>"Count: " {count}</Text>` is `Text("Count: 3")`. The same holds for `.child(...)` and `.children(...)` in builder code.

## Components and props

Each parameter of a `#[component]` function is a prop. `view!` sets props by name, in any order:

| Parameter                                           | In `view!`                                                                  |
| --------------------------------------------------- | --------------------------------------------------------------------------- |
| `title: String`                                     | Required: `title=…`. Leaving it out is a compile error that names the prop. |
| `#[prop(into)] title: String`                       | Accepts anything that converts into the type, such as `title="Wi-Fi"`.      |
| `#[prop(optional)] detail: Option<String>`          | May be left out, which gives `None`. When set, it takes a plain `String`.   |
| `#[prop(optional)] dense: bool`                     | May be left out, which gives `Default::default()`. `<Row dense />` sets it. |
| `#[prop(optional_no_strip)] icon: Option<Element>`  | May be left out, and takes the `Option` itself when set.                    |
| `#[prop(default = 4)] gap: u8`                      | May be left out, which gives the default expression.                        |
| `children: Children`                                | The tag's children, as a `Fragment`.                                        |
| `on_select: impl Fn(&Item) + Send + Sync + 'static` | A closure prop. Generic parameters and `impl Trait` work.                   |

Use PascalCase names and `#[component]` for reusable UI, even when a caller could invoke a plain function directly. This keeps named props and markup available as the UI grows.

Plain functions still suit host entry points with fixed signatures, methods that render an existing model, fallible factories returning `Result`, and helpers that transform or collect elements. Generated sign-in screens and the auth-page example keep the generator's output so they can validate it without hand edits. The counter example deliberately uses builders to show the underlying API. Builder snippets in API references and lint diagnostics demonstrate that API or a reported violation; they are not the recommended screen style. Some composition APIs, such as `ListTile`, `VirtualList`, and the accessible widget parts, expose functions or methods rather than tags; embed their results in `{…}`.

Doc comments on parameters are listed under **Props** in the component's documentation. A component's name starts with a capital letter. It returns `impl IntoView`, an `Element`, or another `IntoView` type, and the macro rewrites `impl IntoView` to `impl leptoskit::IntoView`.

Setting a prop twice, or leaving out a required prop, fails to compile:

```text
error[E0277]: `<Greeting>` is missing the required prop `name`
  --> src/screen.rs:13:10
   |
13 |     let _ = view! { <Greeting excited /> };
   |                      -------- set it with `name=…`
```

## Reactivity

Screens are rebuilt whenever their state changes. On native, the host calls the screen function again and commits the new tree. On the web, wrap `leptoskit::web::render(view! { … })` in a closure that reads your signals. A closure child, such as `{move || count.get()}`, is called each time the view is built, so a signal read inside it is tracked by the closure that rebuilds the screen. Closures keep the `Fn() + Send + Sync + 'static` bound of a reactive region, so they will keep working when regions update on their own.

## leptos's `view!` or leptoskit's

Use leptoskit's `view!` and `#[component]` for screens that render natively: everything built from `View`, `Text`, and the other components. Use leptos's `view!` for web-only HTML, such as `<div>`, `<a href>`, and islands. A lowercase tag in leptoskit's `view!` is a compile error that points you to leptos.

Both preludes export `view` and `component`. A file that glob-imports both fails to compile only where it uses an ambiguous name. Import the one you mean explicitly, which takes precedence over globs:

```rust
use leptos::prelude::*;
use leptoskit::component;
use leptoskit::prelude::*;
use leptoskit::view; // markup in this file is leptoskit markup
```

A leptoskit element is a child of leptos markup through `leptoskit::web::render`:

```rust
leptos::view! { <main>{leptoskit::web::render(view! { <Text>"Shared"</Text> })}</main> }
```
