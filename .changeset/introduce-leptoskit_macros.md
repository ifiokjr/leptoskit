---
leptoskit_macros: breaking
---

# leptoskit_macros: view! and #[component]

`leptoskit_macros` provides the `view!` markup macro and `#[component]` attribute that expand into the same builders a hand-written tree uses.

Use them through `leptoskit::prelude`. The macro parses rstml markup and emits builder calls, so markup and builder code produce one element tree and neither is a second-class path.
