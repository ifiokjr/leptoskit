use serde_json::Value;

#[cfg(target_arch = "wasm32")]
pub async fn request(key: &str, path: &str, body: Option<Value>) -> Result<Value, String> {
	#[cfg(target_arch = "wasm32")]
	{
		use leptoskit_plugin_http::Header;
		use leptoskit_plugin_http::Method;
		use leptoskit_plugin_http::Request;
		let origin = origin()?;
		let mut request = Request::get(format!("{origin}{path}"));
		request.headers = vec![
			Header {
				name: "authorization".into(),
				value: format!("Bearer {key}").into_bytes(),
			},
			Header {
				name: "content-type".into(),
				value: b"application/json".to_vec(),
			},
		];
		if let Some(body) = body {
			request.method = Method::Post;
			request.body = serde_json::to_vec(&body).map_err(|_| "Invalid request")?;
		}
		#[cfg(feature = "native")]
		let response = leptoskit_plugin_http::guest::HttpClient::connect()
			.map_err(|_| "HTTP plugin unavailable")?
			.execute(request)
			.await
			.map_err(|_| "Connection failed")?;
		#[cfg(all(feature = "web", not(feature = "native")))]
		let response = leptoskit_plugin_http::Client::new(
			leptoskit_plugin_http::Config::new([&origin]).map_err(|_| "Invalid server origin")?,
		)
		.map_err(|_| "Invalid HTTP configuration")?
		.execute(request)
		.await
		.map_err(|_| "Connection failed")?;
		if !(200..300).contains(&response.status) {
			return Err(match response.status {
				401 => "Sign in with a valid account key",
				403 => "Check your subscription and installation permissions",
				409 => "Refresh and review the current application state",
				_ => "The service could not complete this request",
			}
			.into());
		}
		if response.body.is_empty() {
			Ok(Value::Null)
		} else {
			serde_json::from_slice(&response.body).map_err(|_| "Invalid service response".into())
		}
	}
}
#[cfg(not(target_arch = "wasm32"))]
pub fn request(
	_key: &str,
	_path: &str,
	_body: Option<Value>,
) -> impl Future<Output = Result<Value, String>> {
	std::future::ready(Err("Run the app in a browser or native host".into()))
}

#[cfg(target_arch = "wasm32")]
fn origin() -> Result<String, String> {
	#[cfg(all(feature = "web", not(feature = "native")))]
	{
		web_sys::window()
			.ok_or("Browser unavailable")?
			.location()
			.origin()
			.map_err(|_| "Origin unavailable".into())
	}
	#[cfg(feature = "native")]
	{
		let parsed = url::Url::parse(crate::SERVER).map_err(|_| "Invalid server origin")?;
		let local = matches!(parsed.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"));
		if !(parsed.scheme() == "https" || (local && parsed.scheme() == "http"))
			|| parsed.path() != "/"
			|| parsed.query().is_some()
			|| parsed.fragment().is_some()
			|| !parsed.username().is_empty()
			|| parsed.password().is_some()
		{
			return Err("Use an HTTPS server origin, or localhost for development".into());
		}
		Ok(parsed.origin().ascii_serialization())
	}
}
#[cfg(all(target_arch = "wasm32", feature = "native"))]
pub async fn save_key(value: &str) -> Result<(), leptoskit::guest::Error> {
	use leptoskit_plugin_secure_store::Key;
	use leptoskit_plugin_secure_store::guest::SecureStore;
	let store = SecureStore::new()?;
	use sha2::Digest as _;
	let name = format!("{:x}", sha2::Sha256::digest(crate::SERVER.as_bytes()));
	let key = Key::new("leptoskit-cloud", &name)?;
	if value.is_empty() {
		store.delete(key).await
	} else {
		store.set(key, value.as_bytes()).await
	}
}
#[cfg(all(target_arch = "wasm32", feature = "native"))]
pub async fn load_key() -> Option<String> {
	use leptoskit_plugin_secure_store::Key;
	use leptoskit_plugin_secure_store::guest::SecureStore;
	let store = SecureStore::new().ok()?;
	use sha2::Digest as _;
	let name = format!("{:x}", sha2::Sha256::digest(crate::SERVER.as_bytes()));
	let key = Key::new("leptoskit-cloud", &name).ok()?;
	String::from_utf8(store.get(key).await.ok()??.to_vec()).ok()
}

#[cfg(target_arch = "wasm32")]
pub async fn copy(value: &str) -> Result<(), String> {
	#[cfg(feature = "native")]
	{
		leptoskit_plugin_clipboard::guest::Clipboard::new()
			.map_err(|_| "Clipboard unavailable")?
			.write_text(value)
			.await
			.map_err(|_| "Clipboard access denied".into())
	}
	#[cfg(all(feature = "web", not(feature = "native")))]
	{
		leptoskit_plugin_clipboard::web::write_text(value)
			.await
			.map_err(|_| "Clipboard access denied".into())
	}
}
#[cfg(not(target_arch = "wasm32"))]
pub fn copy(_value: &str) -> impl Future<Output = Result<(), String>> {
	std::future::ready(Err("Run in the browser or native app".into()))
}
