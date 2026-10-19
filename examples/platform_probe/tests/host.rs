//! Desktop validation: the full probe suite as an ordinary host test.

#[test]
fn desktop_platform_probe() {
	let failures = platform_probe::run_all();
	assert_eq!(
		failures, 0,
		"the probe output above names the failing probes"
	);
}
