//! Polymorphic tables: a Rust enum whose variants are tables, stored with
//! class-table inheritance.
//!
//! ```ignore
//! /// Anything that can own a budget.
//! #[db::polymorphic(name = "actors", public)]
//! pub enum Actor {
//! 	User(User),
//! 	Group(Group),
//! }
//!
//! #[db::table(name = "users", public, extends = Actor)]
//! pub struct User { pub id: Id<Self>, pub handle: String }
//!
//! #[db::table(name = "groups", public, extends = Actor)]
//! pub struct Group { pub id: Id<Self>, pub title: String }
//!
//! #[db::table(name = "budgets", public)]
//! pub struct Budget { pub id: Id<Self>, pub owner_id: Id<Actor>, pub name: String }
//!
//! match budget.owner(&db).await? {
//! 	Actor::User(user) => …,
//! 	Actor::Group(group) => …,
//! }
//! ```
//!
//! **Storage.** `actors(id, kind)` holds one row per variant row, and each
//! variant's key is also a foreign key to it, so `Id<Actor>` columns are
//! ordinary foreign keys. Inserting a `User` inserts its `actors` row first,
//! in the same transaction; removing it removes that row after it. Nothing
//! runs in the database: there are no triggers.
//!
//! **Reading through the supertype** runs one query per variant table,
//! each through that table's own policies and soft deletes, ordered by key
//! and merged. Loading one `Id<Actor>` reads its `actors` row for the kind,
//! then that variant's row: two queries.
//!
//! **Policies.** The supertype's `#[db::policy]` rules apply to every
//! variant: a variant row is visible only when its supertype row is, and
//! changing or deleting one needs the supertype row to allow that too.

use std::fmt;
use std::hash::Hash;
use std::marker::PhantomData;

use leptoskit_db_schema::TableName;
use uuid::Uuid;

use crate::Action;
use crate::Col;
use crate::Connection;
use crate::Expr;
use crate::HasId;
use crate::Id;
use crate::Query;
use crate::Read;
use crate::Result;
use crate::Table;
use crate::Value;
use crate::policy::Access;
use crate::policy::Explanation;
use crate::policy::Subquery;

/// A polymorphic table: an enum with one variant per table, declared with
/// `#[db::polymorphic]`, which implements it. Match on it, exhaustively, to
/// handle each kind of row.
pub trait Polymorphic: crate::Referenced + Sized + Send + 'static {
	/// The supertype table, holding each variant row's key and kind:
	/// generated as `{Enum}Row`, keyed by `Id<Self>`.
	type Row: Table;

	/// Which variant a row is: generated as `{Enum}Kind`, stored as text.
	type Kind: Copy + Eq + Hash + fmt::Debug + fmt::Display + Send + Sync + 'static;

	/// Every kind, in declaration order.
	const KINDS: &'static [Self::Kind];

	/// The row's key, which its variant table and the supertype share.
	fn id(&self) -> Id<Self>;

	/// Which variant the row is.
	fn kind(&self) -> Self::Kind;

	/// The variant table each kind is stored in.
	fn table_of(kind: Self::Kind) -> TableName;

	/// The supertype row for a variant row.
	#[doc(hidden)]
	fn row(id: Id<Self>, kind: Self::Kind) -> Self::Row;

	/// The kind a supertype row records.
	#[doc(hidden)]
	fn row_kind(row: &Self::Row) -> Self::Kind;

	/// The rows of one variant on a page, as the enum.
	#[doc(hidden)]
	fn fetch(
		connection: &impl Read,
		kind: Self::Kind,
		page: &Page,
	) -> impl Future<Output = Result<Vec<Self>>> + Send;

	/// How many rows of one variant are on a page, ignoring its limit.
	#[doc(hidden)]
	fn count(
		connection: &impl Read,
		kind: Self::Kind,
		page: &Page,
	) -> impl Future<Output = Result<u64>> + Send;
}

/// A table that is one variant of a polymorphic table:
/// `#[db::table(extends = Actor)]`, listed in `Actor`'s enum.
/// `#[db::polymorphic]` implements it.
#[diagnostic::on_unimplemented(
	message = "`{Self}` isn't a variant of a polymorphic table",
	note = "list it in the `#[db::polymorphic]` enum it extends"
)]
pub trait Variant: Table + HasId {
	/// The polymorphic table it's a variant of.
	type Of: Polymorphic;

	/// Its kind.
	const KIND: <Self::Of as Polymorphic>::Kind;

	/// The row, as the enum.
	fn into_supertype(self) -> Self::Of;
}

impl<V: Variant> Id<V> {
	/// The same key as the polymorphic table's: every `User` is an
	/// `Actor`. Rust's coherence rules keep this from being `From`.
	pub fn upcast(self) -> Id<V::Of> {
		Id::from_uuid(self.uuid())
	}
}

impl<P: Polymorphic> Id<P> {
	/// The `V` row with this key, if the key is one of `V`'s and the
	/// request may see it.
	pub async fn downcast<V: Variant<Of = P>>(self, connection: &impl Read) -> Result<Option<V>> {
		crate::links::by_id(Id::<V>::from_uuid(self.uuid()))
			.first(connection)
			.await
	}
}

/// The rows of a polymorphic table, of every variant or some: start one
/// with `Actor::query()`.
///
/// It runs one query per variant table, each with that table's policies
/// and soft deletes, ordered by key, which for generated keys is the order
/// rows were created in. A limit or an offset applies to the merged rows.
#[must_use = "a query does nothing until it runs"]
pub struct Variants<P: Polymorphic> {
	kinds: Option<Vec<P::Kind>>,
	page: Page,
	offset: u64,
	types: PhantomData<fn() -> P>,
}

/// What a variant table's query keeps: the parts of a [`Variants`] query
/// each table runs on its own.
#[doc(hidden)]
#[derive(Clone, Debug, Default)]
pub struct Page {
	ids: Option<Vec<Uuid>>,
	after: Option<Uuid>,
	descending: bool,
	limit: Option<u64>,
	with_deleted: bool,
}

impl<P: Polymorphic> Default for Variants<P> {
	fn default() -> Self {
		Self::new()
	}
}

impl<P: Polymorphic> Variants<P> {
	/// Every row of every variant.
	pub fn new() -> Self {
		Self {
			kinds: None,
			page: Page::default(),
			offset: 0,
			types: PhantomData,
		}
	}

	/// Only rows of these kinds.
	pub fn only(mut self, kinds: impl IntoIterator<Item = P::Kind>) -> Self {
		let mut only: Vec<P::Kind> = Vec::new();

		for kind in kinds {
			if !only.contains(&kind) {
				only.push(kind);
			}
		}

		self.kinds = Some(only);
		self
	}

	/// Only the rows with these keys.
	pub fn ids(mut self, ids: impl IntoIterator<Item = Id<P>>) -> Self {
		self.page.ids = Some(ids.into_iter().map(|id| id.uuid()).collect());
		self
	}

	/// Only the rows after this key, in the query's order: the next page
	/// after the last row of this one.
	pub fn after(mut self, id: Id<P>) -> Self {
		self.page.after = Some(id.uuid());
		self
	}

	/// Newest first: in descending key order.
	pub fn descending(mut self) -> Self {
		self.page.descending = true;
		self
	}

	/// At most this many rows.
	pub fn limit(mut self, limit: u64) -> Self {
		self.page.limit = Some(limit);
		self
	}

	/// Skips this many rows.
	pub fn offset(mut self, offset: u64) -> Self {
		self.offset = offset;
		self
	}

	/// Includes soft-deleted rows.
	pub fn with_deleted(mut self) -> Self {
		self.page.with_deleted = true;
		self
	}

	fn kinds(&self) -> &[P::Kind] {
		self.kinds.as_deref().unwrap_or(P::KINDS)
	}

	/// Every matching row, as the enum.
	pub async fn all(self, connection: &impl Read) -> Result<Vec<P>> {
		let limit = self.page.limit;
		// Each table's first `offset + limit` rows hold the merged page.
		let page = Page {
			limit: limit.map(|limit| limit.saturating_add(self.offset)),
			..self.page.clone()
		};
		let mut rows = Vec::new();

		for kind in self.kinds() {
			rows.extend(P::fetch(connection, *kind, &page).await?);
		}

		rows.sort_by_key(Polymorphic::id);

		if self.page.descending {
			rows.reverse();
		}

		let skipped = usize::try_from(self.offset).unwrap_or(usize::MAX);
		let taken = limit.map_or(usize::MAX, |limit| {
			usize::try_from(limit).unwrap_or(usize::MAX)
		});
		Ok(rows.into_iter().skip(skipped).take(taken).collect())
	}

	/// The first matching row, if any.
	pub async fn first(self, connection: &impl Read) -> Result<Option<P>> {
		Ok(self.limit(1).all(connection).await?.into_iter().next())
	}

	/// How many rows match, ignoring the limit and offset.
	pub async fn count(self, connection: &impl Read) -> Result<u64> {
		let mut total = 0;

		for kind in self.kinds() {
			total += P::count(connection, *kind, &self.page).await?;
		}

		Ok(total)
	}
}

/// A variant table's query for a page: its keys, the rows after a key, and
/// soft-deleted rows when asked for.
fn variant_query<V: Variant>(page: &Page) -> Query<V> {
	let key = Col::<V, Id<V>>::new(V::KEY[0]);
	let mut query = Query::<V>::new();

	if let Some(ids) = &page.ids {
		let ids: Vec<Id<V>> = ids.iter().copied().map(Id::from_uuid).collect();
		query = query.filter(|_| key.in_list(ids));
	}

	if let Some(after) = page.after {
		let after = Id::<V>::from_uuid(after);
		query = query.filter(|_| {
			if page.descending {
				key.lt(after)
			} else {
				key.gt(after)
			}
		});
	}

	if page.with_deleted {
		query = query.with_deleted();
	}

	query
}

/// One variant's rows on a page, in key order, as the enum.
#[doc(hidden)]
pub async fn fetch_variant<V: Variant>(connection: &impl Read, page: &Page) -> Result<Vec<V::Of>> {
	let key = Col::<V, Id<V>>::new(V::KEY[0]);
	let mut query = variant_query::<V>(page).order_by(|_| {
		if page.descending {
			key.desc()
		} else {
			key.asc()
		}
	});

	if let Some(limit) = page.limit {
		query = query.limit(limit);
	}

	Ok(query
		.all(connection)
		.await?
		.into_iter()
		.map(V::into_supertype)
		.collect())
}

/// How many of one variant's rows are on a page.
#[doc(hidden)]
pub async fn count_variant<V: Variant>(connection: &impl Read, page: &Page) -> Result<u64> {
	variant_query::<V>(page).count(connection).await
}

/// The row with this key, of whichever variant it is: its supertype row
/// says which, then that variant's table has it.
#[doc(hidden)]
pub async fn load<P: Polymorphic>(connection: &impl Read, id: Id<P>) -> Result<Option<P>> {
	let key = Col::<P::Row, Id<P>>::new(P::Row::KEY[0]);
	let Some(row) = Query::<P::Row>::new()
		.filter(|_| key.eq(id))
		.point()
		.first(connection)
		.await?
	else {
		return Ok(None);
	};
	let page = Page {
		ids: Some(vec![id.uuid()]),
		..Page::default()
	};

	Ok(P::fetch(connection, P::row_kind(&row), &page)
		.await?
		.into_iter()
		.next())
}

/// Inserts the supertype row for a variant row about to be inserted, with
/// the supertype's own policies.
#[doc(hidden)]
pub async fn write_supertype<V: Variant<Of = P>, P: Polymorphic>(
	connection: &impl Connection,
	id: Id<V>,
) -> Result<()> {
	let row = P::row(id.upcast(), V::KIND);

	#[cfg(feature = "postgres")]
	if connection.row_security() {
		return crate::write::insert_unreturned(connection, row).await;
	}

	crate::write::insert(connection, row).await?;
	Ok(())
}

/// Removes the supertype rows of variant rows just removed. Each must go:
/// one a policy keeps would be left pointing at nothing, so that fails
/// instead.
#[doc(hidden)]
pub async fn remove_supertypes<V: Variant<Of = P>, P: Polymorphic>(
	connection: &impl Connection,
	keys: Vec<Value>,
) -> Result<()> {
	if keys.is_empty() {
		return Ok(());
	}

	let expected = keys.len() as u64;
	let rows = Expr::In {
		column: P::Row::KEY[0],
		values: keys,
	};
	let filter = rows.clone();
	let removed = Query::<P::Row>::new()
		.filter(|_| filter)
		.destroy(connection)
		.await?;

	if removed == expected {
		Ok(())
	} else {
		Err(crate::policy::refuse::<P::Row>(connection, &rows, Action::Delete).await?)
	}
}

/// What a variant table knows of its supertype, as plain functions, so
/// that policies and soft deletes can follow it from any variant.
#[doc(hidden)]
#[derive(Clone, Copy, Debug)]
pub struct Supertype {
	pub(crate) table: fn() -> TableName,
	key: &'static str,
	condition: fn(&Access, Action) -> Option<Expr>,
	explanations: fn(&Access, Action) -> Vec<Explanation>,
}

impl Supertype {
	/// The supertype stored in table `R`.
	pub const fn of<R: Table>() -> Self {
		Self {
			table: R::table_name,
			key: R::KEY[0],
			condition: crate::policy::condition::<R>,
			explanations: crate::policy::explanations::<R>,
		}
	}

	/// The variant rows, by their key `column`, whose supertype row meets
	/// `rule`.
	fn within(&self, column: &'static str, rule: Expr) -> Expr {
		Expr::Within {
			column,
			selected: self.key,
			rows: Box::new(Subquery::rows_of((self.table)(), rule)),
		}
	}
}

/// What `T`'s supertype's policy requires of a `T` row for `action`, or
/// `None` when nothing does. Changing a variant row also needs its
/// supertype row to be visible, as Postgres row-level security would
/// require of the subquery.
pub(crate) fn inherited<T: Table>(access: &Access, action: Action) -> Option<Expr> {
	let supertype = T::SUPERTYPE?;
	let rule = (supertype.condition)(access, action)?;
	let rule = match action {
		Action::Select => rule,
		_ => {
			match (supertype.condition)(access, Action::Select) {
				Some(select) => select & rule,
				None => rule,
			}
		}
	};

	Some(supertype.within(T::KEY[0], rule))
}

/// The supertype's rules for `action`, each as a rule on `T`'s rows, to
/// explain a refusal.
pub(crate) fn inherited_explanations<T: Table>(
	access: &Access,
	action: Action,
) -> Vec<Explanation> {
	let Some(supertype) = T::SUPERTYPE else {
		return Vec::new();
	};

	(supertype.explanations)(access, action)
		.into_iter()
		.map(|(rule, message, deny)| (supertype.within(T::KEY[0], rule), message, deny))
		.collect()
}
