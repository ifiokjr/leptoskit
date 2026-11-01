//! Hybrid search: full-text ranking and vector similarity, fused.

use std::collections::HashMap;

use super::column::VectorColumn;
use super::column::VectorValue;
use super::nearest::Nearest;
use crate::Col;
use crate::Connection;
use crate::Expr;
use crate::Query;
use crate::Result;
use crate::Searchable;
use crate::Table;
use crate::policy;

/// How much a rank counts in reciprocal rank fusion: a row's score is the
/// sum, over the rankings it's in, of `1 / (K + rank)`. 60 is the value the
/// method's authors found best, and the one most search engines use.
const K: f64 = 60.0;

/// How many rows each ranking contributes at least.
const MIN_CANDIDATES: u64 = 50;

impl<T: Searchable> Query<T> {
	/// The matching rows ranked by full-text relevance and vector
	/// similarity together, best first:
	///
	/// ```ignore
	/// let results = Article::query()
	/// 	.hybrid(|h| h.text("passkeys in rust").vector(|a| a.embedding, &query_embedding))
	/// 	.limit(10)
	/// 	.all(&db)
	/// 	.await?;
	/// ```
	///
	/// The two rankings are fused by reciprocal rank fusion, so a row near
	/// the top of either comes near the top, and one high in both comes
	/// first; their scores, which can't be compared, play no part. Each
	/// ranking is [`Query::search`] and [`Query::nearest`] as they are
	/// alone, with the query's filters, `select` policy, and soft-delete
	/// filter, and contributes its best
	/// [`candidates`](Hybrid::candidates) rows.
	pub fn hybrid(self, build: impl FnOnce(Hybrid<T>) -> Hybrid<T>) -> Hybrid<T> {
		build(Hybrid {
			query: self,
			text: None,
			vector: None,
			candidates: None,
		})
	}
}

/// A hybrid search of table `T`. Start one with [`Query::hybrid`].
#[must_use = "a search does nothing until it runs"]
pub struct Hybrid<T> {
	query: Query<T>,
	text: Option<String>,
	vector: Option<(&'static str, std::result::Result<Vec<u8>, String>)>,
	candidates: Option<u64>,
}

/// One row a hybrid search found, with how it ranked.
#[derive(Debug)]
pub struct HybridHit<T> {
	/// The row.
	pub row: T,
	/// Its fused score: higher is better.
	pub score: f64,
	/// Its place in the full-text ranking, from 1, if it's there.
	pub text_rank: Option<usize>,
	/// Its place in the vector ranking, from 1, if it's there.
	pub vector_rank: Option<usize>,
	/// Its vector's distance from the target, if it's in that ranking.
	pub distance: Option<f32>,
}

impl<T: Searchable> Hybrid<T> {
	/// Ranks rows by how well their searched columns match `text`, as
	/// [`Query::search`] reads it.
	pub fn text(mut self, text: &str) -> Self {
		self.text = Some(text.to_string());
		self
	}

	/// Ranks rows by how near their vectors in `column` are `target`, as
	/// [`Query::nearest`] measures them.
	pub fn vector<V: VectorColumn>(
		mut self,
		column: impl FnOnce(&T::Columns) -> Col<T, V>,
		target: &V::Value,
	) -> Self {
		self.vector = Some((column(T::columns()).name(), target.to_bytes()));
		self
	}

	/// How many rows each ranking contributes: by default twice the rows
	/// asked for, and at least 50.
	pub fn candidates(mut self, candidates: u64) -> Self {
		self.candidates = Some(candidates);
		self
	}

	/// Keeps only rows matching `condition` as well.
	pub fn filter(mut self, condition: impl FnOnce(&T::Columns) -> Expr) -> Self {
		self.query = self.query.filter(condition);
		self
	}

	/// At most this many rows.
	pub fn limit(mut self, limit: u64) -> Self {
		self.query = self.query.limit(limit);
		self
	}

	/// Skips this many rows.
	pub fn offset(mut self, offset: u64) -> Self {
		self.query = self.query.offset(offset);
		self
	}

	/// Includes soft-deleted rows.
	pub fn with_deleted(mut self) -> Self {
		self.query = self.query.with_deleted();
		self
	}

	/// The rows, best first.
	pub async fn all(self, connection: &impl Connection) -> Result<Vec<T>> {
		Ok(self
			.hits(connection)
			.await?
			.into_iter()
			.map(|hit| hit.row)
			.collect())
	}

	/// The rows, best first, each with how it ranked.
	pub async fn hits(self, connection: &impl Connection) -> Result<Vec<HybridHit<T>>> {
		if self.text.is_none() && self.vector.is_none() {
			return Err(crate::expr::invalid(
				"a hybrid search ranks by text, a vector, or both; give it `text(…)` or \
				 `vector(…)`",
			));
		}

		let offset = self.query.offset.unwrap_or(0);
		let wanted = self.query.limit.map(|limit| limit.saturating_add(offset));
		let candidates = self.candidates.unwrap_or_else(|| {
			wanted
				.map_or(MIN_CANDIDATES, |wanted| wanted.saturating_mul(2))
				.max(MIN_CANDIDATES)
		});
		let mut base = self.query.clone();
		base.limit = Some(candidates);
		base.offset = None;
		base.order.clear();

		let mut fused = Fused::default();

		if let Some(text) = &self.text {
			let hits = base.clone().search(text).hits(connection).await?;

			for (rank, hit) in hits.into_iter().enumerate() {
				fused.add(hit.row, rank + 1, None)?;
			}
		}

		if let Some((column, target)) = self.vector {
			let rows = Nearest::<T, ()>::stored(base, column, target)
				.rows(connection)
				.await?;

			for (rank, (row, distance)) in rows.into_iter().enumerate() {
				fused.add(row, rank + 1, Some(distance))?;
			}
		}

		let mut hits = fused.hits;
		hits.sort_by(|left, right| {
			right
				.score
				.total_cmp(&left.score)
				.then_with(|| best(left).cmp(&best(right)))
		});

		Ok(hits
			.into_iter()
			.skip(usize::try_from(offset).unwrap_or(usize::MAX))
			.take(self.query.limit.map_or(usize::MAX, |limit| {
				usize::try_from(limit).unwrap_or(usize::MAX)
			}))
			.collect())
	}
}

/// The better of a hit's two places, which breaks ties in score.
fn best<T>(hit: &HybridHit<T>) -> usize {
	hit.text_rank
		.into_iter()
		.chain(hit.vector_rank)
		.min()
		.unwrap_or(usize::MAX)
}

/// The rankings fused so far, each row once, by key.
struct Fused<T> {
	hits: Vec<HybridHit<T>>,
	/// Each row's position in `hits`, by its key's debug text: `Value`
	/// floats aren't `Hash`.
	by_key: HashMap<String, usize>,
}

impl<T> Default for Fused<T> {
	fn default() -> Self {
		Self {
			hits: Vec::new(),
			by_key: HashMap::new(),
		}
	}
}

impl<T: Table> Fused<T> {
	/// Counts `row` at `rank` in the text ranking, or, with its distance,
	/// in the vector ranking.
	fn add(&mut self, row: T, rank: usize, distance: Option<f32>) -> Result<()> {
		let key = format!("{:?}", policy::key_of::<T>(&row.to_values()?));
		let score = 1.0 / (K + rank as f64);
		let position = *self.by_key.entry(key).or_insert_with(|| {
			self.hits.push(HybridHit {
				row,
				score: 0.0,
				text_rank: None,
				vector_rank: None,
				distance: None,
			});
			self.hits.len() - 1
		});
		let hit = &mut self.hits[position];
		hit.score += score;

		if distance.is_some() {
			hit.vector_rank = Some(rank);
			hit.distance = distance;
		} else {
			hit.text_rank = Some(rank);
		}

		Ok(())
	}
}
