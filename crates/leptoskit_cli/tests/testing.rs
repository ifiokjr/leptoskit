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
	assert!(!native(invalid).is_empty());
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
#[ignore = "subprocess fixture"]
fn fixture_success() {
	println!("fixture output");
}

#[test]
#[ignore = "subprocess fixture"]
fn fixture_failure() {
	println!("fixture output");
	panic!("deliberate test failure");
}

#[test]
#[ignore = "subprocess fixture"]
fn fixture_timeout() {
	println!("fixture output before timeout");
	std::thread::sleep(std::time::Duration::from_secs(30));
}
