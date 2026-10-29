<p align="center">
  <a href="https://ifiokjr.github.io/leptoskit/"><img src="https://raw.githubusercontent.com/ifiokjr/leptoskit/main/docs/src/assets/mark.svg" alt="leptoskit" width="64"></a>
</p>

# leptoskit_lint

> The project rules behind `leptoskit lint`: components carry `#[component]`, and component trees are written the way `leptoskit.toml` says.

rustc and clippy check Rust. This crate checks the conventions a leptoskit project adds on top of it. It parses each file with [syn](https://crates.io/crates/syn) and builds nothing, so a whole project lints in milliseconds. That speed is why `leptoskit lint` can run these rules alongside clippy and report both together.

| Rule                  | What it asks for                                                                                                  |
| --------------------- | ----------------------------------------------------------------------------------------------------------------- |
| `component_attribute` | A capitalized function that returns a view (`impl IntoView`, `Element`) is marked `#[component]`.                 |
| `markup_style`        | Component trees are written the way `[lint] markup` says: `view!` markup by default, or builder calls.            |
| `redundant_view`      | A `View` with nothing but one child is not a wrapper for its own sake (Flutter's `avoid_unnecessary_containers`). |
| `spacer_view`         | Space comes from `gap-*` or `Spacer()`, not an empty sized `View` (Flutter's `sized_box_for_whitespace`).         |
| `children_last`       | Builder trees set properties before children (Flutter's `sort_child_properties_last`).                            |
| `image_label`         | Every `Image` has an accessible name or is marked decorative.                                                     |
| `color_literal`       | Colors come from the theme, not hand-written values; off by default (React Native's `no-color-literals`).         |

```sh
leptoskit lint                  # these rules and clippy, side by side
leptoskit lint --no-clippy      # these rules alone
cargo run -p leptoskit_lint --features cli --bin leptoskit-lint -- examples   # without the CLI
```

Each file follows the `[lint]` table of the nearest `leptoskit.toml`:

```toml
[lint]
markup = "view" # "builder" or "any"
exclude = ["tests/fixtures/**"]

[lint.rules]
component_attribute = "deny" # "allow", "warn", or "deny"
markup_style = "warn"
```

A `// leptoskit-lint: allow(markup_style)` comment on the line above a finding silences it there.

As a library:

```rust
let report = leptoskit_lint::lint_paths(&["src".into()])?;
for diagnostic in &report.diagnostics {
	eprint!("{}", leptoskit_lint::render_diagnostic(diagnostic, ".".as_ref()));
}
# Ok::<(), leptoskit_lint::LintError>(())
```

The rules know the built-in components that have a `view!` tag (`View`, `Text`, `Button`, …) and every function marked `#[component]` in the files they check. Only calls to those count as builder trees. Helpers that take elements as arguments, such as `ListTile`, have no tag and stay builder calls inside markup, and a capitalized tuple-struct constructor such as `Meters(3.0)` is never flagged.

See the [linting guide](https://ifiokjr.github.io/leptoskit/guide/23-linting.html).
