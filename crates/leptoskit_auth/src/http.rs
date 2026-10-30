//! Sign-in over HTTP, for axum servers: a layer that resolves each
//! request's session, and the `/auth` routes.
//!
//! ```ignore
//! let app = Router::new()
//! 	.merge(my_app_backend::router())     // handlers take `Extension<Db>`, already scoped to the request
//! 	.merge(auth.router())                // /auth/*
//! 	.layer(auth.layer());                // resolves the session first
//! ```

use std::fmt::Write;
use std::future::Future;
use std::net::SocketAddr;
use std::pin::Pin;
use std::task::Context as TaskContext;
use std::task::Poll;

use axum::Extension;
use axum::Json;
use axum::Router;
use axum::body::Body;
use axum::extract::ConnectInfo;
use axum::extract::FromRequestParts;
use axum::extract::OptionalFromRequestParts;
use axum::extract::State;
use axum::http::HeaderMap;
use axum::http::HeaderValue;
use axum::http::Method;
use axum::http::Request;
use axum::http::StatusCode;
use axum::http::header;
use axum::http::request::Parts;
use axum::response::Html;
use axum::response::IntoResponse;
use axum::response::Response;
use axum::routing::get;
use axum::routing::post;
use leptoskit_db::Db;

use crate::Auth;
use crate::AuthError;
use crate::ErrorResponse;
use crate::PublicSession;
use crate::RequestInfo;
use crate::RevokeSession;
use crate::SendCode;
use crate::Session;
use crate::SessionResponse;
use crate::SessionToken;
use crate::SignIn;
use crate::SignedIn;
use crate::SignedInResponse;
use crate::Transport;
use crate::VerifyCode;

/// What the session layer learned about a request, as a request
/// extension.
#[derive(Clone, Debug)]
pub struct CurrentSession {
	/// Who the request is from, if its session is live.
	pub signed_in: Option<SignedIn>,
	/// Where the request came from.
	pub request: RequestInfo,
	pub(crate) token: Option<SessionToken>,
}

impl Auth {
	/// The layer that resolves each request's session before any handler
	/// runs. It adds a [`CurrentSession`], and a [`Db`] whose policies see
	/// the signed-in person, as request extensions. It also refuses
	/// state-changing requests from other sites.
	///
	/// Add it last, so it wraps every route, including [`Auth::router`]'s.
	pub fn layer(&self) -> SessionLayer {
		SessionLayer { auth: self.clone() }
	}

	/// The `/auth` routes:
	///
	/// | Route                                | Does                                               |
	/// | ------------------------------------ | -------------------------------------------------- |
	/// | `POST /auth/email/send`              | emails a code ([`SendCode`])                        |
	/// | `POST /auth/email/verify`            | signs in with it ([`VerifyCode`])                   |
	/// | `GET /auth/session`                  | the request's session ([`SessionResponse`])         |
	/// | `POST /auth/sign-out`                | ends it                                            |
	/// | `GET /auth/sessions`                 | the person's live sessions                         |
	/// | `POST /auth/sessions/revoke`         | ends one of them ([`RevokeSession`])                |
	/// | `POST /auth/sessions/revoke-others`  | ends all but the current one                       |
	///
	/// In development,
	/// [`AuthBuilder::dev_mailbox`](crate::AuthBuilder::dev_mailbox)
	/// adds `GET /_leptoskit/mail`, and
	/// [`AuthBuilder::dev_sign_in`](crate::AuthBuilder::dev_sign_in) adds
	/// `POST /auth/dev/sign-in`.
	pub fn router<S>(&self) -> Router<S>
	where
		S: Clone + Send + Sync + 'static,
	{
		let mut router = Router::new()
			.route("/auth/email/send", post(send_code))
			.route("/auth/email/verify", post(verify_code))
			.route("/auth/session", get(session))
			.route("/auth/sign-out", post(sign_out))
			.route("/auth/sessions", get(sessions))
			.route("/auth/sessions/revoke", post(revoke))
			.route("/auth/sessions/revoke-others", post(revoke_others));

		if cfg!(debug_assertions) {
			if self.inner.dev_mailbox.is_some() {
				router = router.route("/_leptoskit/mail", get(mailbox));
			}

			if self.inner.dev_sign_in {
				router = router.route("/auth/dev/sign-in", post(dev_sign_in));
			}
		}

		router.with_state(self.clone())
	}

	/// What the layer learns about a request: its session, from the bearer
	/// token or the session cookie, and where it came from.
	async fn current_session(
		&self,
		parts_headers: &HeaderMap,
		peer: Option<SocketAddr>,
	) -> Result<CurrentSession, AuthError> {
		let request = RequestInfo {
			ip: self.client_ip(parts_headers, peer),
			user_agent: parts_headers
				.get(header::USER_AGENT)
				.and_then(|agent| agent.to_str().ok())
				.map(str::to_owned),
		};
		let token = bearer(parts_headers).or_else(|| cookie(parts_headers, self.cookie_name()));
		let signed_in = match &token {
			Some(token) => self.resolve(token).await?,
			None => None,
		};

		Ok(CurrentSession {
			signed_in,
			request,
			token,
		})
	}

	fn client_ip(&self, headers: &HeaderMap, peer: Option<SocketAddr>) -> Option<std::net::IpAddr> {
		match &self.inner.client_ip_header {
			Some(name) => {
				headers
					.get(name.as_str())
					.and_then(|value| value.to_str().ok())
					.and_then(|value| value.split(',').next())
					.and_then(|value| value.trim().parse().ok())
			}
			None => peer.map(|peer| peer.ip()),
		}
	}

	/// Whether a request may change anything: safe methods always may;
	/// otherwise a browser's request must come from this site.
	fn same_site(&self, method: &Method, headers: &HeaderMap) -> bool {
		if matches!(*method, Method::GET | Method::HEAD | Method::OPTIONS) {
			return true;
		}

		// A bearer token is never sent by a browser on its own, so a request
		// that carries one, and no session cookie, can't have been forged by
		// another site. This is how browser extensions and other apps on their
		// own origin call.
		let bearer = headers
			.get(header::AUTHORIZATION)
			.and_then(|value| value.to_str().ok())
			.is_some_and(|value| value.starts_with("Bearer "));
		if bearer && cookie(headers, self.cookie_name()).is_none() {
			return true;
		}

		if let Some(origin) = headers.get(header::ORIGIN) {
			let origin = origin.to_str().unwrap_or_default();
			if let Some(expected) = &self.inner.origin {
				return origin == expected;
			}

			let host = headers
				.get(header::HOST)
				.and_then(|host| host.to_str().ok());
			return origin
				.split_once("://")
				.is_some_and(|(_, authority)| Some(authority) == host);
		}

		if let Some(site) = headers.get("sec-fetch-site") {
			return site == "same-origin" || site == "none";
		}

		// Neither header: not a browser, unless it sends the cookie a browser
		// would attach on its own.
		cookie(headers, self.cookie_name()).is_none()
	}

	/// Whether the session cookie needs `Secure`: always, unless the site
	/// is served over plain HTTP, which only development does.
	fn secure(&self) -> bool {
		self.inner
			.origin
			.as_deref()
			.is_none_or(|origin| !origin.starts_with("http://"))
	}

	/// `__Host-session` when secure, which browsers only accept from this
	/// exact origin over HTTPS.
	fn cookie_name(&self) -> &'static str {
		if self.secure() {
			"__Host-session"
		} else {
			"session"
		}
	}

	pub(crate) fn session_cookie(&self, token: &SessionToken) -> HeaderValue {
		let max_age = self.inner.sessions.absolute.as_secs();
		self.cookie(token.expose(), max_age)
	}

	pub(crate) fn cleared_cookie(&self) -> HeaderValue {
		self.cookie("", 0)
	}

	fn cookie(&self, value: &str, max_age: u64) -> HeaderValue {
		let secure = if self.secure() { "; Secure" } else { "" };
		let cookie = format!(
			"{}={value}; Path=/; HttpOnly; SameSite=Lax; Max-Age={max_age}{secure}",
			self.cookie_name()
		);
		HeaderValue::from_str(&cookie).expect("tokens are base64url, which headers allow")
	}

	/// The answer to a sign-in: the cookie, or the token in the body.
	fn signed_in_response(&self, sign_in: SignIn, transport: Transport) -> Response {
		let body = SignedInResponse {
			session: PublicSession::from(sign_in.session),
			signed_up: sign_in.signed_up,
			token: (transport == Transport::Bearer).then(|| sign_in.token.expose().to_owned()),
		};
		let mut response = Json(body).into_response();

		if transport == Transport::Cookie {
			response
				.headers_mut()
				.insert(header::SET_COOKIE, self.session_cookie(&sign_in.token));
		}

		response
	}
}

fn bearer(headers: &HeaderMap) -> Option<SessionToken> {
	let value = headers.get(header::AUTHORIZATION)?.to_str().ok()?;
	let (scheme, token) = value.split_once(' ')?;
	scheme
		.eq_ignore_ascii_case("bearer")
		.then(|| SessionToken::new(token.trim()))
}

fn cookie(headers: &HeaderMap, name: &str) -> Option<SessionToken> {
	headers
		.get_all(header::COOKIE)
		.iter()
		.filter_map(|value| value.to_str().ok())
		.flat_map(|value| value.split(';'))
		.filter_map(|pair| pair.trim().split_once('='))
		.find(|(key, value)| *key == name && !value.is_empty())
		.map(|(_, value)| SessionToken::new(value))
}

/// The layer [`Auth::layer`] returns.
#[derive(Clone, Debug)]
pub struct SessionLayer {
	auth: Auth,
}

impl<S> tower::Layer<S> for SessionLayer {
	type Service = SessionService<S>;

	fn layer(&self, inner: S) -> Self::Service {
		SessionService {
			auth: self.auth.clone(),
			inner,
		}
	}
}

/// The service [`SessionLayer`] wraps routes in.
#[derive(Clone, Debug)]
pub struct SessionService<S> {
	auth: Auth,
	inner: S,
}

impl<S> tower::Service<Request<Body>> for SessionService<S>
where
	S: tower::Service<Request<Body>, Response = Response> + Clone + Send + 'static,
	S::Future: Send,
{
	type Error = S::Error;
	type Future = Pin<Box<dyn Future<Output = Result<Response, S::Error>> + Send>>;
	type Response = Response;

	fn poll_ready(&mut self, cx: &mut TaskContext<'_>) -> Poll<Result<(), Self::Error>> {
		self.inner.poll_ready(cx)
	}

	fn call(&mut self, mut request: Request<Body>) -> Self::Future {
		let auth = self.auth.clone();
		// The clone isn't ready; the original, which `poll_ready` readied, is
		// the one to call.
		let clone = self.inner.clone();
		let mut inner = std::mem::replace(&mut self.inner, clone);

		Box::pin(async move {
			if !auth.same_site(request.method(), request.headers()) {
				// Asking for a code, and signing in for a bearer token, set no
				// cookie, so another origin, such as a browser extension, may
				// do both. The verify route refuses cookie sign-in from
				// elsewhere, which would sign the browser in to someone else's
				// account.
				let without_cookie = cookie(request.headers(), auth.cookie_name()).is_none();
				let sets_no_cookie = matches!(
					request.uri().path(),
					"/auth/email/send" | "/auth/email/verify"
				);
				if !(without_cookie && sets_no_cookie) {
					return Ok(error_response(
						StatusCode::FORBIDDEN,
						"forbidden",
						"this request came from another site",
					));
				}
				request.extensions_mut().insert(CrossSite);
			}

			let peer = request
				.extensions()
				.get::<ConnectInfo<SocketAddr>>()
				.map(|ConnectInfo(peer)| *peer);
			let current = match auth.current_session(request.headers(), peer).await {
				Ok(current) => current,
				Err(error) => return Ok(error.into_response()),
			};
			let db = auth
				.inner
				.db
				.with_context(crate::context(current.signed_in.clone()));
			request.extensions_mut().insert(db);
			request.extensions_mut().insert(auth.clone());
			request.extensions_mut().insert(current);

			inner.call(request).await
		})
	}
}

impl AuthError {
	/// The status and body an HTTP client gets for this error.
	pub(crate) fn describe(&self) -> (StatusCode, ErrorResponse) {
		let (status, code) = match self {
			Self::InvalidEmail => (StatusCode::BAD_REQUEST, "invalid_email"),
			Self::InvalidCode => (StatusCode::BAD_REQUEST, "invalid_code"),
			Self::RateLimited { .. } => (StatusCode::TOO_MANY_REQUESTS, "rate_limited"),
			Self::Disabled => (StatusCode::FORBIDDEN, "disabled"),
			Self::Locked { .. } => (StatusCode::FORBIDDEN, "locked"),
			Self::SignedOut => (StatusCode::UNAUTHORIZED, "signed_out"),
			Self::Mail(_) => (StatusCode::BAD_GATEWAY, "mail"),
			Self::SignUp(_) => (StatusCode::UNPROCESSABLE_ENTITY, "sign_up"),
			Self::Config(_) | Self::Db(_) => (StatusCode::INTERNAL_SERVER_ERROR, "server"),
		};
		let retry_after = match self {
			Self::RateLimited { retry_after } => Some(retry_after.as_secs().max(1)),
			_ => None,
		};
		// The server's own failures never reach the client, which hears only
		// that something went wrong.
		let message = match self {
			Self::Config(_) | Self::Db(_) => "something went wrong; try again".to_owned(),
			Self::Mail(_) => "the email couldn't be sent; try again".to_owned(),
			Self::SignUp(_) => "the account couldn't be created".to_owned(),
			_ => self.to_string(),
		};
		(
			status,
			ErrorResponse {
				error: code.to_owned(),
				message,
				retry_after,
			},
		)
	}
}

impl From<AuthError> for ErrorResponse {
	fn from(error: AuthError) -> Self {
		error.describe().1
	}
}

impl IntoResponse for AuthError {
	fn into_response(self) -> Response {
		let (status, body) = self.describe();
		let retry_after = body.retry_after;
		let mut response = (status, Json(body)).into_response();

		if let Some(seconds) = retry_after {
			response
				.headers_mut()
				.insert(header::RETRY_AFTER, HeaderValue::from(seconds));
		}

		response
	}
}

fn error_response(status: StatusCode, code: &str, message: &str) -> Response {
	(
		status,
		Json(ErrorResponse {
			error: code.to_owned(),
			message: message.to_owned(),
			retry_after: None,
		}),
	)
		.into_response()
}

/// The signed-in person, or `401 signed_out`:
/// `async fn drafts(me: SignedIn, Extension(db): Extension<Db>) -> …`.
impl<S: Send + Sync> FromRequestParts<S> for SignedIn {
	type Rejection = AuthError;

	async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
		parts
			.extensions
			.get::<CurrentSession>()
			.and_then(|current| current.signed_in.clone())
			.ok_or(AuthError::SignedOut)
	}
}

/// `Option<SignedIn>`: the signed-in person, if there is one.
impl<S: Send + Sync> OptionalFromRequestParts<S> for SignedIn {
	type Rejection = std::convert::Infallible;

	async fn from_request_parts(
		parts: &mut Parts,
		_state: &S,
	) -> Result<Option<Self>, Self::Rejection> {
		Ok(parts
			.extensions
			.get::<CurrentSession>()
			.and_then(|current| current.signed_in.clone()))
	}
}

/// Where the request came from, as the layer saw it.
impl<S: Send + Sync> FromRequestParts<S> for RequestInfo {
	type Rejection = std::convert::Infallible;

	async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
		Ok(parts
			.extensions
			.get::<CurrentSession>()
			.map(|current| current.request.clone())
			.unwrap_or_default())
	}
}

async fn send_code(
	State(auth): State<Auth>,
	request: RequestInfo,
	Json(body): Json<SendCode>,
) -> Result<StatusCode, AuthError> {
	auth.send_email_code(&body.email, &request).await?;
	Ok(StatusCode::NO_CONTENT)
}

/// Marks a request the same-site check let through only because it sets no
/// cookie.
#[derive(Clone, Copy, Debug)]
struct CrossSite;

async fn verify_code(
	State(auth): State<Auth>,
	cross_site: Option<Extension<CrossSite>>,
	request: RequestInfo,
	Json(body): Json<VerifyCode>,
) -> Result<Response, AuthError> {
	if cross_site.is_some() && body.transport == Transport::Cookie {
		return Ok(error_response(
			StatusCode::FORBIDDEN,
			"forbidden",
			"this request came from another site",
		));
	}
	let sign_in = auth
		.verify_email_code(&body.email, &body.code, &request)
		.await?;
	Ok(auth.signed_in_response(sign_in, body.transport))
}

async fn session(
	State(auth): State<Auth>,
	me: Option<SignedIn>,
) -> Result<Json<SessionResponse>, AuthError> {
	let session = match me {
		Some(me) => {
			Session::find(&auth.inner.db, me.session)
				.await?
				.map(PublicSession::from)
		}
		None => None,
	};
	Ok(Json(SessionResponse { session }))
}

async fn sign_out(
	State(auth): State<Auth>,
	Extension(current): Extension<CurrentSession>,
) -> Result<Response, AuthError> {
	if let Some(token) = &current.token {
		auth.sign_out(token, &current.request).await?;
	}

	let mut response = StatusCode::NO_CONTENT.into_response();
	response
		.headers_mut()
		.insert(header::SET_COOKIE, auth.cleared_cookie());
	Ok(response)
}

async fn sessions(
	_me: SignedIn,
	Extension(db): Extension<Db>,
) -> Result<Json<Vec<PublicSession>>, AuthError> {
	let now = leptoskit_db::now();
	// The request's own database: the sessions table's policy limits this to
	// the person's sessions.
	let sessions = Session::query()
		.filter(|session| {
			session.revoked_at.is_null()
				& session.expires_at.gt(now)
				& session.idle_expires_at.gt(now)
		})
		.order_by(|session| session.last_seen_at.desc())
		.all(&db)
		.await?;
	Ok(Json(
		sessions.into_iter().map(PublicSession::from).collect(),
	))
}

async fn revoke(
	State(auth): State<Auth>,
	me: SignedIn,
	request: RequestInfo,
	Json(body): Json<RevokeSession>,
) -> Result<StatusCode, AuthError> {
	if auth.revoke(&me, body.id, &request).await? {
		Ok(StatusCode::NO_CONTENT)
	} else {
		Ok(StatusCode::NOT_FOUND)
	}
}

async fn revoke_others(
	State(auth): State<Auth>,
	me: SignedIn,
	request: RequestInfo,
) -> Result<StatusCode, AuthError> {
	auth.revoke_others(&me, &request).await?;
	Ok(StatusCode::NO_CONTENT)
}

async fn dev_sign_in(
	State(auth): State<Auth>,
	request: RequestInfo,
	Json(body): Json<SendCode>,
) -> Result<Response, AuthError> {
	let sign_in = auth.dev_sign_in(&body.email, &request).await?;
	Ok(auth.signed_in_response(sign_in, Transport::Cookie))
}

/// The development mailbox, newest first.
async fn mailbox(State(auth): State<Auth>) -> Html<String> {
	let messages = auth
		.inner
		.dev_mailbox
		.as_ref()
		.map(crate::Mailbox::messages)
		.unwrap_or_default();
	let mut page = String::from(
		"<!doctype html><meta charset=utf-8><title>Mailbox</title><style>body{font:16px \
		 system-ui;max-width:40rem;margin:2rem auto;padding:0 1rem}article{border-top:1px solid \
		 #ccc;padding:1rem 0}pre{white-space:pre-wrap}</style><h1>Mailbox</h1>",
	);

	if messages.is_empty() {
		page.push_str("<p>No email yet. Ask for a sign-in code, then reload.</p>");
	}

	for mail in messages.iter().rev() {
		let _ = write!(
			page,
			"<article><p><strong>To:</strong> {}</p><h2>{}</h2><pre>{}</pre></article>",
			escape(&mail.to),
			escape(&mail.subject),
			escape(&mail.text)
		);
	}

	Html(page)
}

fn escape(text: &str) -> String {
	text.replace('&', "&amp;")
		.replace('<', "&lt;")
		.replace('>', "&gt;")
		.replace('"', "&quot;")
}
