//! The remote client against a scripted server: what it sends, how it
//! signs and re-signs requests, when it sends again, and how it reads the
//! answers.

use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::Mutex;

use leptoskit_query::BoxFuture;
use serde_json::json;

use super::Api;
use super::Args;
use super::CallOptions;
use super::ClientConfig;
use super::Credentials;
use super::Key;
use super::protocol::Http;
use super::protocol::HttpRequest;
use super::protocol::HttpResponse;
use super::protocol::Remote;
use super::protocol::Unreachable;
use super::protocol::decode;
use crate::FnError;

/// Answers each request with the next scripted response, and keeps what it
/// was sent.
#[derive(Clone, Default)]
struct Script {
	answers: Arc<Mutex<VecDeque<Result<HttpResponse, FnError>>>>,
	sent: Arc<Mutex<Vec<HttpRequest>>>,
}

impl Script {
	fn answer(self, status: u16, body: &str) -> Self {
		self.answers.lock().unwrap().push_back(Ok(HttpResponse {
			status,
			body: body.as_bytes().to_vec(),
		}));
		self
	}

	fn drop_connection(self) -> Self {
		self.answers
			.lock()
			.unwrap()
			.push_back(Err(FnError::Network("reset".to_owned())));
		self
	}

	fn sent(&self) -> Vec<HttpRequest> {
		self.sent.lock().unwrap().clone()
	}
}

impl Http for Script {
	fn send(&self, request: HttpRequest) -> BoxFuture<'static, Result<HttpResponse, FnError>> {
		self.sent.lock().unwrap().push(request);
		let answer = self
			.answers
			.lock()
			.unwrap()
			.pop_front()
			.expect("a scripted answer");
		Box::pin(std::future::ready(answer))
	}
}

/// A bearer token that renews once after a `401`.
struct Token {
	current: Mutex<String>,
	renewals: Mutex<u32>,
}

impl Credentials for Token {
	fn authorization(&self) -> BoxFuture<'_, Result<Option<String>, FnError>> {
		let header = format!("Bearer {}", self.current.lock().unwrap());
		Box::pin(std::future::ready(Ok(Some(header))))
	}

	fn refused(&self, _sent: Option<String>) -> BoxFuture<'_, Result<bool, FnError>> {
		let mut renewals = self.renewals.lock().unwrap();
		*renewals += 1;
		*self.current.lock().unwrap() = "fresh".to_owned();
		Box::pin(std::future::ready(Ok(*renewals == 1)))
	}
}

fn remote(script: &Script, config: ClientConfig) -> Remote<Script> {
	Remote::new(config, script.clone())
}

#[tokio::test]
async fn calls_post_the_arguments_to_the_operations_route() {
	let script = Script::default().answer(200, r#"["milk"]"#);
	let client = remote(
		&script,
		ClientConfig::new().origin("https://notes.example/"),
	);
	let value = client
		.call("notes", json!({ "starting": "m" }), Key::None)
		.await
		.unwrap();
	assert_eq!(value, json!(["milk"]));

	let sent = script.sent();
	assert_eq!(sent.len(), 1);
	assert_eq!(sent[0].url, "https://notes.example/api/notes");
	assert_eq!(sent[0].body, r#"{"starting":"m"}"#);
	assert_eq!(
		sent[0].headers(),
		[
			("accept", "application/json"),
			("content-type", "application/json"),
		]
	);
}

#[tokio::test]
async fn credentials_sign_requests_and_renew_after_a_refusal() {
	let token = Arc::new(Token {
		current: Mutex::new("stale".to_owned()),
		renewals: Mutex::new(0),
	});
	let script = Script::default()
		.answer(401, "ServerError|expired")
		.answer(200, "1")
		.answer(401, "ServerError|expired");
	let client = remote(
		&script,
		ClientConfig {
			credentials: Some(token.clone()),
			..ClientConfig::default()
		},
	);

	assert_eq!(
		client.call("count", json!({}), Key::None).await.unwrap(),
		json!(1)
	);
	let sent = script.sent();
	assert_eq!(sent[0].authorization.as_deref(), Some("Bearer stale"));
	assert_eq!(sent[1].authorization.as_deref(), Some("Bearer fresh"));
	assert_eq!(sent[1].url, "/api/count");

	// A second refusal that renewing can't fix reaches the caller.
	assert_eq!(
		client.call("count", json!({}), Key::None).await,
		Err(FnError::Server("expired".to_owned()))
	);
	assert_eq!(*token.renewals.lock().unwrap(), 2);
}

#[tokio::test]
async fn a_keyed_call_is_sent_again_with_the_same_key_when_the_network_drops_it() {
	let script = Script::default()
		.drop_connection()
		.drop_connection()
		.answer(200, r#""id-1""#);
	let client = remote(&script, ClientConfig::new());
	let value = client
		.call(
			"add_note",
			json!({ "title": "milk" }),
			Key::Given("k".to_owned()),
		)
		.await
		.unwrap();
	assert_eq!(value, json!("id-1"));
	let sent = script.sent();
	assert_eq!(sent.len(), 3);
	assert!(
		sent.iter()
			.all(|request| request.idempotency_key.as_deref() == Some("k"))
	);
	assert!(sent[0].headers().contains(&("idempotency-key", "k")));

	// Without a key, a dropped call isn't sent again: it might have run.
	let script = Script::default().drop_connection();
	let client = remote(&script, ClientConfig::new());
	assert!(matches!(
		client.call("add_note", json!({}), Key::None).await,
		Err(FnError::Network(_))
	));
	assert_eq!(script.sent().len(), 1);
}

#[test]
fn answers_decode_as_values_or_classified_errors() {
	let response = |status: u16, body: &str| {
		HttpResponse {
			status,
			body: body.as_bytes().to_vec(),
		}
	};
	assert_eq!(decode(&response(200, "")), Ok(json!(null)));
	assert_eq!(decode(&response(200, "[1]")), Ok(json!([1])));
	assert!(matches!(
		decode(&response(200, "{")),
		Err(FnError::Decode(_))
	));
	assert_eq!(
		decode(&response(400, "Args|too long")),
		Err(FnError::Invalid("too long".to_owned()))
	);
	assert_eq!(
		decode(&response(404, " ")),
		Err(FnError::Other("the server answered HTTP 404".to_owned()))
	);
}

#[tokio::test]
async fn a_build_with_no_way_to_the_server_says_so() {
	let client = Remote::new(ClientConfig::new(), Unreachable);
	let error = client
		.call("notes", json!({}), Key::None)
		.await
		.unwrap_err();
	assert!(error.message().contains("`web` feature"), "{error}");
}

#[test]
fn config_and_options_build_up() {
	let config = ClientConfig::new()
		.origin("https://a.example")
		.credentials(Token {
			current: Mutex::new(String::new()),
			renewals: Mutex::new(0),
		});
	assert_eq!(config.origin.as_deref(), Some("https://a.example"));
	assert!(format!("{config:?}").contains("credentials: true"));
	assert_eq!(
		CallOptions::new().idempotency_key("k").idempotency_key,
		Some("k".to_owned())
	);
}

/// A server build without a database can't run queries, so it has no
/// context to give one.
#[cfg(any(not(feature = "server"), feature = "postgres", feature = "sqlite"))]
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
struct Search {
	text: String,
	score: f64,
}

#[cfg(any(not(feature = "server"), feature = "postgres", feature = "sqlite"))]
impl crate::Operation for Search {
	#[cfg(feature = "server")]
	type Context = crate::QueryCx;
	type Output = Vec<String>;

	const KIND: crate::Kind = crate::Kind::Query;
	const NAME: &'static str = "search";

	#[cfg(feature = "server")]
	fn run(self, _cx: &crate::QueryCx) -> impl Future<Output = crate::Result<Vec<String>>> + Send {
		std::future::ready(Ok(vec![self.text]))
	}
}

#[cfg(any(not(feature = "server"), feature = "postgres", feature = "sqlite"))]
#[test]
fn query_keys_compare_by_their_arguments() {
	use std::hash::BuildHasher;

	let key = |text: &str| {
		Args::new(Search {
			text: text.to_owned(),
			score: 0.5,
		})
	};
	let state = std::collections::hash_map::RandomState::new();
	assert_eq!(key("a"), key("a"));
	assert_ne!(key("a"), key("b"));
	assert_eq!(state.hash_one(key("a")), state.hash_one(key("a")));
	assert_eq!(format!("{:?}", key("a")), r#"{"text":"a","score":0.5}"#);
	let cloned = key("c").clone();
	assert_eq!(cloned.get().text, "c");

	assert_eq!(Args::from(cloned.into_inner()), key("c"));
	assert_eq!(
		super::query_scope::<Search>().name(),
		"rpc:search",
		"scopes are named after the operation, so the browser picks up the server's data"
	);
	let _ = super::mutation_scope::<Search>();
}

#[tokio::test]
async fn keys_are_long_random_and_fresh() {
	let first = Key::Fresh.resolve().await.unwrap();
	let second = Key::Fresh.resolve().await.unwrap();
	assert_eq!(Key::None.resolve().await, None);
	assert_eq!(first.len(), 32);
	assert!(first.chars().all(|character| character.is_ascii_hexdigit()));
	assert_ne!(first, second);
}

/// A server's client for a service: where it sends, and how it signs.
#[tokio::test]
async fn a_server_calls_a_service_at_its_url_anonymously_unless_as_the_app() {
	use super::services::FromEnvironment;

	let lookup = |name: &str| {
		match name {
			"LEPTOSKIT_SERVICE_EVENT_STORE_URL" => Some("http://127.0.0.1:3100/".to_owned()),
			"LEPTOSKIT_SERVICE_EVENT_STORE_TOKEN" => Some("t0k3n".to_owned()),
			_ => None,
		}
	};

	// What `call`, the query scopes and the server's rendering send: no
	// token, though the server has one, so a page only ever renders what
	// the browser could fetch itself.
	let script = Script::default().answer(200, r#""pong""#);
	let client = FromEnvironment::new("event-store", script.clone(), lookup);
	let value = client
		.call("ping", json!({ "word": "hi" }), Key::None)
		.await
		.unwrap();
	assert_eq!(value, json!("pong"));
	let sent = script.sent();
	assert_eq!(
		sent[0].url,
		"http://127.0.0.1:3100/_leptoskit/services/event-store/api/ping"
	);
	assert_eq!(sent[0].authorization, None, "anonymous by default");

	// `call_as_app`: signed with the service's token.
	let script = Script::default().answer(200, r#""pong""#);
	let as_app = FromEnvironment::new("event-store", script.clone(), lookup).signed_as_app();
	as_app.call("ping", json!({}), Key::None).await.unwrap();
	assert_eq!(
		script.sent()[0].authorization.as_deref(),
		Some("Bearer t0k3n")
	);

	// As the app without a token: refused before anything is sent.
	let script = Script::default();
	let tokenless = FromEnvironment::new("cards", script.clone(), |name| {
		(name == "LEPTOSKIT_SERVICE_CARDS_URL").then(|| "http://cards:1".to_owned())
	})
	.signed_as_app();
	let error = tokenless
		.call("count", json!({}), Key::None)
		.await
		.unwrap_err();
	assert!(
		error
			.message()
			.contains("LEPTOSKIT_SERVICE_CARDS_TOKEN isn't set"),
		"{error}"
	);
	assert_eq!(script.sent().len(), 0);

	let script = Script::default();
	let missing = FromEnvironment::new("cards", script.clone(), |_| Some("  ".to_owned()));
	let error = missing
		.call("count", json!({}), Key::None)
		.await
		.unwrap_err();
	assert!(
		error
			.message()
			.contains("LEPTOSKIT_SERVICE_CARDS_URL isn't set"),
		"{error}"
	);
	assert_eq!(script.sent().len(), 0, "nothing is sent without a URL");
}

#[tokio::test]
async fn a_browser_or_native_app_calls_a_service_on_the_apps_origin() {
	use super::services::at_service;
	use super::services::service_origin;

	// In the browser: the page's own origin, with the cookie and no token.
	let script = Script::default().answer(200, "1");
	let browser = remote(&script, at_service(&ClientConfig::new(), "analytics"));
	browser.call("capture", json!({}), Key::None).await.unwrap();
	assert_eq!(
		script.sent()[0].url,
		"/_leptoskit/services/analytics/api/capture"
	);

	// In a native app: the configured origin, and none of the app's own
	// credentials: the person's access token stays with the app.
	let token = Arc::new(Token {
		current: Mutex::new("person".to_owned()),
		renewals: Mutex::new(0),
	});
	let app = ClientConfig {
		credentials: Some(Arc::clone(&token) as Arc<dyn Credentials>),
		..ClientConfig::new()
			.origin("https://app.example")
			.offline_store(super::MemoryQueue::default())
	};
	let service = at_service(&app, "analytics");
	assert!(service.credentials.is_none());
	// The queue, where it's kept, and its status are the app's: a service's
	// live client never restores or saves them, and sends its mutations
	// over HTTP while its socket is down.
	assert!(!service.offline_queue, "the offline queue is the app's own");
	assert!(service.offline_store.is_none());
	let script = Script::default().answer(401, "\"who are you?\"");
	let native = remote(&script, service);
	let _ = native.call("capture", json!({}), Key::None).await;
	assert_eq!(
		script.sent()[0].url,
		"https://app.example/_leptoskit/services/analytics/api/capture"
	);
	assert_eq!(script.sent()[0].authorization, None);
	assert_eq!(script.sent().len(), 1, "a service's 401 renews nothing");
	assert_eq!(
		*token.renewals.lock().unwrap(),
		0,
		"the app's credentials are never asked to renew for a service"
	);

	// Credentials the app names for the service go to that service only.
	let mut app = app;
	app.service_credentials.insert(
		"analytics".to_owned(),
		Arc::clone(&token) as Arc<dyn Credentials>,
	);
	let script = Script::default().answer(200, "1");
	remote(&script, at_service(&app, "analytics"))
		.call("capture", json!({}), Key::None)
		.await
		.unwrap();
	assert_eq!(
		script.sent()[0].authorization.as_deref(),
		Some("Bearer person")
	);
	assert!(at_service(&app, "search").credentials.is_none());

	assert_eq!(
		service_origin(None, "analytics"),
		"/_leptoskit/services/analytics"
	);
	assert_eq!(
		service_origin(Some("https://app.example/"), "analytics"),
		"https://app.example/_leptoskit/services/analytics"
	);
}
