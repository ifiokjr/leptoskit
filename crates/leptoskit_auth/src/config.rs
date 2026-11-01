//! Configuring [`Auth`] from `leptoskit.toml`'s `[auth]` table.

use std::time::Duration;

use leptoskit_config::AuthConfig;
use leptoskit_config::AuthMfa;
use leptoskit_config::LeptoskitConfig;
use leptoskit_db::Db;

use crate::Auth;
use crate::AuthBuilder;
use crate::AuthError;
use crate::Keys;
use crate::MfaPolicy;
use crate::Result;
use crate::SessionLifetimes;

const DAY: u64 = 24 * 60 * 60;

impl Auth {
	/// A builder configured from `[auth]`, reading each secret from the
	/// environment variable it names. Add the app's own pieces, such as
	/// [`AuthBuilder::on_sign_up`], then build it.
	///
	/// ```ignore
	/// let config = LeptoskitConfig::from_toml(include_str!("../../leptoskit.toml"), Path::new("my_app"))?;
	/// let auth = Auth::from_config(&db, &config)?.on_sign_up::<User>().build()?;
	/// ```
	///
	/// Email goes over SMTP or a webhook, as `[auth.mail]` says. With
	/// neither, a debug build keeps it in the development mailbox, and a
	/// release build refuses to start.
	///
	/// # Errors
	///
	/// [`AuthError::Config`] without `[auth]`, when a named variable is
	/// missing, or when the configured mailer's feature (`smtp` or
	/// `webhook`) is off.
	pub fn from_config(db: &Db, config: &LeptoskitConfig) -> Result<AuthBuilder> {
		let auth = config
			.auth
			.as_ref()
			.ok_or_else(|| AuthError::Config("add an [auth] table to leptoskit.toml".into()))?;
		let mut builder = Self::builder(db, Keys::from_env(&auth.keys_env)?)
			.app_name(
				auth.app_name
					.clone()
					.unwrap_or_else(|| config.app.name.clone()),
			)
			.sessions(SessionLifetimes {
				idle: Duration::from_secs(u64::from(auth.idle_days) * DAY),
				absolute: Duration::from_secs(u64::from(auth.absolute_days) * DAY),
				access: Duration::from_secs(u64::from(auth.access_minutes) * 60),
			});

		if let Some(origin) = &auth.origin {
			builder = builder.origin(origin);
		}

		if let Some(header) = &auth.client_ip_header {
			builder = builder.client_ip_header(header);
		}

		builder = builder.mfa(MfaPolicy {
			required: auth.mfa == AuthMfa::Required,
			required_for: auth.required_for.clone(),
		});
		builder = builder.solana(crate::SolanaOptions {
			sign_in: auth.solana.sign_in,
			chain_id: auth
				.solana
				.chain_id
				.clone()
				.unwrap_or_else(|| crate::SolanaOptions::default().chain_id),
		});
		builder = builder.native_apps(crate::NativeApps {
			package_id: config.native.package_id.clone(),
			apple_team_id: config.native.apple_team_id.clone(),
			android_certificates: config.native.android_certificate_digests(),
		});

		if auth.passwords.enabled {
			builder = builder.passwords(crate::PasswordOptions {
				min_length: auth.passwords.min_length,
				breach_check: auth.passwords.breach_check,
			});
		}

		builder = providers(builder, auth)?;
		mailer(builder, auth)
	}
}

/// Adds `[auth.providers]`, reading each one's secrets from the
/// environment.
#[cfg(feature = "oauth")]
fn providers(mut builder: AuthBuilder, auth: &AuthConfig) -> Result<AuthBuilder> {
	use crate::Provider;

	for (id, config) in &auth.providers {
		let get =
			|name: &Option<String>| -> Result<String> { env(name.as_deref().unwrap_or_default()) };
		let optional = |name: &Option<String>| name.as_deref().map(env).transpose();
		let kind = config.kind(id);

		let mut provider = match kind.as_str() {
			"microsoft" => {
				Provider::microsoft(
					get(&config.client_id_env)?,
					get(&config.client_secret_env)?,
					config.tenant.as_deref().unwrap_or("common"),
				)
			}
			"apple" => {
				Provider::apple(
					get(&config.client_id_env)?,
					get(&config.team_id_env)?,
					get(&config.key_id_env)?,
					&get(&config.private_key_env)?,
				)?
			}
			"facebook" => {
				Provider::facebook(
					get(&config.client_id_env)?,
					get(&config.client_secret_env)?,
					config
						.api_version
						.as_deref()
						.unwrap_or(crate::oauth::FACEBOOK_GRAPH_VERSION),
				)
			}
			"gitlab" => {
				Provider::gitlab(
					get(&config.client_id_env)?,
					optional(&config.client_secret_env)?,
					config.issuer.as_deref().unwrap_or(crate::oauth::GITLAB_COM),
				)?
			}
			"oidc" => {
				Provider::oidc(
					id,
					config.name.clone().unwrap_or_else(|| id.clone()),
					config.issuer.clone().unwrap_or_default(),
					get(&config.client_id_env)?,
					optional(&config.client_secret_env)?,
				)?
			}
			kind => {
				crate::oauth::built_in(
					kind,
					get(&config.client_id_env)?,
					get(&config.client_secret_env)?,
				)
				.ok_or_else(|| {
					AuthError::Config(format!("`{kind}` isn't a provider leptoskit knows"))
				})?
			}
		}
		.with_id(id);

		if let Some(name) = &config.name {
			provider = provider.name(name);
		}

		if let Some(scopes) = &config.scopes {
			provider = provider.scopes(scopes.iter().cloned());
		}

		if config.store_tokens {
			provider = provider.store_tokens(true);
		}

		builder = builder.provider(provider);
	}

	Ok(builder)
}

#[cfg(not(feature = "oauth"))]
fn providers(builder: AuthBuilder, auth: &AuthConfig) -> Result<AuthBuilder> {
	if auth.providers.is_empty() {
		Ok(builder)
	} else {
		Err(AuthError::Config(
			"[auth.providers] signs in with other sites; turn on leptoskit_auth's `oauth` feature"
				.into(),
		))
	}
}

fn mailer(builder: AuthBuilder, auth: &AuthConfig) -> Result<AuthBuilder> {
	let mail = &auth.mail;

	if let Some(name) = &mail.smtp_url_env {
		let url = env(name)?;
		let from = mail.from.as_deref().unwrap_or_default();

		return smtp(builder, &url, from);
	}

	if let Some(name) = &mail.webhook_url_env {
		let url = env(name)?;
		let secret = env(mail.webhook_secret_env.as_deref().unwrap_or_default())?;

		return webhook(builder, url, &secret, mail.from.clone());
	}

	development_mailbox(builder)
}

#[cfg(feature = "smtp")]
fn smtp(builder: AuthBuilder, url: &str, from: &str) -> Result<AuthBuilder> {
	Ok(builder.mailer(crate::SmtpMailer::new(url, from)?))
}

#[cfg(not(feature = "smtp"))]
fn smtp(_builder: AuthBuilder, _url: &str, _from: &str) -> Result<AuthBuilder> {
	Err(AuthError::Config(
		"[auth.mail] sends over SMTP; turn on leptoskit_auth's `smtp` feature".into(),
	))
}

#[cfg(feature = "webhook")]
fn webhook(
	builder: AuthBuilder,
	url: String,
	secret: &str,
	from: Option<String>,
) -> Result<AuthBuilder> {
	Ok(builder.mailer(crate::WebhookMailer::new(url, secret, from)?))
}

#[cfg(not(feature = "webhook"))]
fn webhook(
	_builder: AuthBuilder,
	_url: String,
	_secret: &str,
	_from: Option<String>,
) -> Result<AuthBuilder> {
	Err(AuthError::Config(
		"[auth.mail] sends to a webhook; turn on leptoskit_auth's `webhook` feature".into(),
	))
}

#[cfg(debug_assertions)]
#[allow(clippy::unnecessary_wraps, reason = "a release build refuses instead")]
fn development_mailbox(builder: AuthBuilder) -> Result<AuthBuilder> {
	Ok(builder.dev_mailbox())
}

#[cfg(not(debug_assertions))]
fn development_mailbox(_builder: AuthBuilder) -> Result<AuthBuilder> {
	Err(AuthError::Config(
		"set smtp_url_env or webhook_url_env in [auth.mail]; only development keeps email in a \
		 mailbox"
			.into(),
	))
}

fn env(name: &str) -> Result<String> {
	std::env::var(name).map_err(|_| AuthError::Config(format!("set {name}")))
}
