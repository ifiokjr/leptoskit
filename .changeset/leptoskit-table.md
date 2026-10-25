---
"release": minor
---

# Add leptoskit_table: headless tables

New `leptoskit_table` crate porting TanStack Table's core model: typed `Column`s (cell reader, optional row comparator, optional filter rule), `TableState` with pure reducers (`for_header_press` cycling none→ascending→descending, `for_filter`, `for_page` with clamping), and a `rows` pipeline that filters, stably sorts, and paginates. `DataTable` renders the grid — pressable `columnheader` buttons carrying `aria-sort`, `row`/`cell` roles, a filter field, and a pagination footer with a live page status — over the shared element tree, so one table runs on the web and every native backend. Six model tests and two widget tests over the real renderer; new tables guide; re-exported behind the `leptoskit` `table` feature.
