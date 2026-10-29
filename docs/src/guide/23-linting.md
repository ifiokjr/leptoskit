# Linting

clippy checks your Rust, but it knows nothing about how a leptoskit project is put together: which functions are components, or whether a screen should be written as markup or as builder calls. `leptoskit lint` runs leptoskit's own rules and clippy at the same time, and reports both in one place.

```bash
leptoskit lint                    # the leptoskit rules and clippy, over the whole project
leptoskit lint src/screens        # the leptoskit rules over part of it; clippy still checks the workspace
leptoskit lint --deny-warnings    # fail on warnings too, for CI
leptoskit lint --no-clippy        # only the leptoskit rules, in milliseconds
leptoskit lint -- -W clippy::nursery   # anything after `--` goes to clippy
```

`lk lint` is the same command. Both checks run in parallel: the leptoskit rules read your source without building it, so they are done long before clippy, and every finding prints in rustc's layout once clippy finishes:

```text
warning[leptoskit::markup_style]: `View(…)` builds this tree with builder calls
  --> src/screen.rs:12:2
   |
12 |     View()
   |     ^^^^ this project writes component trees as markup
   |
   = help: write it with `view!`, as `view! { <View …>…</View> }`, or set `markup = "any"` under `[lint]` in leptoskit.toml

  ▲ leptoskit  14 files · 1 warning  21ms
  ✔ clippy     no problems           6.4s
```

The run fails when either check finds an error, or a warning with `--deny-warnings`.

## The rules

### `component_attribute`

A capitalized function that returns a view, an `impl IntoView` or an `Element`, is a component, so it carries `#[component]`:

```rust,ignore
// warning: `Greeting` returns a view but is not marked `#[component]`
fn Greeting(name: String) -> impl IntoView {
	view! { <Text>"Hello, " {name}</Text> }
}

#[component]
fn Greeting(#[prop(into)] name: String) -> impl IntoView {
	view! { <Text>"Hello, " {name}</Text> }
}
```

With the attribute, `view!` can call the component by name, `<Greeting name="Ada" />`, and rustc stops asking for a snake case function name. leptos's `#[component]` and `#[island]` count too.

### `markup_style`

Every component tree in a project is written one way. By default that is markup in `view!`:

```rust,ignore
view! {
	<View class="gap-2 p-4">
		<Text class="text-xl font-bold">"Hello"</Text>
		<Button on:press=move |_| count.update(|count| *count += 1)>"Add one"</Button>
	</View>
}
```

and a tree built with the component functions instead is reported once, at its root. Helpers that take elements as arguments and have no tag, such as `ListTile` and `Expanded`, stay builder calls inside the markup and are never reported.

```rust,ignore
// warning: `View(…)` builds this tree with builder calls
View()
	.class("gap-2 p-4")
	.child(Text("Hello").class("text-xl font-bold"))
```

Set `markup = "builder"` to ask for builder calls everywhere instead, and report `view!`; or `markup = "any"` to allow both.

## Configuration

The rules follow the `[lint]` table of the nearest `leptoskit.toml`, and every key is optional:

```toml
[lint]
# How component trees are written: "view" (the default), "builder", or "any".
markup = "view"
# Run clippy beside the leptoskit rules. `--no-clippy` turns it off for one run.
clippy = true
# Paths the leptoskit rules skip, relative to this file.
exclude = ["tests/fixtures/**"]

[lint.rules]
# Each rule is "allow", "warn" (the default), or "deny".
component_attribute = "deny"
markup_style = "warn"
```

To allow one finding, put a comment on the line above it:

```rust,ignore
// leptoskit-lint: allow(markup_style)
let row = View().class("flex-row").child(icon);
```

## Projects written by coding agents

Markup is for people: it reads like the interface it builds, which is why these docs and leptoskit's examples use it. A coding agent is better served by builder calls. The compiler reports errors on the exact call, there is no macro between the code and the diagnostic, and trees build faster without a proc macro to expand them.

So when an agent writes a project, make builders the rule and markup an error:

```toml
[lint]
markup = "builder"

[lint.rules]
markup_style = "deny"
```

leptoskit ships an agent skill that says the same and walks through the builder API. Install it with `gh skill install ifiokjr/leptoskit leptoskit`, or copy `skills/leptoskit` from the repository into your agent's skills directory.
