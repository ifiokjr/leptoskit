---
"release": minor
---

# Add leptoskit_table: headless tables

New `leptoskit_table` crate porting TanStack Table's core model: typed `Column`s (cell reader, optional row comparator, optional filter rule), `TableState` with pure reducers (`for_header_press` cycling none→ascending→descending→none, `for_filter`, `for_page` with clamping), and a `rows` pipeline that filters, stably sorts, and paginates, reading each text sort key once. A page size of zero turns pagination off, and a page past the end of shrunk data shows the last page. `DataTable(&columns, &data, &state, &labels, on_change)` renders the grid — `columnheader` cells holding real sort buttons, `aria-sort` on the sorted one, `row`/`cell` roles, a filter field, and a pagination footer with a live page status — over the shared element tree, and hands every interaction back as the whole next `TableState`. `TableLabels` translates its words. Re-exported behind the `leptoskit` `table` feature.
