//! The queries and mutations panels: every query and mutation of a tracked
//! `leptoskit_query` cache, with refetch, invalidate, and edit.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use leptoskit_devtools_protocol::Command;
use leptoskit_devtools_protocol::MAX_VALUE_BYTES;
use leptoskit_devtools_protocol::Value;
use leptoskit_query::FetchStatus;
use leptoskit_query::MutationInfo;
use leptoskit_query::MutationStatus;
use leptoskit_query::QueryClient;
use leptoskit_query::QueryInfo;
use leptoskit_query::QueryMeta;
use leptoskit_query::QueryStatus;
use leptoskit_query::ScopeId;
use leptoskit_query::devtools::InspectedMutation;
use leptoskit_query::devtools::InspectedQuery;
use reactive_graph::effect::ImmediateEffect;
use reactive_graph::owner::on_cleanup;
use reactive_graph::traits::Track;
use serde_json::json;

use crate::registry;
use crate::registry::Report;
use crate::registry::Source;
use crate::value::short_type_name;

/// Why a query without a codec shows no data.
const NO_CODEC: &str = "This query's scope has no codec, so its data can't be shown or edited \
                        here. Give the scope `.with_ssr()` or `.with_codec(..)` to see it.";

/// Report every query and mutation of `client` to the `DevTools`, and let
/// them refetch, invalidate, and edit its queries.
///
/// Call it once with the client the app provides. The queries panel shows
/// each query by its scope and key, with its status, its fetch status,
/// whether it is stale, how many views observe it, when its data and its
/// last error arrived, and the data itself. The mutations panel shows each
/// mutation's status. Refetch and invalidate work on every query. Data is
/// shown and edited through the scope's codec, the one
/// [`QueryScope::with_ssr`](leptoskit_query::QueryScope::with_ssr) adds,
/// so a query whose scope has none is read-only, and its value says so.
///
/// ```ignore
/// let client = QueryClient::new();
/// client.provide();
/// #[cfg(feature = "devtools")]
/// leptoskit::devtools::track_queries(&client);
/// ```
///
/// Outside a `leptoskit dev` build this does nothing.
pub fn track_queries(client: &QueryClient) {
	if !registry::active() {
		return;
	}

	let id = registry::register(
		None,
		Rc::new(QuerySource {
			client: client.clone(),
			cache: RefCell::default(),
		}),
	);
	// Cache events say a query or mutation changed. A fetch starting or
	// stopping raises none, but moves the fetch counters.
	let queries = client.subscribe(move |_| registry::mark_dirty(id));
	let mutations = client.subscribe_mutations(move |_| registry::mark_dirty(id));
	let fetching = client.fetching_count_signal();
	let mutating = client.mutating_count_signal();
	let effect = ImmediateEffect::new_isomorphic(move || {
		fetching.track();
		mutating.track();
		registry::mark_dirty(id);
	});
	registry::keep(id, (queries, mutations, effect));
	on_cleanup(move || registry::unregister(id));
}

/// A query's identity in its cache.
type QueryId = (ScopeId, u64, String);

/// Everything a query's value is built from, except its data, which
/// changes only together with `meta`.
#[derive(Clone, Copy, PartialEq)]
struct Stamp {
	status: QueryStatus,
	fetch_status: FetchStatus,
	observers: usize,
	is_stale: bool,
	meta: QueryMeta,
}

impl Stamp {
	fn of(info: &QueryInfo) -> Self {
		Self {
			status: info.status,
			fetch_status: info.fetch_status,
			observers: info.observers,
			is_stale: info.is_stale,
			meta: info.meta,
		}
	}
}

struct Cached {
	stamp: Stamp,
	value: Value,
}

struct QuerySource {
	client: QueryClient,
	/// Each query's last value, so its data is encoded again only when it
	/// changed.
	cache: RefCell<HashMap<QueryId, Cached>>,
}

impl Source for QuerySource {
	fn collect(&self, report: &mut Report) {
		let mut cache = self.cache.borrow_mut();
		let mut live = HashMap::with_capacity(cache.len());

		for (key, query, info) in labelled(&self.client) {
			let stamp = Stamp::of(&info);
			let id: QueryId = (info.scope, info.key_hash, info.key.clone());
			let value = match cache.remove(&id) {
				Some(cached) if cached.stamp == stamp => cached.value,
				_ => query_value(&query, &info),
			};
			report.query(key, value.clone());
			live.insert(id, Cached { stamp, value });
		}

		// Queries that left the cache leave this one too.
		*cache = live;

		for mutation in self.client.inspect_mutations() {
			let info = mutation.info();
			report.mutation(
				format!("{} #{}", scope_label(&info.scope_name), info.id),
				mutation_value(&mutation, &info),
			);
		}
	}

	fn apply(&self, command: &Command) -> Option<Result<(), String>> {
		let find = |key: &str| {
			labelled(&self.client)
				.into_iter()
				.find(|(label, ..)| label == key)
				.map(|(_, query, _)| query)
		};

		match command {
			Command::InvalidateQuery { key } => {
				find(key)?.invalidate();
				Some(Ok(()))
			}
			Command::RefetchQuery { key } => {
				find(key)?.refetch();
				Some(Ok(()))
			}
			Command::SetQueryData { key, value } => Some(set_data(&find(key)?, key, value)),
			_ => None,
		}
	}
}

/// Every query of `client` with the key the `DevTools` know it by: its
/// scope and its key, numbered in a stable order when two print the same.
fn labelled(client: &QueryClient) -> Vec<(String, InspectedQuery, QueryInfo)> {
	let mut queries: Vec<(String, InspectedQuery, QueryInfo)> = client
		.inspect_queries()
		.into_iter()
		.map(|query| {
			let info = query.info();
			(label(&info), query, info)
		})
		.collect();
	queries.sort_by(|(left, _, left_info), (right, _, right_info)| {
		left.cmp(right)
			.then(left_info.scope.cmp(&right_info.scope))
			.then(left_info.key_hash.cmp(&right_info.key_hash))
	});

	let mut seen: HashMap<String, usize> = HashMap::new();
	for (label, ..) in &mut queries {
		let count = seen.entry(label.clone()).or_insert(0);
		*count += 1;
		if *count > 1 {
			*label = format!("{label} #{count}");
		}
	}

	queries
}

/// A query's key in the `DevTools`: its scope, then its key as `Debug`
/// prints it, unless the key is `()`.
fn label(info: &QueryInfo) -> String {
	let scope = scope_label(&info.scope_name);
	if info.key == "()" {
		scope.to_owned()
	} else {
		format!("{scope} {}", info.key)
	}
}

/// A scope's name without the closure an unnamed scope is named after:
/// `app::queries::clips::{{closure}}` reads `app::queries::clips`.
fn scope_label(name: &str) -> &str {
	let mut name = name;
	while let Some(outer) = name.strip_suffix("::{{closure}}") {
		name = outer;
	}
	name
}

fn query_value(query: &InspectedQuery, info: &QueryInfo) -> Value {
	let mut json = json!({
		"scope": scope_label(&info.scope_name),
		"key": info.key,
		"status": query_status(info.status),
		"fetch_status": fetch_status(info.fetch_status),
		"stale": info.is_stale,
		"invalidated": info.meta.is_invalidated,
		"observers": info.observers,
		"data_updated_at": info.meta.data_updated_at,
		"error_updated_at": info.meta.error_updated_at,
		"failure_count": info.meta.failure_count,
		"data": null,
	});
	let mut truncated = false;

	match query.encode() {
		Some(Ok(data)) => {
			let bytes = data.to_string().len();
			if bytes > MAX_VALUE_BYTES {
				// Keep the status readable and leave only the data out.
				json["data"] = json!({ "truncated": true, "bytes": bytes });
				truncated = true;
			} else {
				json["data"] = data;
			}
		}
		Some(Err(error)) => {
			json["read_only"] = format!("Its codec couldn't encode the data: {error}").into();
		}
		None if !query.has_codec() => json["read_only"] = NO_CODEC.into(),
		None => {}
	}

	let mut value = Value::from_json(
		format!("QueryState<{}>", short_type_name(query.data_type())),
		json,
	);
	value.truncated |= truncated;
	value
}

fn mutation_value(mutation: &InspectedMutation, info: &MutationInfo) -> Value {
	Value::from_json(
		format!(
			"MutationState<{}, {}>",
			short_type_name(mutation.variables_type()),
			short_type_name(mutation.data_type())
		),
		json!({
			"scope": scope_label(&info.scope_name),
			"status": mutation_status(info.status),
			"paused": info.is_paused,
			"submitted_at": info.submitted_at,
			"failure_count": info.failure_count,
		}),
	)
}

/// Replaces a query's data with the `data` of the value the `DevTools`
/// edited: they edit the whole value the agent sent.
fn set_data(query: &InspectedQuery, key: &str, value: &Value) -> Result<(), String> {
	if value.truncated {
		return Err(format!(
			"the new data for {key:?} arrived truncated, so it wasn't set"
		));
	}
	let data = value
		.json
		.get("data")
		.cloned()
		.ok_or_else(|| format!("the new value for {key:?} has no `data` to set"))?;
	query
		.set_json(data)
		.map_err(|error| format!("{key:?}: {error}"))
}

const fn query_status(status: QueryStatus) -> &'static str {
	match status {
		QueryStatus::Pending => "pending",
		QueryStatus::Error => "error",
		QueryStatus::Success => "success",
	}
}

const fn fetch_status(status: FetchStatus) -> &'static str {
	match status {
		FetchStatus::Idle => "idle",
		FetchStatus::Fetching => "fetching",
		FetchStatus::Paused => "paused",
	}
}

const fn mutation_status(status: MutationStatus) -> &'static str {
	match status {
		MutationStatus::Idle => "idle",
		MutationStatus::Pending => "pending",
		MutationStatus::Error => "error",
		MutationStatus::Success => "success",
	}
}
