//! Configuring [`Idp`] from `leptoskit.toml`'s `[auth.idp]` table.

use std::time::Duration;

use leptoskit_auth::Auth;
use leptoskit_config::LeptoskitConfig;
use leptoskit_db::Db;

use crate::Idp;
use crate::IdpBuilder;
use crate::IdpError;
use crate::Lifetimes;
use crate::Portal;
use crate::Result;

const MINUTE: u64 = 60;
const DAY: u64 = 24 * 60 * MINUTE;

impl Idp {
	/// A builder configured from `[auth.idp]`, reading the RSA key from the
	/// environment variable it names. Add the app's own pieces, its scopes
	/// and its claims, then build it.
	///
	/// ```ignore
	/// let idp = Idp::from_config(&db, &auth, &config)?
	/// 	.scopes::<Scope>()
	/// 	.claims::<User>()
	/// 	.build()?;
	/// ```
	///
	/// # Errors
	///
	/// [`IdpError::Config`] without `[auth.idp]`, or when the variable
	/// `rsa_key_env` names isn't set.
	pub fn from_config(db: &Db, auth: &Auth, config: &LeptoskitConfig) -> Result<IdpBuilder> {
		let idp = config
			.auth
			.as_ref()
			.and_then(|auth| auth.idp.as_ref())
			.ok_or_else(|| IdpError::Config("add an [auth.idp] table to leptoskit.toml".into()))?;
		let mut builder = Self::builder(db, auth)
			.sign_in_page(&idp.sign_in_page)
			.consent_page(&idp.consent_page)
			.signed_out_page(&idp.signed_out_page)
			.device_page(&idp.device_page)
			.lifetimes(Lifetimes {
				access: Duration::from_secs(u64::from(idp.access_minutes) * MINUTE),
				id_token: Duration::from_secs(u64::from(idp.id_token_minutes) * MINUTE),
				refresh: Duration::from_secs(u64::from(idp.refresh_days) * DAY),
				signing_key: Duration::from_secs(u64::from(idp.signing_key_days) * DAY),
				..Lifetimes::default()
			});

		if let Some(issuer) = &idp.issuer {
			builder = builder.issuer(issuer);
		}

		if let Some(audience) = &idp.audience {
			builder = builder.audience(audience);
		}

		if let Some(name) = &idp.rsa_key_env {
			let key = std::env::var(name).map_err(|_| IdpError::Config(format!("set {name}")))?;
			builder = builder.rsa_key(key);
		}

		if idp.developer_portal {
			builder = builder.developer_portal(Portal {
				apps_per_person: idp.apps_per_person,
			});
		}

		Ok(builder)
	}
}

#[cfg(all(test, feature = "sqlite"))]
mod tests {
	use std::path::Path;

	use leptoskit_auth::Keys;
	use leptoskit_auth::Mailbox;

	use super::*;

	async fn auth() -> (Db, Auth) {
		let db = leptoskit_db::testing::sqlite().await;
		let auth = Auth::builder(&db, Keys::single("test", [2; 32]))
			.origin("https://notes.example")
			.mailer(Mailbox::new())
			.build()
			.unwrap();
		(db, auth)
	}

	#[tokio::test]
	async fn the_table_configures_the_provider() {
		let (db, auth) = auth().await;
		let config = LeptoskitConfig::from_toml(
			"[auth.idp]\naudience = \"https://api.notes.example\"\ndevice_page = \
			 \"/link\"\naccess_minutes = 5\ndeveloper_portal = true\n",
			Path::new("notes"),
		)
		.unwrap();
		let idp = Idp::from_config(&db, &auth, &config)
			.unwrap()
			.build()
			.unwrap();

		assert_eq!(idp.issuer(), "https://notes.example");
		assert_eq!(idp.inner.audience, "https://api.notes.example");
		assert_eq!(idp.inner.device_page, "/link");
		assert_eq!(idp.inner.lifetimes.access, Duration::from_secs(300));
		assert!(idp.has_developer_portal());
		assert!(idp.inner.rsa_key.is_none());

		let config = LeptoskitConfig::from_toml(
			"[auth.idp]\nissuer = \"https://id.notes.example\"\n",
			Path::new("notes"),
		)
		.unwrap();
		let idp = Idp::from_config(&db, &auth, &config)
			.unwrap()
			.build()
			.unwrap();
		assert_eq!(idp.issuer(), "https://id.notes.example");
		assert!(!idp.has_developer_portal());
	}

	#[tokio::test]
	async fn missing_settings_are_errors() {
		let (db, auth) = auth().await;
		let none = LeptoskitConfig::from_toml("[auth]\n", Path::new("notes")).unwrap();
		assert!(Idp::from_config(&db, &auth, &none).is_err());

		let rsa = LeptoskitConfig::from_toml(
			"[auth.idp]\nrsa_key_env = \"LEPTOSKIT_IDP_TEST_MISSING_RSA_KEY\"\n",
			Path::new("notes"),
		)
		.unwrap();
		let error = Idp::from_config(&db, &auth, &rsa).unwrap_err();
		assert!(
			error
				.to_string()
				.contains("LEPTOSKIT_IDP_TEST_MISSING_RSA_KEY")
		);
	}
}
