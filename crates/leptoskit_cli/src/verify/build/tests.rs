use super::*;

fn run_git(repository: &Path, args: &[&str]) -> String {
	let output = git(repository)
		.args(["-c", "user.name=Test", "-c", "user.email=test@example.com"])
		.args(["-c", "commit.gpgsign=false"])
		.args(args)
		.output()
		.unwrap();
	assert!(
		output.status.success(),
		"git {args:?}: {}",
		String::from_utf8_lossy(&output.stderr)
	);
	String::from_utf8(output.stdout).unwrap().trim().to_owned()
}

/// A repository whose one commit holds `files`, returning its commit ID.
fn repository(files: &[(&str, &str)]) -> (tempfile::TempDir, String) {
	let directory = tempfile::tempdir().unwrap();
	run_git(directory.path(), &["init", "--quiet"]);

	for (path, contents) in files {
		let path = directory.path().join(path);
		fs::create_dir_all(path.parent().unwrap()).unwrap();
		fs::write(path, contents).unwrap();
	}

	run_git(directory.path(), &["add", "--all"]);
	run_git(
		directory.path(),
		&["commit", "--quiet", "--message", "fixture"],
	);
	let commit = run_git(directory.path(), &["rev-parse", "HEAD"]);
	(directory, commit)
}

fn recipe() -> BuildRecipe {
	BuildRecipe {
		repository: "https://github.com/example/backend".to_owned(),
		commit: "a".repeat(40),
		builder_image: format!("docker.io/library/rust@sha256:{}", "b".repeat(64)),
		target: "x86_64-unknown-linux-gnu".to_owned(),
		package: "backend".to_owned(),
		binary: "server".to_owned(),
		features: vec!["postgres".to_owned(), "tls".to_owned()],
		no_default_features: true,
		rustflags: vec!["-C".to_owned(), "target-cpu=x86-64-v2".to_owned()],
		infra: None,
	}
}

#[test]
fn export_writes_the_commit_tree_exactly_and_ignores_export_attributes() {
	let (repository, commit) = repository(&[
		("Cargo.toml", "[package]\nname = \"backend\"\n"),
		("src/main.rs", "fn main() {}\n"),
		(
			".gitattributes",
			"excluded.txt export-ignore\nstamp.txt export-subst\n",
		),
		("excluded.txt", "git archive would drop this\n"),
		("stamp.txt", "$Format:%H$\n"),
	]);
	let dest = tempfile::tempdir().unwrap();
	export_tree(
		repository.path(),
		&commit,
		None,
		&dest.path().join("source"),
	)
	.unwrap();
	let source = dest.path().join("source");

	assert_eq!(
		fs::read_to_string(source.join("excluded.txt")).unwrap(),
		"git archive would drop this\n"
	);
	assert_eq!(
		fs::read_to_string(source.join("stamp.txt")).unwrap(),
		"$Format:%H$\n"
	);
	assert!(!source.join(".git").exists());
	let entries = crate::verify::walk(&source).unwrap();
	assert_eq!(
		entries.keys().map(String::as_str).collect::<Vec<_>>(),
		[
			".gitattributes",
			"Cargo.toml",
			"excluded.txt",
			"src",
			"src/main.rs",
			"stamp.txt"
		]
	);
}

#[cfg(unix)]
#[test]
fn export_keeps_the_executable_bit_whatever_the_umask() {
	use std::os::unix::fs::PermissionsExt as _;

	let (repository, commit) = repository(&[("run.sh", "#!/bin/sh\n"), ("notes.txt", "")]);
	fs::set_permissions(
		repository.path().join("run.sh"),
		fs::Permissions::from_mode(0o755),
	)
	.unwrap();
	run_git(repository.path(), &["add", "run.sh"]);
	run_git(
		repository.path(),
		&["commit", "--quiet", "--message", "executable"],
	);
	let commit_with_mode = run_git(repository.path(), &["rev-parse", "HEAD"]);
	assert_ne!(commit, commit_with_mode);
	let dest = tempfile::tempdir().unwrap();
	export_tree(repository.path(), &commit_with_mode, None, dest.path()).unwrap();

	let mode = |name: &str| {
		fs::metadata(dest.path().join(name))
			.unwrap()
			.permissions()
			.mode()
			& 0o777
	};
	assert_eq!(mode("run.sh"), 0o755);
	assert_eq!(mode("notes.txt"), 0o644);
}

#[test]
fn submodules_and_unknown_commits_are_rejected() {
	let (repository, commit) = repository(&[("Cargo.toml", "")]);
	run_git(
		repository.path(),
		&[
			"update-index",
			"--add",
			"--cacheinfo",
			&format!("160000,{commit},vendor/library"),
		],
	);
	run_git(
		repository.path(),
		&["commit", "--quiet", "--message", "submodule"],
	);
	let with_submodule = run_git(repository.path(), &["rev-parse", "HEAD"]);
	let dest = tempfile::tempdir().unwrap();
	let error = export_tree(
		repository.path(),
		&with_submodule,
		None,
		&dest.path().join("a"),
	)
	.unwrap_err();
	assert!(error.to_string().contains("submodule"), "{error}");

	let error = export_tree(
		repository.path(),
		&"c".repeat(40),
		None,
		&dest.path().join("b"),
	)
	.unwrap_err();
	assert!(format!("{error:#}").contains("is not in"), "{error:#}");
}

#[cfg(unix)]
#[test]
fn symbolic_links_in_the_commit_are_rejected() {
	let (repository, _) = repository(&[("Cargo.toml", "")]);
	std::os::unix::fs::symlink("/etc/passwd", repository.path().join("link")).unwrap();
	run_git(repository.path(), &["add", "link"]);
	run_git(
		repository.path(),
		&["commit", "--quiet", "--message", "link"],
	);
	let commit = run_git(repository.path(), &["rev-parse", "HEAD"]);
	let dest = tempfile::tempdir().unwrap();
	let error = export_tree(repository.path(), &commit, None, dest.path()).unwrap_err();
	assert!(error.to_string().contains("symbolic link"), "{error}");
}

#[test]
fn an_infrastructure_prefix_exports_only_that_directory() {
	let (repository, commit) = repository(&[
		("Cargo.toml", ""),
		("deploy/main.tf", "resource {}\n"),
		("deploy/policy/workload.json", "{}\n"),
		("deployment-notes.txt", "not infrastructure\n"),
	]);
	let dest = tempfile::tempdir().unwrap();
	export_tree(
		repository.path(),
		&commit,
		Some("deploy"),
		&dest.path().join("infra"),
	)
	.unwrap();
	let entries = crate::verify::walk(&dest.path().join("infra")).unwrap();
	assert_eq!(
		entries.keys().map(String::as_str).collect::<Vec<_>>(),
		["main.tf", "policy", "policy/workload.json"]
	);

	for prefix in ["absent", "Cargo.toml"] {
		let error = export_tree(
			repository.path(),
			&commit,
			Some(prefix),
			&dest.path().join(prefix),
		)
		.unwrap_err();
		assert!(error.to_string().contains("directory"), "{prefix}: {error}");
	}
}

#[test]
fn only_linux_targets_with_a_native_builder_platform_are_supported() {
	assert_eq!(platform("x86_64-unknown-linux-gnu").unwrap(), "linux/amd64");
	assert_eq!(
		platform("aarch64-unknown-linux-musl").unwrap(),
		"linux/arm64"
	);

	for target in [
		"wasm32-unknown-unknown",
		"x86_64-apple-darwin",
		"riscv64gc-unknown-linux-gnu",
	] {
		assert!(platform(target).is_err(), "{target}");
	}
}

fn builder(recipe: &BuildRecipe, engine: Engine) -> Builder<'_> {
	Builder {
		engine,
		recipe,
		platform: platform(&recipe.target).unwrap(),
	}
}

fn arguments(run: &Run) -> Vec<String> {
	run.arguments()
		.into_iter()
		.map(|argument| argument.into_string().unwrap())
		.collect()
}

#[test]
fn compiling_has_no_network_and_only_scratch_space_is_writable() {
	let recipe = recipe();
	let directories = tempfile::tempdir().unwrap();
	let [source, dependencies, work] = ["source", "dependencies", "work"].map(|name| {
		let path = directories.path().join(name);
		fs::create_dir(&path).unwrap();
		path
	});
	let run = builder(&recipe, Engine::Podman)
		.compile_run(&source, &dependencies, &work)
		.unwrap();
	let arguments = arguments(&run);
	let joined = arguments.join(" ");

	for expected in [
		"run --rm --pull=never --platform linux/amd64 --read-only",
		"--cap-drop ALL --security-opt no-new-privileges",
		"--network none",
		"target=/build/source,readonly=true",
		"target=/build/dependencies,readonly=true",
		"--env CARGO_NET_OFFLINE=true",
		"--env SOURCE_DATE_EPOCH=315532800",
		"--env CARGO_ENCODED_RUSTFLAGS=-C\u{1f}target-cpu=x86-64-v2",
		"cargo build --release --frozen --target x86_64-unknown-linux-gnu --package backend --bin \
		 server --config /build/dependencies/.leptoskit-vendor.toml --features postgres,tls \
		 --no-default-features",
	] {
		assert!(
			joined.contains(expected),
			"{expected:?} missing from {joined}"
		);
	}

	let writable: Vec<_> = run
		.mounts
		.iter()
		.filter(|(_, _, read_only)| !read_only)
		.map(|(_, container, _)| *container)
		.collect();
	assert_eq!(writable, [WORK_MOUNT]);
	assert!(
		run.env.iter().all(|(name, _)| {
			!name.starts_with("HOME") && *name != "PATH" && *name != "SSH_AUTH_SOCK"
		}),
		"only builder variables are passed: {:?}",
		run.env
	);
	assert!(!joined.contains("--user"), "rootless podman needs no user");
}

#[test]
fn only_vendoring_reaches_the_network_and_it_cannot_write_the_source() {
	let recipe = recipe();
	let directories = tempfile::tempdir().unwrap();
	let [source, dependencies, work] = ["source", "dependencies", "work"].map(|name| {
		let path = directories.path().join(name);
		fs::create_dir(&path).unwrap();
		path
	});
	let run = builder(&recipe, Engine::Docker)
		.vendor_run(&source, &dependencies, &work)
		.unwrap();
	let joined = arguments(&run).join(" ");
	assert!(!joined.contains("--network"), "{joined}");
	assert!(joined.contains("target=/build/source,readonly=true"));
	assert!(joined.contains("cargo vendor --locked --versioned-dirs /build/dependencies"));
	#[cfg(unix)]
	assert!(
		joined.contains("--user "),
		"docker builds as the caller: {joined}"
	);
	#[cfg(not(unix))]
	assert!(
		!joined.contains("--user "),
		"docker uses its default user without Unix ownership: {joined}"
	);

	let comma = directories.path().join("a,b");
	fs::create_dir(&comma).unwrap();
	assert!(
		builder(&recipe, Engine::Docker)
			.vendor_run(&comma, &dependencies, &work)
			.is_err()
	);
}

#[test]
fn comparison_reports_changed_missing_and_extra_files() {
	let directories = tempfile::tempdir().unwrap();
	let expected = directories.path().join("expected");
	let actual = directories.path().join("actual");

	for root in [&expected, &actual] {
		fs::create_dir_all(root.join("src")).unwrap();
		fs::write(root.join("src/main.rs"), "fn main() {}\n").unwrap();
	}

	compare(&expected, &actual, "source/").unwrap();
	fs::write(actual.join("src/main.rs"), "fn main() { evil() }\n").unwrap();
	let error = compare(&expected, &actual, "source/").unwrap_err();
	assert_eq!(
		error.to_string(),
		"source/: src/main.rs is missing or differs"
	);
	fs::write(actual.join("src/main.rs"), "fn main() {}\n").unwrap();
	fs::write(actual.join("build.rs"), "fn main() {}\n").unwrap();
	let error = compare(&expected, &actual, "source/").unwrap_err();
	assert_eq!(error.to_string(), "source/: build.rs is extra");
	fs::remove_file(actual.join("build.rs")).unwrap();
	fs::remove_file(actual.join("src/main.rs")).unwrap();
	assert!(compare(&expected, &actual, "source/").is_err());
}

#[test]
fn provenance_names_its_inputs_and_is_the_same_on_every_machine() {
	let recipe = recipe();
	let statement = provenance(
		&recipe,
		"1".repeat(64).as_str(),
		&"2".repeat(64),
		&"3".repeat(64),
		"rustc 1.99.1\n",
	);
	assert_eq!(
		statement,
		provenance(
			&recipe,
			&"1".repeat(64),
			&"2".repeat(64),
			&"3".repeat(64),
			"rustc 1.99.1\n"
		)
	);
	assert_eq!(statement["_type"], "https://in-toto.io/Statement/v1");
	assert_eq!(statement["predicateType"], "https://slsa.dev/provenance/v1");
	assert_eq!(statement["subject"][0]["digest"]["sha256"], "1".repeat(64));
	assert_eq!(statement["subject"][1]["name"], "inventory.json");
	let definition = &statement["predicate"]["buildDefinition"];
	assert_eq!(
		definition["externalParameters"]["recipe"]["binary"],
		"server"
	);
	assert_eq!(
		definition["resolvedDependencies"][0]["digest"]["gitCommit"],
		recipe.commit
	);
	assert_eq!(
		definition["resolvedDependencies"][1]["digest"]["sha256"],
		"b".repeat(64)
	);
	let text = serde_json::to_string(&statement).unwrap();
	assert!(!text.contains("startedOn") && !text.contains("finishedOn"));
}

#[test]
fn build_refuses_an_output_directory_that_already_exists() {
	let directory = tempfile::tempdir().unwrap();
	let recipe_path = directory.path().join("build.json");
	fs::write(&recipe_path, serde_json::to_vec(&recipe()).unwrap()).unwrap();
	let out = directory.path().join("out");
	fs::create_dir(&out).unwrap();

	let error = build(&BuildArgs {
		recipe: recipe_path,
		checkout: Some(directory.path().to_owned()),
		secrets: Vec::new(),
		out,
		engine: Some(Engine::Docker),
	})
	.unwrap_err();

	assert!(
		error
			.to_string()
			.contains("could not create a new directory"),
		"{error}"
	);
}

#[test]
fn build_rejects_a_secret_assignment_before_running_anything() {
	let directory = tempfile::tempdir().unwrap();
	let recipe_path = directory.path().join("build.json");
	fs::write(&recipe_path, serde_json::to_vec(&recipe()).unwrap()).unwrap();

	let error = build(&BuildArgs {
		recipe: recipe_path,
		checkout: None,
		secrets: vec!["API_KEY=value-that-must-not-print".to_owned()],
		out: directory.path().join("out"),
		engine: None,
	})
	.unwrap_err();
	let text = format!("{error:#}");

	assert!(
		text.contains("never secret values or assignments"),
		"{text}"
	);
	assert!(!text.contains("value-that-must-not-print"), "{text}");
}

/// A harmless `docker` and `podman` that log their arguments and do what a
/// scenario's environment says. The child test below reads that environment;
/// the parent only ever sets it on the subprocess, never on itself.
#[cfg(unix)]
mod fake_engine {
	use std::os::unix::fs::PermissionsExt as _;
	use std::path::PathBuf;
	use std::process::Command;

	use super::*;

	const HELPER: &str = "verify::build::tests::fake_engine::helper";
	const SYSTEM_PATH: &str = "/usr/bin:/bin:/usr/sbin:/sbin";

	fn script() -> &'static str {
		r#"#!/bin/sh
set -eu
name=${0##*/}
{
	printf '%s' "$name"
	for argument in "$@"; do printf ' %s' "$argument"; done
	printf '\n'
} >> "$LEPTOSKIT_TEST_ENGINE_LOG"

case "${1:-}" in
	info)
		case "$name" in
			docker) exit "${LEPTOSKIT_TEST_DOCKER_INFO:-0}" ;;
			podman) exit "${LEPTOSKIT_TEST_PODMAN_INFO:-0}" ;;
		esac
		exit 0
		;;
	pull) exit "${LEPTOSKIT_TEST_PULL_STATUS:-0}" ;;
	run) ;;
	*) exit 0 ;;
esac

work=
dependencies=
previous=
for argument in "$@"; do
	if [ "$previous" = "--mount" ]; then
		case "$argument" in
			*target=/build/work*)
				work=$(printf '%s' "$argument" | sed -n 's/.*source=\([^,]*\).*/\1/p')
				;;
			*target=/build/dependencies*)
				dependencies=$(printf '%s' "$argument" | sed -n 's/.*source=\([^,]*\).*/\1/p')
				;;
		esac
	fi
	previous=$argument
done

mode=
for argument in "$@"; do
	case "$argument" in
		vendor) mode=vendor ;;
		build) mode=build ;;
		rustc) mode=rustc ;;
		cargo) [ -n "$mode" ] || mode=cargo ;;
	esac
done

case "$mode" in
	vendor)
		status=${LEPTOSKIT_TEST_VENDOR_STATUS:-0}
		if [ "$status" -ne 0 ]; then
			printf '%s' "${LEPTOSKIT_TEST_FAIL_STDERR:-}" >&2
			exit "$status"
		fi
		printf '%s' "${LEPTOSKIT_TEST_VENDOR_STDOUT:-}"
		;;
	rustc|cargo)
		status=${LEPTOSKIT_TEST_TOOLCHAIN_STATUS:-0}
		if [ "$status" -ne 0 ]; then
			printf '%s' "${LEPTOSKIT_TEST_FAIL_STDERR:-}" >&2
			exit "$status"
		fi
		if [ "$mode" = rustc ]; then
			printf '%s' "${LEPTOSKIT_TEST_RUSTC_STDOUT:-}"
		else
			printf '%s' "${LEPTOSKIT_TEST_CARGO_STDOUT:-}"
		fi
		;;
	build)
		status=${LEPTOSKIT_TEST_BUILD_STATUS:-0}
		if [ "$status" -ne 0 ]; then
			printf '%s' "${LEPTOSKIT_TEST_FAIL_STDERR:-}" >&2
			exit "$status"
		fi
		if [ -n "${LEPTOSKIT_TEST_OUTPUT_RELATIVE:-}" ]; then
			output="$work/$LEPTOSKIT_TEST_OUTPUT_RELATIVE"
			mkdir -p "$(dirname "$output")"
			printf '%s' "${LEPTOSKIT_TEST_OUTPUT_CONTENT:-}" > "$output"
		fi
		;;
esac
"#
	}

	pub(super) struct Fixture {
		directory: tempfile::TempDir,
		bin: PathBuf,
		log: PathBuf,
	}

	impl Fixture {
		pub(super) fn new() -> Self {
			let directory = tempfile::tempdir().unwrap();
			let bin = directory.path().join("bin");
			fs::create_dir(&bin).unwrap();

			for program in ["docker", "podman"] {
				let path = bin.join(program);
				fs::write(&path, script()).unwrap();
				fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
			}

			let log = directory.path().join("engine.log");
			fs::write(&log, "").unwrap();
			Self {
				directory,
				bin,
				log,
			}
		}

		/// Runs one scenario in a child test process whose `PATH` finds the
		/// fixture first, so no process-global environment changes.
		pub(super) fn run(&self, scenario: &str, environment: &[(&str, &str)]) -> Vec<String> {
			let mut command = Command::new(std::env::current_exe().unwrap());
			command
				.args(["--exact", HELPER, "--nocapture", "--test-threads=1"])
				.env_clear()
				.env("PATH", format!("{}:{SYSTEM_PATH}", self.bin.display()))
				.env("TMPDIR", self.directory.path())
				.env("LEPTOSKIT_TEST_SCENARIO", scenario)
				.env("LEPTOSKIT_TEST_ENGINE_LOG", &self.log)
				.current_dir(self.directory.path());

			for (name, value) in environment {
				command.env(name, value);
			}

			if let Some(profile) = std::env::var_os("LLVM_PROFILE_FILE") {
				command.env("LLVM_PROFILE_FILE", profile);
			}

			let output = command.output().unwrap();
			assert!(
				output.status.success(),
				"scenario {scenario} failed\nstdout:\n{}\nstderr:\n{}",
				String::from_utf8_lossy(&output.stdout),
				String::from_utf8_lossy(&output.stderr)
			);
			fs::read_to_string(&self.log)
				.unwrap()
				.lines()
				.map(str::to_owned)
				.collect()
		}
	}

	struct Work {
		_directory: tempfile::TempDir,
		source: PathBuf,
		dependencies: PathBuf,
		scratch: PathBuf,
	}

	impl Work {
		fn new() -> Self {
			let directory = tempfile::tempdir().unwrap();
			let [source, dependencies, scratch] = ["source", "dependencies", "work"].map(|name| {
				let path = directory.path().join(name);
				fs::create_dir(&path).unwrap();
				path
			});
			Self {
				_directory: directory,
				source,
				dependencies,
				scratch,
			}
		}
	}

	struct Offline {
		_directory: tempfile::TempDir,
		bundle: PathBuf,
		manifest: PathBuf,
		digest: String,
	}

	fn offline_bundle(artifact: &str) -> Offline {
		let directory = tempfile::tempdir().unwrap();
		let bundle = directory.path().join("bundle");

		for path in ["source/src", "dependencies", "infra", "artifacts"] {
			fs::create_dir_all(bundle.join(path)).unwrap();
		}

		for (path, contents) in [
			(
				"source/Cargo.toml",
				"[package]\nname = \"backend\"\nversion = \"0.1.0\"\n",
			),
			("source/Cargo.lock", "version = 4\n"),
			("source/src/main.rs", "fn main() {}\n"),
			("dependencies/.leptoskit-vendor.toml", ""),
			("artifacts/backend", artifact),
			("secrets.json", "[]"),
		] {
			fs::write(bundle.join(path), contents).unwrap();
		}

		fs::write(
			bundle.join("build.json"),
			serde_json::to_vec_pretty(&recipe()).unwrap(),
		)
		.unwrap();
		let manifest = directory.path().join("inventory.json");
		let digest = super::super::super::record(&bundle, &manifest).unwrap();
		Offline {
			_directory: directory,
			bundle,
			manifest,
			digest,
		}
	}

	fn log() -> Vec<String> {
		fs::read_to_string(std::env::var("LEPTOSKIT_TEST_ENGINE_LOG").unwrap())
			.unwrap()
			.lines()
			.map(str::to_owned)
			.collect()
	}

	fn expect() -> String {
		std::env::var("LEPTOSKIT_TEST_EXPECT").unwrap()
	}

	fn recipe_without_features() -> BuildRecipe {
		let mut recipe = recipe();
		recipe.features.clear();
		recipe.no_default_features = false;
		recipe
	}

	fn builder_pull() {
		let recipe = recipe();
		let builder = Builder::new(Engine::Docker, &recipe, false).unwrap();
		assert_eq!(builder.platform, "linux/amd64");
		let lines = log();
		assert_eq!(lines.len(), 1);
		assert_eq!(
			lines[0],
			format!(
				"docker pull --quiet --platform linux/amd64 {}",
				recipe.builder_image
			)
		);

		Builder::new(Engine::Podman, &recipe, true).unwrap();
		assert_eq!(log().len(), 1, "an offline build must not pull");
	}

	fn builder_pull_failure() {
		let error = Builder::new(Engine::Docker, &recipe(), false)
			.err()
			.expect("the pull should fail");
		assert!(
			error
				.to_string()
				.contains("could not pull the builder image"),
			"{error}"
		);
		assert!(log()[0].contains(" pull "), "{:?}", log());
	}

	fn engine_detect() {
		let detected = Engine::detect(None);

		match expect().as_str() {
			"docker" => assert_eq!(detected.unwrap(), Engine::Docker),
			"podman" => assert_eq!(detected.unwrap(), Engine::Podman),
			"none" => {
				let error = detected.unwrap_err().to_string();
				assert!(error.contains("running docker or podman"), "{error}");
			}
			other => panic!("unknown engine {other}"),
		}
	}

	fn execute() {
		let work = Work::new();
		let recipe = recipe();
		let builder = builder(&recipe, Engine::Docker);

		match expect().as_str() {
			"success" => {
				let run = builder
					.run(&work.scratch, false, vec!["cargo".into(), "-V".into()])
					.unwrap();
				let stdout = builder.execute(&run, true).unwrap();
				assert_eq!(stdout, b"cargo 1.99.1\n");
				assert!(log()[0].contains("--network none"));
			}
			"failure" => {
				let run = builder
					.run(&work.scratch, false, vec!["cargo".into(), "build".into()])
					.unwrap();
				let error = builder.execute(&run, true).unwrap_err();
				assert!(
					error
						.to_string()
						.contains("`cargo build` failed in the builder container"),
					"{error}"
				);
			}
			other => panic!("unknown execute case {other}"),
		}
	}

	fn vendor() {
		let work = Work::new();
		let recipe = recipe();
		let builder = builder(&recipe, Engine::Docker);

		match expect().as_str() {
			"success" => {
				builder
					.vendor(&work.source, &work.dependencies, &work.scratch)
					.unwrap();
				assert_eq!(
					fs::read(work.dependencies.join(VENDOR_CONFIG)).unwrap(),
					b"[source.crates-io]\nreplace-with = \"vendored-sources\"\n"
				);

				let log = log();
				assert_eq!(log.len(), 1);
				assert!(
					log[0].contains("cargo vendor --locked --versioned-dirs /build/dependencies")
				);
				assert!(!log[0].contains("--network none"), "{}", log[0]);
				assert!(log[0].contains("target=/build/source,readonly=true"));
				assert!(!log[0].contains("target=/build/dependencies,readonly=true"));
			}
			"failure" => {
				let error = builder
					.vendor(&work.source, &work.dependencies, &work.scratch)
					.unwrap_err();
				assert!(
					error.to_string().contains(
						"`cargo vendor --locked --versioned-dirs /build/dependencies` failed in \
						 the builder container"
					),
					"{error}"
				);
				assert!(!work.dependencies.join(VENDOR_CONFIG).exists());
			}
			other => panic!("unknown vendor case {other}"),
		}
	}

	fn toolchain() {
		let work = Work::new();
		let recipe = recipe();
		let builder = builder(&recipe, Engine::Docker);

		match expect().as_str() {
			"success" => {
				let report = builder.toolchain(&work.source, &work.scratch).unwrap();
				assert_eq!(report, "rustc 1.99.1\ncargo 1.99.1\n");

				let log = log();
				assert_eq!(log.len(), 2);
				assert!(log[0].contains("rustc -vV"), "{}", log[0]);
				assert!(log[1].contains("cargo -V"), "{}", log[1]);

				for run in log {
					assert!(run.contains("--network none"), "{run}");
					assert!(run.contains("target=/build/source,readonly=true"), "{run}");
				}
			}
			"failure" => {
				let error = builder.toolchain(&work.source, &work.scratch).unwrap_err();
				assert!(
					error
						.to_string()
						.contains("`rustc -vV` failed in the builder container"),
					"{error}"
				);
			}
			other => panic!("unknown toolchain case {other}"),
		}
	}

	fn compile() {
		let work = Work::new();
		let recipe = recipe_without_features();
		let builder = builder(&recipe, Engine::Docker);

		match expect().as_str() {
			"success" => {
				let executable = builder
					.compile(&work.source, &work.dependencies, &work.scratch)
					.unwrap();
				assert_eq!(
					executable,
					work.scratch
						.join("target/x86_64-unknown-linux-gnu/release/server")
				);
				assert_eq!(fs::read(&executable).unwrap(), b"compiled server\n");

				let log = log();
				assert_eq!(log.len(), 1);
				for expected in [
					"cargo build --release --frozen --target x86_64-unknown-linux-gnu --package \
					 backend --bin server --config /build/dependencies/.leptoskit-vendor.toml",
					"--network none",
					"target=/build/source,readonly=true",
					"target=/build/dependencies,readonly=true",
					"--env CARGO_ENCODED_RUSTFLAGS=-C\u{1f}target-cpu=x86-64-v2",
				] {
					assert!(
						log[0].contains(expected),
						"{expected:?} missing from {}",
						log[0]
					);
				}
				assert!(!log[0].contains("--features"), "{}", log[0]);
				assert!(!log[0].contains("--no-default-features"), "{}", log[0]);
			}
			"missing" => {
				let error = builder
					.compile(&work.source, &work.dependencies, &work.scratch)
					.unwrap_err();
				assert!(
					error
						.to_string()
						.contains("the build finished without producing server"),
					"{error}"
				);
			}
			"failure" => {
				let error = builder
					.compile(&work.source, &work.dependencies, &work.scratch)
					.unwrap_err();
				let text = error.to_string();
				assert!(text.contains("`cargo build"), "{text}");
				assert!(text.contains("failed in the builder container"), "{text}");
			}
			other => panic!("unknown compile case {other}"),
		}
	}

	fn reproduce_case() {
		let case = expect();
		let artifact = if case == "mismatch" {
			"different bytes\n"
		} else {
			"reproduced bytes\n"
		};
		let offline = offline_bundle(artifact);
		let args = ReproduceArgs {
			bundle: offline.bundle,
			manifest: offline.manifest,
			sha256: offline.digest,
			checkout: None,
			offline: true,
			engine: Some(Engine::Docker),
		};

		match case.as_str() {
			"match" => {
				reproduce(&args).unwrap();
				let log = log();
				assert_eq!(log.len(), 1);
				assert!(log[0].contains("cargo build"), "{}", log[0]);
				assert!(!log[0].contains(" pull "), "{}", log[0]);
				assert!(log[0].contains("target=/build/source,readonly=true"));
				assert!(log[0].contains("target=/build/dependencies,readonly=true"));
			}
			"mismatch" => {
				let error = reproduce(&args).unwrap_err().to_string();
				assert!(error.contains("was not reproduced"), "{error}");
				assert_eq!(log().len(), 1);
			}
			other => panic!("unknown reproduce case {other}"),
		}
	}

	#[test]
	fn helper() {
		let Ok(scenario) = std::env::var("LEPTOSKIT_TEST_SCENARIO") else {
			return;
		};

		match scenario.as_str() {
			"builder-pull" => builder_pull(),
			"builder-pull-failure" => builder_pull_failure(),
			"engine-detect" => engine_detect(),
			"execute" => execute(),
			"vendor" => vendor(),
			"toolchain" => toolchain(),
			"compile" => compile(),
			"reproduce" => reproduce_case(),
			other => panic!("unknown scenario {other}"),
		}
	}
}

#[cfg(unix)]
#[test]
fn a_builder_pulls_its_pinned_image_only_when_online() {
	let fixture = fake_engine::Fixture::new();
	assert_eq!(fixture.run("builder-pull", &[]).len(), 1);
}

#[cfg(unix)]
#[test]
fn a_failed_pull_names_the_builder_image() {
	let fixture = fake_engine::Fixture::new();
	fixture.run(
		"builder-pull-failure",
		&[("LEPTOSKIT_TEST_PULL_STATUS", "1")],
	);
}

#[cfg(unix)]
#[test]
fn engine_detection_prefers_docker_then_podman_and_reports_none() {
	let fixture = fake_engine::Fixture::new();

	for (expected, docker, podman) in [
		("docker", "0", "0"),
		("podman", "1", "0"),
		("none", "1", "1"),
	] {
		fixture.run(
			"engine-detect",
			&[
				("LEPTOSKIT_TEST_EXPECT", expected),
				("LEPTOSKIT_TEST_DOCKER_INFO", docker),
				("LEPTOSKIT_TEST_PODMAN_INFO", podman),
			],
		);
	}
}

#[cfg(unix)]
#[test]
fn execute_captures_a_successful_run_and_reports_a_failed_one() {
	let fixture = fake_engine::Fixture::new();
	fixture.run(
		"execute",
		&[
			("LEPTOSKIT_TEST_EXPECT", "success"),
			("LEPTOSKIT_TEST_CARGO_STDOUT", "cargo 1.99.1\n"),
		],
	);
	fixture.run(
		"execute",
		&[
			("LEPTOSKIT_TEST_EXPECT", "failure"),
			("LEPTOSKIT_TEST_BUILD_STATUS", "1"),
			("LEPTOSKIT_TEST_FAIL_STDERR", "compiler exploded\n"),
		],
	);
}

#[cfg(unix)]
#[test]
fn vendoring_writes_the_configuration_the_builder_printed() {
	let fixture = fake_engine::Fixture::new();
	let output = "[source.crates-io]\nreplace-with = \"vendored-sources\"\n";
	fixture.run(
		"vendor",
		&[
			("LEPTOSKIT_TEST_EXPECT", "success"),
			("LEPTOSKIT_TEST_VENDOR_STDOUT", output),
		],
	);
	fixture.run(
		"vendor",
		&[
			("LEPTOSKIT_TEST_EXPECT", "failure"),
			("LEPTOSKIT_TEST_VENDOR_STATUS", "1"),
			("LEPTOSKIT_TEST_FAIL_STDERR", "vendoring failed\n"),
		],
	);
}

#[cfg(unix)]
#[test]
fn the_toolchain_report_is_both_versions_and_a_failure_stops_it() {
	let fixture = fake_engine::Fixture::new();
	fixture.run(
		"toolchain",
		&[
			("LEPTOSKIT_TEST_EXPECT", "success"),
			("LEPTOSKIT_TEST_RUSTC_STDOUT", "rustc 1.99.1\n"),
			("LEPTOSKIT_TEST_CARGO_STDOUT", "cargo 1.99.1\n"),
		],
	);
	fixture.run(
		"toolchain",
		&[
			("LEPTOSKIT_TEST_EXPECT", "failure"),
			("LEPTOSKIT_TEST_TOOLCHAIN_STATUS", "1"),
			("LEPTOSKIT_TEST_FAIL_STDERR", "rustc is broken\n"),
		],
	);
}

#[cfg(unix)]
#[test]
fn compiling_returns_the_executable_or_says_what_is_missing() {
	let fixture = fake_engine::Fixture::new();
	fixture.run(
		"compile",
		&[
			("LEPTOSKIT_TEST_EXPECT", "success"),
			(
				"LEPTOSKIT_TEST_OUTPUT_RELATIVE",
				"target/x86_64-unknown-linux-gnu/release/server",
			),
			("LEPTOSKIT_TEST_OUTPUT_CONTENT", "compiled server\n"),
		],
	);
	fixture.run("compile", &[("LEPTOSKIT_TEST_EXPECT", "missing")]);
	fixture.run(
		"compile",
		&[
			("LEPTOSKIT_TEST_EXPECT", "failure"),
			("LEPTOSKIT_TEST_BUILD_STATUS", "1"),
			("LEPTOSKIT_TEST_FAIL_STDERR", "compiler exploded\n"),
		],
	);
}

#[cfg(unix)]
#[test]
fn offline_reproduce_compares_the_rebuilt_executable() {
	let output = [
		(
			"LEPTOSKIT_TEST_OUTPUT_RELATIVE",
			"target/x86_64-unknown-linux-gnu/release/server",
		),
		("LEPTOSKIT_TEST_OUTPUT_CONTENT", "reproduced bytes\n"),
	];

	for expected in ["match", "mismatch"] {
		let fixture = fake_engine::Fixture::new();
		let mut environment = vec![("LEPTOSKIT_TEST_EXPECT", expected)];
		environment.extend(output);
		fixture.run("reproduce", &environment);
	}
}
