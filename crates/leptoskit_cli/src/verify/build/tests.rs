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
