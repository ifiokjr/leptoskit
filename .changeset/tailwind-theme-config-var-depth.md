---
"leptoskit": minor
"leptoskit_config": minor
"leptoskit_native": minor
---

# Theme tokens in leptoskit.toml and deeper CSS var()

`[theme.colors]` and `[theme.lengths]` in `leptoskit.toml` name design tokens that `Theme::install_maps` loads into the process-wide Theme map. Arbitrary color and length utilities accept `var(--name)`, one nested `var()` fallback, and `/opacity` on color vars. The web adapter prepends `Theme::css()` so the browser sees the same `:root` custom properties native resolves through Theme. Tailwind `@theme` / `tailwind.config` files remain unsupported.
