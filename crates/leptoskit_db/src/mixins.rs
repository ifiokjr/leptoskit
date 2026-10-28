//! Mixins: sets of columns declared once and flattened into any table, as
//! Gel's abstract types are extended.

use crate::Timestamp;
use crate::mixin;

/// A set of columns declared with `#[db::mixin]`. The attribute implements
/// it; you don't.
pub trait Mixin {
	/// What a client sees: a generated `Public…` struct of the public
	/// fields, or `()` when there are none.
	type Public;
	/// What an insert provides: the mixin itself, a generated `New…` struct
	/// of the fields the ORM doesn't fill in, or `()` when it fills them all.
	type New;
}

/// The type of a mixin's field by position, so a table can name a column's
/// type without seeing the mixin's imports.
#[doc(hidden)]
pub trait MixinField<const N: usize> {
	type Type;
}

/// A table that flattens mixin `M` without a prefix. Write code once for
/// every table with a mixin:
///
/// ```ignore
/// fn age<T: HasMixin<Timestamps>>(row: &T) -> chrono::TimeDelta {
/// 	now() - row.mixin().created_at
/// }
/// ```
pub trait HasMixin<M> {
	/// The mixin's fields.
	fn mixin(&self) -> &M;

	/// The mixin's fields, to change.
	fn mixin_mut(&mut self) -> &mut M;
}

/// When a row was created and last written. The ORM fills both in.
#[mixin(crate = "crate")]
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Timestamps {
	/// When the row was inserted.
	#[db(created_at)]
	pub created_at: Timestamp,
	/// When the row was last written.
	#[db(updated_at)]
	pub updated_at: Timestamp,
}

/// When a row was deleted. Deleting the row sets it instead of removing the
/// row, and queries skip deleted rows unless they ask for them.
#[mixin(crate = "crate")]
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct SoftDelete {
	/// When the row was deleted, if it was.
	#[db(deleted_at)]
	pub deleted_at: Option<Timestamp>,
}

/// A version number for optimistic concurrency: `save` writes the row only
/// while its version is the one it read, then increments it. A save based on
/// a stale read fails with [`DbError::Stale`](crate::DbError::Stale) instead
/// of overwriting someone else's change.
#[mixin(crate = "crate")]
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Versioned {
	/// How many times the row has been written.
	#[db(version)]
	pub version: i64,
}
