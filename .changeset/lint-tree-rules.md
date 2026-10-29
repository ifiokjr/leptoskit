---
"release": minor
---

# Lint rules from React Native and Flutter

`leptoskit lint` gains rules about component trees. Each reads a tree the same way whether it is written with builder calls or `view!` markup:

- `redundant_view`: a `View` with no classes, props, or handlers that only wraps one child, after Flutter's `avoid_unnecessary_containers`.
- `spacer_view`: an empty `View` sized only to push its siblings apart. Use `gap-*` on the parent or `Spacer()` instead, after Flutter's `sized_box_for_whitespace`.
- `children_last`: a builder property set after `.child(…)`, after Flutter's `sort_child_properties_last`.
- `image_label`: an `Image` with no accessible name. Add `accessibility_label`, or mark decoration with `aria_hidden`.
- `color_literal`: a hand-written color such as `bg-[#1e293b]` or `Color::rgba(…)`, after React Native's `no-color-literals`. Off by default.

The first four warn by default. Set any of them under `[lint.rules]` in `leptoskit.toml`.
