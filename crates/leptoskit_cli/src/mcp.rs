//! The `leptoskit mcp` server: the same device control API, spoken over the
//! Model Context Protocol.
//!
//! One JSON-RPC dispatcher answers both transports. `leptoskit mcp` speaks
//! newline-delimited JSON-RPC on standard input and output, which MCP clients
//! launch as a command. `leptoskit mcp serve` accepts the same messages as
//! HTTP POST bodies on loopback, so anything that can POST JSON — a script,
//! a test, an agent with nothing but curl — can drive devices without an MCP
//! client at all.
//!
//! The tools map straight onto the control document: `list_devices` reports
//! the inventory, `control` runs a whole document, and `screenshot` returns a
//! capture as image content so vision-capable agents see the screen inline.
//!
//! The devtools tools (`devtools_sessions`, `inspect_tree`, `inspect_node`,
//! `explain_styles`, `query_cache`, `navigate`, `highlight`, and `set_state`)
//! read a running app through the hub `leptoskit dev` starts; see
//! [`devtools`].

use std::io::BufRead;
use std::io::Write;

use anyhow::Context;
use anyhow::Result;
use clap::Subcommand;
use serde_json::Value;
use serde_json::json;

mod devtools;

/// The protocol revisions this server speaks.
const PROTOCOL_VERSION: &str = "2025-06-18";
/// Older clients still in the wild.
const LEGACY_PROTOCOL_VERSION: &str = "2024-11-05";

/// MCP commands.
#[derive(Debug, Subcommand)]
pub enum McpCommand {
	/// Serves MCP on standard input and output, as clients launch it.
	Serve,
	/// Serves MCP over HTTP POST on loopback, for direct JSON calls.
	Http {
		/// The address to bind; loopback only by default.
		#[arg(long, default_value = "127.0.0.1")]
		address: String,
		/// The port to listen on.
		#[arg(long, default_value_t = 7331)]
		port: u16,
	},
}

/// Dispatches MCP commands.
pub fn run(command: McpCommand) -> Result<()> {
	match command {
		McpCommand::Serve => stdio(),
		McpCommand::Http { address, port } => http(&address, port),
	}
}

/// Answers newline-delimited JSON-RPC on standard input until it ends.
fn stdio() -> Result<()> {
	serve(std::io::stdin().lock(), std::io::stdout())
}

/// Answers newline-delimited JSON-RPC read from `input`, one response per
/// line on `output`, until `input` ends.
fn serve(input: impl BufRead, mut output: impl Write) -> Result<()> {
	for line in input.lines() {
		let line = line.context("reading a message from standard input")?;

		if line.trim().is_empty() {
			continue;
		}

		let message: Value = match serde_json::from_str(&line) {
			Ok(message) => message,
			Err(error) => {
				let response = error_response(&Value::Null, -32700, &error.to_string());
				writeln!(output, "{response}")?;
				output.flush()?;
				continue;
			}
		};

		if let Some(response) = dispatch(&message) {
			writeln!(output, "{response}")?;
			output.flush()?;
		}
	}

	Ok(())
}

/// The HTTP transport's app: one route, the dispatcher behind it, and a
/// guard that keeps web pages out.
///
/// `hosts` lists the `Host` headers the server answers. With `None` (a
/// server bound to every interface) any host is accepted, since anything on
/// the network can reach it already.
fn http_router(hosts: Option<Vec<String>>) -> axum::Router {
	use axum::Router;
	use axum::extract::State;
	use axum::http::StatusCode;
	use axum::routing::post;

	#[derive(Clone)]
	struct Dispatcher(fn(&Value) -> Option<Value>);

	async fn message(State(dispatcher): State<Dispatcher>, body: String) -> (StatusCode, String) {
		match serde_json::from_str::<Value>(&body) {
			Ok(request) => {
				// The control executor blocks on drivers and platform tools;
				// it belongs on the blocking pool, not a request worker.
				let answer = tokio::task::spawn_blocking(move || (dispatcher.0)(&request))
					.await
					.expect("dispatch cannot panic");
				match answer {
					Some(response) => (StatusCode::OK, response.to_string()),
					// A notification: accepted, with nothing to answer.
					None => (StatusCode::ACCEPTED, String::new()),
				}
			}
			Err(error) => {
				(
					StatusCode::BAD_REQUEST,
					error_response(&Value::Null, -32700, &error.to_string()).to_string(),
				)
			}
		}
	}

	Router::new()
		.route("/", post(message))
		// The streamable-HTTP transport allows servers without
		// server-initiated streams to refuse the GET channel.
		.route(
			"/",
			axum::routing::get(|| {
				async {
					(
						StatusCode::METHOD_NOT_ALLOWED,
						"POST JSON-RPC messages only",
					)
				}
			}),
		)
		.with_state(Dispatcher(dispatch))
		.layer(axum::middleware::from_fn_with_state(
			std::sync::Arc::new(hosts),
			refuse_web_pages,
		))
}

/// Refuses requests a web page could make. The `control` tool can launch
/// browsers with chosen capabilities and read pages, so a page the developer
/// visits must not reach it. MCP clients send no `Origin`, a page's `fetch`
/// always does; requiring a JSON body makes any cross-origin page preflight,
/// which this server never answers; and the `Host` check stops a rebound DNS
/// name from reading the replies.
async fn refuse_web_pages(
	axum::extract::State(hosts): axum::extract::State<std::sync::Arc<Option<Vec<String>>>>,
	request: axum::extract::Request,
	next: axum::middleware::Next,
) -> axum::response::Response {
	use axum::http::Method;
	use axum::http::StatusCode;
	use axum::response::IntoResponse;

	let headers = request.headers();
	let value = |name: &str| headers.get(name).and_then(|value| value.to_str().ok());

	if let Some(hosts) = hosts.as_ref()
		&& !hosts
			.iter()
			.any(|host| Some(host.as_str()) == value("host"))
	{
		return (StatusCode::FORBIDDEN, "the Host header is not this server").into_response();
	}

	if headers.contains_key("origin") || headers.contains_key("sec-fetch-site") {
		return (StatusCode::FORBIDDEN, "browser requests are refused").into_response();
	}

	let json = value("content-type").is_some_and(|kind| {
		kind.split(';')
			.next()
			.is_some_and(|kind| kind.trim().eq_ignore_ascii_case("application/json"))
	});

	if request.method() == Method::POST && !json {
		return (
			StatusCode::UNSUPPORTED_MEDIA_TYPE,
			"POST JSON-RPC messages with content-type: application/json",
		)
			.into_response();
	}

	next.run(request).await
}

/// The `Host` headers a server bound to `address` answers: the loopback
/// names, plus the address itself. A server on every interface answers any.
fn allowed_hosts(address: &str, port: u16) -> Option<Vec<String>> {
	let ip = address.parse::<std::net::IpAddr>().ok();

	if ip.is_some_and(|ip| ip.is_unspecified()) {
		return None;
	}

	let shown = match ip {
		Some(std::net::IpAddr::V6(ip)) => format!("[{ip}]:{port}"),
		_ => format!("{address}:{port}"),
	};

	let mut hosts = vec![
		format!("127.0.0.1:{port}"),
		format!("localhost:{port}"),
		format!("[::1]:{port}"),
	];

	if !hosts.contains(&shown) {
		hosts.push(shown);
	}

	Some(hosts)
}

/// Answers HTTP `POSTed` JSON-RPC bodies on one loopback listener.
fn http(address: &str, port: u16) -> Result<()> {
	let runtime = tokio::runtime::Builder::new_multi_thread()
		.enable_all()
		.build()?;
	let address = address.to_owned();
	let bound = runtime
		.block_on(async { tokio::net::TcpListener::bind((address.as_str(), port)).await })?;
	let port = bound.local_addr()?.port();
	let router = http_router(allowed_hosts(&address, port));
	println!("leptoskit mcp listening on http://{address}:{port}");

	if address
		.parse::<std::net::IpAddr>()
		.map_or(address != "localhost", |ip| !ip.is_loopback())
	{
		eprintln!(
			"warning: {address} is not a loopback address; anything that can reach it can run the \
			 control tool, with no authentication"
		);
	}
	runtime.block_on(async { axum::serve(bound, router).await })?;
	Ok(())
}

/// Answers one JSON-RPC message, returning [`None`] for notifications.
fn dispatch(message: &Value) -> Option<Value> {
	// Notifications carry no id; there is nothing to answer.
	let id = message.get("id").cloned()?;
	let method = message.get("method").and_then(Value::as_str).unwrap_or("");
	let params = message.get("params");

	let result = match method {
		"initialize" => {
			let requested = params
				.and_then(|params| params.get("protocolVersion"))
				.and_then(Value::as_str);
			let version = match requested {
				Some(PROTOCOL_VERSION | LEGACY_PROTOCOL_VERSION) => requested,
				_ => Some(PROTOCOL_VERSION),
			};

			json!({
				"protocolVersion": version,
				"capabilities": {"tools": {}},
				"serverInfo": {
					"name": "leptoskit",
					"version": env!("CARGO_PKG_VERSION"),
				},
			})
		}
		"ping" => json!({}),
		"tools/list" => json!({"tools": tools()}),
		"tools/call" => {
			match call_tool(params) {
				Ok(result) => result,
				Err(error) => return Some(error_response(&id, -32602, &error)),
			}
		}
		"" => return Some(error_response(&id, -32600, "not a request")),
		other => {
			return Some(error_response(
				&id,
				-32601,
				&format!("unknown method {other:?}"),
			));
		}
	};

	Some(json!({"jsonrpc": "2.0", "id": id, "result": result}))
}

fn error_response(id: &Value, code: i64, message: &str) -> Value {
	json!({
		"jsonrpc": "2.0",
		"id": id.clone(),
		"error": {"code": code, "message": message},
	})
}

/// The tool list, with the control document's own schema on `control`.
fn tools() -> Value {
	let mut tools = control_tools();

	if let Value::Array(tools) = &mut tools {
		tools.extend(devtools::tools());
	}

	tools
}

/// The device control tools.
fn control_tools() -> Value {
	json!([
		{
			"name": "list_devices",
			"description": "Lists everything attached right now: iOS simulators and devices, \
				Android devices and emulators, installed AVDs, the desktop, and browsers with \
				drivers installed. Every entry's id is what a control document's \
				target.device uses.",
			"inputSchema": {"type": "object", "properties": {}, "additionalProperties": false},
		},
		{
			"name": "control",
			"description": "Runs a leptoskit control document: a target — a device, a native \
				app, or a website — and ordered steps that tap, type, swipe, read state, \
				screenshot, launch and stop apps, boot simulators and emulators, and read \
				logs. Steps stop at the first failure; the reply reports each step and every \
				artifact path. Driver steps need an Appium endpoint for native targets \
				(environment LEPTOSKIT_APPIUM_ENDPOINT, default http://127.0.0.1:4723) and \
				spawn the browser's own driver for websites.",
			"inputSchema": document_schema_value(),
		},
		{
			"name": "screenshot",
			"description": "Captures one screenshot and returns it as image content, with the \
				saved path. The target fields are the control document's; without an \
				application this captures the whole device screen.",
			"inputSchema": document_schema_value()["properties"]["target"],
		},
	])
}

/// The published control document schema, as plain JSON.
fn document_schema_value() -> Value {
	serde_json::to_value(leptoskit_testing::control::document_schema(
		"https://ifiokjr.github.io/leptoskit/schemas/control.schema.json",
	))
	.expect("schemas serialize")
}

/// Runs one tool call; the error is an JSON-RPC error message.
fn call_tool(params: Option<&Value>) -> Result<Value, String> {
	let Some(params) = params else {
		return Err("tools/call needs parameters".to_owned());
	};
	let name = params
		.get("name")
		.and_then(Value::as_str)
		.ok_or("tools/call needs a tool name")?;
	let arguments = params.get("arguments").cloned().unwrap_or(json!({}));

	if devtools::NAMES.contains(&name) {
		return Ok(devtools::call(name, &arguments));
	}

	match name {
		"list_devices" => {
			let inventory = leptoskit_testing::control::inventory::list();
			Ok(text_result(
				&serde_json::to_string_pretty(&inventory).unwrap(),
			))
		}
		"control" => {
			let document: leptoskit_testing::control::Document =
				serde_json::from_value(arguments).map_err(|error| error.to_string())?;
			let outcome = leptoskit_testing::control::run(&document);
			let passed = outcome.passed;
			let text = serde_json::to_string_pretty(&outcome).unwrap();
			Ok(json!({
				"content": [{"type": "text", "text": text}],
				"isError": !passed,
			}))
		}
		"screenshot" => {
			// The label names the capture but is not a target field, so it
			// rides along with the arguments and is lifted out before the
			// target parses.
			let mut arguments = arguments;
			let label = arguments
				.as_object_mut()
				.and_then(|fields| fields.remove("label"))
				.and_then(|value| value.as_str().map(str::to_owned))
				.unwrap_or_else(|| "screenshot".to_owned());
			let target: leptoskit_testing::control::Target =
				serde_json::from_value(arguments).map_err(|error| error.to_string())?;
			let step = if target.application.is_none()
				&& target.kind != leptoskit_testing::control::TargetKind::Web
				&& matches!(
					target.kind,
					leptoskit_testing::control::TargetKind::IosSimulator
						| leptoskit_testing::control::TargetKind::Android
				) {
				leptoskit_testing::control::Step::DeviceScreenshot { label }
			} else {
				leptoskit_testing::control::Step::Screenshot { label }
			};
			let document = leptoskit_testing::control::Document {
				schema_version: leptoskit_testing::control::SCHEMA_VERSION,
				target,
				steps: vec![step],
			};
			let outcome = leptoskit_testing::control::run(&document);
			let Some(artifact) = outcome.steps.first().and_then(|step| step.artifact.clone())
			else {
				return Ok(json!({
					"content": [{
						"type": "text",
						"text": serde_json::to_string_pretty(&outcome).unwrap(),
					}],
					"isError": true,
				}));
			};
			let bytes = std::fs::read(&artifact).map_err(|error| error.to_string())?;
			use base64::Engine;
			let encoded = base64::engine::general_purpose::STANDARD.encode(bytes);
			Ok(json!({
				"content": [
					{"type": "image", "data": encoded, "mimeType": "image/png"},
					{"type": "text", "text": artifact.display().to_string()},
				],
				"isError": !outcome.passed,
			}))
		}
		other => Err(format!("unknown tool {other:?}")),
	}
}

fn text_result(text: &str) -> Value {
	json!({"content": [{"type": "text", "text": text}]})
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn initialize_answers_the_client_revision_or_ours() {
		let request = json!({
			"jsonrpc": "2.0",
			"id": 1,
			"method": "initialize",
			"params": {"protocolVersion": "2024-11-05", "capabilities": {}},
		});
		let response = dispatch(&request).unwrap();
		assert_eq!(response["result"]["protocolVersion"], "2024-11-05");
		let request = json!({
			"jsonrpc": "2.0",
			"id": 2,
			"method": "initialize",
			"params": {"protocolVersion": "1999-01-01", "capabilities": {}},
		});
		let response = dispatch(&request).unwrap();
		assert_eq!(response["result"]["protocolVersion"], PROTOCOL_VERSION);
		assert_eq!(response["result"]["serverInfo"]["name"], "leptoskit");
	}

	#[test]
	fn notifications_are_silent_and_unknown_methods_are_rejected() {
		assert!(
			dispatch(&json!({
				"jsonrpc": "2.0",
				"method": "notifications/initialized",
			}))
			.is_none()
		);
		let response = dispatch(&json!({
			"jsonrpc": "2.0",
			"id": 7,
			"method": "workspace/attach",
		}))
		.unwrap();
		assert_eq!(response["error"]["code"], -32601);
	}

	#[test]
	fn tools_list_names_the_control_surface() {
		let response = dispatch(&json!({
			"jsonrpc": "2.0",
			"id": 1,
			"method": "tools/list",
		}))
		.unwrap();
		let names: Vec<&str> = response["result"]["tools"]
			.as_array()
			.unwrap()
			.iter()
			.map(|tool| tool["name"].as_str().unwrap())
			.collect();
		assert_eq!(
			names,
			[
				"list_devices",
				"control",
				"screenshot",
				"devtools_sessions",
				"inspect_tree",
				"inspect_node",
				"explain_styles",
				"query_cache",
				"navigate",
				"highlight",
				"set_state",
			]
		);
		// The control tool carries the document schema, so clients validate
		// journeys before sending them.
		let schema = &response["result"]["tools"][1]["inputSchema"];
		assert_eq!(schema["title"], "leptoskit control document");
		assert!(schema["$defs"]["Step"].is_object());
	}

	/// A minimal W3C driver: one reply per request, over a real socket.
	fn fake_driver(replies: Vec<(&'static str, Value)>) -> (String, std::thread::JoinHandle<()>) {
		use std::io::BufRead;
		use std::io::BufReader;
		use std::io::Read;
		use std::io::Write;
		let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
		listener.set_nonblocking(true).unwrap();
		let endpoint = format!("http://{}", listener.local_addr().unwrap());
		let handle = std::thread::spawn(move || {
			for (expected, value) in replies {
				let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
				let mut stream = loop {
					match listener.accept() {
						Ok((stream, _)) => break stream,
						Err(error)
							if error.kind() == std::io::ErrorKind::WouldBlock
								&& std::time::Instant::now() < deadline =>
						{
							std::thread::sleep(std::time::Duration::from_millis(5));
						}
						Err(error) => panic!("expected {expected}: {error}"),
					}
				};

				stream.set_nonblocking(false).unwrap();
				let mut reader = BufReader::new(&stream);
				let mut first = String::new();
				reader.read_line(&mut first).unwrap();
				assert!(
					first.contains(expected),
					"{first} should contain {expected}"
				);
				let mut length = 0;
				loop {
					let mut line = String::new();
					reader.read_line(&mut line).unwrap();
					if line == "\r\n" {
						break;
					}
					if let Some(value) = line.to_lowercase().strip_prefix("content-length:") {
						length = value.trim().parse().unwrap();
					}
				}

				let mut bytes = vec![0; length];
				if length > 0 {
					reader.read_exact(&mut bytes).unwrap();
				}

				let body = json!({"value": value}).to_string();
				write!(
					stream,
					"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: \
					 {}\r\nConnection: close\r\n\r\n{body}",
					body.len()
				)
				.unwrap();
			}
		});
		(endpoint, handle)
	}

	fn png() -> Vec<u8> {
		let mut bytes = Vec::new();
		{
			let mut encoder = png::Encoder::new(&mut bytes, 2, 1);
			encoder.set_color(png::ColorType::Rgb);
			encoder
				.write_header()
				.unwrap()
				.write_image_data(&[255, 0, 0, 0, 255, 0])
				.unwrap();
		}
		bytes
	}

	#[test]
	fn tools_run_whole_documents_and_return_screenshots() {
		use base64::Engine;
		let bytes = png();
		let encoded = base64::engine::general_purpose::STANDARD.encode(&bytes);
		let replies = |encoded: String| {
			vec![
				("GET /status ", json!({"ready": true})),
				("POST /session ", json!({"sessionId": "s"})),
				("POST /session/s/timeouts ", Value::Null),
				("POST /session/s/url ", Value::Null),
				("GET /session/s/screenshot ", json!(encoded)),
				("DELETE /session/s ", Value::Null),
			]
		};
		let (endpoint, server) = fake_driver(replies(encoded.clone()));
		let outcome = dispatch(&json!({
			"jsonrpc": "2.0",
			"id": 9,
			"method": "tools/call",
			"params": {
				"name": "control",
				"arguments": {
					"target": {
						"kind": "web",
						"url": "http://127.0.0.1:1/",
						"endpoint": endpoint,
					},
					"steps": [{"action": "screenshot", "label": "tool"}],
				},
			},
		}))
		.unwrap();
		assert_eq!(outcome["result"]["isError"], false);
		let parsed: Value =
			serde_json::from_str(outcome["result"]["content"][0]["text"].as_str().unwrap())
				.unwrap();

		assert_eq!(parsed["passed"], true);

		let (endpoint, screenshot_server) = fake_driver(replies(encoded));
		let shot = dispatch(&json!({
			"jsonrpc": "2.0",
			"id": 10,
			"method": "tools/call",
			"params": {
				"name": "screenshot",
				"arguments": {
					"kind": "web",
					"url": "http://127.0.0.1:1/",
					"endpoint": endpoint,
					"label": "inline",
				},
			},
		}))
		.unwrap();
		assert_eq!(shot["result"]["content"][0]["type"], "image");
		assert_eq!(shot["result"]["content"][0]["mimeType"], "image/png");
		let image = shot["result"]["content"][0]["data"].as_str().unwrap();
		assert_eq!(
			base64::engine::general_purpose::STANDARD
				.decode(image)
				.unwrap(),
			bytes
		);
		let path = shot["result"]["content"][1]["text"].as_str().unwrap();
		std::fs::remove_file(path).unwrap();

		server.join().unwrap();
		screenshot_server.join().unwrap();
	}

	#[tokio::test]
	async fn the_http_transport_answers_posts_and_refuses_get() {
		use axum::http::Request;
		use axum::http::StatusCode;
		use tower::ServiceExt;

		let app = http_router(allowed_hosts("127.0.0.1", 7331));
		let ping = app
			.clone()
			.oneshot(
				Request::post("/")
					.header("host", "127.0.0.1:7331")
					.header("content-type", "application/json")
					.body(r#"{"jsonrpc":"2.0","id":1,"method":"ping"}"#.to_owned())
					.unwrap(),
			)
			.await
			.unwrap();
		assert_eq!(ping.status(), StatusCode::OK);
		let body = axum::body::to_bytes(ping.into_body(), 4096).await.unwrap();
		assert!(
			std::str::from_utf8(&body)
				.unwrap()
				.contains(r#""result":{}"#)
		);

		let notification = app
			.clone()
			.oneshot(
				Request::post("/")
					.header("host", "localhost:7331")
					.header("content-type", "application/json")
					.body(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#.to_owned())
					.unwrap(),
			)
			.await
			.unwrap();
		assert_eq!(notification.status(), StatusCode::ACCEPTED);

		let malformed = app
			.clone()
			.oneshot(
				Request::post("/")
					.header("host", "[::1]:7331")
					.header("content-type", "application/json")
					.body("not json".to_owned())
					.unwrap(),
			)
			.await
			.unwrap();
		assert_eq!(malformed.status(), StatusCode::BAD_REQUEST);

		let get = app
			.oneshot(
				Request::get("/")
					.header("host", "127.0.0.1:7331")
					.body(axum::body::Body::empty())
					.unwrap(),
			)
			.await
			.unwrap();
		assert_eq!(get.status(), StatusCode::METHOD_NOT_ALLOWED);
	}

	#[tokio::test]
	async fn the_http_transport_refuses_what_a_web_page_can_send() {
		use axum::http::Request;
		use axum::http::StatusCode;
		use tower::ServiceExt;

		let app = http_router(allowed_hosts("127.0.0.1", 7331));
		let ping = r#"{"jsonrpc":"2.0","id":1,"method":"ping"}"#;
		let send = |request: Request<String>| {
			let app = app.clone();
			async move { app.oneshot(request).await.unwrap().status() }
		};

		// A page's `fetch(..., {mode: "no-cors"})` carries its Origin.
		let cross_origin = Request::post("/")
			.header("host", "127.0.0.1:7331")
			.header("origin", "https://attacker.example")
			.header("content-type", "application/json")
			.body(ping.to_owned())
			.unwrap();
		assert_eq!(send(cross_origin).await, StatusCode::FORBIDDEN);

		// A form post, or a text/plain body: the requests a browser sends
		// without asking first.
		for kind in ["text/plain", "application/x-www-form-urlencoded"] {
			let simple = Request::post("/")
				.header("host", "127.0.0.1:7331")
				.header("content-type", kind)
				.body(ping.to_owned())
				.unwrap();
			assert_eq!(send(simple).await, StatusCode::UNSUPPORTED_MEDIA_TYPE);
		}

		// A DNS name rebound to loopback still names itself in Host.
		let rebound = Request::post("/")
			.header("host", "attacker.example:7331")
			.header("content-type", "application/json")
			.body(ping.to_owned())
			.unwrap();
		assert_eq!(send(rebound).await, StatusCode::FORBIDDEN);

		let fetch_metadata = Request::post("/")
			.header("host", "127.0.0.1:7331")
			.header("sec-fetch-site", "cross-site")
			.header("content-type", "application/json")
			.body(ping.to_owned())
			.unwrap();
		assert_eq!(send(fetch_metadata).await, StatusCode::FORBIDDEN);
	}

	#[test]
	fn the_http_transport_answers_the_hosts_it_is_bound_to() {
		let loopback = allowed_hosts("127.0.0.1", 7331).unwrap();
		assert!(loopback.contains(&"localhost:7331".to_owned()));
		assert!(!loopback.iter().any(|host| host.contains("example")));

		let lan = allowed_hosts("192.168.1.20", 7331).unwrap();
		assert!(lan.contains(&"192.168.1.20:7331".to_owned()));
		assert_eq!(
			allowed_hosts("::1", 9000).unwrap().last().unwrap(),
			"[::1]:9000"
		);
		assert_eq!(allowed_hosts("0.0.0.0", 7331), None);
	}

	#[test]
	fn list_devices_answers_with_the_inventory() {
		let response = dispatch(&json!({
			"jsonrpc": "2.0",
			"id": 34,
			"method": "tools/call",
			"params": {"name": "list_devices", "arguments": {}},
		}))
		.unwrap();
		let text = response["result"]["content"][0]["text"].as_str().unwrap();
		let inventory: Value = serde_json::from_str(text).unwrap();
		assert_ne!(inventory["devices"], json!([]));
	}

	#[test]
	fn screenshots_of_session_targets_choose_the_session_step() {
		let shot = dispatch(&json!({
			"jsonrpc": "2.0",
			"id": 33,
			"method": "tools/call",
			"params": {
				"name": "screenshot",
				"arguments": {
					"kind": "macos",
					"application": {"kind": "identifier", "value": "dev.app"},
					// No driver listens, and the session's readiness loop
					// otherwise waits out the whole startup budget.
					"startup_seconds": 1,
				},
			},
		}))
		.unwrap();
		// The document runs, fails for want of a driver, and reports the
		// outcome — through the session step, not the device camera.
		assert_eq!(shot["result"]["isError"], true);
		let outcome: Value =
			serde_json::from_str(shot["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
		assert_eq!(outcome["steps"][0]["action"], "screenshot");
	}

	#[test]
	fn failing_documents_report_is_errors() {
		// A document that parses but whose step fails against a bogus device:
		// the reply is a result with isError set, not a protocol error.
		let response = dispatch(&json!({
			"jsonrpc": "2.0",
			"id": 31,
			"method": "tools/call",
			"params": {
				"name": "control",
				"arguments": {
					"target": {
						"kind": "ios_simulator",
						"device": "00000000-0000-0000-0000-000000000000",
					},
					"steps": [{"action": "boot"}],
				},
			},
		}))
		.unwrap();
		assert_eq!(response["result"]["isError"], true);
		let outcome: Value =
			serde_json::from_str(response["result"]["content"][0]["text"].as_str().unwrap())
				.unwrap();
		assert_eq!(outcome["passed"], false);

		// A screenshot that cannot be taken reports its outcome the same way.
		let shot = dispatch(&json!({
			"jsonrpc": "2.0",
			"id": 32,
			"method": "tools/call",
			"params": {
				"name": "screenshot",
				"arguments": {
					"kind": "ios_simulator",
					"device": "00000000-0000-0000-0000-000000000000",
					"label": "nothing",
				},
			},
		}))
		.unwrap();
		assert_eq!(shot["result"]["isError"], true);
		assert_eq!(shot["result"]["content"][0]["type"], "text");
	}

	#[test]
	fn tools_call_without_parameters_is_rejected() {
		let response = dispatch(&json!({
			"jsonrpc": "2.0",
			"id": 8,
			"method": "tools/call",
		}))
		.unwrap();
		assert_eq!(response["error"]["code"], -32602);
	}

	#[test]
	fn stdio_answers_one_response_per_line_and_swallows_notifications() {
		let input = "not json\n".to_owned()
			+ r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#
			+ "\n"
			+ r#"{"jsonrpc":"2.0","id":1,"method":"ping"}"#
			+ "\n\n";
		let mut output = Vec::new();
		serve(std::io::Cursor::new(input), &mut output).unwrap();
		let lines: Vec<&str> = std::str::from_utf8(&output).unwrap().lines().collect();
		assert_eq!(lines.len(), 2, "{lines:?}");
		assert!(lines[0].contains("-32700"));
		assert!(lines[1].contains(r#""result":{}"#));
	}

	#[test]
	fn calling_an_unknown_tool_is_an_invalid_params_error() {
		let response = dispatch(&json!({
			"jsonrpc": "2.0",
			"id": 3,
			"method": "tools/call",
			"params": {"name": "teleport", "arguments": {}},
		}))
		.unwrap();
		assert_eq!(response["error"]["code"], -32602);
	}

	#[test]
	fn control_rejects_documents_that_do_not_parse() {
		let response = dispatch(&json!({
			"jsonrpc": "2.0",
			"id": 4,
			"method": "tools/call",
			"params": {"name": "control", "arguments": {"target": {"kind": "web"}}},
		}))
		.unwrap();
		// The target is missing its url; parse fails before the run starts.
		assert_eq!(response["error"]["code"], -32602);
	}
}
