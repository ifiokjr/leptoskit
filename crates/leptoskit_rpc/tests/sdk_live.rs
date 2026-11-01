//! The generated SDKs' live side, end to end: each language's client runs
//! against a real `Live` server on SQLite, through a proxy that can cut the
//! connection at the worst moment.
//!
//! Every client program in `tests/sdk_live/` runs the same scenario:
//!
//! 1. Ada and Bob connect with bearer tokens and subscribe to `sdk_notes`, and
//!    each receives the first value.
//! 2. Bob adds a note over the socket; when the call resolves, his own
//!    subscription already shows it (read-your-writes), and Ada's receives it
//!    as a patch.
//! 3. Ada adds a note marked `drop-me`. The proxy lets it reach the server,
//!    holds back the answer, and cuts the connection. Ada's client reconnects,
//!    subscribes again, and sends the mutation again with the same request ID
//!    and key; the receipt makes it run once.
//! 4. Bob adds another note, and Ada's resumed subscription receives it.
//! 5. Both join the `sdk_room` channel, see each other's presence, and Bob's
//!    signal reaches Ada.
//!
//! The harness then checks that the `drop-me` note was written exactly once.
//!
//! The tests need each language's toolchain, so they're ignored by default:
//! `cargo nextest run -p leptoskit_rpc --all-features --test sdk_live
//! --run-ignored all`. A test whose toolchain is missing says so and passes,
//! unless `LEPTOSKIT_REQUIRE_SDK_TOOLCHAINS=1`.

#![cfg(all(feature = "live", feature = "sqlite"))]

use std::path::Path;
use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;

use futures::SinkExt;
use futures::StreamExt;
use leptoskit_db::Context;
use leptoskit_db::Db;
use leptoskit_db::Id;
use leptoskit_db::table;
use leptoskit_query::BoxFuture;
use leptoskit_rpc as rpc;
use leptoskit_rpc::api;
use leptoskit_rpc::server::Identify;
use leptoskit_rpc::server::Identity;
use leptoskit_rpc::server::Live;
use leptoskit_rpc::spec::SpecOptions;
use leptoskit_rpc_codegen::GenerateOptions;
use leptoskit_rpc_codegen::Scaffold;
use leptoskit_rpc_codegen::TargetLanguage;
use schemars::JsonSchema;
use serde::Deserialize;
use serde::Serialize;
use tokio::net::TcpListener;
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::handshake::client::generate_key;
use tokio_tungstenite::tungstenite::handshake::server::Request;
use tokio_tungstenite::tungstenite::handshake::server::Response;
use tokio_tungstenite::tungstenite::http;

/// A note the SDKs write and watch.
#[table(name = "sdk_notes", live)]
pub struct SdkNoteRow {
	pub id: Id<Self>,
	pub body: String,
}

/// One note, as the query returns it.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize, JsonSchema)]
pub struct SdkNote {
	/// The note's id.
	pub id: String,
	/// What it says.
	pub body: String,
}

/// Every note that starts with `prefix`, in alphabetical order.
#[api]
#[rpc::query]
pub async fn sdk_notes(cx: &rpc::QueryCx, prefix: String) -> rpc::Result<Vec<SdkNote>> {
	let notes = SdkNoteRow::query()
		.order_by(|note| note.body.asc())
		.all(cx.db())
		.await?;
	Ok(notes
		.into_iter()
		.filter(|note| note.body.starts_with(&prefix))
		.map(|note| {
			SdkNote {
				id: note.id.to_string(),
				body: note.body,
			}
		})
		.collect())
}

/// Adds a note, and returns its id.
#[api]
#[rpc::mutation]
pub async fn add_sdk_note(cx: &rpc::MutationCx, body: String) -> rpc::Result<String> {
	let note = NewSdkNoteRow { body }.insert(cx.tx()).await?;
	Ok(note.id.to_string())
}

/// What a member of the room shows the others.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize, JsonSchema)]
pub struct SdkPresence {
	/// Their name.
	pub name: String,
}

/// What members of the room send each other.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize, JsonSchema)]
pub struct SdkSignal {
	/// The text.
	pub text: String,
}

/// A room anyone signed in may join.
#[api]
#[rpc::channel(message = SdkSignal, presence = SdkPresence, authorize = may_join)]
pub struct SdkRoom {
	/// Which room.
	pub room: String,
}

#[allow(clippy::unused_async, reason = "the signature the macro expects")]
async fn may_join(cx: &rpc::QueryCx, _room: &SdkRoom) -> rpc::Result<()> {
	cx.identity()
		.caller
		.as_ref()
		.map(|_| ())
		.ok_or_else(|| rpc::Error::unauthorized("sign in to join"))
}

/// Trusts a bearer token as the caller's name, for the test.
struct Tokens;

impl Identify for Tokens {
	fn identify<'a>(
		&'a self,
		request: &'a http::request::Parts,
	) -> BoxFuture<'a, Result<Identity, rpc::Error>> {
		Box::pin(async move {
			let token = request
				.headers
				.get(http::header::AUTHORIZATION)
				.and_then(|value| value.to_str().ok())
				.and_then(|value| value.strip_prefix("Bearer "));

			Ok(match token {
				Some(token) => {
					Identity::new(Context::new().with(token.to_owned())).with_caller(token)
				}
				None => Identity::anonymous(),
			})
		})
	}
}

/// A running server, and the proxy every client connects through.
struct Harness {
	db: Db,
	/// The proxy's origin, such as `http://127.0.0.1:4321`.
	url: String,
	/// How many connections the proxy has cut.
	cuts: Arc<AtomicUsize>,
	tasks: Vec<tokio::task::JoinHandle<()>>,
}

impl Drop for Harness {
	fn drop(&mut self) {
		for task in &self.tasks {
			task.abort();
		}
	}
}

async fn start() -> Harness {
	let db = leptoskit_db::testing::sqlite().await;
	let live = Live::new(db.clone()).identify(Tokens);
	let app = live.router::<()>();
	let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
	let server_address = listener.local_addr().unwrap();
	let server = tokio::spawn(async move {
		axum::serve(listener, app).await.unwrap();
	});

	let proxy = TcpListener::bind("127.0.0.1:0").await.unwrap();
	let proxy_address = proxy.local_addr().unwrap();
	let cuts = Arc::new(AtomicUsize::new(0));
	let armed = Arc::new(AtomicBool::new(true));
	let proxy_cuts = Arc::clone(&cuts);
	let proxy = tokio::spawn(async move {
		loop {
			let Ok((stream, _)) = proxy.accept().await else {
				return;
			};
			let armed = Arc::clone(&armed);
			let cuts = Arc::clone(&proxy_cuts);
			tokio::spawn(async move {
				relay(stream, server_address, armed, cuts).await;
			});
		}
	});

	Harness {
		db,
		url: format!("http://{proxy_address}"),
		cuts,
		tasks: vec![server, proxy],
	}
}

/// Relays one connection to the server. The first mutation whose arguments
/// say `drop-me` reaches the server, but its answer never comes back: the
/// proxy cuts both sides as soon as the server sends it.
async fn relay(
	stream: TcpStream,
	server: std::net::SocketAddr,
	armed: Arc<AtomicBool>,
	cuts: Arc<AtomicUsize>,
) {
	let mut authorization = None;
	let callback = |request: &Request, mut response: Response| {
		authorization = request.headers().get(http::header::AUTHORIZATION).cloned();
		let offered = request
			.headers()
			.get(http::header::SEC_WEBSOCKET_PROTOCOL)
			.and_then(|value| value.to_str().ok())
			.unwrap_or_default();

		if offered
			.split(',')
			.any(|protocol| protocol.trim() == rpc::live::SUBPROTOCOL)
		{
			response.headers_mut().insert(
				http::header::SEC_WEBSOCKET_PROTOCOL,
				http::HeaderValue::from_static(rpc::live::SUBPROTOCOL),
			);
		}

		Ok(response)
	};
	let Ok(client) = tokio_tungstenite::accept_hdr_async(stream, callback).await else {
		return;
	};

	let mut request = http::Request::builder()
		.uri(format!("ws://{server}/live"))
		.header(http::header::HOST, server.to_string())
		.header(http::header::CONNECTION, "Upgrade")
		.header(http::header::UPGRADE, "websocket")
		.header(http::header::SEC_WEBSOCKET_VERSION, "13")
		.header(http::header::SEC_WEBSOCKET_KEY, generate_key())
		.header(http::header::SEC_WEBSOCKET_PROTOCOL, rpc::live::SUBPROTOCOL);

	if let Some(authorization) = authorization {
		request = request.header(http::header::AUTHORIZATION, authorization);
	}

	let Ok((upstream, _)) = tokio_tungstenite::connect_async(request.body(()).unwrap()).await
	else {
		return;
	};

	let (mut client_sink, mut client_stream) = client.split();
	let (mut server_sink, mut server_stream) = upstream.split();
	let cutting = Arc::new(AtomicBool::new(false));
	let upstream_cutting = Arc::clone(&cutting);

	let up = async move {
		while let Some(Ok(message)) = client_stream.next().await {
			if let Message::Text(text) = &message
				&& text.contains("\"mutate\"")
				&& text.contains("drop-me")
				&& armed.swap(false, Ordering::SeqCst)
			{
				upstream_cutting.store(true, Ordering::SeqCst);
			}

			if server_sink.send(message).await.is_err() {
				return;
			}
		}
	};
	let down = async move {
		while let Some(Ok(message)) = server_stream.next().await {
			if cutting.load(Ordering::SeqCst)
				&& let Message::Text(text) = &message
				&& text.contains("\"done\"")
			{
				// The mutation committed; its answer is lost with the
				// connection.
				cuts.fetch_add(1, Ordering::SeqCst);
				return;
			}

			if client_sink.send(message).await.is_err() {
				return;
			}
		}
	};

	// Whichever side ends first ends the connection; dropping both halves
	// closes the sockets without a close frame, like a lost network.
	tokio::select! {
		() = up => {}
		() = down => {}
	}
}

/// Generates `language`'s client into `dir`.
fn generate(language: TargetLanguage, package: &str, dir: &Path) {
	let spec = rpc::spec::collect(&SpecOptions::new("SdkLive", "0.1.0")).expect("the spec");
	assert!(
		spec.channels
			.iter()
			.any(|channel| channel.name == "sdk_room"),
		"the channel is published"
	);
	spec.generate(&GenerateOptions::new(language, package))
		.write_to(dir, Scaffold::Full)
		.expect("write the client");
}

/// Whether `program` runs; otherwise the test is skipped, unless every
/// toolchain is required.
fn have(program: &str) -> bool {
	let found = Command::new(program)
		.arg("--version")
		.output()
		.is_ok_and(|output| output.status.success());

	if !found {
		assert!(
			std::env::var("LEPTOSKIT_REQUIRE_SDK_TOOLCHAINS").as_deref() != Ok("1"),
			"`{program}` is required but isn't on PATH"
		);
		eprintln!("skipped: `{program}` isn't on PATH");
	}

	found
}

/// A fresh directory for one test's client.
fn scratch(name: &str) -> tempfile::TempDir {
	tempfile::Builder::new()
		.prefix(&format!("leptoskit_sdk_live_{name}_"))
		.tempdir()
		.expect("a scratch directory")
}

/// The client programs' directory.
fn programs() -> PathBuf {
	Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/sdk_live")
}

/// Runs `command` against the harness, with `SDK_LIVE_URL` and
/// `SDK_LIVE_PREFIX` set, and checks it passed and wrote the `drop-me` note
/// once.
async fn run(harness: &Harness, mut command: Command, prefix: &str) {
	command
		.env("SDK_LIVE_URL", &harness.url)
		.env("SDK_LIVE_PREFIX", prefix);
	let description = format!("{command:?}");
	let output = tokio::task::spawn_blocking(move || command.output())
		.await
		.expect("the client runs")
		.expect("start the client");
	let stdout = String::from_utf8_lossy(&output.stdout);
	let stderr = String::from_utf8_lossy(&output.stderr);
	assert!(
		output.status.success(),
		"{description} failed\n--- stdout\n{stdout}\n--- stderr\n{stderr}"
	);

	for step in [
		"initial",
		"read-your-writes",
		"patch",
		"reconnect",
		"resumed",
		"channel",
	] {
		assert!(
			stdout.lines().any(|line| line == format!("ok {step}")),
			"{description} didn't report `ok {step}`\n--- stdout\n{stdout}\n--- stderr\n{stderr}"
		);
	}

	assert!(
		harness.cuts.load(Ordering::SeqCst) >= 1,
		"the proxy cut a connection"
	);
	let marked = format!("{prefix}drop-me");
	let written = SdkNoteRow::query()
		.all(&harness.db)
		.await
		.unwrap()
		.into_iter()
		.filter(|note| note.body == marked)
		.count();
	assert_eq!(written, 1, "the re-sent mutation ran once");
}

/// Runs a client with a deadline, so a hung client fails the test.
fn deadline(mut command: Command) -> Command {
	// Every client program has its own 60-second budget; this is the
	// harness's backstop, applied by `timeout` where the platform has it.
	if Command::new("timeout").arg("--version").output().is_ok() {
		let mut wrapped = Command::new("timeout");
		wrapped.arg("180").arg(command.get_program());
		wrapped.args(command.get_args());

		if let Some(dir) = command.get_current_dir() {
			wrapped.current_dir(dir);
		}

		for (key, value) in command.get_envs() {
			match value {
				Some(value) => wrapped.env(key, value),
				None => wrapped.env_remove(key),
			};
		}

		command = wrapped;
	}

	command
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs deno; run with --run-ignored all"]
async fn typescript_sdk_runs_live_under_deno() {
	if !have("deno") {
		return;
	}

	let harness = start().await;
	let dir = scratch("deno");
	generate(TargetLanguage::TypeScript, "sdk-live", dir.path());
	std::fs::copy(programs().join("client.ts"), dir.path().join("client.ts")).unwrap();
	let check = Command::new("deno")
		.args(["check", "index.ts", "live.ts", "client.ts"])
		.current_dir(dir.path())
		.output()
		.unwrap();
	assert!(
		check.status.success(),
		"the generated TypeScript type-checks: {}",
		String::from_utf8_lossy(&check.stderr)
	);
	let mut command = Command::new("deno");
	command
		.args(["run", "--allow-net", "--allow-env", "client.ts"])
		.current_dir(dir.path());
	run(&harness, deadline(command), "deno-").await;
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs node 22 or newer; run with --run-ignored all"]
async fn typescript_sdk_runs_live_under_node() {
	if !have("node") {
		return;
	}

	let harness = start().await;
	let dir = scratch("node");
	generate(TargetLanguage::TypeScript, "sdk-live", dir.path());
	std::fs::copy(programs().join("client.ts"), dir.path().join("client.ts")).unwrap();
	let mut command = Command::new("node");
	command
		.args(["--experimental-strip-types", "--no-warnings", "client.ts"])
		.current_dir(dir.path());
	run(&harness, deadline(command), "node-").await;
}
