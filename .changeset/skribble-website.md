---
"release": minor
---

# Draw the website and book with skribble

The hosted landing page and documentation use skribble's pinned Rust rough engine, following kit.pina.rs: generated SVG borders, hatching and artwork, a paper-and-ink palette in light and dark modes, and locally served Skribble fonts. The shared LeptosKit example and Rive motion remain interactive.

`leptoskit_docs::Site::generated_asset` packages build-time artwork under `_leptoskit/generated/` and preserves it during development-server rebuilds without modifying the book's sources. Generated filenames are validated before the build changes its output.
