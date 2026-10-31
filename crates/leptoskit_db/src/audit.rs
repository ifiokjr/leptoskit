//! The audit log: `#[db::table(audit)]` records every change the database
//! layer makes to a table, in the same transaction, in `_leptoskit_audit`.
//!
//! Each entry says who made the change (the [`Actor`](crate::Actor), and the
//! reason when [`Db::system`](crate::Db::system) bypassed policies), what
//! changed (the table, the row's key, and the row before and after, without
//! its `secret` columns), and when.
//!
//! Reading the log is refused unless the request's context holds an
//! [`AuditReader`]; a `system` handle reads all of it.

use leptoskit_db_schema::TableName;
use uuid::Uuid;

use crate::Connection;
use crate::Context;
use crate::Enum;
use crate::Expr;
use crate::Id;
use crate::Json;
use crate::Query;
use crate::Result;
use crate::Table;
use crate::Timestamp;
use crate::Value;
use crate::policy;
use crate::sql::Sql;
use crate::table;

/// What a change did to a row.
#[derive(Enum, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AuditOp {
	/// The row was inserted: there is no `before`.
	Insert,
	/// The row was changed.
	Update,
	/// The row was deleted: for good when there is no `after`, and softly,
	/// with its `deleted_at` set, when there is.
	Delete,
}

/// One change to a row of an audited table.
#[table(
	name = "_leptoskit_audit",
	__internal,
	index(table_name, row_key),
	index(at)
)]
#[policy(select, |entry, cx| cx.global(|reader: &AuditReader| (reader.rule)(entry, cx)))]
pub struct AuditEntry {
	/// The entry's key, which orders entries by when they were made.
	pub id: Id<Self>,
	/// When the change was made: the same instant for every row one write
	/// changed.
	pub at: Timestamp,
	/// The changed table, with its namespace: `posts` or `billing.invoices`.
	pub table_name: String,
	/// The changed row's key: its key columns' values as a JSON array.
	pub row_key: String,
	/// What the change did.
	pub op: AuditOp,
	/// Who made the change, if anyone was acting.
	pub actor: Option<Uuid>,
	/// Why policies were bypassed, for a change made through
	/// [`Db::system`](crate::Db::system).
	pub reason: Option<String>,
	/// The row before the change, without its `secret` columns.
	pub before: Option<Json<serde_json::Value>>,
	/// The row after the change, without its `secret` columns.
	pub after: Option<Json<serde_json::Value>>,
}

impl AuditEntry {
	/// The entries about one row of `T`, oldest first. What a request sees
	/// still depends on its [`AuditReader`].
	///
	/// ```ignore
	/// let history = AuditEntry::for_row(&post).all(&db.system("support ticket 42")).await?;
	/// ```
	pub fn for_row<T: Table>(row: &T) -> Result<Query<Self>> {
		let key = row_key_of(&policy::key_of::<T>(&row.to_values()?));
		Ok(Self::for_table::<T>().filter(|entry| entry.row_key.eq(key)))
	}

	/// The entries about any row of `T`, oldest first.
	pub fn for_table<T: Table>() -> Query<Self> {
		let table = T::table_name().to_string();
		Self::query()
			.filter(|entry| entry.table_name.eq(table))
			.order_by(|entry| entry.id.asc())
	}

	/// Who made the change, if anyone was acting.
	pub fn actor(&self) -> Option<crate::Actor> {
		self.actor.map(crate::Actor::from_uuid)
	}
}

/// Lets a request read the audit log. Without one in its [`Context`], a
/// request reads no entry at all; a `system` handle reads every entry.
///
/// ```ignore
/// // Administrators read everything.
/// let cx = cx.with(AuditReader::everything());
/// // Everyone else reads what they did themselves.
/// let cx = cx.with(AuditReader::matching(|entry, cx| match cx.actor() {
/// 	Some(actor) => entry.actor.eq(actor.uuid()),
/// 	None => Expr::never(),
/// }));
/// ```
#[derive(Clone, Copy)]
pub struct AuditReader {
	rule: fn(&AuditEntryColumns, &Context) -> Expr,
}

impl AuditReader {
	/// Reads every entry.
	pub const fn everything() -> Self {
		Self { rule: everything }
	}

	/// Reads the entries `rule` matches.
	pub const fn matching(rule: fn(&AuditEntryColumns, &Context) -> Expr) -> Self {
		Self { rule }
	}
}

fn everything(_: &AuditEntryColumns, _: &Context) -> Expr {
	Expr::always()
}

impl std::fmt::Debug for AuditReader {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("AuditReader").finish_non_exhaustive()
	}
}

/// Under row-level security, Postgres sees whether a request holds a
/// reader, though not what its rule matches: see [`row_security`].
impl crate::Setting for AuditReader {
	type Value = bool;

	const NAME: &'static str = "audit_reader";

	fn value(&self) -> bool {
		true
	}
}

crate::register_setting!(AuditReader);

/// The audit log's row-level security. An [`AuditReader`]'s rule is Rust
/// code, which the database can't run, so Postgres lets a request read the
/// log when it holds any reader, and the rule narrows only what the
/// database layer reads. The database layer records entries as the request
/// whose change they record, so a request may add one; changing or removing
/// one bypasses policies, as only `system` handles do.
pub(crate) fn row_security() -> leptoskit_db_schema::RowSecurity {
	use leptoskit_db_schema::PolicyCommand;
	use leptoskit_db_schema::RowPolicy;

	let reader = format!(
		"NULLIF(current_setting({}, true), '')::boolean",
		leptoskit_db_schema::quote_literal(&format!(
			"{}{}",
			leptoskit_db_schema::SETTING_PREFIX,
			<AuditReader as crate::Setting>::NAME
		))
	);
	let policy = |command, using: Option<&str>, check: Option<&str>| {
		RowPolicy {
			command,
			using: using.map(str::to_string),
			check: check.map(str::to_string),
		}
	};

	leptoskit_db_schema::RowSecurity {
		policies: vec![
			policy(PolicyCommand::Select, Some(&reader), None),
			policy(PolicyCommand::Insert, None, Some("1 = 1")),
			policy(PolicyCommand::Update, Some("1 = 0"), Some("1 = 0")),
			policy(PolicyCommand::Delete, Some("1 = 0"), None),
		],
	}
}

/// Whether changes to `T`'s rows are recorded: in the audit log, or as live
/// changes for subscribers. Writes to such a table read the rows they
/// change, and run in a transaction.
pub(crate) const fn records<T: Table>() -> bool {
	T::AUDIT || T::LIVE
}

/// What the audit log and live changes need to know about a table to record
/// its rows, without its Rust type: cascades reach tables they know only by
/// name.
#[derive(Clone, Copy)]
pub(crate) struct Subject {
	pub(crate) table: fn() -> TableName,
	pub(crate) columns: &'static [&'static str],
	pub(crate) key: &'static [&'static str],
	pub(crate) secret: &'static [&'static str],
	/// Reads raw values back through the table's Rust type, so they record
	/// the same whichever statement returned them: SQLite returns a UUID as
	/// bytes, for one.
	pub(crate) normalize: fn(Vec<Value>) -> Result<Vec<Value>>,
	/// Whether changes go into the audit log.
	pub(crate) audit: bool,
	/// Whether changes are published to live subscribers.
	pub(crate) live: bool,
}

impl Subject {
	pub(crate) const fn of<T: Table>() -> Self {
		Self {
			table: T::table_name,
			columns: T::COLUMNS,
			key: T::KEY,
			secret: T::SECRET,
			normalize: normalize::<T>,
			audit: T::AUDIT,
			live: T::LIVE,
		}
	}

	/// The subject, when its table records changes at all.
	pub(crate) const fn recorded<T: Table>() -> Option<Self> {
		if records::<T>() {
			Some(Self::of::<T>())
		} else {
			None
		}
	}

	pub(crate) fn key_of(&self, values: &[Value]) -> Vec<Value> {
		self.key
			.iter()
			.filter_map(|key| {
				self.columns
					.iter()
					.position(|column| column == key)
					.map(|position| values[position].clone())
			})
			.collect()
	}

	/// The row as JSON, without its secret columns.
	fn row(&self, values: &[Value]) -> serde_json::Value {
		serde_json::Value::Object(
			self.columns
				.iter()
				.zip(values)
				.filter(|(column, _)| !self.secret.contains(column))
				.map(|(column, value)| ((*column).to_string(), json(value)))
				.collect(),
		)
	}
}

fn normalize<T: Table>(values: Vec<Value>) -> Result<Vec<Value>> {
	Ok(T::from_values(values)?.to_values()?)
}

/// One row's change, as values in [`Subject::columns`] order.
pub(crate) struct Change {
	pub(crate) op: AuditOp,
	pub(crate) before: Option<Vec<Value>>,
	pub(crate) after: Option<Vec<Value>>,
}

/// Records a change to one row of `T`, when `T` is audited.
pub(crate) async fn record<T: Table>(
	connection: &impl Connection,
	op: AuditOp,
	before: Option<&T>,
	after: Option<&T>,
) -> Result<()> {
	if !records::<T>() {
		return Ok(());
	}

	let change = Change {
		op,
		before: before.map(Table::to_values).transpose()?,
		after: after.map(Table::to_values).transpose()?,
	};
	record_changes(connection, Subject::of::<T>(), vec![change]).await
}

/// The most entries one statement inserts, well inside both databases'
/// limits on bound values.
const BATCH: usize = 200;

/// Records changes to rows of one table: in the audit log, in as few
/// statements as fit, and for live subscribers, once the transaction
/// commits.
pub(crate) async fn record_changes(
	connection: &impl Connection,
	subject: Subject,
	changes: Vec<Change>,
) -> Result<()> {
	let access = connection.access();
	let actor = access.actor().map(|actor| actor.uuid());
	let reason = access.reason().map(ToString::to_string);
	let at = crate::now();
	let name = (subject.table)();
	let table = name.to_string();
	let mut entries = Vec::with_capacity(changes.len());
	let mut published = Vec::new();

	for change in changes {
		let before = change.before.map(subject.normalize).transpose()?;
		let after = change.after.map(subject.normalize).transpose()?;
		let Some(values) = after.as_ref().or(before.as_ref()) else {
			continue;
		};

		if subject.live {
			published.push(crate::live::RowChange::new(
				&subject,
				name.clone(),
				change.op,
				before.as_deref(),
				after.as_deref(),
			));
		}

		if !subject.audit {
			continue;
		}

		let entry = AuditEntry {
			id: Id::new(),
			at,
			table_name: table.clone(),
			row_key: row_key_of(&subject.key_of(values)),
			op: change.op,
			actor,
			reason: reason.clone(),
			before: before.as_deref().map(|row| Json(subject.row(row))),
			after: after.as_deref().map(|row| Json(subject.row(row))),
		};
		entries.push(entry.to_values()?);
	}

	for batch in entries.chunks(BATCH) {
		let mut sql = Sql::new(connection.dialect());
		sql.push("INSERT INTO ")
			.table::<AuditEntry>()
			.push(" (")
			.columns(AuditEntry::COLUMNS)
			.push(") VALUES ");

		for (row, values) in batch.iter().enumerate() {
			sql.push(if row == 0 { "(" } else { ", (" });

			for (index, value) in values.iter().enumerate() {
				if index > 0 {
					sql.push(", ");
				}

				sql.value(value.clone());
			}

			sql.push(")");
		}

		connection.execute(sql.finish()).await?;
	}

	if !published.is_empty() {
		connection.publish(published).await?;
	}

	Ok(())
}

/// A row's key as the audit log stores it.
fn row_key_of(key: &[Value]) -> String {
	serde_json::Value::Array(key.iter().map(json).collect()).to_string()
}

/// A value as JSON: text for UUIDs, instants (RFC 3339), dates, and bytes
/// (hexadecimal); numbers that JSON can't hold, such as `NaN`, as text.
pub(crate) fn json(value: &Value) -> serde_json::Value {
	use serde_json::Value as Json;

	match value {
		Value::Null => Json::Null,
		Value::Bool(value) => Json::Bool(*value),
		Value::Int(value) => Json::from(*value),
		Value::Float(value) => {
			serde_json::Number::from_f64(*value)
				.map_or_else(|| Json::String(value.to_string()), Json::Number)
		}
		Value::Text(value) => Json::String(value.clone()),
		Value::Bytes(bytes) => {
			Json::String(bytes.iter().fold(String::new(), |mut hex, byte| {
				use std::fmt::Write;
				let _ = write!(hex, "{byte:02x}");
				hex
			}))
		}
		Value::Uuid(uuid) => Json::String(uuid.to_string()),
		Value::Timestamp(at) => {
			Json::String(at.to_rfc3339_opts(chrono::SecondsFormat::Millis, true))
		}
		Value::Date(date) => Json::String(date.to_string()),
		Value::Json(value) => value.clone(),
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn values_become_json_that_reads_back_the_same_everywhere() {
		let at = chrono::DateTime::parse_from_rfc3339("2026-10-30T12:00:00.123Z")
			.unwrap()
			.to_utc();
		let cases = [
			(Value::Null, serde_json::json!(null)),
			(Value::Bool(true), serde_json::json!(true)),
			(Value::Int(-4), serde_json::json!(-4)),
			(Value::Float(1.5), serde_json::json!(1.5)),
			(Value::Float(f64::NAN), serde_json::json!("NaN")),
			(Value::Text("hi".into()), serde_json::json!("hi")),
			(Value::Bytes(vec![0, 171]), serde_json::json!("00ab")),
			(
				Value::Uuid(Uuid::from_u128(1)),
				serde_json::json!("00000000-0000-0000-0000-000000000001"),
			),
			(
				Value::Timestamp(at),
				serde_json::json!("2026-10-30T12:00:00.123Z"),
			),
			(
				Value::Date(at.date_naive()),
				serde_json::json!("2026-10-30"),
			),
			(
				Value::Json(serde_json::json!({ "a": [1] })),
				serde_json::json!({ "a": [1] }),
			),
		];

		for (value, expected) in cases {
			assert_eq!(json(&value), expected, "{value:?}");
		}
	}

	#[test]
	fn rows_leave_out_secret_columns_and_keys_are_arrays() {
		let subject = Subject {
			table: || TableName::new("people"),
			columns: &["id", "name", "password_hash"],
			key: &["id"],
			secret: &["password_hash"],
			normalize: Ok,
			audit: true,
			live: false,
		};
		let values = [
			Value::Int(1),
			Value::Text("Ada".into()),
			Value::Text("hunter2".into()),
		];

		assert_eq!(
			subject.row(&values),
			serde_json::json!({ "id": 1, "name": "Ada" })
		);
		assert_eq!(row_key_of(&subject.key_of(&values)), "[1]");
		assert_eq!(
			format!("{:?}", AuditReader::everything()),
			"AuditReader { .. }"
		);
	}

	/// A table whose changes go to live subscribers only.
	#[crate::table(name = "live_things", live)]
	pub struct Thing {
		pub id: Id<Self>,
		#[db(secret)]
		pub token: String,
	}

	#[test]
	fn only_tables_that_record_changes_have_a_subject() {
		assert!(Subject::recorded::<AuditEntry>().is_none());
		let subject = Subject::recorded::<Thing>().unwrap();
		assert!(subject.live && !subject.audit);
		assert_eq!(subject.secret, ["token"]);
		assert_eq!((subject.table)(), TableName::new("live_things"));
		assert!(records::<Thing>() && !records::<AuditEntry>());

		let entry = table::TableEntry::of::<Thing>();
		assert_eq!((entry.name)(), TableName::new("live_things"));
		assert_eq!((entry.definition)().columns.len(), 2);
		assert!(entry.subject.live);
	}

	#[test]
	fn reading_the_log_needs_a_reader() {
		let columns = AuditEntry::columns();
		let condition = |cx: Context| {
			policy::condition::<AuditEntry>(&policy::Access::Context(cx), crate::Action::Select)
		};

		assert_eq!(condition(Context::new()), Some(Expr::never()));
		assert_eq!(
			condition(Context::new().with(AuditReader::everything())),
			Some(Expr::always())
		);
		let mine = AuditReader::matching(|entry, _| entry.table_name.eq("posts"));
		assert_eq!(
			condition(Context::new().with(mine)),
			Some(columns.table_name.eq("posts"))
		);

		for action in [
			crate::Action::Insert,
			crate::Action::Update,
			crate::Action::Delete,
		] {
			let cx = Context::new().with(AuditReader::everything());
			assert_eq!(
				policy::condition::<AuditEntry>(&policy::Access::Context(cx), action),
				Some(Expr::never()),
				"{action:?}"
			);
		}
	}
}
