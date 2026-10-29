//! `Auth::from_config`: `[auth]` from `leptoskit.toml`, with its secrets
//! read from the environment, and email sent the way `[auth.mail]` says.
//!
//! One test, in its own process, because it sets environment variables.

#![cfg(all(
	feature = "axum",
	feature = "sqlite",
	feature = "smtp",
	feature = "webhook"
))]
#![allow(
	unsafe_code,
	reason = "tests set environment variables in their own process"
)]

use std::sync::Arc;
use std::sync::Mutex;

use axum::Router;
use axum::body::Body;
use axum::http::HeaderMap;
use axum::http::Request;
use axum::http::StatusCode;
use axum::routing::post;
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use hmac::Hmac;
use hmac::Mac;
use leptoskit_auth::Auth;
use leptoskit_auth::AuthError;
use leptoskit_auth::RequestInfo;
use leptoskit_config::AuthConfig;
use leptoskit_config::AuthMailConfig;
use leptoskit_config::LeptoskitConfig;
use serde_json::Value;
use sha2::Sha256;
use tower::ServiceExt;

const KEYS: &str = "test:BwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwc";

fn config(auth: Option<AuthConfig>) -> LeptoskitConfig {
	let mut config = LeptoskitConfig::default();
	config.app.name = "notes".into();
	config.auth = auth;
	config
}

/// What a webhook receiver saw.
type Received = Arc<Mutex<Vec<(HeaderMap, String)>>>;

/// A webhook receiver on a free port, answering `status`.
async fn receiver(status: StatusCode) -> (String, Received) {
	let received = Received::default();
	let log = Arc::clone(&received);
	let app = Router::new().route(
		"/mail",
		post(move |headers: HeaderMap, body: String| {
			let log = Arc::clone(&log);
			async move {
				log.lock().unwrap().push((headers, body));
				status
			}
		}),
	);
	let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
	let address = listener.local_addr().unwrap();
	tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
	(format!("http://{address}/mail"), received)
}

#[tokio::test]
async fn auth_is_configured_from_leptoskit_toml() {
	let db = leptoskit_db::testing::sqlite().await;
	let request = RequestInfo::default();

	let missing = Auth::from_config(&db, &config(None)).unwrap_err();
	assert!(missing.to_string().contains("add an [auth] table"));

	let no_keys = Auth::from_config(
		&db,
		&config(Some(AuthConfig {
			keys_env: "LEPTOSKIT_AUTH_CONFIG_TEST_MISSING".into(),
			..AuthConfig::default()
		})),
	)
	.unwrap_err();
	assert!(
		no_keys
			.to_string()
			.contains("set LEPTOSKIT_AUTH_CONFIG_TEST_MISSING")
	);

	// SAFETY: this test is alone in its process, and nothing else reads the
	// environment while it runs.
	unsafe {
		std::env::set_var("LEPTOSKIT_AUTH_CONFIG_TEST_KEYS", KEYS);
		std::env::set_var("LEPTOSKIT_AUTH_CONFIG_TEST_SECRET", "whsec_c2VjcmV0");
		std::env::set_var("LEPTOSKIT_AUTH_CONFIG_TEST_SMTP", "smtp://127.0.0.1:1");
	}
	let auth_config = AuthConfig {
		keys_env: "LEPTOSKIT_AUTH_CONFIG_TEST_KEYS".into(),
		origin: Some("https://notes.example".into()),
		client_ip_header: Some("fly-client-ip".into()),
		..AuthConfig::default()
	};

	// Without a mailer, development keeps email in the mailbox.
	let auth = Auth::from_config(&db, &config(Some(auth_config.clone())))
		.unwrap()
		.build()
		.unwrap();
	assert_eq!(auth.app_name(), "notes");
	auth.send_email_code("ada@example.com", &request)
		.await
		.unwrap();
	let page = auth
		.router::<()>()
		.layer(auth.layer())
		.oneshot(
			Request::builder()
				.uri("/_leptoskit/mail")
				.body(Body::empty())
				.unwrap(),
		)
		.await
		.unwrap();
	assert_eq!(page.status(), StatusCode::OK);

	// A webhook gets each email, signed.
	let (url, received) = receiver(StatusCode::NO_CONTENT).await;
	unsafe {
		std::env::set_var("LEPTOSKIT_AUTH_CONFIG_TEST_WEBHOOK", &url);
	}
	let webhook = AuthConfig {
		app_name: Some("Notes".into()),
		mail: AuthMailConfig {
			from: Some("Notes <hi@notes.example>".into()),
			webhook_url_env: Some("LEPTOSKIT_AUTH_CONFIG_TEST_WEBHOOK".into()),
			webhook_secret_env: Some("LEPTOSKIT_AUTH_CONFIG_TEST_SECRET".into()),
			..AuthMailConfig::default()
		},
		..auth_config.clone()
	};
	let auth = Auth::from_config(&db, &config(Some(webhook)))
		.unwrap()
		.build()
		.unwrap();
	auth.send_email_code("bob@example.com", &request)
		.await
		.unwrap();

	let (headers, body) = received.lock().unwrap().remove(0);
	let json: Value = serde_json::from_str(&body).unwrap();
	assert_eq!(json["type"], "email");
	assert_eq!(json["to"], "bob@example.com");
	assert_eq!(json["from"], "Notes <hi@notes.example>");
	assert!(
		json["subject"]
			.as_str()
			.unwrap()
			.starts_with("Your Notes sign-in code")
	);
	let header = |name: &str| headers[name].to_str().unwrap().to_owned();
	let mut mac = <Hmac<Sha256> as Mac>::new_from_slice(b"secret").unwrap();
	mac.update(
		format!(
			"{}.{}.{body}",
			header("webhook-id"),
			header("webhook-timestamp")
		)
		.as_bytes(),
	);
	assert_eq!(
		header("webhook-signature"),
		format!("v1,{}", STANDARD.encode(mac.finalize().into_bytes()))
	);
	assert!(header("webhook-id").starts_with("msg_"));

	// A receiver that refuses is a failure to send.
	let (url, _) = receiver(StatusCode::SERVICE_UNAVAILABLE).await;
	unsafe {
		std::env::set_var("LEPTOSKIT_AUTH_CONFIG_TEST_WEBHOOK", &url);
	}
	let refusing = AuthConfig {
		mail: AuthMailConfig {
			webhook_url_env: Some("LEPTOSKIT_AUTH_CONFIG_TEST_WEBHOOK".into()),
			webhook_secret_env: Some("LEPTOSKIT_AUTH_CONFIG_TEST_SECRET".into()),
			..AuthMailConfig::default()
		},
		..auth_config.clone()
	};
	let auth = Auth::from_config(&db, &config(Some(refusing)))
		.unwrap()
		.build()
		.unwrap();
	let failed = auth
		.send_email_code("eve@example.com", &request)
		.await
		.unwrap_err();
	assert!(
		matches!(&failed, AuthError::Mail(error) if error.to_string() == "the webhook answered 503 Service Unavailable"),
		"{failed}"
	);

	// SMTP is configured the same way.
	let smtp = AuthConfig {
		mail: AuthMailConfig {
			from: Some("Notes <hi@notes.example>".into()),
			smtp_url_env: Some("LEPTOSKIT_AUTH_CONFIG_TEST_SMTP".into()),
			..AuthMailConfig::default()
		},
		..auth_config
	};
	let auth = Auth::from_config(&db, &config(Some(smtp)))
		.unwrap()
		.build()
		.unwrap();
	let unreachable = auth
		.send_email_code("ada@example.com", &request)
		.await
		.unwrap_err();
	assert!(matches!(unreachable, AuthError::Mail(_)), "{unreachable}");
}
