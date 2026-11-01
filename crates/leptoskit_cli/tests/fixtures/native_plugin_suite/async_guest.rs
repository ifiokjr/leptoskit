/// Runs through the production guest bridge in the generated Check files
/// action.
async fn async_plugins() -> Result<(), leptoskit_bridge::Error> {
	use leptoskit_bridge::Error;
	use leptoskit_plugin_http::Request;
	use leptoskit_plugin_http::guest::HttpClient;
	use leptoskit_plugin_websocket::Message;
	use leptoskit_plugin_websocket::guest::WebSocketClient;

	let http = HttpClient::connect()?;
	let mut pending = Box::pin(http.execute(Request::get(format!("{HTTP_ORIGIN}/slow"))));

	if !futures::poll!(&mut pending).is_pending() {
		return Err(Error::Native);
	}

	let bytes = http
		.execute(Request::get(format!("{HTTP_ORIGIN}/bytes")))
		.await?;
	drop(pending);

	if bytes.status != 200 || bytes.body != [0, 127, 255] {
		return Err(Error::Native);
	}

	if http
		.execute(Request::get("http://denied.invalid/bytes"))
		.await
		!= Err(Error::PermissionDenied)
	{
		return Err(Error::Native);
	}
	let redirect = http
		.execute(Request::get(format!("{HTTP_ORIGIN}/redirect")))
		.await?;

	if redirect.status != 302 {
		return Err(Error::Native);
	}

	if http
		.execute(Request::get(format!("{HTTP_ORIGIN}/large")))
		.await
		!= Err(Error::TooLarge)
	{
		return Err(Error::Native);
	}
	let websocket = WebSocketClient::connect()?;
	if !matches!(
		websocket.open("ws://denied.invalid/echo", &[]).await,
		Err(Error::PermissionDenied)
	) {
		return Err(Error::Native);
	}
	let socket = websocket.open(&format!("{WS_ORIGIN}/echo"), &[]).await?;

	for message in [
		Message::Text("async guest ready".into()),
		Message::Binary(vec![0, 127, 255]),
	] {
		socket.send(message.clone()).await?;

		if socket.receive().await? != Some(message) {
			return Err(Error::Native);
		}
	}
	socket.close(1000, "complete").await?;
	Ok(())
}
