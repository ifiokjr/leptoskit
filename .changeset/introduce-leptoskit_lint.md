---
leptoskit_lint: breaking
---

# leptoskit_lint: project conventions

`leptoskit_lint` contains the rules behind `leptoskit lint`: components carry `#[component]`, and component trees follow the markup style `leptoskit.toml` selects.

rustc and clippy check Rust; these rules check the conventions a leptoskit project relies on, with diagnostics that name the file, line, and fix.
