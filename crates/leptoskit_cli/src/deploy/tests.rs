//! `leptoskit generate deploy` against projects written by hand: every
//! target with no database, SQLite, and Postgres, pinned file by file.

use std::fmt::Write as _;
use std::path::Path;

use leptoskit_config::LeptoskitConfig;

use super::Project;
use super::Target;
use super::generate;
use super::generate_with;

const SERVER: &str = r"
use leptoskit::server::Readiness;

fn main() {
	let readiness = Readiness::new();
}
";

#[derive(Clone, Copy, Debug)]
enum Db {
	None,
	Sqlite,
	Postgres,
}

impl Db {
	fn name(self) -> &'static str {
		match self {
			Self::None => "no_db",
			Self::Sqlite => "sqlite",
			Self::Postgres => "postgres",
		}
	}
}

fn write(root: &Path, file: &str, contents: &str) {
	let path = root.join(file);
	std::fs::create_dir_all(path.parent().unwrap()).unwrap();
	std::fs::write(path, contents).unwrap();
}

/// A workspace like the scaffold's: the server package at the root, the
/// client in a member. The Postgres project also signs people in.
fn project(root: &Path, db: Db) -> LeptoskitConfig {
	let mut config = String::from(
		"schema_version = 0\n\n[app]\nname = \"notes\"\nversion = \"0.1.0\"\nrender = \
		 \"ssr\"\n\n[serve]\naddress = \"127.0.0.1\"\nport = 3000\n\n[build]\nclient_package = \
		 \"notes_app\"\n",
	);

	match db {
		Db::None => {}
		Db::Sqlite => config.push_str("\n[db]\ndialect = \"sqlite\"\n"),
		Db::Postgres => {
			config.push_str(
				"\n[db]\ndialect = \"postgres\"\n\n[auth]\nkeys_env = \
				 \"AUTH_ENCRYPTION_KEYS\"\n\n[auth.mail]\nfrom = \"Notes \
				 <hello@notes.example>\"\nsmtp_url_env = \"SMTP_URL\"\n",
			);
		}
	}

	write(root, "leptoskit.toml", &config);
	write(
		root,
		"Cargo.toml",
		"[package]\nname = \"notes\"\nversion = \"0.1.0\"\n\n[workspace]\nmembers = \
		 [\"notes_app\"]\n",
	);
	write(root, "src/main.rs", SERVER);
	write(
		root,
		"notes_app/Cargo.toml",
		"[package]\nname = \"notes_app\"\nversion = \"0.1.0\"\n\n[lib]\ncrate-type = [\"rlib\", \
		 \"cdylib\"]\n",
	);
	LeptoskitConfig::from_toml(&config, root).expect("config")
}

fn with_version_masked() -> insta::Settings {
	let mut settings = insta::Settings::clone_current();
	settings.add_filter(&regex_escape(env!("CARGO_PKG_VERSION")), "[version]");
	settings
}

fn regex_escape(text: &str) -> String {
	text.chars()
		.map(|character| {
			if character.is_ascii_alphanumeric() {
				character.to_string()
			} else {
				format!("\\{character}")
			}
		})
		.collect()
}

/// Every target, with every database: the output, then each file.
#[test]
fn every_target_and_database_is_pinned() {
	let _version = with_version_masked().bind_to_scope();
	for target in [
		Target::Docker,
		Target::Compose,
		Target::Fly,
		Target::Kubernetes,
		Target::Systemd,
	] {
		for db in [Db::None, Db::Sqlite, Db::Postgres] {
			let root = tempfile::tempdir().unwrap();
			let config = project(root.path(), db);
			let report = generate(root.path(), &config, target, false).expect("generate");
			let mut pinned = format!("$ leptoskit generate deploy --target {target}\n{report}");

			for (file, _) in target.files(&Project::read(root.path(), &config).unwrap()) {
				let contents = std::fs::read_to_string(root.path().join(&file)).unwrap();

				write!(pinned, "\n===== {file} =====\n{contents}").unwrap();
			}

			insta::assert_snapshot!(format!("{target}_{}", db.name()), pinned);
		}
	}
}

/// Kubernetes with each role its own Deployment, on Postgres with a relay
/// and a replica: the output, then the manifest.
#[test]
fn split_roles_are_pinned() {
	let _version = with_version_masked().bind_to_scope();
	let root = tempfile::tempdir().unwrap();
	project(root.path(), Db::Postgres);
	let toml = std::fs::read_to_string(root.path().join("leptoskit.toml"))
		.unwrap()
		.replace(
			"dialect = \"postgres\"\n",
			"dialect = \"postgres\"\nreplica_url_envs = [\"DATABASE_REPLICA_URL\"]\n",
		)
		+ "\n[rpc.live]\nrelay_url_env = \"NATS_URL\"\n";
	write(root.path(), "leptoskit.toml", &toml);
	let config = LeptoskitConfig::from_toml(&toml, root.path()).unwrap();
	let report = generate_with(root.path(), &config, Target::Kubernetes, false, true).unwrap();
	let manifest = std::fs::read_to_string(root.path().join("kubernetes.yaml")).unwrap();
	insta::assert_snapshot!(
		"kubernetes_split_roles",
		format!(
			"$ leptoskit generate deploy --target kubernetes --split-roles\n{report}\n===== \
			 kubernetes.yaml =====\n{manifest}"
		)
	);
}

#[test]
fn split_roles_need_kubernetes_and_postgres() {
	let root = tempfile::tempdir().unwrap();
	let config = project(root.path(), Db::Postgres);
	let error = generate_with(root.path(), &config, Target::Fly, false, true).unwrap_err();
	assert!(error.to_string().contains("--target kubernetes"), "{error}");

	let root = tempfile::tempdir().unwrap();
	let config = project(root.path(), Db::Sqlite);
	let error = generate_with(root.path(), &config, Target::Kubernetes, false, true).unwrap_err();
	assert!(error.to_string().contains("needs Postgres"), "{error}");
}

#[test]
fn running_again_changes_nothing_and_says_so() {
	let root = tempfile::tempdir().unwrap();
	let config = project(root.path(), Db::Sqlite);
	generate(root.path(), &config, Target::Fly, false).unwrap();
	let fly = std::fs::read_to_string(root.path().join("fly.toml")).unwrap();

	let again = generate(root.path(), &config, Target::Fly, false).unwrap();
	assert!(
		again.changed.is_empty() && again.kept.is_empty(),
		"{again:?}"
	);
	assert!(
		again
			.to_string()
			.starts_with("nothing changed: the fly files are already in place\n\nnext:\n"),
		"{again}"
	);
	assert_eq!(
		std::fs::read_to_string(root.path().join("fly.toml")).unwrap(),
		fly
	);

	// Another target shares the Dockerfile and adds only its own file.
	let compose = generate(root.path(), &config, Target::Compose, false).unwrap();
	assert_eq!(compose.changed, ["created compose.yaml"]);
}

#[test]
fn an_edited_file_is_kept_unless_forced() {
	let root = tempfile::tempdir().unwrap();
	let config = project(root.path(), Db::None);
	generate(root.path(), &config, Target::Docker, false).unwrap();
	let dockerfile = root.path().join("Dockerfile");
	let original = std::fs::read_to_string(&dockerfile).unwrap();
	std::fs::write(&dockerfile, "FROM scratch\n").unwrap();

	let kept = generate(root.path(), &config, Target::Docker, false).unwrap();
	assert_eq!(
		kept.kept,
		["kept Dockerfile: it differs from the template; --force replaces it"]
	);
	assert!(kept.to_string().contains("nothing changed"), "{kept}");
	assert_eq!(
		std::fs::read_to_string(&dockerfile).unwrap(),
		"FROM scratch\n"
	);

	let forced = generate(root.path(), &config, Target::Docker, true).unwrap();
	assert_eq!(forced.changed, ["replaced Dockerfile"]);
	assert_eq!(forced.kept, [] as [String; 0]);
	assert_eq!(std::fs::read_to_string(&dockerfile).unwrap(), original);
}

#[test]
fn a_directory_in_the_way_is_an_error() {
	let root = tempfile::tempdir().unwrap();
	let config = project(root.path(), Db::None);
	std::fs::create_dir(root.path().join("Dockerfile")).unwrap();
	let error = generate(root.path(), &config, Target::Docker, true).unwrap_err();
	assert_eq!(error.to_string(), "Dockerfile isn't an ordinary file");
}

#[test]
fn a_server_without_probes_is_told_how_to_add_them() {
	let root = tempfile::tempdir().unwrap();
	let config = project(root.path(), Db::None);
	write(root.path(), "src/main.rs", "fn main() {}\n");
	let report = generate(root.path(), &config, Target::Docker, false).unwrap();
	assert!(
		report.next[0].starts_with("The server doesn't answer /healthz and /readyz yet"),
		"{report}"
	);
}

#[test]
fn the_project_is_read_from_its_manifests() {
	let root = tempfile::tempdir().unwrap();
	let mut config = project(root.path(), Db::None);
	write(
		root.path(),
		"rust-toolchain.toml",
		"[toolchain]\nchannel = \"1.96.0\"\n",
	);
	write(
		root.path(),
		"notes_app/Cargo.toml",
		"[package]\nname = \"notes_app\"\n\n[lib]\nname = \"notes_client\"\n",
	);
	config.app.platforms = vec![leptoskit_config::Platform::Ios];
	config.serve.port = 8080;
	let project = Project::read(root.path(), &config).unwrap();
	assert_eq!(project.binary, "notes");

	assert_eq!(project.output_name, "notes_client");
	assert_eq!(project.port, 8080);
	assert_eq!(project.platform, leptoskit_config::Platform::Server);
	assert_eq!(
		project.toolchain_file.as_deref(),
		Some("rust-toolchain.toml")
	);
	assert_eq!(project.rust_version, "1.96.0");
	assert!(project.has_probes);

	// A server-only build skips the wasm toolchain and binaryen, and the
	// toolchain file comes in before anything is built.
	let dockerfile = super::dockerfile(&project);
	assert!(!dockerfile.contains("wasm"), "{dockerfile}");
	assert!(dockerfile.contains("ARG RUST_VERSION=1.96.0\n"));
	assert!(dockerfile.contains("COPY rust-toolchain.toml ./\nCOPY . .\n"));
	assert!(dockerfile.contains("leptoskit build --release --platform server"));
	assert!(dockerfile.contains("EXPOSE 8080\n"));

	// Without a client package, the output name is the app's.
	config.build.client_package = None;
	assert_eq!(
		Project::read(root.path(), &config).unwrap().output_name,
		"notes"
	);
}

#[test]
fn toolchain_channels_pin_the_image_only_when_they_name_a_release() {
	let root = tempfile::tempdir().unwrap();
	let file = root.path().join("rust-toolchain");
	for (contents, expected) in [
		("1.96\n", Some("1.96")),
		("stable\n", None),
		("nightly-2026-10-01\n", None),
		("[toolchain]\nchannel = \"beta\"\n", None),
		("[toolchain]\nprofile = \"minimal\"\n", None),
		("1..2\n", None),
		("\n", None),
	] {
		std::fs::write(&file, contents).unwrap();
		assert_eq!(
			super::toolchain_channel(&file).as_deref(),
			expected,
			"{contents:?}"
		);
	}
	assert_eq!(super::toolchain_channel(&root.path().join("missing")), None);
}

#[test]
fn the_binary_is_the_one_named_after_the_package_or_the_only_one() {
	let root = tempfile::tempdir().unwrap();
	let dir = root.path();
	write(
		dir,
		"Cargo.toml",
		"[package]\nname = \"notes\"\n\n[[bin]]\nname = \"serve\"\npath = \"src/serve.rs\"\n",
	);
	write(dir, "src/serve.rs", "fn main() {}\n");
	let (binary, source) = super::binary(dir, "notes").unwrap();
	assert_eq!(binary, "serve");
	assert_eq!(source, Some(dir.join("src/serve.rs")));

	write(
		dir,
		"Cargo.toml",
		"[package]\nname = \"notes\"\n\n[[bin]]\nname = \"serve\"\n\n[[bin]]\nname = \"notes\"\n",
	);
	let (binary, source) = super::binary(dir, "notes").unwrap();
	assert_eq!(binary, "notes");
	assert_eq!(source, None, "src/bin/notes.rs doesn't exist");

	write(
		dir,
		"Cargo.toml",
		"[package]\nname = \"notes\"\n\n[[bin]]\nname = \"a\"\n\n[[bin]]\nname = \"b\"\n",
	);
	let error = super::binary(dir, "notes").unwrap_err().to_string();
	assert!(
		error.starts_with(
			"the server package `notes` has several binaries and none is called `notes`"
		),
		"{error}"
	);

	write(dir, "Cargo.toml", "[package]\nname = \"notes\"\n");
	std::fs::remove_file(dir.join("src/serve.rs")).unwrap();
	let error = super::binary(dir, "notes").unwrap_err().to_string();
	assert!(
		error.starts_with("the server package `notes` has no binary"),
		"{error}"
	);
}

#[test]
fn what_is_missing_is_named() {
	let root = tempfile::tempdir().unwrap();
	let mut config = project(root.path(), Db::None);

	config.app.render = leptoskit_config::RenderMode::Csr;
	let error = Project::read(root.path(), &config).unwrap_err().to_string();
	assert!(
		error.starts_with("app.render is `csr`, so the app is the static files in dist/"),
		"{error}"
	);

	config.app.render = leptoskit_config::RenderMode::Islands;
	config.build.client_package = Some("missing_app".into());
	let error = Project::read(root.path(), &config).unwrap_err().to_string();
	assert_eq!(
		error,
		"build.client_package `missing_app` in leptoskit.toml isn't a package in the workspace"
	);

	config.app.name = "missing".into();
	let error = Project::read(root.path(), &config).unwrap_err().to_string();
	assert!(
		error.starts_with(
			"app.name `missing` in leptoskit.toml must name the server package, but there's no \
			 package called `missing` in "
		),
		"{error}"
	);

	write(root.path(), "Cargo.toml", "[package\n");
	let error = Project::read(root.path(), &config).unwrap_err().to_string();
	assert!(error.starts_with("could not parse "), "{error}");

	std::fs::remove_file(root.path().join("Cargo.toml")).unwrap();
	let error = Project::read(root.path(), &config).unwrap_err().to_string();
	assert!(error.starts_with("there's no Cargo.toml at "), "{error}");
}
