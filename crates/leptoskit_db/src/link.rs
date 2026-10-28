//! Following links: the rows an `Id<T>` column points at, which
//! `#[db::table]` generates accessors for.

use crate::Col;
use crate::Connection;
use crate::Id;
use crate::Query;
use crate::Result;
use crate::Table;

/// A table keyed by a single `Id<Self>`, which is what an `Id<T>` column
/// points at. `#[db::table]` implements it.
pub trait HasId: Table {
	/// The row's key.
	fn id(&self) -> Id<Self>;
}

/// The rows of `T` with this key: one at most, unless it's soft-deleted.
pub fn by_id<T: HasId>(id: Id<T>) -> Query<T> {
	let key = Col::<T, Id<T>>::new(T::KEY[0]);
	Query::new().filter(move |_| key.eq(id))
}

/// The row a required link points at.
#[doc(hidden)]
pub async fn one<T: HasId>(connection: &impl Connection, id: Id<T>) -> Result<T> {
	by_id(id).one(connection).await
}

/// The row an optional link points at, if it's set and the row exists.
#[doc(hidden)]
pub async fn optional<T: HasId>(
	connection: &impl Connection,
	id: Option<Id<T>>,
) -> Result<Option<T>> {
	match id {
		Some(id) => by_id(id).first(connection).await,
		None => Ok(None),
	}
}
