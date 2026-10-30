# counter

The whole leptoskit stack in one binary, and the example written with **builder calls** instead of `view!` markup.

- **The interface** (`src/main.rs`): a column with a count and a button, built as `View().class(…).child(Text(…))`. Builders are the API `view!` expands to, so this is the same tree markup would build, written by hand.
- **The renderer**: a headless `NativeSurface` lays the tree out with yoga, diffs each rebuild into native mutations, and dispatches a press to the button the way a platform shell does.
- **Code push**: the example packs its own source into a signed release, as an app ships an update over the air.

```bash
cargo run -p counter_example
```

Every other example writes markup, which `leptoskit lint` asks for by default. This one's `leptoskit.toml` relaxes the rule:

```toml
[lint]
markup = "any" # builder calls and `view!` side by side; "builder" asks for builders everywhere
```

See the [linting guide](https://ifiokjr.github.io/leptoskit/guide/23-linting.html) for the rules and the [`view!` reference](https://ifiokjr.github.io/leptoskit/reference/view.html) for how markup maps to builder calls.
