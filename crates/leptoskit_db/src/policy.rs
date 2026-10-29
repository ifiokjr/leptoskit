//! Access policies, and the request context they read.
//!
//! The semantics are Gel's:
//!
//! - **No policies.** A table with none is open to server code.
//! - **Default deny.** Once a table has any policy, every action is denied
//!   unless an `allow` matches.
//! - **`deny` wins.** A matching `deny` overrides every `allow`.
//! - **Silent filters.** Reads, updates and deletes skip rows you may not
//!   touch, so they simply aren't there.
//! - **Checked results.** An insert or update whose result isn't allowed fails
//!   with [`DbError::Forbidden`](crate::DbError::Forbidden).
//!
//! Policies run for every connection made from a [`Db`](crate::Db), which acts
//! for an empty [`Context`] until [`Db::with_context`](crate::Db::with_context)
//! gives it one. Only [`Db::system`](crate::Db::system) skips them, explicitly.

use std::any::Any;
use std::any::TypeId;
use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;

use crate::Connection;
use crate::DbError;
use crate::Expr;
use crate::Result;
use crate::Table;
use crate::Value;
use crate::expr::Comparison;
use crate::sql::Sql;

/// What a request knows about itself: who is signed in, which tenant it
/// works in, and anything else policies need. Build one per request:
///
/// ```ignore
/// let cx = Context::new().with(Viewer(session.user_id));
/// let db = db.with_context(cx);
/// ```
///
/// Each value is a *global*, found by its type: any `Send + Sync` type
/// works, so a newtype such as `struct Tenant(Id<Organization>)` keeps one
/// from being mistaken for another.
#[derive(Clone, Default)]
pub struct Context {
	globals: Arc<HashMap<TypeId, Arc<dyn Any + Send + Sync>>>,
}

impl Context {
	/// A context with no globals: policies that read one match no rows.
	pub fn new() -> Self {
		Self::default()
	}

	/// The context with `global` added, replacing any global of its type.
	#[must_use]
	pub fn with<G: Send + Sync + 'static>(mut self, global: G) -> Self {
		Arc::make_mut(&mut self.globals).insert(TypeId::of::<G>(), Arc::new(global));
		self
	}

	/// The global of type `G`, if the request has one.
	pub fn get<G: 'static>(&self) -> Option<&G> {
		self.globals
			.get(&TypeId::of::<G>())
			.and_then(|global| global.downcast_ref())
	}

	/// A condition on a global: `condition`'s when the request has a `G`,
	/// and [`Expr::never`] when it doesn't, so a rule about the signed-in
	/// user matches nothing for a signed-out request.
	///
	/// ```ignore
	/// |post, cx| cx.global(|viewer: &Viewer| post.author_id.eq(viewer.0))
	/// ```
	pub fn global<G: 'static>(&self, condition: impl FnOnce(&G) -> Expr) -> Expr {
		self.get().map_or_else(Expr::never, condition)
	}
}

impl fmt::Debug for Context {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		f.debug_struct("Context")
			.field("globals", &self.globals.len())
			.finish()
	}
}

/// What a policy governs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Action {
	/// Reading rows, including through links and counts.
	Select,
	/// Inserting rows, checked on the stored result.
	Insert,
	/// Changing rows: which ones may change, and what they may become.
	Update,
	/// Deleting rows, soft or for good.
	Delete,
}

impl Action {
	/// Every action, for `#[db::policy(all, …)]`.
	pub const ALL: &'static [Self] = &[Self::Select, Self::Insert, Self::Update, Self::Delete];

	fn verb(self) -> &'static str {
		match self {
			Self::Select => "read",
			Self::Insert => "insert",
			Self::Update => "change",
			Self::Delete => "delete",
		}
	}
}

/// One rule of a table's access policy. `#[db::policy]` declares them.
pub struct Policy<T: Table> {
	/// The actions it governs.
	pub actions: &'static [Action],
	/// Whether matching rows are denied rather than allowed.
	pub deny: bool,
	/// The rows it matches, given the request.
	pub rule: fn(&T::Columns, &Context) -> Expr,
	/// What a refused write says, instead of a generic message.
	pub message: Option<&'static str>,
}

/// Who a connection acts for.
#[doc(hidden)]
#[derive(Clone, Debug)]
pub enum Access {
	/// A request, whose context policies read.
	Context(Context),
	/// Server code that bypasses policies, and says why.
	System {
		/// Why: for logs and audits.
		reason: &'static str,
	},
}

impl Default for Access {
	fn default() -> Self {
		Self::Context(Context::default())
	}
}

/// The condition a row must meet for `action`, or `None` when nothing
/// restricts it: the table has no policies, or the connection bypasses
/// them.
pub(crate) fn condition<T: Table>(access: &Access, action: Action) -> Option<Expr> {
	let policies = T::policies();

	if policies.is_empty() {
		return None;
	}

	let Access::Context(cx) = access else {
		return None;
	};

	Some(evaluate(policies, cx, action))
}

fn evaluate<T: Table>(policies: &[Policy<T>], cx: &Context, action: Action) -> Expr {
	let columns = T::columns();
	let matching = |deny: bool| {
		policies
			.iter()
			.filter(move |policy| policy.deny == deny && policy.actions.contains(&action))
			.map(|policy| (policy.rule)(columns, cx))
			.reduce(|left, right| left | right)
	};
	let allowed = matching(false).unwrap_or_else(Expr::never);

	match matching(true) {
		Some(denied) => allowed & !denied,
		None => allowed,
	}
}

/// The rules a refused write is checked against to explain itself: each
/// `deny` that matched, then the `allow`s, in declaration order.
pub(crate) fn explanations<T: Table>(
	access: &Access,
	action: Action,
) -> Vec<(Expr, Option<&'static str>, bool)> {
	let Access::Context(cx) = access else {
		return Vec::new();
	};
	let columns = T::columns();
	let policies = T::policies()
		.iter()
		.filter(|policy| policy.actions.contains(&action));
	let (denies, allows): (Vec<_>, Vec<_>) = policies.partition(|policy| policy.deny);

	denies
		.into_iter()
		.chain(allows)
		.map(|policy| ((policy.rule)(columns, cx), policy.message, policy.deny))
		.collect()
}

/// The rows with these keys, each key in [`Table::KEY`] order.
pub(crate) fn keys_condition<T: Table>(keys: &[Vec<Value>]) -> Expr {
	if let [column] = T::KEY {
		return Expr::In {
			column,
			values: keys.iter().map(|key| key[0].clone()).collect(),
		};
	}

	keys.iter()
		.map(|key| {
			T::KEY
				.iter()
				.zip(key)
				.map(|(column, value)| {
					Expr::Compare {
						column,
						comparison: Comparison::Equal,
						value: value.clone(),
					}
				})
				.reduce(|left, right| left & right)
				.unwrap_or_else(Expr::never)
		})
		.reduce(|left, right| left | right)
		.unwrap_or_else(Expr::never)
}

/// A row's key values, from its values in [`Table::COLUMNS`] order.
pub(crate) fn key_of<T: Table>(values: &[Value]) -> Vec<Value> {
	T::KEY
		.iter()
		.map(|key| {
			let position = T::COLUMNS
				.iter()
				.position(|column| column == key)
				.expect("a key is a column");
			values[position].clone()
		})
		.collect()
}

/// How many rows meet `condition`, soft-deleted or not, without any policy:
/// the checks below decide what to reveal.
pub(crate) async fn count<T: Table>(connection: &impl Connection, condition: &Expr) -> Result<u64> {
	let mut sql = Sql::new(connection.dialect());
	sql.push("SELECT count(*) FROM ")
		.table::<T>()
		.push(" WHERE ");
	condition.write(&mut sql)?;

	crate::sql::count_of(&connection.fetch(sql.finish()).await?)
}

/// Checks that the rows with these keys, as stored now, meet `condition`.
/// Counting the rows that do, rather than looking for rows that don't, keeps
/// a rule that evaluates to `NULL` from letting a row through.
pub(crate) async fn verify<T: Table>(
	connection: &impl Connection,
	keys: &[Vec<Value>],
	action: Action,
	condition: &Expr,
) -> Result<()> {
	if keys.is_empty() {
		return Ok(());
	}

	let rows = keys_condition::<T>(keys);
	let allowed = count::<T>(connection, &(rows.clone() & condition.clone())).await?;

	if allowed == keys.len() as u64 {
		Ok(())
	} else {
		Err(refuse::<T>(connection, &rows, action).await?)
	}
}

/// The refusal of `action` on the rows `rows` matches: the message of the
/// first `deny` that matched, else of the first `allow` that has one, else a
/// generic one.
pub(crate) async fn refuse<T: Table>(
	connection: &impl Connection,
	rows: &Expr,
	action: Action,
) -> Result<DbError> {
	let explanations = explanations::<T>(connection.access(), action);

	for (rule, message, deny) in &explanations {
		if *deny && count::<T>(connection, &(rows.clone() & rule.clone())).await? > 0 {
			return Ok(forbidden::<T>(*message, action));
		}
	}

	let message = explanations
		.iter()
		.find_map(|(_, message, deny)| message.filter(|_| !deny));
	Ok(forbidden::<T>(message, action))
}

/// Why a write to one row changed nothing: it isn't there for this
/// request, it was versioned and changed since, or a policy refused it.
pub(crate) async fn missing<T: Table>(
	connection: &impl Connection,
	values: &[Value],
	action: Action,
	versioned: bool,
) -> Result<DbError> {
	let row = keys_condition::<T>(&[key_of::<T>(values)]);
	// A row the request may not see is not found, whether or not it exists.
	let visible = match condition::<T>(connection.access(), Action::Select) {
		Some(select) => row.clone() & select,
		None => row.clone(),
	};

	if count::<T>(connection, &visible).await? == 0 {
		return Ok(DbError::NotFound {
			table: T::flat_name(),
		});
	}

	if let Some(version) = T::VERSION.filter(|_| versioned) {
		let position = T::COLUMNS
			.iter()
			.position(|column| *column == version)
			.expect("the version is a column");
		let unchanged = row.clone()
			& Expr::Compare {
				column: version,
				comparison: Comparison::Equal,
				value: values[position].clone(),
			};

		if count::<T>(connection, &unchanged).await? == 0 {
			return Ok(DbError::Stale {
				table: T::flat_name(),
			});
		}
	}

	refuse::<T>(connection, &row, action).await
}

fn forbidden<T: Table>(message: Option<&'static str>, action: Action) -> DbError {
	DbError::Forbidden {
		table: T::flat_name(),
		message: message.map_or_else(
			|| {
				format!(
					"you may not {} this `{}` row",
					action.verb(),
					T::flat_name()
				)
			},
			ToString::to_string,
		),
	}
}

#[cfg(test)]
mod tests {
	use leptoskit_db_macros::policy;

	use super::*;
	use crate::table;

	/// A pair keyed by both its halves, which only positive pairs may join.
	#[table(name = "pairs")]
	#[policy(insert, |pair, _| pair.left.gt(0), message = "Pairs start above zero")]
	pub struct Pair {
		#[db(key)]
		pub left: i32,
		#[db(key)]
		pub right: i32,
	}

	#[test]
	fn composite_keys_match_each_row_by_every_column() {
		let pair = |left: i64, right: i64| {
			Expr::Compare {
				column: "left",
				comparison: Comparison::Equal,
				value: Value::Int(left),
			} & Expr::Compare {
				column: "right",
				comparison: Comparison::Equal,
				value: Value::Int(right),
			}
		};
		let keys = [
			vec![Value::Int(1), Value::Int(2)],
			vec![Value::Int(3), Value::Int(4)],
		];

		assert_eq!(keys_condition::<Pair>(&keys), pair(1, 2) | pair(3, 4));
		assert_eq!(keys_condition::<Pair>(&[]), Expr::never());
		assert_eq!(key_of::<Pair>(&keys[1]), keys[1]);
	}

	#[test]
	fn bypassing_access_has_no_conditions_or_explanations() {
		let system = Access::System {
			reason: "a unit test",
		};
		assert!(condition::<Pair>(&system, Action::Insert).is_none());
		assert!(explanations::<Pair>(&system, Action::Insert).is_empty());

		let request = Access::default();
		// No rule allows reading pairs, so no row may be read.
		assert_eq!(
			condition::<Pair>(&request, Action::Select),
			Some(Expr::never())
		);
		let explained = explanations::<Pair>(&request, Action::Insert);
		assert_eq!(explained.len(), 1);
		assert_eq!(explained[0].1, Some("Pairs start above zero"));
	}

	#[test]
	fn refusals_without_a_message_name_the_action_and_table() {
		let message = |action| forbidden::<Pair>(None, action).to_string();
		assert_eq!(message(Action::Select), "you may not read this `pairs` row");
		assert_eq!(
			message(Action::Insert),
			"you may not insert this `pairs` row"
		);
		assert_eq!(
			message(Action::Update),
			"you may not change this `pairs` row"
		);
		assert_eq!(
			message(Action::Delete),
			"you may not delete this `pairs` row"
		);
		assert_eq!(
			forbidden::<Pair>(Some("Pairs start above zero"), Action::Insert).to_string(),
			"Pairs start above zero"
		);
		assert_eq!(Action::ALL.len(), 4);
	}
}
