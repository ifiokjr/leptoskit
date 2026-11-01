---
name: leptoskit
description: Build, run, test, and release apps made with leptoskit, the Rust toolkit that renders one tree of components on the web, as native widgets on iOS, Android, macOS, Windows, and Linux, in the terminal, and as a browser extension. Use it when writing or changing components, screens, leptoskit.toml, tests, or lint configuration in a leptoskit project, or when asked to release one to an app or extension store.
---

# leptoskit

A leptoskit app is one tree of components (`View`, `Text`, `Button`, …) styled with built-in Tailwind classes. The same screens render on the web through leptos, as native widgets on iOS, Android, macOS, Windows, and Linux, in the terminal, and as a browser extension. `leptoskit.toml` at the project root says which platforms the app targets.

The `leptoskit` command (or `lk`) does the work. Run it from the project root. `leptoskit --help` and `leptoskit <command> --help` describe every flag.

| Task                        | Command                                                      |
| --------------------------- | ------------------------------------------------------------ |
| Start a project             | `leptoskit new my_app` (`--db sqlite`, `--extension`)        |
| Check the machine           | `leptoskit doctor`                                           |
| Run every platform, live    | `leptoskit dev` (`--platform web`, `ios`, `extension`, …)    |
| Build one platform          | `leptoskit build --platform <platform> [--release]`          |
| Test a browser extension    | `leptoskit test extension`                                   |
| Database migrations         | `leptoskit db migration create <name>`, `leptoskit db check` |
| Release to the stores       | `leptoskit publish [stores] [--dry-run] [--submit]`          |
| Update screens over the air | `leptoskit codepush release --platform ios`                  |

## Working rules

- Read `leptoskit.toml` first: `[app].platforms`, `[extension]`, `[publish]`, and `[db]` decide what the commands do.
- Screens are file routes under the app crate's `src/routes/`; `leptoskit` regenerates `routes.rs` from them. Don't edit `routes.rs` by hand.
- Tailwind classes are checked when a screen renders: an unsupported class panics with its name. Use the ones the project already uses, or check the CSS coverage page of the docs.
- Generated output lives in `.leptoskit/` and `dist/`; never edit it.
- Secrets never go in `leptoskit.toml`. Credentials come from the environment or a file outside the project.

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
- State that changes in place: hold it in a `RwSignal` and bind the value with `Element::text_from(|| …)` or `Element::background_from(|| …)`, so a change rewrites that node instead of rebuilding the screen. After writing a signal outside an event handler, call `leptoskit::guest::refresh()`.
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

## Debug a running app

When something looks wrong in an app running under `leptoskit dev`, read the app instead of guessing from the code. The `leptoskit mcp` server's devtools tools talk to the hub `leptoskit dev` starts:

1. `devtools_sessions` lists the running apps and their platforms.
2. `inspect_tree` (with `depth`, and `root` to look inside one node) shows each node's kind, text, classes, frame, and the source line that built it. Use the frame and the source line to find the code to change.
3. `inspect_node` and `explain_styles` give one node's details and the Tailwind classes styling it.
4. `query_cache` shows what data the screen has; `navigate` opens a route; `set_state` changes named state the app exposes to the devtools (with `leptoskit::devtools::expose`, which reports from web and native apps alike); `highlight` with `all_sessions` marks the same node on every platform.

Pass `session` (an id, `web`, `ios`, … or the app name) when more than one app is running. "No hub is running" means start `leptoskit dev` first.

## References

Read the one the task needs:

- [Releasing to the stores](references/publishing.md): `leptoskit publish`, the steps it leaves, and how to finish them in each store's dashboard with computer use.
- [Browser extensions](references/extensions.md): the extension's surfaces, building, developing, and testing it.

## More

- Linting rules and configuration: https://ifiokjr.github.io/leptoskit/guide/23-linting.html
- How markup maps to builder calls: https://ifiokjr.github.io/leptoskit/reference/view.html
- The components: https://ifiokjr.github.io/leptoskit/reference/components.html
