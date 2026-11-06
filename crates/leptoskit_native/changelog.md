# Changelog

All notable changes to this project are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.3.0](https://github.com/leptoskit/leptoskit/releases/tag/v0.3.0) (2026-11-06)

### Breaking changes

#### leptoskit_native: components, Tailwind, and the renderer

_Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #429](https://github.com/leptoskit/leptoskit/pull/429)

`leptoskit_native` is the native side of the shared tree: builders for every component, from `View`, `Text`, and `Button` to `TextInput`, `PasswordInput`, `Slider`, and `ScrollView`, the Tailwind compiler, the retained element tree, and the diff that turns a commit into a mutation batch.

Every native backend implements `NativeBackend` and consumes the mutations this crate produces; the web build uses its optional `web` feature to render the same builders through leptos. App code normally reaches it through `leptoskit::prelude`, not as a direct dependency, so one screen works on every target.

### Features

- **Theme tokens in leptoskit.toml and deeper CSS var().** `[theme.colors]` and `[theme.lengths]` in `leptoskit.toml` name design tokens that `Theme::install_maps` loads into the process-wide Theme map. Arbitrary color and length utilities accept `var(--name)`, one nested `var()` fallback, and `/opacity` on color vars. The web adapter prepends `Theme::css()` so the browser sees the same `:root` custom properties native resolves through Theme. Tailwind `@theme` / `tailwind.config` files remain unsupported. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #538](https://github.com/leptoskit/leptoskit/pull/538)
- **CSS color variables and a Theme map.** Arbitrary color utilities can use `var(--name)` or `var(--name, fallback)`. The web stylesheet keeps the CSS function. Native paint resolves the name through `Theme::set_color` (or the fallback when the theme has no entry). `Theme::css` emits a `:root` sheet of the same names for the browser. Nested `var()`, non-color variables, and opacity modifiers on `var()` stay unsupported. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #536](https://github.com/leptoskit/leptoskit/pull/536)

### Fixes

- **Correct README claims about generated renderers, accessibility, runtime plugin registration, password screens, RPC macros, and interpreter updates.** Share crate summaries with Rust documentation and derive MSRV badges from Cargo through MDT templates, with a documentation drift check. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #515](https://github.com/leptoskit/leptoskit/pull/515)
- **Paint italic text and per-edge borders on Linux and Android.** `border-t`, `border-l-2`, and the other edge border utilities now get the same default stroke color as a uniform `border`, and GTK and Android paint each edge instead of leaving those strokes undrawn. The `italic` utility also draws through GTK labels and Android text views, matching the web and the terminal. AppKit, UIKit, and Win32 still leave per-edge borders undrawn and italic upright. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #534](https://github.com/leptoskit/leptoskit/pull/534)
- **Yoga LayoutUnit rounding for Chrome grid parity.** Fractional grid tracks and percentage lengths resolve through Blink-style 1/64 CSS-pixel layout units, so Yoga frames match Chrome `getBoundingClientRect` for the shared Tailwind class trees. The grid parity probe covers nested grids, dense auto-flow, minmax, flex edges, absolute insets, percentages, and overflow. _Owner:_ [@ifiokjr](https://github.com/ifiokjr) · _Review:_ [PR #540](https://github.com/leptoskit/leptoskit/pull/540)
