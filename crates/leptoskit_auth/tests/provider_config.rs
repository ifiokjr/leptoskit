//! `Auth::from_config` with `[auth.providers]` and `mfa`: each provider's
//! secrets come from the environment variables it names.
//!
//! One test, in its own process, because it sets environment variables.

#![cfg(all(feature = "sqlite", feature = "oauth"))]
#![allow(
	unsafe_code,
	reason = "tests set environment variables in their own process"
)]

#[path = "support/oidc.rs"]
mod oidc;

use std::collections::BTreeMap;

use leptoskit_auth::Auth;
use leptoskit_auth::AuthError;
use leptoskit_config::AuthConfig;
use leptoskit_config::AuthMfa;
use leptoskit_config::AuthProviderConfig;
use leptoskit_config::LeptoskitConfig;

const KEYS: &str = "test:BwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwc";

fn provider(pairs: &[(&str, &str)]) -> AuthProviderConfig {
	let mut provider = AuthProviderConfig::default();

	for (setting, value) in pairs {
		let value = Some((*value).to_owned());
		match *setting {
			"client_id_env" => provider.client_id_env = value,
			"client_secret_env" => provider.client_secret_env = value,
			"team_id_env" => provider.team_id_env = value,
			"key_id_env" => provider.key_id_env = value,
			"private_key_env" => provider.private_key_env = value,
			"issuer" => provider.issuer = value,
			"name" => provider.name = value,
			"tenant" => provider.tenant = value,
			_ => unreachable!(),
		}
	}

	provider
}

#[tokio::test]
async fn providers_and_the_mfa_policy_come_from_leptoskit_toml() {
	let db = leptoskit_db::testing::sqlite().await;
	let (pem, _) = oidc::apple_key();
	// SAFETY: this test is alone in its process, and nothing else reads the
	// environment while it runs.
	unsafe {
		std::env::set_var("PROVIDER_TEST_KEYS", KEYS);
		std::env::set_var("PROVIDER_TEST_ID", "client");
		std::env::set_var("PROVIDER_TEST_SECRET", "secret");
		std::env::set_var("PROVIDER_TEST_TEAM", "TEAM1");
		std::env::set_var("PROVIDER_TEST_KEY_ID", "KEY1");
		std::env::set_var("PROVIDER_TEST_P8", &pem);
	}
	let shared = [
		("client_id_env", "PROVIDER_TEST_ID"),
		("client_secret_env", "PROVIDER_TEST_SECRET"),
	];
	let mut providers = BTreeMap::new();
	providers.insert("google".to_owned(), provider(&shared));
	providers.insert("github".to_owned(), provider(&shared));
	providers.insert(
		"microsoft".to_owned(),
		provider(&[shared[0], shared[1], ("tenant", "consumers")]),
	);
	providers.insert(
		"apple".to_owned(),
		provider(&[
			("client_id_env", "PROVIDER_TEST_ID"),
			("team_id_env", "PROVIDER_TEST_TEAM"),
			("key_id_env", "PROVIDER_TEST_KEY_ID"),
			("private_key_env", "PROVIDER_TEST_P8"),
		]),
	);
	providers.insert(
		"facebook".to_owned(),
		AuthProviderConfig {
			api_version: Some("v25.0".into()),
			..provider(&shared)
		},
	);
	providers.insert(
		"gitlab".to_owned(),
		provider(&[
			("client_id_env", "PROVIDER_TEST_ID"),
			("issuer", "https://gitlab.example"),
		]),
	);
	providers.insert(
		"chat".to_owned(),
		AuthProviderConfig {
			kind: Some("discord".into()),
			store_tokens: true,
			..provider(&shared)
		},
	);
	let mut company = provider(&[
		("client_id_env", "PROVIDER_TEST_ID"),
		("issuer", oidc::ISSUER),
		("name", "Company SSO"),
	]);
	company.scopes = Some(vec!["email".into()]);
	providers.insert("company".to_owned(), company);
	let mut config = LeptoskitConfig::default();
	config.app.name = "notes".into();
	config.auth = Some(AuthConfig {
		keys_env: "PROVIDER_TEST_KEYS".into(),
		origin: Some("https://notes.example".into()),
		mfa: AuthMfa::Required,
		providers,
		..AuthConfig::default()
	});

	let auth = Auth::from_config(&db, &config).unwrap().build().unwrap();
	let mut names: Vec<(String, String)> = auth
		.providers()
		.into_iter()
		.map(|provider| (provider.id, provider.name))
		.collect();
	names.sort();
	let expected = [
		("apple", "Apple"),
		("chat", "Discord"),
		("company", "Company SSO"),
		("facebook", "Facebook"),
		("github", "GitHub"),
		("gitlab", "GitLab"),
		("google", "Google"),
		("microsoft", "Microsoft"),
	];
	assert_eq!(
		names,
		expected.map(|(id, name)| (id.to_owned(), name.to_owned()))
	);

	// A missing variable is named, never its value.
	unsafe {
		std::env::remove_var("PROVIDER_TEST_SECRET");
	}
	let missing = Auth::from_config(&db, &config).unwrap_err();
	assert!(
		matches!(&missing, AuthError::Config(message) if message == "set PROVIDER_TEST_SECRET"),
		"{missing}"
	);
}
