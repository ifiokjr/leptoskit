//! Hooks: your own code, run by every single-row write inside the write's
//! transaction, and who a write is recorded as made by.
//!
//! A table names the hooks it has, and implements their traits:
//!
//! ```ignore
//! #[db::table(name = "orders", hooks(before_write, after_update))]
//! pub struct Order { … }
//!
//! impl db::BeforeWrite for Order {
//! 	fn before_write(&mut self, cx: &db::WriteContext<'_, Self>) -> db::Result<()> {
//! 		self.total = self.lines.iter().map(|line| line.price).sum();
//! 		Ok(())
//! 	}
//! }
//!
//! impl db::AfterUpdate for Order {
//! 	async fn after_update(old: &Self, new: &Self, tx: &impl db::Connection) -> db::Result<()> {
//! 		if old.status != new.status {
//! 			NewOrderEvent { order_id: new.id, status: new.status }.insert(tx).await?;
//! 		}
//! 		Ok(())
//! 	}
//! }
//! ```
//!
//! A hook that fails rolls the whole write back, its own writes included.

use std::future::Future;

use uuid::Uuid;

use crate::Connection;
use crate::Context;
use crate::Id;
use crate::Result;
use crate::Table;
use crate::policy::Access;

/// Who is making a request's changes: the global that `#[db(created_by)]`,
/// `#[db(updated_by)]`, and the audit log read.
///
/// `leptoskit_auth` adds one for every signed-in request. Without auth, add
/// it yourself:
///
/// ```ignore
/// let db = db.with_context(Context::new().with(Actor::of(user.id)));
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Actor(Uuid);

/// Under row-level security, Postgres sees the actor's key as
/// `leptoskit.actor`, so a rule that compares a column with it runs there
/// too.
impl crate::Setting for Actor {
	type Value = String;

	const NAME: &'static str = "actor";

	fn value(&self) -> String {
		self.0.to_string()
	}
}

crate::register_setting!(Actor);

impl Actor {
	/// The actor whose user row has this key.
	pub const fn of<T: ?Sized>(id: Id<T>) -> Self {
		Self(id.uuid())
	}

	/// The actor with this UUID, such as an account's.
	pub const fn from_uuid(uuid: Uuid) -> Self {
		Self(uuid)
	}

	/// The actor's key, in whichever user table it names.
	pub const fn id<T: ?Sized>(&self) -> Id<T> {
		Id::from_uuid(self.0)
	}

	/// The actor's UUID.
	pub const fn uuid(&self) -> Uuid {
		self.0
	}
}

/// A column that can record who wrote a row: `Id<T>`, `Uuid`, or either in
/// an `Option`. `#[db(created_by)]` and `#[db(updated_by)]` columns need
/// one.
pub trait ActorColumn: Sized {
	/// The value recording `actor`, or `None` when the column can't record
	/// that nobody is acting.
	fn from_actor(actor: Option<Actor>) -> Option<Self>;

	/// What the column holds before a write fills it in.
	#[doc(hidden)]
	fn unset() -> Self;
}

impl<T: ?Sized> ActorColumn for Id<T> {
	fn from_actor(actor: Option<Actor>) -> Option<Self> {
		actor.map(|actor| actor.id())
	}

	fn unset() -> Self {
		Self::from_uuid(Uuid::nil())
	}
}

impl ActorColumn for Uuid {
	fn from_actor(actor: Option<Actor>) -> Option<Self> {
		actor.map(|actor| actor.uuid())
	}

	fn unset() -> Self {
		Self::nil()
	}
}

impl<T: ActorColumn> ActorColumn for Option<T> {
	fn from_actor(actor: Option<Actor>) -> Option<Self> {
		match actor {
			Some(_) => T::from_actor(actor).map(Some),
			None => Some(None),
		}
	}

	fn unset() -> Self {
		None
	}
}

/// Which hooks a table declares with `hooks(...)`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[expect(
	clippy::struct_excessive_bools,
	reason = "one flag per hook a table may declare"
)]
pub struct Hooks {
	/// [`BeforeWrite`]: runs before every insert and update.
	pub before_write: bool,
	/// [`AfterInsert`].
	pub after_insert: bool,
	/// [`AfterUpdate`].
	pub after_update: bool,
	/// [`AfterDelete`].
	pub after_delete: bool,
}

impl Hooks {
	/// No hooks.
	pub const NONE: Self = Self {
		before_write: false,
		after_insert: false,
		after_update: false,
		after_delete: false,
	};

	/// Whether a bulk update would skip any: it never loads the rows it
	/// changes.
	pub const fn on_update(self) -> bool {
		self.before_write || self.after_update
	}
}

/// Whether a write inserts or changes a row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
	Insert,
	Update,
}

/// What a [`BeforeWrite`] hook knows about the write: whether it inserts or
/// changes a row, the row as stored before a change, and who is writing.
pub struct WriteContext<'a, T> {
	kind: Kind,
	old: Option<&'a T>,
	access: &'a Access,
}

impl<'a, T> WriteContext<'a, T> {
	pub(crate) fn insert(access: &'a Access) -> Self {
		Self {
			kind: Kind::Insert,
			old: None,
			access,
		}
	}

	pub(crate) fn update(access: &'a Access, old: &'a T) -> Self {
		Self {
			kind: Kind::Update,
			old: Some(old),
			access,
		}
	}

	/// Whether the write inserts a new row.
	pub fn is_insert(&self) -> bool {
		self.kind == Kind::Insert
	}

	/// Whether the write changes an existing row.
	pub fn is_update(&self) -> bool {
		self.kind == Kind::Update
	}

	/// The row as stored before this change, or `None` for an insert.
	pub fn old(&self) -> Option<&'a T> {
		self.old
	}

	/// The request's context, or `None` when the write bypasses policies.
	pub fn context(&self) -> Option<&'a Context> {
		match self.access {
			Access::Context(cx) => Some(cx),
			Access::System { .. } => None,
		}
	}

	/// Who is writing, if anyone.
	pub fn actor(&self) -> Option<Actor> {
		self.access.actor()
	}

	/// Why the write bypasses policies, when it does.
	pub fn reason(&self) -> Option<&'static str> {
		self.access.reason()
	}
}

/// Changes a row before every insert and update written one row at a time:
/// `insert`, `upsert`, `save`, and adding a link. Declare it with
/// `hooks(before_write)`. The row is validated after the hook runs, and the
/// database layer's own columns (`created_by`, `updated_by`) are filled in
/// again after it, so a hook can read them but can't forge them.
#[diagnostic::on_unimplemented(message = "`{Self}` declares `hooks(before_write)`, so it needs \
                                          `impl db::BeforeWrite for {Self}`")]
pub trait BeforeWrite: Table {
	/// Changes the row about to be written, or refuses the write.
	fn before_write(&mut self, cx: &WriteContext<'_, Self>) -> Result<()>;
}

/// Runs after a row is inserted, in the same transaction. Declare it with
/// `hooks(after_insert)`.
#[diagnostic::on_unimplemented(message = "`{Self}` declares `hooks(after_insert)`, so it needs \
                                          `impl db::AfterInsert for {Self}`")]
pub trait AfterInsert: Table {
	/// Reacts to `self`, as stored. Writes through `tx` join the insert's
	/// transaction.
	fn after_insert(&self, tx: &impl Connection) -> impl Future<Output = Result<()>> + Send;
}

/// Runs after a row is changed, in the same transaction. Declare it with
/// `hooks(after_update)`. A soft delete is a delete, so it runs
/// [`AfterDelete`] instead.
#[diagnostic::on_unimplemented(message = "`{Self}` declares `hooks(after_update)`, so it needs \
                                          `impl db::AfterUpdate for {Self}`")]
pub trait AfterUpdate: Table {
	/// Reacts to the change from `old` to `new`, both as stored. Writes
	/// through `tx` join the update's transaction.
	fn after_update(
		old: &Self,
		new: &Self,
		tx: &impl Connection,
	) -> impl Future<Output = Result<()>> + Send;
}

/// Runs after a row is deleted, soft or for good, in the same transaction.
/// Declare it with `hooks(after_delete)`.
#[diagnostic::on_unimplemented(message = "`{Self}` declares `hooks(after_delete)`, so it needs \
                                          `impl db::AfterDelete for {Self}`")]
pub trait AfterDelete: Table {
	/// Reacts to `self`, as the delete left it: removed, or with its
	/// `deleted_at` set. Writes through `tx` join the delete's transaction.
	fn after_delete(&self, tx: &impl Connection) -> impl Future<Output = Result<()>> + Send;
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn actor_columns_record_whoever_acts() {
		let actor = Actor::from_uuid(Uuid::from_u128(7));
		let id: Id<()> = Id::from_uuid(Uuid::from_u128(7));

		assert_eq!(Actor::of(id), actor);
		assert_eq!(actor.uuid(), Uuid::from_u128(7));
		assert_eq!(<Id<()>>::from_actor(Some(actor)), Some(id));
		assert_eq!(<Id<()>>::from_actor(None), None);
		assert_eq!(<Option<Id<()>>>::from_actor(Some(actor)), Some(Some(id)));
		assert_eq!(<Option<Id<()>>>::from_actor(None), Some(None));
		assert_eq!(Uuid::from_actor(Some(actor)), Some(Uuid::from_u128(7)));
		assert_eq!(Uuid::from_actor(None), None);

		assert_eq!(<Id<()>>::unset().uuid(), Uuid::nil());
		assert_eq!(Uuid::unset(), Uuid::nil());
		assert_eq!(<Option<Uuid>>::unset(), None);
	}

	#[test]
	fn hooks_say_what_bulk_writes_skip() {
		assert!(!Hooks::NONE.on_update());

		let after_delete = Hooks {
			after_delete: true,
			..Hooks::NONE
		};
		assert!(!after_delete.on_update());
		assert!(
			Hooks {
				before_write: true,
				..Hooks::NONE
			}
			.on_update()
		);
		assert!(
			Hooks {
				after_update: true,
				..Hooks::NONE
			}
			.on_update()
		);
	}

	#[test]
	fn write_contexts_describe_the_write() {
		let actor = Actor::from_uuid(Uuid::from_u128(1));
		let request = Access::Context(Context::new().with(actor));
		let insert = WriteContext::<u8>::insert(&request);
		assert!(insert.is_insert() && !insert.is_update());
		assert_eq!(insert.old(), None);
		assert_eq!(insert.actor(), Some(actor));
		assert!(insert.context().is_some());
		assert_eq!(insert.reason(), None);

		let system = Access::System {
			reason: "a unit test",
			actor: None,
		};
		let old = 3;
		let update = WriteContext::update(&system, &old);
		assert!(update.is_update() && !update.is_insert());
		assert_eq!(update.old(), Some(&3));
		assert_eq!(update.actor(), None);
		assert!(update.context().is_none());
		assert_eq!(update.reason(), Some("a unit test"));
	}
}
