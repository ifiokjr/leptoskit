//! The `ARENA` report line and the statistics behind it.

use serde_json::Map;
use serde_json::Value;
use serde_json::json;

use crate::services::Scenario;

/// The SDK release this app builds against, from this checkout.
pub const FRAMEWORK_VERSION: &str = "0.1.0";
/// The longest JSON text printed on one line; longer reports are split.
const PART_BYTES: usize = 800;
/// The most values one `samples` key may hold.
pub const MAX_SAMPLES: usize = 100;

/// One scenario's result.
#[derive(Debug)]
pub struct Report {
	scenario: Scenario,
	error: Option<String>,
	notes: Vec<String>,
	metrics: Map<String, Value>,
	samples: Map<String, Value>,
}

impl Report {
	pub fn new(scenario: Scenario) -> Self {
		Self {
			scenario,
			error: None,
			notes: Vec::new(),
			metrics: Map::new(),
			samples: Map::new(),
		}
	}

	/// Marks the run failed; a failed run keeps no numbers.
	pub fn fail(mut self, error: impl Into<String>) -> Self {
		self.error = Some(error.into());
		self
	}

	pub fn note(&mut self, note: impl Into<String>) {
		self.notes.push(note.into());
	}

	pub fn metric(&mut self, key: &str, value: f64) {
		self.metrics.insert(key.to_owned(), json!(round(value)));
	}

	/// Raw values behind a metric, at most [`MAX_SAMPLES`] of them.
	pub fn samples(&mut self, key: &str, values: &[f64]) {
		debug_assert!(values.len() <= MAX_SAMPLES, "{key} has too many samples");
		let values: Vec<f64> = values
			.iter()
			.take(MAX_SAMPLES)
			.copied()
			.map(round)
			.collect();
		self.samples.insert(key.to_owned(), json!(values));
	}

	/// The report as one `ARENA {json}` line, or as consecutive
	/// `ARENA-PART i/n <chunk>` lines when the JSON is longer than 800 bytes.
	pub fn lines(&self, platform: &str) -> String {
		let ok = self.error.is_none();
		let mut report = json!({
			"v": 1,
			"framework": "leptoskit",
			"frameworkVersion": FRAMEWORK_VERSION,
			"platform": platform,
			"scenario": self.scenario.id(),
			"lane": "idiomatic",
			"ok": ok,
			"metrics": if ok { Value::Object(self.metrics.clone()) } else { json!({}) },
		});
		let fields = report.as_object_mut().expect("the report is an object");

		if ok && !self.samples.is_empty() {
			fields.insert("samples".into(), Value::Object(self.samples.clone()));
		}

		if let Some(error) = &self.error {
			fields.insert("error".into(), json!(error));
		}

		if !self.notes.is_empty() {
			fields.insert("notes".into(), json!(self.notes));
		}

		split(&report.to_string())
	}
}

/// Splits JSON text into report lines on character boundaries.
fn split(json: &str) -> String {
	if json.len() <= PART_BYTES {
		return format!("ARENA {json}");
	}

	let mut chunks = Vec::new();
	let mut rest = json;

	while !rest.is_empty() {
		let mut end = rest.len().min(PART_BYTES);

		while !rest.is_char_boundary(end) {
			end -= 1;
		}

		let (chunk, tail) = rest.split_at(end);
		chunks.push(chunk);
		rest = tail;
	}

	let count = chunks.len();
	chunks
		.iter()
		.enumerate()
		.map(|(index, chunk)| format!("ARENA-PART {}/{count} {chunk}", index + 1))
		.collect::<Vec<_>>()
		.join("\n")
}

/// Three decimal places keep reports short without hiding microseconds.
fn round(value: f64) -> f64 {
	(value * 1000.0).round() / 1000.0
}

/// The nearest-rank percentile of `values`, which must not be empty.
pub fn percentile(values: &[f64], percent: f64) -> f64 {
	let mut sorted = values.to_vec();
	sorted.sort_by(f64::total_cmp);
	let rank = ((percent / 100.0) * sorted.len() as f64).ceil() as usize;
	sorted[rank.clamp(1, sorted.len()) - 1]
}

/// The median of `values`, which must not be empty.
pub fn median(values: &[f64]) -> f64 {
	percentile(values, 50.0)
}

/// The mean of each consecutive batch of `size` values.
pub fn batch_means(values: &[f64], size: usize) -> Vec<f64> {
	values
		.chunks(size)
		.map(|batch| batch.iter().sum::<f64>() / batch.len() as f64)
		.collect()
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn short_reports_stay_on_one_line() {
		assert_eq!(split("{}"), "ARENA {}");
	}

	#[test]
	fn long_reports_split_into_numbered_parts() {
		let json = format!("{{\"a\":\"{}\"}}", "x".repeat(1700));
		let lines = split(&json);
		let parts: Vec<&str> = lines.lines().collect();
		assert_eq!(parts.len(), 3);
		assert!(parts[0].starts_with("ARENA-PART 1/3 {"));
		let joined: String = parts
			.iter()
			.map(|line| line.splitn(3, ' ').nth(2).unwrap())
			.collect();
		assert_eq!(joined, json);
	}

	#[test]
	fn percentiles_use_nearest_rank() {
		let values: Vec<f64> = (1..=100).map(f64::from).collect();
		assert_eq!(median(&values), 50.0);
		assert_eq!(percentile(&values, 99.0), 99.0);
		assert_eq!(batch_means(&values, 50), vec![25.5, 75.5]);
	}
}
