# Tables

A table is two things: a pipeline over rows — filter, sort, paginate — and a grid of roles a screen reader can navigate. `leptoskit_table` is both, ported from [TanStack Table](https://tanstack.com/table) at its core: typed columns and pure state on one side, a rendered grid of ARIA roles on the other.

```toml
[dependencies]
leptoskit_table = "0.1"
```

Or enable `leptoskit`'s `table` feature and import `leptoskit::table::*`.

[`examples/observatory`](https://github.com/ifiokjr/leptoskit/tree/main/examples/observatory) is a complete app built this way: a sortable, searchable star catalogue with its own page labels.

## Columns

A `Column` is data, not state: build the set once. The cell reads a string; sorting and filtering are defined over the row itself, so they never depend on how a cell happens to format.

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

// Twenty-five rows to a page; a page size of zero turns pagination off.
let state = TableState::new().with_page_size(25);

// A press cycles one column through none → ascending → descending → none.
let state = for_header_press(&state, "moons");

// Typing filters and restarts at the first page.
let state = for_filter(&state, text);

// Page turns clamp to the filtered rows' pages.
let total = rows(&data, &columns, &state).total;
let state = for_page(&state, total, next_page);
```

`rows` resolves the pipeline in the `TanStack` order — filter, then a stable sort, then pagination — and returns the page plus the counts the controls need. A sort on a column that is not sortable, or on an id no column carries, is inert: the rows keep their order. A page past the end, after the data shrank, shows the last page.

## The rendered grid

`DataTable` is one composition of all of it: sortable headers that carry `aria-sort` while their column runs the sort, cells from the columns' readers, a filter field, and a previous/page-status/next footer when the state has a page size. Every interaction hands back the whole next state, so the screen only stores it:

```rust,ignore
use leptoskit_table::{DataTable, TableLabels, TableState};

let state = RwSignal::new(TableState::new().with_page_size(25));
let labels = TableLabels::default();

move || DataTable(&columns, &planets, &state.get(), &labels, move |next| state.set(next))
```

`TableLabels` holds the words the table shows and announces — the filter field's name, the page buttons, and the page status — so a translated screen passes its own; `Default` is English.

The grid is `table`, `row`, `columnheader`, and `cell` roles over the shared element tree. A sortable header holds a real button, so a screen reader announces both the column and the control. Build a custom grid from `rows` and the same roles when this composition is not the one — everything the pipeline produces is public.

## On each platform

The table renders on the web and every native backend. Screen readers navigate it as a table on the web, macOS, Windows, and Linux. iOS and Android have no table roles, so their screen readers read the headers and cells as text in order. `aria-sort` reaches the web only: on native platforms the sorted header shows its ▲ or ▼, and the direction is not announced.
