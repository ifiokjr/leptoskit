---
leptoskit_fmt: breaking
---

# leptoskit_fmt: the view! formatter

`leptoskit_fmt` is the formatter behind `leptoskit format`: rustfmt for Rust and a layout engine for the markup inside `view!`.

rustfmt leaves `view!` markup alone, so this fills the gap without a second Rust formatter. It runs in editor integrations, pre-commit, and CI, and agrees with the lint rules.
