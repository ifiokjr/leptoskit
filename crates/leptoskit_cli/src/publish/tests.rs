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
