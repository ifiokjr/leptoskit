---
"release": minor
---

# Test leptoskit components in a real browser

`leptos_testing_library` is replaced by `leptoskit_testing_library`, which tests leptoskit components rather than raw leptos HTML. `render(|| view! { <Counter count=count /> })` mounts a component with the leptoskit web renderer, as a web app does, and returns a `Screen`:

- **Testing Library queries.** `get_by_*`, `get_all_by_*`, `query_by_*`, and `query_all_by_*`, by text, label, role, and test ID, plus async `find_by_*` and `find_all_by`, which wait for content to appear (one second by default, `Screen::with_timeout` to change it).
- **Widget-test actions.** `tap`, `enter_text`, and `toggle` act on exactly one enabled control and let leptos re-render before returning; `pump` settles leptos on its own.
- **Diagnostics.** Hidden elements never match, and every failure is a `leptoskit_testing::TestError` with the rendered markup.

The queries come from a new `leptoskit_testing::Queries` trait that `WidgetTester` implements too, so the same `get_by_role(Role::Button, "Save")` works headless and in the browser. `WidgetTester::find` is now the same as `Queries::get`.

`Element::test_id("save")` marks an element for `Finder::test_id`; the web renders it as `data-testid="save"`, and `NativeProps::test_id` reads it back.

To migrate, replace `render_for_test` with `render`, build the view from leptoskit components, and swap `get_by_id` and similar DOM lookups for text, label, role, or test ID queries. `leptoskit test web` now defaults to `--package leptoskit_testing_library`.
