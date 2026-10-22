---
"release": minor
---

# Format Rust and view! markup with `leptoskit format`

A new `leptoskit_fmt` crate, built into the CLI as `leptoskit format` (and `lk format`), replaces leptosfmt. It formats Rust with rustfmt — nightly by default, honouring the project's `rustfmt.toml` — and lays out the rstml markup inside `view!` macros (`view`, `leptos::view`, and `leptoskit::view` by default) at the same width and indentation, with every Rust expression inside the markup formatted by rustfmt at the column it sits in. Formatting is idempotent, keeps every comment, copies text and literals byte for byte, and leaves markup that does not parse exactly as written with a warning.

`--check` prints a diff and fails when anything would change, `--stdin` formats for editors, and whole-workspace runs format files in parallel and skip files a previous run left formatted. Settings live in a new `[format]` table in `leptoskit.toml`.
