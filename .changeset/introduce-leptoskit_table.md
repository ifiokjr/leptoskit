---
leptoskit_table: major
---

# leptoskit_table: headless tables

`leptoskit_table` is headless tables in the style of TanStack Table: typed columns, sorting, filtering, pagination, and a renderable grid of ARIA roles.

It computes the model, filtering, then sorting, then paginating, while the screen owns the state: store what `for_header_press`, `for_filter`, and `for_page` propose and rebuild. Render the grid with `DataTable`, or read `rows` and draw your own with leptoskit components; either way it behaves the same on web and native.
