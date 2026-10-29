---
"release": minor
---

# Lint a project with `leptoskit lint`

`leptoskit lint` checks a project with leptoskit's own rules and `cargo clippy` at the same time, and reports both in one place with one summary. The rules read the new `[lint]` table in `leptoskit.toml`:

- `component_attribute`: a capitalized function that returns a view is a component, so it carries `#[component]`.
- `markup_style`: component trees are written the way `markup` says. The default, `"view"`, asks for markup in `view!`; `"builder"` asks for builder calls; `"any"` allows both.

```toml
[lint]
markup = "view"
exclude = ["tests/fixtures/**"]

[lint.rules]
component_attribute = "deny"
markup_style = "warn"
```

A `// leptoskit-lint: allow(markup_style)` comment on the line above a finding silences it there. `--deny-warnings` fails on warnings from both, `--no-clippy` runs the leptoskit rules alone, and anything after `--` goes to clippy. Set `clippy = false` under `[lint]` to leave clippy out by default.
