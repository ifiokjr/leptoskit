//! Checking live changes against a subscriber's policy: the rows a change
//! saw, which aren't stored any more, read as if they were.

use leptoskit_db_schema::Dialect;

use crate::Action;
use crate::Result;
use crate::Table;
use crate::Value;
use crate::policy::Access;
use crate::sql::Sql;
use crate::sql::Statement;

/// The column that numbers each image in [`visible_images`]'s statements.
const IMAGE: &str = "lk_live_image";

/// The alias the images take, as rows of `T`.
const IMAGES: &str = "lk_live";

/// The most values one statement binds, well inside both databases' limits.
const MAX_VALUES: usize = 20_000;

/// How many of the rows `images` stands for meet `condition`: `SELECT
/// count(*) FROM (SELECT … AS col, …) AS lk_live WHERE condition`. Checks a
/// row that isn't stored, such as one about to be inserted.
#[cfg(feature = "postgres")]
pub(crate) fn image_count<T: Table>(
	dialect: Dialect,
	image: &[Value],
	condition: &crate::Expr,
) -> Result<Statement> {
	let types = column_types::<T>();
	let mut sql = Sql::new(dialect);
	sql.root_alias(&T::table_name(), IMAGES);
	sql.push("SELECT count(*) FROM (SELECT ");
	write_image::<T>(&mut sql, image, &types);
	sql.push(") AS ").identifier(IMAGES).push(" WHERE (");
	condition.write(&mut sql)?;
	sql.push(")");
	Ok(sql.finish())
}

/// Each column's Postgres type, which an image's values are cast to.
fn column_types<T: Table>() -> Vec<Option<String>> {
	let definition = T::definition();
	T::COLUMNS
		.iter()
		.map(|name| {
			definition
				.column(name)
				.map(|column| leptoskit_db_schema::postgres_type(&column.ty))
		})
		.collect()
}

/// `value AS column, …` for one image, cast on Postgres.
fn write_image<T: Table>(sql: &mut Sql, image: &[Value], types: &[Option<String>]) {
	let dialect = sql.dialect();

	for (index, ((column, value), ty)) in T::COLUMNS.iter().zip(image).zip(types).enumerate() {
		if index > 0 {
			sql.push(", ");
		}

		match (dialect, ty) {
			(Dialect::Postgres, Some(ty)) => {
				sql.push("CAST(")
					.value(value.clone())
					.push(" AS ")
					.push(ty)
					.push(")");
			}
			_ => {
				sql.value(value.clone());
			}
		}

		sql.push(" AS ").identifier(column);
	}
}

/// Statements that return the number of each image, a row of `T`, that
/// `access` may read: `SELECT image FROM (SELECT 0 AS image, … UNION ALL
/// …) AS lk_live WHERE policy`. `None` when nothing restricts reading `T`,
/// so every image is readable. Images are checked against the policy
/// alone: a soft-deleted row is one its readers could ask for.
pub(crate) fn visible_images<T: Table>(
	access: &Access,
	dialect: Dialect,
	images: &[&[Value]],
) -> Result<Option<Vec<Statement>>> {
	let Some(policy) = crate::policy::condition::<T>(access, Action::Select) else {
		return Ok(None);
	};
	let types = column_types::<T>();
	let per_statement = (MAX_VALUES / T::COLUMNS.len().max(1)).max(1);
	let mut statements = Vec::new();

	for (chunk, batch) in images.chunks(per_statement).enumerate() {
		let mut sql = Sql::new(dialect);
		sql.root_alias(&T::table_name(), IMAGES);
		sql.push("SELECT ").identifier(IMAGE).push(" FROM (");

		for (offset, image) in batch.iter().enumerate() {
			if offset > 0 {
				sql.push(" UNION ALL ");
			}

			let number = chunk * per_statement + offset;
			sql.push(&format!("SELECT {number} AS "))
				.identifier(IMAGE)
				.push(", ");
			write_image::<T>(&mut sql, image, &types);
		}

		sql.push(") AS ").identifier(IMAGES).push(" WHERE (");
		policy.write(&mut sql)?;
		sql.push(")");
		statements.push(sql.finish());
	}

	Ok(Some(statements))
}
