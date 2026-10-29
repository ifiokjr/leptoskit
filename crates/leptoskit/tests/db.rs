//! `leptoskit::db` re-exports the whole database layer, including the macros
//! mixins declare: a table flattens `leptoskit::db::Timestamps` through the
//! facade exactly as through `leptoskit_db`.

#![cfg(feature = "db-server")]

use leptoskit::db;
use leptoskit::db::Id;
use leptoskit::db::Table;

/// A label, checked wherever it's used.
#[derive(db::Scalar, Clone, Debug, PartialEq, Eq)]
#[db(crate = "leptoskit::db")]
#[db(max_len = 10)]
pub struct Label(String);

// This crate's own tests would otherwise resolve the runtime as `crate::db`.
#[db::table(name = "labels", public, crate = "leptoskit::db")]
pub struct Labelled {
	pub id: Id<Self>,
	pub label: Label,
	#[db(flatten)]
	pub times: db::Timestamps,
}

#[test]
fn tables_flatten_mixins_through_the_facade() {
	assert_eq!(
		Labelled::COLUMNS,
		["id", "label", "created_at", "updated_at"]
	);
	let row = NewLabelled {
		label: Label::parse("new").unwrap(),
	}
	.into_row();
	assert_eq!(row.times.created_at, row.times.updated_at);
	assert_eq!(PublicLabelled::from(row).label.as_inner(), "new");
}
