# Website drawing and typography

The hosted site and documentation follow [kit.pina.rs](https://kit.pina.rs/). Its [site implementation](https://github.com/pina-rs/solana_kit/tree/df8619ae6290747f69d4c53aa7705bd7470aa9c5/docs/site) uses `skribble_rough` to generate SVG masks, local Skribble fonts, and CSS palette tokens. The adapted documentation chrome is in `docs/src/theme/skribble.css`; the public layout remains LeptosKit builder components in `crates/leptoskit_app/src/landing.rs`.

`docs/site/src/ink.rs` calls the actual Rust engine pinned in the workspace to skribble commit `00845d0d2a1c07a126079453de342811ee1cff64`. It generates stable frames, rules, underlines, hatching and LeptosKit artwork. CSS masks follow the current paper/ink palette; non-scaling strokes retain their line weight when stretched. The engine's pen-shaped ink draws the coloured hero pieces.

The documentation site's `Site::generated_asset` calls package those files into `_leptoskit/generated/` on every build, including watched rebuilds. No generated SVGs are written to the source book or checked in. The app's packager builds the book into `/docs/`, so the public page, console and book share the same local assets. The book's external theme initializer also runs on the public page and console, preserving the selected theme without inline JavaScript.

The exact upstream font source and export tool version are recorded beside the WOFF2 files in `docs/src/assets/fonts/skribble/readme.md`, with the licenses. The integration uses the same Rust drawing-library approach as the reference site. Existing shared builders, native widgets and Rive playback continue to provide application behavior.
