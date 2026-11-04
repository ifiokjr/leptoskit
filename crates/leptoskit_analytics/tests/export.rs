#![cfg(feature = "server")]
use leptoskit_analytics::*;

#[test]
fn otlp_histograms_export_completed_delta_windows_with_standard_json_encoding() {
	let analytics = Analytics::new(
		Resource {
			service: "notes".into(),
			environment: "test".into(),
			release: "v1".into(),
			instance: "one".into(),
		},
		Limits::default(),
		|| 120,
	)
	.unwrap();
	analytics.record(
		SeriesKey {
			source: Source::Backend,
			signal: Signal::Http,
			operation: "/notes/{id}".into(),
			outcome: Outcome::Ok,
		},
		50.0,
		None,
	);
	let mut snapshot = analytics.snapshot();
	let pending = export::otlp_metrics(&snapshot, 0);
	assert_eq!(
		pending["resourceMetrics"][0]["scopeMetrics"][0]["metrics"][0]["histogram"]["dataPoints"],
		serde_json::json!([])
	);
	snapshot.observed_unix_seconds = 180;
	let payload = export::otlp_metrics(&snapshot, 0);
	let histogram = &payload["resourceMetrics"][0]["scopeMetrics"][0]["metrics"][0]["histogram"];
	assert_eq!(histogram["aggregationTemporality"], 1);
	assert_eq!(histogram["dataPoints"][0]["count"], "1");
	assert_eq!(
		histogram["dataPoints"][0]["startTimeUnixNano"],
		"120000000000"
	);
	assert_eq!(histogram["dataPoints"][0]["timeUnixNano"], "180000000000");
	assert_eq!(
		histogram["dataPoints"][0]["bucketCounts"]
			.as_array()
			.unwrap()
			.len(),
		12
	);
	assert_eq!(
		histogram["dataPoints"][0]["explicitBounds"]
			.as_array()
			.unwrap()
			.len(),
		11
	);
	assert_eq!(histogram["dataPoints"][0]["sum"], 50.0);
	let again = export::otlp_metrics(&snapshot, 180);
	assert_eq!(
		again["resourceMetrics"][0]["scopeMetrics"][0]["metrics"][0]["histogram"]["dataPoints"],
		serde_json::json!([])
	);
}
