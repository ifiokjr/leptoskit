use std::cell::Cell;

use leptoskit_analytics::*;

thread_local! { static NOW: Cell<u64> = const { Cell::new(120) }; }
fn clock() -> u64 {
	NOW.get()
}
fn collector(limits: Limits) -> Analytics {
	NOW.set(120);
	Analytics::new(
		Resource {
			service: "notes".into(),
			environment: "test".into(),
			release: "a1".into(),
			instance: "one".into(),
		},
		limits,
		clock,
	)
	.unwrap()
}
fn key(source: Source, operation: &str, outcome: Outcome) -> SeriesKey {
	SeriesKey {
		source,
		signal: Signal::Http,
		operation: operation.into(),
		outcome,
	}
}

#[test]
fn fixed_histograms_expire_and_handle_clock_steps() {
	let analytics = collector(Limits {
		retained_windows: 2,
		..Limits::default()
	});
	let key = key(Source::Backend, "/notes/{id}", Outcome::Ok);
	for value in [0.0, 5.0, 5.1, 10000.0, 10001.0] {
		assert!(analytics.record(key.clone(), value, None));
	}
	let snapshot = analytics.snapshot();
	let window = &snapshot.series[0].windows[0];
	assert_eq!(window.latency[0], 2);
	assert_eq!(window.latency[1], 1);
	assert_eq!(window.latency[10], 1);
	assert_eq!(window.latency[11], 1);
	assert_eq!(window.p95_upper_ms(), None);
	NOW.set(180);
	analytics.record(key.clone(), 50.0, None);
	assert_eq!(analytics.snapshot().series[0].windows.len(), 2);
	NOW.set(100); // Clock cannot reopen old windows.
	analytics.record(key.clone(), 50.0, None);
	assert_eq!(analytics.snapshot().observed_unix_seconds, 180);
	NOW.set(240);
	assert_eq!(
		analytics.snapshot().series[0].windows[0].start_unix_seconds,
		180
	);
	NOW.set(300);
	assert!(analytics.snapshot().series.is_empty());
}

#[test]
fn browsers_cannot_exhaust_backend_capacity_or_create_backend_findings() {
	let analytics = collector(Limits {
		max_series: 2,
		..Limits::default()
	});
	assert!(analytics.record(
		key(Source::Browser, "/forged", Outcome::ServerError),
		1.0,
		None
	));
	assert!(!analytics.record(key(Source::Browser, "/another", Outcome::Ok), 1.0, None));
	for _ in 0..200 {
		analytics.record(
			key(Source::Browser, "/forged", Outcome::ServerError),
			1.0,
			None,
		);
	}
	assert!(analytics.record(key(Source::Backend, "/first", Outcome::Ok), 1.0, None));
	assert!(analytics.record(key(Source::Backend, "/second", Outcome::Ok), 1.0, None));
	assert!(
		analytics
			.snapshot()
			.series
			.iter()
			.all(|s| s.key.source == Source::Backend)
	);
	assert!(!analytics.record(key(Source::Backend, "/third", Outcome::Ok), 1.0, None));
	NOW.set(180);
	assert!(analytics.evidence().findings.is_empty());
	assert!(!analytics.evidence().infrastructure_changes_authorized);
	assert_eq!(analytics.snapshot().dropped_series_observations, 2);
	assert_eq!(analytics.snapshot().evicted_series, 1);
}

#[test]
fn findings_use_completed_backend_windows_and_actual_sample_counts() {
	let analytics = collector(Limits::default());
	for i in 0..100 {
		analytics.record(
			key(
				Source::Backend,
				"/api/notes",
				if i < 5 {
					Outcome::ServerError
				} else {
					Outcome::Ok
				},
			),
			10.0,
			None,
		);
	}
	assert!(analytics.evidence().findings.is_empty()); // current window
	NOW.set(180);
	let evidence = analytics.evidence();
	assert_eq!(evidence.findings.len(), 1);
	assert_eq!(evidence.findings[0].samples, 100);
	assert_eq!(evidence.findings[0].failures, 5);
	assert!(!evidence.infrastructure_changes_authorized);
	assert!(
		evidence
			.snapshot
			.limitations
			.iter()
			.any(|s| s.contains("Single process"))
	);
}

#[test]
fn invalid_records_and_correlations_are_bounded_and_validated() {
	let analytics = collector(Limits {
		max_recent: 2,
		..Limits::default()
	});
	let key = key(Source::Backend, "/notes/{id}", Outcome::Ok);
	let context =
		TraceContext::parse("00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01").unwrap();
	for _ in 0..5 {
		analytics.record(key.clone(), 10.0, Some(context.clone()));
	}
	for duration in [f64::NAN, f64::INFINITY, -1.0, 86_400_001.0] {
		assert!(!analytics.record(key.clone(), duration, None));
	}
	assert!(!analytics.record(super_key("/notes?email=private@example.com"), 1.0, None));
	let snapshot = analytics.snapshot();
	assert_eq!(snapshot.invalid_observations, 5);
	assert_eq!(snapshot.recent.len(), 2);
	assert_eq!(snapshot.series[0].windows[0].count, 5);
	assert_eq!(snapshot.series[0].windows[0].p95_upper_ms(), Some(10));
	assert!(
		!serde_json::to_string(&snapshot)
			.unwrap()
			.contains("private@")
	);
	NOW.set(1200);
	assert!(analytics.snapshot().recent.is_empty());
}

fn super_key(operation: &str) -> SeriesKey {
	key(Source::Backend, operation, Outcome::Ok)
}

#[test]
fn context_deserialization_enforces_w3c_version_zero() {
	for value in [
		"",
		"00-00000000000000000000000000000000-00f067aa0ba902b7-00",
		"00-4bf92f3577b34da6a3ce929d0e0e4736-0000000000000000-00",
		"ff-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-00",
		"00-4BF92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-00",
		"00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-00-extra",
	] {
		assert!(TraceContext::parse(value).is_none());
		assert!(serde_json::from_value::<TraceContext>(serde_json::json!(value)).is_err());
	}
}
