//! Change capture: the objects the `live` schema pack adds to a schema, and
//! the triggers each live table gets.
//!
//! Every write to a live table, from the ORM, raw SQL, or another program,
//! inserts a row into `live.changes` in the writing transaction. Each
//! committing transaction that wrote one gets the next version in
//! `live.commits`:
//!
//! - **Postgres** registers the transaction in `live.pending` on its first
//!   write. A deferred constraint trigger on that table runs `live.seal()` at
//!   commit, which takes a transaction-scoped advisory lock, allocates the
//!   version, records the commit, and sends `NOTIFY leptoskit_live`. The lock
//!   is held until the commit finishes, so versions become visible in order.
//! - **SQLite** has one writer at a time, so ordering is free. Its triggers
//!   leave `version` empty, and `leptoskit_db` stamps the next version on every
//!   unversioned change just before it commits.
//!
//! The pack's tables are declared in `leptoskit_db`, which reads them; this
//! module renders what runs inside the database.

use crate::ColumnType;
use crate::Dialect;
use crate::Function;
use crate::Schema;
use crate::Sequence;
use crate::Table;
use crate::TableName;
use crate::Trigger;
use crate::TriggerAction;
use crate::TriggerEvent;
use crate::TriggerTiming;
use crate::quote_identifier as ident;
use crate::quote_literal as literal;
use crate::shorten_identifier;

/// The pack's namespace: the Postgres schema `live`, and the prefix `live_`
/// on SQLite.
pub const NAMESPACE: &str = "live";

/// The table with one row per committed transaction that wrote a live table.
pub const COMMITS: &str = "commits";

/// The table with one row per written row of a live table.
pub const CHANGES: &str = "changes";

/// Postgres only: the transactions waiting for a version at commit.
pub const PENDING: &str = "pending";

/// The channel `live.seal()` notifies with each new version.
pub const CHANNEL: &str = "leptoskit_live";

/// The transaction-scoped advisory lock that orders versions by commit
/// (`lkdblive` in ASCII).
pub const COMMIT_LOCK: i64 = 0x6c6b_6462_6c69_7665;

/// The transaction-scoped advisory lock a server holds while it prunes the
/// log, so that servers sharing a database take turns (`lkdbprun` in ASCII).
pub const PRUNE_LOCK: i64 = 0x6c6b_6462_7072_756e;

/// The setting `live.seal()` reads the commit's actor from: a UUID.
pub const ACTOR_SETTING: &str = "leptoskit.user_id";

/// The setting `live.seal()` reads the commit's origin from, such as an
/// operation's name. Without one, a commit's origin is `sql`.
pub const ORIGIN_SETTING: &str = "leptoskit.origin";

/// The origin of a commit that didn't say where it came from.
pub const DEFAULT_ORIGIN: &str = "sql";

/// The name of the Postgres trigger on each live table.
pub const POSTGRES_TRIGGER: &str = "live_capture";

/// A table of the pack, by its own name.
pub fn table(name: &str) -> TableName {
	TableName::namespaced(NAMESPACE, name)
}

/// The capture function every live table's trigger calls on Postgres.
pub fn capture_function() -> TableName {
	table("capture")
}

/// The function that seals a transaction at commit on Postgres.
pub fn seal_function() -> TableName {
	table("seal")
}

/// The sequence versions come from on Postgres.
pub fn version_sequence() -> TableName {
	table("version_seq")
}

/// The sequence that orders changes on Postgres.
pub fn change_sequence() -> TableName {
	table("change_seq")
}

/// The pack's own sequences, functions, and triggers for `dialect`: the
/// capture and seal functions on Postgres, and nothing on SQLite, whose
/// triggers hold their own statements.
pub fn pack(dialect: Dialect) -> (Vec<Sequence>, Vec<Function>, Vec<Trigger>) {
	match dialect {
		Dialect::Sqlite => (Vec::new(), Vec::new(), Vec::new()),
		Dialect::Postgres => {
			let sequences = vec![
				Sequence::new(change_sequence()),
				Sequence::new(version_sequence()),
			];
			let functions = vec![
				Function::new(capture_function(), "trigger", "plpgsql", capture_body())
					.security_definer(),
				Function::new(seal_function(), "trigger", "plpgsql", seal_body())
					.security_definer(),
			];
			let seal = Trigger {
				name: "seal".to_string(),
				table: table(PENDING),
				timing: TriggerTiming::After,
				events: vec![TriggerEvent::Insert],
				deferred: true,
				columns: Vec::new(),
				action: TriggerAction::Function {
					function: seal_function(),
					arguments: Vec::new(),
				},
			};
			(sequences, functions, vec![seal])
		}
	}
}

/// The triggers that capture writes to `table`, copying `columns`: its live
/// columns, without `secret` and `live = skip` ones.
///
/// On Postgres, one trigger passes the table's name, its key columns, and
/// `columns` to `live.capture()`. On SQLite, which has no generic row to
/// JSON function, three triggers list the columns themselves.
pub fn capture(dialect: Dialect, table: &Table, columns: &[&str]) -> Vec<Trigger> {
	let columns: Vec<String> = columns.iter().map(ToString::to_string).collect();

	match dialect {
		Dialect::Postgres => {
			let mut arguments = vec![table.name.to_string(), table.primary_key.join(",")];
			arguments.extend(columns.iter().cloned());

			vec![Trigger {
				name: POSTGRES_TRIGGER.to_string(),
				table: table.name.clone(),
				timing: TriggerTiming::After,
				events: vec![
					TriggerEvent::Insert,
					TriggerEvent::Update,
					TriggerEvent::Delete,
				],
				deferred: false,
				columns,
				action: TriggerAction::Function {
					function: capture_function(),
					arguments,
				},
			}]
		}
		Dialect::Sqlite => {
			[
				TriggerEvent::Insert,
				TriggerEvent::Update,
				TriggerEvent::Delete,
			]
			.into_iter()
			.map(|event| sqlite_trigger(table, &columns, event))
			.collect()
		}
	}
}

/// The columns `table`'s capture triggers copy, when it has them.
pub fn captured(schema: &Schema, table: &TableName) -> Option<Vec<String>> {
	schema
		.triggers_on(table)
		.find(|trigger| is_capture(trigger))
		.map(|trigger| trigger.columns.clone())
}

/// Whether `trigger` is one of a live table's capture triggers.
pub fn is_capture(trigger: &Trigger) -> bool {
	match &trigger.action {
		TriggerAction::Function { function, .. } => *function == capture_function(),
		TriggerAction::Statements { sql, .. } => {
			trigger.name.starts_with("live_") && sql.contains(&ident(&table(CHANGES).flat()))
		}
	}
}

/// The SQLite trigger for one event: it inserts a change with no version,
/// which the commit fills in.
fn sqlite_trigger(table: &Table, columns: &[String], event: TriggerEvent) -> Trigger {
	let (op, suffix, key_row) = match event {
		TriggerEvent::Insert => ("I", "insert", "NEW"),
		TriggerEvent::Update => ("U", "update", "NEW"),
		TriggerEvent::Delete => ("D", "delete", "OLD"),
	};
	let value = |row: &str, column: &str| sqlite_value(table, row, column);
	let key: Vec<String> = table
		.primary_key
		.iter()
		.map(|column| value(key_row, column))
		.collect();
	let image = |row: &str| {
		let pairs: Vec<String> = columns
			.iter()
			.map(|column| format!("{}, {}", literal(column), value(row, column)))
			.collect();
		format!("json_object({})", pairs.join(", "))
	};
	let old = match event {
		TriggerEvent::Insert => "NULL".to_string(),
		_ => image("OLD"),
	};
	let new = match event {
		TriggerEvent::Delete => "NULL".to_string(),
		_ => image("NEW"),
	};

	// Every column, secret or not, is compared and named, but only the
	// live columns' values are copied.
	let mut every: Vec<&str> = table
		.columns
		.iter()
		.map(|column| column.name.as_str())
		.collect();
	every.sort_unstable();
	let differs = |column: &str| format!("OLD.{0} IS NOT NEW.{0}", ident(column));
	let changed = match event {
		TriggerEvent::Update => {
			let names: Vec<String> = every
				.iter()
				.map(|column| format!("CASE WHEN {} THEN {} END", differs(column), literal(column)))
				.collect();
			format!(
				"(SELECT json_group_array(\"value\") FROM json_each(json_array({})) WHERE \
				 \"type\" <> 'null')",
				names.join(", ")
			)
		}
		_ => "'[]'".to_string(),
	};
	let changes_table = ident(&self::table(CHANGES).flat());
	let sql = format!(
		"INSERT INTO {changes_table} (\"seq\", \"txid\", \"version\", \"table_name\", \"op\", \
		 \"key\", \"old\", \"new\", \"changed\") VALUES ((SELECT coalesce(max(\"seq\"), 0) + 1 \
		 FROM {changes_table}), NULL, NULL, {}, '{op}', json_array({}), {old}, {new}, {changed});",
		literal(&table.name.to_string()),
		key.join(", "),
	);

	Trigger {
		name: shorten_identifier(&format!("live_{}_{suffix}", table.name.flat())),
		table: table.name.clone(),
		timing: TriggerTiming::After,
		events: vec![event],
		deferred: false,
		columns: columns.to_vec(),
		action: TriggerAction::Statements { when: None, sql },
	}
}

/// A column of `row` as SQLite's JSON functions can hold it: blobs, which
/// they refuse, as lowercase hex, and JSON as JSON rather than text.
fn sqlite_value(table: &Table, row: &str, column: &str) -> String {
	let value = format!("{row}.{}", ident(column));

	match table.column(column).map(|column| &column.ty) {
		Some(
			ColumnType::Uuid
			| ColumnType::Bytes
			| ColumnType::Geometry { .. }
			| ColumnType::Geography { .. },
		) => {
			format!("CASE WHEN {value} IS NULL THEN NULL ELSE lower(hex({value})) END")
		}
		Some(ColumnType::Json) => format!("json({value})"),
		_ => value,
	}
}

/// `live.capture()`: records one written row. Its arguments are the table's
/// name, its key columns joined by `,`, and then the columns to copy.
///
/// Every written row is recorded, even by an update that changes nothing,
/// since the write itself is what subscribers hear of. For an update,
/// `changed` names every column whose value changed, `secret` ones
/// included, so that a query filtering on one still re-runs; only the
/// listed columns' values are copied.
fn capture_body() -> String {
	format!(
		"DECLARE
	captured_table text := TG_ARGV[0];
	key_columns text[] := string_to_array(TG_ARGV[1], ',');
	copied_columns text[] := TG_ARGV[2:TG_NARGS - 1];
	current_txid bigint := pg_current_xact_id()::text::bigint;
	old_row jsonb;
	new_row jsonb;
	changed_columns jsonb := '[]'::jsonb;
BEGIN
	IF TG_OP <> 'INSERT' THEN
		old_row := to_jsonb(OLD);
	END IF;

	IF TG_OP <> 'DELETE' THEN
		new_row := to_jsonb(NEW);
	END IF;

	IF TG_OP = 'UPDATE' THEN
		SELECT coalesce(jsonb_agg(entry.key ORDER BY entry.key), '[]'::jsonb)
		INTO changed_columns
		FROM jsonb_each(new_row) AS entry
		WHERE entry.value IS DISTINCT FROM old_row -> entry.key;
	END IF;

	INSERT INTO {changes} (\"seq\", \"txid\", \"version\", \"table_name\", \"op\", \"key\", \"old\", \
		 \"new\", \"changed\")
	VALUES (
		nextval('{change_seq}'),
		current_txid,
		NULL,
		captured_table,
		left(TG_OP, 1),
		(SELECT jsonb_agg(coalesce(new_row, old_row) -> part.name ORDER BY part.position)
			FROM unnest(key_columns) WITH ORDINALITY AS part(name, position)),
		(SELECT jsonb_object_agg(copied.name, old_row -> copied.name)
			FROM unnest(copied_columns) AS copied(name) WHERE old_row IS NOT NULL),
		(SELECT jsonb_object_agg(copied.name, new_row -> copied.name)
			FROM unnest(copied_columns) AS copied(name) WHERE new_row IS NOT NULL),
		changed_columns
	);

	-- The first write in this transaction registers it for a version at commit.
	INSERT INTO {pending} (txid) VALUES (current_txid) ON CONFLICT DO NOTHING;
	RETURN NULL;
END;",
		changes = postgres_name(CHANGES),
		pending = postgres_name(PENDING),
		change_seq = postgres_name("change_seq"),
	)
}

/// `live.seal()`: runs at commit, once for each transaction that wrote a
/// live table, after the deferred checks queued before its first write.
///
/// The advisory lock is transaction-scoped, so a transaction holds it from
/// taking its version until it has committed, and the next one takes its
/// version only after that: no version becomes visible after a higher one.
/// A transaction that fails a later check rolls back, leaving a hole in the
/// sequence, and its notification is discarded with it.
fn seal_body() -> String {
	format!(
		"DECLARE
	next_version bigint;
BEGIN
	DELETE FROM {pending} WHERE txid = NEW.txid;

	-- Sealed already: `SET CONSTRAINTS ALL IMMEDIATE` ran this before a later
	-- write registered the transaction again. Its changes join the version it has.
	IF EXISTS (SELECT 1 FROM {commits} WHERE txid = NEW.txid) THEN
		RETURN NULL;
	END IF;

	PERFORM pg_advisory_xact_lock({lock});
	next_version := nextval('{version_seq}');
	INSERT INTO {commits} (version, txid, committed_at, actor, origin)
	VALUES (
		next_version,
		NEW.txid,
		clock_timestamp(),
		nullif(current_setting('{actor}', true), '')::uuid,
		coalesce(nullif(current_setting('{origin}', true), ''), '{default_origin}')
	);
	PERFORM pg_notify('{channel}', next_version::text);
	RETURN NULL;
END;",
		pending = postgres_name(PENDING),
		commits = postgres_name(COMMITS),
		version_seq = postgres_name("version_seq"),
		lock = COMMIT_LOCK,
		actor = ACTOR_SETTING,
		origin = ORIGIN_SETTING,
		default_origin = DEFAULT_ORIGIN,
		channel = CHANNEL,
	)
}

/// A pack object's name as Postgres SQL writes it.
fn postgres_name(name: &str) -> String {
	format!("{NAMESPACE}.{name}")
}
