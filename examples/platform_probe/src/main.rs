//! The probe binary: prints one PASS/FAIL line per probe and exits non-zero
//! on any failure. Run on the host, the iOS simulator, and an Android
//! emulator.

fn main() {
	let failures = platform_probe::run_all();
	if failures == 0 {
		println!(
			"platform probe: all probes passed on {}",
			std::env::consts::OS
		);
	} else {
		println!(
			"platform probe: {failures} probe(s) failed on {}",
			std::env::consts::OS
		);
	}
	std::process::exit(i32::try_from(failures).unwrap_or(1));
}
