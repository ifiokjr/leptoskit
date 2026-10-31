//! Sign in with your app over HTTP, for axum servers: the `/oauth` routes,
//! and the layer that serves the token endpoints and lets third-party
//! tokens reach the functions that accept them.
//!
//! ```ignore
//! let app = Router::new()
//! 	.merge(my_app_backend::router())
//! 	.merge(auth.router())     // /auth/*
//! 	.merge(idp.router())      // /oauth/authorize, consent, connected apps, sign-out
//! 	.layer(auth.layer())      // resolves the session
//! 	.layer(idp.layer());      // token endpoints, and third-party tokens: outermost
//! ```

use std::convert::Infallible;
use std::fmt::Write;
use std::future::Future;
use std::pin::Pin;
use std::task::Context as TaskContext;
use std::task::Poll;

use axum::Extension;
use axum::Form;
use axum::Json;
use axum::Router;
use axum::body::Body;
use axum::extract::Query;
use axum::extract::State;
use axum::extract::rejection::FormRejection;
use axum::http::HeaderMap;
use axum::http::HeaderValue;
use axum::http::Method;
use axum::http::Request;
use axum::http::StatusCode;
use axum::http::header;
use axum::response::Html;
use axum::response::IntoResponse;
use axum::response::Response;
use axum::routing::get;
use axum::routing::post;
use leptoskit_auth::CurrentSession;
use leptoskit_auth::Delegation;
use leptoskit_auth::RequestInfo;
use leptoskit_auth::SignedIn;
use serde::Deserialize;
use tower::ServiceExt;

use crate::AuthorizeOutcome;
use crate::AuthorizeRequest;
use crate::ClientCredentials;
use crate::ConsentDecision;
use crate::ConsentRedirect;
use crate::DisconnectApp;
use crate::EndSessionRequest;
use crate::Idp;
use crate::IdpError;
use crate::OAuthError;
use crate::TokenRequest;

/// The routes the layer serves itself, outside the session layer: they
/// read no cookie, so apps on other sites may call them.
const ENDPOINTS: [&str; 6] = [
	"/.well-known/openid-configuration",
	"/oauth/jwks",
	"/oauth/token",
	"/oauth/userinfo",
	"/oauth/revoke",
	"/oauth/introspect",
];

impl Idp {
	/// The routes people's browsers visit, which need their session:
	///
	/// | Route                         | Does                                                     |
	/// | ----------------------------- | -------------------------------------------------------- |
	/// | `GET /oauth/authorize`        | starts a sign-in ([`AuthorizeRequest`])                  |
	/// | `GET /oauth/consent`          | what the consent page shows ([`ConsentRequest`](crate::ConsentRequest)) |
	/// | `POST /oauth/consent`         | answers it ([`ConsentDecision`])                         |
	/// | `GET /oauth/end_session`      | an app signing someone out ([`EndSessionRequest`])       |
	/// | `GET /oauth/grants`           | the person's connected apps ([`ConnectedApp`](crate::ConnectedApp)) |
	/// | `POST /oauth/grants/revoke`   | disconnects one ([`DisconnectApp`])                      |
	///
	/// Merge them inside [`Auth::layer`](leptoskit_auth::Auth::layer), like
	/// `auth.router()`. The token endpoints come from [`Idp::layer`].
	pub fn router<S>(&self) -> Router<S>
	where
		S: Clone + Send + Sync + 'static,
	{
		Router::new()
			.route("/oauth/authorize", get(authorize))
			.route("/oauth/consent", get(consent).post(decide))
			.route("/oauth/end_session", get(end_session))
			.route("/oauth/grants", get(grants))
			.route("/oauth/grants/revoke", post(disconnect))
			.with_state(self.clone())
	}

	/// The layer that serves what other apps' servers and browsers call,
	/// and lets their access tokens reach your API:
	///
	/// | Route                                   | Does                                  |
	/// | --------------------------------------- | ------------------------------------- |
	/// | `GET /.well-known/openid-configuration` | discovery                             |
	/// | `GET /oauth/jwks`                       | the public signing keys               |
	/// | `POST /oauth/token`                     | trades codes and refresh tokens       |
	/// | `GET`/`POST /oauth/userinfo`            | claims about the signed-in person     |
	/// | `POST /oauth/revoke`                    | revokes a token (RFC 7009)            |
	/// | `POST /oauth/introspect`                | describes a token (RFC 7662)          |
	///
	/// These answer cross-origin requests, since they read no cookies.
	///
	/// A request carrying one of this provider's access tokens reaches only
	/// an `#[api(scope = …)]` function whose every scope the token carries.
	/// It then runs as the person who consented: the layer hands
	/// [`Auth::layer`](leptoskit_auth::Auth::layer) a [`Delegation`], so
	/// policies apply unchanged. Every other route refuses the token.
	///
	/// Add it last, outside `auth.layer()`.
	pub fn layer(&self) -> IdpLayer {
		IdpLayer {
			idp: self.clone(),
			endpoints: Router::new()
				.route("/.well-known/openid-configuration", get(discovery))
				.route("/oauth/jwks", get(jwks))
				.route("/oauth/token", post(token))
				.route("/oauth/userinfo", get(userinfo).post(userinfo))
				.route("/oauth/revoke", post(revoke))
				.route("/oauth/introspect", post(introspect))
				.with_state(self.clone()),
		}
	}
}

/// The layer [`Idp::layer`] returns.
#[derive(Clone)]
pub struct IdpLayer {
	idp: Idp,
	endpoints: Router,
}

impl std::fmt::Debug for IdpLayer {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("IdpLayer")
			.field("idp", &self.idp)
			.finish_non_exhaustive()
	}
}

impl<S> tower::Layer<S> for IdpLayer {
	type Service = IdpService<S>;

	fn layer(&self, inner: S) -> Self::Service {
		IdpService {
			idp: self.idp.clone(),
			endpoints: self.endpoints.clone(),
			inner,
		}
	}
}

/// The service [`IdpLayer`] wraps routes in.
#[derive(Clone)]
pub struct IdpService<S> {
	idp: Idp,
	endpoints: Router,
	inner: S,
}

impl<S> std::fmt::Debug for IdpService<S> {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("IdpService")
			.field("idp", &self.idp)
			.finish_non_exhaustive()
	}
}

impl<S> tower::Service<Request<Body>> for IdpService<S>
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
		let idp = self.idp.clone();
		let endpoints = self.endpoints.clone();
		// The clone isn't ready; the original, which `poll_ready` readied, is
		// the one to call.
		let clone = self.inner.clone();
		let mut inner = std::mem::replace(&mut self.inner, clone);

		Box::pin(async move {
			if ENDPOINTS.contains(&request.uri().path()) {
				let preflight = request.method() == Method::OPTIONS;
				let mut response = if preflight {
					StatusCode::NO_CONTENT.into_response()
				} else {
					let Ok(response) = endpoints
						.oneshot(request)
						.await
						.map_err(|never: Infallible| never);
					response
				};
				allow_any_origin(response.headers_mut(), preflight);
				return Ok(response);
			}

			let Some(token) = bearer(request.headers()).filter(|token| looks_like_jwt(token))
			else {
				return inner.call(request).await;
			};
			let access = match idp.verify_access_token(&token).await {
				Ok(Some(access)) => access,
				Ok(None) => {
					return Ok(bearer_error(
						StatusCode::UNAUTHORIZED,
						"invalid_token",
						None,
					));
				}
				Err(error) => return Ok(json_error(&error)),
			};
			let required = leptoskit_rpc::spec::scopes_for(request.method(), request.uri().path())
				.unwrap_or_default();

			if required.is_empty() {
				// Only functions that name a scope take another app's token.
				return Ok(bearer_error(
					StatusCode::FORBIDDEN,
					"insufficient_scope",
					None,
				));
			}
			if !required
				.iter()
				.all(|scope| access.scopes.iter().any(|granted| granted == scope))
			{
				return Ok(bearer_error(
					StatusCode::FORBIDDEN,
					"insufficient_scope",
					Some(&required.join(" ")),
				));
			}

			request.extensions_mut().insert(Delegation {
				signed_in: access.signed_in(),
				client_id: access.client_id,
				scopes: access.scopes,
			});
			inner.call(request).await
		})
	}
}

/// The token in `Authorization: Bearer`, if there is one.
fn bearer(headers: &HeaderMap) -> Option<String> {
	let value = headers.get(header::AUTHORIZATION)?.to_str().ok()?;
	let (scheme, token) = value.split_once(' ')?;
	scheme
		.eq_ignore_ascii_case("bearer")
		.then(|| token.trim().to_owned())
}

/// Whether a bearer token is shaped like a JWT, so this provider's, rather
/// than a first-party session token, which has no dots.
fn looks_like_jwt(token: &str) -> bool {
	token.bytes().filter(|byte| *byte == b'.').count() == 2
}

/// CORS for the token endpoints: any origin, never credentials, since
/// they read none.
fn allow_any_origin(headers: &mut HeaderMap, preflight: bool) {
	headers.insert(
		header::ACCESS_CONTROL_ALLOW_ORIGIN,
		HeaderValue::from_static("*"),
	);
	if preflight {
		headers.insert(
			header::ACCESS_CONTROL_ALLOW_METHODS,
			HeaderValue::from_static("GET, POST"),
		);
		headers.insert(
			header::ACCESS_CONTROL_ALLOW_HEADERS,
			HeaderValue::from_static("authorization, content-type"),
		);
		headers.insert(
			header::ACCESS_CONTROL_MAX_AGE,
			HeaderValue::from_static("600"),
		);
	}
}

/// A `WWW-Authenticate: Bearer` challenge (RFC 6750 §3).
fn bearer_error(status: StatusCode, code: &str, scope: Option<&str>) -> Response {
	let mut challenge = format!("Bearer error=\"{code}\"");
	if let Some(scope) = scope {
		let _ = write!(challenge, ", scope=\"{scope}\"");
	}
	let mut response = (
		status,
		Json(OAuthError {
			error: code.to_owned(),
			error_description: None,
		}),
	)
		.into_response();
	if let Ok(challenge) = HeaderValue::from_str(&challenge) {
		response
			.headers_mut()
			.insert(header::WWW_AUTHENTICATE, challenge);
	}
	response
}

/// An error as JSON, with the status its code calls for.
fn json_error(error: &IdpError) -> Response {
	let status = match error {
		IdpError::OAuth { code, .. } if *code == "invalid_client" => StatusCode::UNAUTHORIZED,
		IdpError::OAuth { code, .. } if *code == "insufficient_scope" => StatusCode::FORBIDDEN,
		IdpError::OAuth { .. } => StatusCode::BAD_REQUEST,
		IdpError::SignedOut => StatusCode::UNAUTHORIZED,
		IdpError::NotFound => StatusCode::NOT_FOUND,
		_ => StatusCode::INTERNAL_SERVER_ERROR,
	};
	no_store(
		(
			status,
			Json(OAuthError {
				error: error.code().to_owned(),
				error_description: Some(error.description()),
			}),
		)
			.into_response(),
	)
}

/// Tokens and the pages that lead to them are never cached.
fn no_store(mut response: Response) -> Response {
	let headers = response.headers_mut();
	headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
	headers.insert(header::PRAGMA, HeaderValue::from_static("no-cache"));
	response
}

/// A page for a sign-in link too broken to send back to the app.
fn error_page(error: &IdpError) -> Response {
	let description = error
		.description()
		.replace('&', "&amp;")
		.replace('<', "&lt;")
		.replace('>', "&gt;")
		.replace('"', "&quot;");
	no_store(
		(
			StatusCode::BAD_REQUEST,
			Html(format!(
				"<!doctype html><meta charset=utf-8><title>This sign-in link doesn't \
				 work</title><h1>This sign-in link doesn't work</h1><p>{description}</p>"
			)),
		)
			.into_response(),
	)
}

fn redirect(to: &str) -> Response {
	match HeaderValue::from_str(to) {
		Ok(location) => {
			no_store((StatusCode::SEE_OTHER, [(header::LOCATION, location)]).into_response())
		}
		Err(_) => {
			error_page(&IdpError::invalid_request(
				"the redirect isn't a valid address",
			))
		}
	}
}

/// The signed-in person, as long as they came with their own session: a
/// third-party token never reaches these routes through the layer, and
/// this makes sure of it.
fn person(current: Option<&CurrentSession>) -> Option<&SignedIn> {
	let current = current?;
	if current.delegation.is_some() {
		return None;
	}
	current.signed_in.as_ref()
}

async fn authorize(
	State(idp): State<Idp>,
	current: Option<Extension<CurrentSession>>,
	query: Result<Query<AuthorizeRequest>, axum::extract::rejection::QueryRejection>,
) -> Response {
	let Ok(Query(request)) = query else {
		return error_page(&IdpError::invalid_request(
			"the sign-in link's parameters are malformed or repeated",
		));
	};
	match idp.authorize(&request, person(current.as_deref())).await {
		Ok(AuthorizeOutcome::Redirect(to)) => redirect(&to),
		Ok(AuthorizeOutcome::Invalid(error)) => error_page(&error),
		Err(error) => json_error(&error),
	}
}

#[derive(Deserialize)]
struct ConsentQuery {
	request: String,
}

async fn consent(
	State(idp): State<Idp>,
	current: Option<Extension<CurrentSession>>,
	Query(query): Query<ConsentQuery>,
) -> Response {
	let Some(me) = person(current.as_deref()) else {
		return json_error(&IdpError::SignedOut);
	};
	match idp.consent_request(me, &query.request).await {
		Ok(request) => no_store(Json(request).into_response()),
		Err(error) => json_error(&error),
	}
}

async fn decide(
	State(idp): State<Idp>,
	current: Option<Extension<CurrentSession>>,
	Json(decision): Json<ConsentDecision>,
) -> Response {
	let Some(me) = person(current.as_deref()) else {
		return json_error(&IdpError::SignedOut);
	};
	match idp.decide(me, &decision.request, decision.approve).await {
		Ok(redirect_to) => no_store(Json(ConsentRedirect { redirect_to }).into_response()),
		Err(error) => json_error(&error),
	}
}

async fn end_session(
	State(idp): State<Idp>,
	current: Option<Extension<CurrentSession>>,
	request_info: RequestInfo,
	query: Result<Query<EndSessionRequest>, axum::extract::rejection::QueryRejection>,
) -> Response {
	let Ok(Query(request)) = query else {
		return error_page(&IdpError::invalid_request("the sign-out link is malformed"));
	};
	match idp
		.end_session(&request, person(current.as_deref()), &request_info)
		.await
	{
		Ok(ended) => {
			let mut response = redirect(&ended.redirect_to);
			if ended.signed_out {
				response
					.headers_mut()
					.insert(header::SET_COOKIE, idp.inner.auth.cleared_cookie());
			}
			response
		}
		Err(error @ IdpError::OAuth { .. }) => error_page(&error),
		Err(error) => json_error(&error),
	}
}

async fn grants(State(idp): State<Idp>, current: Option<Extension<CurrentSession>>) -> Response {
	let Some(me) = person(current.as_deref()) else {
		return json_error(&IdpError::SignedOut);
	};
	match idp.connected_apps(me).await {
		Ok(apps) => Json(apps).into_response(),
		Err(error) => json_error(&error),
	}
}

async fn disconnect(
	State(idp): State<Idp>,
	current: Option<Extension<CurrentSession>>,
	Json(body): Json<DisconnectApp>,
) -> Response {
	let Some(me) = person(current.as_deref()) else {
		return json_error(&IdpError::SignedOut);
	};
	match idp.disconnect(me, &body.client_id).await {
		Ok(true) => StatusCode::NO_CONTENT.into_response(),
		Ok(false) => StatusCode::NOT_FOUND.into_response(),
		Err(error) => json_error(&error),
	}
}

async fn discovery(State(idp): State<Idp>) -> Response {
	Json(idp.discovery()).into_response()
}

async fn jwks(State(idp): State<Idp>) -> Response {
	match idp.jwks().await {
		Ok(keys) => Json(keys).into_response(),
		Err(error) => json_error(&error),
	}
}

/// Client credentials from the header and the form.
fn credentials(
	headers: &HeaderMap,
	client_id: Option<String>,
	client_secret: Option<String>,
) -> Result<ClientCredentials, IdpError> {
	let authorization = headers
		.get(header::AUTHORIZATION)
		.and_then(|value| value.to_str().ok());
	ClientCredentials::new(authorization, client_id, client_secret)
}

/// An error from an endpoint apps authenticate to: `invalid_client` after
/// Basic authentication challenges it (RFC 6749 §5.2).
fn client_error(error: &IdpError, basic: bool) -> Response {
	let mut response = json_error(error);
	if basic && error.code() == "invalid_client" {
		response.headers_mut().insert(
			header::WWW_AUTHENTICATE,
			HeaderValue::from_static("Basic realm=\"oauth\""),
		);
	}
	response
}

fn malformed_form() -> Response {
	json_error(&IdpError::invalid_request(
		"send a form (application/x-www-form-urlencoded) with each parameter once",
	))
}

async fn token(
	State(idp): State<Idp>,
	headers: HeaderMap,
	form: Result<Form<TokenRequest>, FormRejection>,
) -> Response {
	let Ok(Form(request)) = form else {
		return malformed_form();
	};
	let credentials = match credentials(
		&headers,
		request.client_id.clone(),
		request.client_secret.clone(),
	) {
		Ok(credentials) => credentials,
		Err(error) => return client_error(&error, headers.contains_key(header::AUTHORIZATION)),
	};
	match idp.token(&request, &credentials).await {
		Ok(tokens) => no_store(Json(tokens).into_response()),
		Err(error) => client_error(&error, credentials.used_basic()),
	}
}

async fn userinfo(State(idp): State<Idp>, headers: HeaderMap) -> Response {
	let Some(token) = bearer(&headers) else {
		return bearer_error(StatusCode::UNAUTHORIZED, "invalid_token", None);
	};
	let access = match idp.verify_access_token(&token).await {
		Ok(Some(access)) => access,
		Ok(None) => return bearer_error(StatusCode::UNAUTHORIZED, "invalid_token", None),
		Err(error) => return json_error(&error),
	};
	match idp.userinfo(&access).await {
		Ok(claims) => no_store(Json(claims).into_response()),
		Err(error) if error.code() == "insufficient_scope" => {
			bearer_error(StatusCode::FORBIDDEN, "insufficient_scope", Some("openid"))
		}
		Err(error) => json_error(&error),
	}
}

/// The form of `POST /oauth/revoke` and `POST /oauth/introspect`.
#[derive(Deserialize)]
struct TokenForm {
	#[serde(default)]
	token: Option<String>,
	/// Accepted and ignored: both endpoints try every kind of token.
	#[serde(default)]
	#[allow(dead_code)]
	token_type_hint: Option<String>,
	#[serde(default)]
	client_id: Option<String>,
	#[serde(default)]
	client_secret: Option<String>,
}

async fn revoke(
	State(idp): State<Idp>,
	headers: HeaderMap,
	form: Result<Form<TokenForm>, FormRejection>,
) -> Response {
	let Ok(Form(form)) = form else {
		return malformed_form();
	};
	let credentials = match credentials(&headers, form.client_id, form.client_secret) {
		Ok(credentials) => credentials,
		Err(error) => return client_error(&error, headers.contains_key(header::AUTHORIZATION)),
	};
	let Some(token) = form.token.filter(|token| !token.is_empty()) else {
		return json_error(&IdpError::invalid_request("token is missing"));
	};
	match idp.revoke_token(&credentials, &token).await {
		Ok(()) => no_store(StatusCode::OK.into_response()),
		Err(error) => client_error(&error, credentials.used_basic()),
	}
}

async fn introspect(
	State(idp): State<Idp>,
	headers: HeaderMap,
	form: Result<Form<TokenForm>, FormRejection>,
) -> Response {
	let Ok(Form(form)) = form else {
		return malformed_form();
	};
	let credentials = match credentials(&headers, form.client_id, form.client_secret) {
		Ok(credentials) => credentials,
		Err(error) => return client_error(&error, headers.contains_key(header::AUTHORIZATION)),
	};
	let Some(token) = form.token.filter(|token| !token.is_empty()) else {
		return json_error(&IdpError::invalid_request("token is missing"));
	};
	match idp.introspect(&credentials, &token).await {
		Ok(introspection) => no_store(Json(introspection).into_response()),
		Err(error) => client_error(&error, credentials.used_basic()),
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn only_jwt_shaped_bearer_tokens_are_the_providers() {
		assert!(looks_like_jwt("a.b.c"));
		assert!(!looks_like_jwt("Zm9vYmFyYmF6"));
		assert!(!looks_like_jwt("a.b"));

		let mut headers = HeaderMap::new();
		headers.insert(
			header::AUTHORIZATION,
			HeaderValue::from_static("bearer a.b.c"),
		);
		assert_eq!(bearer(&headers).as_deref(), Some("a.b.c"));
		headers.insert(header::AUTHORIZATION, HeaderValue::from_static("Basic abc"));
		assert_eq!(bearer(&headers), None);
	}

	#[test]
	fn errors_map_to_their_status() {
		let status = |error: IdpError| json_error(&error).status();
		assert_eq!(status(IdpError::invalid_client()), StatusCode::UNAUTHORIZED);
		assert_eq!(
			status(IdpError::invalid_grant("x")),
			StatusCode::BAD_REQUEST
		);
		assert_eq!(status(IdpError::NotFound), StatusCode::NOT_FOUND);
		assert_eq!(
			status(IdpError::Config("x".into())),
			StatusCode::INTERNAL_SERVER_ERROR
		);
		assert_eq!(
			client_error(&IdpError::invalid_client(), true).headers()[header::WWW_AUTHENTICATE],
			"Basic realm=\"oauth\""
		);
		assert_eq!(
			error_page(&IdpError::invalid_request("<b>")).status(),
			StatusCode::BAD_REQUEST
		);
	}
}
