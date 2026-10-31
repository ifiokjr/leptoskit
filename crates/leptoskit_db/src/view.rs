//! Views: read-only queries the database keeps under a name, declared with
//! `#[db::view]` and created by the migrations.

use leptoskit_db_schema::Generated;

use crate::Connection;
use crate::DbError;
use crate::Result;
use crate::Table;
use crate::connection::atomically;
use crate::sql::Sql;
use crate::sql::Statement;

/// A view declared with `#[db::view]`. The attribute implements it; you
/// don't. Read one with its `query()`, like a table; nothing writes to it.
pub trait View: Table {
	/// Whether it keeps its rows until refreshed.
	const MATERIALIZED: bool;

	/// The view's definition, as migrations see it.
	fn view() -> leptoskit_db_schema::View;
}

/// One view, registered by `#[db::view]` so that the schema can be exported
/// without listing views by hand.
#[doc(hidden)]
pub struct ViewEntry {
	definition: fn() -> leptoskit_db_schema::View,
}

impl ViewEntry {
	pub const fn of<V: View>() -> Self {
		Self {
			definition: V::view,
		}
	}
}

inventory::collect!(ViewEntry);

/// Every view linked into this program.
pub(crate) fn views() -> Vec<leptoskit_db_schema::View> {
	inventory::iter::<ViewEntry>()
		.map(|entry| (entry.definition)())
		.collect()
}

/// A view's definition from its columns, as `#[db::view]` declares them, and
/// its query.
#[doc(hidden)]
pub fn view_definition(
	table: leptoskit_db_schema::Table,
	postgres: &str,
	sqlite: &str,
	materialized: bool,
) -> leptoskit_db_schema::View {
	let mut view = leptoskit_db_schema::View::new(
		table.name,
		table.columns,
		Generated {
			postgres: postgres.to_string(),
			sqlite: sqlite.to_string(),
		},
	);
	view.materialized = materialized;
	view.comment = table.comment;
	view.indexes = table.indexes;
	view
}

/// Runs a materialized view's query again and keeps what it returns. On
/// Postgres that's `REFRESH MATERIALIZED VIEW`, `CONCURRENTLY` when asked;
/// on SQLite, which has none, the table that stands in for the view is
/// emptied and refilled in one transaction.
#[doc(hidden)]
pub async fn refresh<V: View>(connection: &impl Connection, concurrently: bool) -> Result<()> {
	if !V::MATERIALIZED {
		return Err(DbError::ReadOnly {
			table: V::flat_name(),
			message: format!(
				"`{}` isn't materialized, so it has nothing to refresh",
				V::flat_name()
			),
		});
	}

	let mut sql = Sql::new(connection.dialect());

	if connection.dialect() == crate::Dialect::Postgres {
		sql.push(if concurrently {
			"REFRESH MATERIALIZED VIEW CONCURRENTLY "
		} else {
			"REFRESH MATERIALIZED VIEW "
		})
		.table::<V>();

		if !connection.row_security() {
			connection.execute(sql.finish()).await?;
			return Ok(());
		}

		// Under row-level security the view would keep only the rows its
		// refresher may read; it keeps every row, as it does without.
		let bypass = |value: &str| {
			Statement {
				sql: "SELECT set_config($1, $2, true)".to_string(),
				args: vec![
					crate::Value::Text(leptoskit_db_schema::BYPASS_SETTING.to_string()),
					crate::Value::Text(value.to_string()),
				],
			}
		};
		let restore = match connection.access() {
			crate::policy::Access::System { .. } => "on",
			crate::policy::Access::Context(_) => "",
		};

		return atomically!(connection, |tx| {
			async {
				tx.execute(bypass("on")).await?;
				tx.execute(sql.finish()).await?;
				tx.execute(bypass(restore)).await?;
				Ok(())
			}
			.await
		});
	}

	let view = V::view();
	let mut fill = Sql::new(connection.dialect());
	fill.push("INSERT INTO ")
		.table::<V>()
		.push(" (")
		.columns(V::COLUMNS)
		.push(") ")
		.push(view.sql.sqlite.trim());
	sql.push("DELETE FROM ").table::<V>();

	atomically!(connection, |tx| {
		async {
			tx.execute(sql.finish()).await?;
			tx.execute(fill.finish()).await?;
			Ok(())
		}
		.await
	})
}
