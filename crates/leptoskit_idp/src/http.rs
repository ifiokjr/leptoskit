//! Sign in with your app over HTTP, for axum servers: the `/oauth` routes,
//! and the layer that serves the token endpoints and lets third-party
//! tokens reach the functions that accept them.
//!
//! ```ignore
//! let app = Router::new()
//! 	.merge(my_app_backend::router())
//! 	.merge(auth.router())     // /auth/*
//! 	.merge(idp.router())      // /oauth/authorize, consent, devices, connected apps, sign-out
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
use axum::extract::rejection::JsonRejection;
use axum::extract::rejection::QueryRejection;
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
use serde::Serialize;
use tower::ServiceExt;

use crate::AccessToken;
use crate::AppChanges;
use crate::AppRef;
use crate::AuthorizeOutcome;
use crate::AuthorizeRequest;
use crate::ClientCredentials;
use crate::ClientMetadata;
use crate::ConsentDecision;
use crate::ConsentRedirect;
use crate::DeviceDecision;
use crate::DisconnectApp;
use crate::EndSessionRequest;
use crate::Idp;
use crate::IdpError;
use crate::NewApp;
use crate::OAuthError;
use crate::TokenRequest;
use crate::dpop::Expected;

/// The routes the layer serves itself, outside the session layer: they
/// read no cookie, so apps on other sites may call them.
const ENDPOINTS: [&str; 8] = [
	"/.well-known/openid-configuration",
	"/oauth/jwks",
	"/oauth/token",
	"/oauth/userinfo",
	"/oauth/revoke",
	"/oauth/introspect",
	"/oauth/par",
	"/oauth/device_authorization",
];

/// The browser routes an app's page may also `POST` a form to. The layer
/// answers those posts by sending the browser to the same route with a
/// `GET`, which carries the session cookie and passes the same-site check.
const FORM_POSTS: [&str; 2] = ["/oauth/authorize", "/oauth/end_session"];

/// The largest form body the layer reads from a cross-site post.
const MAX_FORM: usize = 64 * 1024;

/// Where an endpoint's request came from, which the layer works out before
/// the session layer would.
#[derive(Clone, Debug)]
struct Source(RequestInfo);

impl Idp {
	/// The routes people's browsers visit, which need their session:
	///
	/// | Route                         | Does                                                     |
	/// | ----------------------------- | -------------------------------------------------------- |
	/// | `GET /oauth/authorize`        | starts a sign-in ([`AuthorizeRequest`])                  |
	/// | `GET /oauth/consent`          | what the consent page shows ([`ConsentRequest`](crate::ConsentRequest)) |
	/// | `POST /oauth/consent`         | answers it ([`ConsentDecision`])                         |
	/// | `GET /oauth/device`           | what the device page shows ([`DeviceRequest`](crate::DeviceRequest)) |
	/// | `POST /oauth/device`          | answers it ([`DeviceDecision`])                          |
	/// | `GET /oauth/end_session`      | an app signing someone out ([`EndSessionRequest`])       |
	/// | `GET /oauth/grants`           | the person's connected apps ([`ConnectedApp`](crate::ConnectedApp)) |
	/// | `POST /oauth/grants/revoke`   | disconnects one ([`DisconnectApp`])                      |
	///
	/// With the [developer portal](crate::IdpBuilder::developer_portal) on:
	///
	/// | Route                              | Does                                         |
	/// | ---------------------------------- | -------------------------------------------- |
	/// | `GET /oauth/apps`                  | the person's own apps ([`ClientInfo`](crate::ClientInfo)) |
	/// | `POST /oauth/apps`                 | registers one ([`NewApp`])                   |
	/// | `POST /oauth/apps/update`          | changes one ([`AppChanges`])                 |
	/// | `POST /oauth/apps/rotate-secret`   | replaces its secret ([`AppRef`])             |
	/// | `POST /oauth/apps/delete`          | deletes it ([`AppRef`])                      |
	///
	/// Merge them inside [`Auth::layer`](leptoskit_auth::Auth::layer), like
	/// `auth.router()`. The token endpoints come from [`Idp::layer`].
	pub fn router<S>(&self) -> Router<S>
	where
		S: Clone + Send + Sync + 'static,
	{
		let mut router = Router::new()
			.route("/oauth/authorize", get(authorize))
			.route("/oauth/consent", get(consent).post(decide))
			.route("/oauth/device", get(device).post(decide_device))
			.route("/oauth/end_session", get(end_session))
			.route("/oauth/grants", get(grants))
			.route("/oauth/grants/revoke", post(disconnect));

		if self.has_developer_portal() {
			router = router
				.route("/oauth/apps", get(my_apps).post(register_app))
				.route("/oauth/apps/update", post(update_app))
				.route("/oauth/apps/rotate-secret", post(rotate_app_secret))
				.route("/oauth/apps/delete", post(delete_app));
		}

		router.with_state(self.clone())
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
	/// | `POST /oauth/par`                       | keeps a pushed request (RFC 9126)     |
	/// | `POST /oauth/device_authorization`      | starts the device flow (RFC 8628)     |
	///
	/// With [dynamic registration](crate::IdpBuilder::dynamic_registration)
	/// on, it also serves `POST /oauth/register` (RFC 7591), and `GET`, `PUT`
	/// and `DELETE /oauth/register/{client_id}` (RFC 7592).
	///
	/// With [DPoP](crate::IdpBuilder::dpop) on, the token endpoint binds
	/// tokens to the key of a proof in the `DPoP` header, and a bound token
	/// works only as `Authorization: DPoP`, with a proof signed by that key.
	///
	/// These answer cross-origin requests, since they read no cookies. The
	/// layer also takes form `POST`s to `/oauth/authorize` and
	/// `/oauth/end_session`, as OpenID Connect allows, and sends the browser
	/// on to the `GET` route with the same parameters.
	///
	/// A request carrying one of this provider's access tokens reaches only
	/// an `#[api(scope = …)]` function whose every scope the token carries.
	/// It then runs as the person who consented, or as no one for an app
	/// acting as itself: the layer hands
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
				.route("/oauth/par", post(par))
				.route("/oauth/device_authorization", post(device_authorization))
				.route("/oauth/authorize", post(form_post::<AuthorizeRequest>))
				.route("/oauth/end_session", post(form_post::<EndSessionRequest>))
				.route("/oauth/register", post(register))
				.route(
					"/oauth/register/{client_id}",
					get(read_registration)
						.put(update_registration)
						.delete(delete_registration),
				)
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

/// Whether the layer answers this request itself. `registration` says
/// whether apps may register themselves, so the routes exist.
fn served_here(method: &Method, path: &str, registration: bool) -> bool {
	ENDPOINTS.contains(&path)
		|| (method == Method::POST && FORM_POSTS.contains(&path))
		|| (registration && (path == "/oauth/register" || path.starts_with("/oauth/register/")))
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
			if served_here(
				request.method(),
				request.uri().path(),
				idp.has_dynamic_registration(),
			) {
				let preflight = request.method() == Method::OPTIONS;
				let mut response = if preflight {
					StatusCode::NO_CONTENT.into_response()
				} else {
					let source = Source(idp.inner.auth.request_info(&request));
					request.extensions_mut().insert(source);
					let Ok(response) = endpoints
						.oneshot(request)
						.await
						.map_err(|never: Infallible| never);
					response
				};
				allow_any_origin(response.headers_mut(), preflight);
				return Ok(response);
			}

			// The headless client reads the provider in process while the
			// server renders a page.
			request.extensions_mut().insert(idp.clone());

			let Some(presented) = presented(request.headers(), idp.takes_dpop())
				.filter(|token| token.scheme == Scheme::Dpop || looks_like_jwt(&token.token))
			else {
				return inner.call(request).await;
			};
			let scheme = presented.scheme;
			let path = request.uri().path().to_owned();
			let access =
				match check_access(&idp, request.method(), &path, request.headers(), presented)
					.await
				{
					Ok(access) => access,
					Err(refused) => return Ok(*refused),
				};
			let required =
				leptoskit_rpc::spec::scopes_for(request.method(), &path).unwrap_or_default();

			if required.is_empty() {
				// Only functions that name a scope take another app's token.
				return Ok(challenge(
					scheme,
					StatusCode::FORBIDDEN,
					"insufficient_scope",
					None,
				));
			}
			if !required
				.iter()
				.all(|scope| access.scopes.iter().any(|granted| granted == scope))
			{
				return Ok(challenge(
					scheme,
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
	presented(headers, false).map(|presented| presented.token)
}

/// How a request presents an access token.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Scheme {
	Bearer,
	/// `Authorization: DPoP`, with a proof in the `DPoP` header (RFC 9449
	/// §7.1).
	Dpop,
}

/// An access token a request presents.
struct Presented {
	scheme: Scheme,
	token: String,
}

/// The token in `Authorization`, as `Bearer`, or as `DPoP` when `dpop` is
/// on.
fn presented(headers: &HeaderMap, dpop: bool) -> Option<Presented> {
	let value = headers.get(header::AUTHORIZATION)?.to_str().ok()?;
	let (scheme, token) = value.split_once(' ')?;
	let scheme = if scheme.eq_ignore_ascii_case("bearer") {
		Scheme::Bearer
	} else if dpop && scheme.eq_ignore_ascii_case("dpop") {
		Scheme::Dpop
	} else {
		return None;
	};
	Some(Presented {
		scheme,
		token: token.trim().to_owned(),
	})
}

/// The request's one `DPoP` header, if it has one.
fn dpop_proof(headers: &HeaderMap) -> Result<Option<&str>, IdpError> {
	let mut proofs = headers.get_all("dpop").iter();
	let Some(proof) = proofs.next() else {
		return Ok(None);
	};
	if proofs.next().is_some() {
		return Err(crate::dpop::invalid("send one `DPoP` header"));
	}
	proof
		.to_str()
		.map(Some)
		.map_err(|_| crate::dpop::invalid("it must be a compact JWT"))
}

/// Checks the access token a request to `path` presents: its signature and
/// family, and for a token bound to a key, that it came as `DPoP` with a
/// proof that key signed for this request. A bound token sent as `Bearer`,
/// or a bearer token sent as `DPoP`, is refused.
async fn check_access(
	idp: &Idp,
	method: &Method,
	path: &str,
	headers: &HeaderMap,
	presented: Presented,
) -> Result<AccessToken, Box<Response>> {
	let scheme = presented.scheme;
	let refuse = |code: &str| challenge(scheme, StatusCode::UNAUTHORIZED, code, None);
	let access = match idp.verify_access_token(&presented.token).await {
		Ok(Some(access)) => access,
		Ok(None) => return Err(Box::new(refuse("invalid_token"))),
		Err(error) => return Err(Box::new(json_error(&error))),
	};

	match (scheme, &access.jkt) {
		(Scheme::Bearer, None) => Ok(access),
		(Scheme::Bearer, Some(_)) | (Scheme::Dpop, None) => Err(Box::new(refuse("invalid_token"))),
		(Scheme::Dpop, Some(bound)) => {
			let Ok(Some(proof)) = dpop_proof(headers) else {
				return Err(Box::new(refuse("invalid_dpop_proof")));
			};
			let url = idp.url_of(path);
			let expected = Expected {
				method: method.as_str(),
				url: &url,
				access_token: Some(&presented.token),
			};
			match idp.check_proof(proof, &expected).await {
				Ok(thumbprint) if thumbprint == *bound => Ok(access),
				Ok(_) | Err(IdpError::OAuth { .. }) => Err(Box::new(refuse("invalid_dpop_proof"))),
				Err(error) => Err(Box::new(json_error(&error))),
			}
		}
	}
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
			HeaderValue::from_static("GET, POST, PUT, DELETE"),
		);
		headers.insert(
			header::ACCESS_CONTROL_ALLOW_HEADERS,
			HeaderValue::from_static("authorization, content-type, dpop"),
		);
		headers.insert(
			header::ACCESS_CONTROL_MAX_AGE,
			HeaderValue::from_static("600"),
		);
	}
}

/// A `WWW-Authenticate: Bearer` challenge (RFC 6750 §3).
fn bearer_error(status: StatusCode, code: &str, scope: Option<&str>) -> Response {
	challenge(Scheme::Bearer, status, code, scope)
}

/// A `WWW-Authenticate` challenge for the scheme the token came with: a
/// `DPoP` one also lists the algorithms proofs may use (RFC 9449 §7.1).
fn challenge(scheme: Scheme, status: StatusCode, code: &str, scope: Option<&str>) -> Response {
	let mut challenge = match scheme {
		Scheme::Bearer => format!("Bearer error=\"{code}\""),
		Scheme::Dpop => {
			format!(
				"DPoP error=\"{code}\", algs=\"{}\"",
				crate::dpop::ALGORITHMS.join(" ")
			)
		}
	};
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
		IdpError::OAuth { code, .. } if *code == "invalid_token" => StatusCode::UNAUTHORIZED,
		IdpError::OAuth { .. } => StatusCode::BAD_REQUEST,
		IdpError::SignedOut => StatusCode::UNAUTHORIZED,
		IdpError::NotFound => StatusCode::NOT_FOUND,
		_ if error.retry_after().is_some() => StatusCode::TOO_MANY_REQUESTS,
		_ => StatusCode::INTERNAL_SERVER_ERROR,
	};
	let mut response = no_store(
		(
			status,
			Json(OAuthError {
				error: error.code().to_owned(),
				error_description: Some(error.description()),
			}),
		)
			.into_response(),
	);
	if let Some(retry_after) = error.retry_after() {
		response.headers_mut().insert(
			header::RETRY_AFTER,
			HeaderValue::from(retry_after.as_secs().max(1)),
		);
	}
	response
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

/// Answers a form `POST` to a browser route by sending the browser to the
/// `GET` route with the same parameters. A cross-site post carries no
/// `SameSite=Lax` cookie; the `GET` it's sent on to does.
async fn form_post<T>(request: Request<Body>) -> Response
where
	T: Serialize + serde::de::DeserializeOwned + Send,
{
	let path = request.uri().path().to_owned();
	let Ok(body) = axum::body::to_bytes(request.into_body(), MAX_FORM).await else {
		return error_page(&IdpError::invalid_request("the form is too large"));
	};
	let Ok(form) = serde_urlencoded::from_bytes::<T>(&body) else {
		return error_page(&IdpError::invalid_request(
			"the form's parameters are malformed or repeated",
		));
	};
	let Ok(query) = serde_urlencoded::to_string(&form) else {
		return error_page(&IdpError::invalid_request("the form can't be read"));
	};
	redirect(&format!("{path}?{query}"))
}

async fn authorize(
	State(idp): State<Idp>,
	current: Option<Extension<CurrentSession>>,
	info: RequestInfo,
	query: Result<Query<AuthorizeRequest>, QueryRejection>,
) -> Response {
	let Ok(Query(request)) = query else {
		return error_page(&IdpError::invalid_request(
			"the sign-in link's parameters are malformed or repeated",
		));
	};
	match idp
		.authorize(&request, person(current.as_deref()), &info)
		.await
	{
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
	query: Result<Query<ConsentQuery>, QueryRejection>,
) -> Response {
	let Some(me) = person(current.as_deref()) else {
		return json_error(&IdpError::SignedOut);
	};
	let Ok(Query(query)) = query else {
		return json_error(&IdpError::invalid_request("request is missing"));
	};
	match idp.consent_request(me, &query.request).await {
		Ok(request) => no_store(Json(request).into_response()),
		Err(error) => json_error(&error),
	}
}

async fn decide(
	State(idp): State<Idp>,
	current: Option<Extension<CurrentSession>>,
	info: RequestInfo,
	decision: Result<Json<ConsentDecision>, JsonRejection>,
) -> Response {
	let Some(me) = person(current.as_deref()) else {
		return json_error(&IdpError::SignedOut);
	};
	let Ok(Json(decision)) = decision else {
		return malformed_json();
	};
	match idp
		.decide(me, &decision.request, decision.approve, &info)
		.await
	{
		Ok(redirect_to) => no_store(Json(ConsentRedirect { redirect_to }).into_response()),
		Err(error) => json_error(&error),
	}
}

#[derive(Deserialize)]
struct DeviceQuery {
	user_code: String,
}

async fn device(
	State(idp): State<Idp>,
	current: Option<Extension<CurrentSession>>,
	query: Result<Query<DeviceQuery>, QueryRejection>,
) -> Response {
	let Some(me) = person(current.as_deref()) else {
		return json_error(&IdpError::SignedOut);
	};
	let Ok(Query(query)) = query else {
		return json_error(&IdpError::invalid_request("user_code is missing"));
	};
	match idp.device_request(me, &query.user_code).await {
		Ok(request) => no_store(Json(request).into_response()),
		Err(error) => json_error(&error),
	}
}

async fn decide_device(
	State(idp): State<Idp>,
	current: Option<Extension<CurrentSession>>,
	info: RequestInfo,
	decision: Result<Json<DeviceDecision>, JsonRejection>,
) -> Response {
	let Some(me) = person(current.as_deref()) else {
		return json_error(&IdpError::SignedOut);
	};
	let Ok(Json(decision)) = decision else {
		return malformed_json();
	};
	match idp
		.decide_device(me, &decision.user_code, decision.approve, &info)
		.await
	{
		Ok(()) => StatusCode::NO_CONTENT.into_response(),
		Err(error) => json_error(&error),
	}
}

async fn end_session(
	State(idp): State<Idp>,
	current: Option<Extension<CurrentSession>>,
	request_info: RequestInfo,
	query: Result<Query<EndSessionRequest>, QueryRejection>,
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
		Ok(apps) => no_store(Json(apps).into_response()),
		Err(error) => json_error(&error),
	}
}

async fn disconnect(
	State(idp): State<Idp>,
	current: Option<Extension<CurrentSession>>,
	info: RequestInfo,
	body: Result<Json<DisconnectApp>, JsonRejection>,
) -> Response {
	let Some(me) = person(current.as_deref()) else {
		return json_error(&IdpError::SignedOut);
	};
	let Ok(Json(body)) = body else {
		return malformed_json();
	};
	match idp.disconnect(me, &body.client_id, &info).await {
		Ok(true) => StatusCode::NO_CONTENT.into_response(),
		Ok(false) => StatusCode::NOT_FOUND.into_response(),
		Err(error) => json_error(&error),
	}
}

async fn my_apps(State(idp): State<Idp>, current: Option<Extension<CurrentSession>>) -> Response {
	let Some(me) = person(current.as_deref()) else {
		return json_error(&IdpError::SignedOut);
	};
	match idp.my_apps(me).await {
		Ok(apps) => no_store(Json(apps).into_response()),
		Err(error) => json_error(&error),
	}
}

async fn register_app(
	State(idp): State<Idp>,
	current: Option<Extension<CurrentSession>>,
	info: RequestInfo,
	body: Result<Json<NewApp>, JsonRejection>,
) -> Response {
	let Some(me) = person(current.as_deref()) else {
		return json_error(&IdpError::SignedOut);
	};
	let Ok(Json(app)) = body else {
		return malformed_json();
	};
	match idp.register_app(me, app, &info).await {
		Ok(registered) => no_store((StatusCode::CREATED, Json(registered)).into_response()),
		Err(error) => json_error(&error),
	}
}

async fn update_app(
	State(idp): State<Idp>,
	current: Option<Extension<CurrentSession>>,
	info: RequestInfo,
	body: Result<Json<AppChanges>, JsonRejection>,
) -> Response {
	let Some(me) = person(current.as_deref()) else {
		return json_error(&IdpError::SignedOut);
	};
	let Ok(Json(changes)) = body else {
		return malformed_json();
	};
	match idp.update_app(me, changes, &info).await {
		Ok(app) => no_store(Json(app).into_response()),
		Err(error) => json_error(&error),
	}
}

async fn rotate_app_secret(
	State(idp): State<Idp>,
	current: Option<Extension<CurrentSession>>,
	info: RequestInfo,
	body: Result<Json<AppRef>, JsonRejection>,
) -> Response {
	let Some(me) = person(current.as_deref()) else {
		return json_error(&IdpError::SignedOut);
	};
	let Ok(Json(app)) = body else {
		return malformed_json();
	};
	match idp.rotate_app_secret(me, &app.client_id, &info).await {
		Ok(registered) => no_store(Json(registered).into_response()),
		Err(error) => json_error(&error),
	}
}

async fn delete_app(
	State(idp): State<Idp>,
	current: Option<Extension<CurrentSession>>,
	info: RequestInfo,
	body: Result<Json<AppRef>, JsonRejection>,
) -> Response {
	let Some(me) = person(current.as_deref()) else {
		return json_error(&IdpError::SignedOut);
	};
	let Ok(Json(app)) = body else {
		return malformed_json();
	};
	match idp.delete_app(me, &app.client_id, &info).await {
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

/// How a form says which app it comes from, besides `Authorization`.
#[derive(Default, Deserialize)]
struct ClientForm {
	#[serde(default, rename = "client_id")]
	id: Option<String>,
	#[serde(default, rename = "client_secret")]
	secret: Option<String>,
	#[serde(default, rename = "client_assertion_type")]
	assertion_type: Option<String>,
	#[serde(default, rename = "client_assertion")]
	assertion: Option<String>,
}

/// Client credentials from the header and the form.
fn credentials(headers: &HeaderMap, form: ClientForm) -> Result<ClientCredentials, IdpError> {
	let authorization = headers
		.get(header::AUTHORIZATION)
		.and_then(|value| value.to_str().ok());
	Ok(ClientCredentials::new(authorization, form.id, form.secret)?
		.with_assertion(form.assertion_type, form.assertion))
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

fn malformed_json() -> Response {
	json_error(&IdpError::invalid_request(
		"send a JSON body (application/json) of the expected shape",
	))
}

/// Why the client's credentials can't be read, as an answer.
fn unreadable(headers: &HeaderMap, error: &IdpError) -> Response {
	client_error(error, headers.contains_key(header::AUTHORIZATION))
}

async fn token(
	State(idp): State<Idp>,
	Extension(Source(info)): Extension<Source>,
	headers: HeaderMap,
	form: Result<Form<TokenRequest>, FormRejection>,
) -> Response {
	let Ok(Form(mut request)) = form else {
		return malformed_form();
	};

	if idp.takes_dpop() {
		let proof = match dpop_proof(&headers) {
			Ok(proof) => proof,
			Err(error) => return json_error(&error),
		};
		if let Some(proof) = proof {
			let url = idp.url_of("/oauth/token");
			let expected = Expected {
				method: "POST",
				url: &url,
				access_token: None,
			};
			match idp.check_proof(proof, &expected).await {
				Ok(thumbprint) => request.dpop_jkt = Some(thumbprint),
				Err(error) => return json_error(&error),
			}
		}
	}

	let client = ClientForm {
		id: request.client_id.clone(),
		secret: request.client_secret.clone(),
		assertion_type: request.client_assertion_type.clone(),
		assertion: request.client_assertion.clone(),
	};
	let credentials = match credentials(&headers, client) {
		Ok(credentials) => credentials,
		Err(error) => return unreadable(&headers, &error),
	};
	match idp.token(&request, &credentials, &info).await {
		Ok(tokens) => no_store(Json(tokens).into_response()),
		Err(error) => client_error(&error, credentials.used_basic()),
	}
}

async fn userinfo(State(idp): State<Idp>, method: Method, headers: HeaderMap) -> Response {
	let Some(presented) = presented(&headers, idp.takes_dpop()) else {
		return bearer_error(StatusCode::UNAUTHORIZED, "invalid_token", None);
	};
	let scheme = presented.scheme;
	let access = match check_access(&idp, &method, "/oauth/userinfo", &headers, presented).await {
		Ok(access) => access,
		Err(refused) => return *refused,
	};
	match idp.userinfo(&access).await {
		Ok(claims) => no_store(Json(claims).into_response()),
		Err(error) if error.code() == "insufficient_scope" => {
			challenge(
				scheme,
				StatusCode::FORBIDDEN,
				"insufficient_scope",
				Some("openid"),
			)
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
	#[serde(flatten)]
	client: ClientForm,
}

async fn revoke(
	State(idp): State<Idp>,
	headers: HeaderMap,
	form: Result<Form<TokenForm>, FormRejection>,
) -> Response {
	let Ok(Form(form)) = form else {
		return malformed_form();
	};
	let credentials = match credentials(&headers, form.client) {
		Ok(credentials) => credentials,
		Err(error) => return unreadable(&headers, &error),
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
	let credentials = match credentials(&headers, form.client) {
		Ok(credentials) => credentials,
		Err(error) => return unreadable(&headers, &error),
	};
	let Some(token) = form.token.filter(|token| !token.is_empty()) else {
		return json_error(&IdpError::invalid_request("token is missing"));
	};
	match idp.introspect(&credentials, &token).await {
		Ok(introspection) => no_store(Json(introspection).into_response()),
		Err(error) => client_error(&error, credentials.used_basic()),
	}
}

/// The form of `POST /oauth/par`: an authorization request, and how the
/// app authenticates.
#[derive(Deserialize)]
struct PushForm {
	#[serde(flatten)]
	request: AuthorizeRequest,
	#[serde(default)]
	client_secret: Option<String>,
	#[serde(default)]
	client_assertion_type: Option<String>,
	#[serde(default)]
	client_assertion: Option<String>,
}

async fn par(
	State(idp): State<Idp>,
	headers: HeaderMap,
	form: Result<Form<PushForm>, FormRejection>,
) -> Response {
	let Ok(Form(form)) = form else {
		return malformed_form();
	};
	let client = ClientForm {
		id: form.request.client_id.clone(),
		secret: form.client_secret,
		assertion_type: form.client_assertion_type,
		assertion: form.client_assertion,
	};
	let credentials = match credentials(&headers, client) {
		Ok(credentials) => credentials,
		Err(error) => return unreadable(&headers, &error),
	};
	match idp.push_request(&credentials, &form.request).await {
		Ok(pushed) => no_store((StatusCode::CREATED, Json(pushed)).into_response()),
		Err(error) => client_error(&error, credentials.used_basic()),
	}
}

/// The form of `POST /oauth/device_authorization`.
#[derive(Deserialize)]
struct DeviceForm {
	#[serde(default)]
	scope: Option<String>,
	#[serde(flatten)]
	client: ClientForm,
}

async fn device_authorization(
	State(idp): State<Idp>,
	headers: HeaderMap,
	form: Result<Form<DeviceForm>, FormRejection>,
) -> Response {
	let Ok(Form(form)) = form else {
		return malformed_form();
	};
	let credentials = match credentials(&headers, form.client) {
		Ok(credentials) => credentials,
		Err(error) => return unreadable(&headers, &error),
	};
	match idp
		.authorize_device(&credentials, form.scope.as_deref())
		.await
	{
		Ok(device) => no_store(Json(device).into_response()),
		Err(error) => client_error(&error, credentials.used_basic()),
	}
}

/// The registration metadata in a JSON body, or why it can't be read.
fn client_metadata(
	body: Result<Json<ClientMetadata>, JsonRejection>,
) -> Result<ClientMetadata, IdpError> {
	body.map(|Json(metadata)| metadata).map_err(|_| {
		IdpError::oauth(
			"invalid_client_metadata",
			"send the app's metadata as a JSON object (application/json)",
		)
	})
}

/// A registration answer, or its error with the challenge an
/// `invalid_token` calls for.
fn registration_response(
	result: Result<crate::ClientRegistration, IdpError>,
	status: StatusCode,
) -> Response {
	match result {
		Ok(registration) => no_store((status, Json(registration)).into_response()),
		Err(error) => registration_error(&error),
	}
}

/// A registration error, with `WWW-Authenticate` for a missing or wrong
/// token.
fn registration_error(error: &IdpError) -> Response {
	let mut response = json_error(error);
	if error.code() == "invalid_token" {
		response.headers_mut().insert(
			header::WWW_AUTHENTICATE,
			HeaderValue::from_static("Bearer error=\"invalid_token\""),
		);
	}
	response
}

async fn register(
	State(idp): State<Idp>,
	Extension(Source(info)): Extension<Source>,
	headers: HeaderMap,
	body: Result<Json<ClientMetadata>, JsonRejection>,
) -> Response {
	let metadata = match client_metadata(body) {
		Ok(metadata) => metadata,
		Err(error) => return json_error(&error),
	};
	let token = bearer(&headers);
	registration_response(
		idp.register_dynamically(token.as_deref(), metadata, &info)
			.await,
		StatusCode::CREATED,
	)
}

async fn read_registration(
	State(idp): State<Idp>,
	axum::extract::Path(client_id): axum::extract::Path<String>,
	headers: HeaderMap,
) -> Response {
	let token = bearer(&headers);
	registration_response(
		idp.read_registration(&client_id, token.as_deref()).await,
		StatusCode::OK,
	)
}

async fn update_registration(
	State(idp): State<Idp>,
	axum::extract::Path(client_id): axum::extract::Path<String>,
	headers: HeaderMap,
	body: Result<Json<ClientMetadata>, JsonRejection>,
) -> Response {
	let metadata = match client_metadata(body) {
		Ok(metadata) => metadata,
		Err(error) => return json_error(&error),
	};
	let token = bearer(&headers);
	registration_response(
		idp.update_registration(&client_id, token.as_deref(), metadata)
			.await,
		StatusCode::OK,
	)
}

async fn delete_registration(
	State(idp): State<Idp>,
	Extension(Source(info)): Extension<Source>,
	axum::extract::Path(client_id): axum::extract::Path<String>,
	headers: HeaderMap,
) -> Response {
	let token = bearer(&headers);
	match idp
		.delete_registration(&client_id, token.as_deref(), &info)
		.await
	{
		Ok(()) => no_store(StatusCode::NO_CONTENT.into_response()),
		Err(error) => registration_error(&error),
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

		headers.insert(
			header::AUTHORIZATION,
			HeaderValue::from_static("DPoP a.b.c"),
		);
		assert!(presented(&headers, false).is_none());
		assert_eq!(
			presented(&headers, true).map(|presented| presented.scheme),
			Some(Scheme::Dpop)
		);

		assert_eq!(dpop_proof(&headers).unwrap(), None);
		headers.append("dpop", HeaderValue::from_static("a.b.c"));
		assert_eq!(dpop_proof(&headers).unwrap(), Some("a.b.c"));
		headers.append("dpop", HeaderValue::from_static("d.e.f"));
		assert_eq!(
			dpop_proof(&headers).unwrap_err().code(),
			"invalid_dpop_proof"
		);
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
		let limited = json_error(&IdpError::Auth(leptoskit_auth::AuthError::RateLimited {
			retry_after: std::time::Duration::from_secs(9),
		}));
		assert_eq!(limited.status(), StatusCode::TOO_MANY_REQUESTS);
		assert_eq!(limited.headers()[header::RETRY_AFTER], "9");
		assert_eq!(
			client_error(&IdpError::invalid_client(), true).headers()[header::WWW_AUTHENTICATE],
			"Basic realm=\"oauth\""
		);
		assert_eq!(
			error_page(&IdpError::invalid_request("<b>")).status(),
			StatusCode::BAD_REQUEST
		);
	}

	#[test]
	fn the_layer_serves_endpoints_and_form_posts() {
		assert!(served_here(&Method::POST, "/oauth/token", false));
		assert!(served_here(&Method::GET, "/oauth/jwks", false));
		assert!(served_here(&Method::POST, "/oauth/authorize", false));
		assert!(!served_here(&Method::GET, "/oauth/authorize", false));
		assert!(!served_here(&Method::GET, "/oauth/grants", false));
		assert!(!served_here(&Method::POST, "/oauth/register", false));
		assert!(served_here(&Method::POST, "/oauth/register", true));
		assert!(served_here(&Method::DELETE, "/oauth/register/app", true));
	}
}
