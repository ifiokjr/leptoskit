<p align="center">
  <a href="https://ifiokjr.github.io/leptoskit/"><img src="https://raw.githubusercontent.com/ifiokjr/leptoskit/main/docs/src/assets/mark.svg" alt="leptoskit" width="64"></a>
</p>

# leptoskit_macros

> The `view!` markup macro and `#[component]` attribute for leptoskit's cross-platform components.

Use them through `leptoskit::prelude`:

```rust
use leptoskit::prelude::*;

#[component]
fn Greeting(#[prop(into)] name: String, #[prop(optional)] excited: bool) -> impl IntoView {
	view! {
		<View class="gap-2 p-4">
			<Text class="text-xl font-bold">"Hello, " {name}</Text>
			{excited.then(|| view! { <Text>"Welcome back!"</Text> })}
		</View>
	}
}

let screen = view! { <Greeting name="Ada" excited /> };
```

- `view!` parses [rstml](https://github.com/rs-tml/rstml) markup and expands each tag into builder calls: `<Text class="p-4">"Hi"</Text>` becomes `Text::builder().class("p-4").children(…).build()`. The result is the same `Element` tree the builders make, so it renders on the web and on every native backend. Parsing recovers from errors, so rust-analyzer keeps completing inside half-written markup.
- `#[component]` keeps a function's signature and declares a props builder beside it, so `view!` can set props by name. `#[prop(optional)]`, `#[prop(optional_no_strip)]`, `#[prop(default = …)]`, and `#[prop(into)]` work as they do in leptos. A missing or repeated prop is a compile error that names it.

See the [`view!` and `#[component]` reference](https://ifiokjr.github.io/leptoskit/reference/view.html).
