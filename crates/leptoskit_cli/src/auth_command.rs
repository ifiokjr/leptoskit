//! `leptoskit auth`: checking sign-in's configuration, rotating its keys,
//! and adding third-party providers.

use std::fmt::Write as _;
use std::path::Path;

use anyhow::Context;
use anyhow::Result;
use anyhow::bail;
use leptoskit_config::LeptoskitConfig;
use leptoskit_config::Platform;

/// The `leptoskit auth` subcommands.
#[derive(Debug, clap::Subcommand)]
pub enum AuthCommand {
	/// Checks `[auth]`: that every secret's environment variable is set
	/// (never printing it), the origin and passkey relying party, where
	/// providers send people back, and how old each provider's definition
	/// is. Exits non-zero when something would fail.
	Doctor {
		/// Checks for a release build: `http` origins, including those a
		/// native app may reach, and the development mailbox are errors.
		#[arg(long)]
		release: bool,
	},
	/// Re-encrypts stored secrets under the newest key in the keys variable,
	/// so older keys can be retired.
	RotateKeys {
		/// The database URL; defaults to the variable `db.url_env` names.
		#[arg(long)]
		url: Option<String>,
	},
	/// Manages third-party sign-in providers.
	Provider {
		#[command(subcommand)]
		command: ProviderCommand,
	},
}

/// The `leptoskit auth provider` subcommands.
#[derive(Debug, clap::Subcommand)]
pub enum ProviderCommand {
	/// Adds `[auth.providers.<id>]` to leptoskit.toml, naming the
	/// environment variables its secrets go in, and prints how to set the
	/// provider up.
	Add {
		/// `google`, `apple`, `microsoft`, `github`, or your own ID for an
		/// OpenID Connect issuer, such as `company`.
		id: String,
		/// For an OpenID Connect provider: its issuer, such as
		/// `https://sso.example.com`.
		#[arg(long)]
		issuer: Option<String>,
	},
}

/// Runs a `leptoskit auth` subcommand.
pub fn run(command: AuthCommand) -> Result<()> {
	let (config, root) = LeptoskitConfig::load()?;

	match command {
		AuthCommand::Doctor { release } => {
			let findings = doctor(&config, |name| std::env::var(name).ok(), release);
			print!("{}", report(&findings));

			if findings.iter().any(|finding| finding.level == Level::Error) {
				bail!(
					"sign-in isn't ready{}",
					if release { " for release" } else { "" }
				);
			}

			Ok(())
		}
		AuthCommand::RotateKeys { url } => rotate_keys(&config, url),
		AuthCommand::Provider {
			command: ProviderCommand::Add { id, issuer },
		} => {
			let output = add_provider(&root, &config, &id, issuer.as_deref())?;
			print!("{output}");
			Ok(())
		}
	}
}

/// How serious a [`Finding`] is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Level {
	Ok,
	Warning,
	Error,
}

/// One thing `doctor` found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Finding {
	pub level: Level,
	pub message: String,
}

fn ok(message: impl Into<String>) -> Finding {
	Finding {
		level: Level::Ok,
		message: message.into(),
	}
}

fn warning(message: impl Into<String>) -> Finding {
	Finding {
		level: Level::Warning,
		message: message.into(),
	}
}

fn error(message: impl Into<String>) -> Finding {
	Finding {
		level: Level::Error,
		message: message.into(),
	}
}

/// Checks sign-in's configuration, reading environment variables through
/// `env`. It only ever says whether a variable is set, never its value.
pub(crate) fn doctor(
	config: &LeptoskitConfig,
	env: impl Fn(&str) -> Option<String>,
	release: bool,
) -> Vec<Finding> {
	let Some(auth) = &config.auth else {
		return vec![error(
			"there's no [auth] table in leptoskit.toml; `leptoskit generate auth` adds one",
		)];
	};
	let mut findings = Vec::new();
	let serious = |message: String| {
		if release {
			error(message)
		} else {
			warning(message)
		}
	};

	match env(&auth.keys_env).map(|keys| leptoskit_auth::Keys::parse(&keys)) {
		None => findings.push(error(format!("{} isn't set", auth.keys_env))),
		Some(Err(problem)) => {
			findings.push(error(format!("{} is invalid: {problem}", auth.keys_env)));
		}
		Some(Ok(keys)) => {
			let ids = keys.ids();
			findings.push(ok(format!(
				"{} holds {} key{}; `{}` encrypts new secrets",
				auth.keys_env,
				ids.len(),
				if ids.len() == 1 { "" } else { "s" },
				keys.newest()
			)));

			if ids.len() > 1 {
				findings.push(ok(
					"after rotating, `leptoskit auth rotate-keys` re-encrypts what's stored"
				));
			}
		}
	}

	match &auth.origin {
		None => {
			findings.push(serious(
				"[auth] origin isn't set: passkeys and third-party sign-in need it, and a release \
				 should name its own"
					.into(),
			));
		}
		Some(origin) => {
			let host = url::Url::parse(origin)
				.ok()
				.and_then(|url| url.host_str().map(str::to_owned))
				.unwrap_or_default();
			findings.push(ok(format!(
				"the origin is {origin}; passkeys belong to `{host}`"
			)));

			if origin.starts_with("http://") {
				findings.push(serious(format!(
					"the origin {origin} is plain http, which only local development may use"
				)));
			}
		}
	}

	findings.extend(mail(&auth.mail, &env, release));
	findings.extend(native_origins(config, release));
	findings.extend(providers(config, &env));

	if !auth.required_for.is_empty() {
		findings.push(ok(format!(
			"two-step verification is required for {}; the server must read roles with \
			 `AuthBuilder::roles`",
			auth.required_for.join(", ")
		)));
	}

	findings
}

fn mail(
	mail: &leptoskit_config::AuthMailConfig,
	env: &impl Fn(&str) -> Option<String>,
	release: bool,
) -> Vec<Finding> {
	let names = [
		&mail.smtp_url_env,
		&mail.webhook_url_env,
		&mail.webhook_secret_env,
	];
	let mut findings: Vec<Finding> = names
		.into_iter()
		.flatten()
		.map(|name| {
			if env(name).is_some() {
				ok(format!("{name} is set"))
			} else {
				error(format!("{name} isn't set"))
			}
		})
		.collect();

	if mail.smtp_url_env.is_none() && mail.webhook_url_env.is_none() {
		let message = "email goes to the development mailbox; a release build refuses to start \
		               without smtp_url_env or webhook_url_env in [auth.mail]";
		findings.push(if release { error(message) } else { ok(message) });
	}

	findings
}

/// The origins a native app's HTTP plugin may reach. A release that still
/// lists `http` ones sends tokens in the clear to whatever answers.
fn native_origins(config: &LeptoskitConfig, release: bool) -> Vec<Finding> {
	let origins = config
		.native
		.plugin_options
		.get("http")
		.and_then(|options| options.get("origins"))
		.and_then(|origins| origins.as_array())
		.map(|origins| {
			origins
				.iter()
				.filter_map(|origin| origin.as_str())
				.filter(|origin| origin.starts_with("http://"))
				.map(str::to_owned)
				.collect::<Vec<_>>()
		})
		.unwrap_or_default();

	if origins.is_empty() {
		return Vec::new();
	}

	let message = format!(
		"[native.plugin-options.http] origins lists plain http ({}); native apps would send \
		 tokens unencrypted. Keep them for development only",
		origins.join(", ")
	);
	vec![if release {
		error(message)
	} else {
		warning(message)
	}]
}

fn providers(config: &LeptoskitConfig, env: &impl Fn(&str) -> Option<String>) -> Vec<Finding> {
	let Some(auth) = &config.auth else {
		return Vec::new();
	};
	let mut findings = Vec::new();
	let origin = auth.origin.as_deref().unwrap_or("http://localhost:3000");
	let today = chrono::Utc::now().date_naive();

	for (id, provider) in &auth.providers {
		let kind = provider.kind(id);
		findings.push(ok(format!(
			"{id} ({kind}) sends people back to {origin}/auth/callback/{id}"
		)));

		for (setting, name) in provider.env_names() {
			if env(name).is_none() {
				findings.push(error(format!("{id}: {name} ({setting}) isn't set")));
			}
		}

		let verified = leptoskit_auth::PROVIDERS_VERIFIED
			.iter()
			.find(|(provider, _)| *provider == kind)
			.and_then(|(_, date)| chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d").ok());

		if let Some(date) = verified
			&& (today - date).num_days() > 365
		{
			findings.push(warning(format!(
				"{id}: the built-in {kind} definition was last checked on {date}; check its setup \
				 page for changes"
			)));
		}
	}

	let third_party = auth
		.providers
		.iter()
		.any(|(id, provider)| provider.kind(id) != "apple");
	let apple = auth
		.providers
		.iter()
		.any(|(id, provider)| provider.kind(id) == "apple");
	let ios = config.app.platforms.contains(&Platform::Ios);

	if ios && third_party && !apple {
		findings.push(warning(
			"the App Store expects an app with third-party sign-in to offer an equivalent \
			 privacy-focused option (App Review guideline 4.8); consider `leptoskit auth provider \
			 add apple`",
		));
	}

	findings
}

/// The findings, one per line.
pub(crate) fn report(findings: &[Finding]) -> String {
	let mut out = String::new();

	for finding in findings {
		let mark = match finding.level {
			Level::Ok => "ok",
			Level::Warning => "warning",
			Level::Error => "error",
		};
		let _ = writeln!(out, "{mark:>7}  {}", finding.message);
	}

	out
}

/// Re-encrypts stored secrets under the newest key.
fn rotate_keys(config: &LeptoskitConfig, url: Option<String>) -> Result<()> {
	let auth = config
		.auth
		.as_ref()
		.context("there's no [auth] table in leptoskit.toml")?;
	let db = config
		.db
		.as_ref()
		.context("sign-in keeps its secrets in the database; add a [db] table")?;
	let url = match url {
		Some(url) => url,
		None => {
			std::env::var(&db.url_env)
				.with_context(|| format!("set {} to the database URL, or pass --url", db.url_env))?
		}
	};
	let keys = leptoskit_auth::Keys::from_env(&auth.keys_env)?;
	let newest = keys.newest().to_owned();
	let runtime = tokio::runtime::Builder::new_multi_thread()
		.enable_all()
		.build()
		.context("could not start the async runtime")?;
	let rotated = runtime.block_on(async {
		let db = leptoskit_db::Db::connect(&url).await?;
		rotate_in(&db, keys).await
	})?;
	print!("{}", rotation_report(&newest, rotated));
	Ok(())
}

/// Re-encrypts what `db` stores under `keys`' newest.
async fn rotate_in(
	db: &leptoskit_db::Db,
	keys: leptoskit_auth::Keys,
) -> Result<leptoskit_auth::Rotated> {
	let auth = leptoskit_auth::Auth::builder(db, keys)
		.mailer(leptoskit_auth::Mailbox::new())
		.build()?;
	Ok(auth.rotate_keys().await?)
}

pub(crate) fn rotation_report(newest: &str, rotated: leptoskit_auth::Rotated) -> String {
	format!(
		"Re-encrypted under `{newest}`: {} authenticator secret(s) and {} sign-in(s) in progress; \
		 {} already were.\n\nRecovery codes and emailed codes are hashes, which can't be \
		 re-keyed: keep the older keys listed until people with recovery codes have made new \
		 ones, or accept that their old codes stop working.\n",
		rotated.totp, rotated.ceremonies, rotated.current
	)
}

/// What a built-in provider needs: its variables, its kind's extra
/// settings, and how to set it up.
struct Recipe {
	settings: Vec<(&'static str, String)>,
	steps: Vec<String>,
}

fn recipe(id: &str, issuer: Option<&str>, redirects: &str) -> Result<Recipe> {
	let upper = id.to_ascii_uppercase().replace('-', "_");
	let pair = || {
		vec![
			("client_id_env", format!("{upper}_CLIENT_ID")),
			("client_secret_env", format!("{upper}_CLIENT_SECRET")),
		]
	};
	let set = |names: &[(&str, String)]| {
		let names: Vec<&str> = names
			.iter()
			.filter(|(setting, _)| setting.ends_with("_env"))
			.map(|(_, name)| name.as_str())
			.collect();
		format!(
			"Set the environment variables (for example through Monosecret): {}",
			names.join(", ")
		)
	};

	Ok(match id {
		"google" => {
			let settings = pair();
			Recipe {
				steps: vec![
					"Open https://console.cloud.google.com/apis/credentials and create an \"OAuth \
					 client ID\" of type \"Web application\"."
						.into(),
					format!("Add these Authorized redirect URIs:\n{redirects}"),
					set(&settings),
				],
				settings,
			}
		}
		"github" => {
			let settings = pair();
			Recipe {
				steps: vec![
					"Open https://github.com/settings/developers and create a \"New OAuth App\"."
						.into(),
					format!("Set its Authorization callback URL to one of:\n{redirects}"),
					"Generate a client secret.".into(),
					set(&settings),
				],
				settings,
			}
		}
		"microsoft" => {
			let mut settings = pair();
			settings.push(("tenant", "common".into()));
			Recipe {
				steps: vec![
					"Open https://entra.microsoft.com, go to App registrations, and create a \
					 registration of type \"Web\"."
						.into(),
					format!("Add these redirect URIs:\n{redirects}"),
					"Under Certificates & secrets, create a client secret. `tenant` is `common`; \
					 use `consumers`, `organizations`, or your tenant's ID to narrow it."
						.into(),
					set(&settings),
				],
				settings,
			}
		}
		"apple" => {
			let settings = vec![
				("client_id_env", "APPLE_SERVICES_ID".to_owned()),
				("team_id_env", "APPLE_TEAM_ID".to_owned()),
				("key_id_env", "APPLE_KEY_ID".to_owned()),
				("private_key_env", "APPLE_PRIVATE_KEY".to_owned()),
			];
			Recipe {
				steps: vec![
					"In https://developer.apple.com/account/resources, create a Services ID and \
					 turn on Sign in with Apple for it."
						.into(),
					format!("Add the site's domain and these Return URLs:\n{redirects}"),
					"Create a key with Sign in with Apple, and download its .p8 file once. \
					 APPLE_PRIVATE_KEY holds its contents."
						.into(),
					set(&settings),
				],
				settings,
			}
		}
		_ => {
			let Some(issuer) = issuer else {
				bail!(
					"`{id}` isn't a built-in provider; pass --issuer with its OpenID Connect \
					 issuer, such as https://sso.example.com"
				);
			};
			let mut settings = vec![("issuer", issuer.trim_end_matches('/').to_owned())];
			settings.extend(pair());
			Recipe {
				steps: vec![
					format!("Register a web application with {issuer}."),
					format!("Allow these redirect URIs:\n{redirects}"),
					set(&settings),
				],
				settings,
			}
		}
	})
}

/// Adds `[auth.providers.<id>]` to the project's leptoskit.toml, keeping
/// its formatting, and returns what to tell the developer.
pub(crate) fn add_provider(
	root: &Path,
	config: &LeptoskitConfig,
	id: &str,
	issuer: Option<&str>,
) -> Result<String> {
	let valid = !id.is_empty()
		&& id
			.bytes()
			.all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-');

	if !valid {
		bail!("provider IDs are lowercase letters, digits, and dashes, such as `company-sso`");
	}

	let Some(auth) = &config.auth else {
		bail!("there's no [auth] table yet; run `leptoskit generate auth` first");
	};
	let mut redirects = format!("      http://localhost:3000/auth/callback/{id}   (development)");

	if let Some(origin) = &auth.origin {
		let _ = write!(
			redirects,
			"\n      {origin}/auth/callback/{id}   (from [auth] origin)"
		);
	}

	let recipe = recipe(id, issuer, &redirects)?;
	let path = root.join(leptoskit_config::CONFIG_FILE);
	let text = std::fs::read_to_string(&path)
		.with_context(|| format!("couldn't read {}", path.display()))?;
	let mut document: toml_edit::DocumentMut = text
		.parse()
		.with_context(|| format!("couldn't parse {}", path.display()))?;
	let mut out = String::new();

	if auth.providers.contains_key(id) {
		let _ = writeln!(out, "[auth.providers.{id}] is already in leptoskit.toml.");
	} else {
		let providers = document["auth"]
			.as_table_mut()
			.context("[auth] must be a table")?
			.entry("providers")
			.or_insert_with(|| {
				let mut table = toml_edit::Table::new();
				table.set_implicit(true);
				toml_edit::Item::Table(table)
			})
			.as_table_mut()
			.context("[auth.providers] must be a table")?;
		let mut table = toml_edit::Table::new();

		for (setting, value) in &recipe.settings {
			table.insert(setting, toml_edit::value(value.as_str()));
		}

		providers.insert(id, toml_edit::Item::Table(table));
		let updated = document.to_string();
		LeptoskitConfig::from_toml(&updated, root).with_context(|| {
			format!("adding [auth.providers.{id}] would make leptoskit.toml invalid")
		})?;
		std::fs::write(&path, updated)
			.with_context(|| format!("couldn't write {}", path.display()))?;
		let _ = writeln!(out, "Added [auth.providers.{id}] to leptoskit.toml.");
	}

	let _ = writeln!(out, "\n{id} sign-in: {} steps\n", recipe.steps.len() + 1);

	for (number, step) in recipe.steps.iter().enumerate() {
		let _ = writeln!(out, " {}. {step}", number + 1);
	}

	let _ = writeln!(
		out,
		" {}. Run `leptoskit auth doctor` to check the configuration.",
		recipe.steps.len() + 1
	);
	Ok(out)
}

#[cfg(test)]
mod tests {
	use std::collections::HashMap;

	use super::*;

	fn load(text: &str) -> LeptoskitConfig {
		LeptoskitConfig::from_toml(text, Path::new("notes")).unwrap()
	}

	fn env(set: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
		let set: HashMap<String, String> = set
			.iter()
			.map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
			.collect();
		move |name| set.get(name).cloned()
	}

	const KEYS: &str = "2026-10:BwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwc";

	#[test]
	fn doctor_names_what_is_missing_without_printing_secrets() {
		let config = load(
			"[app]\nplatforms = [\"web\", \"ios\"]\n\n[auth]\norigin = \
			 \"https://notes.example\"\n\n[auth.mail]\nfrom = \"a@b.c\"\nsmtp_url_env = \
			 \"SMTP_URL\"\n\n[auth.providers.google]\nclient_id_env = \"G_ID\"\nclient_secret_env \
			 = \"G_SECRET\"\n",
		);
		let findings = doctor(
			&config,
			env(&[
				("AUTH_ENCRYPTION_KEYS", KEYS),
				("SMTP_URL", "smtps://user:hunter2@smtp.example"),
				("G_ID", "id"),
			]),
			true,
		);
		let report = report(&findings);
		assert!(!report.contains("hunter2"), "{report}");
		assert!(
			report.contains("holds 1 key; `2026-10` encrypts"),
			"{report}"
		);
		assert!(
			report.contains("passkeys belong to `notes.example`"),
			"{report}"
		);
		assert!(report.contains("SMTP_URL is set"), "{report}");
		assert!(
			report.contains("error  google: G_SECRET (client_secret_env) isn't set"),
			"{report}"
		);
		assert!(
			report.contains("https://notes.example/auth/callback/google"),
			"{report}"
		);
		assert!(report.contains("guideline 4.8"), "{report}");
	}

	#[test]
	fn release_builds_refuse_plain_http_and_the_dev_mailbox() {
		let config = load(
			"[auth]\norigin = \"http://localhost:3000\"\n\n[native]\nplugins = [\"http\"]\n\n\
			 [native.plugin-options.http]\norigins = [\"http://10.0.2.2:3000\", \
			 \"https://notes.example\"]\n",
		);
		let keys = env(&[("AUTH_ENCRYPTION_KEYS", KEYS)]);
		let development = doctor(&config, &keys, false);
		assert!(
			development
				.iter()
				.all(|finding| finding.level != Level::Error),
			"{}",
			report(&development)
		);
		let release = doctor(&config, &keys, true);
		let errors: Vec<&str> = release
			.iter()
			.filter(|finding| finding.level == Level::Error)
			.map(|finding| finding.message.as_str())
			.collect();
		assert_eq!(errors.len(), 3, "{errors:?}");
		assert!(errors[0].contains("plain http"));
		assert!(errors[1].contains("development mailbox"));
		assert!(errors[2].contains("http://10.0.2.2:3000"));
		assert!(!errors[2].contains("https://notes.example"));
	}

	#[test]
	fn doctor_checks_keys_and_the_table() {
		assert_eq!(doctor(&load(""), env(&[]), false)[0].level, Level::Error);
		let config = load("[auth]\nrequired_for = [\"Admin\"]\n");
		let missing = report(&doctor(&config, env(&[]), false));
		assert!(
			missing.contains("error  AUTH_ENCRYPTION_KEYS isn't set"),
			"{missing}"
		);
		assert!(missing.contains("origin isn't set"), "{missing}");
		assert!(missing.contains("required for Admin"), "{missing}");
		let invalid = report(&doctor(
			&config,
			env(&[("AUTH_ENCRYPTION_KEYS", "nope")]),
			false,
		));
		assert!(
			invalid.contains("AUTH_ENCRYPTION_KEYS is invalid"),
			"{invalid}"
		);
		let rotated = report(&doctor(
			&config,
			env(&[(
				"AUTH_ENCRYPTION_KEYS",
				&format!("2027-04:AQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQE,{KEYS}"),
			)]),
			false,
		));
		assert!(rotated.contains("holds 2 keys"), "{rotated}");
		assert!(rotated.contains("rotate-keys"), "{rotated}");
	}

	#[test]
	fn adding_a_provider_writes_its_table_and_says_how() {
		let root = tempfile::tempdir().unwrap();
		let original =
			"# My app\n[app]\nname = \"notes\"\n\n[auth]\norigin = \"https://notes.example\" # \
			 the site\n";
		std::fs::write(root.path().join("leptoskit.toml"), original).unwrap();
		let config = LeptoskitConfig::from_toml(original, root.path()).unwrap();

		let output = add_provider(root.path(), &config, "google", None).unwrap();
		let written = std::fs::read_to_string(root.path().join("leptoskit.toml")).unwrap();
		assert!(written.starts_with(original), "{written}");
		assert!(written.contains("[auth.providers.google]"), "{written}");
		assert!(written.contains("client_secret_env = \"GOOGLE_CLIENT_SECRET\""));
		assert!(output.contains("http://localhost:3000/auth/callback/google"));
		assert!(output.contains("https://notes.example/auth/callback/google"));
		assert!(
			output.contains("4. Run `leptoskit auth doctor`"),
			"{output}"
		);

		// Again: it's already there, and the file doesn't change.
		let config = LeptoskitConfig::from_toml(&written, root.path()).unwrap();
		let again = add_provider(root.path(), &config, "google", None).unwrap();
		assert!(
			again.starts_with("[auth.providers.google] is already"),
			"{again}"
		);
		assert_eq!(
			std::fs::read_to_string(root.path().join("leptoskit.toml")).unwrap(),
			written
		);

		for id in ["github", "microsoft", "apple"] {
			add_provider(root.path(), &config, id, None).unwrap();
		}
		let company = add_provider(
			root.path(),
			&config,
			"company-sso",
			Some("https://sso.example/"),
		)
		.unwrap();
		assert!(company.contains("COMPANY_SSO_CLIENT_ID"), "{company}");
		let written = std::fs::read_to_string(root.path().join("leptoskit.toml")).unwrap();
		let reloaded = LeptoskitConfig::from_toml(&written, root.path()).unwrap();
		let providers = reloaded.auth.unwrap().providers;
		assert_eq!(providers.len(), 5);
		assert_eq!(
			providers["company-sso"].issuer.as_deref(),
			Some("https://sso.example")
		);
		assert_eq!(providers["microsoft"].tenant.as_deref(), Some("common"));

		assert!(add_provider(root.path(), &config, "company", None).is_err());
		assert!(add_provider(root.path(), &config, "Bad ID", None).is_err());
		let no_auth = LeptoskitConfig::from_toml("", root.path()).unwrap();
		assert!(add_provider(root.path(), &no_auth, "google", None).is_err());
	}

	#[tokio::test]
	async fn rotating_an_empty_database_moves_nothing() {
		let db = leptoskit_db::testing::sqlite().await;
		let keys = leptoskit_auth::Keys::parse(KEYS).unwrap();
		assert_eq!(
			rotate_in(&db, keys).await.unwrap(),
			leptoskit_auth::Rotated::default()
		);
	}

	#[test]
	fn rotation_says_what_moved_and_what_cannot() {
		let report = rotation_report(
			"2027-04",
			leptoskit_auth::Rotated {
				totp: 3,
				ceremonies: 1,
				current: 2,
			},
		);
		assert!(report.starts_with("Re-encrypted under `2027-04`: 3 authenticator secret(s)"));
		assert!(report.contains("can't be re-keyed"));
	}
}
