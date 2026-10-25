# Tables

A table is two things: a pipeline over rows — filter, sort, paginate — and a grid of roles a screen reader can navigate. `leptoskit_table` is both, ported from [TanStack Table](https://tanstack.com/table) at its core: typed columns and pure state on one side, a rendered grid of ARIA roles on the other.

```toml
[dependencies]
leptoskit_table = "0.1"
```

Or enable `leptoskit`'s `table` feature and import `leptoskit::table::*`.

## Columns

A [`Column`][readme-column] is data, not state: build the set once. The cell reads a string; sorting and filtering are defined over the row itself, so they never depend on how a cell happens to format.

```rust,ignore
use leptoskit_table::Column;

struct Planet { name: &'static str, moons: u8 }

let columns = [
    Column::new("name", "Name", |planet: &Planet| planet.name.to_owned()).sortable(),
    Column::new("moons", "Moons", |planet: &Planet| planet.moons.to_string())
        .sort_by(|a, b| a.moons.cmp(&b.moons)),
];
```

`.sortable()` sorts lexically over the cell text; `.sort_by` compares the rows. `.filter_by` replaces the column's contribution to the global filter, which by default matches cell text case-insensitively across any column.

## State, owned by the screen

`TableState` holds the active sort, the global filter text, and the page. It never mutates itself: reducers propose, the screen stores.

```rust,ignore
use leptoskit_table::{TableState, for_header_press, for_filter, for_page, rows};

// A press cycles none → ascending → descending → none.
let state = for_header_press(&state, "moons");

// Typing filters and restarts at the first page.
let state = for_filter(&state, text);

// Page turns clamp to the filtered rows' pages.
let total = rows(&data, &columns, &state).total;
let state = for_page(&state, total, next_page);
```

`rows` resolves the pipeline in the `TanStack` order — filter, then a stable sort, then pagination — and returns the page plus the counts the controls need. A sort on an id no column carries is inert: the pipeline only sorts by columns that exist.

## The rendered grid

`DataTable` is one composition of all of it: pressable headers that carry `aria-sort` while their column runs the sort, cells from the columns' readers, a filter field, and a previous/page-status/next footer when the state has a page size. The grid is `table`, `row`, `columnheader`, and `cell` roles over the shared element tree, so screen readers announce it on the web and on every native backend.

Build a custom grid from `rows` and the same roles when this composition is not the one — everything the pipeline produces is public.

## Testing

The widget tests drive the real thing: tapping a header commits `aria-sort="ascending"` and reorders the committed rows through the real event dispatch, and the pagination presses move between pages with the page status announced as a live region.

[readme-column]: https://github.com/ifiokjr/leptoskit/tree/main/crates/leptoskit_table#columns
