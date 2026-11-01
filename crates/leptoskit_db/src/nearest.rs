//! The rows nearest a value: a shape on a geospatial column, or a vector
//! on a vector column.

use crate::Col;
use crate::Query;
use crate::Table;

/// A column whose rows [`Query::nearest`] orders by distance to a `Target`:
/// a geospatial column and a shape it can measure against, or a vector
/// column and a vector of its type and dimensions.
#[diagnostic::on_unimplemented(
	message = "`nearest` can't measure a `{Self}` column against a `{Target}`",
	label = "a geospatial column measures shapes in its coordinate system; a vector column \
	         measures vectors of its own type and dimensions"
)]
pub trait NearestTo<Target>: Sized {
	/// The search [`Query::nearest`] starts.
	type Search<T: Table>;

	#[doc(hidden)]
	fn nearest<T: Table>(query: Query<T>, column: Col<T, Self>, target: &Target)
	-> Self::Search<T>;
}

impl<T: Table> Query<T> {
	/// The matching rows nearest `target`, nearest first, by the values in
	/// `column`; rows without one are left out. Give it a limit:
	///
	/// ```ignore
	/// let nearby = Place::query()
	/// 	.nearest(|p| p.location, &here)
	/// 	.with_distance()
	/// 	.limit(20)
	/// 	.all(&db)
	/// 	.await?;
	/// ```
	///
	/// On a geospatial column, see [`geo::Nearest`](crate::geo::Nearest);
	/// on a vector column, see [`vector::Nearest`](crate::vector::Nearest).
	pub fn nearest<V: NearestTo<G>, G>(
		self,
		column: impl FnOnce(&T::Columns) -> Col<T, V>,
		target: &G,
	) -> V::Search<T> {
		V::nearest(self, column(T::columns()), target)
	}
}
