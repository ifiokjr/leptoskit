//! Following links: the rows an `Id<T>` column points at, which
//! `#[db::table]` generates accessors for.

use std::collections::HashMap;

use crate::Action;
use crate::Col;
use crate::Connection;
use crate::Expr;
use crate::Id;
use crate::Query;
use crate::Result;
use crate::Table;
use crate::connection::atomically;
use crate::policy::Subquery;

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

/// A table linking two others many to many, whose rows can carry
/// properties of the link itself: `#[db::link]` declares one and implements
/// this.
///
/// ```ignore
/// /// A tag on a post.
/// #[db::link(name = "post_tags", from = Post as tags, to = Tag as posts)]
/// pub struct PostTag {
/// 	pub position: i16,
/// }
///
/// post.tags().add(&db, tag.id, PostTagProperties { position: 0 }).await?;
/// let tags: Vec<(Tag, PostTag)> = post.tags().order_by(|link| link.position.asc()).all(&db).await?;
/// ```
pub trait Link: Table {
	/// The table a link starts from.
	type From: HasId;
	/// The table a link points at.
	type To: HasId;
	/// What a new link holds besides its two ends: the generated
	/// `{Link}Properties`, or `()` when the link holds nothing else.
	type Properties;
	/// The column holding the starting row's key.
	const FROM: &'static str;
	/// The column holding the linked row's key.
	const TO: &'static str;

	/// A new link between two rows, with the ORM's columns filled in.
	fn link(from: Id<Self::From>, to: Id<Self::To>, properties: Self::Properties) -> Self;

	/// The row the link starts from.
	fn source_id(&self) -> Id<Self::From>;

	/// The row the link points at.
	fn target_id(&self) -> Id<Self::To>;
}

/// A link that code outside its crate may add and remove: every link but a
/// schema pack's, which only the pack's own API changes.
#[diagnostic::on_unimplemented(
	message = "`{Self}` belongs to a schema pack, so only the pack's own API adds or removes its \
	           links",
	label = "a pack's link"
)]
pub trait WritableLink: Link {}

/// The rows one row is linked to through a [`Link`] table, in one
/// direction: `post.tags()`, or back the other way, `tag.posts()`.
///
/// Reading obeys every table's policies: a link the request may not read,
/// or one pointing at a row it may not see or that is soft-deleted, isn't
/// there.
#[must_use = "links do nothing until they're read or changed"]
pub struct Links<J: Link, Near, Far> {
	near: Id<Near>,
	near_column: &'static str,
	far_column: &'static str,
	far_of: fn(&J) -> Id<Far>,
	build: fn(Id<Near>, Id<Far>, J::Properties) -> J,
	far_filter: Option<Expr>,
	rows: Query<J>,
}

impl<J: Link> Links<J, J::From, J::To> {
	/// The rows `from` links to.
	pub fn forward(from: Id<J::From>) -> Self {
		Self::new(from, J::FROM, J::TO, J::target_id, J::link)
	}
}

impl<J: Link> Links<J, J::To, J::From> {
	/// The rows linking to `to`.
	pub fn backward(to: Id<J::To>) -> Self {
		Self::new(to, J::TO, J::FROM, J::source_id, |to, from, properties| {
			J::link(from, to, properties)
		})
	}
}

impl<J: Link, Near: HasId, Far: HasId> Links<J, Near, Far> {
	fn new(
		near: Id<Near>,
		near_column: &'static str,
		far_column: &'static str,
		far_of: fn(&J) -> Id<Far>,
		build: fn(Id<Near>, Id<Far>, J::Properties) -> J,
	) -> Self {
		Self {
			near,
			near_column,
			far_column,
			far_of,
			build,
			far_filter: None,
			rows: Query::new(),
		}
	}

	/// Keeps only linked rows matching `condition`. Several filters must all
	/// match.
	pub fn filter(mut self, condition: impl FnOnce(&Far::Columns) -> Expr) -> Self {
		let condition = condition(Far::columns());
		self.far_filter = Some(match self.far_filter.take() {
			Some(filter) => filter & condition,
			None => condition,
		});
		self
	}

	/// Keeps only links whose own columns match `condition`.
	pub fn filter_link(mut self, condition: impl FnOnce(&J::Columns) -> Expr) -> Self {
		self.rows = self.rows.filter(condition);
		self
	}

	/// Orders by the links' own columns, such as a position. Without one,
	/// links come in the order of the linked rows' keys.
	pub fn order_by(mut self, order: impl FnOnce(&J::Columns) -> crate::Order) -> Self {
		self.rows = self.rows.order_by(order);
		self
	}

	/// At most this many links.
	pub fn limit(mut self, limit: u64) -> Self {
		self.rows = self.rows.limit(limit);
		self
	}

	/// Skips this many links.
	pub fn offset(mut self, offset: u64) -> Self {
		self.rows = self.rows.offset(offset);
		self
	}

	/// The linked rows, each with its link.
	pub async fn all(self, connection: &impl Connection) -> Result<Vec<(Far, J)>> {
		let far_of = self.far_of;
		let links = self.query(connection).all(connection).await?;
		let ids: Vec<Id<Far>> = links.iter().map(far_of).collect();
		let key = Col::<Far, Id<Far>>::new(Far::KEY[0]);
		let mut rows: HashMap<Id<Far>, Far> = Query::<Far>::new()
			.filter(|_| key.in_list(ids.iter().copied()))
			.all(connection)
			.await?
			.into_iter()
			.map(|row| (row.id(), row))
			.collect();

		// A row hidden or deleted between the two reads drops its link.
		Ok(links
			.into_iter()
			.zip(ids)
			.filter_map(|(link, id)| rows.remove(&id).map(|row| (row, link)))
			.collect())
	}

	/// The links themselves, in order.
	pub async fn link_rows(self, connection: &impl Connection) -> Result<Vec<J>> {
		self.query(connection).all(connection).await
	}

	/// The linked rows' keys, in order.
	pub async fn ids(self, connection: &impl Connection) -> Result<Vec<Id<Far>>> {
		let far_of = self.far_of;
		Ok(self
			.query(connection)
			.all(connection)
			.await?
			.iter()
			.map(far_of)
			.collect())
	}

	/// How many rows are linked.
	pub async fn count(self, connection: &impl Connection) -> Result<u64> {
		self.query(connection).count(connection).await
	}

	/// Whether `far` is linked.
	pub async fn contains(self, connection: &impl Connection, far: Id<Far>) -> Result<bool> {
		let column = Col::<J, Id<Far>>::new(self.far_column);
		Ok(self
			.query(connection)
			.filter(|_| column.eq(far))
			.count(connection)
			.await?
			> 0)
	}

	/// The query for this row's links whose ends the request may both see.
	fn query(self, connection: &impl Connection) -> Query<J> {
		let near = Col::<J, Id<Near>>::new(self.near_column);
		let id = self.near;
		let far = self.far_column;
		let rows = visible_ends::<J, Near, Far>(
			connection,
			self.rows.filter(|_| near.eq(id)),
			self.near_column,
			far,
			self.far_filter,
		);
		rows.ordered_or(|| Col::<J, Id<Far>>::new(far).asc())
	}
}

/// `links`, narrowed to those whose near and far rows the request may see,
/// and whose far row matches `far_filter`. Without a filter, a policy, or
/// soft deletes on a side, its foreign key alone says the row is there.
fn visible_ends<J: Link, Near: HasId, Far: HasId>(
	connection: &impl Connection,
	mut links: Query<J>,
	near_column: &'static str,
	far_column: &'static str,
	far_filter: Option<Expr>,
) -> Query<J> {
	let access = connection.access();
	let near = Subquery::new::<Near>(
		None,
		crate::policy::condition::<Near>(access, Action::Select),
		false,
	);
	let far = Subquery::new::<Far>(
		far_filter,
		crate::policy::condition::<Far>(access, Action::Select),
		false,
	);

	for (column, key, rows) in [
		(near_column, Near::KEY[0], near),
		(far_column, Far::KEY[0], far),
	] {
		if !rows.is_everything() {
			links = links.filter(|_| {
				Expr::Within {
					column,
					selected: key,
					rows: Box::new(rows),
				}
			});
		}
	}

	links
}

/// Fails with [`DbError::NotFound`](crate::DbError::NotFound) unless the
/// request may see the row with this key: no one links to or from a row
/// they can't see.
async fn require_visible<T: HasId>(connection: &impl Connection, id: Id<T>) -> Result<()> {
	by_id(id).one(connection).await.map(|_| ())
}

impl<J: WritableLink, Near: HasId, Far: HasId> Links<J, Near, Far> {
	/// Links `far`, or, when it's already linked, replaces the link's
	/// properties, and returns the link as stored. Both ends must be rows
	/// the request may see, or the link fails with
	/// [`DbError::NotFound`](crate::DbError::NotFound); with policies on the
	/// link table, the request must also be allowed to insert the link, or
	/// to change the one there.
	pub async fn add(
		self,
		connection: &impl Connection,
		far: Id<Far>,
		properties: J::Properties,
	) -> Result<J> {
		let link = (self.build)(self.near, far, properties);
		let near = self.near;

		atomically!(connection, |tx| {
			add_link::<J, Near, Far>(tx, link, near, far).await
		})
	}

	/// Unlinks `far`, and returns whether it was linked. Only a link the
	/// request may see, between rows it may see, is removed, and only when
	/// the link table's policies allow deleting it. The link table's
	/// `after_delete` hook runs, and an audited link table records it.
	pub async fn remove(self, connection: &impl Connection, far: Id<Far>) -> Result<bool> {
		let near = Col::<J, Id<Near>>::new(self.near_column);
		let column = Col::<J, Id<Far>>::new(self.far_column);
		let id = self.near;
		let links = Query::<J>::new().filter(|_| near.eq(id) & column.eq(far));
		let links = visible_ends::<J, Near, Far>(
			connection,
			links,
			self.near_column,
			self.far_column,
			None,
		);

		if !J::HOOKS.after_delete {
			return Ok(links.delete(connection).await? > 0);
		}

		atomically!(connection, |tx| remove_each(tx, links).await)
	}
}

impl<J: WritableLink, Near: HasId, Far: HasId> Links<J, Near, Far> {
	/// Makes `links` exactly the rows linked, each with its properties, in
	/// one transaction: links the ones that aren't, replaces the properties
	/// of the ones that are, and unlinks the rest. Returns the links as
	/// stored, in the order given.
	///
	/// ```ignore
	/// post.tags().set(&db, [(rust, PostTagProperties { position: 0 }), (sql, PostTagProperties { position: 1 })]).await?;
	/// ```
	///
	/// Only what the request sees is replaced: a link to a row it can't
	/// see stays, unseen, and filters narrow which links are replaced, so
	/// `post.tags().filter(|tag| tag.kind.eq("topic")).set(…)` leaves other
	/// tags alone. Every row in `links` must be one the request may see,
	/// as for [`Links::add`]. When a policy keeps a link from being
	/// removed, the whole change fails with
	/// [`DbError::Forbidden`](crate::DbError::Forbidden) and nothing
	/// changes.
	pub async fn set(
		self,
		connection: &impl Connection,
		links: impl IntoIterator<Item = (Id<Far>, J::Properties)>,
	) -> Result<Vec<J>> {
		let links: Vec<(Id<Far>, J::Properties)> = links.into_iter().collect();
		let mut seen = std::collections::HashSet::new();

		if let Some((far, _)) = links.iter().find(|(far, _)| !seen.insert(*far)) {
			return Err(crate::expr::invalid(&format!(
				"`{far}` is listed twice; a link holds each pair of rows once"
			)));
		}

		if self.rows.is_paged() {
			return Err(crate::expr::invalid(
				"`set` replaces every link the filters match, so it takes no `limit` or `offset`",
			));
		}

		atomically!(connection, |tx| set_links(tx, self, links).await)
	}
}

/// Removes the visible links to rows not in `links`, then adds or updates
/// each of `links`.
async fn set_links<J: Link, Near: HasId, Far: HasId>(
	connection: &impl Connection,
	current: Links<J, Near, Far>,
	links: Vec<(Id<Far>, J::Properties)>,
) -> Result<Vec<J>> {
	let Links {
		near,
		near_column,
		far_column,
		build,
		far_filter,
		rows,
		..
	} = current;
	require_visible(connection, near).await?;

	let near_key = Col::<J, Id<Near>>::new(near_column);
	let far_key = Col::<J, Id<Far>>::new(far_column);
	let kept: Vec<Id<Far>> = links.iter().map(|(far, _)| *far).collect();
	let stale = visible_ends::<J, Near, Far>(
		connection,
		rows.filter(|_| near_key.eq(near) & !far_key.in_list(kept.iter().copied())),
		near_column,
		far_column,
		far_filter,
	);
	let (condition, _) = stale.into_condition();
	let condition = condition.unwrap_or_else(Expr::always);
	let visible = Query::<J>::new()
		.filter(|_| condition.clone())
		.count(connection)
		.await?;
	let deletable = Query::<J>::new().filter(|_| condition.clone());
	let removed = if J::HOOKS.after_delete {
		remove_each_counted(connection, deletable).await?
	} else {
		deletable.delete(connection).await?
	};

	if removed < visible {
		return Err(crate::policy::refuse::<J>(connection, &condition, Action::Delete).await?);
	}

	let mut stored = Vec::with_capacity(links.len());

	for (far, properties) in links {
		require_visible(connection, far).await?;
		let link = build(near, far, properties);
		stored.push(crate::write::upsert(connection, link, &[J::FROM, J::TO]).await?);
	}

	Ok(stored)
}

/// Deletes the links `links` finds that the request may delete, one at a
/// time, so the link table's hooks run for each.
async fn remove_each<J: Link>(connection: &impl Connection, links: Query<J>) -> Result<bool> {
	Ok(remove_each_counted(connection, links).await? > 0)
}

/// Deletes the links `links` finds that the request may delete, one at a
/// time, and returns how many it deleted.
async fn remove_each_counted<J: Link>(
	connection: &impl Connection,
	links: Query<J>,
) -> Result<u64> {
	let (filter, _) = links.into_condition();
	let mut condition = filter.unwrap_or_else(Expr::always);

	if let Some(deletable) = crate::policy::condition::<J>(connection.access(), Action::Delete) {
		condition = condition & deletable;
	}

	if let Some(column) = J::DELETED_AT {
		condition = condition
			& Expr::Null {
				column,
				negated: false,
			};
	}

	let rows = crate::write::select_for_update::<J>(connection, &condition).await?;
	let removed = rows.len() as u64;

	for mut row in rows {
		crate::write::delete(connection, &mut row).await?;
	}

	Ok(removed)
}

/// Checks both ends are visible, then writes the link, in one transaction.
async fn add_link<J: Link, Near: HasId, Far: HasId>(
	connection: &impl Connection,
	link: J,
	near: Id<Near>,
	far: Id<Far>,
) -> Result<J> {
	require_visible(connection, near).await?;
	require_visible(connection, far).await?;
	crate::write::upsert(connection, link, &[J::FROM, J::TO]).await
}
