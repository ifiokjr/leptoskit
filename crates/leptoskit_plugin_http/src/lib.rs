//! Explicitly scoped HTTP clients with bounded request/response bodies, a total
//! timeout, and cancellation by dropping the request future. Redirects are not
//! followed: native callers receive the 3xx response, while browsers reject
//! redirecting fetches because their target cannot be inspected safely first.
//!
//! Native registrants enable `async-host` and install a scoped HTTP handler.
//! Wasm apps enable `guest` and use the cached `guest::HttpClient`.

#[cfg(any(not(target_arch = "wasm32"), feature = "browser"))]
use std::time::Duration;

#[cfg(any(not(target_arch = "wasm32"), feature = "browser"))]
use url::Url;

#[cfg(all(feature = "guest", target_arch = "wasm32"))]
pub mod guest;
#[cfg(all(feature = "async-host", not(target_arch = "wasm32")))]
mod host;
pub mod protocol;
#[cfg(all(feature = "async-host", not(target_arch = "wasm32")))]
pub use host::HttpPlugin;
#[cfg(all(feature = "async-host", not(target_arch = "wasm32")))]
pub use host::from_config;

/// Request failures independent of the selected backend.
#[derive(Debug, thiserror::Error)]
pub enum Error {
	/// Invalid URL, header, method/body combination, or configuration.
	#[error("invalid request: {0}")]
	Invalid(String),
	/// Destination origin was not explicitly allowed by the host.
	#[error("destination is not allowed")]
	DestinationDenied,
	/// Request/response body or headers exceeded the configured bound.
	#[error("request or response exceeded its size limit")]
	TooLarge,
	/// The entire request, including body reading, exceeded its deadline.
	#[error("HTTP request timed out")]
	Timeout,
	/// Transport, TLS, CORS, or browser fetch failure.
	#[error("HTTP transport failed: {0}")]
	Transport(String),
	/// No window/fetch API is available in the current environment.
	#[error("HTTP backend is unavailable")]
	Unavailable,
}

/// Explicit HTTP method. CONNECT/TRACE are deliberately not exposed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
	/// Retrieve a representation.
	Get,
	/// Retrieve response headers.
	Head,
	/// Submit content.
	Post,
	/// Replace content.
	Put,
	/// Partially update content.
	Patch,
	/// Delete content.
	Delete,
	/// Query supported operations.
	Options,
}
impl Method {
	/// Standard token used on the wire.
	pub fn as_str(self) -> &'static str {
		match self {
			Self::Get => "GET",
			Self::Head => "HEAD",
			Self::Post => "POST",
			Self::Put => "PUT",
			Self::Patch => "PATCH",
			Self::Delete => "DELETE",
			Self::Options => "OPTIONS",
		}
	}
}

/// Header values preserve native response bytes. Browser requests require UTF-8
/// values; browser responses expose only headers permitted by CORS.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Header {
	/// Case-insensitive HTTP field name.
	pub name: String,
	/// Field value without CR/LF.
	pub value: Vec<u8>,
}

/// Owned request. Body ownership moves into the transport.
#[derive(Debug, Clone)]
pub struct Request {
	/// HTTP method.
	pub method: Method,
	/// Absolute HTTP(S) destination.
	pub url: String,
	/// Additional request headers.
	pub headers: Vec<Header>,
	/// Binary request body. GET/HEAD bodies are rejected consistently.
	pub body: Vec<u8>,
}
impl Request {
	/// Creates an empty GET request.
	pub fn get(url: impl Into<String>) -> Self {
		Self {
			method: Method::Get,
			url: url.into(),
			headers: Vec::new(),
			body: Vec::new(),
		}
	}
}

/// Completed bounded response; unsuccessful HTTP status codes are still valid
/// responses and do not become transport failures.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Response {
	/// Numeric HTTP status.
	pub status: u16,
	/// Final URL (redirects are disabled).
	pub url: String,
	/// Native headers, or browser CORS-visible headers.
	pub headers: Vec<Header>,
	/// Binary response bytes.
	pub body: Vec<u8>,
}

/// Host-owned limits and exact origin allowlist. There is no allow-all default.
/// An origin grants every path on that origin; this is not a DNS/IP firewall.
#[derive(Debug, Clone)]
#[cfg(any(not(target_arch = "wasm32"), feature = "browser"))]
pub struct Config {
	origins: Vec<url::Origin>,
	/// Maximum request or response body, enforced while receiving chunks.
	pub max_body_bytes: usize,
	/// Maximum combined name/value bytes for exposed request/response headers.
	pub max_header_bytes: usize,
	/// Total deadline, including connection, headers and body.
	pub timeout: Duration,
}
#[cfg(any(not(target_arch = "wasm32"), feature = "browser"))]
impl Config {
	/// Accepts exact `http://host[:port]` / `https://host[:port]` origins only.
	/// Paths, credentials, queries, fragments, wildcard hosts, and opaque URL
	/// schemes are rejected. Plain HTTP must therefore be opted into
	/// explicitly.
	pub fn new(origins: impl IntoIterator<Item = impl AsRef<str>>) -> Result<Self, Error> {
		let mut allowed = Vec::new();
		for origin in origins {
			let url = parse_url(origin.as_ref())?;
			if url.path() != "/" || url.query().is_some() {
				return Err(Error::Invalid("allowlist entries must be origins".into()));
			}
			if !allowed.contains(&url.origin()) {
				allowed.push(url.origin());
			}
		}
		if allowed.is_empty() {
			return Err(Error::Invalid("at least one origin is required".into()));
		}
		Ok(Self {
			origins: allowed,
			max_body_bytes: 8 * 1024 * 1024,
			max_header_bytes: 32 * 1024,
			timeout: Duration::from_secs(30),
		})
	}

	fn validate(&self) -> Result<(), Error> {
		if self.max_body_bytes == 0
			|| self.max_header_bytes == 0
			|| self.timeout.is_zero()
			|| self.timeout.as_millis() > i32::MAX as u128
		{
			return Err(Error::Invalid(
				"limits must be positive and timeout must fit a browser timer".into(),
			));
		}
		Ok(())
	}

	fn request_url(&self, request: &Request) -> Result<Url, Error> {
		let url = parse_url(&request.url)?;
		if !self.origins.contains(&url.origin()) {
			return Err(Error::DestinationDenied);
		}
		if request.body.len() > self.max_body_bytes {
			return Err(Error::TooLarge);
		}
		if matches!(request.method, Method::Get | Method::Head) && !request.body.is_empty() {
			return Err(Error::Invalid(
				"GET/HEAD request bodies are unsupported".into(),
			));
		}
		check_headers(&request.headers, self.max_header_bytes)?;
		for header in &request.headers {
			if matches!(
				header.name.to_ascii_lowercase().as_str(),
				"host"
					| "content-length"
					| "connection"
					| "transfer-encoding"
					| "proxy-authorization"
					| "upgrade"
			) {
				return Err(Error::Invalid("transport-owned request header".into()));
			}
		}
		Ok(url)
	}
}

#[cfg(any(not(target_arch = "wasm32"), feature = "browser"))]
fn parse_url(value: &str) -> Result<Url, Error> {
	let url =
		Url::parse(value).map_err(|_| Error::Invalid("absolute HTTP(S) URL required".into()))?;
	if !matches!(url.scheme(), "http" | "https")
		|| url.host_str().is_none_or(|host| host.contains('*'))
		|| !url.username().is_empty()
		|| url.password().is_some()
		|| url.fragment().is_some()
	{
		return Err(Error::Invalid(
			"HTTP(S) URL without credentials or fragment required".into(),
		));
	}
	Ok(url)
}
#[cfg(any(not(target_arch = "wasm32"), feature = "browser"))]
fn check_headers(headers: &[Header], max_bytes: usize) -> Result<(), Error> {
	let mut size = 0_usize;
	for header in headers {
		size = size
			.checked_add(header.name.len())
			.and_then(|sum| sum.checked_add(header.value.len()))
			.ok_or(Error::TooLarge)?;
		if size > max_bytes {
			return Err(Error::TooLarge);
		}
		if header.name.is_empty()
			|| !header
				.name
				.bytes()
				.all(|byte| byte.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&byte))
			|| header.value.iter().any(|byte| {
				*byte == b'\r' || *byte == b'\n' || (*byte < 32 && *byte != b'\t') || *byte == 127
			}) {
			return Err(Error::Invalid("invalid HTTP header".into()));
		}
	}
	Ok(())
}
#[cfg(not(target_arch = "wasm32"))]
fn append_body(output: &mut Vec<u8>, chunk: &[u8], limit: usize) -> Result<(), Error> {
	if chunk.len() > limit.saturating_sub(output.len()) {
		return Err(Error::TooLarge);
	}
	output.extend_from_slice(chunk);
	Ok(())
}

#[cfg(not(target_arch = "wasm32"))]
mod native;
#[cfg(not(target_arch = "wasm32"))]
pub use native::Client;
#[cfg(all(feature = "browser", target_arch = "wasm32", target_os = "unknown"))]
mod web;
#[cfg(all(feature = "browser", target_arch = "wasm32", target_os = "unknown"))]
pub use web::Client;

#[cfg(all(test, any(not(target_arch = "wasm32"), feature = "browser")))]
mod tests {
	use super::*;
	#[test]
	fn only_explicit_origins_and_safe_urls_are_accepted() {
		let config = Config::new(["https://example.com"]).unwrap();
		assert!(
			config
				.request_url(&Request::get("https://example.com/data?q=1"))
				.is_ok()
		);
		for url in [
			"http://example.com",
			"https://example.com:444",
			"https://example.com.attacker.test",
		] {
			assert!(matches!(
				config.request_url(&Request::get(url)),
				Err(Error::DestinationDenied)
			));
		}
		for url in [
			"file:///secret",
			"data:text/plain,hello",
			"ftp://example.com",
			"https://user:pass@example.com",
			"https://example.com/#fragment",
			"not a URL",
		] {
			assert!(matches!(
				config.request_url(&Request::get(url)),
				Err(Error::Invalid(_))
			));
		}
		assert!(Config::new(["https://example.com/path"]).is_err());
	}
	#[test]
	fn request_limits_and_header_injection_are_checked_before_io() {
		let mut config = Config::new(["https://example.com"]).unwrap();
		config.max_body_bytes = 2;
		let mut request = Request::get("https://example.com");
		request.method = Method::Post;
		request.body = vec![1, 2, 3];
		assert!(matches!(config.request_url(&request), Err(Error::TooLarge)));
		request.body.clear();
		request.headers.push(Header {
			name: "X-Test".into(),
			value: b"yes\r\nInjected: true".to_vec(),
		});
		assert!(matches!(
			config.request_url(&request),
			Err(Error::Invalid(_))
		));
		request.headers = vec![Header {
			name: "Host".into(),
			value: b"elsewhere.test".to_vec(),
		}];
		assert!(matches!(
			config.request_url(&request),
			Err(Error::Invalid(_))
		));
	}

	#[test]
	fn configuration_limits_methods_and_header_sizes_are_validated() {
		let config = Config::new(["https://example.com", "https://example.com/"]).unwrap();
		assert_eq!(config.origins.len(), 1, "duplicate origins collapse");
		assert!(matches!(
			Config::new(Vec::<&str>::new()),
			Err(Error::Invalid(_))
		));
		for broken in [
			Config {
				max_body_bytes: 0,
				..config.clone()
			},
			Config {
				max_header_bytes: 0,
				..config.clone()
			},
			Config {
				timeout: Duration::ZERO,
				..config.clone()
			},
			Config {
				timeout: Duration::from_millis(u64::from(u32::MAX)),
				..config.clone()
			},
		] {
			assert!(matches!(broken.validate(), Err(Error::Invalid(_))));
		}
		let tokens = [
			(Method::Get, "GET"),
			(Method::Head, "HEAD"),
			(Method::Post, "POST"),
			(Method::Put, "PUT"),
			(Method::Patch, "PATCH"),
			(Method::Delete, "DELETE"),
			(Method::Options, "OPTIONS"),
		];
		for (method, token) in tokens {
			assert_eq!(method.as_str(), token);
		}
		let mut request = Request::get("https://example.com");
		request.body = vec![1];
		assert!(matches!(
			config.request_url(&request),
			Err(Error::Invalid(_))
		));
		request.body.clear();
		request.headers.push(Header {
			name: "X-Large".into(),
			value: vec![b'a'; config.max_header_bytes],
		});
		assert!(matches!(config.request_url(&request), Err(Error::TooLarge)));
	}
}
