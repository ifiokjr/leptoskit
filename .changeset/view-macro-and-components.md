---
"release": minor
---

# Write shared screens as markup with `view!`

`leptoskit::prelude` now exports a `view!` macro and a `#[component]` attribute from the new `leptoskit_macros` crate. `view!` parses rstml markup, the syntax leptos uses, and expands it into the builder calls leptoskit screens already make, so markup renders on the web and on every native backend: `view! { <View class="p-4"><Text>"Count: " {count}</Text></View> }`.

`#[component]` keeps a function's signature and gives it a props builder, so `view!` can set its props by name, with leptos's `#[prop(optional)]`, `#[prop(default = …)]`, and `#[prop(into)]`. A missing or repeated prop is a compile error that names the prop.

To support them, `leptoskit_native` adds `IntoView`, `IntoChildren`, `Fragment`, and `Children`; `Element` gains `source`, `checked`, `radio_group`, `slider`, `progress`, and `running` setters; and a `Text` or `Button` now takes plain text children as its content. The gallery's screens are written with `view!`, and a snapshot test checks that they build the same element trees as before.
