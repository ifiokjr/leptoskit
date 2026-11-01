//! The local `DevTools` hub.
//!
//! `leptoskit dev` starts it on loopback unless `--no-devtools` is passed.
//! Agents open a session with a [`Hello`], post events, and long-poll for
//! commands. Front ends read the mirrored state, stream changes, and send
//! commands. The hub refuses a session on another protocol version, binds
//! only loopback, and checks `Host`, `Origin`, `Sec-Fetch-Site`, and the JSON
//! content type on every request.
//!
//! The hub serves the interface at `/` (see [`ui`]): a local build, the
//! archive downloaded for this CLI's version (see [`fetch`]), or a plain
//! page.

pub mod fetch;
pub mod ui;

use std::collections::BTreeMap;
use std::convert::Infallible;
use std::net::Ipv4Addr;
use std::net::SocketAddr;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

use anyhow::Context;
use anyhow::Result;
use axum::Json;
use axum::Router;
use axum::extract::Path as AxumPath;
use axum::extract::Query;
use axum::extract::Request;
use axum::extract::State;
use axum::http::HeaderValue;
use axum::http::Method;
use axum::http::StatusCode;
use axum::http::Uri;
use axum::http::header;
use axum::middleware::Next;
use axum::response::IntoResponse;
use axum::response::Response;
use axum::response::sse::Event as SseEvent;
use axum::response::sse::KeepAlive;
use axum::response::sse::Sse;
use axum::routing::get;
use axum::routing::post;
use futures::stream;
use leptoskit_devtools_protocol::Change;
use leptoskit_devtools_protocol::Command;
use leptoskit_devtools_protocol::Event;
use leptoskit_devtools_protocol::HISTORY;
use leptoskit_devtools_protocol::PROTOCOL_VERSION;
use leptoskit_devtools_protocol::SessionId;
use leptoskit_devtools_protocol::SessionInfo;
use leptoskit_devtools_protocol::SessionState;
pub use leptoskit_devtools_protocol::SessionView;
use leptoskit_devtools_protocol::VersionMismatch;
use serde::Deserialize;
use serde::Serialize;
use tokio::sync::Notify;
use tokio::sync::RwLock;
use tokio::sync::broadcast;
use uuid::Uuid;

/// The port the hub binds when `--devtools-port` is not passed.
pub const DEFAULT_PORT: u16 = 7340;

/// How long one command long-poll waits.
const POLL: Duration = Duration::from_secs(25);

/// How long a session lives after its last request.
const EXPIRY: Duration = Duration::from_secs(60);

/// The running hub.
pub struct Hub {
	url: String,
	pid: u32,
}

impl Hub {
	/// The hub's loopback URL.
	#[must_use]
	pub fn url(&self) -> &str {
		&self.url
	}

	/// The process that owns the hub.
	#[must_use]
	pub fn pid(&self) -> u32 {
		self.pid
	}
}

/// Start the hub on loopback and write `.leptoskit/devtools.json`.
///
/// # Errors
///
/// When the port is taken or the config file cannot be written.
pub fn start(
	root: &Path,
	runtime: &tokio::runtime::Runtime,
	port: Option<u16>,
	web_port: u16,
) -> Result<Hub> {
	let listener = std::net::TcpListener::bind(SocketAddr::from((
		Ipv4Addr::LOCALHOST,
		port.unwrap_or(DEFAULT_PORT),
	)))
	.context("could not bind the devtools hub")?;
	listener.set_nonblocking(true)?;
	let address = listener.local_addr()?;
	let url = format!("http://{address}");
	write_config(root, &url, std::process::id())?;
	let mut state = HubState::new(address.port(), web_port, POLL, EXPIRY);
	root.clone_into(&mut state.root);
	state.interface = ui::Sources::from_env();
	fetch::on_first_use(&state.interface);
	let state = Arc::new(state);
	let router = router(state.clone());
	let listener = {
		let _guard = runtime.enter();
		tokio::net::TcpListener::from_std(listener)?
	};
	runtime.spawn(async move {
		if let Err(error) = axum::serve(listener, router).await {
			eprintln!("[devtools] hub stopped: {error}");
		}
	});
	state.spawn_expiry(runtime);
	println!("[devtools] hub at {url}");
	Ok(Hub {
		url,
		pid: std::process::id(),
	})
}

/// Write the file `leptoskit mcp` and the panel use to find the hub.
fn write_config(root: &Path, url: &str, pid: u32) -> Result<()> {
	let file = root.join(".leptoskit/devtools.json");

	if let Some(parent) = file.parent() {
		std::fs::create_dir_all(parent)?;
	}

	let config = serde_json::json!({ "url": url, "pid": pid });
	std::fs::write(&file, serde_json::to_vec_pretty(&config)?)
		.with_context(|| format!("could not write {}", file.display()))
}

/// The hub's state.
struct HubState {
	port: u16,
	web_port: u16,
	poll: Duration,
	expiry: Duration,
	sessions: RwLock<BTreeMap<SessionId, Session>>,
	changes: broadcast::Sender<Change>,
	/// The project `leptoskit dev` runs in, so the interface can open source
	/// files in an editor.
	root: PathBuf,
	/// Where the interface is looked up.
	interface: ui::Sources,
}

impl HubState {
	fn new(port: u16, web_port: u16, poll: Duration, expiry: Duration) -> Self {
		let (changes, _) = broadcast::channel(HISTORY);
		Self {
			port,
			web_port,
			poll,
			expiry,
			sessions: RwLock::new(BTreeMap::new()),
			changes,
			root: PathBuf::new(),
			interface: ui::Sources::default(),
		}
	}

	fn spawn_expiry(self: &Arc<Self>, runtime: &tokio::runtime::Runtime) {
		let state = Arc::clone(self);
		runtime.spawn(async move {
			let mut tick = tokio::time::interval(Duration::from_secs(10));
			loop {
				tick.tick().await;
				let now = now_ms();
				let expiry = u64::try_from(state.expiry.as_millis()).unwrap_or(u64::MAX);
				let mut ended = Vec::new();
				state.sessions.write().await.retain(|id, session| {
					let alive = now.saturating_sub(session.info.last_seen_ms) <= expiry;
					if !alive {
						ended.push(id.clone());
					}

					alive
				});

				for session in ended {
					state.broadcast(Change::Closed { session });
				}
			}
		});
	}

	fn broadcast(&self, change: Change) {
		let _ = self.changes.send(change);
	}
}

/// One agent session and its mirrored state.
struct Session {
	info: SessionInfo,
	view: SessionView,
	commands: Vec<Command>,
	notify: Arc<Notify>,
}

/// What `GET /api/hub` answers: who the hub is and what it serves.
#[derive(Clone, Debug, Serialize)]
struct HubInfo {
	/// The CLI's version, which is also the interface's.
	version: &'static str,
	/// The protocol the hub speaks.
	protocol: u32,
	/// The project's root, for opening source files in an editor.
	root: PathBuf,
	/// Which interface the hub serves: `local`, `downloaded`, or `missing`.
	interface: &'static str,
}

#[derive(Deserialize)]
struct After {
	#[serde(default)]
	after: usize,
}

/// A request the hub refuses.
struct ApiError {
	status: StatusCode,
	message: String,
}

impl ApiError {
	fn new(status: StatusCode, message: impl Into<String>) -> Self {
		Self {
			status,
			message: message.into(),
		}
	}

	fn not_found() -> Self {
		Self::new(StatusCode::NOT_FOUND, "no such session")
	}
}

impl IntoResponse for ApiError {
	fn into_response(self) -> Response {
		(
			self.status,
			Json(serde_json::json!({ "error": self.message })),
		)
			.into_response()
	}
}

impl From<VersionMismatch> for ApiError {
	fn from(error: VersionMismatch) -> Self {
		Self::new(StatusCode::BAD_REQUEST, error.to_string())
	}
}

fn router(state: Arc<HubState>) -> Router {
	Router::new()
		.route("/api/hub", get(hub))
		.route("/agent/sessions", post(create_session))
		.route("/agent/sessions/{id}/events", post(events))
		.route("/agent/sessions/{id}/commands", get(commands))
		.route("/api/sessions", get(sessions))
		.route("/api/sessions/{id}", get(session))
		.route("/api/sessions/{id}/commands", post(send_command))
		.route("/api/stream", get(sse))
		.fallback(interface)
		.layer(axum::middleware::from_fn_with_state(
			Arc::clone(&state),
			secure,
		))
		.with_state(state)
}

/// The interface's files, for any `GET` the API does not answer.
async fn interface(State(state): State<Arc<HubState>>, method: Method, uri: Uri) -> Response {
	if method != Method::GET && method != Method::HEAD {
		return StatusCode::METHOD_NOT_ALLOWED.into_response();
	}

	if is_api(uri.path()) {
		return ApiError::new(StatusCode::NOT_FOUND, "no such route").into_response();
	}

	state.interface.resolve().respond(uri.path())
}

async fn hub(State(state): State<Arc<HubState>>) -> Json<HubInfo> {
	Json(HubInfo {
		version: env!("CARGO_PKG_VERSION"),
		protocol: PROTOCOL_VERSION,
		root: state.root.clone(),
		interface: state.interface.resolve().kind(),
	})
}

async fn create_session(
	State(state): State<Arc<HubState>>,
	Json(hello): Json<leptoskit_devtools_protocol::Hello>,
) -> Result<Json<SessionId>, ApiError> {
	hello.check_protocol()?;
	let id = SessionId::new(Uuid::new_v4().to_string());
	let now = now_ms();
	let info = SessionInfo {
		id: id.clone(),
		hello,
		connected_at_ms: now,
		last_seen_ms: now,
	};
	let session = Session {
		info: info.clone(),
		view: SessionView::default(),
		commands: Vec::new(),
		notify: Arc::new(Notify::new()),
	};
	state.sessions.write().await.insert(id.clone(), session);
	state.broadcast(Change::Opened { info });
	Ok(Json(id))
}

async fn events(
	State(state): State<Arc<HubState>>,
	AxumPath(id): AxumPath<String>,
	Json(events): Json<Vec<Event>>,
) -> Result<StatusCode, ApiError> {
	let id = SessionId(id);
	{
		let mut sessions = state.sessions.write().await;
		let session = sessions.get_mut(&id).ok_or_else(ApiError::not_found)?;
		session.info.last_seen_ms = now_ms();

		for event in events {
			session.view.apply(&event);
			state.broadcast(Change::Event {
				session: id.clone(),
				event,
			});
		}
	}
	Ok(StatusCode::NO_CONTENT)
}

async fn commands(
	State(state): State<Arc<HubState>>,
	AxumPath(id): AxumPath<String>,
	Query(after): Query<After>,
) -> Result<Json<Vec<Command>>, ApiError> {
	let id = SessionId(id);
	let deadline = tokio::time::Instant::now() + state.poll;

	loop {
		let (commands, notify) = {
			let mut sessions = state.sessions.write().await;
			let session = sessions.get_mut(&id).ok_or_else(ApiError::not_found)?;
			session.info.last_seen_ms = now_ms();
			let commands: Vec<Command> =
				session.commands.iter().skip(after.after).cloned().collect();
			(commands, Arc::clone(&session.notify))
		};

		if !commands.is_empty() {
			return Ok(Json(commands));
		}

		if tokio::time::timeout_at(deadline, notify.notified())
			.await
			.is_err()
		{
			return Ok(Json(Vec::new()));
		}
	}
}

async fn sessions(State(state): State<Arc<HubState>>) -> Json<Vec<SessionInfo>> {
	let sessions = state.sessions.read().await;
	Json(
		sessions
			.values()
			.map(|session| session.info.clone())
			.collect(),
	)
}

async fn session(
	State(state): State<Arc<HubState>>,
	AxumPath(id): AxumPath<String>,
) -> Result<Json<SessionState>, ApiError> {
	let sessions = state.sessions.read().await;
	let session = sessions
		.get(&SessionId(id))
		.ok_or_else(ApiError::not_found)?;
	Ok(Json(SessionState {
		info: session.info.clone(),
		view: session.view.clone(),
	}))
}

async fn send_command(
	State(state): State<Arc<HubState>>,
	AxumPath(id): AxumPath<String>,
	Json(command): Json<Command>,
) -> Result<StatusCode, ApiError> {
	let id = SessionId(id);
	let mut sessions = state.sessions.write().await;
	let session = sessions.get_mut(&id).ok_or_else(ApiError::not_found)?;
	session.commands.push(command.clone());
	session.notify.notify_waiters();
	state.broadcast(Change::Command {
		session: id,
		command,
	});
	Ok(StatusCode::NO_CONTENT)
}

async fn sse(
	State(state): State<Arc<HubState>>,
) -> Sse<impl futures::Stream<Item = Result<SseEvent, Infallible>>> {
	let stream = stream::unfold(state.changes.subscribe(), next_change);
	Sse::new(stream).keep_alive(KeepAlive::default())
}

/// The next change as a server-sent event. Lagged changes are skipped; the
/// stream ends when the hub's broadcast channel closes.
async fn next_change(
	mut receiver: broadcast::Receiver<Change>,
) -> Option<(Result<SseEvent, Infallible>, broadcast::Receiver<Change>)> {
	loop {
		match receiver.recv().await {
			Ok(change) => {
				if let Ok(event) = SseEvent::default().json_data(&change) {
					return Some((Ok(event), receiver));
				}
			}
			Err(broadcast::error::RecvError::Lagged(_)) => {}
			Err(broadcast::error::RecvError::Closed) => return None,
		}
	}
}

/// Refuse requests a web page could use to reach the hub.
async fn secure(State(state): State<Arc<HubState>>, request: Request, next: Next) -> Response {
	let headers = request.headers();
	let host = headers
		.get(header::HOST)
		.and_then(|value| value.to_str().ok())
		.unwrap_or_default();

	if !allowed_host(host, state.port) {
		return forbidden("the Host header is not this hub");
	}

	let origin = headers
		.get(header::ORIGIN)
		.and_then(|value| value.to_str().ok())
		.map(str::to_owned);

	if let Some(origin) = &origin
		&& !allowed_origin(origin, state.port, state.web_port)
	{
		return forbidden("the Origin is not allowed");
	}

	if headers
		.get("sec-fetch-site")
		.and_then(|value| value.to_str().ok())
		== Some("cross-site")
		&& origin.is_none()
		&& !opens_interface(&request)
	{
		return forbidden("cross-site requests need an allowed Origin");
	}
	let is_write = matches!(
		request.method(),
		&Method::POST | &Method::PUT | &Method::PATCH
	);

	if is_write && !json_content(headers) {
		return (
			StatusCode::UNSUPPORTED_MEDIA_TYPE,
			"POST requests need application/json",
		)
			.into_response();
	}

	if request.method() == Method::OPTIONS {
		let mut response = StatusCode::NO_CONTENT.into_response();
		cors(&mut response, origin.as_deref());

		return response;
	}

	let mut response = next.run(request).await;
	cors(&mut response, origin.as_deref());
	response
}

/// Whether `path` is the API's, which only agents and front ends call.
fn is_api(path: &str) -> bool {
	path.starts_with("/api/") || path.starts_with("/agent/")
}

/// Whether `request` loads one of the interface's pages into a window or a
/// frame, as the browser `DevTools` panel does from its extension page. A
/// browser sends that navigation cross-site with no `Origin`, and shows the
/// page only where `frame-ancestors` allows. The pages are the interface's
/// own files: the data stays behind the API's checks.
fn opens_interface(request: &Request) -> bool {
	matches!(request.method(), &Method::GET | &Method::HEAD)
		&& request
			.headers()
			.get("sec-fetch-mode")
			.and_then(|value| value.to_str().ok())
			== Some("navigate")
		&& !is_api(request.uri().path())
}

fn json_content(headers: &axum::http::HeaderMap) -> bool {
	headers
		.get(header::CONTENT_TYPE)
		.and_then(|value| value.to_str().ok())
		.is_some_and(|value| {
			value
				.split(';')
				.next()
				.is_some_and(|kind| kind.trim().eq_ignore_ascii_case("application/json"))
		})
}

fn forbidden(message: &str) -> Response {
	(StatusCode::FORBIDDEN, message.to_owned()).into_response()
}

fn cors(response: &mut Response, origin: Option<&str>) {
	let Some(origin) = origin else {
		return;
	};

	if let Ok(origin) = HeaderValue::from_str(origin) {
		let headers = response.headers_mut();
		headers.insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, origin);
		headers.insert(header::VARY, HeaderValue::from_static("Origin"));
		headers.insert(
			header::ACCESS_CONTROL_ALLOW_METHODS,
			HeaderValue::from_static("GET, POST, OPTIONS"),
		);
		headers.insert(
			header::ACCESS_CONTROL_ALLOW_HEADERS,
			HeaderValue::from_static("content-type"),
		);
	}
}

fn allowed_host(host: &str, port: u16) -> bool {
	[
		format!("127.0.0.1:{port}"),
		format!("localhost:{port}"),
		format!("[::1]:{port}"),
	]
	.iter()
	.any(|allowed| allowed == host)
}

fn allowed_origin(origin: &str, port: u16, web_port: u16) -> bool {
	if origin.starts_with("chrome-extension://") || origin.starts_with("moz-extension://") {
		return true;
	}

	[
		format!("http://127.0.0.1:{web_port}"),
		format!("http://localhost:{web_port}"),
		format!("http://[::1]:{web_port}"),
		format!("http://127.0.0.1:{port}"),
		format!("http://localhost:{port}"),
		format!("http://[::1]:{port}"),
	]
	.iter()
	.any(|allowed| allowed == origin)
}

fn now_ms() -> u64 {
	SystemTime::now()
		.duration_since(UNIX_EPOCH)
		.map_or(0, |duration| {
			u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
		})
}

#[cfg(test)]
mod tests {
	use axum::body::Body;
	use axum::body::to_bytes;
	use axum::http::Request as HttpRequest;
	use leptoskit_devtools_protocol::Capability;
	use leptoskit_devtools_protocol::Hello;
	use leptoskit_devtools_protocol::MatchKey;
	use leptoskit_devtools_protocol::Node;
	use leptoskit_devtools_protocol::NodeRef;
	use leptoskit_devtools_protocol::Platform;
	use leptoskit_devtools_protocol::Snapshot;
	use leptoskit_devtools_protocol::SourceLocation;
	use leptoskit_devtools_protocol::Tree;
	use leptoskit_devtools_protocol::Value;
	use tower::ServiceExt;

	use super::*;

	const PORT: u16 = 7340;
	const WEB: u16 = 3000;

	fn state() -> Arc<HubState> {
		Arc::new(HubState::new(
			PORT,
			WEB,
			Duration::from_millis(100),
			Duration::from_secs(60),
		))
	}

	fn app(state: &Arc<HubState>) -> Router {
		router(Arc::clone(state))
	}

	fn hello() -> Hello {
		Hello::new(
			"commonplace",
			Platform::Ios,
			Some("iPhone 17".into()),
			[Capability::Tree, Capability::Queries],
		)
	}

	fn request(method: &str, uri: &str, body: Option<serde_json::Value>) -> HttpRequest<Body> {
		let mut builder = HttpRequest::builder()
			.method(method)
			.uri(uri)
			.header(header::HOST, format!("127.0.0.1:{PORT}"))
			.header(header::ORIGIN, format!("http://127.0.0.1:{WEB}"));

		let body = match body {
			Some(body) => {
				builder = builder.header(header::CONTENT_TYPE, "application/json");
				Body::from(body.to_string())
			}
			None => Body::empty(),
		};

		builder.body(body).unwrap()
	}

	async fn body<T: serde::de::DeserializeOwned>(response: Response) -> T {
		let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
		serde_json::from_slice(&bytes).unwrap()
	}

	#[tokio::test]
	async fn the_plain_page_is_served_without_an_interface_build() {
		let state = state();
		let response = app(&state)
			.oneshot(request("GET", "/?embed=panel", None))
			.await
			.unwrap();
		assert_eq!(response.status(), StatusCode::OK);
		assert_eq!(
			response.headers()[header::CONTENT_SECURITY_POLICY],
			ui::FRAME_ANCESTORS
		);
		let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
		let page = String::from_utf8(bytes.to_vec()).unwrap();
		assert!(page.contains("id=\"sessions\""));
		assert!(page.contains("/api/stream"));

		assert!(page.contains("request_snapshot"));
		assert!(page.contains("id=\"notice\""));
	}

	#[tokio::test]
	async fn a_local_interface_build_is_served() {
		let dir = tempfile::tempdir().unwrap();
		std::fs::write(dir.path().join("index.html"), "<html>ui</html>").unwrap();
		let mut hub = HubState::new(
			PORT,
			WEB,
			Duration::from_millis(100),
			Duration::from_secs(60),
		);
		hub.interface.local = Some(dir.path().to_owned());
		let state = Arc::new(hub);
		let response = app(&state)
			.oneshot(request("GET", "/", None))
			.await
			.unwrap();
		assert_eq!(response.status(), StatusCode::OK);
		let response = app(&state)
			.oneshot(request("GET", "/api/hub", None))
			.await
			.unwrap();

		let info: serde_json::Value = body(response).await;
		assert_eq!(info["interface"], "local");
		assert_eq!(info["version"], env!("CARGO_PKG_VERSION"));
		assert_eq!(info["protocol"], PROTOCOL_VERSION);
		let response = app(&state)
			.oneshot(request("GET", "/api/nothing", None))
			.await
			.unwrap();
		assert_eq!(response.status(), StatusCode::NOT_FOUND);
	}

	#[tokio::test]
	async fn opening_a_session_is_streamed() {
		let state = state();
		let mut changes = state.changes.subscribe();
		let id: SessionId = body(
			app(&state)
				.oneshot(request(
					"POST",
					"/agent/sessions",
					Some(serde_json::to_value(hello()).unwrap()),
				))
				.await
				.unwrap(),
		)
		.await;
		let Change::Opened { info } = changes.recv().await.unwrap() else {
			panic!("a new session is announced first");
		};
		assert_eq!(info.id, id);
	}

	#[tokio::test]
	async fn a_session_opens_and_is_listed() {
		let state = state();
		let response = app(&state)
			.oneshot(request(
				"POST",
				"/agent/sessions",
				Some(serde_json::to_value(hello()).unwrap()),
			))
			.await
			.unwrap();
		assert_eq!(response.status(), StatusCode::OK);
		let id: SessionId = body(response).await;
		let response = app(&state)
			.oneshot(request("GET", "/api/sessions", None))
			.await
			.unwrap();
		let sessions: Vec<SessionInfo> = body(response).await;
		assert_eq!(sessions.len(), 1);
		assert_eq!(sessions[0].id, id);

		assert_eq!(sessions[0].hello.platform, Platform::Ios);
	}

	#[tokio::test]
	async fn a_wrong_protocol_is_refused_with_both_versions() {
		let state = state();
		let mut hello = hello();
		hello.protocol = PROTOCOL_VERSION + 1;
		let response = app(&state)
			.oneshot(request(
				"POST",
				"/agent/sessions",
				Some(serde_json::to_value(hello).unwrap()),
			))
			.await
			.unwrap();
		assert_eq!(response.status(), StatusCode::BAD_REQUEST);
		let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
		let message = String::from_utf8(bytes.to_vec()).unwrap();
		assert!(message.contains(&(PROTOCOL_VERSION + 1).to_string()));

		assert!(message.contains(&PROTOCOL_VERSION.to_string()));
	}

	#[tokio::test]
	async fn events_are_mirrored() {
		let state = state();
		let id: SessionId = body(
			app(&state)
				.oneshot(request(
					"POST",
					"/agent/sessions",
					Some(serde_json::to_value(hello()).unwrap()),
				))
				.await
				.unwrap(),
		)
		.await;
		let tree = Tree {
			root: NodeRef::Native(1),
			nodes: vec![Node {
				id: NodeRef::Native(1),
				key: MatchKey::new("root"),
				parent: None,
				kind: "View".into(),
				source: Some(SourceLocation {
					file: "src/lib.rs".into(),
					line: 1,
					column: 1,
				}),
				frame: None,
				classes: Vec::new(),
				text: None,
				children: Vec::new(),
				component: None,
				styles: Vec::new(),
			}],
		};
		let snapshot = Snapshot {
			tree,
			route: Some("/clips".into()),
			queries: vec![leptoskit_devtools_protocol::NamedValue {
				name: "clips".into(),
				value: Value::from_json("Vec<Clip>", serde_json::json!([])),
			}],
			mutations: Vec::new(),
			forms: Vec::new(),
			state: Vec::new(),
			calls: Vec::new(),
			rpc: Vec::new(),
		};
		let events = vec![Event::Snapshot { snapshot }];
		let response = app(&state)
			.oneshot(request(
				"POST",
				&format!("/agent/sessions/{}/events", id.0),
				Some(serde_json::to_value(events).unwrap()),
			))
			.await
			.unwrap();
		assert_eq!(response.status(), StatusCode::NO_CONTENT);
		let response = app(&state)
			.oneshot(request("GET", &format!("/api/sessions/{}", id.0), None))
			.await
			.unwrap();

		let view: SessionState = body(response).await;
		assert_eq!(view.view.route.as_deref(), Some("/clips"));
		assert_eq!(view.view.tree.as_ref().unwrap().nodes.len(), 1);
		assert_eq!(view.view.queries.len(), 1);
	}

	#[tokio::test]
	async fn a_command_is_delivered_once_and_then_waits() {
		let state = state();
		let id: SessionId = body(
			app(&state)
				.oneshot(request(
					"POST",
					"/agent/sessions",
					Some(serde_json::to_value(hello()).unwrap()),
				))
				.await
				.unwrap(),
		)
		.await;
		let response = app(&state)
			.oneshot(request(
				"POST",
				&format!("/api/sessions/{}/commands", id.0),
				Some(serde_json::to_value(Command::RequestSnapshot).unwrap()),
			))
			.await
			.unwrap();
		assert_eq!(response.status(), StatusCode::NO_CONTENT);
		let response = app(&state)
			.oneshot(request(
				"GET",
				&format!("/agent/sessions/{}/commands?after=0", id.0),
				None,
			))
			.await
			.unwrap();
		let commands: Vec<Command> = body(response).await;
		assert_eq!(commands, [Command::RequestSnapshot]);
		let response = app(&state)
			.oneshot(request(
				"GET",
				&format!("/agent/sessions/{}/commands?after=1", id.0),
				None,
			))
			.await
			.unwrap();

		let commands: Vec<Command> = body(response).await;
		assert_eq!(commands, Vec::<Command>::new());
	}

	#[tokio::test]
	async fn a_foreign_host_is_refused() {
		let state = state();
		let request = HttpRequest::builder()
			.method("GET")
			.uri("/api/sessions")
			.header(header::HOST, "evil.example:7340")
			.body(Body::empty())
			.unwrap();
		let response = app(&state).oneshot(request).await.unwrap();
		assert_eq!(response.status(), StatusCode::FORBIDDEN);
	}

	#[tokio::test]
	async fn a_foreign_origin_is_refused() {
		let state = state();
		let request = HttpRequest::builder()
			.method("GET")
			.uri("/api/sessions")
			.header(header::HOST, format!("127.0.0.1:{PORT}"))
			.header(header::ORIGIN, "https://evil.example")
			.body(Body::empty())
			.unwrap();
		let response = app(&state).oneshot(request).await.unwrap();
		assert_eq!(response.status(), StatusCode::FORBIDDEN);
	}

	#[tokio::test]
	async fn a_cross_site_request_without_an_origin_is_refused() {
		let state = state();
		let request = HttpRequest::builder()
			.method("GET")
			.uri("/api/sessions")
			.header(header::HOST, format!("127.0.0.1:{PORT}"))
			.header("sec-fetch-site", "cross-site")
			.body(Body::empty())
			.unwrap();
		let response = app(&state).oneshot(request).await.unwrap();
		assert_eq!(response.status(), StatusCode::FORBIDDEN);
	}

	/// A request as a browser sends it for a page on another site, with the
	/// fetch metadata headers it sets.
	fn cross_site(uri: &str, mode: &str, dest: &str) -> HttpRequest<Body> {
		HttpRequest::builder()
			.method("GET")
			.uri(uri)
			.header(header::HOST, format!("127.0.0.1:{PORT}"))
			.header("sec-fetch-site", "cross-site")
			.header("sec-fetch-mode", mode)
			.header("sec-fetch-dest", dest)
			.body(Body::empty())
			.unwrap()
	}

	#[tokio::test]
	async fn the_devtools_panel_frames_the_interface() {
		let state = state();
		// What Chromium sends when the panel's extension page frames the
		// interface: a cross-site navigation with no Origin.
		let response = app(&state)
			.oneshot(cross_site("/?embed=panel", "navigate", "iframe"))
			.await
			.unwrap();
		assert_eq!(response.status(), StatusCode::OK);
		let policy = response.headers()[header::CONTENT_SECURITY_POLICY]
			.to_str()
			.unwrap();

		for ancestor in ["chrome-extension:", "moz-extension:", "devtools:"] {
			assert!(
				policy.split(' ').any(|source| source == ancestor),
				"{policy}"
			);
		}

		// The panel's own look for the hub carries no Origin either.
		let request = HttpRequest::builder()
			.method("GET")
			.uri("/api/hub")
			.header(header::HOST, format!("127.0.0.1:{PORT}"))
			.header("sec-fetch-site", "none")
			.header("sec-fetch-mode", "cors")
			.body(Body::empty())
			.unwrap();
		let response = app(&state).oneshot(request).await.unwrap();
		assert_eq!(response.status(), StatusCode::OK);
	}

	#[tokio::test]
	async fn other_sites_reach_neither_the_api_nor_the_files_without_navigating() {
		let state = state();
		for request in [
			// A form or a link that opens the API.
			cross_site("/api/sessions", "navigate", "document"),
			cross_site("/agent/sessions/x/commands", "navigate", "iframe"),
			// An image or a script tag pointing at the interface.
			cross_site("/", "no-cors", "image"),
			cross_site("/pkg/app.js", "no-cors", "script"),
		] {
			let uri = request.uri().clone();
			let response = app(&state).oneshot(request).await.unwrap();
			assert_eq!(response.status(), StatusCode::FORBIDDEN, "{uri}");
		}
	}

	#[tokio::test]
	async fn a_post_needs_json() {
		let state = state();
		let request = HttpRequest::builder()
			.method("POST")
			.uri("/agent/sessions")
			.header(header::HOST, format!("127.0.0.1:{PORT}"))
			.header(header::CONTENT_TYPE, "text/plain")
			.body(Body::from("{}"))
			.unwrap();
		let response = app(&state).oneshot(request).await.unwrap();
		assert_eq!(response.status(), StatusCode::UNSUPPORTED_MEDIA_TYPE);
	}

	#[tokio::test]
	async fn an_allowed_origin_gets_cors_headers() {
		let state = state();
		let response = app(&state)
			.oneshot(request("GET", "/api/sessions", None))
			.await
			.unwrap();
		assert_eq!(
			response
				.headers()
				.get(header::ACCESS_CONTROL_ALLOW_ORIGIN)
				.unwrap(),
			&format!("http://127.0.0.1:{WEB}")
		);
	}
}
