//! Inserting, saving, and deleting single rows. `#[db::table]` generates
//! methods that call these.
//!
//! On a table with access policies, a write is filtered by what the request
//! may change and then checked: the stored result must be one the request
//! may insert or update, or the write rolls back with
//! [`DbError::Forbidden`]. The check reads the row as stored, defaults and
//! all, in the same transaction.

use std::collections::HashSet;

use leptoskit_db_schema::Predicate;

use crate::Action;
use crate::Connection;
use crate::DbError;
use crate::Expr;
use crate::Result;
use crate::Table;
use crate::Value;
use crate::WriteContext;
use crate::audit;
use crate::audit::AuditOp;
use crate::connection::atomically;
use crate::expr::Comparison;
use crate::policy;
use crate::sql::Sql;

/// Whether inserting a `T` does more than write it: hooks to run, slugs to
/// fill, a change to record in the audit log, or a supertype row to insert
/// first. Such inserts run in a transaction.
fn insert_effects<T: Table>() -> bool {
	T::HOOKS.before_write
		|| T::HOOKS.after_insert
		|| audit::records::<T>()
		|| !T::SLUGS.is_empty()
		|| T::SUPERTYPE.is_some()
}

/// Whether changing a `T` needs the row as stored first, for hooks or the
/// audit log.
fn update_effects<T: Table>() -> bool {
	T::HOOKS.on_update() || audit::records::<T>()
}

/// Whether deleting a `T` does more than delete it.
fn delete_effects<T: Table>() -> bool {
	T::HOOKS.after_delete || audit::records::<T>()
}

/// Gets a row ready to be written, once whatever it replaces is known: the
/// `before_write` hook, then the slugs of a new row, then who wrote it,
/// again, so that a hook can't forge it, then the columns' rules.
async fn prepare<T: Table>(
	connection: &impl Connection,
	row: &mut T,
	cx: &WriteContext<'_, T>,
) -> Result<()> {
	row.hook_before_write(cx)?;

	if cx.is_insert() {
		fill_slugs(connection, row).await?;
	}

	row.stamp(connection.access().actor(), cx.is_insert())?;
	row.validate()?;
	Ok(())
}

/// Fills each `#[db(slug(from = …))]` column from its source column: the
/// source as a slug, with `-2`, `-3`, … added until no row of the table has
/// it.
async fn fill_slugs<T: Table>(connection: &impl Connection, row: &mut T) -> Result<()> {
	if T::SLUGS.is_empty() {
		return Ok(());
	}

	let mut values = row.to_values()?;

	for (column, from) in T::SLUGS {
		let Value::Text(source) = &values[position::<T>(from)] else {
			return Err(crate::expr::invalid(&format!(
				"`{column}` is a slug of `{from}`, which must be text"
			)));
		};
		let base = crate::Slug::from_text(source).into_inner();
		values[position::<T>(column)] =
			Value::Text(unique_slug::<T>(connection, column, base).await?);
	}

	*row = T::from_values(values)?;
	Ok(())
}

/// `base`, or `base` with the first free numeric suffix. Every row counts,
/// whatever the request may see, since the column is unique among them all;
/// only soft-deleted rows don't, when the unique index leaves them out.
async fn unique_slug<T: Table>(
	connection: &impl Connection,
	column: &str,
	base: String,
) -> Result<String> {
	let mut sql = Sql::new(connection.dialect());
	sql.push("SELECT ")
		.identifier(column)
		.push(" FROM ")
		.table::<T>()
		.push(" WHERE (")
		.identifier(column)
		.push(" = ")
		.value(Value::Text(base.clone()))
		.push(" OR ")
		.identifier(column)
		.push(" LIKE ")
		.value(Value::Text(format!("{base}-%")))
		.push(")");
	write_predicate(&mut sql, unique_predicate::<T>(&[column]), " AND ");

	let taken: HashSet<String> = connection
		.fetch(sql.finish())
		.await?
		.into_iter()
		.filter_map(|row| {
			match row.into_iter().next() {
				Some(Value::Text(slug)) => Some(slug),
				_ => None,
			}
		})
		.collect();

	if !taken.contains(&base) {
		return Ok(base);
	}

	// `taken` holds at most `len - 1` suffixed slugs, so one of these is free.
	let last = taken.len() as u64 + 1;
	Ok((2..=last)
		.map(|suffix| format!("{base}-{suffix}"))
		.find(|slug| !taken.contains(slug))
		.expect("fewer suffixed slugs are taken than there are suffixes to try"))
}

/// The rows of `T` matching `condition`, soft-deleted or not, as stored.
/// On Postgres they stay locked until the transaction ends, so what a write
/// records as their old values is what it changes.
pub(crate) async fn select_for_update<T: Table>(
	connection: &impl Connection,
	condition: &Expr,
) -> Result<Vec<T>> {
	let mut sql = Sql::new(connection.dialect());
	sql.push("SELECT ")
		.columns(T::COLUMNS)
		.push(" FROM ")
		.table::<T>()
		.push(" WHERE (");
	condition.write(&mut sql)?;
	sql.push(")");

	if connection.dialect() == crate::Dialect::Postgres {
		sql.push(" FOR UPDATE");
	}

	connection
		.fetch(sql.finish())
		.await?
		.into_iter()
		.map(|values| T::from_values(values).map_err(DbError::from))
		.collect()
}

/// The stored row with the key in `values`, within `filter`.
async fn stored_row<T: Table>(
	connection: &impl Connection,
	values: &[Value],
	filter: Option<&Expr>,
) -> Result<Option<T>> {
	let row = policy::keys_condition::<T>(&[policy::key_of::<T>(values)]);

	let condition = match filter {
		Some(filter) => row & filter.clone(),
		None => row,
	};

	Ok(select_for_update(connection, &condition)
		.await?
		.into_iter()
		.next())
}

/// Fails unless a hook left the columns that pick the row alone.
fn unchanged<T: Table>(before: &[Value], after: &[Value], columns: &[&str]) -> Result<()> {
	match columns
		.iter()
		.find(|column| before[position::<T>(column)] != after[position::<T>(column)])
	{
		Some(column) => {
			Err(DbError::ReadOnly {
				table: T::flat_name(),
				message: format!(
					"a `before_write` hook changed `{column}`, which picks the `{}` row to write",
					T::flat_name()
				),
			})
		}
		None => Ok(()),
	}
}

/// Inserts a row and returns it as stored.
pub async fn insert<T: Table>(connection: &impl Connection, mut row: T) -> Result<T> {
	row.stamp(connection.access().actor(), true)?;
	let check = policy::condition::<T>(connection.access(), Action::Insert);

	if insert_effects::<T>() {
		return atomically!(connection, |tx| {
			effected_insert(tx, row, check.as_ref()).await
		});
	}

	row.validate()?;

	let Some(check) = check else {
		return write_insert(connection, row).await;
	};

	atomically!(connection, |tx| checked_insert(tx, row, &check).await)
}

/// Inserts with hooks, slugs, and the audit log, all in one transaction.
async fn effected_insert<T: Table>(
	connection: &impl Connection,
	mut row: T,
	check: Option<&Expr>,
) -> Result<T> {
	prepare(
		connection,
		&mut row,
		&WriteContext::insert(connection.access()),
	)
	.await?;
	// The row's policies may read its supertype row, so that goes first.
	row.write_supertype(connection).await?;

	if let Some(check) = check {
		refuse_before_row_security::<T>(connection, &row, check).await?;
	}

	let stored = write_insert(connection, row).await?;

	if let Some(check) = check {
		let key = policy::key_of::<T>(&stored.to_values()?);
		policy::verify::<T>(connection, &[key], Action::Insert, check).await?;
	}

	audit::record(connection, AuditOp::Insert, None, Some(&stored)).await?;
	stored.hook_after_insert(connection).await?;
	Ok(stored)
}

/// Inserts, then checks the stored row. Errors return from here rather than
/// from inside `atomically!`, which must see them to roll back.
async fn checked_insert<T: Table>(connection: &impl Connection, row: T, check: &Expr) -> Result<T> {
	refuse_before_row_security::<T>(connection, &row, check).await?;
	let stored = write_insert(connection, row).await?;
	let key = policy::key_of::<T>(&stored.to_values()?);
	policy::verify::<T>(connection, &[key], Action::Insert, check).await?;
	Ok(stored)
}

/// Under row-level security, refuses an insert its policy won't allow
/// before writing it, with the database layer's own refusal: Postgres would
/// refuse it first, with an error that says less.
async fn refuse_before_row_security<T: Table>(
	connection: &impl Connection,
	row: &T,
	check: &Expr,
) -> Result<()> {
	refuse_result_before_row_security::<T>(connection, &row.to_values()?, Action::Insert, check)
		.await
}

/// Under row-level security, refuses a write whose result, `values`, its
/// policy won't allow for `action`, before writing it: Postgres checks the
/// result first, and its refusal names no rule.
#[cfg_attr(
	not(feature = "postgres"),
	expect(clippy::unused_async, reason = "only Postgres has row-level security")
)]
async fn refuse_result_before_row_security<T: Table>(
	connection: &impl Connection,
	values: &[Value],
	action: Action,
	check: &Expr,
) -> Result<()> {
	#[cfg(feature = "postgres")]
	if connection.row_security()
		&& let Some(refusal) = policy::refuse_image::<T>(connection, values, action, check).await?
	{
		return Err(refusal);
	}

	#[cfg(not(feature = "postgres"))]
	let _ = (connection, values, action, check);

	Ok(())
}

/// Inserts a row without reading it back: a polymorphic table's supertype
/// row under row-level security. Postgres refuses to return a row the
/// request can't read, which would refuse the insert before the variant
/// row's own check could say why. The row must be complete as given, with
/// nothing for the database to fill in.
#[cfg(feature = "postgres")]
pub(crate) async fn insert_unreturned<T: Table>(
	connection: &impl Connection,
	mut row: T,
) -> Result<()> {
	row.stamp(connection.access().actor(), true)?;
	row.hook_before_write(&WriteContext::insert(connection.access()))?;
	row.stamp(connection.access().actor(), true)?;
	row.validate()?;

	if let Some(check) = policy::condition::<T>(connection.access(), Action::Insert) {
		refuse_before_row_security::<T>(connection, &row, &check).await?;
	}

	let mut sql = Sql::new(connection.dialect());
	insert_statement::<T>(&mut sql, row.to_values()?);
	connection.execute(sql.finish()).await?;
	audit::record(connection, AuditOp::Insert, None, Some(&row)).await?;
	row.hook_after_insert(connection).await
}

async fn write_insert<T: Table>(connection: &impl Connection, row: T) -> Result<T> {
	let mut sql = Sql::new(connection.dialect());
	insert_statement::<T>(&mut sql, row.to_values()?);
	returning::<T>(connection, sql).await
}

/// Inserts a row, or updates the existing row whose `target` columns match.
/// The key and `created_at` of an existing row are kept.
///
/// With policies, overwriting an existing row needs it to be one the request
/// may update, and the result must be one it may insert or update.
pub async fn upsert<T: Table>(
	connection: &impl Connection,
	mut row: T,
	target: &[&'static str],
) -> Result<T> {
	row.stamp(connection.access().actor(), true)?;
	let insert = policy::condition::<T>(connection.access(), Action::Insert);
	let update = policy::condition::<T>(connection.access(), Action::Update);

	if insert_effects::<T>() || update_effects::<T>() {
		return atomically!(connection, |tx| {
			effected_upsert(tx, row, target, insert.as_ref(), update.as_ref()).await
		});
	}

	row.validate()?;

	// A table's policies cover every action once it has any, so the two
	// conditions come and go together.
	let (Some(insert), Some(update)) = (insert, update) else {
		return write_upsert(connection, row, target).await;
	};

	atomically!(connection, |tx| {
		checked_upsert(tx, row, target, &insert, &update).await
	})
}

/// Upserts with hooks, slugs, and the audit log. The row it would
/// overwrite is read first, and locked on Postgres, so the hooks know
/// whether they insert or update. Without one it inserts plainly: a row
/// another transaction inserts meanwhile makes it fail as a conflict,
/// rather than run the insert's hooks for an update.
async fn effected_upsert<T: Table>(
	connection: &impl Connection,
	mut row: T,
	target: &[&'static str],
	insert: Option<&Expr>,
	update: Option<&Expr>,
) -> Result<T> {
	let before = row.to_values()?;
	let conflict = conflict_condition::<T>(&before, target);
	let existing = select_for_update::<T>(connection, &conflict)
		.await?
		.into_iter()
		.next();
	let access = connection.access();

	let Some(old) = existing else {
		prepare(connection, &mut row, &WriteContext::insert(access)).await?;
		unchanged::<T>(&before, &row.to_values()?, target)?;
		row.write_supertype(connection).await?;

		if let Some(check) = insert {
			refuse_before_row_security::<T>(connection, &row, check).await?;
		}

		let stored = write_insert(connection, row).await?;

		if let Some(check) = insert {
			let key = policy::key_of::<T>(&stored.to_values()?);
			policy::verify::<T>(connection, &[key], Action::Insert, check).await?;
		}

		audit::record(connection, AuditOp::Insert, None, Some(&stored)).await?;
		stored.hook_after_insert(connection).await?;

		return Ok(stored);
	};

	if let Some(check) = update
		&& policy::count::<T>(connection, &(conflict.clone() & check.clone())).await? == 0
	{
		return Err(policy::refuse::<T>(connection, &conflict, Action::Update).await?);
	}

	prepare(connection, &mut row, &WriteContext::update(access, &old)).await?;
	unchanged::<T>(&before, &row.to_values()?, target)?;

	if let Some(check) = update {
		let result = upserted::<T>(&old.to_values()?, &row.to_values()?, target);
		refuse_result_before_row_security::<T>(connection, &result, Action::Update, check).await?;
	}

	let stored = write_upsert(connection, row, target).await?;

	if let Some(check) = update {
		let key = policy::key_of::<T>(&stored.to_values()?);
		policy::verify::<T>(connection, &[key], Action::Update, check).await?;
	}

	audit::record(connection, AuditOp::Update, Some(&old), Some(&stored)).await?;
	T::hook_after_update(&old, &stored, connection).await?;
	Ok(stored)
}

async fn checked_upsert<T: Table>(
	connection: &impl Connection,
	row: T,
	target: &[&'static str],
	insert: &Expr,
	update: &Expr,
) -> Result<T> {
	let conflict = conflict_condition::<T>(&row.to_values()?, target);
	let overwrites = policy::count::<T>(connection, &conflict).await? > 0;

	if overwrites
		&& policy::count::<T>(connection, &(conflict.clone() & update.clone())).await? == 0
	{
		return Err(policy::refuse::<T>(connection, &conflict, Action::Update).await?);
	}

	let (action, check) = if overwrites {
		(Action::Update, update)
	} else {
		(Action::Insert, insert)
	};

	#[cfg(feature = "postgres")]
	if connection.row_security() {
		let values = row.to_values()?;
		let result = if overwrites {
			let old = select_for_update::<T>(connection, &conflict)
				.await?
				.into_iter()
				.next()
				.map(|old| old.to_values())
				.transpose()?
				.unwrap_or_else(|| values.clone());
			upserted::<T>(&old, &values, target)
		} else {
			values
		};

		refuse_result_before_row_security::<T>(connection, &result, action, check).await?;
	}

	let stored = write_upsert(connection, row, target).await?;
	let key = policy::key_of::<T>(&stored.to_values()?);
	policy::verify::<T>(connection, &[key], action, check).await?;
	Ok(stored)
}

/// The row an upsert leaves when it overwrites `old` with `new`: what it
/// keeps of `old`, as `write_upsert`'s `DO UPDATE` does, and `new`
/// otherwise.
fn upserted<T: Table>(old: &[Value], new: &[Value], target: &[&'static str]) -> Vec<Value> {
	T::COLUMNS
		.iter()
		.zip(old.iter().zip(new))
		.map(|(column, (old, new))| {
			let kept = T::KEY.contains(column)
				|| target.contains(column)
				|| Some(*column) == T::CREATED_AT
				|| Some(*column) == T::CREATED_BY
				|| T::GENERATED.contains(column)
				|| T::SLUGS.iter().any(|(slug, _)| slug == column);
			if kept { old.clone() } else { new.clone() }
		})
		.collect()
}

/// The existing rows an upsert would overwrite: the target columns' values,
/// within the unique index's predicate when it's partial.
fn conflict_condition<T: Table>(values: &[Value], target: &[&'static str]) -> Expr {
	let matches = target
		.iter()
		.map(|column| {
			Expr::Compare {
				column,
				comparison: Comparison::Equal,
				value: values[position::<T>(column)].clone(),
			}
		})
		.reduce(|left, right| left & right)
		.unwrap_or_else(Expr::never);

	// Generated tables only make `IS NULL` predicates (`unique(except = …)`);
	// a hand-written SQL predicate has no expression to repeat.
	let partial = match unique_predicate::<T>(target) {
		Some(Predicate::IsNull(column)) => Some((column, false)),
		Some(Predicate::IsNotNull(column)) => Some((column, true)),
		Some(Predicate::Sql(_)) | None => None,
	};

	match partial {
		Some((column, negated)) => {
			matches
				& Expr::Null {
					column: static_column::<T>(&column),
					negated,
				}
		}
		None => matches,
	}
}

/// The predicate of the partial unique index over `target`, if it is one.
fn unique_predicate<T: Table>(target: &[&str]) -> Option<Predicate> {
	T::definition()
		.indexes
		.into_iter()
		.find(|index| {
			index.unique
				&& index
					.columns
					.iter()
					.map(String::as_str)
					.eq(target.iter().copied())
		})
		.and_then(|index| index.predicate)
}

/// `{lead}predicate`, for a partial unique index's predicate.
fn write_predicate(sql: &mut Sql, predicate: Option<Predicate>, lead: &str) {
	match predicate {
		Some(Predicate::IsNull(column)) => {
			sql.push(lead).identifier(&column).push(" IS NULL");
		}
		Some(Predicate::IsNotNull(column)) => {
			sql.push(lead).identifier(&column).push(" IS NOT NULL");
		}
		Some(Predicate::Sql(predicate)) => {
			sql.push(lead).push("(").push(&predicate).push(")");
		}
		None => {}
	}
}

/// The table's own name for a column, which outlives the definition.
fn static_column<T: Table>(column: &str) -> &'static str {
	T::COLUMNS[position::<T>(column)]
}

async fn write_upsert<T: Table>(
	connection: &impl Connection,
	row: T,
	target: &[&'static str],
) -> Result<T> {
	let mut sql = Sql::new(connection.dialect());
	insert_statement::<T>(&mut sql, row.to_values()?);
	sql.push(" ON CONFLICT (").columns(target).push(")");

	// A partial unique index only arbitrates the conflict when the statement
	// repeats its predicate.
	write_predicate(&mut sql, unique_predicate::<T>(target), " WHERE ");

	// What a row keeps from when it was inserted: its key, who made it and
	// when, and its slugs.
	let updated: Vec<&str> = T::COLUMNS
		.iter()
		.copied()
		.filter(|column| {
			!T::KEY.contains(column)
				&& !target.contains(column)
				&& Some(*column) != T::CREATED_AT
				&& Some(*column) != T::CREATED_BY
				&& !T::GENERATED.contains(column)
				&& !T::SLUGS.iter().any(|(slug, _)| slug == column)
		})
		.collect();
	// With nothing else to change, a no-op update still returns the row.
	let updated = if updated.is_empty() {
		target.to_vec()
	} else {
		updated
	};

	sql.push(" DO UPDATE SET ");

	for (index, column) in updated.iter().enumerate() {
		if index > 0 {
			sql.push(", ");
		}

		sql.identifier(column)
			.push(" = EXCLUDED.")
			.identifier(column);
	}

	returning::<T>(connection, sql).await
}

/// Writes every column of an existing row, matched by its key, and bumps
/// `updated_at`. A versioned row is only written while its version is the
/// one it was read with; otherwise the save fails with
/// [`DbError::Stale`]. With policies, the row must be one the request may
/// change, and so must what it becomes.
pub async fn save<T: Table>(connection: &impl Connection, row: &mut T) -> Result<()> {
	row.touch();
	row.stamp(connection.access().actor(), false)?;
	let check = policy::condition::<T>(connection.access(), Action::Update);

	if update_effects::<T>() {
		let stored = atomically!(connection, |tx| {
			effected_save(tx, row, check.as_ref()).await
		})?;
		*row = stored;

		return Ok(());
	}

	row.validate()?;
	let values = row.to_values()?;

	let stored = match check {
		None => write_save::<T>(connection, &values, None, false).await?,
		Some(check) => {
			atomically!(connection, |tx| {
				checked_save::<T>(tx, &values, &check).await
			})?
		}
	};

	// The database recomputed its generated columns, so the row takes them,
	// and the version it stored, from what it returned.
	match stored {
		Some(stored) => *row = stored,
		None => row.bump_version(),
	}

	Ok(())
}

/// Saves with hooks and the audit log: reads the row as stored, within
/// what the request may change, runs `before_write`, writes, and returns
/// the row as stored again.
async fn effected_save<T: Table>(
	connection: &impl Connection,
	row: &mut T,
	check: Option<&Expr>,
) -> Result<T> {
	let before = row.to_values()?;
	let Some(old) = stored_row::<T>(connection, &before, check).await? else {
		return Err(policy::missing::<T>(connection, &before, Action::Update, true).await?);
	};

	prepare(
		connection,
		row,
		&WriteContext::update(connection.access(), &old),
	)
	.await?;
	let values = row.to_values()?;
	unchanged::<T>(&before, &values, T::KEY)?;

	if let Some(check) = check {
		refuse_result_before_row_security::<T>(
			connection,
			&saved::<T>(&values),
			Action::Update,
			check,
		)
		.await?;
	}

	let stored = write_save::<T>(connection, &values, check, true)
		.await?
		.expect("a save asked to return the row returns it");

	if let Some(check) = check {
		policy::verify::<T>(
			connection,
			&[policy::key_of::<T>(&values)],
			Action::Update,
			check,
		)
		.await?;
	}

	audit::record(connection, AuditOp::Update, Some(&old), Some(&stored)).await?;
	T::hook_after_update(&old, &stored, connection).await?;
	Ok(stored)
}

/// Saves the row the request may change, then checks what it became.
async fn checked_save<T: Table>(
	connection: &impl Connection,
	values: &[Value],
	check: &Expr,
) -> Result<Option<T>> {
	refuse_result_before_row_security::<T>(connection, &saved::<T>(values), Action::Update, check)
		.await?;
	let stored = write_save::<T>(connection, values, Some(check), false).await?;
	policy::verify::<T>(
		connection,
		&[policy::key_of::<T>(values)],
		Action::Update,
		check,
	)
	.await?;
	Ok(stored)
}

/// The row a save leaves: `values`, with the version it increments.
fn saved<T: Table>(values: &[Value]) -> Vec<Value> {
	let mut saved = values.to_vec();

	if let Some(version) = T::VERSION
		&& let Value::Int(read) = saved[position::<T>(version)]
	{
		saved[position::<T>(version)] = Value::Int(read + 1);
	}

	saved
}

/// Writes the row's columns, and returns the row as stored when asked to,
/// or when the table has generated columns, which only the database knows.
async fn write_save<T: Table>(
	connection: &impl Connection,
	values: &[Value],
	filter: Option<&Expr>,
	returning: bool,
) -> Result<Option<T>> {
	let mut sql = Sql::new(connection.dialect());
	sql.push("UPDATE ").table::<T>().push(" SET ");
	let mut first = true;

	for (column, value) in T::COLUMNS.iter().zip(values) {
		if T::KEY.contains(column)
			|| Some(*column) == T::CREATED_AT
			|| Some(*column) == T::CREATED_BY
			|| T::GENERATED.contains(column)
		{
			continue;
		}

		if !first {
			sql.push(", ");
		}

		first = false;
		sql.identifier(column).push(" = ");

		if Some(*column) == T::VERSION {
			sql.identifier(column).push(" + 1");
		} else {
			sql.value(value.clone());
		}
	}

	write_key::<T>(&mut sql, values);

	if let Some(version) = T::VERSION {
		sql.push(" AND ")
			.identifier(version)
			.push(" = ")
			.value(values[position::<T>(version)].clone());
	}

	write_filter(&mut sql, filter)?;

	if T::GENERATED.is_empty() && !returning {
		if connection.execute(sql.finish()).await? == 0 {
			return Err(policy::missing::<T>(connection, values, Action::Update, true).await?);
		}

		return Ok(None);
	}

	sql.push(" RETURNING ").columns(T::COLUMNS);

	match connection.fetch(sql.finish()).await?.into_iter().next() {
		Some(stored) => Ok(Some(T::from_values(stored)?)),
		None => Err(policy::missing::<T>(connection, values, Action::Update, true).await?),
	}
}

/// Where a column sits in [`Table::COLUMNS`].
fn position<T: Table>(column: &str) -> usize {
	T::COLUMNS
		.iter()
		.position(|candidate| *candidate == column)
		.expect("a marker is a column")
}

/// Deletes a row: soft-deletes it when the table has a `#[db(deleted_at)]`
/// column, setting that column on `row` too. Rows linked to it with
/// `on_soft_delete = cascade` are soft-deleted with it, in one transaction.
/// With policies, the row must be one the request may delete.
pub async fn delete<T: Table>(connection: &impl Connection, row: &mut T) -> Result<()> {
	if T::DELETED_AT.is_none() {
		return destroy(connection, row).await;
	}

	row.mark_deleted();
	row.touch();
	row.stamp(connection.access().actor(), false)?;
	let filter = policy::condition::<T>(connection.access(), Action::Delete);

	if delete_effects::<T>() {
		let stored = atomically!(connection, |tx| {
			effected_soft_delete(tx, row, filter.as_ref()).await
		})?;
		*row = stored;

		return Ok(());
	}

	#[cfg(any(feature = "postgres", feature = "sqlite"))]
	let deleted = if crate::cascade::has_cascades::<T>() {
		atomically!(connection, |tx| {
			soft_delete_cascading(tx, row, filter.as_ref()).await
		})
	} else {
		soft_delete_cascading(connection, row, filter.as_ref()).await
	};

	#[cfg(not(any(feature = "postgres", feature = "sqlite")))]
	let deleted = soft_delete_cascading(connection, row, filter.as_ref()).await;

	deleted?;
	row.bump_version();
	Ok(())
}

/// Soft-deletes with the audit log and `after_delete`, returning the row as
/// the delete left it.
async fn effected_soft_delete<T: Table>(
	connection: &impl Connection,
	row: &T,
	filter: Option<&Expr>,
) -> Result<T> {
	let values = row.to_values()?;
	let old = if audit::records::<T>() {
		match stored_row::<T>(connection, &values, filter).await? {
			Some(old) => Some(old),
			None => {
				return Err(
					policy::missing::<T>(connection, &values, Action::Delete, false).await?,
				);
			}
		}
	} else {
		None
	};

	let stored = soft_delete::<T>(connection, &values, filter, true)
		.await?
		.expect("a soft delete asked to return the row returns it");
	audit::record(connection, AuditOp::Delete, old.as_ref(), Some(&stored)).await?;
	cascade_from::<T>(connection, &values).await?;
	stored.hook_after_delete(connection).await?;
	Ok(stored)
}

/// Writes a row's soft delete, already marked on `row`, then follows its
/// cascading links.
async fn soft_delete_cascading<T: Table>(
	connection: &impl Connection,
	row: &T,
	filter: Option<&Expr>,
) -> Result<()> {
	let values = row.to_values()?;
	soft_delete::<T>(connection, &values, filter, false).await?;
	cascade_from::<T>(connection, &values).await
}

/// Soft-deletes the rows linked to the row with these values through
/// cascading links. Cascades are integrity, not access: they reach linked
/// rows whatever those tables' policies say.
async fn cascade_from<T: Table>(connection: &impl Connection, values: &[Value]) -> Result<()> {
	let column = T::DELETED_AT.expect("only tables with a soft-delete column soft-delete");

	if let ([key], Value::Timestamp(now)) = (T::KEY, &values[position::<T>(column)]) {
		crate::cascade::soft_delete_from::<T>(
			connection,
			vec![values[position::<T>(key)].clone()],
			*now,
		)
		.await?;
	}

	Ok(())
}

/// Writes the soft delete marked in `values`: its `deleted_at`, and its
/// `updated_at` and `updated_by`. Returns the row as stored when asked to.
async fn soft_delete<T: Table>(
	connection: &impl Connection,
	values: &[Value],
	filter: Option<&Expr>,
	returning: bool,
) -> Result<Option<T>> {
	let column = T::DELETED_AT.expect("only tables with a soft-delete column soft-delete");
	let mut sql = Sql::new(connection.dialect());
	sql.push("UPDATE ").table::<T>().push(" SET ");
	let mut changed = vec![column];
	changed.extend(T::UPDATED_AT);
	changed.extend(T::UPDATED_BY);

	for (index, column) in changed.iter().enumerate() {
		if index > 0 {
			sql.push(", ");
		}

		sql.identifier(column)
			.push(" = ")
			.value(values[position::<T>(column)].clone());
	}

	if let Some(version) = T::VERSION {
		sql.push(", ")
			.identifier(version)
			.push(" = ")
			.identifier(version)
			.push(" + 1");
	}

	write_key::<T>(&mut sql, values);
	write_filter(&mut sql, filter)?;

	if !returning {
		if connection.execute(sql.finish()).await? == 0 {
			return Err(policy::missing::<T>(connection, values, Action::Delete, false).await?);
		}

		return Ok(None);
	}

	sql.push(" RETURNING ").columns(T::COLUMNS);

	match connection.fetch(sql.finish()).await?.into_iter().next() {
		Some(stored) => Ok(Some(T::from_values(stored)?)),
		None => Err(policy::missing::<T>(connection, values, Action::Delete, false).await?),
	}
}

/// Removes a row for good. With policies, the row must be one the request
/// may delete.
pub async fn destroy<T: Table>(connection: &impl Connection, row: &T) -> Result<()> {
	let values = row.to_values()?;
	let filter = policy::condition::<T>(connection.access(), Action::Delete);

	if delete_effects::<T>() || crate::fallout::reaches::<T>() || T::SUPERTYPE.is_some() {
		return atomically!(connection, |tx| {
			effected_destroy::<T>(tx, &values, filter.as_ref()).await
		});
	}

	let mut sql = Sql::new(connection.dialect());
	sql.push("DELETE FROM ").table::<T>();
	write_key::<T>(&mut sql, &values);
	write_filter(&mut sql, filter.as_ref())?;

	if connection.execute(sql.finish()).await? == 0 {
		return Err(policy::missing::<T>(connection, &values, Action::Delete, false).await?);
	}

	Ok(())
}

/// Removes a row with the audit log and `after_delete`, which get the row
/// as it was stored, and records the rows the database's own cascades
/// change with it.
async fn effected_destroy<T: Table>(
	connection: &impl Connection,
	values: &[Value],
	filter: Option<&Expr>,
) -> Result<()> {
	let fallout = if crate::fallout::reaches::<T>() {
		let Some(old) = stored_row::<T>(connection, values, filter).await? else {
			return Err(policy::missing::<T>(connection, values, Action::Delete, false).await?);
		};
		Some(
			crate::fallout::before_delete(
				connection,
				T::table_name(),
				T::COLUMNS,
				vec![old.to_values()?],
			)
			.await?,
		)
	} else {
		None
	};

	let mut sql = Sql::new(connection.dialect());
	sql.push("DELETE FROM ").table::<T>();
	write_key::<T>(&mut sql, values);
	write_filter(&mut sql, filter)?;
	sql.push(" RETURNING ").columns(T::COLUMNS);

	let Some(stored) = connection.fetch(sql.finish()).await?.into_iter().next() else {
		return Err(policy::missing::<T>(connection, values, Action::Delete, false).await?);
	};
	let old = T::from_values(stored)?;
	audit::record(connection, AuditOp::Delete, Some(&old), None).await?;

	if let Some(fallout) = fallout {
		fallout.record(connection).await?;
	}

	T::remove_supertypes(policy::key_of::<T>(values), connection).await?;
	old.hook_after_delete(connection).await
}

/// `INSERT INTO table (…) VALUES (…)` for every column the database
/// doesn't compute.
fn insert_statement<T: Table>(sql: &mut Sql, values: Vec<Value>) {
	let (columns, values): (Vec<&str>, Vec<Value>) = T::COLUMNS
		.iter()
		.copied()
		.zip(values)
		.filter(|(column, _)| !T::GENERATED.contains(column))
		.unzip();
	sql.push("INSERT INTO ")
		.table::<T>()
		.push(" (")
		.columns(&columns)
		.push(") VALUES (");

	for (index, value) in values.into_iter().enumerate() {
		if index > 0 {
			sql.push(", ");
		}

		sql.value(value);
	}

	sql.push(")");
}

async fn returning<T: Table>(connection: &impl Connection, mut sql: Sql) -> Result<T> {
	sql.push(" RETURNING ").columns(T::COLUMNS);
	let row = connection.fetch(sql.finish()).await?.into_iter().next();
	let row = row.ok_or_else(|| {
		DbError::NotFound {
			table: T::flat_name(),
		}
	})?;
	Ok(T::from_values(row)?)
}

/// `WHERE key = …` for a row's key columns.
fn write_key<T: Table>(sql: &mut Sql, values: &[Value]) {
	sql.push(" WHERE ");

	for (index, key) in T::KEY.iter().enumerate() {
		if index > 0 {
			sql.push(" AND ");
		}

		sql.identifier(key)
			.push(" = ")
			.value(values[position::<T>(key)].clone());
	}
}

/// `AND (…)` for a policy's filter, after `write_key`.
fn write_filter(sql: &mut Sql, filter: Option<&Expr>) -> Result<()> {
	if let Some(filter) = filter {
		sql.push(" AND (");
		filter.write(sql)?;
		sql.push(")");
	}

	Ok(())
}
