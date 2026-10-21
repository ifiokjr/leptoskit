//! Runs the `coverage_gate` binary against throwaway git repositories.

use std::fmt::Write as _;
use std::fs;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;
use std::process::Output;

/// A scratch repository with one committed base and one committed change.
///
/// Every git process, including the one the binary spawns, runs without the
/// global and system git configuration, so the developer's identity, signing,
/// and diff settings never leak into a fixture.
struct Repo {
	dir: PathBuf,
	empty_config: PathBuf,
}

impl Repo {
	fn new(name: &str) -> Self {
		let dir = std::env::temp_dir().join(format!("coverage_gate-{name}-{}", std::process::id()));
		let _ = fs::remove_dir_all(&dir);
		fs::create_dir_all(&dir).unwrap();
		let empty_config = dir.with_extension("gitconfig");
		fs::write(&empty_config, "").unwrap();
		let repo = Self { dir, empty_config };
		repo.git(&["init", "--quiet", "--initial-branch=main"]);
		repo
	}

	fn command(&self, program: impl AsRef<std::ffi::OsStr>) -> Command {
		let mut command = Command::new(program);
		command
			.current_dir(&self.dir)
			.env("GIT_CONFIG_GLOBAL", &self.empty_config)
			.env("GIT_CONFIG_NOSYSTEM", "1")
			.env("GIT_AUTHOR_NAME", "fixture")
			.env("GIT_AUTHOR_EMAIL", "fixture@example.invalid")
			.env("GIT_COMMITTER_NAME", "fixture")
			.env("GIT_COMMITTER_EMAIL", "fixture@example.invalid");
		command
	}

	fn git(&self, args: &[&str]) -> String {
		let output = self.command("git").args(args).output().unwrap();
		assert!(
			output.status.success(),
			"git {args:?}: {}",
			String::from_utf8_lossy(&output.stderr)
		);
		String::from_utf8(output.stdout).unwrap()
	}

	fn write(&self, path: &str, contents: &str) {
		let path = self.dir.join(path);
		fs::create_dir_all(path.parent().unwrap()).unwrap();
		fs::write(path, contents).unwrap();
	}

	fn commit(&self, message: &str) {
		self.git(&["add", "--all"]);
		self.git(&["commit", "--quiet", "--message", message]);
	}

	/// The repository root as git spells it, which is how the coverage tool
	/// spells source paths.
	fn root(&self) -> String {
		self.git(&["rev-parse", "--show-toplevel"])
			.trim()
			.to_owned()
	}

	/// Writes a tracefile with one record per `(path, [(line, hits)])`.
	fn tracefile(&self, records: &[(&str, &[(u32, u64)])]) {
		let root = self.root();
		let mut lcov = String::new();
		for (path, lines) in records {
			writeln!(lcov, "SF:{root}/{path}").unwrap();
			for (line, hits) in *lines {
				writeln!(lcov, "DA:{line},{hits}").unwrap();
			}
			lcov.push_str("end_of_record\n");
		}
		self.write("lcov.info", &lcov);
	}

	fn gate(&self, args: &[&str]) -> Output {
		self.command(env!("CARGO_BIN_EXE_coverage_gate"))
			.args(args)
			.output()
			.unwrap()
	}

	fn path(&self, path: &str) -> PathBuf {
		self.dir.join(path)
	}
}

impl Drop for Repo {
	fn drop(&mut self) {
		let _ = fs::remove_dir_all(&self.dir);
		let _ = fs::remove_file(&self.empty_config);
	}
}

/// A repository whose change adds lines 3 and 4 to `crates/a/src/lib.rs`, and
/// adds a test file that coverage must ignore.
fn changed_repo(name: &str) -> Repo {
	let repo = Repo::new(name);
	repo.write("crates/a/src/lib.rs", "fn a() {}\nfn b() {}\n");
	repo.write("floor.txt", "# comment\n50\n");
	repo.commit("base");
	repo.write(
		"crates/a/src/lib.rs",
		"fn a() {}\nfn b() {}\nfn c() {}\nfn d() {}\n",
	);
	repo.write("crates/a/tests/it.rs", "#[test]\nfn it() {}\n");
	repo.commit("change");
	repo
}

fn stdout(output: &Output) -> String {
	String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
	String::from_utf8_lossy(&output.stderr).into_owned()
}

#[test]
fn passes_a_covered_change_and_appends_the_summary() {
	let repo = changed_repo("covered");
	repo.tracefile(&[
		("crates/a/src/lib.rs", &[(1, 1), (2, 0), (3, 2), (4, 1)]),
		("crates/a/tests/it.rs", &[(2, 0)]),
	]);
	repo.write("summary.md", "# earlier step\n");

	let output = repo.gate(&[
		"--lcov",
		"lcov.info",
		"--floor",
		"floor.txt",
		"--base",
		"HEAD~1",
		"--markdown",
		"summary.md",
		"--annotate",
	]);

	assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
	let report = stdout(&output);
	assert!(report.contains("| `crates/a` | 4 | 75.00% |"), "{report}");
	assert!(report.contains("**100.00%** of 2 changed instrumented lines are covered"));
	assert!(!report.contains("::warning"));
	let summary = fs::read_to_string(repo.path("summary.md")).unwrap();
	assert!(summary.starts_with("# earlier step\n## Coverage\n"));
	assert!(summary.ends_with("**Result:** passed.\n"));
}

#[test]
fn fails_an_uncovered_change_and_annotates_it() {
	let repo = changed_repo("uncovered");
	repo.tracefile(&[("crates/a/src/lib.rs", &[(1, 1), (2, 1), (3, 0), (4, 0)])]);

	let output = repo.gate(&[
		"--lcov",
		"lcov.info",
		"--floor",
		"floor.txt",
		"--base",
		"HEAD~1",
		"--annotate",
	]);

	assert_eq!(output.status.code(), Some(1));
	assert!(stdout(&output).contains(
		"::warning file=crates/a/src/lib.rs,line=3,endLine=4,title=Uncovered change::Lines 3-4 \
		 are not covered by any test"
	));
	assert_eq!(
		stderr(&output),
		"coverage_gate: patch coverage 0.00% is below the minimum of 90.00%\n"
	);
}

#[test]
fn the_patch_minimum_is_configurable() {
	let repo = changed_repo("minimum");
	repo.tracefile(&[("crates/a/src/lib.rs", &[(1, 1), (2, 1), (3, 1), (4, 0)])]);

	let lenient = repo.gate(&[
		"--lcov",
		"lcov.info",
		"--floor",
		"floor.txt",
		"--base",
		"HEAD~1",
		"--patch-minimum",
		"50",
	]);
	let default = repo.gate(&[
		"--lcov",
		"lcov.info",
		"--floor",
		"floor.txt",
		"--base",
		"HEAD~1",
	]);

	assert_eq!(lenient.status.code(), Some(0), "{}", stderr(&lenient));
	assert_eq!(default.status.code(), Some(1));
}

#[test]
fn fails_below_the_floor_without_a_base() {
	let repo = changed_repo("floor");
	repo.tracefile(&[("crates/a/src/lib.rs", &[(1, 1), (2, 0), (3, 0), (4, 0)])]);

	let output = repo.gate(&["--lcov", "lcov.info", "--floor", "floor.txt"]);

	assert_eq!(output.status.code(), Some(1));
	assert!(!stdout(&output).contains("Patch coverage"));
	assert_eq!(
		stderr(&output),
		"coverage_gate: workspace coverage 25.00% is below the floor of 50.00%\n"
	);
}

#[test]
fn writes_the_measured_floor_keeping_its_comments() {
	let repo = changed_repo("write");
	repo.tracefile(&[("crates/a/src/lib.rs", &[(1, 1), (2, 1), (3, 1), (4, 0)])]);

	let output = repo.gate(&[
		"--lcov",
		"lcov.info",
		"--floor",
		"floor.txt",
		"--write-floor",
	]);

	assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
	assert_eq!(stdout(&output), "coverage floor set to 75.00%\n");
	assert_eq!(
		fs::read_to_string(repo.path("floor.txt")).unwrap(),
		"# comment\n75.00\n"
	);
}

#[test]
fn reports_unusable_inputs() {
	let repo = changed_repo("inputs");
	repo.tracefile(&[("crates/a/src/lib.rs", &[(1, 1)])]);

	let missing = repo.gate(&["--lcov", "missing.info", "--floor", "floor.txt"]);
	assert_eq!(missing.status.code(), Some(2));
	assert!(stderr(&missing).starts_with("coverage_gate: missing.info: "));

	let usage = repo.gate(&["--lcov"]);
	assert_eq!(usage.status.code(), Some(2));
	assert!(stderr(&usage).starts_with("coverage_gate: --lcov needs a value\n\nusage:"));

	let unknown = repo.gate(&["--lcov", "lcov.info", "--floor", "floor.txt", "--fast"]);
	assert!(stderr(&unknown).starts_with("coverage_gate: unknown argument `--fast`"));

	let required = repo.gate(&["--floor", "floor.txt"]);
	assert!(stderr(&required).starts_with("coverage_gate: --lcov is required"));

	repo.write("two.txt", "50\n60\n");
	let ambiguous = repo.gate(&["--lcov", "lcov.info", "--floor", "two.txt"]);
	assert_eq!(ambiguous.status.code(), Some(2));
	assert_eq!(
		stderr(&ambiguous),
		"coverage_gate: two.txt: expected exactly one percentage\n"
	);

	let base = repo.gate(&[
		"--lcov",
		"lcov.info",
		"--floor",
		"floor.txt",
		"--base",
		"no-such-rev",
	]);
	assert_eq!(base.status.code(), Some(2));
	assert!(stderr(&base).contains("failed"), "{}", stderr(&base));

	repo.write("bad.info", "DA:1,1\n");
	let malformed = repo.gate(&["--lcov", "bad.info", "--floor", "floor.txt"]);
	assert_eq!(malformed.status.code(), Some(2));
	assert_eq!(
		stderr(&malformed),
		"coverage_gate: lcov line 1: `DA` outside a record\n"
	);
}

#[test]
fn fails_outside_a_git_repository() {
	let dir = std::env::temp_dir().join(format!("coverage_gate-nogit-{}", std::process::id()));
	fs::create_dir_all(&dir).unwrap();
	// A ceiling stops git from finding an enclosing repository, such as the
	// one the temporary directory may live in.
	let output = Command::new(env!("CARGO_BIN_EXE_coverage_gate"))
		.args(["--lcov", "lcov.info", "--floor", "floor.txt"])
		.current_dir(&dir)
		.env(
			"GIT_CEILING_DIRECTORIES",
			dir.parent().unwrap_or(Path::new("/")),
		)
		.output()
		.unwrap();
	let _ = fs::remove_dir_all(&dir);

	assert_eq!(output.status.code(), Some(2));
	assert!(stderr(&output).contains("`git rev-parse --show-toplevel` failed"));
}
