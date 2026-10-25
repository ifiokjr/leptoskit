# leptoskit_table

Headless tables for the leptoskit element tree, ported from [TanStack Table](https://tanstack.com/table) at its core: typed `Column`s describe how a row's cells read, sort, and filter; `TableState` holds what the screen did to the rows; pure reducers (`for_header_press`, `for_filter`, `for_page`) propose the next state; and `DataTable` renders the whole grid — pressable headers with `aria-sort`, cells, a filter field, pagination — as ARIA roles over the shared tree.

```rust
use leptoskit_table::Column;
use leptoskit_table::SortDirection;
use leptoskit_table::TableState;
use leptoskit_table::for_header_press;
use leptoskit_table::rows;

struct Planet { name: &'static str, moons: u8 }

let data = [
    Planet { name: "Jupiter", moons: 95 },
    Planet { name: "Mars", moons: 2 },
    Planet { name: "Saturn", moons: 146 },
];
let columns = [
    Column::new("name", "Name", |planet: &Planet| planet.name.to_owned()).sortable(),
    Column::new("moons", "Moons", |planet: &Planet| planet.moons.to_string())
        .sort_by(|a, b| a.moons.cmp(&b.moons)),
];

let state = for_header_press(&TableState::new(), "moons");
assert_eq!(state.sorting.map(|sort| sort.direction), Some(SortDirection::Ascending));

let page = rows(&data, &columns, &state);
assert_eq!(page.rows.iter().map(|p| p.name).collect::<Vec<_>>(), ["Mars", "Jupiter", "Saturn"]);
```

The pipeline runs in the `TanStack` order — filter, sort (stable), paginate — and `DataTable` is one composition of the result: build a custom grid from `rows` and the same `table`/`row`/`columnheader`/`cell` roles when it is not the one you want. The grid is roles over the shared element tree, so it is announced by screen readers on the web and on every native backend.

Use the crate directly or enable `leptoskit`'s `table` feature and import `leptoskit::table::*`.
