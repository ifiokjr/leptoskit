//! The echo service, under `/_leptoskit/services/echo`:
//!
//! - `headers` answers with the `Cookie` it received and sets a cookie of its
//!   own, so a check can tell whether the edge strips either;
//! - `volume` writes and reads back a file on its `notes` volume;
//! - `app` asks the app's `/readyz` at `LEPTOSKIT_APP_URL`.

use std::io::Read as _;
use std::io::Write as _;
use std::net::TcpStream;
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
	Ok(Router::new().nest(&env.public_prefix(), public))
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
