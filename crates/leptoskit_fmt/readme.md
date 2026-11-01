<h1 align="center">leptoskit_fmt</h1>

<p align="center">
  <a href="https://ifiokjr.github.io/leptoskit/"><img src="https://raw.githubusercontent.com/ifiokjr/leptoskit/main/docs/src/assets/mark.svg" alt="leptoskit" width="64"></a>
</p>

<p align="center">
  <strong>leptoskit_fmt</strong> is the formatter behind `leptoskit format`: rustfmt for Rust and a layout engine for the markup inside `view!`.
</p>

<br>

<p align="center">
  <a href="#overview"><strong>Overview</strong></a> ·
  <a href="#how-it-fits"><strong>How it fits</strong></a> ·
  <a href="#use-it"><strong>Use it</strong></a> ·
  <a href="#learn-more"><strong>Learn more</strong></a> ·
  <a href="https://docs.rs/leptoskit_fmt"><strong>Docs</strong></a> ·
  <a href="https://github.com/ifiokjr/leptoskit"><strong>Repository</strong></a>
</p>

<br>

<p align="center">
  <a href="https://crates.io/crates/leptoskit_fmt"><img alt="crates.io" src="https://img.shields.io/crates/v/leptoskit_fmt.svg"></a>
  <a href="https://docs.rs/leptoskit_fmt"><img alt="docs.rs" src="https://docs.rs/leptoskit_fmt/badge.svg"></a>
  <a href="https://github.com/ifiokjr/leptoskit/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/ifiokjr/leptoskit/actions/workflows/ci.yml/badge.svg"></a>
  <a href="https://github.com/ifiokjr/leptoskit/blob/main/Cargo.toml"><img alt="MSRV" src="https://img.shields.io/badge/MSRV-1.95.0-blue.svg"></a>
  <a href="https://opensource.org/license/unlicense"><img alt="License: Unlicense" src="https://img.shields.io/badge/license-Unlicense-blue.svg"></a>
</p>

<br>

## Overview

rustfmt leaves `view!` markup alone. This crate parses that markup with [rstml](https://crates.io/crates/rstml), lays it out at the width and indentation in the project's `rustfmt.toml`, and sends every Rust expression inside it back through rustfmt at the column it sits in, all in a single rustfmt run per file.

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

## How it fits

rustfmt leaves `view!` markup alone, so this fills the gap without a second Rust formatter. It runs in editor integrations, pre-commit, and CI, and agrees with the lint rules.

## Use it

```sh
leptoskit format            # format every Rust file under the current directory
leptoskit format --check    # print a diff and fail if anything would change
leptoskit format --stdin --stdin-filepath src/app.rs < src/app.rs
```

## Learn more

- [Crate documentation](https://docs.rs/leptoskit_fmt)
- [leptoskit documentation](https://ifiokjr.github.io/leptoskit/)
- [Repository](https://github.com/ifiokjr/leptoskit)
- [Formatting](https://ifiokjr.github.io/leptoskit/guide/12-formatting.html)
