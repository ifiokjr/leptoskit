//! Linux hosts behind Cloudflare, with managed or independently supervised
//! services.

use serde::Deserialize;
use serde::Serialize;

/// Infrastructure intent. Credentials are supplied by the operator's
/// environment.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct InfraConfig {
	/// Provision hosts and edge routing for an independently supervised
	/// service. Omitting this keeps the managed Postgres/Kamal release preset.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub external: Option<InfraExternal>,
	/// Optional self-hosted deployment service. Credentials are kept outside
	/// the repository; ordinary `lk deploy` uses its durable operation queue.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub control_url: Option<String>,
	/// Use the hosted service's locking HTTP state backend. Credentials stay on
	/// the worker.
	#[serde(default, skip_serializing_if = "std::ops::Not::not")]
	pub hosted_state: bool,
	/// Compute provider. Hetzner is the default; sizes and locations use the
	/// selected provider's own names.
	#[serde(default)]
	pub compute: ComputeProvider,
	/// Application hosts. The first owns persistent services; the others are
	/// stateless application replicas.
	#[serde(default = "one")]
	pub instances: u8,
	/// Optional shared cache and live relay.
	#[serde(default)]
	pub services: InfraServices,
	/// The managed Postgres server's connection limit, and the pool size the
	/// app's server opens, for the connection budget `lk infra plan` and
	/// `lk doctor` check.
	#[serde(default, skip_serializing_if = "InfraPostgres::is_default")]
	pub postgres: InfraPostgres,
	/// Versioned optional infrastructure capabilities. No plugin executes
	/// arbitrary downloaded code or gets provider credentials at runtime.
	#[serde(default)]
	pub plugins: InfraPlugins,
	/// Encrypted backup scheduling, verification, and retention.
	#[serde(default)]
	pub backups: InfraBackups,
	/// When present, provision the private backup bucket in Backblaze B2.
	/// Signed update delivery remains on R2 behind Cloudflare.
	#[serde(default)]
	pub b2: Option<InfraB2>,
	/// Optional live-price guard on compute and primary IPv4, excluding usage,
	/// tax, storage, domains, and other services. This is not a billing cap.
	#[serde(default)]
	pub budget: Option<InfraBudget>,
	/// Resource-name prefix; use a distinct prefix for each environment.
	pub name: String,
	/// Public application hostname on the selected Cloudflare zone.
	pub domain: String,
	/// Provider location, for example Hetzner `hel1` or `DigitalOcean` `ams3`.
	pub location: String,
	/// Provider size, for example `cx23`. No automatic paid fallback.
	pub server_type: String,
	/// Existing provider SSH key IDs to authorize on the host.
	pub ssh_key_ids: Vec<u64>,
	/// Administrator source CIDRs permitted to SSH; no world-open rule.
	pub ssh_cidrs: Vec<String>,
	/// Cloudflare account ID, not a credential.
	pub account_id: String,
	/// Cloudflare DNS zone ID, not a credential.
	pub zone_id: String,
	/// Public hostname for signed code-push artifacts in R2.
	#[serde(default, skip_serializing_if = "String::is_empty")]
	pub updates_domain: String,
	/// Private R2 or B2 bucket for encrypted database backups.
	#[serde(default, skip_serializing_if = "String::is_empty")]
	pub backup_bucket: String,
	/// R2 bucket exposed only for signed release artifacts.
	#[serde(default, skip_serializing_if = "String::is_empty")]
	pub updates_bucket: String,
	/// Image repository without a tag, for example `ghcr.io/team/app`.
	#[serde(default, skip_serializing_if = "String::is_empty")]
	pub image: String,
	/// Registry hostname, for example `ghcr.io`.
	#[serde(default, skip_serializing_if = "String::is_empty")]
	pub registry: String,
	/// Registry login name; password is `KAMAL_REGISTRY_PASSWORD`.
	#[serde(default, skip_serializing_if = "String::is_empty")]
	pub registry_username: String,
	/// Additional application secret environment names.
	#[serde(default)]
	pub secrets: Vec<String>,
	/// Optional S3 remote-state backend. Local state is private and gitignored.
	#[serde(default)]
	pub state: Option<InfraState>,
}

/// A single host whose application lifecycle is managed outside Kamal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct InfraExternal {
	/// Cloudflare Tunnel connects to this port on the host's loopback address.
	#[cfg_attr(feature = "schema", schemars(range(min = 1, max = 65535)))]
	pub origin_port: u16,
}

const fn one() -> u8 {
	1
}

/// Supported virtual-machine APIs.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "lowercase")]
pub enum ComputeProvider {
	/// Hetzner Cloud, using `HCLOUD_TOKEN`.
	#[default]
	Hetzner,
	/// `DigitalOcean`, using `DIGITALOCEAN_TOKEN`.
	Digitalocean,
}

/// Services run on the primary host and are reachable only on its private
/// network.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields, default)]
pub struct InfraServices {
	/// Provision a bounded Valkey cache. Binds `VALKEY_URL` in the app.
	pub cache: bool,
	/// Provision NATS `JetStream` and bind the app's configured relay variable.
	/// Required for several application hosts to share live channels/presence.
	pub relay: bool,
}

/// The managed Postgres server's connection budget.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields, default)]
pub struct InfraPostgres {
	/// Postgres's `max_connections`. The server starts with this limit, and
	/// the app's and its services' pools must fit inside it.
	#[cfg_attr(feature = "schema", schemars(range(min = 20, max = 10000)))]
	pub max_connections: u32,
	/// The most connections the app server's pool opens: `PoolOptions`'s
	/// `max_connections` in its `main`. `leptoskit_db`'s default is 8.
	#[cfg_attr(feature = "schema", schemars(range(min = 1)))]
	pub app_pool: u32,
}

impl InfraPostgres {
	/// Postgres's own default limit.
	pub const DEFAULT_MAX_CONNECTIONS: u32 = 100;
	/// `leptoskit_db`'s default pool size.
	pub const DEFAULT_POOL: u32 = 8;

	// Two `u32`s are a trivial copy only where pointers are 64 bits; on wasm32
	// the lint doesn't fire.
	#[cfg_attr(
		target_pointer_width = "64",
		expect(
			clippy::trivially_copy_pass_by_ref,
			reason = "serde's skip_serializing_if passes a reference"
		)
	)]
	fn is_default(&self) -> bool {
		*self == Self::default()
	}

	fn problem(self) -> Option<String> {
		(!(20..=10_000).contains(&self.max_connections) || self.app_pool == 0).then(|| {
			"infra.postgres.max_connections must be between 20 and 10000, and \
			 infra.postgres.app_pool at least 1"
				.to_owned()
		})
	}
}

impl Default for InfraPostgres {
	fn default() -> Self {
		Self {
			max_connections: Self::DEFAULT_MAX_CONNECTIONS,
			app_pool: Self::DEFAULT_POOL,
		}
	}
}

/// Built-in infrastructure plugins. Unknown plugin names fail parsing;
/// enabling a capability must be an intentional configuration change.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields, default)]
pub struct InfraPlugins {
	pub analytics: Option<InfraAnalytics>,
}

/// Bounded, in-process analytics. Requires the application's `analytics`
/// feature and the `LeptosKit` server entrypoint (or an explicit Axum mount).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields, default)]
pub struct InfraAnalytics {
	/// Optional OTLP/HTTP metrics endpoint. Omitted: use the connected
	/// deployment service, when present. No separate collector is required.
	pub otlp_endpoint: Option<String>,
	/// Plugin configuration contract, independent of the CLI release.
	#[cfg_attr(feature = "schema", schemars(range(min = 1, max = 1)))]
	pub version: u32,
	pub environment: String,
	/// Completed and current minute windows retained in memory.
	#[cfg_attr(feature = "schema", schemars(range(min = 1, max = 60)))]
	pub retained_windows: usize,
	/// Maximum aggregate series across all sources; some capacity is reserved
	/// for backend measurements.
	#[cfg_attr(feature = "schema", schemars(range(min = 1, max = 512)))]
	pub max_series: usize,
	/// Explicit app route templates accepted from the browser SDK. Empty
	/// disables browser ingestion without disabling backend analytics.
	#[cfg_attr(feature = "schema", schemars(length(max = 32)))]
	pub browser_routes: Vec<String>,
}

impl Default for InfraAnalytics {
	fn default() -> Self {
		Self {
			otlp_endpoint: None,
			version: 1,
			environment: "production".into(),
			retained_windows: 15,
			max_series: 128,
			browser_routes: Vec::new(),
		}
	}
}

impl InfraAnalytics {
	#[must_use]
	pub fn problem(&self) -> Option<String> {
		if self
			.otlp_endpoint
			.as_deref()
			.is_some_and(|value| !export_endpoint(value))
		{
			return Some(
				"infra.plugins.analytics.otlp_endpoint needs an HTTP(S) endpoint without URL \
				 credentials, query or fragment"
					.into(),
			);
		}
		let valid = |s: &str| {
			!s.is_empty()
				&& s.len() <= 128
				&& s.bytes()
					.all(|b| b.is_ascii_alphanumeric() || b"/:-_.*{}".contains(&b))
		};
		if self.version != 1 {
			return Some("infra.plugins.analytics.version must be 1".into());
		}
		if !valid(&self.environment)
			|| !(1..=60).contains(&self.retained_windows)
			|| !(1..=512).contains(&self.max_series)
			|| self.browser_routes.len() > 32
			|| self.browser_routes.iter().any(|s| !valid(s))
		{
			return Some(
				"invalid analytics environment, retention, series budget or browser route \
				 templates"
					.into(),
			);
		}
		None
	}
}

/// Restic encrypts database archives and release metadata before upload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields, default)]
pub struct InfraBackups {
	/// Install and enable the backup timer on the primary host.
	pub enabled: bool,
	/// A systemd calendar expression, validated on the host before activation.
	pub schedule: String,
	/// Daily snapshots to retain, after a successful verified backup.
	pub daily: u16,
	/// Weekly snapshots to retain.
	pub weekly: u16,
	/// Monthly snapshots to retain.
	pub monthly: u16,
	/// Status reports older successful backups as unhealthy.
	pub max_age_hours: u16,
}

impl Default for InfraBackups {
	fn default() -> Self {
		Self {
			enabled: true,
			schedule: "daily".into(),
			daily: 7,
			weekly: 4,
			monthly: 3,
			max_age_hours: 36,
		}
	}
}

/// B2's region determines its S3-compatible endpoint.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct InfraB2 {
	/// For example `us-west-004`. Bucket provisioning uses B2 credentials;
	/// the backup agent receives a separate bucket-scoped S3 key.
	pub region: String,
}

/// Refuse plan/apply when the selected servers exceed this base-price budget.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct InfraBudget {
	/// Currency returned by the provider, for example EUR or USD; no FX
	/// conversion.
	pub currency: String,
	/// Maximum monthly compute plus primary IPv4 price for all app hosts.
	pub monthly_compute: f64,
}

/// A separately bootstrapped, locking-capable S3 state backend.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct InfraState {
	/// Existing state bucket. Do not use the application backups bucket.
	pub bucket: String,
	/// Unique object key for this environment.
	pub key: String,
	/// State bucket's AWS region.
	pub region: String,
}

impl InfraConfig {
	fn ssh_problem(&self) -> Option<String> {
		if self.ssh_key_ids.is_empty() || self.ssh_cidrs.is_empty() {
			return Some("infra requires existing ssh_key_ids and administrator ssh_cidrs".into());
		}
		for cidr in &self.ssh_cidrs {
			let valid = cidr.split_once('/').is_some_and(|(ip, bits)| {
				match (ip.parse::<std::net::IpAddr>(), bits.parse::<u8>()) {
					(Ok(ip), Ok(bits)) => bits > 0 && bits <= if ip.is_ipv4() { 32 } else { 128 },
					_ => false,
				}
			});
			if !valid {
				return Some("infra.ssh_cidrs requires valid CIDRs with nonzero prefixes".into());
			}
		}
		None
	}

	fn external_problem(&self) -> Option<String> {
		if let Some(external) = &self.external {
			if external.origin_port == 0 {
				return Some("infra.external.origin_port must be between 1 and 65535".into());
			}
			if self.instances != 1
				|| self.services.cache
				|| self.services.relay
				|| self.plugins.analytics.is_some()
				|| self.backups.enabled
				|| self.control_url.is_some()
				|| self.hosted_state
				|| !self.secrets.is_empty()
			{
				return Some(
					"infra.external provisions one host; manage its services, application \
					 secrets, backups and releases separately, without a hosted worker or runtime \
					 plugins"
						.into(),
				);
			}
			if [
				&self.updates_domain,
				&self.updates_bucket,
				&self.image,
				&self.registry,
				&self.registry_username,
			]
			.iter()
			.any(|value| !value.is_empty())
			{
				return Some(
					"infra.external does not use image, registry or code-push settings".into(),
				);
			}
			if self.b2.is_some() && self.backup_bucket.is_empty() {
				return Some("infra.b2 requires a backup_bucket".into());
			}
		}
		None
	}

	fn plugins_problem(&self) -> Option<String> {
		if self
			.control_url
			.as_deref()
			.is_some_and(|url| !control_origin(url))
		{
			return Some("infra.control_url must use HTTPS or localhost HTTP".into());
		}
		self.plugins
			.analytics
			.as_ref()
			.and_then(InfraAnalytics::problem)
	}

	/// A configuration problem, before generation or external commands run.
	pub fn problem(&self) -> Option<String> {
		if let Some(problem) = self.external_problem() {
			return Some(problem);
		}
		if self.hosted_state && self.state.is_some() {
			return Some("choose hosted_state or infra.state, not both".into());
		}
		if let Some(problem) = self.plugins_problem().or_else(|| self.postgres.problem()) {
			return Some(problem);
		}
		let safe = |text: &str| {
			!text.is_empty()
				&& text
					.bytes()
					.all(|c| c.is_ascii_alphanumeric() || b"-._/".contains(&c))
		};
		if self.budget.as_ref().is_some_and(|b| {
			!b.monthly_compute.is_finite()
				|| b.monthly_compute <= 0.0
				|| b.currency.len() != 3
				|| !b.currency.bytes().all(|b| b.is_ascii_uppercase())
		}) {
			return Some(
				"infra.budget requires a positive monthly_compute and three-letter uppercase \
				 currency"
					.into(),
			);
		}
		if !(1..=16).contains(&self.instances) {
			return Some("infra.instances must be between 1 and 16".into());
		}
		if self.instances > 1 && !self.services.relay {
			return Some(
				"several hosts require infra.services.relay so live presence and signals are \
				 shared"
					.into(),
			);
		}
		if self.backups.daily == 0
			|| self.backups.max_age_hours == 0
			|| self.backups.schedule.is_empty()
			|| !self
				.backups
				.schedule
				.bytes()
				.all(|b| b.is_ascii_alphanumeric() || b" *,:./-".contains(&b))
		{
			return Some(
				"backups need a calendar schedule, at least one daily snapshot, and a positive \
				 maximum age"
					.into(),
			);
		}
		if self
			.b2
			.as_ref()
			.is_some_and(|b2| !safe(&b2.region) || b2.region.contains(['/', '.']))
		{
			return Some("infra.b2.region must be a B2 region name".into());
		}
		if self.server_type.starts_with("cax") {
			return Some(
				"the managed preset uses amd64 images; select an x86 Hetzner server type".into(),
			);
		}
		if !safe(&self.name) || self.name.contains(['/', '.']) || self.name.starts_with('-') {
			return Some(
				"infra.name must contain only letters, digits, hyphens, and underscores".into(),
			);
		}
		for (key, value) in [
			("location", &self.location),
			("server_type", &self.server_type),
			("image", &self.image),
			("registry", &self.registry),
			("registry_username", &self.registry_username),
		] {
			if self.external.is_some() && matches!(key, "image" | "registry" | "registry_username")
			{
				continue;
			}
			if !safe(value) {
				return Some(format!("infra.{key} contains unsupported characters"));
			}
		}
		for (key, value) in [
			("domain", &self.domain),
			("updates_domain", &self.updates_domain),
		] {
			if self.external.is_some() && key == "updates_domain" {
				continue;
			}
			if !value.contains('.')
				|| value.split('.').any(|label| {
					label.is_empty()
						|| label.starts_with('-')
						|| label.ends_with('-')
						|| !label
							.bytes()
							.all(|c| c.is_ascii_alphanumeric() || c == b'-')
				}) {
				return Some(format!("infra.{key} must be a DNS hostname"));
			}
		}
		if self.external.is_none()
			&& (self.domain == self.updates_domain || self.backup_bucket == self.updates_bucket)
		{
			return Some(
				"application/update hostnames and backup/update buckets must be distinct".into(),
			);
		}
		for value in [&self.account_id, &self.zone_id] {
			if value.len() != 32 || !value.bytes().all(|c| c.is_ascii_hexdigit()) {
				return Some(
					"infra.account_id and zone_id must be 32 hexadecimal characters".into(),
				);
			}
		}
		for value in [&self.backup_bucket, &self.updates_bucket] {
			if self.external.is_some() && value.is_empty() {
				continue;
			}
			if !(3..=63).contains(&value.len())
				|| !value
					.bytes()
					.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-')
			{
				return Some(
					"infra bucket names must be 3–63 lowercase letters, digits, or hyphens".into(),
				);
			}
		}
		if let Some(problem) = self.ssh_problem() {
			return Some(problem);
		}

		if self.secrets.iter().any(|name| !env_name(name)) {
			return Some("infra.secrets entries must be environment variable names".into());
		}
		if let Some(state) = &self.state
			&& ([&state.bucket, &state.key, &state.region]
				.iter()
				.any(|s| !safe(s))
				|| state.bucket == self.backup_bucket
				|| state.bucket == self.updates_bucket)
		{
			return Some(
				"infra.state needs a distinct existing bucket, object key, and region".into(),
			);
		}
		None
	}
}

/// Whether a string can name an environment variable in generated bindings.
pub fn env_name(name: &str) -> bool {
	let mut bytes = name.bytes();
	matches!(bytes.next(), Some(b'A'..=b'Z' | b'_'))
		&& bytes.all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == b'_')
}

fn control_origin(url: &str) -> bool {
	let Ok(parsed) = url::Url::parse(url) else {
		return false;
	};
	export_endpoint(url)
		&& parsed.path() == "/"
		&& (parsed.scheme() == "https"
			|| matches!(parsed.host_str(), Some("localhost" | "127.0.0.1" | "[::1]")))
}

fn export_endpoint(value: &str) -> bool {
	let Ok(url) = url::Url::parse(value) else {
		return false;
	};
	matches!(url.scheme(), "http" | "https")
		&& url.host_str().is_some()
		&& url.username().is_empty()
		&& url.password().is_none()
		&& url.query().is_none()
		&& url.fragment().is_none()
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn plugin_versions_limits_and_route_names_are_checked() {
		let mut plugin = InfraAnalytics::default();
		assert!(plugin.problem().is_none());
		for endpoint in [
			"https://key:secret@example.com/v1/metrics",
			"https://example.com/v1/metrics?key=secret",
			"file:///tmp/collector",
		] {
			plugin.otlp_endpoint = Some(endpoint.into());
			assert!(plugin.problem().is_some());
		}
		plugin.otlp_endpoint = None;
		plugin.version = 2;
		assert!(plugin.problem().is_some());
		plugin.version = 1;
		plugin.browser_routes = vec!["/notes/{id}".into()];
		assert!(plugin.problem().is_none());
		plugin.browser_routes = vec!["/notes?token=secret".into()];
		assert!(plugin.problem().is_some());
		plugin.browser_routes.clear();
		plugin.retained_windows = 61;
		assert!(plugin.problem().is_some());
		assert!(toml::from_str::<InfraPlugins>("unknown = {} ").is_err());
		assert!(
			toml::from_str::<InfraPlugins>("[analytics]\ncustom_executable='plugin.sh'").is_err()
		);
	}
}
