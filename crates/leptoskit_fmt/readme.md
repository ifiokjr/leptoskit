<p align="center">
  <a href="https://ifiokjr.github.io/leptoskit/"><img src="https://raw.githubusercontent.com/ifiokjr/leptoskit/main/docs/src/assets/mark.svg" alt="leptoskit" width="64"></a>
</p>

# leptoskit_fmt

> The formatter behind `leptoskit format`: rustfmt for Rust, and a layout engine for the markup inside `view!` macros.

rustfmt leaves `view!` markup alone. This crate parses that markup with [rstml](https://crates.io/crates/rstml), lays it out at the width and indentation in the project's `rustfmt.toml`, and sends every Rust expression inside it back through rustfmt at the column it sits in, all in a single rustfmt run per file.

```sh
leptoskit format            # format every Rust file under the current directory
leptoskit format --check    # print a diff and fail if anything would change
leptoskit format --stdin --stdin-filepath src/app.rs < src/app.rs
```

```rust
view! {
	<button on:click=move |_| set_count.update(|count| *count += 1)>"Clicks: "{count}</button>
	<For each=move || todos.get() key=|todo| todo.id let:todo>
		<TodoItem todo=todo />
	</For>
}
```

- **rustfmt does the Rust.** Nightly rustfmt by default, so unstable `rustfmt.toml` options apply; `--stable` or `rustfmt_toolchain = "stable"` switches. Width, tabs, and indentation come from the same `rustfmt.toml`.
- **Idempotent and safe.** Text, string literals, and unquoted text are copied byte for byte. Braces are removed from attribute values only where the parser reads them the same without. Comments stay put. Markup that does not parse is left exactly as written, with a warning.
- **Fast.** One rustfmt run per file however much markup it holds, files formatted in parallel, and files a previous run left formatted skipped by content hash.

Settings live in the `[format]` table of `leptoskit.toml`:

```toml
[format]
macros = ["view", "leptos::view", "leptoskit::view"]
attribute_braces = "when_required" # "always", "always_unless_literal", or "preserve"
closing_tags = "self_closing" # "explicit" or "preserve"
rustfmt_toolchain = "nightly"
```

See the [formatter guide](https://ifiokjr.github.io/leptoskit/guide/12-formatting.html) for the full style and the benchmarks.
