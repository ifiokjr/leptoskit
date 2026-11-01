//! What leptoskit's `DevTools` read from a cache, and the edits they make.
//!
//! The `devtools` feature gives an agent that knows none of the app's types
//! type-erased access to every cached query and mutation.
//! [`QueryClient::inspect_queries`] lists each query with the Rust type of its
//! data and the data encoded by its scope's codec, and an [`InspectedQuery`]
//! invalidates, refetches, and replaces that data. Data travels only through a
//! codec, the one dehydration uses ([`QueryScope::with_ssr`] or
//! [`QueryScope::with_codec`]), so a query whose scope has none is read-only.
//!
//! [`QueryScope::with_ssr`]: crate::QueryScope::with_ssr
//! [`QueryScope::with_codec`]: crate::QueryScope::with_codec

use std::fmt;
use std::sync::Arc;

use crate::client::QueryClient;
use crate::filters::QueryFilter;
use crate::mutation_cache::AnyMutation;
use crate::mutation_cache::MutationInfo;
use crate::query::AnyQuery;
use crate::query::QueryInfo;

/// One cached query, for leptoskit's `DevTools`.
#[derive(Clone)]
pub struct InspectedQuery {
	client: QueryClient,
	query: Arc<dyn AnyQuery>,
}

/// One cached mutation, for leptoskit's `DevTools`.
#[derive(Clone)]
pub struct InspectedMutation {
	mutation: Arc<dyn AnyMutation>,
}

/// Why the `DevTools` could not replace a query's data.
#[derive(Debug)]
pub enum EditError {
	/// The scope has no codec, so JSON cannot become its data.
	NoCodec,
	/// The scope's codec refused the JSON.
	Decode(serde_json::Error),
}

impl fmt::Display for EditError {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		match self {
			Self::NoCodec => {
				f.write_str(
					"the query's scope has no codec, so its data is read-only; give the scope \
					 `.with_ssr()` or `.with_codec(..)` to edit it",
				)
			}
			Self::Decode(error) => write!(f, "the query's codec refused the data: {error}"),
		}
	}
}

impl std::error::Error for EditError {
	fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
		match self {
			Self::NoCodec => None,
			Self::Decode(error) => Some(error),
		}
	}
}

impl QueryClient {
	/// Every cached query, for leptoskit's `DevTools`.
	#[must_use]
	pub fn inspect_queries(&self) -> Vec<InspectedQuery> {
		self.cache()
			.all()
			.into_iter()
			.map(|query| {
				InspectedQuery {
					client: self.clone(),
					query,
				}
			})
			.collect()
	}

	/// Every cached mutation, oldest first, for leptoskit's `DevTools`.
	#[must_use]
	pub fn inspect_mutations(&self) -> Vec<InspectedMutation> {
		self.mutations()
			.all()
			.into_iter()
			.map(|mutation| InspectedMutation { mutation })
			.collect()
	}
}

impl InspectedQuery {
	/// What [`QueryClient::query_infos`] reports for this query.
	#[must_use]
	pub fn info(&self) -> QueryInfo {
		self.query.info()
	}

	/// The Rust type of the query's data, as [`std::any::type_name`] writes
	/// it.
	#[must_use]
	pub fn data_type(&self) -> &'static str {
		self.query.data_type()
	}

	/// Whether the scope has a codec, so the data can be shown and replaced.
	#[must_use]
	pub fn has_codec(&self) -> bool {
		self.query.has_codec()
	}

	/// The data, encoded by the scope's codec: `None` when the scope has no
	/// codec or the query has no data.
	#[must_use]
	pub fn encode(&self) -> Option<Result<serde_json::Value, serde_json::Error>> {
		self.query.encode_data()
	}

	/// Replaces the data with `json` decoded by the scope's codec, as
	/// [`QueryClient::set_query_data`] would: every observer sees it at once.
	///
	/// # Errors
	///
	/// [`EditError::NoCodec`] when the scope has no codec, and
	/// [`EditError::Decode`] when the codec refuses the JSON.
	pub fn set_json(&self, json: serde_json::Value) -> Result<(), EditError> {
		self.query.set_json(json)
	}

	/// Marks the query stale and refetches it if something observes it, as
	/// [`QueryClient::invalidate_query`] does.
	pub fn invalidate(&self) {
		self.client
			.invalidate_queries(QueryFilter::exactly(&self.query));
	}

	/// Refetches the query now, restarting a fetch in flight.
	///
	/// Unlike [`QueryClient::refetch_queries`], this also refetches a
	/// disabled or static query: the `DevTools` ask for this query by name,
	/// the way TanStack Query's devtools fetch the query they show.
	pub fn refetch(&self) {
		drop(self.query.refetch(true));
	}
}

impl fmt::Debug for InspectedQuery {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		f.debug_struct("InspectedQuery")
			.field("info", &self.info())
			.field("data_type", &self.data_type())
			.finish_non_exhaustive()
	}
}

impl InspectedMutation {
	/// What [`QueryClient::mutation_infos`] reports for this mutation.
	#[must_use]
	pub fn info(&self) -> MutationInfo {
		self.mutation.info()
	}

	/// The Rust type of the mutation's variables, as
	/// [`std::any::type_name`] writes it.
	#[must_use]
	pub fn variables_type(&self) -> &'static str {
		self.mutation.types().0
	}

	/// The Rust type of the mutation's data, as [`std::any::type_name`]
	/// writes it.
	#[must_use]
	pub fn data_type(&self) -> &'static str {
		self.mutation.types().1
	}
}

impl fmt::Debug for InspectedMutation {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		f.debug_struct("InspectedMutation")
			.field("info", &self.info())
			.field("variables_type", &self.variables_type())
			.field("data_type", &self.data_type())
			.finish()
	}
}
