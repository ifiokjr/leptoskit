//! Bounded snapshot export to the `LeptosKit` service, or OTLP/HTTP JSON
//! metrics. There is one in-flight batch and no unbounded retry queue. The next
//! snapshot covers retained windows after an outage; older data can be lost.
use std::sync::Arc;
use std::sync::Mutex;

use serde_json::json;

use crate::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Protocol {
	Leptoskit,
	OtlpMetrics,
}

/// Endpoint and server-only bearer credential. Debug is intentionally absent.
pub struct Config {
	pub endpoint: String,
	pub key: Option<String>,
	pub protocol: Protocol,
}

pub(crate) struct Driver {
	task: tokio::task::JoinHandle<()>,
	pub health: Arc<Mutex<ExportHealth>>,
}
impl Drop for Driver {
	fn drop(&mut self) {
		self.task.abort();
	}
}

pub(crate) fn start(analytics: Analytics, config: Config) -> Result<Arc<Driver>, &'static str> {
	let url = url::Url::parse(&config.endpoint).map_err(|_| "invalid export endpoint")?;
	if !matches!(url.scheme(), "https" | "http")
		|| !url.username().is_empty()
		|| url.password().is_some()
		|| url.fragment().is_some()
		|| url.query().is_some()
	{
		return Err("invalid export endpoint");
	}
	if config.key.as_ref().is_some_and(|key| {
		key.is_empty() || key.len() > 256 || !key.bytes().all(|b| (33..127).contains(&b))
	}) || config.protocol == Protocol::Leptoskit
		&& config.key.as_ref().is_none_or(|key| key.len() < 32)
	{
		return Err("invalid export credential");
	}
	let client = reqwest::Client::builder()
		.redirect(reqwest::redirect::Policy::none())
		.timeout(std::time::Duration::from_secs(10))
		.build()
		.map_err(|_| "could not initialize analytics exporter")?;
	let health = Arc::new(Mutex::new(ExportHealth::default()));
	let shared = health.clone();
	let runtime = tokio::runtime::Handle::try_current()
		.map_err(|_| "analytics export needs a Tokio runtime")?;
	let task = runtime.spawn(async move {
		let mut interval = tokio::time::interval(std::time::Duration::from_secs(30));
		interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
		let mut sent_through = 0;
		loop {
			interval.tick().await;
			let mut snapshot = analytics.snapshot();
			snapshot.host = Some(host::sample());
			snapshot.export = shared.lock().ok().map(|health| health.clone());
			let body = match config.protocol {
				Protocol::Leptoskit => serde_json::to_value(&snapshot).ok(),
				Protocol::OtlpMetrics => Some(otlp_metrics(&snapshot, sent_through)),
			};
			let Some(body) = body else {
				continue;
			};
			let mut request = client.post(&config.endpoint).json(&body);
			if let Some(key) = &config.key {
				request = request.bearer_auth(key);
			}
			let succeeded = match request.send().await {
				Ok(response) => accepted(response, config.protocol).await,
				Err(_) => false,
			};
			if succeeded {
				sent_through = snapshot.observed_unix_seconds / snapshot.window_seconds
					* snapshot.window_seconds;
			}
			if let Ok(mut health) = shared.lock() {
				if succeeded {
					health.successful_batches = health.successful_batches.saturating_add(1);
					health.last_success_unix_seconds = Some(snapshot.observed_unix_seconds);
				} else {
					health.failed_batches = health.failed_batches.saturating_add(1);
				}
			}
		}
	});
	Ok(Arc::new(Driver { task, health }))
}

async fn accepted(mut response: reqwest::Response, protocol: Protocol) -> bool {
	if !response.status().is_success() {
		return false;
	}
	if protocol == Protocol::Leptoskit {
		return response.status() == reqwest::StatusCode::NO_CONTENT;
	}
	let mut bytes = Vec::new();
	loop {
		match response.chunk().await {
			Ok(Some(chunk)) => {
				if bytes.len() + chunk.len() > 65536 {
					return false;
				}
				bytes.extend_from_slice(&chunk);
			}
			Ok(None) => break,
			Err(_) => return false,
		}
	}
	if bytes.is_empty() {
		return true;
	}
	let Ok(reply) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
		return false;
	};
	let rejected = &reply["partialSuccess"]["rejectedDataPoints"];
	rejected
		.as_u64()
		.or_else(|| rejected.as_str().and_then(|n| n.parse().ok()))
		.unwrap_or_default()
		== 0
}

/// OTLP delta histograms for completed windows. Timestamps and uint64 values
/// follow OTLP JSON encoding. A retry can repeat a delta after an ambiguous
/// transport failure; use a backend that deduplicates time-series timestamps.
#[must_use]
pub fn otlp_metrics(snapshot: &Snapshot, sent_through: u64) -> serde_json::Value {
	let completed =
		snapshot.observed_unix_seconds / snapshot.window_seconds * snapshot.window_seconds;
	let mut points = Vec::new();
	for series in &snapshot.series {
		for window in &series.windows {
			if window.start_unix_seconds < sent_through || window.start_unix_seconds >= completed {
				continue;
			}
			let attributes = [
				(
					"leptoskit.source",
					serde_json::to_value(series.key.source).unwrap_or_default(),
				),
				(
					"leptoskit.signal",
					serde_json::to_value(series.key.signal).unwrap_or_default(),
				),
				("leptoskit.operation", json!(series.key.operation)),
				(
					"leptoskit.outcome",
					serde_json::to_value(series.key.outcome).unwrap_or_default(),
				),
			]
			.into_iter()
			.map(
				|(key, value)| json!({"key":key,"value":{"stringValue":value.as_str().unwrap_or_default()}}),
			)
			.collect::<Vec<_>>();
			points.push(json!({"attributes":attributes,"startTimeUnixNano":(window.start_unix_seconds*1_000_000_000).to_string(),"timeUnixNano":((window.start_unix_seconds+snapshot.window_seconds)*1_000_000_000).to_string(),"count":window.count.to_string(),"sum":window.duration_sum_ms,"max":window.duration_max_ms,"bucketCounts":window.latency.iter().map(u64::to_string).collect::<Vec<_>>(),"explicitBounds":LATENCY_BOUNDS}));
		}
	}
	let resource = &snapshot.resource;
	let attributes = [
		("service.name", &resource.service),
		("service.version", &resource.release),
		("service.instance.id", &resource.instance),
		("deployment.environment.name", &resource.environment),
	]
	.into_iter()
	.map(|(key, value)| json!({"key":key,"value":{"stringValue":value}}))
	.collect::<Vec<_>>();
	json!({"resourceMetrics":[{"resource":{"attributes":attributes},"scopeMetrics":[{"scope":{"name":"leptoskit_analytics","version":env!("CARGO_PKG_VERSION")},"metrics":[{"name":"leptoskit.operation.duration","unit":"ms","histogram":{"aggregationTemporality":1,"dataPoints":points}}]}]}]})
}
