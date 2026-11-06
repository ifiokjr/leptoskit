---
leptoskit_fmt: major
---

# leptoskit_fmt: the view! formatter

`leptoskit_fmt` is the formatter behind `leptoskit format`: rustfmt for Rust and a layout engine for the markup inside `view!`.

rustfmt leaves `view!` markup alone, so this fills the gap without a second Rust formatter: it lays markup out with the project's `rustfmt.toml` width and indentation, copies text exactly, and keeps every comment. The `[format]` table in `leptoskit.toml` sets its style, `--check` fails CI when a file would change, and `--stdin` serves editors.
