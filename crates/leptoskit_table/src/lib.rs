//! Headless tables for the shared leptoskit element tree.
//!
//! This is `TanStack Table`'s model at its core, ported to leptoskit's grain:
//! typed [`Column`]s describe how a row's cells read, sort, and filter;
//! [`TableState`] holds what the screen did to the rows — the active sort,
//! the global filter, the page — and pure reducers propose the next state; a
//! [`rows`] pass resolves the pipeline in the `TanStack` order (filter, sort,
//! paginate) and [`DataTable`] renders the whole grid as ARIA roles.
//!
//! The screen owns the state, exactly like every controlled control here:
//! store what [`for_header_press`], [`for_filter`], and [`for_page`] propose,
//! and rebuild.
//!
//! ```
//! use leptoskit_table::Column;
//! use leptoskit_table::SortDirection;
//! use leptoskit_table::TableState;
//! use leptoskit_table::for_header_press;
//! use leptoskit_table::rows;
//!
//! struct Planet {
//! 	name: &'static str,
//! 	moons: u8,
//! }
//!
//! let data = [
//! 	Planet {
//! 		name: "Jupiter",
//! 		moons: 95,
//! 	},
//! 	Planet {
//! 		name: "Mars",
//! 		moons: 2,
//! 	},
//! 	Planet {
//! 		name: "Saturn",
//! 		moons: 146,
//! 	},
//! ];
//! let columns = [
//! 	Column::new("name", "Name", |planet: &Planet| planet.name.to_owned()).sortable(),
//! 	Column::new("moons", "Moons", |planet: &Planet| planet.moons.to_string())
//! 		.sort_by(|a, b| a.moons.cmp(&b.moons)),
//! ];
//!
//! // Pressing a header proposes the next state; the screen stores it.
//! let state = for_header_press(&TableState::new(), "moons");
//! assert_eq!(
//! 	state.sorting.as_ref().map(|sort| sort.direction),
//! 	Some(SortDirection::Ascending)
//! );
//!
//! let page = rows(&data, &columns, &state);
//! assert_eq!(
//! 	page.rows
//! 		.iter()
//! 		.map(|planet| planet.name)
//! 		.collect::<Vec<_>>(),
//! 	["Mars", "Jupiter", "Saturn"]
//! );
//! ```

// The rendered component keeps its PascalCase name, like the built-ins.
#![allow(non_snake_case)]

mod column;
mod model;
mod view;

pub use column::Column;
pub use model::SortDirection;
pub use model::SortSpec;
pub use model::TableRows;
pub use model::TableState;
pub use model::for_filter;
pub use model::for_header_press;
pub use model::for_page;
pub use model::page_count;
pub use model::rows;
pub use view::DataTable;
