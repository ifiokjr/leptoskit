//! Services: other processes the app runs beside its server.
//!
//! A `[services.<name>]` table names where a service comes from (a package in
//! the workspace, an installed crate, or a remote URL) and overrides what the
//! service's own manifest, `[package.metadata.leptoskit.service]`, declares:
//! its port, whether browsers reach it, its database, and the variables it
//! and the app receive.
//!
//! ```toml
//! [services.analytics]
//! package = "analytics_service"
//!
//! [services.billing]
//! url = "https://billing.internal.example"
//! ```

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::Deserialize;
use serde::Serialize;

use crate::infra::env_name;

/// Where every service's public paths start: a service is reachable at
/// `/_leptoskit/services/<name>/` on the app's origin when it's public.
pub const SERVICES_PREFIX: &str = "/_leptoskit/services";

/// Paths the app serves itself, which a service's `routes` can't shadow.
pub const RESERVED_PATHS: &[&str] = &["/pkg", "/api", "/live", "/_leptoskit"];

/// The placeholders a `bind` value can use.
pub const BIND_PLACEHOLDERS: &[&str] = &["url", "token", "public_path"];

/// One `[services.<name>]` table.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields, default)]
pub struct ServiceConfig {
	/// A package in this Cargo workspace.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub package: Option<String>,
	/// A crate at this path, relative to the project, built by `leptoskit`.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub path: Option<PathBuf>,
	/// A crate from the registry, built by `leptoskit`. With `git`, the
	/// package to build from the repository, which otherwise is the
	/// service's name.
	#[serde(rename = "crate", skip_serializing_if = "Option::is_none")]
	pub krate: Option<String>,
	/// The version requirement for `crate`.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub version: Option<String>,
	/// A git repository holding the crate, built by `leptoskit`.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub git: Option<String>,
	/// The git commit to build.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub rev: Option<String>,
	/// The git branch to build.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub branch: Option<String>,
	/// The git tag to build.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub tag: Option<String>,
	/// A service already running elsewhere: `leptoskit` connects the app to
	/// it and runs nothing.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub url: Option<String>,
	/// Which binary to run, when the package has several.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub bin: Option<String>,
	/// Cargo features the service binary is built with.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub features: Option<Vec<String>>,
	/// The port the service listens on.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub port: Option<u16>,
	/// Whether browsers and native apps reach the service at
	/// `/_leptoskit/services/<name>/` on the app's origin.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub public: Option<bool>,
	/// Extra public path prefixes sent to the service, such as `/_analytics`.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub routes: Option<Vec<String>>,
	/// Whether the service gets a database of its own, on the engine
	/// `[db] dialect` names, or SQLite when the app has no `[db]`.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub database: Option<bool>,
	/// The most connections the service's database pool opens. Every
	/// process's pools, across replicas, share the database server's limit.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub pool: Option<u32>,
	/// How far the app trusts the service with its origin: `first-party`
	/// services get the app's cookies on their public paths, `isolated` ones
	/// don't. Defaults to `first-party` for a `package` in the workspace and
	/// `isolated` for anything installed or remote.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub trust: Option<ServiceTrust>,
	/// Plain values set on the service.
	#[serde(skip_serializing_if = "BTreeMap::is_empty")]
	pub env: BTreeMap<String, String>,
	/// Variables passed through from the environment to the service only.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub secrets: Option<Vec<String>>,
	/// Named persistent directories the service owns.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub volumes: Option<Vec<String>>,
	/// Variables set on the app while this service is present. Values can use
	/// `{url}`, `{token}`, and `{public_path}`.
	#[serde(skip_serializing_if = "BTreeMap::is_empty")]
	pub bind: BTreeMap<String, String>,
}

/// What a service crate declares about itself, in
/// `[package.metadata.leptoskit.service]`. Every key is optional;
/// `leptoskit.toml` overrides any of them.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct ServiceManifest {
	/// The service's name: the `service = "…"` its operations are declared
	/// with. Required: an operation routes by this name, so the
	/// `[services.<name>]` table that uses the package must match it.
	pub name: Option<String>,
	/// Which binary to run, when the package has several.
	pub bin: Option<String>,
	/// Cargo features the service binary is built with.
	pub features: Option<Vec<String>>,
	/// The port the service listens on.
	pub port: Option<u16>,
	/// Whether browsers and native apps reach the service.
	pub public: Option<bool>,
	/// Extra public path prefixes.
	pub routes: Option<Vec<String>>,
	/// Whether the service gets a database of its own.
	pub database: Option<bool>,
	/// The most connections the service's database pool opens.
	pub pool: Option<u32>,
	/// Plain values set on the service.
	pub env: BTreeMap<String, String>,
	/// Variables passed through from the environment.
	pub secrets: Option<Vec<String>>,
	/// Named persistent directories.
	pub volumes: Option<Vec<String>>,
	/// Variables set on the app.
	pub bind: BTreeMap<String, String>,
}

/// How far the app trusts a service with its origin.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "kebab-case")]
pub enum ServiceTrust {
	/// The app's own code: its public paths get the app's cookies, and its
	/// responses can set them.
	FirstParty,
	/// Someone else's code: the proxy and the edge strip `Cookie` from
	/// requests to its public paths and `Set-Cookie` from its responses.
	Isolated,
}

impl ServiceTrust {
	/// How it's spelled in `leptoskit.toml`.
	#[must_use]
	pub fn as_str(self) -> &'static str {
		match self {
			Self::FirstParty => "first-party",
			Self::Isolated => "isolated",
		}
	}
}

/// A git reference to build.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GitReference {
	/// A commit.
	Rev(String),
	/// A branch.
	Branch(String),
	/// A tag.
	Tag(String),
}

/// Where a service comes from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum ServiceSource {
	/// A package in the app's Cargo workspace.
	Package {
		/// The package name.
		name: String,
	},
	/// A crate at a path, relative to the project.
	Path {
		/// The crate's directory.
		path: PathBuf,
	},
	/// A crate from the registry.
	Crate {
		/// The crate name.
		name: String,
		/// The version requirement, or the newest.
		version: Option<String>,
	},
	/// A crate in a git repository.
	Git {
		/// The repository URL.
		url: String,
		/// The package to build from it: `crate`, or the service's name.
		package: String,
		/// The reference to build, or the default branch.
		reference: Option<GitReference>,
	},
	/// A service already running elsewhere.
	Remote {
		/// Its base URL.
		url: String,
	},
}

impl ServiceSource {
	/// Whether `leptoskit` runs this service as a process.
	#[must_use]
	pub fn is_process(&self) -> bool {
		!matches!(self, Self::Remote { .. })
	}

	/// The source's kind, as `leptoskit services` names it.
	#[must_use]
	pub fn kind(&self) -> &'static str {
		match self {
			Self::Package { .. } => "package",
			Self::Path { .. } => "path",
			Self::Crate { .. } => "crate",
			Self::Git { .. } => "git",
			Self::Remote { .. } => "remote",
		}
	}
}

/// A service's settings once its manifest and `leptoskit.toml` are merged.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedService {
	/// The service's name, the key of its `[services.<name>]` table.
	pub name: String,
	/// Where it comes from.
	pub source: ServiceSource,
	/// The binary to run, when the package has several.
	pub bin: Option<String>,
	/// Cargo features the binary is built with.
	pub features: Vec<String>,
	/// Its port; `None` only for a remote service.
	pub port: Option<u16>,
	/// Whether browsers reach it.
	pub public: bool,
	/// Extra public path prefixes.
	pub routes: Vec<String>,
	/// Whether it has a database of its own.
	pub database: bool,
	/// The most connections its database pool opens, when set.
	pub pool: Option<u32>,
	/// How far the app trusts it with its origin.
	pub trust: ServiceTrust,
	/// Plain values set on it.
	pub env: BTreeMap<String, String>,
	/// Variables passed through to it.
	pub secrets: Vec<String>,
	/// Its persistent directories.
	pub volumes: Vec<String>,
	/// Variables set on the app.
	pub bind: BTreeMap<String, String>,
}

/// A manifest must name its service, and the name must be the table's:
/// operations declared `service = "analytics"` reach
/// `/_leptoskit/services/analytics` and `LEPTOSKIT_SERVICE_ANALYTICS_*`
/// whatever the table is called, so any other key would misroute them.
fn check_manifest_name(name: &str, manifest: &ServiceManifest) -> Result<(), String> {
	match manifest.name.as_deref() {
		None => {
			Err(format!(
				"services.{name}: the package's [package.metadata.leptoskit.service] has no \
				 `name`; add `name = \"…\"` with the `service = \"…\"` its operations are \
				 declared with"
			))
		}
		Some(declared) if declared != name => {
			Err(format!(
				"services.{name}: the package's [package.metadata.leptoskit.service] names the \
				 service `{declared}`, and its operations route by that name; rename the table to \
				 [services.{declared}]"
			))
		}
		Some(_) => Ok(()),
	}
}

impl ResolvedService {
	/// Merges a service's manifest with its `[services.<name>]` table: a value
	/// in `leptoskit.toml` replaces the manifest's, and `env` and `bind`
	/// merge key by key, `leptoskit.toml` winning.
	///
	/// # Errors
	///
	/// When the table's source is invalid (see [`ServiceConfig::source`]),
	/// or the manifest doesn't name the service `name`.
	pub fn merge(
		name: &str,
		config: &ServiceConfig,
		manifest: Option<&ServiceManifest>,
	) -> Result<Self, String> {
		let source = config.source(name)?;
		let trust = config.trust.unwrap_or(match source {
			ServiceSource::Package { .. } => ServiceTrust::FirstParty,
			_ => ServiceTrust::Isolated,
		});
		if let Some(manifest) = manifest {
			check_manifest_name(name, manifest)?;
		}

		let empty = ServiceManifest::default();
		let manifest = manifest.unwrap_or(&empty);
		let pick = |config: &Option<Vec<String>>, manifest: &Option<Vec<String>>| {
			config
				.clone()
				.or_else(|| manifest.clone())
				.unwrap_or_default()
		};
		let mut env = manifest.env.clone();
		env.extend(config.env.clone());
		let mut bind = manifest.bind.clone();
		bind.extend(config.bind.clone());

		Ok(Self {
			name: name.to_owned(),
			source,
			bin: config.bin.clone().or_else(|| manifest.bin.clone()),
			features: pick(&config.features, &manifest.features),
			port: config.port.or(manifest.port),
			public: config.public.or(manifest.public).unwrap_or(false),
			routes: pick(&config.routes, &manifest.routes),
			database: config.database.or(manifest.database).unwrap_or(false),
			pool: config.pool.or(manifest.pool),
			trust,
			env,
			secrets: pick(&config.secrets, &manifest.secrets),
			volumes: pick(&config.volumes, &manifest.volumes),
			bind,
		})
	}

	/// The prefix browsers reach the service under:
	/// `/_leptoskit/services/<name>`.
	#[must_use]
	pub fn public_path(&self) -> String {
		public_path(&self.name)
	}

	/// Every public path prefix: the service's own, then its `routes`. Empty
	/// when the service isn't public.
	#[must_use]
	pub fn public_prefixes(&self) -> Vec<String> {
		if !self.public {
			return Vec::new();
		}

		std::iter::once(self.public_path())
			.chain(self.routes.iter().cloned())
			.collect()
	}
}

/// The prefix browsers reach the service `name` under.
#[must_use]
pub fn public_path(name: &str) -> String {
	format!("{SERVICES_PREFIX}/{name}")
}

/// The service's name as it appears in environment variables: uppercase,
/// with `-` as `_`.
#[must_use]
pub fn env_segment(name: &str) -> String {
	name.to_ascii_uppercase().replace('-', "_")
}

/// Whether `name` can name a service or a volume: a lowercase letter, then
/// lowercase letters, digits, and `-`.
#[must_use]
pub fn is_service_name(name: &str) -> bool {
	let mut bytes = name.bytes();
	matches!(bytes.next(), Some(b'a'..=b'z'))
		&& bytes.all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
		&& !name.ends_with('-')
}

impl ServiceConfig {
	/// The table's one source.
	///
	/// # Errors
	///
	/// When the table names no source or several, or a source's companion
	/// key appears with another source.
	pub fn source(&self, name: &str) -> Result<ServiceSource, String> {
		let named: Vec<&str> = [
			("package", self.package.is_some()),
			("path", self.path.is_some()),
			// With `git`, `crate` names the package in the repository.
			("crate", self.krate.is_some() && self.git.is_none()),
			("git", self.git.is_some()),
			("url", self.url.is_some()),
		]
		.into_iter()
		.filter_map(|(key, set)| set.then_some(key))
		.collect();

		match named.as_slice() {
			[] => {
				return Err(format!(
					"services.{name} needs a source: `package`, `path`, `crate`, `git`, or `url`"
				));
			}
			[_] => {}
			several => {
				return Err(format!(
					"services.{name} names {} sources; keep one",
					several
						.iter()
						.map(|key| format!("`{key}`"))
						.collect::<Vec<_>>()
						.join(" and ")
				));
			}
		}

		if self.version.is_some() && (self.krate.is_none() || self.git.is_some()) {
			return Err(format!(
				"services.{name}.version goes with a registry `crate`; a `git` source takes \
				 `rev`, `branch`, or `tag`"
			));
		}

		let references: Vec<GitReference> = [
			self.rev.clone().map(GitReference::Rev),
			self.branch.clone().map(GitReference::Branch),
			self.tag.clone().map(GitReference::Tag),
		]
		.into_iter()
		.flatten()
		.collect();

		if !references.is_empty() && self.git.is_none() {
			return Err(format!(
				"services.{name}: `rev`, `branch`, and `tag` go with `git`, which this table \
				 doesn't set"
			));
		}

		if references.len() > 1 {
			return Err(format!(
				"services.{name} sets more than one of `rev`, `branch`, and `tag`; keep one"
			));
		}

		Ok(if let Some(package) = &self.package {
			ServiceSource::Package {
				name: package.clone(),
			}
		} else if let Some(path) = &self.path {
			ServiceSource::Path { path: path.clone() }
		} else if let Some(git) = &self.git {
			ServiceSource::Git {
				url: git.clone(),
				package: self.krate.clone().unwrap_or_else(|| name.to_owned()),
				reference: references.into_iter().next(),
			}
		} else if let Some(krate) = &self.krate {
			ServiceSource::Crate {
				name: krate.clone(),
				version: self.version.clone(),
			}
		} else {
			ServiceSource::Remote {
				url: self.url.clone().unwrap_or_default(),
			}
		})
	}

	/// What's wrong with the table on its own, before its manifest is read.
	fn problem(&self, name: &str) -> Option<String> {
		if !is_service_name(name) {
			return Some(format!(
				"services.{name}: a service's name starts with a lowercase letter and holds only \
				 lowercase letters, digits, and `-`"
			));
		}

		// `LEPTOSKIT_SERVICE_VOLUME_*` names a service's own volumes, so a
		// service called `volume…` would read the app-side
		// `LEPTOSKIT_SERVICE_VOLUME_…_URL` as a volume.
		if name == "volume" || name.starts_with("volume-") {
			return Some(format!(
				"services.{name}: `volume` and names starting `volume-` are reserved, because \
				 `LEPTOSKIT_SERVICE_VOLUME_*` names a service's volumes; pick another name"
			));
		}

		let source = match self.source(name) {
			Ok(source) => source,
			Err(problem) => return Some(problem),
		};

		if let ServiceSource::Remote { url } = &source {
			let process_only = [
				("bin", self.bin.is_some()),
				("features", self.features.is_some()),
				("port", self.port.is_some()),
				("database", self.database.is_some()),
				("pool", self.pool.is_some()),
				("env", !self.env.is_empty()),
				("secrets", self.secrets.is_some()),
				("volumes", self.volumes.is_some()),
			];

			if let Some((key, _)) = process_only.iter().find(|(_, set)| *set) {
				return Some(format!(
					"services.{name} is remote (`url`), so `leptoskit` runs nothing for it and \
					 `{key}` has no meaning; remote services take `url`, `bind`, and `trust`"
				));
			}

			// Browsers reach a public service at the app's own origin, through
			// the dev proxy or the deploy's edge, and neither forwards to a
			// remote origin yet: a public remote service would answer 404.
			if self.public == Some(true) || self.routes.is_some() {
				return Some(format!(
					"services.{name} is remote (`url`), and remote services can't be public yet: \
					 browsers would reach `/_leptoskit/services/{name}` on the app's origin, \
					 which nothing forwards to `{url}`; remove `public` and `routes`, or call it \
					 from the app's server only"
				));
			}

			let parsed = url::Url::parse(url).ok();

			if !parsed.is_some_and(|url| matches!(url.scheme(), "http" | "https")) {
				return Some(format!(
					"services.{name}.url must be an http or https URL, not `{url}`"
				));
			}
		}

		if let Some(package) = &self.package
			&& package.trim().is_empty()
		{
			return Some(format!("services.{name}.package is empty"));
		}

		if self.pool == Some(0) {
			return Some(format!(
				"services.{name}.pool is 0; a pool needs at least one connection"
			));
		}

		if self.bin.as_deref().is_some_and(str::is_empty) {
			return Some(format!("services.{name}.bin is empty"));
		}

		if let Some(feature) = self
			.features
			.iter()
			.flatten()
			.find(|feature| feature.trim().is_empty())
		{
			return Some(format!(
				"services.{name}.features has an empty feature `{feature}`"
			));
		}

		settings_problem(
			name,
			&self.env,
			self.secrets.as_deref().unwrap_or_default(),
			self.volumes.as_deref().unwrap_or_default(),
			&self.bind,
			self.routes.as_deref().unwrap_or_default(),
		)
	}
}

/// The variables the runtime contract sets, which `env`, `secrets`, and
/// `bind` can't name.
fn reserved_variable(name: &str) -> bool {
	name.starts_with("LEPTOSKIT_SERVICE")
		|| matches!(
			name,
			"LEPTOSKIT_APP_URL" | "LEPTOSKIT_PUBLIC_ORIGIN" | "DATABASE_URL"
		)
}

fn settings_problem(
	name: &str,
	env: &BTreeMap<String, String>,
	secrets: &[String],
	volumes: &[String],
	bind: &BTreeMap<String, String>,
	routes: &[String],
) -> Option<String> {
	for (key, names) in [
		("env", env.keys().collect::<Vec<_>>()),
		("secrets", secrets.iter().collect()),
		("bind", bind.keys().collect()),
	] {
		if let Some(variable) = names.iter().find(|variable| !env_name(variable)) {
			return Some(format!(
				"services.{name}.{key}: `{variable}` isn't an environment variable name; use \
				 uppercase letters, digits, and `_`"
			));
		}

		if let Some(variable) = names.iter().find(|variable| reserved_variable(variable)) {
			return Some(format!(
				"services.{name}.{key}: `{variable}` is set by `leptoskit` itself; pick another \
				 name"
			));
		}
	}

	if let Some(volume) = volumes.iter().find(|volume| !is_service_name(volume)) {
		return Some(format!(
			"services.{name}.volumes: `{volume}` must start with a lowercase letter and hold only \
			 lowercase letters, digits, and `-`"
		));
	}

	let mut seen = Vec::new();

	for volume in volumes {
		if seen.contains(&volume) {
			return Some(format!("services.{name}.volumes lists `{volume}` twice"));
		}

		seen.push(volume);
	}

	for (variable, value) in bind {
		if let Some(placeholder) = unknown_placeholder(value) {
			return Some(format!(
				"services.{name}.bind.{variable}: `{{{placeholder}}}` isn't a placeholder; use {}",
				BIND_PLACEHOLDERS
					.iter()
					.map(|known| format!("`{{{known}}}`"))
					.collect::<Vec<_>>()
					.join(", ")
			));
		}
	}

	for route in routes {
		if let Some(problem) = route_problem(route) {
			return Some(format!("services.{name}.routes: `{route}` {problem}"));
		}
	}

	None
}

/// The first `{…}` in `value` that isn't a known placeholder.
fn unknown_placeholder(value: &str) -> Option<String> {
	let mut rest = value;

	while let Some(start) = rest.find('{') {
		let after = &rest[start + 1..];
		let Some(end) = after.find('}') else {
			return Some(after.to_owned());
		};
		let placeholder = &after[..end];

		if !BIND_PLACEHOLDERS.contains(&placeholder) {
			return Some(placeholder.to_owned());
		}

		rest = &after[end + 1..];
	}

	None
}

/// What's wrong with one route prefix on its own.
fn route_problem(route: &str) -> Option<&'static str> {
	if !route.starts_with('/') {
		return Some("must start with `/`");
	}

	if route == "/" {
		return Some(
			"would send every request to the service; name a prefix such as `/_analytics`",
		);
	}

	if route.ends_with('/') {
		return Some("must not end with `/`");
	}

	if route.contains("//")
		|| route
			.chars()
			.any(|c| c.is_whitespace() || matches!(c, '?' | '#' | '*' | '{' | '}'))
	{
		return Some("must be a plain path, without `//`, spaces, `?`, `#`, `*`, or braces");
	}

	if RESERVED_PATHS
		.iter()
		.any(|reserved| overlaps(route, reserved))
	{
		return Some(
			"overlaps a path the app serves itself (`/pkg`, `/api`, `/live`, `/_leptoskit`)",
		);
	}

	None
}

/// Whether two path prefixes can match the same request path.
#[must_use]
pub fn overlaps(a: &str, b: &str) -> bool {
	let under = |path: &str, prefix: &str| {
		path == prefix
			|| path
				.strip_prefix(prefix)
				.is_some_and(|rest| rest.starts_with('/'))
	};
	under(a, b) || under(b, a)
}

/// What's wrong with the `[services]` tables on their own.
pub(crate) fn problem(
	services: &BTreeMap<String, ServiceConfig>,
	serve_port: u16,
) -> Option<String> {
	for (name, service) in services {
		if let Some(problem) = service.problem(name) {
			return Some(problem);
		}
	}

	// What the tables alone fix: their ports and routes. The manifests fill
	// in the rest, checked by `check_resolved` once they're read.
	let partial: Vec<ResolvedService> = services
		.iter()
		.filter_map(|(name, service)| ResolvedService::merge(name, service, None).ok())
		.collect();
	check_set(&partial, serve_port, false).err()
}

/// Checks the services once their manifests are merged in: each process
/// service has a port, ports are unique and aren't the app's, routes go with
/// `public` and don't overlap.
///
/// # Errors
///
/// The first problem found, worded for `leptoskit.toml`.
pub fn check_resolved(services: &[ResolvedService], serve_port: u16) -> Result<(), String> {
	for service in services {
		if let Some(problem) = settings_problem(
			&service.name,
			&service.env,
			&service.secrets,
			&service.volumes,
			&service.bind,
			&service.routes,
		) {
			return Err(problem);
		}

		if service.pool == Some(0) {
			return Err(format!(
				"services.{} has a pool of 0 connections; a pool needs at least one",
				service.name
			));
		}

		if service.source.is_process() && service.port.is_none() {
			return Err(format!(
				"services.{} has no port: set `port` in its table, or in the service's \
				 [package.metadata.leptoskit.service]",
				service.name
			));
		}

		if !service.source.is_process() && (service.database || !service.volumes.is_empty()) {
			return Err(format!(
				"services.{} is remote, so it can't have a database or volumes here",
				service.name
			));
		}
	}

	check_set(services, serve_port, true)
}

fn check_set(services: &[ResolvedService], serve_port: u16, complete: bool) -> Result<(), String> {
	let mut ports: Vec<(u16, &str)> = Vec::new();

	for service in services {
		if let Some(port) = service.port {
			if port == serve_port {
				return Err(format!(
					"services.{}.port is {port}, which the app serves on (serve.port)",
					service.name
				));
			}

			if let Some((_, other)) = ports.iter().find(|(taken, _)| *taken == port) {
				return Err(format!(
					"services.{} and services.{other} both use port {port}",
					service.name
				));
			}

			ports.push((port, &service.name));
		}

		if complete && !service.public && !service.routes.is_empty() {
			return Err(format!(
				"services.{} has `routes` but isn't public; set `public = true`",
				service.name
			));
		}
	}

	let mut prefixes: Vec<(&str, &str)> = Vec::new();

	for service in services {
		for route in &service.routes {
			if let Some((_, other)) = prefixes.iter().find(|(taken, _)| overlaps(taken, route)) {
				return Err(format!(
					"services.{}.routes: `{route}` overlaps a route of services.{other}",
					service.name
				));
			}

			prefixes.push((route, &service.name));
		}
	}

	Ok(())
}

#[cfg(test)]
mod tests {
	use super::*;

	fn table(raw: &str) -> ServiceConfig {
		toml::from_str(raw).expect("service table")
	}

	#[test]
	fn reads_each_source() {
		let source = |raw| table(raw).source("svc");
		assert_eq!(
			source("package = 'a'"),
			Ok(ServiceSource::Package { name: "a".into() })
		);
		assert_eq!(
			source("crate = 'a'\nversion = '1'"),
			Ok(ServiceSource::Crate {
				name: "a".into(),
				version: Some("1".into())
			})
		);
		assert_eq!(
			source("git = 'https://example.com/a'\ntag = 'v1'"),
			Ok(ServiceSource::Git {
				url: "https://example.com/a".into(),
				package: "svc".into(),
				reference: Some(GitReference::Tag("v1".into()))
			})
		);
		assert_eq!(
			source("git = 'https://example.com/a'\ncrate = 'b'"),
			Ok(ServiceSource::Git {
				url: "https://example.com/a".into(),
				package: "b".into(),
				reference: None
			})
		);
		assert_eq!(
			source("url = 'https://a.example'"),
			Ok(ServiceSource::Remote {
				url: "https://a.example".into()
			})
		);
	}

	#[test]
	fn rejects_missing_or_several_sources() {
		assert!(
			table("port = 1")
				.source("svc")
				.unwrap_err()
				.contains("needs a source")
		);
		assert!(
			table("package = 'a'\nurl = 'https://a.example'")
				.source("svc")
				.unwrap_err()
				.contains("`package` and `url`")
		);
		assert!(
			table("package = 'a'\nversion = '1'")
				.source("svc")
				.unwrap_err()
				.contains("goes with a registry `crate`")
		);
		assert!(
			table("package = 'a'\nrev = 'abc'")
				.source("svc")
				.unwrap_err()
				.contains("go with `git`")
		);
		assert!(
			table("git = 'https://e.x/a'\nrev = 'a'\ntag = 'b'")
				.source("svc")
				.unwrap_err()
				.contains("more than one")
		);
	}

	#[test]
	fn the_table_wins_and_maps_merge() {
		let manifest = ServiceManifest {
			name: Some("analytics".into()),
			port: Some(3100),
			public: Some(true),
			features: Some(vec!["service".into()]),
			env: [
				("A".into(), "manifest".into()),
				("B".into(), "manifest".into()),
			]
			.into(),
			bind: [("X".into(), "{url}".into())].into(),
			..ServiceManifest::default()
		};
		let config = table("package = 'a'\nport = 4000\nenv = { B = 'table' }");
		let merged = ResolvedService::merge("analytics", &config, Some(&manifest)).unwrap();
		assert_eq!(merged.port, Some(4000));
		assert!(merged.public);
		assert_eq!(merged.features, ["service"]);
		assert_eq!(merged.env["A"], "manifest");
		assert_eq!(merged.env["B"], "table");
		assert_eq!(merged.bind["X"], "{url}");
		assert_eq!(merged.public_prefixes(), ["/_leptoskit/services/analytics"]);
	}

	fn services(raw: &str) -> BTreeMap<String, ServiceConfig> {
		toml::from_str(raw).expect("services")
	}

	#[test]
	fn checks_tables_on_their_own() {
		let check = |raw| problem(&services(raw), 3000);
		assert_eq!(check("[a]\npackage = 'a'\nport = 3100"), None);
		assert!(check("[Bad]\npackage = 'a'").unwrap().contains("lowercase"));
		assert!(
			check("[a]\nurl = 'https://a.example'\nport = 1")
				.unwrap()
				.contains("remote")
		);
		assert!(
			check("[a]\nurl = 'ftp://a'")
				.unwrap()
				.contains("http or https")
		);
		assert!(
			check("[a]\npackage = 'a'\nport = 3000")
				.unwrap()
				.contains("serve.port")
		);
		assert!(
			check("[a]\npackage = 'a'\nport = 1\n[b]\npackage = 'b'\nport = 1")
				.unwrap()
				.contains("both use port 1")
		);
		assert!(
			check("[a]\npackage = 'a'\nroutes = ['/api/x']")
				.unwrap()
				.contains("overlaps a path the app serves")
		);
		assert!(
			check("[a]\npackage = 'a'\nroutes = ['/']")
				.unwrap()
				.contains("every request")
		);
		assert!(
			check("[a]\npackage = 'a'\nroutes = ['/x']\n[b]\npackage = 'b'\nroutes = ['/x/y']")
				.unwrap()
				.contains("overlaps a route of services.a")
		);
		assert!(
			check("[a]\npackage = 'a'\nenv = { lower = '1' }")
				.unwrap()
				.contains("isn't an environment variable")
		);
		assert!(
			check("[a]\npackage = 'a'\nsecrets = ['DATABASE_URL']")
				.unwrap()
				.contains("set by `leptoskit`")
		);
		assert!(
			check("[a]\npackage = 'a'\nbind = { X = '{nope}' }")
				.unwrap()
				.contains("`{nope}` isn't a placeholder")
		);
		assert!(
			check("[a]\npackage = 'a'\nvolumes = ['Data']")
				.unwrap()
				.contains("volumes")
		);
		// Without the app's `[db]`, a service's database is SQLite.
		assert!(problem(&services("[a]\npackage = 'a'\ndatabase = true"), 3000).is_none());
		assert!(
			problem(&services("[volume-x]\npackage = 'a'"), 3000)
				.unwrap()
				.contains("reserved")
		);
	}

	#[test]
	fn checks_merged_services() {
		let resolved = |raw: &str| ResolvedService::merge("a", &table(raw), None).expect("merged");
		assert!(
			check_resolved(&[resolved("package = 'a'")], 3000)
				.unwrap_err()
				.contains("has no port")
		);
		assert!(
			check_resolved(
				&[resolved("package = 'a'\nport = 1\nroutes = ['/x']")],
				3000
			)
			.unwrap_err()
			.contains("isn't public")
		);
		assert!(
			check_resolved(
				&[resolved(
					"package = 'a'\nport = 1\npublic = true\nroutes = ['/x']"
				)],
				3000
			)
			.is_ok()
		);
		assert!(check_resolved(&[resolved("url = 'https://a.example'")], 3000).is_ok());
	}

	#[test]
	fn trust_defaults_by_source_and_the_table_overrides_it() {
		let trust = |raw: &str| {
			ResolvedService::merge("a", &table(raw), None)
				.expect("merged")
				.trust
		};
		assert_eq!(trust("package = 'a'"), ServiceTrust::FirstParty);
		assert_eq!(trust("crate = 'a'"), ServiceTrust::Isolated);
		assert_eq!(trust("path = '../a'"), ServiceTrust::Isolated);
		assert_eq!(trust("url = 'https://a.example'"), ServiceTrust::Isolated);
		assert_eq!(
			trust("crate = 'a'\ntrust = 'first-party'"),
			ServiceTrust::FirstParty
		);
		assert_eq!(
			trust("package = 'a'\ntrust = 'isolated'"),
			ServiceTrust::Isolated
		);
	}

	#[test]
	fn the_manifest_must_name_the_service_the_table_does() {
		let named = |name: Option<&str>| {
			ServiceManifest {
				name: name.map(str::to_owned),
				..ServiceManifest::default()
			}
		};
		let config = table("package = 'a'");
		assert!(
			ResolvedService::merge("analytics", &config, Some(&named(Some("analytics")))).is_ok()
		);
		let wrong =
			ResolvedService::merge("stats", &config, Some(&named(Some("analytics")))).unwrap_err();
		assert!(wrong.contains("[services.analytics]"), "{wrong}");
		let missing = ResolvedService::merge("analytics", &config, Some(&named(None))).unwrap_err();
		assert!(missing.contains("has no `name`"), "{missing}");
		// A service without a manifest, such as a remote one, has nothing to
		// match.
		assert!(
			ResolvedService::merge("billing", &table("url = 'https://b.example'"), None).is_ok()
		);
	}

	#[test]
	fn pool_comes_from_the_manifest_unless_the_table_sets_it() {
		let manifest = ServiceManifest {
			name: Some("a".into()),
			pool: Some(4),
			..ServiceManifest::default()
		};
		let merged = |raw: &str| {
			ResolvedService::merge("a", &table(raw), Some(&manifest))
				.expect("merged")
				.pool
		};
		assert_eq!(merged("package = 'a'"), Some(4));
		assert_eq!(merged("package = 'a'\npool = 16"), Some(16));
		assert!(
			problem(&services("[a]\npackage = 'a'\npool = 0"), 3000)
				.unwrap()
				.contains("pool is 0")
		);
		assert!(
			problem(&services("[a]\nurl = 'https://a.example'\npool = 2"), 3000)
				.unwrap()
				.contains("`pool` has no meaning")
		);
		assert_eq!(
			problem(
				&services("[a]\nurl = 'https://a.example'\ntrust = 'first-party'"),
				3000
			),
			None
		);
	}

	#[test]
	fn overlap_is_by_segment() {
		assert!(overlaps("/a", "/a/b"));
		assert!(overlaps("/a", "/a"));
		assert!(!overlaps("/a", "/ab"));
		assert_eq!(env_segment("web-search"), "WEB_SEARCH");
	}
}
