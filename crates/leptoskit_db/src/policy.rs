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
use std::cell::RefCell;
use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;

use leptoskit_db_schema::TableName;

use crate::Connection;
use crate::DbError;
use crate::Expr;
use crate::Query;
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
	/// What the rules read, when they're being rendered as row-level
	/// security rather than evaluated for a request.
	symbolic: Option<Arc<std::sync::Mutex<Symbolic>>>,
}

/// What rules read while being rendered for row-level security.
#[derive(Default)]
struct Symbolic {
	/// Globals read as Rust values, which the database can't see.
	opaque: Vec<&'static str>,
	/// Settings read, by type.
	settings: Vec<(TypeId, &'static str, &'static str)>,
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

	/// Who is making the request's changes: the [`Actor`](crate::Actor)
	/// global, which `#[db(created_by)]`, `#[db(updated_by)]` and the audit
	/// log read. `leptoskit_auth` adds one for a signed-in request.
	pub fn actor(&self) -> Option<crate::Actor> {
		self.get::<crate::Actor>().copied()
	}

	/// The global of type `G`, if the request has one.
	///
	/// A rule that reads a global this way, or through [`Context::global`],
	/// runs Rust code with its value, so it can't become a row-level
	/// security policy; [`Context::setting`] can.
	pub fn get<G: 'static>(&self) -> Option<&G> {
		if let Some(symbolic) = &self.symbolic {
			lock(symbolic).opaque.push(std::any::type_name::<G>());
			return None;
		}

		self.globals
			.get(&TypeId::of::<G>())
			.and_then(|global| global.downcast_ref())
	}

	/// The value of setting `G`, for a rule to compare a column with:
	/// `post.author_id.eq(cx.setting::<Viewer>())`. Without one, the
	/// comparison matches no row, even negated, as SQL's `NULL` does.
	///
	/// Unlike [`Context::global`], the database can read a setting too, so
	/// rules written with settings also run as Postgres row-level security
	/// (`[db] policies = "rls"`). Register each setting with
	/// [`register_setting!`](crate::register_setting).
	pub fn setting<G: crate::Setting>(&self) -> crate::SettingValue<G::Value> {
		if let Some(symbolic) = &self.symbolic {
			lock(symbolic)
				.settings
				.push((TypeId::of::<G>(), G::NAME, std::any::type_name::<G>()));
		}

		crate::SettingValue::of::<G>(self)
	}

	/// The global behind setting `G`, if the request has one.
	pub(crate) fn setting_global<G: 'static>(&self) -> Option<&G> {
		if self.symbolic.is_some() {
			return None;
		}

		self.globals
			.get(&TypeId::of::<G>())
			.and_then(|global| global.downcast_ref())
	}

	/// A context that records what rules read instead of holding values,
	/// for rendering them as row-level security.
	fn symbolic() -> Self {
		Self {
			globals: Arc::default(),
			symbolic: Some(Arc::default()),
		}
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

	/// How many rows `query` finds, for a rule to compare with a number. It
	/// counts what this request may see, as `query.count(&db)` would: the
	/// counted table's own `select` policy applies, and soft-deleted rows are
	/// left out unless the query asks for them. The one exception is a count
	/// inside the counted table's own `select` rules, such as "members see
	/// the members of their teams": applying the rule there would never end,
	/// so it counts every row.
	///
	/// Inside the query's filter, a column of the row the policy checks
	/// follows a link back to it, so "a member of the team that owns this
	/// project" is:
	///
	/// ```ignore
	/// |project, cx| cx.global(|viewer: &Viewer| {
	/// 	cx.count(Member::query().filter(|member| {
	/// 		member.team_id.eq(project.team_id) & member.user_id.eq(viewer.0)
	/// 	}))
	/// 	.gt(0)
	/// })
	/// ```
	///
	/// When the query's table is the policy's own, its columns on the right
	/// of a comparison are the checked row's:
	/// `post.author_id.eq(row.author_id)` counts the posts that share the
	/// checked post's author.
	pub fn count<U: Table>(&self, query: Query<U>) -> Count {
		let (filter, with_deleted) = query.into_condition();

		// Inside `U`'s own read policy, its counts of `U` can't apply that
		// policy again without going round forever; as in Gel, they count
		// the table's rows as they are.
		let policy = if Reading::underway::<U>() {
			None
		} else {
			condition::<U>(&Access::Context(self.clone()), Action::Select)
		};

		Count {
			rows: Subquery::new::<U>(filter, policy, with_deleted),
		}
	}

	/// Whether `query` finds any row this request may see:
	/// `count(query).gt(0)`.
	pub fn exists<U: Table>(&self, query: Query<U>) -> Expr {
		self.count(query).gt(0)
	}
}

/// The number of rows a query finds, inside a policy's rule. Compare it with
/// a number to make a condition; [`Context::count`] makes one.
#[derive(Clone, Debug, PartialEq)]
#[must_use = "a count does nothing until it's compared with a number"]
pub struct Count {
	rows: Subquery,
}

/// The rows of another table that a condition reads: a filter, the policy
/// the request reading them obeys, and the soft-delete column to skip by.
/// Counts and link lookups write one as a subquery in its own scope.
#[doc(hidden)]
#[derive(Clone, Debug, PartialEq)]
pub struct Subquery {
	table: TableName,
	filter: Option<Box<Expr>>,
	policy: Option<Box<Expr>>,
	deleted_at: Option<&'static str>,
}

impl Subquery {
	/// The rows of `U` matching `filter` and `policy`, leaving soft-deleted
	/// ones out unless `with_deleted`.
	pub(crate) fn new<U: Table>(
		filter: Option<Expr>,
		policy: Option<Expr>,
		with_deleted: bool,
	) -> Self {
		Self {
			table: U::table_name(),
			filter: filter.map(Box::new),
			policy: policy.map(Box::new),
			deleted_at: U::DELETED_AT.filter(|_| !with_deleted),
		}
	}

	/// The rows of `table` that `policy` allows, as a condition on another
	/// table reads them: a variant table reading its supertype's.
	pub(crate) fn rows_of(table: TableName, policy: Expr) -> Self {
		Self {
			table,
			filter: None,
			policy: Some(Box::new(policy)),
			deleted_at: None,
		}
	}

	/// Whether every row of the table qualifies, so the subquery can be left
	/// out.
	pub(crate) fn is_everything(&self) -> bool {
		self.filter.is_none() && self.policy.is_none() && self.deleted_at.is_none()
	}

	/// `(SELECT selected FROM table AS lk_<kind>_<depth> WHERE …)`,
	/// correlated with the rows around it through the filter's columns.
	pub(crate) fn write(&self, sql: &mut Sql, selected: &str, kind: &str) -> Result<()> {
		sql.push("(SELECT ")
			.push(selected)
			.push(" FROM ")
			.enter(&self.table, kind);
		let written = self.write_where(sql);
		sql.leave();
		sql.push(")");
		written
	}

	fn write_where(&self, sql: &mut Sql) -> Result<()> {
		let mut first = true;
		let mut next = |sql: &mut Sql| {
			sql.push(if first { " WHERE " } else { " AND " });
			first = false;
		};

		if let Some(filter) = &self.filter {
			next(sql);
			sql.push("(");
			filter.write(sql)?;
			sql.push(")");
		}

		if let Some(policy) = &self.policy {
			next(sql);
			sql.push("(");
			sql.isolated(|sql| policy.write(sql))?;
			sql.push(")");
		}

		if let Some(column) = self.deleted_at {
			next(sql);
			sql.identifier(column).push(" IS NULL");
		}

		Ok(())
	}
}

impl Count {
	/// Exactly `value` rows.
	pub fn eq(self, value: i64) -> Expr {
		self.compare(Comparison::Equal, value)
	}

	/// Any number of rows but `value`.
	pub fn ne(self, value: i64) -> Expr {
		self.compare(Comparison::NotEqual, value)
	}

	/// More than `value` rows.
	pub fn gt(self, value: i64) -> Expr {
		self.compare(Comparison::Greater, value)
	}

	/// At least `value` rows.
	pub fn gte(self, value: i64) -> Expr {
		self.compare(Comparison::GreaterOrEqual, value)
	}

	/// Fewer than `value` rows.
	pub fn lt(self, value: i64) -> Expr {
		self.compare(Comparison::Less, value)
	}

	/// At most `value` rows.
	pub fn lte(self, value: i64) -> Expr {
		self.compare(Comparison::LessOrEqual, value)
	}

	fn compare(self, comparison: Comparison, value: i64) -> Expr {
		Expr::Count {
			count: Box::new(self),
			comparison,
			value,
		}
	}

	/// `(SELECT count(*) FROM table AS alias WHERE …)`, correlated with the
	/// rows around it through the filter's columns.
	pub(crate) fn write(&self, sql: &mut Sql) -> Result<()> {
		self.rows.write(sql, "count(*)", "count")
	}
}

fn lock(symbolic: &std::sync::Mutex<Symbolic>) -> std::sync::MutexGuard<'_, Symbolic> {
	symbolic
		.lock()
		.unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// A table's access policy as Postgres row-level security: its rules for
/// each action rendered as SQL over the row and the request's settings, or
/// `None` for a table without policies, which is open to server code.
///
/// # Errors
///
/// Why the rules can't be rendered: they read globals as Rust values, read
/// a setting nobody registered, or count rows of their own table, which
/// Postgres would check with the same policy forever.
pub(crate) fn row_security<T: Table>()
-> std::result::Result<Option<leptoskit_db_schema::RowSecurity>, String> {
	use leptoskit_db_schema::PolicyCommand;
	use leptoskit_db_schema::RowPolicy;

	let cx = Context::symbolic();
	let access = Access::Context(cx.clone());

	// A table with no rules of its own, and none from a supertype, is open.
	if condition::<T>(&access, Action::Select).is_none() {
		return Ok(None);
	}

	let rules_for = |action| condition::<T>(&access, action).unwrap_or_else(Expr::always);
	let mut rendered = Vec::new();

	for (action, command) in [
		(Action::Select, PolicyCommand::Select),
		(Action::Insert, PolicyCommand::Insert),
		(Action::Update, PolicyCommand::Update),
		(Action::Delete, PolicyCommand::Delete),
	] {
		let condition = rules_for(action);
		// The database layer soft-deletes with an `UPDATE` it checks against
		// the delete rules, which Postgres checks against the update
		// policy: that admits what the delete rules allow too, as long as
		// the row it leaves is soft-deleted.
		let soft_delete = T::DELETED_AT
			.filter(|_| action == Action::Update)
			.map(|column| (rules_for(Action::Delete), column));
		let mut counted = Vec::new();
		subquery_tables(&condition, &mut counted);

		if counted.contains(&T::table_name()) {
			return Err(format!(
				"its {} rules count its own rows, which row-level security can't: Postgres would \
				 check each counted row against the same policy again",
				action.verb_noun()
			));
		}

		let render = |condition: &Expr| -> std::result::Result<String, String> {
			let mut sql = Sql::policy(&T::table_name());
			condition
				.write(&mut sql)
				.map_err(|error| error.to_string())?;
			Ok(sql.finish().sql)
		};
		let (using, check) = match (command, soft_delete) {
			(PolicyCommand::Select | PolicyCommand::Delete, _) => (Some(render(&condition)?), None),
			(PolicyCommand::Insert, _) => (None, Some(render(&condition)?)),
			(PolicyCommand::Update, None) => {
				let sql = render(&condition)?;
				(Some(sql.clone()), Some(sql))
			}
			(PolicyCommand::Update, Some((delete, column))) => {
				let deleted = Expr::Null {
					column,
					negated: true,
				};
				(
					Some(render(&(condition.clone() | delete.clone()))?),
					Some(render(&(condition | (delete & deleted)))?),
				)
			}
		};
		rendered.push(RowPolicy {
			command,
			using,
			check,
		});
	}

	let symbolic = cx.symbolic.as_ref().map(|symbolic| lock(symbolic));

	if let Some(symbolic) = symbolic {
		if let Some(global) = symbolic.opaque.first() {
			return Err(format!(
				"its rules read `{global}` with `cx.global` or `cx.get`, as a Rust value the \
				 database can't see; implement `Setting` for it and compare with \
				 `cx.setting::<…>()` instead"
			));
		}

		if let Some((_, name, type_name)) = symbolic
			.settings
			.iter()
			.find(|(id, ..)| !crate::settings::registered(*id))
		{
			return Err(format!(
				"its rules read setting `{name}` (`{type_name}`), which isn't registered, so no \
				 transaction would set it; add `register_setting!({type_name})`"
			));
		}
	}

	Ok(Some(leptoskit_db_schema::RowSecurity {
		policies: rendered,
	}))
}

/// The tables a condition's counts and lookups read, at any depth.
fn subquery_tables(condition: &Expr, tables: &mut Vec<TableName>) {
	let subquery = |rows: &Subquery, tables: &mut Vec<TableName>| {
		tables.push(rows.table.clone());

		for inner in rows.filter.iter().chain(&rows.policy) {
			subquery_tables(inner, tables);
		}
	};

	match condition {
		Expr::And(left, right) | Expr::Or(left, right) => {
			subquery_tables(left, tables);
			subquery_tables(right, tables);
		}
		Expr::Not(inner) => subquery_tables(inner, tables),
		Expr::Count { count, .. } => subquery(&count.rows, tables),
		Expr::Within { rows, .. } => subquery(rows, tables),
		_ => {}
	}
}

#[expect(
	clippy::missing_fields_in_debug,
	reason = "a symbolic context only exists while rules are rendered, never in a request"
)]
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

	/// The action's name in a sentence about rules: `select`.
	fn verb_noun(self) -> &'static str {
		match self {
			Self::Select => "select",
			Self::Insert => "insert",
			Self::Update => "update",
			Self::Delete => "delete",
		}
	}

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
		/// Who asked, when a request's handle made the bypass.
		actor: Option<crate::Actor>,
	},
}

impl Access {
	/// Who is making the connection's changes, if anyone: the request's
	/// [`Actor`](crate::Actor), or the one a bypass was made for.
	pub(crate) fn actor(&self) -> Option<crate::Actor> {
		match self {
			Self::Context(cx) => cx.actor(),
			Self::System { actor, .. } => *actor,
		}
	}

	/// Why policies are bypassed, for a connection that bypasses them.
	pub(crate) fn reason(&self) -> Option<&'static str> {
		match self {
			Self::Context(_) => None,
			Self::System { reason, .. } => Some(reason),
		}
	}
}

impl Default for Access {
	fn default() -> Self {
		Self::Context(Context::default())
	}
}

/// The condition a row must meet for `action`, or `None` when nothing
/// restricts it: neither the table nor its supertype has policies, or the
/// connection bypasses them.
pub(crate) fn condition<T: Table>(access: &Access, action: Action) -> Option<Expr> {
	match (
		own_condition::<T>(access, action),
		crate::polymorphic::inherited::<T>(access, action),
	) {
		(Some(own), Some(inherited)) => Some(own & inherited),
		(own, inherited) => own.or(inherited),
	}
}

/// What the table's own rules require for `action`.
fn own_condition<T: Table>(access: &Access, action: Action) -> Option<Expr> {
	let policies = T::policies();

	if policies.is_empty() {
		return None;
	}

	let Access::Context(cx) = access else {
		return None;
	};

	Some(evaluate(policies, cx, action))
}

thread_local! {
	/// The read policies being worked out on this thread, innermost last.
	static READING: RefCell<Vec<TypeId>> = const { RefCell::new(Vec::new()) };
}

/// Marks a table's read policy as being worked out, until dropped.
struct Reading;

impl Reading {
	fn enter<T: Table>() -> Self {
		READING.with_borrow_mut(|reading| reading.push(TypeId::of::<T>()));
		Self
	}

	/// Whether `T`'s read policy is being worked out further out.
	fn underway<T: Table>() -> bool {
		READING.with_borrow(|reading| reading.contains(&TypeId::of::<T>()))
	}
}

impl Drop for Reading {
	fn drop(&mut self) {
		READING.with_borrow_mut(Vec::pop);
	}
}

fn evaluate<T: Table>(policies: &[Policy<T>], cx: &Context, action: Action) -> Expr {
	let _reading = (action == Action::Select).then(Reading::enter::<T>);
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

/// A rule a refusal is explained by: the rows it matches, its message, and
/// whether it denies.
pub(crate) type Explanation = (Expr, Option<&'static str>, bool);

/// The rules a refused write is checked against to explain itself: each
/// `deny` that matched, then the `allow`s, in declaration order, the
/// table's own before its supertype's.
pub(crate) fn explanations<T: Table>(access: &Access, action: Action) -> Vec<Explanation> {
	let Access::Context(cx) = access else {
		return Vec::new();
	};
	let columns = T::columns();
	let mut rules: Vec<Explanation> = T::policies()
		.iter()
		.filter(|policy| policy.actions.contains(&action))
		.map(|policy| ((policy.rule)(columns, cx), policy.message, policy.deny))
		.chain(crate::polymorphic::inherited_explanations::<T>(
			access, action,
		))
		.collect();
	// Stable, so each group keeps its order.
	rules.sort_by_key(|(_, _, deny)| !deny);
	rules
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

/// The refusal of writing `image`, a row not stored yet, as `action`, or
/// `None` when `condition` allows it: what [`verify`] and [`refuse`] would
/// say once it's stored. Under row-level security Postgres refuses such a
/// write itself, before the database layer can look, so it asks first.
#[cfg(feature = "postgres")]
pub(crate) async fn refuse_image<T: Table>(
	connection: &impl Connection,
	image: &[Value],
	action: Action,
	condition: &Expr,
) -> Result<Option<DbError>> {
	let count = async |condition: &Expr| -> Result<u64> {
		let statement = crate::live::image_count::<T>(connection.dialect(), image, condition)?;
		crate::sql::count_of(&connection.fetch(statement).await?)
	};

	if count(condition).await? > 0 {
		return Ok(None);
	}

	let explanations = explanations::<T>(connection.access(), action);

	for (rule, message, deny) in &explanations {
		if *deny && count(rule).await? > 0 {
			return Ok(Some(forbidden::<T>(*message, action)));
		}
	}

	let message = explanations
		.iter()
		.find_map(|(_, message, deny)| message.filter(|_| !deny));
	Ok(Some(forbidden::<T>(message, action)))
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

	/// A shelf anyone sees once it holds a book.
	#[table(name = "shelves")]
	#[policy(select, |shelf, cx| cx.exists(Book::query().filter(|book| book.shelf_id.eq(shelf.id))))]
	pub struct Shelf {
		pub id: crate::Id<Self>,
	}

	/// A book, seen while its shelf holds fewer than ten: a count of its own
	/// table, which skips its own read rule.
	#[table(name = "books", namespace = "library")]
	#[policy(select, |book, cx| cx.count(Book::query().filter(|other| other.shelf_id.eq(book.shelf_id))).lt(10))]
	pub struct Book {
		pub id: crate::Id<Self>,
		pub shelf_id: crate::Id<Shelf>,
		#[db(deleted_at)]
		pub deleted_at: Option<crate::Timestamp>,
	}

	/// A global read as a Rust value.
	struct Reader(i32);

	/// Rows a reader sees through a closure, which the database can't run.
	#[table(name = "opaque_rows")]
	#[policy(select, |row, cx| cx.global(|reader: &Reader| row.left.eq(reader.0)))]
	pub struct OpaqueRow {
		#[db(key)]
		pub left: i32,
	}

	/// A setting nobody registered.
	struct Shelfmark(i32);

	impl crate::Setting for Shelfmark {
		type Value = i32;

		const NAME: &'static str = "shelfmark";

		fn value(&self) -> i32 {
			self.0
		}
	}

	/// Rows read through that setting.
	#[table(name = "marked_rows")]
	#[policy(select, |row, cx| row.left.eq(cx.setting::<Shelfmark>()))]
	#[policy(insert, |row, cx| !row.left.eq(cx.setting::<Shelfmark>()))]
	pub struct MarkedRow {
		#[db(key)]
		pub left: i32,
	}

	fn select_sql<T: Table>(dialect: crate::Dialect) -> String {
		let mut sql = Sql::new(dialect);
		sql.push("SELECT * FROM ").table::<T>().push(" WHERE ");
		condition::<T>(&Access::default(), Action::Select)
			.unwrap()
			.write(&mut sql)
			.unwrap();
		sql.finish().sql
	}

	#[test]
	fn counts_are_correlated_subqueries_on_both_dialects() {
		assert_eq!(
			select_sql::<Shelf>(crate::Dialect::Postgres),
			"SELECT * FROM \"shelves\" WHERE (SELECT count(*) FROM \"library\".\"books\" AS \
			 \"lk_count_1\" WHERE (\"shelf_id\" = \"shelves\".\"id\") AND ((SELECT count(*) FROM \
			 \"library\".\"books\" AS \"lk_count_2\" WHERE (\"shelf_id\" = \
			 \"lk_count_1\".\"shelf_id\") AND \"deleted_at\" IS NULL) < $1) AND \"deleted_at\" IS \
			 NULL) > $2"
		);
		assert_eq!(
			select_sql::<Book>(crate::Dialect::Sqlite),
			"SELECT * FROM \"library_books\" WHERE (SELECT count(*) FROM \"library_books\" AS \
			 \"lk_count_1\" WHERE (\"shelf_id\" = \"library_books\".\"shelf_id\") AND \
			 \"deleted_at\" IS NULL) < ?1"
		);
	}

	#[test]
	fn rules_render_as_row_level_security_only_when_the_database_can_run_them() {
		let pairs = row_security::<Pair>().unwrap().unwrap();
		let sql = |command| {
			pairs
				.policies
				.iter()
				.find(|policy| policy.command == command)
				.unwrap()
				.clone()
		};
		use leptoskit_db_schema::PolicyCommand;
		assert_eq!(sql(PolicyCommand::Select).using.as_deref(), Some("1 = 0"));
		assert_eq!(
			sql(PolicyCommand::Insert).check.as_deref(),
			Some("\"left\" > 0")
		);
		assert_eq!(sql(PolicyCommand::Insert).using, None);
		let update = sql(PolicyCommand::Update);
		assert_eq!(
			(update.using.as_deref(), update.check.as_deref()),
			(Some("1 = 0"), Some("1 = 0"))
		);

		let error = row_security::<Book>().unwrap_err();
		assert!(error.contains("count its own rows"), "{error}");
		let error = row_security::<OpaqueRow>().unwrap_err();
		assert!(
			error.contains("Reader") && error.contains("cx.setting"),
			"{error}"
		);
		let error = row_security::<MarkedRow>().unwrap_err();
		assert!(
			error.contains("`shelfmark`") && error.contains("register_setting!"),
			"{error}"
		);
		// A count of another table renders, with that table's own policy.
		let shelves = row_security::<Shelf>().unwrap().unwrap();
		assert!(
			shelves.policies[0]
				.using
				.as_ref()
				.unwrap()
				.contains("FROM \"library\".\"books\" AS \"lk_count_1\"")
		);
		assert!(
			row_security::<crate::AuditEntry>()
				.unwrap_err()
				.contains("AuditReader")
		);
	}

	#[test]
	fn settings_bind_their_value_and_compare_with_null_without_one() {
		let sql = |cx: Context, action| {
			let mut sql = Sql::new(crate::Dialect::Postgres);
			condition::<MarkedRow>(&Access::Context(cx), action)
				.unwrap()
				.write(&mut sql)
				.unwrap();
			sql.finish()
		};
		let marked = sql(Context::new().with(Shelfmark(3)), Action::Select);
		assert_eq!(marked.sql, "\"left\" = $1");
		assert_eq!(marked.args, [Value::Int(3)]);
		// Without the setting, a comparison and its negation both match
		// nothing, as `NULL` does.
		assert_eq!(sql(Context::new(), Action::Select).sql, "\"left\" = NULL");
		assert_eq!(
			sql(Context::new(), Action::Insert).sql,
			"NOT (\"left\" = NULL)"
		);
		// A setting isn't a value of its own.
		let setting = Context::new().with(Shelfmark(1)).setting::<Shelfmark>();
		assert!(crate::Operand::<i32>::into_value(setting).is_err());
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
			actor: None,
		};
		assert!(condition::<Pair>(&system, Action::Insert).is_none());
		assert_eq!(
			explanations::<Pair>(&system, Action::Insert),
			[] as [(Expr, Option<&str>, bool); 0]
		);

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
