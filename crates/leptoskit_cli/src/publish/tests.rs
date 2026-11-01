//! Each store client against a local server that answers as the store does,
//! from its documentation, and records what it was sent.

use std::sync::Arc;
use std::sync::Mutex;

use axum::body::Bytes;
use axum::http::HeaderMap;
use axum::http::Method;
use axum::http::StatusCode;
use axum::http::Uri;
use axum::response::IntoResponse;
use axum::response::Response;

use super::app_store::AppStore;
use super::app_store::Key;
use super::chrome::Chrome;
use super::edge::Edge;
use super::firefox::Firefox;
use super::http::Http;
use super::secrets::Secret;

/// One request the fake store received.
#[derive(Clone, Debug)]
struct Seen {
	method: String,
	path: String,
	query: String,
	authorization: String,
	content_type: String,
	body: Vec<u8>,
}

type Answer = Box<dyn Fn(&Seen, usize) -> Response + Send + Sync>;

/// A store on loopback: `answer` gets each request and how many came
/// before it.
fn store(answer: Answer) -> (String, Arc<Mutex<Vec<Seen>>>) {
	let seen: Arc<Mutex<Vec<Seen>>> = Arc::default();
	let recorded = Arc::clone(&seen);
	let answer = Arc::new(answer);
	let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
	let address = listener.local_addr().unwrap();
	listener.set_nonblocking(true).unwrap();
	std::thread::spawn(move || {
		let runtime = tokio::runtime::Builder::new_current_thread()
			.enable_all()
			.build()
			.unwrap();
		runtime.block_on(async move {
			let listener = tokio::net::TcpListener::from_std(listener).unwrap();
			let router = axum::Router::new().fallback(
				move |method: Method, uri: Uri, headers: HeaderMap, body: Bytes| {
					let recorded = Arc::clone(&recorded);
					let answer = Arc::clone(&answer);
					async move {
						let header = |name: &str| {
							headers
								.get(name)
								.and_then(|value| value.to_str().ok())
								.unwrap_or_default()
								.to_owned()
						};
						let request = Seen {
							method: method.to_string(),
							path: uri.path().to_owned(),
							query: uri.query().unwrap_or_default().to_owned(),
							authorization: header("authorization"),
							content_type: header("content-type"),
							body: body.to_vec(),
						};
						let count = {
							let mut seen = recorded.lock().unwrap();
							seen.push(request.clone());
							seen.len() - 1
						};
						answer(&request, count)
					}
				},
			);
			axum::serve(listener, router).await.unwrap();
		});
	});
	(format!("http://{address}"), seen)
}

fn json(value: serde_json::Value) -> Response {
	axum::Json(value).into_response()
}

/// A fresh RSA key in PKCS#8 PEM, so no private key is ever committed.
fn rsa_key() -> Option<String> {
	let output = std::process::Command::new("openssl")
		.args([
			"genpkey",
			"-algorithm",
			"RSA",
			"-pkeyopt",
			"rsa_keygen_bits:2048",
		])
		.output()
		.ok()?;
	output
		.status
		.success()
		.then(|| String::from_utf8(output.stdout).unwrap())
}

#[test]
fn chrome_signs_in_uploads_and_submits() {
	let Some(key) = rsa_key() else { return };
	let (base, seen) = store(Box::new(|request, _| {
		match request.path.as_str() {
			"/token" => json(serde_json::json!({ "access_token": "token-1", "expires_in": 3600 })),
			path if path.ends_with(":upload") => {
				json(serde_json::json!({ "uploadState": "SUCCEEDED", "crxVersion": "1.4.0" }))
			}
			path if path.ends_with(":publish") => {
				json(serde_json::json!({ "state": "PENDING_REVIEW" }))
			}
			_ => StatusCode::NOT_FOUND.into_response(),
		}
	}));
	let account = Secret::new(serde_json::json!({ "client_email": "ci@project.iam.gserviceaccount.com", "private_key": key }).to_string());
	let http = Http::default();
	let chrome = Chrome::connect(
		&http,
		&base,
		&format!("{base}/token"),
		&account,
		"pub-1",
		"abcdefghijklmnopabcdefghijklmnop",
	)
	.unwrap();
	assert_eq!(chrome.upload(b"PK zip".to_vec()).unwrap(), "1.4.0");
	assert_eq!(chrome.submit(false).unwrap(), "PENDING_REVIEW");

	let seen = seen.lock().unwrap();
	let form = String::from_utf8(seen[0].body.clone()).unwrap();
	assert!(form.starts_with(
		"grant_type=urn%3Aietf%3Aparams%3Aoauth%3Agrant-type%3Ajwt-bearer&assertion="
	));
	let assertion = form.rsplit('=').next().unwrap();
	assert_eq!(assertion.split('.').count(), 3, "a signed JWT");
	assert_eq!(
		seen[1].path,
		"/upload/v2/publishers/pub-1/items/abcdefghijklmnopabcdefghijklmnop:upload"
	);
	assert_eq!(seen[1].authorization, "Bearer token-1");
	assert_eq!(seen[1].content_type, "application/zip");
	assert_eq!(seen[1].body, b"PK zip");
	assert_eq!(
		seen[2].path,
		"/v2/publishers/pub-1/items/abcdefghijklmnopabcdefghijklmnop:publish"
	);
	let publish: serde_json::Value = serde_json::from_slice(&seen[2].body).unwrap();
	assert_eq!(publish["publishType"], "DEFAULT_PUBLISH");
}

#[test]
fn chrome_reports_a_refused_package() {
	let Some(key) = rsa_key() else { return };
	let (base, _) = store(Box::new(|request, _| {
		match request.path.as_str() {
			"/token" => json(serde_json::json!({ "access_token": "t" })),
			_ => (StatusCode::BAD_REQUEST, "manifest version must be newer").into_response(),
		}
	}));
	let account =
		Secret::new(serde_json::json!({ "client_email": "a@b", "private_key": key }).to_string());
	let http = Http::default();
	let chrome = Chrome::connect(
		&http,
		&base,
		&format!("{base}/token"),
		&account,
		"p",
		"abcdefghijklmnopabcdefghijklmnop",
	)
	.unwrap();
	let error = chrome.upload(b"zip".to_vec()).unwrap_err().to_string();
	assert!(
		error.contains("400") && error.contains("must be newer"),
		"{error}"
	);
}

#[test]
fn edge_uploads_waits_for_its_operations_and_submits() {
	let (base, seen) = store(Box::new(|request, count| {
		let accepted = |location: &str| {
			let mut response = StatusCode::ACCEPTED.into_response();
			response
				.headers_mut()
				.insert("location", location.parse().unwrap());
			response
		};
		match (request.method.as_str(), request.path.as_str()) {
			("POST", "/v1/products/prod-1/submissions/draft/package") => accepted("op-upload"),
			("GET", "/v1/products/prod-1/submissions/draft/package/operations/op-upload") => {
				// In progress once, then done.
				json(
					serde_json::json!({ "status": if count < 2 { "InProgress" } else { "Succeeded" } }),
				)
			}
			("POST", "/v1/products/prod-1/submissions") => {
				accepted("/v1/products/prod-1/submissions/operations/op-submit")
			}
			("GET", "/v1/products/prod-1/submissions/operations/op-submit") => {
				json(serde_json::json!({ "status": "Succeeded" }))
			}
			_ => StatusCode::NOT_FOUND.into_response(),
		}
	}));
	let http = Http::default();
	let edge = Edge::new(
		&http,
		&base,
		"prod-1",
		Secret::new("client".into()),
		Secret::new("key".into()),
	);
	edge.upload(b"zip".to_vec()).unwrap();
	edge.submit("Version 1.4.0.").unwrap();
	let seen = seen.lock().unwrap();
	assert_eq!(seen[0].authorization, "ApiKey key");
	assert_eq!(seen[0].content_type, "application/zip");
	assert_eq!(
		seen.iter()
			.filter(|request| request.path.ends_with("op-upload"))
			.count(),
		2
	);
	let notes: serde_json::Value =
		serde_json::from_slice(&seen.last().unwrap().body.clone()).unwrap_or_default();
	assert!(notes.is_null() || notes.is_object());
}

#[test]
fn edge_explains_an_expired_key() {
	let (base, _) = store(Box::new(|_, _| StatusCode::UNAUTHORIZED.into_response()));
	let http = Http::default();
	let edge = Edge::new(
		&http,
		&base,
		"p",
		Secret::new("c".into()),
		Secret::new("old".into()),
	);
	let error = edge.upload(b"zip".to_vec()).unwrap_err().to_string();
	assert!(error.contains("72 days"), "{error}");
}

#[test]
fn firefox_validates_then_submits_with_the_source() {
	let (base, seen) = store(Box::new(|request, count| {
		match (request.method.as_str(), request.path.as_str()) {
			("POST", "/addons/upload/") => {
				json(serde_json::json!({ "uuid": "u-1", "processed": false }))
			}
			("GET", "/addons/upload/u-1/") => {
				json(serde_json::json!({ "uuid": "u-1", "processed": count > 1, "valid": true }))
			}
			("POST", "/addons/addon/notes@example.com/versions/") => {
				json(serde_json::json!({ "id": 7, "version": "1.4.0" }))
			}
			_ => StatusCode::NOT_FOUND.into_response(),
		}
	}));
	let http = Http::default();
	let firefox = Firefox::new(
		&http,
		&base,
		"notes@example.com",
		Secret::new("user:1".into()),
		Secret::new("secret".into()),
	);
	let upload = firefox.upload(b"PK package", "listed").unwrap();
	assert_eq!(upload, "u-1");
	let version = firefox.submit(&upload, b"PK source").unwrap();
	assert_eq!(version["version"], "1.4.0");

	let seen = seen.lock().unwrap();
	assert!(seen[0].authorization.starts_with("JWT "));
	assert!(
		seen[0]
			.content_type
			.starts_with("multipart/form-data; boundary=")
	);
	let upload_body = String::from_utf8_lossy(&seen[0].body);
	assert!(upload_body.contains("name=\"channel\"\r\n\r\nlisted"));
	assert!(upload_body.contains("PK package"));
	let version_body = String::from_utf8_lossy(&seen.last().unwrap().body);
	assert!(version_body.contains("name=\"upload\"\r\n\r\nu-1"));
	assert!(version_body.contains("filename=\"firefox-source.zip\""));
	assert!(version_body.contains("PK source"));
}

#[test]
fn firefox_reports_failed_validation_and_a_missing_add_on() {
	let (base, _) = store(Box::new(|request, _| {
		match (request.method.as_str(), request.path.as_str()) {
			("POST", "/addons/upload/") => json(serde_json::json!({ "uuid": "u-2" })),
			("GET", "/addons/upload/u-2/") => {
				json(serde_json::json!({
					"processed": true, "valid": false,
					"validation": { "messages": [{ "type": "error", "message": "manifest.json is invalid" }] }
				}))
			}
			_ => StatusCode::NOT_FOUND.into_response(),
		}
	}));
	let http = Http::default();
	let firefox = Firefox::new(
		&http,
		&base,
		"new@example.com",
		Secret::new("i".into()),
		Secret::new("s".into()),
	);
	let error = firefox.upload(b"zip", "listed").unwrap_err().to_string();
	assert!(error.contains("manifest.json is invalid"), "{error}");
	let error = firefox.submit("u-2", b"source").unwrap_err().to_string();
	assert!(
		error.contains("has no add-on new@example.com yet"),
		"{error}"
	);
}

fn apple_key() -> Option<Key> {
	let pem = super::secrets::tests::ec_key()?;
	Some(
		Key::new(
			Secret::new("issuer-1".into()),
			Secret::new("KEY123".into()),
			&Secret::new(pem),
		)
		.unwrap(),
	)
}

/// The claims of a bearer token.
fn claims(authorization: &str) -> serde_json::Value {
	use base64::Engine as _;

	let token = authorization.strip_prefix("Bearer ").unwrap();
	let claims = token.split('.').nth(1).unwrap();
	serde_json::from_slice(
		&base64::engine::general_purpose::URL_SAFE_NO_PAD
			.decode(claims)
			.unwrap(),
	)
	.unwrap()
}

#[test]
fn app_store_numbers_the_next_build_and_waits_for_processing() {
	let Some(key) = apple_key() else { return };
	let (base, seen) = store(Box::new(|request, count| {
		match (request.method.as_str(), request.path.as_str()) {
			("GET", "/v1/builds") if request.query.contains("sort=-uploadedDate") => {
				json(serde_json::json!({ "data": [
					{ "id": "b1", "attributes": { "version": "7" } },
					{ "id": "b2", "attributes": { "version": "41" } },
					{ "id": "b3", "attributes": { "version": "1.0.3" } },
				]}))
			}
			("GET", "/v1/builds") => {
				// Not listed yet, then processing, then done.
				match count {
					1 => json(serde_json::json!({ "data": [] })),
					2 => {
						json(serde_json::json!({ "data": [
							{ "id": "b4", "attributes": { "processingState": "PROCESSING" } },
						]}))
					}
					_ => {
						json(serde_json::json!({ "data": [
							{ "id": "b4", "attributes": { "processingState": "VALID" } },
						]}))
					}
				}
			}
			_ => StatusCode::NOT_FOUND.into_response(),
		}
	}));
	let http = Http::default();
	let client = AppStore::new(&http, &base, &key);
	assert_eq!(client.highest_build("123").unwrap(), Some(41));
	assert_eq!(client.wait_for_build("123", "1.4.0", 42).unwrap(), "b4");

	let seen = seen.lock().unwrap();
	assert_eq!(
		seen[0].query,
		"filter%5Bapp%5D=123&sort=-uploadedDate&limit=200&fields%5Bbuilds%5D=version"
	);
	assert_eq!(
		seen[1].query,
		"filter%5Bapp%5D=123&filter%5Bversion%5D=42&filter%5BpreReleaseVersion.version%5D=1.4.0&\
		 limit=1"
	);
	let claims = claims(&seen[0].authorization);
	assert_eq!(claims["iss"], "issuer-1");
	assert_eq!(claims["aud"], "appstoreconnect-v1");
	assert_eq!(
		claims["exp"].as_u64().unwrap() - claims["iat"].as_u64().unwrap(),
		1200
	);
}

#[test]
fn app_store_reports_a_build_that_failed_processing() {
	let Some(key) = apple_key() else { return };
	let (base, _) = store(Box::new(|_, _| {
		json(serde_json::json!({ "data": [
			{ "id": "b4", "attributes": { "processingState": "INVALID" } },
		]}))
	}));
	let http = Http::default();
	let error = AppStore::new(&http, &base, &key)
		.wait_for_build("123", "1.4.0", 42)
		.unwrap_err()
		.to_string();
	assert!(error.contains("couldn't process build 42"), "{error}");
}

#[test]
fn app_store_adds_builds_to_testflight_groups() {
	let Some(key) = apple_key() else { return };
	let (base, seen) = store(Box::new(|request, _| {
		match (request.method.as_str(), request.path.as_str()) {
			("GET", "/v1/apps/123/betaGroups") => {
				json(serde_json::json!({ "data": [
					{ "id": "g1", "attributes": { "name": "Team", "isInternalGroup": true, "hasAccessToAllBuilds": true } },
					{ "id": "g2", "attributes": { "name": "QA", "isInternalGroup": true, "hasAccessToAllBuilds": false } },
					{ "id": "g3", "attributes": { "name": "Beta", "isInternalGroup": false } },
				]}))
			}
			("POST", "/v1/builds/b4/relationships/betaGroups") => {
				StatusCode::NO_CONTENT.into_response()
			}
			("POST", "/v1/betaAppReviewSubmissions") => {
				(StatusCode::CONFLICT, "Missing export compliance").into_response()
			}
			_ => StatusCode::NOT_FOUND.into_response(),
		}
	}));
	let http = Http::default();
	let client = AppStore::new(&http, &base, &key);
	let names = ["Team", "QA", "Beta"].map(str::to_owned);
	let groups = client.add_to_groups("123", "b4", &names).unwrap();
	assert_eq!(groups.len(), 3);
	let error = client.submit_for_beta_review("b4").unwrap_err().to_string();
	assert!(error.contains("Missing export compliance"), "{error}");
	let error = client
		.add_to_groups("123", "b4", &["Friends".to_owned()])
		.unwrap_err()
		.to_string();
	assert!(
		error.contains("no TestFlight group `Friends`; it has `Team`, `QA`, `Beta`"),
		"{error}"
	);

	let seen = seen.lock().unwrap();
	let added: serde_json::Value = serde_json::from_slice(&seen[1].body).unwrap();
	// The group that gets every build already has it.
	assert_eq!(
		added,
		serde_json::json!({ "data": [
			{ "type": "betaGroups", "id": "g2" },
			{ "type": "betaGroups", "id": "g3" },
		]})
	);
	let review: serde_json::Value = serde_json::from_slice(&seen[2].body).unwrap();
	assert_eq!(review["data"]["relationships"]["build"]["data"]["id"], "b4");
}

#[test]
fn app_store_submits_a_version_for_review() {
	let Some(key) = apple_key() else { return };
	let (base, seen) = store(Box::new(|request, _| {
		match (request.method.as_str(), request.path.as_str()) {
			("GET", "/v1/apps/123/appStoreVersions") => {
				json(serde_json::json!({ "data": [
					{ "id": "v1", "attributes": { "versionString": "1.0.0", "appStoreState": "READY_FOR_SALE" } },
					{ "id": "v2", "attributes": { "versionString": "1.1.0", "appStoreState": "PREPARE_FOR_SUBMISSION" } },
				]}))
			}
			("PATCH", "/v1/appStoreVersions/v2") => {
				json(serde_json::json!({ "data": { "id": "v2" } }))
			}
			("GET", "/v1/appStoreVersions/v2/appStoreVersionLocalizations") => {
				json(serde_json::json!({ "data": [
					{ "id": "l1", "attributes": { "locale": "en-US" } },
				]}))
			}
			("PATCH", "/v1/appStoreVersionLocalizations/l1") => json(serde_json::json!({})),
			("PATCH", "/v1/appStoreVersions/v2/relationships/build") => {
				StatusCode::NO_CONTENT.into_response()
			}
			("GET", "/v1/reviewSubmissions") => json(serde_json::json!({ "data": [] })),
			("POST", "/v1/reviewSubmissions") => {
				json(serde_json::json!({ "data": { "id": "s1" } }))
			}
			("POST", "/v1/reviewSubmissionItems") => {
				(
					StatusCode::CREATED,
					axum::Json(serde_json::json!({ "data": { "id": "i1" } })),
				)
					.into_response()
			}
			("PATCH", "/v1/reviewSubmissions/s1") => {
				json(
					serde_json::json!({ "data": { "id": "s1", "attributes": { "state": "WAITING_FOR_REVIEW" } } }),
				)
			}
			_ => StatusCode::NOT_FOUND.into_response(),
		}
	}));
	let http = Http::default();
	let client = AppStore::new(&http, &base, &key);
	// The version in preparation takes the new number.
	let (version, first) = client.version("123", "1.2.0").unwrap();
	assert_eq!((version.as_str(), first), ("v2", false));
	client
		.localize(
			&version,
			"en-US",
			&serde_json::json!({ "whatsNew": "Clip from the side panel." }),
		)
		.unwrap();
	client.attach(&version, "b4").unwrap();
	assert_eq!(
		client.submit_for_review("123", &version).unwrap(),
		"waiting for review"
	);

	let seen = seen.lock().unwrap();
	let body =
		|index: usize| -> serde_json::Value { serde_json::from_slice(&seen[index].body).unwrap() };
	assert_eq!(seen[0].query, "filter%5Bplatform%5D=IOS&limit=200");
	assert_eq!(body(1)["data"]["attributes"]["versionString"], "1.2.0");
	assert_eq!(
		body(3)["data"]["attributes"]["whatsNew"],
		"Clip from the side panel."
	);
	assert_eq!(
		body(4),
		serde_json::json!({ "data": { "type": "builds", "id": "b4" } })
	);
	assert_eq!(
		seen[5].query,
		"filter%5Bapp%5D=123&filter%5Bplatform%5D=IOS&filter%5Bstate%5D=READY_FOR_REVIEW"
	);
	assert_eq!(body(6)["data"]["attributes"]["platform"], "IOS");
	let item = body(7);
	assert_eq!(
		item["data"]["relationships"]["reviewSubmission"]["data"]["id"],
		"s1"
	);
	assert_eq!(
		item["data"]["relationships"]["appStoreVersion"]["data"]["id"],
		"v2"
	);
	assert_eq!(body(8)["data"]["attributes"]["submitted"], true);
}

#[test]
fn app_store_creates_a_first_version_and_refuses_a_released_one() {
	let Some(key) = apple_key() else { return };
	let (base, seen) = store(Box::new(|request, _| {
		match (request.method.as_str(), request.path.as_str()) {
			("GET", "/v1/apps/1/appStoreVersions") => json(serde_json::json!({ "data": [] })),
			("POST", "/v1/appStoreVersions") => json(serde_json::json!({ "data": { "id": "v9" } })),
			("GET", "/v1/apps/2/appStoreVersions") => {
				json(serde_json::json!({ "data": [
					{ "id": "v1", "attributes": { "versionString": "1.0.0", "appStoreState": "READY_FOR_SALE" } },
				]}))
			}
			_ => StatusCode::NOT_FOUND.into_response(),
		}
	}));
	let http = Http::default();
	let client = AppStore::new(&http, &base, &key);
	assert_eq!(
		client.version("1", "1.0.0").unwrap(),
		("v9".to_owned(), true)
	);
	let created: serde_json::Value = serde_json::from_slice(&seen.lock().unwrap()[1].body).unwrap();
	assert_eq!(created["data"]["attributes"]["platform"], "IOS");
	assert_eq!(created["data"]["relationships"]["app"]["data"]["id"], "1");
	let error = client.version("2", "1.0.0").unwrap_err().to_string();
	assert!(error.contains("already ready for sale"), "{error}");
}

#[test]
fn app_store_explains_a_refused_key() {
	let Some(key) = apple_key() else { return };
	let (base, _) = store(Box::new(|_, _| StatusCode::UNAUTHORIZED.into_response()));
	let http = Http::default();
	let error = AppStore::new(&http, &base, &key)
		.highest_build("1")
		.unwrap_err()
		.to_string();
	assert!(error.contains("LEPTOSKIT_PUBLISH_APPLE_KEY_ID"), "{error}");
}

/// Every credential, set to a value no output may contain.
#[test]
fn no_credential_value_reaches_any_output() {
	use std::fmt::Write as _;

	let dir = tempfile::tempdir().unwrap();
	let project = tempfile::tempdir().unwrap();
	let file = dir.path().join("credentials.toml");
	let value = |name: &str| format!("value-of-{name}");
	let mut text = String::new();
	let mut names: Vec<&str> = super::NEEDED.iter().map(|needed| needed.name).collect();
	names.sort_unstable();
	names.dedup();
	for name in names {
		let _ = writeln!(text, "{name} = \"{}\"", value(name));
	}
	std::fs::write(&file, text).unwrap();
	let credentials = super::Credentials::load(Some(&file), project.path()).unwrap();
	let stores: Vec<&str> = leptoskit_config::publish::STORES.to_vec();

	let mut outputs = vec![
		super::list_credentials(&stores, &credentials),
		format!("{credentials:?}"),
	];
	for needed in &super::NEEDED {
		let secret = credentials.require(needed.name, needed.how).unwrap();
		outputs.push(format!("{secret} {secret:?}"));
	}
	// The stores' refusals, and keys that can't be used.
	let (base, _) = store(Box::new(|_, _| StatusCode::UNAUTHORIZED.into_response()));
	let http = Http::default();
	let get = |name: &str| credentials.get(name).unwrap();
	let edge = Edge::new(
		&http,
		&base,
		"p",
		get(super::edge::CLIENT_ID),
		get(super::edge::API_KEY),
	);
	outputs.push(format!("{:#}", edge.upload(b"zip".to_vec()).unwrap_err()));
	let firefox = Firefox::new(
		&http,
		&base,
		"a@b",
		get(super::firefox::ISSUER),
		get(super::firefox::SECRET),
	);
	outputs.push(format!(
		"{:#}",
		firefox.upload(b"zip", "listed").unwrap_err()
	));
	outputs.push(format!(
		"{:#}",
		Chrome::connect(
			&http,
			&base,
			&base,
			&get(super::chrome::CREDENTIAL),
			"p",
			"i"
		)
		.err()
		.unwrap()
	));
	let apple = Key::new(
		get(super::app_store::ISSUER),
		get(super::app_store::KEY_ID),
		&get(super::app_store::KEY),
	);
	outputs.push(format!("{:#}", apple.err().unwrap()));

	for output in &outputs {
		for needed in &super::NEEDED {
			assert!(
				!output.contains(&value(needed.name)),
				"{} leaked into: {output}",
				needed.name
			);
		}
	}
	assert!(outputs[0].contains("LEPTOSKIT_PUBLISH_APPLE_KEY "));
}

/// `leptoskit publish --dry-run` on Commonplace: every store's checks and
/// remaining steps, from its real configuration and `store/` folder, with
/// the packages standing in for a build.
#[test]
fn commonplace_dry_run() {
	use leptoskit_config::Browser;

	let example =
		std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/commonplace");
	let config = leptoskit_config::LeptoskitConfig::from_toml(
		&std::fs::read_to_string(example.join("leptoskit.toml")).unwrap(),
		&example,
	)
	.unwrap();
	let store = super::listing::Store::load(&example).unwrap();
	let extension_config = config.extension.clone().unwrap();
	let manifest =
		crate::extension::manifest::manifest(&crate::extension::manifest::ManifestInput {
			browser: Browser::Chromium,
			config: &config,
			extension: &extension_config,
			plugin_permissions: &[],
			icons: &[16, 32, 48, 128],
			dev: None,
		});
	let out = tempfile::tempdir().unwrap();
	// Commonplace's iOS host is `ios/`, as in the example.
	std::fs::create_dir_all(out.path().join("ios")).unwrap();
	std::fs::write(out.path().join("ios/Cargo.toml"), "").unwrap();
	// And its Android host is `android/`, with its own Gradle script.
	std::fs::create_dir_all(out.path().join("android/app")).unwrap();
	std::fs::write(out.path().join("android/Cargo.toml"), "").unwrap();
	std::fs::copy(
		example.join("android/app/build.gradle.kts"),
		out.path().join("android/app/build.gradle.kts"),
	)
	.unwrap();
	let package = |browser: Browser| {
		crate::extension::Package {
			browser,
			dir: out.path().join(browser.name()),
			zip: Some(out.path().join(format!("{}.zip", browser.name()))),
		}
	};
	let extension = super::Extension {
		built: crate::extension::Built {
			packages: vec![package(Browser::Chromium), package(Browser::Firefox)],
			source: Some(out.path().join("firefox-source.zip")),
		},
		manifest,
	};
	let args = super::PublishArgs {
		stores: Vec::new(),
		dry_run: true,
		submit: false,
		staged: false,
		yes: false,
		json: false,
		credentials: None,
		build_number: super::BuildNumber::Auto,
	};
	let credentials = super::Credentials::default();
	let http = Http::default();
	let runner = super::tools::Recorder::default();
	let session = super::Session {
		args: &args,
		root: out.path(),
		config: &config,
		credentials: &credentials,
		store: &store,
		http: &http,
		runner: &runner,
		extension: Some(&extension),
		name: super::display_name(&config),
		version: super::apple_version(&config),
		submit: false,
	};
	let mut report = super::Report::default();
	for name in config.publish.stores() {
		let (summary, steps) = session.publish(name).unwrap();
		report.steps.extend(steps);
		report.stores.push(super::Status {
			store: name.to_owned(),
			summary,
		});
	}
	// A dry run runs no tool and contacts no store.
	assert!(runner.seen.borrow().is_empty());
	assert!(
		out.path()
			.join(".leptoskit/publish/ios/ExportOptions.plist")
			.is_file()
	);
	insta::assert_snapshot!(report.text());
}

fn google_account() -> Option<Secret> {
	let key = rsa_key()?;
	Some(Secret::new(
		serde_json::json!({ "client_email": "ci@project.iam.gserviceaccount.com", "private_key": key })
			.to_string(),
	))
}

#[test]
fn play_numbers_uploads_and_releases_through_an_edit() {
	use super::play::Play;
	use super::play::Rollout;

	let Some(account) = google_account() else {
		return;
	};
	let app = "/androidpublisher/v3/applications/com.example.notes";
	let (base, seen) = store(Box::new(move |request, count| {
		let path = request.path.as_str();
		match (request.method.as_str(), path) {
			("POST", "/token") => json(serde_json::json!({ "access_token": "token-1" })),
			("POST", path) if path == format!("{app}/edits") => {
				json(serde_json::json!({ "id": format!("edit-{count}") }))
			}
			("GET", path) if path.ends_with("/tracks") => {
				json(serde_json::json!({ "tracks": [
					{ "track": "internal", "releases": [{ "versionCodes": ["40", "41"] }] },
					{ "track": "production", "releases": [{ "versionCodes": ["39"] }] },
				]}))
			}
			("GET", path) if path.ends_with("/bundles") => {
				json(serde_json::json!({ "bundles": [{ "versionCode": 38 }] }))
			}
			("DELETE", _) => StatusCode::NO_CONTENT.into_response(),
			("POST", path) if path.ends_with("/bundles") => {
				json(serde_json::json!({ "versionCode": 42, "sha256": "…" }))
			}
			("PUT", path) if path.ends_with("/tracks/production") => {
				json(serde_json::json!({ "track": "production" }))
			}
			("POST", path) if path.ends_with(":commit") => {
				json(serde_json::json!({ "id": "edit" }))
			}
			_ => StatusCode::NOT_FOUND.into_response(),
		}
	}));
	let http = Http::default();
	let play = Play::connect(
		&http,
		&base,
		&format!("{base}/token"),
		&account,
		"com.example.notes",
	)
	.unwrap();
	let edit = play.edit().unwrap().unwrap();
	assert_eq!(edit.highest_version_code().unwrap(), Some(41));
	edit.discard();
	let edit = play.edit().unwrap().unwrap();
	assert_eq!(edit.upload(b"PK aab".to_vec()).unwrap(), 42);
	edit.release(
		"production",
		&super::play::release("1.4.0", 42, Rollout::Staged(0.1), Some("Faster sync.")),
	)
	.unwrap();
	edit.commit().unwrap();

	let seen = seen.lock().unwrap();
	let form = String::from_utf8(seen[0].body.clone()).unwrap();
	let assertion = form.rsplit('=').next().unwrap();
	let claims: serde_json::Value = {
		use base64::Engine as _;
		serde_json::from_slice(
			&base64::engine::general_purpose::URL_SAFE_NO_PAD
				.decode(assertion.split('.').nth(1).unwrap())
				.unwrap(),
		)
		.unwrap()
	};
	assert_eq!(
		claims["scope"],
		"https://www.googleapis.com/auth/androidpublisher"
	);
	assert_eq!(seen[1].authorization, "Bearer token-1");
	assert_eq!(seen[4].method, "DELETE");
	assert_eq!(seen[4].path, format!("{app}/edits/edit-1"));
	let upload = &seen[6];
	assert_eq!(
		upload.path,
		"/upload/androidpublisher/v3/applications/com.example.notes/edits/edit-5/bundles"
	);
	assert_eq!(upload.query, "uploadType=media");
	assert_eq!(upload.content_type, "application/octet-stream");
	assert_eq!(upload.body, b"PK aab");
	let track: serde_json::Value = serde_json::from_slice(&seen[7].body).unwrap();
	assert_eq!(track["track"], "production");
	assert_eq!(track["releases"][0]["status"], "inProgress");
	assert_eq!(track["releases"][0]["userFraction"], 0.1);
	assert_eq!(track["releases"][0]["versionCodes"][0], "42");
	assert_eq!(seen[8].path, format!("{app}/edits/edit-5:commit"));
}

#[test]
fn play_reports_a_missing_app_and_a_service_account_without_access() {
	let Some(account) = google_account() else {
		return;
	};
	let (base, _) = store(Box::new(|request, _| {
		match request.path.as_str() {
			"/token" => json(serde_json::json!({ "access_token": "t" })),
			path if path.contains("com.example.new") => StatusCode::NOT_FOUND.into_response(),
			_ => StatusCode::FORBIDDEN.into_response(),
		}
	}));
	let http = Http::default();
	let connect = |package: &str| {
		super::play::Play::connect(&http, &base, &format!("{base}/token"), &account, package)
			.unwrap()
	};
	assert!(connect("com.example.new").edit().unwrap().is_none());
	let error = connect("com.example.notes")
		.edit()
		.err()
		.unwrap()
		.to_string();
	assert!(error.contains("Users and permissions"), "{error}");
}

#[test]
fn app_store_sets_the_privacy_policy_on_app_information() {
	let Some(key) = apple_key() else { return };
	let (base, seen) = store(Box::new(|request, _| {
		match (request.method.as_str(), request.path.as_str()) {
			("GET", "/v1/apps/123/appInfos") => {
				json(serde_json::json!({ "data": [
					{ "id": "live", "attributes": { "appStoreState": "READY_FOR_SALE" } },
					{ "id": "next", "attributes": { "appStoreState": "PREPARE_FOR_SUBMISSION" } },
				]}))
			}
			("GET", "/v1/appInfos/next/appInfoLocalizations") => {
				json(serde_json::json!({ "data": [
					{ "id": "fr", "attributes": { "locale": "fr-FR" } },
					{ "id": "en", "attributes": { "locale": "en-US" } },
				]}))
			}
			("PATCH", "/v1/appInfoLocalizations/en") => json(serde_json::json!({})),
			_ => StatusCode::NOT_FOUND.into_response(),
		}
	}));
	let http = Http::default();
	assert!(
		AppStore::new(&http, &base, &key)
			.privacy_policy("123", "https://example.com/privacy")
			.unwrap()
	);
	let seen = seen.lock().unwrap();
	let body: serde_json::Value = serde_json::from_slice(&seen[2].body).unwrap();
	assert_eq!(
		body["data"]["attributes"]["privacyPolicyUrl"],
		"https://example.com/privacy"
	);
}

#[test]
fn play_sets_the_store_listing() {
	let Some(account) = google_account() else {
		return;
	};
	let (base, seen) = store(Box::new(|request, _| {
		match (request.method.as_str(), request.path.as_str()) {
			("POST", "/token") => json(serde_json::json!({ "access_token": "t" })),
			("POST", path) if path.ends_with("/edits") => json(serde_json::json!({ "id": "e1" })),
			("PUT", path) if path.ends_with("/edits/e1/listings/en-US") => {
				json(serde_json::json!({}))
			}
			_ => StatusCode::NOT_FOUND.into_response(),
		}
	}));
	let http = Http::default();
	let play = super::play::Play::connect(&http, &base, &format!("{base}/token"), &account, "a.b")
		.unwrap();
	play.edit()
		.unwrap()
		.unwrap()
		.listing("en-US", "Notes", "Keep notes.", "Keep notes, everywhere.")
		.unwrap();
	let body: serde_json::Value = serde_json::from_slice(&seen.lock().unwrap()[2].body).unwrap();
	assert_eq!(
		body,
		serde_json::json!({
			"language": "en-US", "title": "Notes",
			"shortDescription": "Keep notes.", "fullDescription": "Keep notes, everywhere.",
		})
	);
}

#[test]
fn firefox_sets_the_listing_text_it_has() {
	let (base, seen) = store(Box::new(|request, _| {
		match (request.method.as_str(), request.path.as_str()) {
			("PATCH", "/addons/addon/notes@example.com/") => json(serde_json::json!({})),
			_ => StatusCode::NOT_FOUND.into_response(),
		}
	}));
	let http = Http::default();
	let firefox = |guid: &str| {
		Firefox::new(
			&http,
			&base,
			guid,
			Secret::new("i".into()),
			Secret::new("s".into()),
		)
	};
	assert!(
		firefox("notes@example.com")
			.listing(None, Some("Keep notes."), Some("Keep notes, everywhere."))
			.unwrap()
	);
	// An add-on that doesn't exist yet has no listing to set.
	assert!(
		!firefox("new@example.com")
			.listing(None, Some("Keep notes."), None)
			.unwrap()
	);
	let body: serde_json::Value = serde_json::from_slice(&seen.lock().unwrap()[0].body).unwrap();
	assert_eq!(
		body,
		serde_json::json!({
			"summary": { "en-US": "Keep notes." },
			"description": { "en-US": "Keep notes, everywhere." },
		})
	);
}

#[test]
fn a_store_without_its_icon_gets_a_step() {
	let root = tempfile::tempdir().unwrap();
	let config = |icon: &str| {
		leptoskit_config::LeptoskitConfig::from_toml(
			&format!("[app]\nplatforms = [\"web\", \"ios\"]\n{icon}"),
			root.path(),
		)
		.unwrap()
	};
	let without = config("");
	let step = super::icon_step(root.path(), &without, "app_store").unwrap();
	assert!(step.what.contains("set app.icon"));
	assert!(super::icon_step(root.path(), &config("icon = \"icon.png\"\n"), "play").is_none());
	// An extension may bring its own sizes instead.
	assert!(super::icon_step(root.path(), &without, "chrome").is_some());
	std::fs::create_dir_all(root.path().join("assets")).unwrap();
	std::fs::write(root.path().join("assets/icon-128.png"), "png").unwrap();
	assert!(super::icon_step(root.path(), &without, "chrome").is_none());
}
