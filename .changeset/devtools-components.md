---
"release": minor
---

# See your components in DevTools

Every element built inside a `#[component]` now carries the component's name in development builds. The DevTools tree names a component on the row where its output starts, as in `<ClipCard> View`, the node panel shows it, and the search finds it. Release builds and builds without the `devtools` feature are unchanged. Native apps show components in a later release.
