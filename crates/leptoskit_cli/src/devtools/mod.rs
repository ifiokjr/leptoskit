//! The local `DevTools` hub.
//!
//! `leptoskit dev` starts it on loopback unless `--no-devtools` is passed.
//! Agents open a session with a [`Hello`], post events, and long-poll for
//! commands. Front ends read the mirrored state, stream changes, and send
//! commands. The hub refuses a session on another protocol version, binds
//! only loopback, and checks `Host`, `Origin`, `Sec-Fetch-Site`, and the JSON
//! content type on every request.

use std::collections::BTreeMap;
use std::convert::Infallible;
use std::net::Ipv4Addr;
use std::net::SocketAddr;
use std::path::Path;
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
use axum::http::header;
use axum::middleware::Next;
use axum::response::Html;
use axum::response::IntoResponse;
use axum::response::Response;
use axum::response::sse::Event as SseEvent;
use axum::response::sse::KeepAlive;
use axum::response::sse::Sse;
use axum::routing::get;
use axum::routing::post;
use futures::stream;
use leptoskit_devtools_protocol::CallRecord;
use leptoskit_devtools_protocol::Command;
use leptoskit_devtools_protocol::Commit;
use leptoskit_devtools_protocol::Event;
use leptoskit_devtools_protocol::Node;
use leptoskit_devtools_protocol::NodeRef;
use leptoskit_devtools_protocol::RpcAttempt;
use leptoskit_devtools_protocol::SessionId;
use leptoskit_devtools_protocol::SessionInfo;
use leptoskit_devtools_protocol::Snapshot;
use leptoskit_devtools_protocol::Tree;
use leptoskit_devtools_protocol::TreeOp;
use leptoskit_devtools_protocol::TreePatch;
use leptoskit_devtools_protocol::Value;
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

/// How many commits, calls, or rpc attempts a session keeps.
const HISTORY: usize = 256;

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
	let state = Arc::new(HubState::new(address.port(), web_port, POLL, EXPIRY));
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
				state
					.sessions
					.write()
					.await
					.retain(|_, session| now.saturating_sub(session.info.last_seen_ms) <= expiry);
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

/// The hub's mirror of a session's state.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SessionView {
	/// The latest tree.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub tree: Option<Tree>,
	/// The current route.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub route: Option<String>,
	/// The current queries, by key.
	#[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
	pub queries: BTreeMap<String, Value>,
	/// The current mutations, by key.
	#[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
	pub mutations: BTreeMap<String, Value>,
	/// The current forms, by name.
	#[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
	pub forms: BTreeMap<String, Value>,
	/// The current named state, by name.
	#[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
	pub state: BTreeMap<String, Value>,
	/// The latest commits.
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	pub commits: Vec<Commit>,
	/// The latest bridge calls.
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	pub calls: Vec<CallRecord>,
	/// The latest rpc attempts.
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	pub rpc: Vec<RpcAttempt>,
}

/// A change a front end sees on the stream.
#[derive(Clone, Debug, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum Change {
	/// An agent event.
	Event {
		/// The session it belongs to.
		session: SessionId,
		/// The event.
		event: Event,
	},
	/// A command sent to an agent.
	Command {
		/// The session it belongs to.
		session: SessionId,
		/// The command.
		command: Command,
	},
}

/// One session as `GET /api/sessions/{id}` returns it.
#[derive(Clone, Debug, Serialize, Deserialize)]
struct SessionResponse {
	#[serde(flatten)]
	info: SessionInfo,
	#[serde(flatten)]
	view: SessionView,
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
		.route("/", get(page))
		.route("/agent/sessions", post(create_session))
		.route("/agent/sessions/{id}/events", post(events))
		.route("/agent/sessions/{id}/commands", get(commands))
		.route("/api/sessions", get(sessions))
		.route("/api/sessions/{id}", get(session))
		.route("/api/sessions/{id}/commands", post(send_command))
		.route("/api/stream", get(sse))
		.layer(axum::middleware::from_fn_with_state(
			Arc::clone(&state),
			secure,
		))
		.with_state(state)
}

/// The hub page.
const PAGE: &str = include_str!("page.html");

async fn page() -> Html<&'static str> {
	Html(PAGE)
}

async fn create_session(
	State(state): State<Arc<HubState>>,
	Json(hello): Json<leptoskit_devtools_protocol::Hello>,
) -> Result<Json<SessionId>, ApiError> {
	hello.check_protocol()?;
	let id = SessionId::new(Uuid::new_v4().to_string());
	let now = now_ms();
	let session = Session {
		info: SessionInfo {
			id: id.clone(),
			hello,
			connected_at_ms: now,
			last_seen_ms: now,
		},
		view: SessionView::default(),
		commands: Vec::new(),
		notify: Arc::new(Notify::new()),
	};
	state.sessions.write().await.insert(id.clone(), session);
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
			apply_event(&mut session.view, &event);
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
) -> Result<Json<SessionResponse>, ApiError> {
	let sessions = state.sessions.read().await;
	let session = sessions
		.get(&SessionId(id))
		.ok_or_else(ApiError::not_found)?;
	Ok(Json(SessionResponse {
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

/// Mirror one event into a session's view.
fn apply_event(view: &mut SessionView, event: &Event) {
	match event {
		Event::Snapshot { snapshot } => apply_snapshot(view, snapshot),
		Event::TreePatch { patch } => {
			if let Some(tree) = &mut view.tree {
				apply_patch(tree, patch);
			}
		}
		Event::Commit { commit } => push(&mut view.commits, commit.clone()),
		Event::RouteChanged { route } => view.route = Some(route.clone()),
		Event::QueryChanged { key, value } => {
			view.queries.insert(key.clone(), value.clone());
		}
		Event::MutationChanged { key, value } => {
			view.mutations.insert(key.clone(), value.clone());
		}
		Event::FormChanged { name, value } => {
			view.forms.insert(name.clone(), value.clone());
		}
		Event::StateChanged { name, value } => {
			view.state.insert(name.clone(), value.clone());
		}
		Event::Call { call } => {
			match view.calls.iter_mut().find(|item| item.id == call.id) {
				Some(item) => *item = call.clone(),
				None => push(&mut view.calls, call.clone()),
			}
		}
		Event::RpcAttempt { attempt } => {
			match view.rpc.iter_mut().find(|item| item.id == attempt.id) {
				Some(item) => *item = attempt.clone(),
				None => push(&mut view.rpc, attempt.clone()),
			}
		}
		Event::Picked { .. } => {}
	}
}

fn apply_snapshot(view: &mut SessionView, snapshot: &Snapshot) {
	view.tree = Some(snapshot.tree.clone());
	view.route.clone_from(&snapshot.route);
	view.queries = snapshot
		.queries
		.iter()
		.map(|value| (value.name.clone(), value.value.clone()))
		.collect();
	view.mutations = snapshot
		.mutations
		.iter()
		.map(|value| (value.name.clone(), value.value.clone()))
		.collect();
	view.forms = snapshot
		.forms
		.iter()
		.map(|value| (value.name.clone(), value.value.clone()))
		.collect();
	view.state = snapshot
		.state
		.iter()
		.map(|value| (value.name.clone(), value.value.clone()))
		.collect();
	view.calls.clone_from(&snapshot.calls);
	view.rpc.clone_from(&snapshot.rpc);

	view.commits.clear();
}

fn apply_patch(tree: &mut Tree, patch: &TreePatch) {
	for operation in &patch.ops {
		match operation {
			TreeOp::Insert { node } => insert_node(tree, node),
			TreeOp::Update { node } => update_node(tree, node),
			TreeOp::Remove { id } => remove_subtree(tree, id),
		}
	}
}

fn insert_node(tree: &mut Tree, node: &Node) {
	if tree.nodes.iter().any(|item| item.id == node.id) {
		return;
	}

	match &node.parent {
		Some(parent) => {
			if let Some(slot) = tree.nodes.iter_mut().find(|item| &item.id == parent) {
				slot.children.push(node.id.clone());
			}
		}
		None => tree.root = node.id.clone(),
	}

	tree.nodes.push(node.clone());
}

fn update_node(tree: &mut Tree, node: &Node) {
	if let Some(slot) = tree.nodes.iter_mut().find(|item| item.id == node.id) {
		*slot = node.clone();
	}
}

fn remove_subtree(tree: &mut Tree, id: &NodeRef) {
	let remove = descendants(tree, id);
	tree.nodes.retain(|node| !remove.contains(&node.id));

	for node in &mut tree.nodes {
		node.children.retain(|child| !remove.contains(child));
	}
}

fn descendants(tree: &Tree, root: &NodeRef) -> Vec<NodeRef> {
	let mut found = vec![root.clone()];
	let mut index = 0;

	while index < found.len() {
		let current = found[index].clone();
		found.extend(
			tree.nodes
				.iter()
				.filter(|node| node.parent.as_ref() == Some(&current))
				.map(|node| node.id.clone()),
		);
		index += 1;
	}

	found
}

fn push<T>(items: &mut Vec<T>, item: T) {
	if items.len() == HISTORY {
		items.remove(0);
	}

	items.push(item);
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
	use leptoskit_devtools_protocol::PROTOCOL_VERSION;
	use leptoskit_devtools_protocol::Platform;
	use leptoskit_devtools_protocol::SourceLocation;
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

	#[test]
	fn the_page_lists_sessions_and_nodes() {
		assert!(PAGE.contains("id=\"sessions\""));
		assert!(PAGE.contains("/api/stream"));
		assert!(PAGE.contains("request_snapshot"));
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

		let view: SessionResponse = body(response).await;
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
