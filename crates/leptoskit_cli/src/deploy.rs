//! `leptoskit generate deploy`: writes the files that run the app's server
//! somewhere else, for one target at a time.
//!
//! The files are the app's to own. A file that already matches is left
//! alone, a file that differs is kept unless `--force` replaces it, and
//! everything the files need comes from `leptoskit.toml` and the server's
//! `Cargo.toml`.

use std::fmt;
use std::fmt::Write as _;
use std::path::Path;
use std::path::PathBuf;

use anyhow::Context;
use anyhow::Result;
use leptoskit_config::DbDialect;
use leptoskit_config::LeptoskitConfig;
use leptoskit_config::Platform;

/// Where the server is deployed. Each target writes the container recipe
/// and, except `docker`, the platform's own file beside it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum Target {
	/// A `Dockerfile` and `.dockerignore`, for any container platform.
	Docker,
	/// `compose.yaml` beside the `Dockerfile`, with the database it needs.
	Compose,
	/// `fly.toml` beside the `Dockerfile`, for Fly.io.
	Fly,
	/// `kubernetes.yaml` beside the `Dockerfile`: a Deployment, Service,
	/// and `PodDisruptionBudget` for any Kubernetes cluster.
	Kubernetes,
	/// A systemd unit beside the `Dockerfile`, for a Linux host.
	Systemd,
}

impl Target {
	/// The name `--target` takes.
	#[must_use]
	pub fn as_str(self) -> &'static str {
		match self {
			Self::Docker => "docker",
			Self::Compose => "compose",
			Self::Fly => "fly",
			Self::Kubernetes => "kubernetes",
			Self::Systemd => "systemd",
		}
	}

	/// The files this target writes, relative to the project root.
	fn files(self, project: &Project) -> Vec<(String, String)> {
		let mut files = vec![
			("Dockerfile".to_owned(), dockerfile(project)),
			(".dockerignore".to_owned(), DOCKERIGNORE.to_owned()),
		];

		match self {
			Self::Docker => {}
			Self::Compose => files.push(("compose.yaml".to_owned(), compose(project))),
			Self::Fly => files.push(("fly.toml".to_owned(), fly(project))),
			Self::Kubernetes => files.push(("kubernetes.yaml".to_owned(), kubernetes(project))),
			Self::Systemd => files.push((format!("{}.service", project.app), systemd(project))),
		}

		files
	}

	/// The commands that take the files from here to a running server.
	fn next(self, project: &Project) -> Vec<String> {
		match self {
			Self::Docker => docker_next(project),
			Self::Compose => compose_next(project),
			Self::Fly => fly_next(project),
			Self::Kubernetes => kubernetes_next(project),
			Self::Systemd => systemd_next(project),
		}
	}
}

impl fmt::Display for Target {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		f.write_str(self.as_str())
	}
}

/// How long the platform waits after `SIGTERM` before it kills the server.
/// The server's own [`GracefulShutdown`](leptoskit_axum::GracefulShutdown)
/// defaults stop it within 25 seconds, so this leaves it a margin.
const STOP_TIMEOUT_SECONDS: u64 = 30;

/// Where the container keeps a SQLite database: the one writable path.
const DATA_DIR: &str = "/data";

/// Where the site goes: `leptoskit build`'s default output directory.
const DIST_DIR: &str = "dist";

/// The binaryen release whose `wasm-opt` shrinks the release client.
const BINARYEN_VERSION: &str = "132";

/// Everything the files need to know about the project.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Project {
	/// `app.name`: the server package.
	pub app: String,
	/// The server package's binary.
	pub binary: String,
	/// The port the server listens on, `serve.port`.
	pub port: u16,
	/// `LEPTOS_OUTPUT_NAME`: the client library's name, which names the
	/// files in `dist/pkg`.
	pub output_name: String,
	/// The platform `leptoskit build` builds: `web` when the app has a web
	/// client, otherwise `server`.
	pub platform: Platform,
	/// The Rust toolchain file at the root, if there is one.
	pub toolchain_file: Option<String>,
	/// The Rust image's version: the toolchain file's channel when it names
	/// a release, otherwise `1`, the latest stable.
	pub rust_version: String,
	/// The database, from `[db]`.
	pub database: Option<Database>,
	/// The environment variables that hold secrets, from `[auth]`.
	pub secrets: Vec<String>,
	/// Whether the server's source serves `/readyz`.
	pub has_probes: bool,
}

/// The database a project uses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Database {
	/// The dialect.
	pub dialect: DbDialect,
	/// The environment variable holding its URL, `db.url_env`.
	pub url_env: String,
}

impl Project {
	/// Reads the project at `root`.
	///
	/// # Errors
	///
	/// Returns an error when the app renders in the browser only, or the
	/// server package or its binary can't be found.
	pub fn read(root: &Path, config: &LeptoskitConfig) -> Result<Self> {
		anyhow::ensure!(
			config.app.render.renders_on_server(),
			"app.render is `{}`, so the app is the static files in {DIST_DIR}/ and has no server \
			 to deploy; upload {DIST_DIR}/ to a CDN or bucket instead, or set app.render to `ssr` \
			 or `islands`",
			config.app.render
		);
		let app = config.app.name.clone();
		let server = package_dir(root, &app)?.with_context(|| {
			format!(
				"app.name `{app}` in {} must name the server package, but there's no package \
				 called `{app}` in {}",
				leptoskit_config::CONFIG_FILE,
				root.join("Cargo.toml").display()
			)
		})?;
		let (binary, main) = binary(&server, &app)?;

		let output_name = match &config.build.client_package {
			Some(client) => {
				let dir = package_dir(root, client)?.with_context(|| {
					format!(
						"build.client_package `{client}` in {} isn't a package in the workspace",
						leptoskit_config::CONFIG_FILE
					)
				})?;
				lib_name(&dir.join("Cargo.toml"))?.unwrap_or_else(|| client.replace('-', "_"))
			}
			None => app.replace('-', "_"),
		};
		let platform = if config.app.platforms.contains(&Platform::Web) {
			Platform::Web
		} else {
			Platform::Server
		};

		let toolchain_file = ["rust-toolchain.toml", "rust-toolchain"]
			.into_iter()
			.find(|file| root.join(file).is_file())
			.map(str::to_owned);
		let rust_version = toolchain_file
			.as_deref()
			.and_then(|file| toolchain_channel(&root.join(file)))
			.unwrap_or_else(|| "1".to_owned());
		let database = config.db.as_ref().map(|db| {
			Database {
				dialect: db.dialect,
				url_env: db.url_env.clone(),
			}
		});
		let mut secrets = Vec::new();

		if let Some(auth) = &config.auth {
			secrets.push(auth.keys_env.clone());
			secrets.extend(
				[
					&auth.mail.smtp_url_env,
					&auth.mail.webhook_url_env,
					&auth.mail.webhook_secret_env,
				]
				.into_iter()
				.flatten()
				.cloned(),
			);
		}

		let has_probes = main
			.and_then(|main| std::fs::read_to_string(main).ok())
			.is_some_and(|source| source.contains("readyz") || source.contains("Readiness"));
		Ok(Self {
			app,
			binary,
			port: config.serve.port,
			output_name,
			platform,
			toolchain_file,
			rust_version,
			database,
			secrets,
			has_probes,
		})
	}

	/// The image name: the app's name in lowercase.
	fn image(&self) -> String {
		self.app.to_ascii_lowercase()
	}

	/// The secrets the server reads at run time: the database URL on
	/// Postgres, which holds a password, then `[auth]`'s.
	fn runtime_secrets(&self) -> Vec<String> {
		let mut secrets = Vec::new();

		if let Some(database) = &self.database
			&& database.dialect == DbDialect::Postgres
		{
			secrets.push(database.url_env.clone());
		}
		secrets.extend(self.secrets.iter().cloned());
		secrets
	}

	fn sqlite(&self) -> Option<&Database> {
		self.database
			.as_ref()
			.filter(|database| database.dialect == DbDialect::Sqlite)
	}

	fn postgres(&self) -> Option<&Database> {
		self.database
			.as_ref()
			.filter(|database| database.dialect == DbDialect::Postgres)
	}

	/// The SQLite URL inside the container.
	fn sqlite_url(&self) -> String {
		format!("sqlite://{DATA_DIR}/{}.sqlite3", self.app)
	}
}

/// Runs `leptoskit generate deploy` in the project around the current
/// directory, and prints what it did and what's left to do.
///
/// # Errors
///
/// Returns an error when there's no project, it has no server to deploy,
/// or a file can't be read or written.
pub fn run(target: Target, force: bool) -> Result<()> {
	let (config, root) = LeptoskitConfig::load()?;
	let report = generate(&root, &config, target, force)?;
	print!("{report}");
	Ok(())
}

/// What one run did, and what's left for the developer.
#[derive(Debug, Default)]
pub struct Report {
	/// The target the files are for.
	pub target: Option<Target>,
	/// One line per file created or replaced.
	pub changed: Vec<String>,
	/// One line per file left as it was because it differs.
	pub kept: Vec<String>,
	/// The steps the generator can't take itself.
	pub next: Vec<String>,
}

impl fmt::Display for Report {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		for line in self.changed.iter().chain(&self.kept) {
			writeln!(f, "{line}")?;
		}

		if self.changed.is_empty() {
			let target = self.target.map_or("deployment", Target::as_str);
			writeln!(
				f,
				"nothing changed: the {target} files are already in place"
			)?;
		}

		if !self.next.is_empty() {
			writeln!(f, "\nnext:")?;

			for (number, step) in self.next.iter().enumerate() {
				writeln!(f, "  {}. {step}", number + 1)?;
			}
		}

		Ok(())
	}
}

/// Writes `target`'s files into the project at `root`, and reports what
/// changed.
///
/// # Errors
///
/// As [`run`].
pub fn generate(
	root: &Path,
	config: &LeptoskitConfig,
	target: Target,
	force: bool,
) -> Result<Report> {
	let project = Project::read(root, config)?;
	let mut report = Report {
		target: Some(target),
		..Report::default()
	};

	for (file, contents) in target.files(&project) {
		write_owned(root, &file, &contents, force, &mut report)?;
	}

	if !project.has_probes {
		report.next.push(
			"The server doesn't answer /healthz and /readyz yet, and the health checks call them. \
			 In its main, make a `leptoskit::server::Readiness`, merge `readiness.routes()` into \
			 the router, and serve with `leptoskit::server::serve_listener_with(listener, app, \
			 GracefulShutdown::new().readiness(&readiness))`; the deployment guide shows the \
			 whole of it."
				.to_owned(),
		);
	}

	report.next.extend(target.next(&project));
	Ok(report)
}

/// Writes a file the app owns once it exists: creates it when missing, and
/// leaves it alone when it matches or, without `force`, when it differs.
fn write_owned(
	root: &Path,
	file: &str,
	contents: &str,
	force: bool,
	report: &mut Report,
) -> Result<()> {
	let path = root.join(file);

	match std::fs::symlink_metadata(&path) {
		Ok(metadata) => {
			anyhow::ensure!(metadata.is_file(), "{file} isn't an ordinary file");
			let existing =
				std::fs::read_to_string(&path).with_context(|| format!("could not read {file}"))?;
			if existing == contents {
				return Ok(());
			}
			if !force {
				report.kept.push(format!(
					"kept {file}: it differs from the template; --force replaces it"
				));
				return Ok(());
			}

			std::fs::write(&path, contents).with_context(|| format!("could not write {file}"))?;
			report.changed.push(format!("replaced {file}"));
		}

		Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
			std::fs::write(&path, contents).with_context(|| format!("could not write {file}"))?;
			report.changed.push(format!("created {file}"));
		}
		Err(error) => return Err(error).with_context(|| format!("could not inspect {file}")),
	}

	Ok(())
}

/// The directory of the package called `name`: the root package, or a
/// workspace member.
fn package_dir(root: &Path, name: &str) -> Result<Option<PathBuf>> {
	let manifest = root.join("Cargo.toml");
	let document = read_manifest(&manifest)?
		.with_context(|| format!("there's no Cargo.toml at {}", root.display()))?;

	if package_name(&document) == Some(name) {
		return Ok(Some(root.to_path_buf()));
	}

	let members = document
		.get("workspace")
		.and_then(|workspace| workspace.get("members"))
		.and_then(toml::Value::as_array)
		.into_iter()
		.flatten()
		.filter_map(toml::Value::as_str);

	for member in members {
		let dir = root.join(member);

		if let Some(member) = read_manifest(&dir.join("Cargo.toml"))?
			&& package_name(&member) == Some(name)
		{
			return Ok(Some(dir));
		}
	}

	Ok(None)
}

/// The server's binary target, and its source when it can be found:
/// the one named after the package, or its only one.
fn binary(dir: &Path, package: &str) -> Result<(String, Option<PathBuf>)> {
	let manifest = dir.join("Cargo.toml");
	let document = read_manifest(&manifest)?.unwrap_or_default();
	let mut binaries: Vec<(String, PathBuf)> = document
		.get("bin")
		.and_then(toml::Value::as_array)
		.into_iter()
		.flatten()
		.filter_map(|bin| {
			let name = bin.get("name")?.as_str()?.to_owned();
			let path = bin.get("path").and_then(toml::Value::as_str).map_or_else(
				|| dir.join("src/bin").join(format!("{name}.rs")),
				|path| dir.join(path),
			);
			Some((name, path))
		})
		.collect();
	let main = dir.join("src/main.rs");

	if main.is_file() && !binaries.iter().any(|(_, path)| *path == main) {
		binaries.push((package.to_owned(), main));
	}

	if let Some(index) = binaries.iter().position(|(name, _)| name == package) {
		let (name, path) = binaries.swap_remove(index);

		return Ok((name, Some(path).filter(|path| path.is_file())));
	}

	match binaries.as_slice() {
		[(name, path)] => {
			Ok((
				name.clone(),
				Some(path.clone()).filter(|path| path.is_file()),
			))
		}
		[] => {
			anyhow::bail!(
				"the server package `{package}` has no binary: add src/main.rs, or a [[bin]] in {}",
				manifest.display()
			)
		}
		_ => {
			anyhow::bail!(
				"the server package `{package}` has several binaries and none is called \
				 `{package}`; name the server's [[bin]] `{package}` in {}",
				manifest.display()
			)
		}
	}
}

/// A library's crate name: `[lib] name`, or the package name with `-` as
/// `_`.
fn lib_name(manifest: &Path) -> Result<Option<String>> {
	let Some(document) = read_manifest(manifest)? else {
		return Ok(None);
	};
	let explicit = document
		.get("lib")
		.and_then(|lib| lib.get("name"))
		.and_then(toml::Value::as_str);
	Ok(explicit
		.or_else(|| package_name(&document))
		.map(|name| name.replace('-', "_")))
}

/// The channel a toolchain file pins, when it's a release such as `1.96`
/// or `1.96.0` that the `rust` image has a tag for.
fn toolchain_channel(file: &Path) -> Option<String> {
	let contents = std::fs::read_to_string(file).ok()?;

	let channel = match contents.parse::<toml::Table>() {
		Ok(table) => table.get("toolchain")?.get("channel")?.as_str()?.to_owned(),
		// The legacy `rust-toolchain` file is the bare channel.
		Err(_) => contents.trim().to_owned(),
	};

	let release = !channel.is_empty()
		&& channel
			.split('.')
			.all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()));
	release.then_some(channel)
}

fn read_manifest(manifest: &Path) -> Result<Option<toml::Table>> {
	let Ok(contents) = std::fs::read_to_string(manifest) else {
		return Ok(None);
	};
	contents
		.parse()
		.map(Some)
		.with_context(|| format!("could not parse {}", manifest.display()))
}

fn package_name(document: &toml::Table) -> Option<&str> {
	document
		.get("package")
		.and_then(|package| package.get("name"))
		.and_then(toml::Value::as_str)
}

/// Keeps local builds, databases, and secrets out of the build context.
const DOCKERIGNORE: &str = "\
# Written by `leptoskit generate deploy`. Keeps the build context small, and
# local builds, databases, and secrets out of the image.
.git
**/target
/dist
/.leptoskit
/android/.gradle
/android/**/build
*.sqlite3*
.env
.env.*
";

/// The multi-stage `Dockerfile`: build with `leptoskit build --release`,
/// run as an unprivileged user on a read-only filesystem.
fn dockerfile(project: &Project) -> String {
	let Project {
		app,
		binary,
		port,
		output_name,
		platform,
		rust_version,
		..
	} = project;
	let image = project.image();
	let web = *platform == Platform::Web;
	let version = env!("CARGO_PKG_VERSION");
	let mut out = format!(
		"# syntax=docker/dockerfile:1
#
# The container for {app}, written by `leptoskit generate deploy`. It's yours
# to edit: running the generator again keeps your changes unless you pass
# --force. Build it from the project root:
#
#   docker build -t {image} .
#
# The first stage runs `leptoskit build --release`, as you would on your
# machine. The second runs the server as an unprivileged user that writes
# nothing to the image's filesystem, so it works with a read-only root
# filesystem.
"
	);

	if project.sqlite().is_some() {
		let _ = writeln!(
			out,
			"#\n# The SQLite database is the one thing it writes, on a volume at {DATA_DIR}."
		);
	}

	let _ = write!(
		out,
		"
# The Rust release that builds the app. `1` is the latest stable; pin one,
# such as 1.96, to build the same way every time.
ARG RUST_VERSION={rust_version}
# The leptoskit CLI that builds the app. Keep it in step with the `leptoskit`
# version in Cargo.toml.
ARG LEPTOSKIT_VERSION={version}
"
	);

	if web {
		let _ = writeln!(
			out,
			"# binaryen's wasm-opt shrinks the release client.\nARG \
			 BINARYEN_VERSION={BINARYEN_VERSION}"
		);
	}

	out.push_str(
		"
FROM rust:${RUST_VERSION}-bookworm AS build
ARG LEPTOSKIT_VERSION
",
	);

	if web {
		out.push_str(
			"ARG BINARYEN_VERSION
RUN curl --fail --silent --show-error --location \\
      \"https://github.com/WebAssembly/binaryen/releases/download/version_${BINARYEN_VERSION}/binaryen-version_${BINARYEN_VERSION}-$(uname -m)-linux.tar.gz\" \\
    | tar -xz -C /opt \\
 && ln -s \"/opt/binaryen-version_${BINARYEN_VERSION}/bin/wasm-opt\" /usr/local/bin/wasm-opt
",
		);
	}

	out.push_str(
		"RUN cargo install leptoskit_cli --version \"${LEPTOSKIT_VERSION}\" --locked
WORKDIR /src
",
	);

	if let Some(file) = &project.toolchain_file {
		let _ = writeln!(
			out,
			"# The toolchain file picks the toolchain everything below uses.\nCOPY {file} ./"
		);
	}

	if web {
		out.push_str("RUN rustup target add wasm32-unknown-unknown\n");
	}

	let _ = write!(
		out,
		"COPY . .
# The cache mounts keep downloaded crates and compiled dependencies between
# builds. The server is copied out, since the target directory is a cache.
RUN --mount=type=cache,target=/usr/local/cargo/registry \\
    --mount=type=cache,target=/src/target \\
    --mount=type=cache,target=/src/.leptoskit \\
    leptoskit build --release --platform {platform} \\
 && cp target/release/{binary} /usr/local/bin/{binary}

FROM debian:bookworm-slim
# curl answers the health check, and `app` is the unprivileged user the
# server runs as.
RUN apt-get update \\
 && apt-get install --yes --no-install-recommends curl \\
 && rm -rf /var/lib/apt/lists/* \\
 && useradd --system --user-group --uid 10001 --no-create-home app",
	);

	if project.sqlite().is_some() {
		let _ = write!(
			out,
			" \\\n && mkdir {DATA_DIR} \\\n && chown app:app {DATA_DIR}"
		);
	}

	let _ = write!(
		out,
		"
WORKDIR /app
COPY --from=build /usr/local/bin/{binary} /app/server
COPY --from=build /src/{DIST_DIR} /app/{DIST_DIR}
# What `leptoskit serve` sets from leptoskit.toml, for a container: listen on
# every interface, and serve the site from /app/{DIST_DIR}.
ENV LEPTOS_ENV=PROD \\
    LEPTOS_OUTPUT_NAME={output_name} \\
    LEPTOS_SITE_ROOT=/app/{DIST_DIR} \\
    LEPTOS_SITE_PKG_DIR=pkg \\
    LEPTOS_SITE_ADDR=0.0.0.0:{port}
"
	);

	if let Some(database) = project.sqlite() {
		let _ = write!(
			out,
			"# The SQLite database lives on a volume, the one place the server writes.
ENV {}={}
VOLUME {DATA_DIR}
",
			database.url_env,
			project.sqlite_url()
		);
	}

	let secrets = project.runtime_secrets();

	if !secrets.is_empty() {
		out.push_str(
			"# These secrets come from the environment when the container starts, never\n# from \
			 this file:\n",
		);

		for secret in secrets {
			let _ = writeln!(out, "#   {secret}");
		}
	}

	let _ = write!(
		out,
		"USER app
EXPOSE {port}
HEALTHCHECK --interval=10s --timeout=3s --start-period=30s --retries=3 \\
  CMD [\"curl\", \"--fail\", \"--silent\", \"--output\", \"/dev/null\", \"http://127.0.0.1:{port}/healthz\"]
CMD [\"/app/server\"]
"
	);

	out
}

/// `compose.yaml`: the app, read-only, with Postgres or a SQLite volume.
fn compose(project: &Project) -> String {
	let image = project.image();
	let port = project.port;
	let user = project.app.replace('-', "_");
	let mut required = Vec::new();

	if project.postgres().is_some() {
		required.push("POSTGRES_PASSWORD".to_owned());
	}

	required.extend(project.secrets.iter().cloned());
	let mut out = format!(
		"# {app} with Docker Compose, written by `leptoskit generate deploy`. It's
# yours to edit: running the generator again keeps your changes unless you
# pass --force.
#
#   docker compose up --build
#
",
		app = project.app
	);

	if required.is_empty() {
		out.push_str(
			"# Secrets, once the app has some, come from the environment or an `.env`\n# file \
			 beside this one, which .dockerignore keeps out of the image. They never\n# go in \
			 this file.\n",
		);
	} else {
		out.push_str(
			"# It reads these secrets from the environment, or from an `.env` file\n# beside this \
			 one, which .dockerignore keeps out of the image. They never go\n# in this file.\n#\n",
		);

		for secret in &required {
			let _ = writeln!(out, "#   {secret}");
		}
	}

	let _ = write!(
		out,
		"
name: {image}

services:
  app:
    build: .
    image: {image}
    # Only this machine reaches the server: put a proxy that terminates TLS
    # in front of it.
    ports:
      - \"127.0.0.1:{port}:{port}\"
"
	);
	let mut environment = Vec::new();

	if let Some(database) = project.postgres() {
		environment.push(format!(
			"{}: postgres://{user}:${{POSTGRES_PASSWORD}}@db:5432/{user}?sslmode=disable",
			database.url_env
		));
	}

	for secret in &project.secrets {
		environment.push(format!("{secret}: ${{{secret}:?set {secret}}}"));
	}

	if !environment.is_empty() {
		out.push_str("    environment:\n");

		for line in environment {
			let _ = writeln!(out, "      {line}");
		}
	}

	let _ = write!(
		out,
		"    # Nothing in the image changes while it runs{}.
    read_only: true
    tmpfs:
      - /tmp
",
		if project.sqlite().is_some() {
			"; the database lives on a volume"
		} else {
			""
		}
	);

	if project.sqlite().is_some() {
		let _ = write!(out, "    volumes:\n      - data:{DATA_DIR}\n");
	}

	if project.postgres().is_some() {
		out.push_str(
			"    depends_on:
      db:
        condition: service_healthy
",
		);
	}

	let _ = write!(
		out,
		"    # On `docker compose down` the server fails /readyz, waits 5 seconds, and
    # gives the requests in flight 20 more, so this leaves it time to finish.
    stop_grace_period: {STOP_TIMEOUT_SECONDS}s
    restart: unless-stopped
"
	);

	if project.postgres().is_some() {
		let _ = write!(
			out,
			"
  db:
    image: postgres:17
    environment:
      POSTGRES_USER: {user}
      POSTGRES_DB: {user}
      POSTGRES_PASSWORD: ${{POSTGRES_PASSWORD:?set POSTGRES_PASSWORD}}
    volumes:
      - postgres:/var/lib/postgresql/data
    healthcheck:
      test: [\"CMD\", \"pg_isready\", \"--username={user}\", \"--dbname={user}\"]
      interval: 5s
      timeout: 3s
      retries: 10
    restart: unless-stopped

volumes:
  postgres:
"
		);
	} else if project.sqlite().is_some() {
		out.push_str("\nvolumes:\n  data:\n");
	}

	out
}

/// `fly.toml`: checks on `/readyz`, a kill timeout past the grace period,
/// and machines that stay up for long-lived connections.
fn fly(project: &Project) -> String {
	let mut out = format!(
		"# {app} on Fly.io, written by `leptoskit generate deploy`. It's yours to
# edit: running the generator again keeps your changes unless you pass
# --force. Create the app from it with `fly launch --copy-config --no-deploy`.

app = \"{fly_app}\"

# On SIGTERM the server fails /readyz, waits 5 seconds for the proxy to
# notice, and gives the requests in flight 20 more. Fly kills it after this.
kill_signal = \"SIGTERM\"
kill_timeout = \"{STOP_TIMEOUT_SECONDS}s\"

[build]
  dockerfile = \"Dockerfile\"

[http_service]
  internal_port = {port}
  force_https = true
  # Machines keep running. Fly stops a machine it thinks is idle, and that
  # drops every WebSocket and streaming connection on it, so clients of an
  # app that holds them would disconnect without warning. An app that only
  # answers short requests can scale to zero instead, with
  # auto_stop_machines = \"stop\" and min_machines_running = 0.
  auto_stop_machines = \"off\"
  auto_start_machines = true
  min_machines_running = 1

  # The proxy sends traffic only to machines whose /readyz answers 200.
  [[http_service.checks]]
    grace_period = \"30s\"
    interval = \"10s\"
    timeout = \"2s\"
    method = \"GET\"
    path = \"/readyz\"
",
		app = project.app,
		fly_app = project.app.to_ascii_lowercase().replace('_', "-"),
		port = project.port,
	);

	if project.sqlite().is_some() {
		let _ = write!(
			out,
			"
# The SQLite database lives on this volume. A volume belongs to one machine,
# so run one machine: `fly scale count 1`.
[mounts]
  source = \"data\"
  destination = \"{DATA_DIR}\"
"
		);
	}

	if let Some(database) = project.postgres() {
		let _ = write!(
			out,
			"
# {} comes from `fly postgres attach`, which stores it as a secret.
",
			database.url_env
		);
	}

	out
}

fn docker_next(project: &Project) -> Vec<String> {
	let image = project.image();
	let port = project.port;
	let mut run = format!("docker run --rm --read-only --tmpfs /tmp -p 127.0.0.1:{port}:{port}");

	if project.sqlite().is_some() {
		let _ = write!(run, " -v {image}-data:{DATA_DIR}");
	}

	for secret in project.runtime_secrets() {
		let _ = write!(run, " -e {secret}");
	}

	let _ = write!(run, " {image}");

	let mut steps = vec![format!("Build the image: `docker build -t {image} .`")];
	let secrets = project.runtime_secrets();

	if !secrets.is_empty() {
		steps.push(format!(
			"Set {} in your shell; `-e NAME` passes each one into the container.",
			secrets.join(", ")
		));
	}

	steps.push(format!("Run it: `{run}`"));
	steps
}

fn compose_next(project: &Project) -> Vec<String> {
	let mut steps = Vec::new();

	if project.postgres().is_some() {
		steps.push(
			"Choose the database password: `export POSTGRES_PASSWORD=\"$(openssl rand -hex \
			 24)\"`, or put it in .env."
				.to_owned(),
		);
	}

	for secret in &project.secrets {
		steps.push(format!("Set {secret}, in your shell or in .env."));
	}

	steps.push("Start it: `docker compose up --build`".to_owned());
	steps
}

fn fly_next(project: &Project) -> Vec<String> {
	let mut steps =
		vec!["Create the app from fly.toml: `fly launch --copy-config --no-deploy`".to_owned()];

	if project.sqlite().is_some() {
		steps.push("Create the database's volume: `fly volumes create data --size 1`".to_owned());
	}

	if let Some(database) = project.postgres() {
		let variable = if database.url_env == "DATABASE_URL" {
			String::new()
		} else {
			format!(" --variable-name {}", database.url_env)
		};

		steps.push(format!(
			"Give it a database: `fly postgres create`, then `fly postgres attach \
			 <name>{variable}` sets {}.",
			database.url_env
		));
	}

	if !project.secrets.is_empty() {
		let pairs: Vec<String> = project
			.secrets
			.iter()
			.map(|secret| format!("{secret}=..."))
			.collect();
		steps.push(format!(
			"Set the secrets: `fly secrets set {}`",
			pairs.join(" ")
		));
	}

	steps.push("Deploy: `fly deploy`".to_owned());
	steps
}

/// A Kubernetes Deployment, Service, and `PodDisruptionBudget`.
#[expect(
	clippy::too_many_lines,
	reason = "one manifest per resource reads best beside the others"
)]
fn kubernetes(project: &Project) -> String {
	let image = project.image();
	let port = project.port;
	let replicas = if project.sqlite().is_some() { 1 } else { 2 };
	let mut out = format!(
		"# {app} on Kubernetes, written by `leptoskit generate deploy`. It's
# yours to edit: running the generator again keeps your changes unless you
# pass --force.
#
#   kubectl apply -f kubernetes.yaml
#
# The Deployment runs {replicas} replica{plural}. On SIGTERM the server fails
# /readyz, waits 5 seconds for the load balancer to stop sending it traffic,
# and gives requests in flight 20 more; the pod's grace period is {STOP_TIMEOUT_SECONDS} seconds.
",
		app = project.app,
		plural = if replicas == 1 { "" } else { "s" },
	);

	if project.sqlite().is_some() {
		out.push_str(
			"#\n# The SQLite database lives on a PersistentVolumeClaim, so the\n# Deployment runs \
			 one replica and replaces it on deploy.\n",
		);
	}

	if !project.runtime_secrets().is_empty() {
		let _ = write!(
			out,
			"#\n# The container reads these from the Secret `{image}`, which you create\n# with \
			 `kubectl create secret generic {image} --from-literal=...`:\n",
		);

		for secret in project.runtime_secrets() {
			let _ = writeln!(out, "#   {secret}");
		}
	}

	let _ = write!(
		out,
		"
apiVersion: apps/v1
kind: Deployment
metadata:
  name: {image}
spec:
  replicas: {replicas}
"
	);

	if project.sqlite().is_some() {
		out.push_str(
			"  strategy:
    type: Recreate
",
		);
	}

	let _ = write!(
		out,
		"  selector:
    matchLabels:
      app: {image}
  template:
    metadata:
      labels:
        app: {image}
    spec:
      terminationGracePeriodSeconds: {STOP_TIMEOUT_SECONDS}
      containers:
        - name: {image}
          image: {image}
          ports:
            - containerPort: {port}
          env:
            - name: LEPTOS_SITE_ADDR
              value: \"0.0.0.0:{port}\"
"
	);

	if !project.runtime_secrets().is_empty() {
		let _ = write!(
			out,
			"          envFrom:
            - secretRef:
                name: {image}
"
		);
	}

	let _ = write!(
		out,
		"          readinessProbe:
            httpGet:
              path: /readyz
              port: {port}
            initialDelaySeconds: 5
            periodSeconds: 5
          livenessProbe:
            httpGet:
              path: /healthz
              port: {port}
            initialDelaySeconds: 10
            periodSeconds: 10
          lifecycle:
            preStop:
              exec:
                command: [\"sh\", \"-c\", \"sleep 5\"]
          securityContext:
            runAsNonRoot: true
            readOnlyRootFilesystem: true
            allowPrivilegeEscalation: false
"
	);

	if project.sqlite().is_some() {
		let _ = write!(
			out,
			"          volumeMounts:
            - name: data
              mountPath: {DATA_DIR}
      volumes:
        - name: data
          persistentVolumeClaim:
            claimName: {image}-data
---
apiVersion: v1
kind: PersistentVolumeClaim
metadata:
  name: {image}-data
spec:
  accessModes: [ReadWriteOnce]
  resources:
    requests:
      storage: 1Gi
"
		);
	}

	let _ = write!(
		out,
		"---
apiVersion: v1
kind: Service
metadata:
  name: {image}
spec:
  selector:
    app: {image}
  ports:
    - port: 80
      targetPort: {port}
---
apiVersion: policy/v1
kind: `PodDisruptionBudget`
metadata:
  name: {image}
spec:
  minAvailable: 1
  selector:
    matchLabels:
      app: {image}
"
	);
	out
}

/// A systemd service that runs the server as a hardened unit.
fn systemd(project: &Project) -> String {
	let mut out = format!(
		"# {app} under systemd, written by `leptoskit generate deploy`. It's
# yours to edit: running the generator again keeps your changes unless you
# pass --force.
#
#   sudo cp {app}.service /etc/systemd/system/
#   sudo systemctl daemon-reload
#   sudo systemctl enable --now {app}
#
# It reads the environment from /etc/{app}/{app}.env, which is where the
# secrets go. The binary and the built site are installed by your deploy;
# the paths below match /usr/local/bin and /srv/{app}.
",
		app = project.app,
	);
	let _ = write!(
		out,
		"
[Unit]
Description={app}
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
User={app}
Group={app}
WorkingDirectory=/srv/{app}
EnvironmentFile=/etc/{app}/{app}.env
Environment=LEPTOS_SITE_ADDR=0.0.0.0:{port}
Environment=LEPTOS_SITE_ROOT=/srv/{app}/{DIST_DIR}
",
		app = project.app,
		port = project.port,
	);

	if project.sqlite().is_some() {
		let _ = writeln!(
			out,
			"Environment={url_env}=sqlite:///var/lib/{app}/{app}.sqlite3",
			url_env = project.sqlite().unwrap().url_env,
			app = project.app
		);
		out.push_str("StateDirectory=");
		out.push_str(&project.app);
		out.push('\n');
	}

	let _ = write!(
		out,
		"ExecStart=/usr/local/bin/{binary}
Restart=on-failure
RestartSec=5
KillSignal=SIGTERM
TimeoutStopSec={STOP_TIMEOUT_SECONDS}

# A read-only filesystem, except for the paths systemd gives the service.
NoNewPrivileges=true
ProtectSystem=strict
ProtectHome=true
PrivateTmp=true
PrivateDevices=true
ProtectKernelTunables=true
ProtectKernelModules=true
ProtectControlGroups=true
RestrictSUIDSGID=true
",
		binary = project.binary,
	);

	if project.sqlite().is_some() {
		let _ = writeln!(out, "ReadWritePaths=/var/lib/{}", project.app);
	}

	out.push_str(
		"
[Install]
WantedBy=multi-user.target
",
	);
	out
}

fn kubernetes_next(project: &Project) -> Vec<String> {
	let image = project.image();
	let mut steps = vec![
		format!(
			"Build and push the image: `docker build -t {image} .` then push it to your registry."
		),
		"Set image to the pushed reference in kubernetes.yaml.".to_owned(),
	];

	if !project.runtime_secrets().is_empty() {
		steps.push(format!(
			"Create the Secret: `kubectl create secret generic {image} --from-literal=NAME=...`."
		));
	}

	steps.push("Apply it: `kubectl apply -f kubernetes.yaml`".to_owned());
	steps
}

fn systemd_next(project: &Project) -> Vec<String> {
	let mut steps = vec![
		"Install the binary at /usr/local/bin and the built site at /srv/<app>.".to_owned(),
		"Put the environment, including secrets, in /etc/<app>/<app>.env.".to_owned(),
		format!(
			"Install the unit and start it: `sudo cp {}.service /etc/systemd/system/ && sudo \
			 systemctl daemon-reload && sudo systemctl enable --now {}`.",
			project.app, project.app
		),
	];
	steps.push("Create the app's system user, or change User/Group in the unit.".to_owned());
	steps
}

#[cfg(test)]
mod tests;
