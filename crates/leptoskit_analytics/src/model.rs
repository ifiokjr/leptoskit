use serde::Deserialize;
use serde::Serialize;

/// Fixed latency histogram boundaries in milliseconds. The last bucket is
/// unbounded. A percentile is a bucket upper bound, not an exact quantile.
pub const LATENCY_BOUNDS: [u64; 11] = [5, 10, 25, 50, 100, 250, 500, 1000, 2500, 5000, 10000];

/// Server-owned identity, shared by every observation in this process.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Resource {
	pub service: String,
	pub environment: String,
	pub release: String,
	pub instance: String,
}

/// Memory and time bounds. A restart clears this deliberately ephemeral tier.
#[derive(Clone, Debug)]
pub struct Limits {
	pub window_seconds: u64,
	pub retained_windows: usize,
	pub max_series: usize,
	pub max_recent: usize,
}

impl Default for Limits {
	fn default() -> Self {
		Self {
			window_seconds: 60,
			retained_windows: 15,
			max_series: 128,
			max_recent: 64,
		}
	}
}

impl Limits {
	pub(crate) fn valid(&self) -> bool {
		(1..=3600).contains(&self.window_seconds)
			&& (1..=60).contains(&self.retained_windows)
			&& (1..=512).contains(&self.max_series)
			&& self.max_recent <= 256
	}
}

/// Collection failed before any observation was retained.
#[derive(Debug, thiserror::Error)]
pub enum AnalyticsError {
	#[error("analytics identity must contain 1..=128 printable ASCII characters per field")]
	Identity,
	#[error("analytics limits exceed the supported memory/time bounds")]
	Limits,
}

/// Data provenance. Browser/native reports never count as backend evidence.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
	Backend,
	Browser,
	Native,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Signal {
	Http,
	Database,
	Rpc,
	Job,
	Live,
	PageView,
	Navigation,
	BrowserError,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
	Ok,
	ClientError,
	ServerError,
	Cancelled,
}

/// Low-cardinality identity. `operation` is an application-owned name or
/// route template; never a raw URL, SQL statement, exception, or user ID.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct SeriesKey {
	pub source: Source,
	pub signal: Signal,
	pub operation: String,
	pub outcome: Outcome,
}

/// One fixed time window. `latency` contains non-cumulative bucket counts.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Window {
	pub start_unix_seconds: u64,
	pub count: u64,
	pub duration_sum_ms: f64,
	pub duration_max_ms: f64,
	pub latency: [u64; 12],
}

impl Window {
	/// Upper bound of p95; `None` for no observations or the unbounded bucket.
	#[must_use]
	pub fn p95_upper_ms(&self) -> Option<u64> {
		if self.count == 0 {
			return None;
		}
		let rank = self.count - self.count / 20;
		let mut total = 0;
		for (i, count) in self.latency.iter().enumerate() {
			total += count;
			if total >= rank {
				return LATENCY_BOUNDS.get(i).copied();
			}
		}
		None
	}
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Series {
	pub key: SeriesKey,
	pub windows: Vec<Window>,
}

/// Correlation only. No headers, bodies, URLs, stack messages or identities.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Correlation {
	pub at_unix_seconds: u64,
	pub key: SeriesKey,
	pub duration_ms: f64,
	pub context: crate::TraceContext,
}

/// A versioned, single-process snapshot. Missing host/DB capacity measurements
/// are explicit: HTTP latency by itself cannot justify buying infrastructure.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Snapshot {
	pub format_version: u32,
	pub resource: Resource,
	pub started_unix_seconds: u64,
	pub observed_unix_seconds: u64,
	pub window_seconds: u64,
	pub retention_seconds: u64,
	pub series: Vec<Series>,
	pub recent: Vec<Correlation>,
	pub dropped_series_observations: u64,
	pub evicted_series: u64,
	pub invalid_observations: u64,
	pub browser_batches_rejected: u64,
	pub limitations: Vec<String>,
	#[serde(default)]
	pub host: Option<HostSample>,
	#[serde(default)]
	pub export: Option<ExportHealth>,
}

/// Platform measurements with an explicit scope. Linux cgroup values describe
/// the containing cgroup; they are not assumed to be whole-VM measurements.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct HostSample {
	pub at_unix_seconds: u64,
	pub process_resident_bytes: Option<u64>,
	pub cgroup_memory_bytes: Option<u64>,
	pub cgroup_memory_limit_bytes: Option<u64>,
	pub cgroup_cpu_seconds: Option<f64>,
	pub cgroup_cpu_limit_cores: Option<f64>,
	pub filesystem_available_bytes: Option<u64>,
	pub filesystem_capacity_bytes: Option<u64>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ExportHealth {
	pub successful_batches: u64,
	pub failed_batches: u64,
	pub last_success_unix_seconds: Option<u64>,
}

/// An advisory finding with concrete counts, never an executable scale command.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Finding {
	pub code: String,
	pub operation: String,
	pub samples: u64,
	pub failures: u64,
	pub recommendation: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Evidence {
	pub snapshot: Snapshot,
	pub findings: Vec<Finding>,
	pub infrastructure_changes_authorized: bool,
}
