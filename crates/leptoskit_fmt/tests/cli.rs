//! The `leptoskit-fmt` binary, end to end: the same command `leptoskit
//! format` runs.

use std::io::Write as _;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;
use std::process::Output;
use std::process::Stdio;

const UNFORMATTED: &str =
	"fn app() -> impl IntoView {\nview! {   <p class=\"a\">\"hi\"</p>   }\n}\n";
const FORMATTED: &str = "fn app() -> impl IntoView {\n\tview! { <p class=\"a\">\"hi\"</p> }\n}\n";

/// A scratch project with its own `rustfmt.toml` and cache directory.
struct Project {
	root: PathBuf,
}

impl Project {
	fn new(name: &str) -> Self {
		let root =
			std::env::temp_dir().join(format!("leptoskit-fmt-cli-{name}-{}", std::process::id()));
		let _ = std::fs::remove_dir_all(&root);
		std::fs::create_dir_all(root.join("src")).expect("create project");
		std::fs::write(
			root.join("rustfmt.toml"),
			"edition = \"2024\"\nhard_tabs = true\n",
		)
		.expect("rustfmt.toml");
		std::fs::write(root.join("Cargo.lock"), "").expect("Cargo.lock");
		Self { root }
	}

	fn write(&self, path: &str, contents: &str) -> PathBuf {
		let path = self.root.join(path);
		std::fs::create_dir_all(path.parent().expect("parent")).expect("create dir");
		std::fs::write(&path, contents).expect("write");
		path
	}

	fn read(&self, path: &str) -> String {
		std::fs::read_to_string(self.root.join(path)).expect("read")
	}

	fn command(&self, args: &[&str]) -> Command {
		let mut command = Command::new(env!("CARGO_BIN_EXE_leptoskit-fmt"));
		command
			.args(args)
			.current_dir(&self.root)
			// The rustfmt on the `PATH`: rustup's, for the toolchain cargo
			// runs the tests with.
			.env(leptoskit_fmt::RUSTFMT_ENV, "rustfmt")
			.env("NO_COLOR", "1")
			.env_remove("CARGO_TARGET_DIR");
		command
	}

	fn run(&self, args: &[&str]) -> Output {
		self.command(args).output().expect("run leptoskit-fmt")
	}
}

impl Drop for Project {
	fn drop(&mut self) {
		let _ = std::fs::remove_dir_all(&self.root);
	}
}

fn text(bytes: &[u8]) -> String {
	String::from_utf8_lossy(bytes).into_owned()
}

#[test]
fn check_reports_a_diff_and_fails_without_writing() {
	let project = Project::new("check");
	project.write("src/lib.rs", UNFORMATTED);
	let output = project.run(&["--check"]);
	assert_eq!(output.status.code(), Some(1), "{}", text(&output.stderr));
	let stdout = text(&output.stdout);
	assert!(
		stdout.contains("-view! {   <p class=\"a\">\"hi\"</p>   }"),
		"{stdout}"
	);
	assert!(
		stdout.contains("+\tview! { <p class=\"a\">\"hi\"</p> }"),
		"{stdout}"
	);
	assert!(
		text(&output.stderr).contains("would format 1 of 1 files"),
		"{}",
		text(&output.stderr)
	);
	assert_eq!(project.read("src/lib.rs"), UNFORMATTED);
}

#[test]
fn formatting_writes_files_and_then_skips_them() {
	let project = Project::new("write");
	project.write("src/lib.rs", UNFORMATTED);
	project.write("src/other.rs", FORMATTED);
	let output = project.run(&["--verbose"]);
	assert!(output.status.success(), "{}", text(&output.stderr));
	assert_eq!(project.read("src/lib.rs"), FORMATTED);
	assert!(
		text(&output.stderr).contains("formatted 1 of 2 files"),
		"{}",
		text(&output.stderr)
	);

	let again = project.run(&["--check", "--verbose"]);
	assert!(again.status.success(), "{}", text(&again.stderr));
	let stderr = text(&again.stderr);
	assert_eq!(
		stderr.matches(" cached ").count(),
		2,
		"both files are known formatted: {stderr}"
	);

	let uncached = project.run(&["--check", "--verbose", "--no-cache"]);
	assert!(uncached.status.success());
	assert_eq!(
		text(&uncached.stderr).matches(" unchanged ").count(),
		2,
		"{}",
		text(&uncached.stderr)
	);
}

#[test]
fn explicit_paths_and_excludes_choose_the_files() {
	let project = Project::new("paths");
	let lib = project.write("src/lib.rs", UNFORMATTED);
	project.write("tests/fixtures/input.rs", UNFORMATTED);
	let output = project.run(&["--check", "--exclude", "**/fixtures/**"]);
	assert_eq!(output.status.code(), Some(1));
	assert!(
		text(&output.stderr).contains("of 1 files"),
		"{}",
		text(&output.stderr)
	);

	let output = project.run(&[lib.to_str().expect("utf-8 path")]);
	assert!(output.status.success(), "{}", text(&output.stderr));
	assert_eq!(project.read("src/lib.rs"), FORMATTED);
	assert_eq!(project.read("tests/fixtures/input.rs"), UNFORMATTED);
}

#[test]
fn stdin_formats_to_stdout() {
	let project = Project::new("stdin");
	let stdin = |args: &[&str], input: &str| {
		let mut child = project
			.command(args)
			.stdin(Stdio::piped())
			.stdout(Stdio::piped())
			.stderr(Stdio::piped())
			.spawn()
			.expect("spawn");
		child
			.stdin
			.take()
			.expect("stdin")
			.write_all(input.as_bytes())
			.expect("write stdin");
		child.wait_with_output().expect("wait")
	};
	let filepath = project.root.join("src/lib.rs");
	let filepath = filepath.to_str().expect("utf-8 path");
	let output = stdin(
		&["--stdin", "--stdin-filepath", filepath, "--verbose"],
		UNFORMATTED,
	);
	assert!(output.status.success(), "{}", text(&output.stderr));
	assert_eq!(text(&output.stdout), FORMATTED);
	assert!(
		text(&output.stderr).contains("rustfmt runs"),
		"{}",
		text(&output.stderr)
	);

	let output = stdin(&["--stdin", "--check"], UNFORMATTED);
	assert_eq!(output.status.code(), Some(1));
	let output = stdin(&["--stdin", "--check"], FORMATTED);
	assert!(output.status.success(), "{}", text(&output.stderr));
}

#[test]
fn failures_exit_with_two() {
	let project = Project::new("failures");
	project.write("src/lib.rs", "fn broken( {\n");
	let output = project.run(&["--check"]);
	assert_eq!(output.status.code(), Some(2));
	assert!(
		text(&output.stderr).contains("rustfmt failed"),
		"{}",
		text(&output.stderr)
	);

	let output = project.run(&["missing.rs"]);
	assert_eq!(output.status.code(), Some(2));
	assert!(
		text(&output.stderr).starts_with("error: missing.rs"),
		"{}",
		text(&output.stderr)
	);

	let output = project.run(&["--exclude", "[", "src"]);
	assert_eq!(output.status.code(), Some(2), "{}", text(&output.stderr));
}

#[test]
fn markup_that_does_not_parse_is_a_warning() {
	let project = Project::new("warning");
	let source = "fn app() {\n\tview! { <div> };\n}\n";
	project.write("src/lib.rs", source);
	let output = project.run(&["--check"]);
	assert!(output.status.success(), "{}", text(&output.stderr));
	let stderr = text(&output.stderr);
	assert!(
		stderr.contains(&format!("warning: {}", Path::new("./src/lib.rs").display())),
		"{stderr}"
	);
	assert!(stderr.contains("markup left as written"), "{stderr}");
}
