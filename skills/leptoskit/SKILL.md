---
name: leptoskit
description: Build leptoskit apps (one Rust interface rendered on the web and as native widgets on macOS, iOS, Android, Linux, and Windows). Use when writing or changing components, screens, leptoskit.toml, tests, or lint configuration in a leptoskit project, or when the user mentions leptoskit, `leptoskit lint`, `view!`, or the builder API.
---

# Building with leptoskit

leptoskit renders one tree of components (`View`, `Text`, `Button`, …) on the web through leptos and as real native widgets on every desktop and mobile platform. A tree can be written two ways that build exactly the same `Element`:

```rust
// Builder calls: plain Rust.
View()
	.class("gap-2 p-4")
	.child(Text("Hello").class("text-xl font-bold"))
	.child(Button("Add one").on_press(move || count.update(|count| *count += 1)))

// Markup: the `view!` macro expands to the calls above.
view! {
	<View class="gap-2 p-4">
		<Text class="text-xl font-bold">"Hello"</Text>
		<Button on:press=move || count.update(|count| *count += 1)>"Add one"</Button>
	</View>
}
```

## Agents write builder calls

When you write leptoskit code, write builder calls, not `view!` markup:

- **The compiler sees them directly.** Errors point at the call that is wrong, rust-analyzer completes every method, and there is no macro expansion between your code and the diagnostic.
- **They build faster.** There is no proc macro to run over every tree.
- **They are just Rust.** Loops, conditionals, and helper functions compose the way any Rust does: `.children(items.iter().map(row))`, `.children(error.map(Alert))`.

`view!` exists for people reading code, which is why leptoskit's own examples and docs use it. Apps you build or change as an agent use builders.

### Enforce it in the projects you work on

`leptoskit lint` checks the markup style from `leptoskit.toml`. In a project you create, or one whose owner asks you to work this way, make builders the rule and a `view!` an error:

```toml
[lint]
markup = "builder"

[lint.rules]
markup_style = "deny"
component_attribute = "deny"
```

Then rewrite any `view!` the lint reports, including the starter screen `leptoskit new` generates, and keep `leptoskit lint --deny-warnings` passing. Don't change an existing project's `markup` without the owner's say-so: a project that chose `"view"` or `"any"` keeps it.

## Components

A component is a capitalized function that returns a view, marked `#[component]`. The attribute keeps the function callable positionally from builder code and gives it named props for `view!`, and `leptoskit lint` asks for it (`component_attribute`):

```rust
use leptoskit::prelude::*;

#[component]
pub fn SettingRow(#[prop(into)] title: String, children: Children) -> impl IntoView {
	View()
		.class("flex-row items-center gap-3 px-4 h-12")
		.child(Text(title).class("flex-1 text-base"))
		.children(children)
}

let row = SettingRow("Wi-Fi".into(), Fragment::new().with(Switch(true)));
```

- `View`, `ScrollView`, `Row`, `Column`, `Card`, … take no arguments; `Text(content)`, `Button(label)`, `Image(source)`, `Switch(checked)`, `Status(text)`, and `Alert(text)` take their content.
- Style with Tailwind through `.class("…")`; it compiles to native layout and styles on every platform.
- Handlers: `.on_press(|| …)`, `.on_change(|text: &str| …)`, `.on_toggle(|checked| …)`, `.on_value_change(|value| …)`.
- Accessibility: `.accessibility_label("…")`, `.role(Role::…)`. Give anything a test needs to find a stable `.test_id("…")`.

## Check your work

```bash
leptoskit lint --deny-warnings    # leptoskit's rules and clippy, side by side
leptoskit format                  # rustfmt, plus layout for any markup
leptoskit test run                # component and device tests
```

Test components with the shared Testing Library queries. `WidgetTester` renders headlessly with the real layout engine; `leptoskit_testing_library::render` renders in a real browser. Both implement `Queries`:

```rust
use leptoskit_testing::Finder;
use leptoskit_testing::Queries;
use leptoskit_testing::WidgetTester;

let mut tester = WidgetTester::new(counter, 390.0, 844.0)?;
tester.tap(&Finder::text("Add one"))?;
tester.get_by_text("Count: 1")?;
assert!(tester.query_by_text("Count: 0")?.is_none());
```

## More

- Linting rules and configuration: https://ifiokjr.github.io/leptoskit/guide/23-linting.html
- How markup maps to builder calls: https://ifiokjr.github.io/leptoskit/reference/view.html
- The components: https://ifiokjr.github.io/leptoskit/reference/components.html
