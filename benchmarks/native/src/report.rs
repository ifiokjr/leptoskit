//! Timing and the JSON report, in the same shape as the browser harness's
//! runs so `benchmarks/scripts/compare.ts` reads host and device results
//! without knowing which layer produced them.

use std::collections::BTreeMap;
use std::time::Instant;

/// How many samples each scenario records, after how many unrecorded passes.
#[derive(Debug, Clone, Copy)]
pub struct Config {
	pub runs: usize,
	pub warmup: usize,
}

/// One scenario's samples; medians are computed when the report is written.
#[derive(Debug, Clone)]
pub struct Metric {
	pub unit: &'static str,
	pub class: &'static str,
	pub label: String,
	pub values: Vec<f64>,
}

/// The accumulating report.
#[derive(Debug, Default)]
pub struct Report {
	metrics: BTreeMap<String, Metric>,
	notes: Vec<String>,
}

impl Report {
	/// Records a duration scenario, in milliseconds.
	pub fn duration(&mut self, id: &str, label: &str, values: Vec<f64>) {
		self.metrics.insert(
			id.to_owned(),
			Metric {
				unit: "ms",
				class: "duration",
				label: label.to_owned(),
				values,
			},
		);
	}

	/// Records a throughput scenario derived from a duration: `count` events
	/// per sample, reported as events per second.
	pub fn throughput(&mut self, id: &str, label: &str, count: f64, durations_ms: &[f64]) {
		let values = durations_ms
			.iter()
			.map(|ms| if *ms > 0.0 { count / ms * 1000.0 } else { 0.0 })
			.collect();
		self.metrics.insert(
			id.to_owned(),
			Metric {
				unit: "per-sec",
				class: "throughput",
				label: label.to_owned(),
				values,
			},
		);
	}

	/// Records why a layer was skipped, so a missing section is explained.
	pub fn note(&mut self, note: impl Into<String>) {
		self.notes.push(note.into());
	}

	/// The JSON document, `benchmarks/scripts/common.ts`'s `BenchmarkRun`.
	pub fn to_json(&self, subject: &str, config: Config) -> serde_json::Value {
		let mut metrics = serde_json::Map::new();
		for (id, metric) in &self.metrics {
			if metric.values.is_empty() {
				continue;
			}
			let mut sorted = metric.values.clone();
			sorted.sort_by(f64::total_cmp);
			let middle = sorted.len() / 2;
			let median = if sorted.len() % 2 == 1 {
				sorted[middle]
			} else {
				f64::midpoint(sorted[middle - 1], sorted[middle])
			};
			metrics.insert(
				id.clone(),
				serde_json::json!({
					"unit": metric.unit,
					"class": metric.class,
					"label": metric.label,
					"values": metric.values,
					"median": median,
					"min": sorted[0],
					"max": sorted[sorted.len() - 1],
				}),
			);
		}
		serde_json::json!({
			"schema": 1,
			"subject": subject,
			"timestamp": timestamp(),
			"gitCommit": serde_json::Value::Null,
			"environment": {
				"browser": "n/a",
				"platform": format!("{} {}", std::env::consts::OS, std::env::consts::ARCH),
				"cpus": std::thread::available_parallelism().map(usize::from).ok(),
			},
			"runs": config.runs,
			"notes": self.notes,
			"metrics": metrics,
		})
	}
}

/// Runs `timed` against a fresh `setup()` state `warmup + runs` times and
/// returns the recorded wall-clock milliseconds. Setup and the state's drop
/// stay outside the timed region.
pub fn sample<S>(
	config: Config,
	mut setup: impl FnMut() -> S,
	mut timed: impl FnMut(&mut S),
) -> Vec<f64> {
	let mut samples = Vec::with_capacity(config.runs);
	for iteration in 0..(config.warmup + config.runs) {
		let mut state = setup();
		let start = Instant::now();
		timed(&mut state);
		let elapsed = start.elapsed().as_secs_f64() * 1000.0;
		drop(state);
		if iteration >= config.warmup {
			samples.push(elapsed);
		}
	}
	samples
}

/// An ISO-8601 timestamp without pulling in a date crate: seconds since the
/// epoch is enough for the report to be ordered.
fn timestamp() -> String {
	let seconds = std::time::SystemTime::now()
		.duration_since(std::time::UNIX_EPOCH)
		.map_or(0, |duration| duration.as_secs());
	// Civil-from-days (Howard Hinnant's algorithm), UTC.
	let days = i64::try_from(seconds / 86_400).unwrap_or(0);
	let remainder = seconds % 86_400;
	let z = days + 719_468;
	let era = z.div_euclid(146_097);
	let day_of_era = z.rem_euclid(146_097);
	let year_of_era =
		(day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
	let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
	let mp = (5 * day_of_year + 2) / 153;
	let day = day_of_year - (153 * mp + 2) / 5 + 1;
	let month = if mp < 10 { mp + 3 } else { mp - 9 };
	let year = year_of_era + era * 400 + i64::from(month <= 2);
	format!(
		"{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
		remainder / 3600,
		(remainder % 3600) / 60,
		remainder % 60
	)
}
