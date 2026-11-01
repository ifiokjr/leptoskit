//! Keeping vectors fresh: which rows' vectors were made from text that has
//! changed since, and a job that makes them again with any model.
//!
//! A vector column with `source(…)` keeps, beside it, the fingerprint of
//! the sources its value was made from, which the database's triggers set
//! whenever a write changes the vector. A write that changes only the
//! sources leaves the fingerprint as it was, so the vector is stale until
//! it's made again.

use std::future::Future;

use leptoskit_db_schema::Dialect;
use leptoskit_db_schema::source_fingerprint;
use leptoskit_db_schema::source_fingerprint_sql;

use super::column::VectorColumn;
use super::column::VectorValue;
use crate::Col;
use crate::Column;
use crate::Connection;
use crate::DbError;
use crate::Expr;
use crate::Query;
use crate::Result;
use crate::Table;
use crate::Update;
use crate::Value;
use crate::policy;
use crate::sql::Sql;

/// Makes vectors from text, with whatever model an app chooses: one on
/// the same machine, a self-hosted one, or a provider's. leptoskit wires
/// none in.
///
/// ```ignore
/// struct OpenAi { client: reqwest::Client, key: String }
///
/// impl Embedder<Vector<1536>> for OpenAi {
/// 	type Error = reqwest::Error;
///
/// 	async fn embed(&self, texts: &[String]) -> Result<Vec<Vector<1536>>, Self::Error> {
/// 		// One request for every text, one vector back for each, in order.
/// 	}
/// }
///
/// Article::stale_embeddings()
/// 	.limit(100)
/// 	.embed_stale(|a| a.embedding, &model, &db)
/// 	.await?;
/// ```
pub trait Embedder<V: VectorValue>: Send + Sync {
	/// Why the model couldn't make the vectors.
	type Error: std::error::Error + Send + Sync + 'static;

	/// One vector for each text, in the same order.
	fn embed(&self, texts: &[String]) -> impl Future<Output = Result<Vec<V>, Self::Error>> + Send;
}

/// A condition that a vector column's value is stale: missing, or made
/// from sources that have changed since.
#[derive(Clone, Debug, PartialEq)]
#[doc(hidden)]
pub struct Stale {
	column: &'static str,
	/// The column keeping the sources' fingerprint.
	source: String,
	sources: Vec<String>,
}

impl Stale {
	/// `vector IS NULL OR fingerprint IS NULL OR fingerprint <> the
	/// sources' fingerprint now`.
	pub(crate) fn write(&self, sql: &mut Sql) {
		let now = source_fingerprint_sql(sql.dialect(), None, &self.sources);

		let differs = match sql.dialect() {
			Dialect::Postgres => " IS DISTINCT FROM ",
			Dialect::Sqlite => " IS NOT ",
		};

		sql.push("(")
			.identifier(self.column)
			.push(" IS NULL OR ")
			.identifier(&self.source)
			.push(differs)
			.push(&now)
			.push(")");
	}
}

/// The `source(…)` a table's vector column is made from, or why it has
/// none.
fn sources_of<T: Table>(column: &str) -> std::result::Result<(String, Vec<String>), String> {
	T::definition()
		.vectors
		.into_iter()
		.find(|vector| vector.column == column && !vector.sources.is_empty())
		.map(|vector| (vector.source_column(), vector.sources))
		.ok_or_else(|| {
			format!(
				"`{}.{column}` isn't made from sources, so it can't be stale; add `source(…)` to \
				 its `#[db(vector(…))]`",
				T::flat_name()
			)
		})
}

impl<T: Table, V: VectorColumn> Col<T, V> {
	/// The vector is stale: the row has none yet, or its sources have
	/// changed since it was made. Stale rows are still found by
	/// [`Query::nearest`]; leave them out with `!a.embedding.is_stale()`.
	pub fn is_stale(&self) -> Expr {
		match sources_of::<T>(self.name()) {
			Ok((source, sources)) => {
				Expr::Stale(Box::new(Stale {
					column: self.name(),
					source,
					sources,
				}))
			}
			Err(problem) => Expr::Invalid(problem),
		}
	}
}

impl<T: Table> Query<T> {
	/// Makes the stale vectors of the matching rows again with `embedder`,
	/// up to the query's limit, and stores them, returning how many rows it
	/// embedded:
	///
	/// ```ignore
	/// Article::stale_embeddings().limit(100).embed_stale(|a| a.embedding, &model, &db).await?;
	/// ```
	///
	/// Each row's text is its sources, in their declared order, joined by
	/// blank lines, leaving out empty ones. Rows are read and written as the
	/// connection's policies allow, and each write records the fingerprint
	/// of the text it was made from, so a row whose text changes meanwhile
	/// stays stale. The writes skip the table's hooks.
	pub async fn embed_stale<V, E>(
		self,
		column: impl FnOnce(&T::Columns) -> Col<T, V>,
		embedder: &E,
		connection: &impl Connection,
	) -> Result<u64>
	where
		V: VectorColumn,
		E: Embedder<V::Value>,
	{
		let column = column(T::columns());
		let (source, sources) =
			sources_of::<T>(column.name()).map_err(|problem| crate::expr::invalid(&problem))?;
		let rows = self.filter(|_| column.is_stale()).all(connection).await?;

		if rows.is_empty() {
			return Ok(0);
		}

		let positions: Vec<usize> = sources
			.iter()
			.map(|name| {
				T::COLUMNS
					.iter()
					.position(|column| column == name)
					.ok_or_else(|| crate::expr::invalid(&format!("`{name}` isn't a column")))
			})
			.collect::<Result<_>>()?;
		let mut keys = Vec::with_capacity(rows.len());
		let mut texts = Vec::with_capacity(rows.len());
		let mut fingerprints = Vec::with_capacity(rows.len());

		for row in &rows {
			let values = row.to_values()?;
			let sources: Vec<Option<&str>> = positions
				.iter()
				.map(|position| {
					match &values[*position] {
						Value::Text(text) => Some(text.as_str()),
						_ => None,
					}
				})
				.collect();
			texts.push(
				sources
					.iter()
					.flatten()
					.filter(|text| !text.is_empty())
					.copied()
					.collect::<Vec<_>>()
					.join("\n\n"),
			);
			fingerprints.push(source_fingerprint(&sources));
			keys.push(policy::key_of::<T>(&values));
		}

		let vectors = embedder
			.embed(&texts)
			.await
			.map_err(|error| DbError::Embedder(Box::new(error)))?;

		if vectors.len() != texts.len() {
			return Err(DbError::Embedder(
				format!(
					"the embedder made {} vectors for {} texts",
					vectors.len(),
					texts.len()
				)
				.into(),
			));
		}

		let mut written = 0;

		for ((key, fingerprint), vector) in keys.into_iter().zip(fingerprints).zip(vectors) {
			let row = policy::keys_condition::<T>(&[key]);
			written += Update::<T>::new()
				.filter(|_| row)
				.set_named(column.name(), vector.to_value()?)
				.set_named(source.clone(), Value::Text(fingerprint))
				.allow_skipping_hooks()
				.run(connection)
				.await?;
		}

		Ok(written)
	}
}
