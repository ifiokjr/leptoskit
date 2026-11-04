use std::collections::BTreeMap;
use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::Mutex;

use crate::*;

#[derive(Default)]
struct State {
	series: BTreeMap<SeriesKey, VecDeque<Window>>,
	recent: VecDeque<Correlation>,
	latest: u64,
	dropped: u64,
	evicted: u64,
	invalid: u64,
	browser_rejected: u64,
}

struct Inner {
	resource: Resource,
	limits: Limits,
	clock: fn() -> u64,
	started: u64,
	state: Mutex<State>,
}

/// Cloneable process-local collector. Recording never performs network or
/// disk I/O. Caller-owned operation names and explicit provenance avoid
/// accidentally collecting private request data.
#[derive(Clone)]
pub struct Analytics(Arc<Inner>);

impl Analytics {
	/// `clock` returns Unix seconds; injection also supports native/Wasm hosts
	/// and deterministic retention tests. Backward clock steps are clamped.
	///
	/// # Errors
	/// Rejects invalid identities or limits above the supported bounds.
	pub fn new(
		resource: Resource,
		limits: Limits,
		clock: fn() -> u64,
	) -> Result<Self, AnalyticsError> {
		if [
			&resource.service,
			&resource.environment,
			&resource.release,
			&resource.instance,
		]
		.into_iter()
		.any(|s| s.is_empty() || s.len() > 128 || !s.bytes().all(|b| (32..127).contains(&b)))
		{
			return Err(AnalyticsError::Identity);
		}
		if !limits.valid() {
			return Err(AnalyticsError::Limits);
		}
		Ok(Self(Arc::new(Inner {
			resource,
			limits,
			clock,
			started: clock(),
			state: Mutex::default(),
		})))
	}

	/// Records a completed operation. Returns false if invalid or over the
	/// series budget. Never pass a raw URL, SQL, or user-supplied operation
	/// name. Browser reports have a separate allocation ceiling and can be
	/// evicted to admit backend observations.
	pub fn record(&self, key: SeriesKey, duration_ms: f64, context: Option<TraceContext>) -> bool {
		let mut state = self
			.0
			.state
			.lock()
			.unwrap_or_else(std::sync::PoisonError::into_inner);
		let now = (self.0.clock)().max(state.latest);
		state.latest = now;
		let start = now / self.0.limits.window_seconds * self.0.limits.window_seconds;
		let cutoff = self.cutoff(start);
		prune(&mut state, cutoff);
		if !valid_operation(&key.operation)
			|| !duration_ms.is_finite()
			|| !(0.0..=86_400_000.0).contains(&duration_ms)
		{
			state.invalid = state.invalid.saturating_add(1);
			return false;
		}
		if !state.series.contains_key(&key) {
			let other_count = state
				.series
				.keys()
				.filter(|k| k.source != Source::Backend)
				.count();
			let full = state.series.len() >= self.0.limits.max_series;
			if key.source != Source::Backend
				&& (full || other_count >= self.0.limits.max_series / 2)
			{
				state.dropped = state.dropped.saturating_add(1);
				return false;
			}
			if full {
				if let Some(victim) = state
					.series
					.keys()
					.find(|k| k.source != Source::Backend)
					.cloned()
				{
					state.series.remove(&victim);
					state.evicted = state.evicted.saturating_add(1);
				} else {
					state.dropped = state.dropped.saturating_add(1);
					return false;
				}
			}
		}
		let windows = state.series.entry(key.clone()).or_default();
		if windows.back().is_none_or(|w| w.start_unix_seconds != start) {
			windows.push_back(Window {
				start_unix_seconds: start,
				..Window::default()
			});
		}
		if let Some(window) = windows.back_mut() {
			window.count = window.count.saturating_add(1);
			window.duration_sum_ms += duration_ms;
			window.duration_max_ms = window.duration_max_ms.max(duration_ms);
			let index = LATENCY_BOUNDS
				.iter()
				.position(|b| duration_ms <= *b as f64)
				.unwrap_or(11);
			window.latency[index] = window.latency[index].saturating_add(1);
		}
		if let Some(context) = context
			&& self.0.limits.max_recent > 0
		{
			if state.recent.len() == self.0.limits.max_recent {
				state.recent.pop_front();
			}
			state.recent.push_back(Correlation {
				at_unix_seconds: now,
				key,
				duration_ms,
				context,
			});
		}
		true
	}

	/// Records overload at the public ingestion boundary without retaining its
	/// payload.
	pub fn reject_browser_batch(&self) {
		let mut state = self
			.0
			.state
			.lock()
			.unwrap_or_else(std::sync::PoisonError::into_inner);
		state.browser_rejected = state.browser_rejected.saturating_add(1);
	}

	#[must_use]
	pub fn snapshot(&self) -> Snapshot {
		let mut state = self
			.0
			.state
			.lock()
			.unwrap_or_else(std::sync::PoisonError::into_inner);
		let now = (self.0.clock)().max(state.latest);
		state.latest = now;
		let start = now / self.0.limits.window_seconds * self.0.limits.window_seconds;
		prune(&mut state, self.cutoff(start));
		Snapshot {
			host: None,
			export: None,
			format_version: 1,
			resource: self.0.resource.clone(),
			started_unix_seconds: self.0.started,
			observed_unix_seconds: now,
			window_seconds: self.0.limits.window_seconds,
			retention_seconds: self.0.limits.window_seconds * self.0.limits.retained_windows as u64,
			series: state
				.series
				.iter()
				.map(|(key, windows)| {
					Series {
						key: key.clone(),
						windows: windows.iter().cloned().collect(),
					}
				})
				.collect(),
			recent: state.recent.iter().cloned().collect(),
			dropped_series_observations: state.dropped,
			evicted_series: state.evicted,
			invalid_observations: state.invalid,
			browser_batches_rejected: state.browser_rejected,
			limitations: [
				"Single process; data is lost on restart and is not merged across replicas.",
				"No CPU, memory, disk, database-pool or relay-capacity measurement is present.",
				"HTTP duration ends at response headers, excluding streamed bodies and WebSocket \
				 lifetimes.",
				"Browser/native observations are untrusted and may be blocked, sampled, or forged.",
				"Resource release identifies this server; client bundle/code-push release \
				 identity is not collected.",
				"Recent contexts are a bounded correlation buffer, not complete distributed \
				 traces.",
				"Current and first windows can be partial; do not interpret their counts as \
				 full-window rates.",
			]
			.into_iter()
			.map(str::to_owned)
			.collect(),
		}
	}

	/// Finds backend errors in completed windows with >=100 requests.
	/// The result suggests investigation; it never grants infrastructure
	/// writes.
	#[must_use]
	pub fn evidence(&self) -> Evidence {
		let snapshot = self.snapshot();
		let start =
			snapshot.observed_unix_seconds / snapshot.window_seconds * snapshot.window_seconds;
		let mut routes: BTreeMap<&str, (u64, u64)> = BTreeMap::new();
		for series in &snapshot.series {
			if series.key.source != Source::Backend || series.key.signal != Signal::Http {
				continue;
			}
			let count = series
				.windows
				.iter()
				.filter(|w| w.start_unix_seconds < start)
				.fold(0_u64, |n, w| n.saturating_add(w.count));
			let (samples, failures) = routes.entry(&series.key.operation).or_default();
			*samples = samples.saturating_add(count);
			if series.key.outcome == Outcome::ServerError {
				*failures = failures.saturating_add(count);
			}
		}
		let findings = routes
			.into_iter()
			.filter(|(_, (samples, failures))| {
				*samples >= 100 && *failures >= samples.div_ceil(100)
			})
			.map(|(route, (samples, failures))| {
				Finding {
					code: "backend_error_rate".into(),
					operation: route.into(),
					samples,
					failures,
					recommendation: "Investigate the failing release and dependencies. Collect \
					                 host and database saturation evidence before proposing \
					                 capacity changes."
						.into(),
				}
			})
			.collect();
		Evidence {
			snapshot,
			findings,
			infrastructure_changes_authorized: false,
		}
	}

	fn cutoff(&self, start: u64) -> u64 {
		start.saturating_sub(
			(self.0.limits.retained_windows as u64 - 1) * self.0.limits.window_seconds,
		)
	}
}

fn prune(state: &mut State, cutoff: u64) {
	state.series.retain(|_, windows| {
		while windows
			.front()
			.is_some_and(|w| w.start_unix_seconds < cutoff)
		{
			windows.pop_front();
		}
		!windows.is_empty()
	});
	while state
		.recent
		.front()
		.is_some_and(|r| r.at_unix_seconds < cutoff)
	{
		state.recent.pop_front();
	}
}

pub(crate) fn valid_operation(value: &str) -> bool {
	!value.is_empty()
		&& value.len() <= 128
		&& value
			.bytes()
			.all(|b| b.is_ascii_alphanumeric() || b"/:-_.*{}".contains(&b))
}
