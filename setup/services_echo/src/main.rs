//! The echo service, under `/_leptoskit/services/echo`:
//!
//! - `headers` answers with the `Cookie` it received and sets a cookie of its
//!   own, so a check can tell whether the edge strips either;
//! - `volume` writes and reads back a file on its `notes` volume;
//! - `app` asks the app's `/readyz` at `LEPTOSKIT_APP_URL`.
//!
//! Every other path answers `404` with [`FALLBACK`], so a check can tell a
//! path the edge sent here from one it sent to the app.

use std::io::Read as _;
use std::io::Write as _;
use std::net::TcpStream;
#[cfg(test)]
use std::path::Path;
use std::path::PathBuf;
use std::time::Duration;

use leptoskit_axum::axum::Router;
use leptoskit_axum::axum::http::HeaderMap;
use leptoskit_axum::axum::http::StatusCode;
use leptoskit_axum::axum::http::header;
use leptoskit_axum::axum::response::IntoResponse;
use leptoskit_axum::axum::routing::get;
use leptoskit_axum::service::ServiceEnv;
use leptoskit_axum::tokio;

leptoskit_axum::service::main!(services_echo, run);

/// What any path the service doesn't serve answers.
const FALLBACK: &str = "echo-fallback\n";

async fn run(env: ServiceEnv) -> Result<Router, String> {
	env.expect_name("echo").map_err(|error| error.to_string())?;
	let volume = env
		.volume("notes")
		.ok_or("the echo service needs its `notes` volume")?
		.to_path_buf();
	let app = env
		.app_url()
		.ok_or("the echo service needs LEPTOSKIT_APP_URL")?
		.to_owned();
	let public = Router::new()
		.route("/headers", get(headers))
		.route("/volume", get(move || note(volume.clone())))
		.route("/app", get(move || app_ready(app.clone())));
	Ok(Router::new()
		.nest(&env.public_prefix(), public)
		.fallback(|| async { (StatusCode::NOT_FOUND, FALLBACK) }))
}

/// The `Cookie` the request carried, or `-`, with a cookie of the service's
/// own on the answer.
async fn headers(request: HeaderMap) -> impl IntoResponse {
	let cookie = request
		.get(header::COOKIE)
		.and_then(|value| value.to_str().ok())
		.unwrap_or("-")
		.to_owned();
	(
		[(header::SET_COOKIE, "echo=set-by-the-service; Path=/")],
		format!("cookie: {cookie}\n"),
	)
}

/// Writes a file on the volume and reads it back.
async fn note(volume: PathBuf) -> (StatusCode, String) {
	let file = volume.join("note.txt");
	let written = tokio::fs::write(&file, "written by the echo service\n").await;
	let read = tokio::fs::read_to_string(&file).await;

	match (written, read) {
		(Ok(()), Ok(text)) => (StatusCode::OK, text),
		(Err(error), _) | (_, Err(error)) => {
			(
				StatusCode::INTERNAL_SERVER_ERROR,
				format!("{}: {error}\n", file.display()),
			)
		}
	}
}

/// The status line of the app's `/readyz`, asked over plain HTTP at its
/// private address.
async fn app_ready(app: String) -> (StatusCode, String) {
	let asked = tokio::task::spawn_blocking(move || readyz(&app)).await;

	match asked {
		Ok(Ok(line)) if line.contains(" 200 ") => (StatusCode::OK, format!("{line}\n")),
		Ok(Ok(line)) => (StatusCode::BAD_GATEWAY, format!("{line}\n")),
		Ok(Err(error)) => (StatusCode::BAD_GATEWAY, format!("{error}\n")),
		Err(error) => (StatusCode::INTERNAL_SERVER_ERROR, format!("{error}\n")),
	}
}

fn readyz(app: &str) -> std::io::Result<String> {
	let authority = app
		.strip_prefix("http://")
		.unwrap_or(app)
		.trim_end_matches('/');
	let mut stream = TcpStream::connect(authority)?;
	stream.set_read_timeout(Some(Duration::from_secs(5)))?;
	write!(
		stream,
		"GET /readyz HTTP/1.1\r\nHost: {authority}\r\nConnection: close\r\n\r\n"
	)?;
	let mut answer = String::new();
	stream.read_to_string(&mut answer)?;
	Ok(answer.lines().next().unwrap_or_default().to_owned())
}

#[cfg(test)]
mod tests {
	use std::io::BufRead as _;
	use std::io::BufReader;
	use std::net::TcpListener;

	use leptoskit_axum::axum::body::Body;
	use leptoskit_axum::axum::body::to_bytes;
	use leptoskit_axum::axum::http::HeaderValue;
	use leptoskit_axum::axum::http::Request;
	use leptoskit_axum::tower::ServiceExt as _;

	use super::*;

	fn temporary(name: &str) -> PathBuf {
		let dir = std::env::temp_dir().join(format!("services_echo_{name}_{}", std::process::id()));
		std::fs::create_dir_all(&dir).unwrap();
		dir
	}

	fn env(name: &str, volume: Option<&Path>, app: Option<&str>) -> ServiceEnv {
		let mut vars = vec![
			("LEPTOSKIT_SERVICE_NAME".to_owned(), name.to_owned()),
			(
				"LEPTOSKIT_SERVICE_ADDR".to_owned(),
				"127.0.0.1:0".to_owned(),
			),
			("LEPTOSKIT_SERVICE_TOKEN".to_owned(), "token".to_owned()),
		];

		if let Some(volume) = volume {
			vars.push((
				"LEPTOSKIT_SERVICE_VOLUME_NOTES".to_owned(),
				volume.display().to_string(),
			));
		}

		if let Some(app) = app {
			vars.push(("LEPTOSKIT_APP_URL".to_owned(), app.to_owned()));
		}

		ServiceEnv::from_vars(vars).unwrap()
	}

	async fn text(response: impl IntoResponse) -> (StatusCode, HeaderMap, String) {
		let response = response.into_response();
		let status = response.status();
		let headers = response.headers().clone();
		let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
		(status, headers, String::from_utf8(body.to_vec()).unwrap())
	}

	/// An app that answers `/readyz` with `status_line`, once.
	fn app(status_line: &'static str) -> String {
		let listener = TcpListener::bind("127.0.0.1:0").unwrap();
		let address = listener.local_addr().unwrap();
		std::thread::spawn(move || {
			let (mut stream, _) = listener.accept().unwrap();
			let mut reader = BufReader::new(&stream);
			let mut request = String::new();
			reader.read_line(&mut request).unwrap();
			assert_eq!(request, "GET /readyz HTTP/1.1\r\n");
			// The rest of the head, so closing doesn't reset the connection.
			let mut line = String::new();
			while reader.read_line(&mut line).unwrap() > 0 && line != "\r\n" {
				line.clear();
			}
			write!(stream, "{status_line}\r\nContent-Length: 0\r\n\r\n").unwrap();
		});
		format!("http://{address}/")
	}

	#[tokio::test]
	async fn headers_reports_the_cookie_and_sets_its_own() {
		let mut request = HeaderMap::new();
		request.insert(header::COOKIE, HeaderValue::from_static("session=abc"));
		let (status, answer, body) = text(headers(request).await).await;
		assert_eq!(status, StatusCode::OK);
		assert_eq!(body, "cookie: session=abc\n");
		assert_eq!(
			answer[header::SET_COOKIE],
			"echo=set-by-the-service; Path=/"
		);

		let (_, _, body) = text(headers(HeaderMap::new()).await).await;
		assert_eq!(body, "cookie: -\n");
	}

	#[tokio::test]
	async fn note_writes_and_reads_back_its_file() {
		let volume = temporary("note");
		let (status, _, body) = text(note(volume.clone()).await).await;
		assert_eq!(
			(status, body.as_str()),
			(StatusCode::OK, "written by the echo service\n")
		);

		let (status, _, body) = text(note(volume.join("missing")).await).await;
		assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
		assert!(body.contains("note.txt"), "{body}");
		std::fs::remove_dir_all(volume).unwrap();
	}

	#[tokio::test]
	async fn app_ready_reports_the_apps_answer() {
		let (status, _, body) = text(app_ready(app("HTTP/1.1 200 OK")).await).await;
		assert_eq!(
			(status, body.as_str()),
			(StatusCode::OK, "HTTP/1.1 200 OK\n")
		);

		let (status, _, body) =
			text(app_ready(app("HTTP/1.1 503 Service Unavailable")).await).await;
		assert_eq!(status, StatusCode::BAD_GATEWAY);
		assert_eq!(body, "HTTP/1.1 503 Service Unavailable\n");

		// Nothing listens on a port that was just released.
		let closed = TcpListener::bind("127.0.0.1:0")
			.unwrap()
			.local_addr()
			.unwrap();
		let (status, ..) = text(app_ready(format!("http://{closed}")).await).await;
		assert_eq!(status, StatusCode::BAD_GATEWAY);
	}

	#[tokio::test]
	async fn the_router_serves_under_the_prefix_and_marks_everything_else() {
		let volume = temporary("router");
		let router = run(env("echo", Some(&volume), Some("http://127.0.0.1:1")))
			.await
			.unwrap();
		let request = |path: &str| Request::get(path).body(Body::empty()).unwrap();

		let (status, _, body) = text(
			router
				.clone()
				.oneshot(request("/_leptoskit/services/echo/volume"))
				.await
				.unwrap(),
		)
		.await;
		assert_eq!(
			(status, body.as_str()),
			(StatusCode::OK, "written by the echo service\n")
		);

		for path in ["/headers", "/_leptoskit/services/echo/missing", "/"] {
			let (status, _, body) =
				text(router.clone().oneshot(request(path)).await.unwrap()).await;
			assert_eq!(
				(status, body.as_str()),
				(StatusCode::NOT_FOUND, FALLBACK),
				"{path}"
			);
		}
		std::fs::remove_dir_all(volume).unwrap();
	}

	#[tokio::test]
	async fn run_needs_its_name_volume_and_app() {
		let volume = temporary("run");
		let app = Some("http://127.0.0.1:1");
		assert!(run(env("other", Some(&volume), app)).await.is_err());
		assert!(run(env("echo", None, app)).await.is_err());
		assert!(run(env("echo", Some(&volume), None)).await.is_err());
		std::fs::remove_dir_all(volume).unwrap();
	}
}
