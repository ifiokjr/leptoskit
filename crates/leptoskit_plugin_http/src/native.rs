use std::error::Error as _;
use std::net::ToSocketAddrs as _;
use std::sync::Arc;

use super::*;

#[derive(Debug)]
struct Resolver {
	jobs: Arc<tokio::sync::Semaphore>,
}

#[derive(Debug, thiserror::Error)]
#[error("HTTP resolver capacity exhausted")]
struct ResolverBusy;

impl reqwest::dns::Resolve for Resolver {
	fn resolve(&self, name: reqwest::dns::Name) -> reqwest::dns::Resolving {
		let permit = Arc::clone(&self.jobs).try_acquire_owned();
		Box::pin(async move {
			let permit = permit.map_err(|_| ResolverBusy)?;
			let addresses = tokio::task::spawn_blocking(move || {
				// Cancelling a queued blocking task does not immediately remove
				// its allocation from Tokio's queue. Keep admission inside the
				// closure until the OS resolver actually finishes.
				let _permit = permit;
				(name.as_str(), 0).to_socket_addrs()
			})
			.await??;
			Ok(Box::new(addresses) as reqwest::dns::Addrs)
		})
	}
}

/// Reusable pooled native client. Construct and call within a Tokio runtime.
/// Dropping an execute future stops reading and releases its in-flight request.
#[derive(Debug, Clone)]
pub struct Client {
	transport: reqwest::Client,
	config: Config,
}
impl Client {
	/// Builds a scoped client with TLS certificate validation, no automatic
	/// redirects, no ambient proxy, and no cookie jar or implicit compression.
	pub fn new(config: Config) -> Result<Self, Error> {
		config.validate()?;
		let transport = reqwest::Client::builder()
			.redirect(reqwest::redirect::Policy::none())
			.no_proxy()
			.timeout(config.timeout)
			.dns_resolver(Arc::new(Resolver {
				jobs: Arc::new(tokio::sync::Semaphore::new(4)),
			}))
			.build()
			.map_err(transport_error)?;
		Ok(Self { transport, config })
	}

	/// Executes one bounded request. HTTP 4xx/5xx statuses are returned
	/// normally.
	pub async fn execute(&self, request: Request) -> Result<Response, Error> {
		let url = self.config.request_url(&request)?;
		let method = reqwest::Method::from_bytes(request.method.as_str().as_bytes())
			.map_err(|_| Error::Invalid("unsupported method".into()))?;
		let mut builder = self.transport.request(method, url).body(request.body);

		for header in request.headers {
			let name = reqwest::header::HeaderName::from_bytes(header.name.as_bytes())
				.map_err(|_| Error::Invalid("invalid header name".into()))?;
			let value = reqwest::header::HeaderValue::from_bytes(&header.value)
				.map_err(|_| Error::Invalid("invalid header value".into()))?;
			builder = builder.header(name, value);
		}

		let mut response = builder.send().await.map_err(transport_error)?;

		if response
			.content_length()
			.is_some_and(|size| size > self.config.max_body_bytes as u64)
		{
			return Err(Error::TooLarge);
		}

		let mut headers = Vec::with_capacity(response.headers().len());
		let mut header_bytes = 0_usize;

		for (name, value) in response.headers() {
			header_bytes = header_bytes
				.checked_add(name.as_str().len())
				.and_then(|size| size.checked_add(value.as_bytes().len()))
				.ok_or(Error::TooLarge)?;

			if header_bytes > self.config.max_header_bytes {
				return Err(Error::TooLarge);
			}

			headers.push(Header {
				name: name.to_string(),
				value: value.as_bytes().to_vec(),
			});
		}

		let mut result = Response {
			status: response.status().as_u16(),
			url: response.url().to_string(),
			headers,
			body: Vec::new(),
		};

		while let Some(chunk) = response.chunk().await.map_err(transport_error)? {
			append_body(&mut result.body, &chunk, self.config.max_body_bytes)?;
		}

		Ok(result)
	}
}
fn transport_error(error: reqwest::Error) -> Error {
	let mut source = error.source();

	while let Some(cause) = source {
		if cause.is::<ResolverBusy>() {
			return Error::Backpressure;
		}

		source = cause.source();
	}

	if error.is_timeout() {
		Error::Timeout
	} else {
		Error::Transport(error.without_url().to_string())
	}
}
