//! Exercise the runner with real subprocesses, including failure and deadlines.
use std::path::PathBuf;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;

use clap::Parser;
use leptoskit_cli::cli::Cli;
use leptoskit_cli::testing::RunArgs;
use leptoskit_cli::testing::TestCommand;
use leptoskit_cli::testing::TestPlatform;

static ID: AtomicU64 = AtomicU64::new(0);

struct Scratch(PathBuf);
impl Scratch {
	fn new() -> Self {
		Self(std::env::temp_dir().join(format!(
			"leptoskit-runner-{}-{}",
			std::process::id(),
			ID.fetch_add(1, Ordering::Relaxed)
		)))
	}

	fn report(&self) -> serde_json::Value {
		let run = std::fs::read_dir(&self.0)
			.unwrap()
			.next()
			.unwrap()
			.unwrap()
			.path();
		serde_json::from_slice(&std::fs::read(run.join("report.json")).unwrap()).unwrap()
	}
}
impl Drop for Scratch {
	fn drop(&mut self) {
		std::fs::remove_dir_all(&self.0).expect("remove test artifacts");
	}
}

fn options(directory: &Scratch, fixture: &str) -> RunArgs {
	RunArgs {
		binary: std::env::current_exe().unwrap(),
		platform: TestPlatform::Host,
		device: None,
		avd: None,
		timeout: 10,
		artifacts: directory.0.clone(),
		args: ["--ignored", "--exact", fixture, "--nocapture"]
			.into_iter()
			.map(Into::into)
			.collect(),
	}
}

#[test]
fn successful_and_failed_executables_produce_accurate_reports() {
	for (fixture, passes) in [("fixture_success", true), ("fixture_failure", false)] {
		let directory = Scratch::new();
		let result = leptoskit_cli::testing::run(TestCommand::Run(options(&directory, fixture)));
		assert_eq!(result.is_ok(), passes);
		let report = directory.report();
		assert_eq!(report["passed"], passes);
		assert_eq!(report["steps"][0]["timed_out"], false);
		assert_eq!(report["steps"][0]["exit_code"] == 0, passes);

		let run = std::fs::read_dir(&directory.0)
			.unwrap()
			.next()
			.unwrap()
			.unwrap()
			.path();
		let output =
			std::fs::read_to_string(run.join(report["steps"][0]["stdout"].as_str().unwrap()))
				.unwrap();
		assert!(output.contains("fixture output"));
	}
}

#[test]
fn timeout_fails_and_retains_partial_output() {
	let directory = Scratch::new();
	let mut options = options(&directory, "fixture_timeout");
	options.timeout = 1;
	assert!(leptoskit_cli::testing::run(TestCommand::Run(options)).is_err());
	let report = directory.report();
	assert_eq!(report["passed"], false);
	assert_eq!(report["steps"][0]["timed_out"], true);
	let run = std::fs::read_dir(&directory.0)
		.unwrap()
		.next()
		.unwrap()
		.unwrap()
		.path();

	let output =
		std::fs::read_to_string(run.join(report["steps"][0]["stdout"].as_str().unwrap())).unwrap();
	assert!(output.contains("fixture output before timeout"));
}

#[test]
fn invalid_binary_and_mobile_options_fail_with_a_report() {
	for missing in [false, true] {
		let directory = Scratch::new();
		let mut options = options(&directory, "fixture_success");

		if missing {
			options.binary = directory.0.join("missing");
		} else {
			options.device = Some("phone".to_owned());
		}

		assert!(leptoskit_cli::testing::run(TestCommand::Run(options)).is_err());

		assert_eq!(directory.report()["passed"], false);
		assert!(directory.report()["error"].is_string());
	}
}

#[test]
fn native_suites_report_unreadable_invalid_and_empty_scenarios() {
	let directory = Scratch::new();
	std::fs::create_dir_all(&directory.0).unwrap();
	let native = |suite: PathBuf| {
		leptoskit_cli::testing::run(TestCommand::Native { suite })
			.unwrap_err()
			.to_string()
	};
	let missing = directory.0.join("missing.json");
	assert!(native(missing.clone()).contains(&format!("reading {}", missing.display())));
	let invalid = directory.0.join("invalid.json");
	std::fs::write(&invalid, r#"{"session": {}, "steps": [], "extra": true}"#).unwrap();
	assert_ne!(native(invalid), "");

	// A scenario without steps fails before contacting any driver.
	let empty = directory.0.join("empty.json");
	std::fs::write(
		&empty,
		r#"{"session": {"endpoint": "http://127.0.0.1:9", "platform": "linux",
		"application": {"kind": "desktop"}}, "steps": []}"#,
	)
	.unwrap();
	assert!(native(empty).contains("native suite has no steps"));
}

#[test]
fn cli_preserves_test_arguments_and_rejects_zero_deadlines() {
	assert!(
		Cli::try_parse_from([
			"leptoskit",
			"test",
			"run",
			"--binary",
			"test",
			"--timeout",
			"0"
		])
		.is_err()
	);
	let cli = Cli::try_parse_from([
		"leptoskit",
		"test",
		"run",
		"--binary",
		"test",
		"--",
		"--exact",
		"a name",
	])
	.unwrap();
	let leptoskit_cli::cli::Command::Test {
		command: TestCommand::Run(args),
	} = cli.command
	else {
		panic!("wrong command")
	};
	assert_eq!(args.args, ["--exact", "a name"]);
}

#[test]
#[ignore = "subprocess fixture: the report tests above run it"]
fn fixture_success() {
	println!("fixture output");
}

#[test]
#[ignore = "subprocess fixture: fails on purpose; the report tests above run it"]
fn fixture_failure() {
	println!("fixture output");
	panic!("deliberate test failure");
}

#[test]
#[ignore = "subprocess fixture: hangs on purpose; the timeout test above runs it"]
fn fixture_timeout() {
	println!("fixture output before timeout");
	std::thread::sleep(std::time::Duration::from_secs(30));
}

#[cfg(unix)]
fn fake_tool(path: &std::path::Path, script: &str) {
	use std::os::unix::fs::PermissionsExt;
	std::fs::create_dir_all(path.parent().unwrap()).unwrap();
	std::fs::write(path, format!("#!/bin/sh\nset -eu\n{script}\n")).unwrap();
	std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).unwrap();
}

#[cfg(unix)]
#[test]
fn android_runner_cleans_up_its_fixture_after_success_and_failure() {
	for failure in ["none", "test", "both", "wait", "exited"] {
		let directory = Scratch::new();
		let sdk = directory.0.join("sdk");
		let log = directory.0.join("commands.txt");
		fake_tool(
			&sdk.join("platform-tools/adb"),
			r#"
if [ "$1" = devices ]; then printf 'List of devices attached\n'; exit 0; fi
[ "$1" = -s ] && [ "$2" = emulator-5554 ]
shift 2
printf '%s\n' "$*" >> "$LK_TEST_LOG"
case "$1" in
  wait-for-device)
    if [ "$LK_FAIL" = exited ]; then sleep 3; fi
    if [ "$LK_FAIL" = wait ]; then exit 7; fi;;
  shell)
    case "$2" in
      'timeout -s KILL '*)
        printf 'fixture device output\n'
        if [ "$LK_FAIL" = test ] || [ "$LK_FAIL" = both ]; then exit 13; fi;;
      'rm -rf '*) if [ "$LK_FAIL" = both ]; then exit 14; fi;;
    esac;;
esac
"#,
		);
		fake_tool(
			&sdk.join("emulator/emulator"),
			r#"
printf '%s\n' "$*" > "$LK_EMULATOR_ARGS"
if [ "$LK_FAIL" = exited ]; then exit 23; fi
exec sleep 60
"#,
		);
		let output = std::process::Command::new(env!("CARGO_BIN_EXE_leptoskit"))
			.args(["test", "run", "--binary"])
			.arg(std::env::current_exe().unwrap())
			.args([
				"--platform",
				"android",
				"--device",
				"emulator-5554",
				"--avd",
				"fixture",
				"--timeout",
				"5",
				"--artifacts",
			])
			.arg(directory.0.join("reports"))
			.args(["--", "two words", "a 'quote'"])
			.env("ANDROID_HOME", &sdk)
			.env("LK_TEST_LOG", &log)
			.env("LK_EMULATOR_ARGS", directory.0.join("emulator-args.txt"))
			.env("LK_FAIL", failure)
			.output()
			.unwrap();
		let reports = Scratch(directory.0.join("reports"));
		let report = reports.report();
		assert_eq!(output.status.success(), failure == "none", "{report}");
		assert_eq!(report["passed"], failure == "none", "{report}");
		let recorded = std::fs::read_to_string(&log).unwrap();
		assert!(
			!recorded.contains("emu kill"),
			"the runner must reap its own process, not kill by reusable serial"
		);
		if failure == "none" || failure == "test" || failure == "both" {
			assert!(
				recorded.contains("'two words' 'a '\\''quote'\\'''"),
				"{recorded}"
			);
			assert_eq!(
				report["steps"].as_array().unwrap().last().unwrap()["name"],
				"remove-test"
			);
		}
		if failure == "both" {
			let error = report["error"].as_str().unwrap();
			assert!(
				error.contains("cleanup also failed") && error.contains("test failed"),
				"{error}"
			);
		}
		if failure == "exited" {
			assert!(
				report["error"]
					.as_str()
					.unwrap()
					.contains("emulator exited during"),
				"{report}"
			);
		}
		assert!(
			std::fs::read_to_string(directory.0.join("emulator-args.txt"))
				.unwrap()
				.contains("-avd fixture -port 5554 -no-window")
		);
	}
}

#[cfg(unix)]
#[test]
fn browser_runner_selects_only_the_requested_driver() {
	for browser in ["chrome", "firefox"] {
		let directory = Scratch::new();
		let bin = directory.0.join("bin");
		fake_tool(
			&bin.join("cargo"),
			r#"
[ "$*" = 'test --target wasm32-unknown-unknown --package fixture_widgets' ]
[ "$WASM_BINDGEN_USE_BROWSER" = 1 ]
[ "$WASM_BINDGEN_TEST_ONLY_WEB" = 1 ]
[ "${NO_HEADLESS-unset}" = unset ]
[ "${SAFARIDRIVER-unset}" = unset ]
case "$LK_BROWSER" in
  chrome) [ "$CHROMEDRIVER" = fixture-chrome ]; [ "${GECKODRIVER-unset}" = unset ];;
  firefox) [ "$GECKODRIVER" = fixture-firefox ]; [ "${CHROMEDRIVER-unset}" = unset ];;
esac
printf 'fixture browser output\n'
"#,
		);
		let mut paths = vec![bin];
		paths.extend(std::env::split_paths(&std::env::var_os("PATH").unwrap()));
		let output = std::process::Command::new(env!("CARGO_BIN_EXE_leptoskit"))
			.args([
				"test",
				"web",
				"--package",
				"fixture_widgets",
				"--browser",
				browser,
				"--timeout",
				"5",
				"--artifacts",
			])
			.arg(directory.0.join("reports"))
			.env("PATH", std::env::join_paths(paths).unwrap())
			.env("CHROMEDRIVER", "fixture-chrome")
			.env("GECKODRIVER", "fixture-firefox")
			.env("SAFARIDRIVER", "fixture-safari")
			.env("NO_HEADLESS", "1")
			.env("LK_BROWSER", browser)
			.output()
			.unwrap();
		let reports = Scratch(directory.0.join("reports"));
		let report = reports.report();
		assert!(
			output.status.success(),
			"{report}: {}",
			String::from_utf8_lossy(&output.stderr)
		);
		assert_eq!(report["platform"], "web");
		assert_eq!(report["passed"], true);
		assert_eq!(report["steps"][0]["name"], "browser");
	}
}
