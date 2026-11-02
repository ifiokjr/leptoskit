<h1 align="center">leptoskit_table</h1>

<p align="center">
  <strong>leptoskit_table</strong> is headless tables in the style of TanStack Table: typed columns, sorting, filtering, pagination, and a renderable grid of ARIA roles.
</p>

<br>

<p align="center">
  <a href="#overview"><strong>Overview</strong></a> ·
  <a href="#how-it-fits"><strong>How it fits</strong></a> ·
  <a href="#use-it"><strong>Use it</strong></a> ·
  <a href="#learn-more"><strong>Learn more</strong></a> ·
  <a href="https://docs.rs/leptoskit_table"><strong>Docs</strong></a> ·
  <a href="https://github.com/ifiokjr/leptoskit"><strong>Repository</strong></a>
</p>

<br>

<p align="center">
  <a href="https://crates.io/crates/leptoskit_table"><img alt="crates.io" src="https://img.shields.io/crates/v/leptoskit_table.svg"></a>
  <a href="https://docs.rs/leptoskit_table"><img alt="docs.rs" src="https://docs.rs/leptoskit_table/badge.svg"></a>
  <a href="https://github.com/ifiokjr/leptoskit/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/ifiokjr/leptoskit/actions/workflows/ci.yml/badge.svg"></a>
  <a href="https://github.com/ifiokjr/leptoskit/blob/main/Cargo.toml"><img alt="MSRV" src="https://img.shields.io/badge/MSRV-1.95.0-blue.svg"></a>
  <a href="https://opensource.org/license/unlicense"><img alt="License: Unlicense" src="https://img.shields.io/badge/license-Unlicense-blue.svg"></a>
</p>

<br>

## Overview

Headless tables for the leptoskit element tree, ported from [TanStack Table](https://tanstack.com/table) at its core: typed `Column`s describe how a row's cells read, sort, and filter; `TableState` holds what the screen did to the rows; pure reducers (`for_header_press`, `for_filter`, `for_page`) propose the next state; and `DataTable` renders the whole grid — header buttons, `aria-sort`, cells, a filter field, pagination, in words `TableLabels` lets you translate — as ARIA roles over the shared tree.

## Columns

A `Column` is data, not state: build the set once. `.sortable()` sorts lexically over the cell text, `.sort_by` compares the rows themselves, and `.filter_by` replaces the column's part in the global filter, which by default matches cell text case-insensitively in any column.

## Rendering

The pipeline runs in the `TanStack` order — filter, sort (stable), paginate — and `DataTable` is one composition of the result: build a custom grid from `rows` and the same `table`/`row`/`columnheader`/`cell` roles when it is not the one you want. `DataTable` hands every interaction back as the whole next `TableState` through one `on_change`.

The roles reach screen readers on the web, macOS, Windows, and Linux. iOS and Android have no table roles, so their screen readers read the cells as text in order. `aria-sort` reaches the web only; on native platforms the sorted header shows its ▲ or ▼ without announcing the direction.

Use the crate directly or enable `leptoskit`'s `table` feature and import `leptoskit::table::*`.

## How it fits

It computes the model; the app renders rows with leptoskit components and keeps the same behavior on web and native. It pairs with `leptoskit_virtual` for long lists.

## Use it

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
assert_eq!(state.sorting.as_ref().map(|sort| sort.direction), Some(SortDirection::Ascending));

let page = rows(&data, &columns, &state);
assert_eq!(page.rows.iter().map(|p| p.name).collect::<Vec<_>>(), ["Mars", "Jupiter", "Saturn"]);
```

## Learn more

- [Crate documentation](https://docs.rs/leptoskit_table)
- [leptoskit documentation](https://ifiokjr.github.io/leptoskit/)
- [Repository](https://github.com/ifiokjr/leptoskit)
- [Tables](https://ifiokjr.github.io/leptoskit/guide/18-tables.html)
