//! Product analytics as a leptoskit service: its own process, its own
//! database, and operations the app calls and watches as if they were its
//! own.
//!
//! The app depends on this library for the operations' types. Each one is
//! declared with `service = "analytics"`, so wherever it's called from (the
//! browser, a native app, or the app's server while it renders a page), the
//! call goes to this service. Only the service binary, built with this
//! crate's `service` feature, compiles the bodies below and serves them.
//!
//! Who may call what:
//!
//! - [`capture`] is public. Browsers send events anonymously, so it checks
//!   sizes and nothing else.
//! - [`summary`] is public too, because it's an aggregate: counts per event and
//!   per day, with no visitor's identity in it. Screens watch it live.
//! - [`recent`] returns individual events, visitor ids included, so only the
//!   app may call it: the app's server calls it with
//!   `rpc::client::call_as_app`, which presents the service's token and so
//!   identifies it as the caller `"app"`. An ordinary `call`, and every query
//!   hook, goes anonymously, so a page never renders it.
//!
//! After changing the table, regenerate the migrations: see `readme.md`.

use std::collections::BTreeMap;

use leptoskit_db::table;
use leptoskit_rpc as rpc;
use schemars::JsonSchema;
use serde::Deserialize;
use serde::Serialize;

// Mutation receipts, which make a retried `capture` record one event, live in
// the `rpc` pack, in the service's own database.
#[cfg(feature = "service")]
leptoskit_db::packs!(leptoskit_rpc);

/// The service's name: the `[services.analytics]` table in the app's
/// `leptoskit.toml` must use it, because every operation below is addressed
/// to it.
pub const SERVICE: &str = "analytics";

/// The longest event name.
pub const MAX_EVENT_LEN: usize = 64;
/// The longest visitor id.
pub const MAX_DISTINCT_ID_LEN: usize = 128;
/// The most properties one event carries.
pub const MAX_PROPERTIES: usize = 16;
/// The longest property name.
pub const MAX_PROPERTY_KEY_LEN: usize = 40;
/// The longest property value.
pub const MAX_PROPERTY_VALUE_LEN: usize = 200;
/// The most days [`summary`] covers.
pub const MAX_DAYS: u32 = 90;
/// The most events [`summary`] reads. Past it the counts are a lower bound,
/// and [`EventSummary::truncated`] says so. A larger deployment would keep
/// per-day rollups instead.
pub const MAX_SCANNED: u64 = 50_000;

/// One thing that happened, as a browser or the app reported it.
#[table(name = "events", live)]
pub struct Event {
	pub id: leptoskit_db::Id<Event>,
	/// What happened, such as `signup` or `clicked_buy`.
	#[db(min_len = 1, max_len = 64)]
	pub name: String,
	/// Who it happened to: an id the browser keeps, not an account.
	#[db(min_len = 1, max_len = 128)]
	pub distinct_id: String,
	/// The event's properties, as a JSON object of strings.
	#[db(max_len = 8192)]
	pub properties: String,
	#[db(created_at)]
	pub created_at: leptoskit_db::Timestamp,
}

leptoskit_db::export_schema!();

/// Every migration, embedded for the service to apply when it starts.
#[cfg(feature = "service")]
pub static MIGRATIONS: &[leptoskit_db::Migration] = leptoskit_db::migrations!("migrations");

/// The same migrations written for Postgres.
#[cfg(feature = "postgres")]
pub static POSTGRES_MIGRATIONS: &[leptoskit_db::Migration] =
	leptoskit_db::migrations!("migrations-postgres");

/// How often each event happened in the period.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct EventCount {
	pub name: String,
	pub count: u64,
}

/// One day's events and distinct visitors.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DayCount {
	/// The day, as `YYYY-MM-DD` in UTC.
	pub day: String,
	pub events: u64,
	pub visitors: u64,
}

/// What happened over the last few days.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct EventSummary {
	/// How many days it covers, today included.
	pub days: u32,
	/// Every event in the period.
	pub total: u64,
	/// Distinct visitors in the period.
	pub visitors: u64,
	/// Each event, most frequent first.
	pub events: Vec<EventCount>,
	/// Each day with events, oldest first.
	pub by_day: Vec<DayCount>,
	/// Whether the period held more than [`MAX_SCANNED`] events.
	pub truncated: bool,
}

/// One event, for the app's own screens.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RecentEvent {
	pub name: String,
	pub distinct_id: String,
	pub properties: BTreeMap<String, String>,
	/// When it was recorded, in RFC 3339.
	pub at: String,
}

/// Checks an event before it's sent or stored, so a screen can reject bad
/// input without a round trip and the service rejects it all the same.
///
/// # Errors
///
/// A message naming what's wrong.
pub fn check_event(
	event: &str,
	distinct_id: &str,
	properties: &BTreeMap<String, String>,
) -> Result<(), String> {
	let event_ok = !event.is_empty()
		&& event.len() <= MAX_EVENT_LEN
		&& event
			.bytes()
			.all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.' | b':'));

	if !event_ok {
		return Err(format!(
			"an event's name is 1 to {MAX_EVENT_LEN} letters, digits, `_`, `-`, `.`, or `:`"
		));
	}

	if distinct_id.trim().is_empty() || distinct_id.len() > MAX_DISTINCT_ID_LEN {
		return Err(format!("a visitor id is 1 to {MAX_DISTINCT_ID_LEN} bytes"));
	}

	if properties.len() > MAX_PROPERTIES {
		return Err(format!("an event has at most {MAX_PROPERTIES} properties"));
	}

	for (key, value) in properties {
		if key.is_empty() || key.len() > MAX_PROPERTY_KEY_LEN {
			return Err(format!(
				"a property's name is 1 to {MAX_PROPERTY_KEY_LEN} bytes"
			));
		}

		if value.len() > MAX_PROPERTY_VALUE_LEN {
			return Err(format!(
				"the property `{key}` is longer than {MAX_PROPERTY_VALUE_LEN} bytes"
			));
		}
	}

	Ok(())
}

/// Records one event. Anyone may call it: a browser sends its visitor id.
#[rpc::mutation(service = "analytics")]
pub async fn capture(
	cx: &rpc::MutationCx,
	event: String,
	distinct_id: String,
	#[serde(default)] properties: BTreeMap<String, String>,
) -> rpc::Result<()> {
	check_event(&event, &distinct_id, &properties).map_err(rpc::Error::invalid)?;
	let properties = serde_json::to_string(&properties)
		.map_err(|error| rpc::Error::server(error.to_string()))?;
	NewEvent {
		name: event,
		distinct_id,
		properties,
	}
	.insert(cx.tx())
	.await?;
	Ok(())
}

/// Counts per event and per day over the last `days` days. It's an
/// aggregate with no visitor in it, so anyone may watch it, live.
#[rpc::query(service = "analytics")]
pub async fn summary(cx: &rpc::QueryCx, days: u32) -> rpc::Result<EventSummary> {
	let days = days.clamp(1, MAX_DAYS);
	let since = leptoskit_db::now() - chrono::TimeDelta::days(i64::from(days) - 1);
	let since = since
		.date_naive()
		.and_hms_opt(0, 0, 0)
		.map_or(since, |midnight| midnight.and_utc());
	let rows = Event::query()
		.filter(|event| event.created_at.gte(since))
		.order_by(|event| event.created_at.asc())
		.limit(MAX_SCANNED + 1)
		.all(cx.db())
		.await?;
	let truncated = rows.len() as u64 > MAX_SCANNED;

	let mut per_event: BTreeMap<String, u64> = BTreeMap::new();
	let mut per_day: BTreeMap<String, (u64, std::collections::BTreeSet<String>)> = BTreeMap::new();
	let mut visitors = std::collections::BTreeSet::new();

	for event in rows.iter().take(MAX_SCANNED as usize) {
		*per_event.entry(event.name.clone()).or_default() += 1;
		let day = per_day
			.entry(event.created_at.format("%Y-%m-%d").to_string())
			.or_default();
		day.0 += 1;
		day.1.insert(event.distinct_id.clone());
		visitors.insert(event.distinct_id.clone());
	}

	let mut events: Vec<EventCount> = per_event
		.into_iter()
		.map(|(name, count)| EventCount { name, count })
		.collect();
	events.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.name.cmp(&b.name)));

	Ok(EventSummary {
		days,
		total: events.iter().map(|event| event.count).sum(),
		visitors: visitors.len() as u64,
		events,
		by_day: per_day
			.into_iter()
			.map(|(day, (events, visitors))| {
				DayCount {
					day,
					events,
					visitors: visitors.len() as u64,
				}
			})
			.collect(),
		truncated,
	})
}

/// The latest events, newest first, visitor ids and all. Only the app may
/// call it.
#[rpc::query(service = "analytics")]
pub async fn recent(cx: &rpc::QueryCx, limit: u32) -> rpc::Result<Vec<RecentEvent>> {
	if cx.identity().caller.as_deref() != Some(rpc::server::APP_CALLER) {
		return Err(rpc::Error::forbidden(
			"only the app reads individual events",
		));
	}

	let rows = Event::query()
		.order_by(|event| event.created_at.desc())
		.limit(u64::from(limit.clamp(1, 100)))
		.all(cx.db())
		.await?;

	Ok(rows
		.into_iter()
		.map(|event| {
			RecentEvent {
				name: event.name,
				distinct_id: event.distinct_id,
				properties: serde_json::from_str(&event.properties).unwrap_or_default(),
				at: event.created_at.to_rfc3339(),
			}
		})
		.collect())
}

/// The service's routes, under `/_leptoskit/services/analytics`: the
/// operations and the live connection. Callers presenting
/// `LEPTOSKIT_SERVICE_TOKEN` are the app.
///
/// `origin` is the app's public origin, which the live connection checks
/// browsers against. Start the returned [`rpc::server::Live`] before
/// serving.
#[cfg(feature = "service")]
pub fn routes(
	db: leptoskit_db::Db,
	origin: Option<&str>,
) -> (leptoskit_axum::axum::Router, rpc::server::Live) {
	use rpc::server::Anonymous;
	use rpc::server::ServiceCallers;

	routes_with(db, origin, ServiceCallers::for_service(Anonymous))
}

/// [`routes`], identifying callers with `callers`.
#[cfg(feature = "service")]
pub fn routes_with(
	db: leptoskit_db::Db,
	origin: Option<&str>,
	callers: rpc::server::ServiceCallers<rpc::server::Anonymous>,
) -> (leptoskit_axum::axum::Router, rpc::server::Live) {
	let server = rpc::server::Server::new(db)
		.service(SERVICE)
		.identify(callers);
	let live = match origin {
		Some(origin) => server.live().origin(origin),
		None => server.live(),
	};
	let router = leptoskit_axum::axum::Router::new().nest(
		&rpc::service_prefix(SERVICE),
		server.router().merge(live.router()),
	);
	(router, live)
}
