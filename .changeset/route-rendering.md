---
"release": minor
---

# Render routes and navigate between them

The generated `routes.rs` now renders: `Navigation` is the app's router, and `navigation.render()` calls the current route's page inside the layouts of every directory above it. Pages and layouts receive the props they declare by name: `route` (a page's struct, or the whole `Route` for a layout), `navigation`, and a layout's `children`. Both a lowercase `page` and a `#[component] Page` are called the same way, and a parameter the router cannot pass is a generation error at its file and line.

`navigate`, `replace`, `back` and `can_go_back` change the route. Natively, earlier routes stay in memory. In the browser, `Navigation::from_browser()` (the `web` feature) starts from the page's URL, adds a history entry per navigation, and follows the back and forward buttons. `navigation.link(to, label)` builds a pressable that navigates; `Element::href` makes any pressable a link, which the web renders as an `<a href>` that opens in a new tab on a modified click while a plain click stays in the app.
