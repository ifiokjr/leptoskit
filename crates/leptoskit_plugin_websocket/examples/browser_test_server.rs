//! Loopback-only fixture for browser integration tests; never an application
//! server.

#[cfg(not(target_arch = "wasm32"))]
use futures::SinkExt;
#[cfg(not(target_arch = "wasm32"))]
use futures::StreamExt;
#[cfg(not(target_arch = "wasm32"))]
use tokio::net::TcpListener;
#[cfg(not(target_arch = "wasm32"))]
use tokio_tungstenite::tungstenite;

#[cfg(not(target_arch = "wasm32"))]
#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
	let argument = std::env::args().nth(1);
	let run_tests = argument.as_deref() == Some("--test");
	let address = if run_tests {
		"127.0.0.1:0"
	} else {
		argument.as_deref().unwrap_or("127.0.0.1:0")
	};
	let address: std::net::SocketAddr = address.parse()?;
	if !address.ip().is_loopback() {
		return Err("browser test server must bind loopback".into());
	}
	let listener = TcpListener::bind(address).await?;
	let origin = format!("ws://{}", listener.local_addr()?);
	println!("{origin}");
	if !run_tests {
		serve(listener).await?;
		return Ok(());
	}
	let mut server = tokio::spawn(serve(listener));
	let mut command =
		tokio::process::Command::new(std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into()));
	command
		.current_dir(env!("CARGO_MANIFEST_DIR"))
		.env("LEPTOSKIT_WEBSOCKET_TEST_URL", origin)
		.args([
			"test",
			"-p",
			"leptoskit_plugin_websocket",
			"--features",
			"browser-integration",
			"--target",
			"wasm32-unknown-unknown",
			"--test",
			"browser",
		])
		.stdin(std::process::Stdio::null())
		.stdout(std::process::Stdio::inherit())
		.stderr(std::process::Stdio::inherit())
		.kill_on_drop(true);
	let mut tests = command.spawn()?;
	let status = tokio::select! {
		status = tests.wait() => status?,
		result = &mut server => {
			return Err(format!("browser fixture exited during tests: {result:?}").into());
		}
	};
	server.abort();
	match server.await {
		Ok(result) => result?,
		Err(error) if error.is_cancelled() => {}
		Err(error) => return Err(error.into()),
	}
	if !status.success() {
		return Err(format!("browser integration tests failed: {status}").into());
	}
	Ok(())
}

#[cfg(not(target_arch = "wasm32"))]
async fn serve(listener: TcpListener) -> std::io::Result<()> {
	loop {
		let (stream, _) = listener.accept().await?;
		tokio::spawn(async move {
			let mut path = String::new();
			#[expect(
				clippy::result_large_err,
				reason = "tungstenite's handshake callback fixes its error type"
			)]
			let socket = tokio_tungstenite::accept_hdr_async(
				stream,
				|request: &tungstenite::handshake::server::Request,
				 response: tungstenite::handshake::server::Response| {
					path = request.uri().path().into();
					Ok(response)
				},
			)
			.await;
			let Ok(mut socket) = socket else { return };
			if path == "/large" {
				if socket
					.send(tungstenite::Message::Binary(vec![0; 9].into()))
					.await
					.is_err()
				{
					return;
				}
			} else if path == "/flood" {
				for _ in 0..8 {
					if socket
						.send(tungstenite::Message::Text("".into()))
						.await
						.is_err()
					{
						return;
					}
				}
			}
			while let Some(message) = socket.next().await {
				match message {
					Ok(tungstenite::Message::Close(_)) => {
						let _ = socket.flush().await;
						break;
					}
					Ok(message) if message.is_text() || message.is_binary() => {
						if socket.send(message).await.is_err() {
							break;
						}
					}
					Err(_) => break,
					_ => {}
				}
			}
		});
	}
}

#[cfg(target_arch = "wasm32")]
fn main() {
	panic!("browser_test_server runs on a native host, not inside the browser");
}
