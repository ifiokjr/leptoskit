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
use axum::extract::Path;
use axum::extract::Query;
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

use crate::AddPasskey;
use crate::Auth;
use crate::AuthError;
use crate::ContinueProvider;
use crate::Device;
use crate::ErrorResponse;
use crate::FactorCode;
use crate::FinishProvider;
use crate::IdentityInfo;
use crate::LinkWallet;
use crate::MergeAccounts;
use crate::MergeOffer;
use crate::MfaStatus;
use crate::PasskeyAddedResponse;
use crate::PasskeyAssertion;
use crate::PasskeyInfo;
use crate::PasskeyOptions;
use crate::PasskeyStep;
use crate::ProveWallet;
use crate::ProviderFinished;
use crate::ProviderInfo;
use crate::ProviderOutcome;
use crate::PublicSession;
use crate::RecoveryCodes;
use crate::RefreshTokens;
use crate::RemovePasskey;
use crate::RenamePasskey;
use crate::RequestInfo;
use crate::RevokeSession;
use crate::SecondFactor;
use crate::SecondFactorChallenge;
use crate::SecondFactorCode;
use crate::SecondFactorResponse;
use crate::SendCode;
use crate::Session;
use crate::SessionResponse;
use crate::SessionToken;
use crate::SignIn;
use crate::SignInStep;
use crate::SignedIn;
use crate::SignedInResponse;
use crate::StartProviderRequest;
use crate::Tokens;
use crate::TotpConfirmed;
use crate::TotpConfirmedResponse;
use crate::TotpSetup;
use crate::Transport;
use crate::UnlinkIdentity;
use crate::UnlinkWallet;
use crate::UpdateWallet;
use crate::VerifyCode;
use crate::WalletChallenge;
use crate::WalletInfo;
use crate::WalletLinked;
use crate::WalletProof;
use crate::WalletSettings;
use crate::WalletSignIn;
use crate::WalletSignInStart;

/// What the session layer learned about a request, as a request
/// extension.
#[derive(Clone, Debug)]
pub struct CurrentSession {
	/// Who the request is from, if its session is live.
	pub signed_in: Option<SignedIn>,
	/// A live session whose account must have two-step verification, which
	/// it hasn't used yet. Policies see no one; the `/auth` routes that set
	/// up or use a second factor, and signing out, are all it may reach.
	pub enrolling: Option<SignedIn>,
	/// Where the request came from.
	pub request: RequestInfo,
	/// When another app made the request for [`signed_in`](Self::signed_in),
	/// with their consent, what it may do.
	pub delegation: Option<Delegation>,
	pub(crate) token: Option<SessionToken>,
}

/// A request another app makes on someone's behalf, with their consent,
/// such as a third-party access token that `leptoskit_idp` checked.
///
/// Only a layer outside [`Auth::layer`] inserts it, as a request extension,
/// once it has verified the token and decided the route may take it. The
/// session layer then treats the request as [`signed_in`](Self::signed_in)'s,
/// so policies apply unchanged, and puts the delegation on
/// [`CurrentSession`]. A delegated request carries no cookie the app reads,
/// so it skips the same-site check.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Delegation {
	/// The person who granted access, or `None` when the app acts as itself,
	/// as a machine-to-machine token does: then the request is signed out,
	/// and policies see no one. A delegated request has no session of its
	/// own, so `signed_in.session` is the nil ID.
	pub signed_in: Option<SignedIn>,
	/// The app acting for them: its OAuth client ID.
	pub client_id: String,
	/// What they let it do.
	pub scopes: Vec<String>,
}

impl Auth {
	/// The layer that resolves each request's session before any handler
	/// runs. It adds a [`CurrentSession`], and a [`Db`](leptoskit_db::Db) whose
	/// policies see the signed-in person, as request extensions. It also
	/// refuses state-changing requests from other sites.
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
	/// | `POST /auth/refresh`                 | trades a native app's tokens ([`RefreshTokens`])    |
	/// | `POST /auth/sign-out`                | ends it                                            |
	/// | `GET /auth/sessions`                 | the person's signed-in devices ([`Device`])         |
	/// | `POST /auth/sessions/revoke`         | ends one of them ([`RevokeSession`])                |
	/// | `POST /auth/sessions/revoke-others`  | ends all but the current one                       |
	/// | `POST /auth/sessions/revoke-all`     | ends every one, the current one included           |
	/// | `POST /auth/second-factor`           | finishes signing in ([`SecondFactorCode`])          |
	/// | `GET /auth/mfa`                      | the account's second factors ([`MfaStatus`])        |
	/// | `POST /auth/step-up`                 | confirms it's still them ([`FactorCode`])           |
	/// | `POST /auth/totp`                    | starts an authenticator app ([`TotpSetup`])         |
	/// | `POST /auth/totp/confirm`            | turns it on ([`FactorCode`])                        |
	/// | `POST /auth/totp/disable`            | turns it off, after step-up                        |
	/// | `POST /auth/recovery-codes`          | new recovery codes, after step-up                  |
	/// | `POST /auth/passkey/sign-in/options` | starts a passkey sign-in ([`PasskeyOptions`])       |
	/// | `POST /auth/passkey/sign-in`         | signs in with a passkey ([`PasskeyAssertion`])      |
	/// | `POST /auth/passkey/second-factor/…` | the second step with a passkey: `options`, then it |
	/// | `POST /auth/passkey/step-up/…`       | step-up with a passkey: `options`, then it         |
	/// | `POST /auth/passkey/register/…`      | adds a passkey: `options`, then it ([`AddPasskey`]) |
	/// | `GET /auth/passkeys`                 | the account's passkeys ([`PasskeyInfo`])            |
	/// | `POST /auth/passkeys/rename`         | renames one ([`RenamePasskey`])                     |
	/// | `POST /auth/passkeys/remove`         | removes one, after step-up ([`RemovePasskey`])      |
	/// | `GET /auth/providers`                | third-party providers ([`ProviderInfo`])            |
	/// | `POST /auth/oauth/start`             | signs in with one ([`StartProviderRequest`])        |
	/// | `POST /auth/oauth/link`              | connects one to the account, likewise              |
	/// | `GET`/`POST /auth/callback/{id}`     | where the provider sends people back               |
	/// | `POST /auth/oauth/finish`            | a native app finishes ([`FinishProvider`])          |
	/// | `POST /auth/oauth/continue`          | a separate account, when the address is taken      |
	/// | `GET /auth/identities`               | connected accounts ([`IdentityInfo`])               |
	/// | `POST /auth/identities/unlink`       | disconnects one ([`UnlinkIdentity`])                |
	/// | `GET`/`POST /auth/merge`             | the merge offer ([`MergeOffer`]), and merging       |
	/// | `POST /auth/merge/passkey/options`   | the other account's passkey, for a merge           |
	/// | `POST /auth/merge/cancel`            | drops the offer                                    |
	/// | `GET /auth/wallets/config`           | whether wallets work here ([`WalletSettings`])      |
	/// | `GET /auth/wallets`                  | the account's wallets ([`WalletInfo`])              |
	/// | `POST /auth/wallets/link/…`          | links one: `options`, then it ([`LinkWallet`])      |
	/// | `POST /auth/wallets/proof/…`         | a fresh proof: `options`, then it ([`WalletProof`]) |
	/// | `POST /auth/wallets/update`          | label, public, signs in ([`UpdateWallet`])          |
	/// | `POST /auth/wallets/unlink`          | unlinks one ([`UnlinkWallet`])                      |
	/// | `POST /auth/wallets/sign-in/…`       | signs in: `options`, then it ([`WalletSignIn`])     |
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
			.route("/auth/refresh", post(refresh))
			.route("/auth/sign-out", post(sign_out))
			.route("/auth/sessions", get(sessions))
			.route("/auth/sessions/revoke", post(revoke))
			.route("/auth/sessions/revoke-others", post(revoke_others))
			.route("/auth/sessions/revoke-all", post(revoke_all))
			.route("/auth/second-factor", post(second_factor))
			.route("/auth/mfa", get(mfa))
			.route("/auth/step-up", post(step_up))
			.route("/auth/totp", post(begin_totp))
			.route("/auth/totp/confirm", post(confirm_totp))
			.route("/auth/totp/disable", post(disable_totp))
			.route("/auth/recovery-codes", post(recovery_codes))
			.route(
				"/auth/passkey/sign-in/options",
				post(passkey_sign_in_options),
			)
			.route("/auth/passkey/sign-in", post(passkey_sign_in))
			.route(
				"/auth/passkey/second-factor/options",
				post(passkey_second_factor_options),
			)
			.route("/auth/passkey/second-factor", post(passkey_second_factor))
			.route(
				"/auth/passkey/step-up/options",
				post(passkey_step_up_options),
			)
			.route("/auth/passkey/step-up", post(passkey_step_up))
			.route(
				"/auth/passkey/register/options",
				post(passkey_register_options),
			)
			.route("/auth/passkey/register", post(passkey_register))
			.route("/auth/passkeys", get(passkeys))
			.route("/auth/passkeys/rename", post(rename_passkey))
			.route("/auth/passkeys/remove", post(remove_passkey))
			.route("/auth/providers", get(providers))
			.route("/auth/oauth/start", post(start_provider))
			.route("/auth/oauth/link", post(link_provider))
			.route("/auth/oauth/continue", post(continue_provider))
			.route("/auth/oauth/finish", post(finish_provider))
			.route(
				"/auth/callback/{provider}",
				get(provider_callback_get).post(provider_callback_post),
			)
			.route("/auth/identities", get(identities))
			.route("/auth/identities/unlink", post(unlink_identity))
			.route("/auth/merge", get(merge_offer).post(merge_accounts))
			.route("/auth/merge/passkey/options", post(merge_passkey_options))
			.route("/auth/merge/cancel", post(cancel_merge))
			.route("/auth/wallets/config", get(wallet_settings))
			.route("/auth/wallets", get(wallets))
			.route("/auth/wallets/link/options", post(wallet_link_options))
			.route("/auth/wallets/link", post(link_wallet))
			.route("/auth/wallets/proof/options", post(wallet_proof_options))
			.route("/auth/wallets/proof", post(prove_wallet))
			.route("/auth/wallets/update", post(update_wallet))
			.route("/auth/wallets/unlink", post(unlink_wallet))
			.route(
				"/auth/wallets/sign-in/options",
				post(wallet_sign_in_options),
			)
			.route("/auth/wallets/sign-in", post(wallet_sign_in));

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

	/// Where a request came from: the client's IP address, as
	/// [`AuthBuilder::client_ip_header`](crate::AuthBuilder::client_ip_header)
	/// says to read it, and its user agent. For routes outside
	/// [`Auth::layer`] that record events, such as `leptoskit_idp`'s token
	/// endpoint; inside it, take [`RequestInfo`] as an extractor.
	pub fn request_info(&self, request: &Request<Body>) -> RequestInfo {
		self.request_info_from(request.headers(), peer(request))
	}

	fn request_info_from(&self, headers: &HeaderMap, peer: Option<SocketAddr>) -> RequestInfo {
		RequestInfo {
			ip: self.client_ip(headers, peer),
			user_agent: headers
				.get(header::USER_AGENT)
				.and_then(|agent| agent.to_str().ok())
				.map(str::to_owned),
			transport: if bearer(headers).is_some() {
				Transport::Bearer
			} else {
				Transport::Cookie
			},
			device_name: None,
		}
	}

	/// What the layer learns about a request: its session, from the bearer
	/// token or the session cookie, and where it came from.
	async fn current_session(
		&self,
		parts_headers: &HeaderMap,
		peer: Option<SocketAddr>,
	) -> Result<CurrentSession, AuthError> {
		let bearer = bearer(parts_headers);
		let request = self.request_info_from(parts_headers, peer);
		let token = bearer.or_else(|| cookie(parts_headers, self.cookie_name()));
		let resolved = match &token {
			Some(token) => self.resolve_session(token).await?,
			None => None,
		};
		let (signed_in, enrolling) = match resolved {
			Some(resolved) if resolved.enrolling => (None, Some(resolved.signed_in)),
			Some(resolved) => (Some(resolved.signed_in), None),
			None => (None, None),
		};

		Ok(CurrentSession {
			signed_in,
			enrolling,
			request,
			delegation: None,
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

		// Neither header: not a browser, unless it sends a cookie a browser
		// would attach on its own.
		[
			self.cookie_name(),
			self.second_factor_cookie_name(),
			self.passkey_cookie_name(),
			self.provider_cookie_name(),
			self.wallet_cookie_name(),
		]
		.into_iter()
		.all(|name| cookie(headers, name).is_none())
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

	/// The cookie that carries the second step of signing in, for five
	/// minutes: `__Host-second-factor` when secure.
	fn second_factor_cookie_name(&self) -> &'static str {
		if self.secure() {
			"__Host-second-factor"
		} else {
			"second-factor"
		}
	}

	/// The cookie that matches a passkey sign-in to its ceremony, for five
	/// minutes: `__Host-passkey` when secure.
	fn passkey_cookie_name(&self) -> &'static str {
		if self.secure() {
			"__Host-passkey"
		} else {
			"passkey"
		}
	}

	/// The cookie that binds a third-party sign-in to the browser that
	/// started it, for ten minutes: `__Host-provider` when secure.
	fn provider_cookie_name(&self) -> &'static str {
		if self.secure() {
			"__Host-provider"
		} else {
			"provider"
		}
	}

	/// The cookie that matches a wallet sign-in to its challenge, for five
	/// minutes: `__Host-wallet` when secure.
	fn wallet_cookie_name(&self) -> &'static str {
		if self.secure() {
			"__Host-wallet"
		} else {
			"wallet"
		}
	}

	pub(crate) fn session_cookie(&self, token: &SessionToken) -> HeaderValue {
		let max_age = self.inner.sessions.absolute.as_secs();
		self.cookie(token.expose(), max_age)
	}

	/// A `Set-Cookie` value that removes the session cookie, for a route
	/// that signs the browser out.
	pub fn cleared_cookie(&self) -> HeaderValue {
		self.cookie("", 0)
	}

	fn cookie(&self, value: &str, max_age: u64) -> HeaderValue {
		self.named_cookie(self.cookie_name(), value, max_age)
	}

	fn named_cookie(&self, name: &str, value: &str, max_age: u64) -> HeaderValue {
		let secure = if self.secure() { "; Secure" } else { "" };
		let cookie =
			format!("{name}={value}; Path=/; HttpOnly; SameSite=Lax; Max-Age={max_age}{secure}");
		HeaderValue::from_str(&cookie).expect("tokens are base64url, which headers allow")
	}

	/// The answer to a first factor when a second is needed: what the step
	/// accepts, and its token, in the body for a native app and in an
	/// `HttpOnly` cookie for a browser.
	fn second_factor_response(
		&self,
		challenge: SecondFactorChallenge,
		transport: Transport,
	) -> Response {
		let expires_in = (challenge.expires_at - leptoskit_db::now())
			.num_seconds()
			.max(0);
		let expires_in = u64::try_from(expires_in).unwrap_or_default();
		let body = SecondFactorResponse {
			second_factor: SecondFactor {
				factors: challenge.factors,
				expires_in,
				token: (transport == Transport::Bearer)
					.then(|| challenge.token.expose().to_owned()),
			},
		};
		let mut response = Json(body).into_response();

		if transport == Transport::Cookie {
			response.headers_mut().insert(
				header::SET_COOKIE,
				self.named_cookie(
					self.second_factor_cookie_name(),
					challenge.token.expose(),
					expires_in,
				),
			);
		}

		response
	}

	/// A privilege change replaced the session's token: the session, a
	/// native app's new tokens, and a browser's new cookie.
	fn rotated(&self, sign_in: SignIn) -> (PublicSession, Option<Tokens>, Option<HeaderValue>) {
		let tokens = sign_in.tokens();
		let cookie = tokens
			.is_none()
			.then(|| self.session_cookie(&sign_in.token));
		(PublicSession::from(sign_in.session), tokens, cookie)
	}

	/// `body`, with the cookie a privilege change set, if any.
	fn rotated_response(body: impl serde::Serialize, cookie: Option<HeaderValue>) -> Response {
		let mut response = Json(body).into_response();

		if let Some(cookie) = cookie {
			response.headers_mut().insert(header::SET_COOKIE, cookie);
		}

		response
	}

	/// The answer to a sign-in: a native app's tokens in the body, or else
	/// the cookie.
	fn signed_in_response(&self, sign_in: SignIn) -> Response {
		let tokens = sign_in.tokens();
		let cookie = tokens
			.is_none()
			.then(|| self.session_cookie(&sign_in.token));
		let body = SignedInResponse {
			session: PublicSession::from(sign_in.session),
			signed_up: sign_in.signed_up,
			tokens,
		};
		let mut response = Json(body).into_response();

		if let Some(cookie) = cookie {
			response.headers_mut().insert(header::SET_COOKIE, cookie);
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

/// The peer address axum's `ConnectInfo` recorded, if the server serves
/// with it.
fn peer(request: &Request<Body>) -> Option<SocketAddr> {
	request
		.extensions()
		.get::<ConnectInfo<SocketAddr>>()
		.map(|ConnectInfo(peer)| *peer)
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
			let delegation = request.extensions_mut().remove::<Delegation>();

			if let Some(delegation) = delegation {
				let current = CurrentSession {
					signed_in: delegation.signed_in.clone(),
					enrolling: None,
					request: RequestInfo {
						transport: Transport::Bearer,
						..auth.request_info(&request)
					},
					delegation: Some(delegation),
					token: None,
				};
				let context = match auth.request_context(current.signed_in.clone()).await {
					Ok(context) => context,
					Err(error) => return Ok(error.into_response()),
				};
				let db = auth.inner.db.with_context(context);
				request.extensions_mut().insert(db);
				request.extensions_mut().insert(auth.clone());
				request.extensions_mut().insert(current);
				return inner.call(request).await;
			}

			// A provider posts its answer from its own site; the callback
			// checks for itself, and only acts on a same-site request.
			let callback = request.uri().path().starts_with("/auth/callback/");

			if !callback && !auth.same_site(request.method(), request.headers()) {
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

			let current = match auth
				.current_session(request.headers(), peer(&request))
				.await
			{
				Ok(current) => current,
				Err(error) => return Ok(error.into_response()),
			};
			let context = match auth.request_context(current.signed_in.clone()).await {
				Ok(context) => context,
				Err(error) => return Ok(error.into_response()),
			};
			let db = auth.inner.db.with_context(context);
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
			Self::SignInExpired => (StatusCode::BAD_REQUEST, "sign_in_expired"),
			Self::StepUpRequired => (StatusCode::FORBIDDEN, "step_up_required"),
			Self::Reauthenticate => (StatusCode::FORBIDDEN, "reauthenticate"),
			Self::AlreadyEnrolled => (StatusCode::CONFLICT, "already_enrolled"),
			Self::NotEnrolled => (StatusCode::CONFLICT, "not_enrolled"),
			Self::MfaRequired => (StatusCode::FORBIDDEN, "mfa_required"),
			Self::PasskeyFailed => (StatusCode::BAD_REQUEST, "passkey_failed"),
			Self::LastSignInMethod => (StatusCode::CONFLICT, "last_sign_in_method"),
			Self::NotFound => (StatusCode::NOT_FOUND, "not_found"),
			Self::InvalidName => (StatusCode::BAD_REQUEST, "invalid_name"),
			Self::UnknownProvider => (StatusCode::NOT_FOUND, "unknown_provider"),
			Self::Provider(_) => (StatusCode::BAD_GATEWAY, "provider"),
			Self::InvalidRedirect => (StatusCode::BAD_REQUEST, "invalid_redirect"),
			Self::Cancelled => (StatusCode::BAD_REQUEST, "cancelled"),
			Self::WalletFailed => (StatusCode::BAD_REQUEST, "wallet_failed"),
			Self::WalletInUse => (StatusCode::CONFLICT, "wallet_in_use"),
			Self::UnknownWallet => (StatusCode::NOT_FOUND, "unknown_wallet"),
			Self::ProofRequired => (StatusCode::FORBIDDEN, "proof_required"),
			Self::MergeConflict(_) => (StatusCode::CONFLICT, "merge_conflict"),
			Self::Unsupported(_) => (StatusCode::NOT_IMPLEMENTED, "unsupported"),
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
			Self::Provider(_) => "signing in with that account didn't work; try again".to_owned(),
			Self::MergeConflict(_) => {
				"the other account's data couldn't move here, so nothing changed; try moving only \
				 its sign-in methods"
					.to_owned()
			}
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

	fn from_request_parts(
		parts: &mut Parts,
		_state: &S,
	) -> impl Future<Output = Result<Self, Self::Rejection>> + Send {
		let current = parts.extensions.get::<CurrentSession>();

		std::future::ready(
			if current.is_some_and(|current| current.enrolling.is_some()) {
				Err(AuthError::MfaRequired)
			} else {
				current
					.and_then(|current| current.signed_in.clone())
					.ok_or(AuthError::SignedOut)
			},
		)
	}
}

/// The signed-in person, including a session that may only set up a
/// second factor: for the routes that do that.
struct Enrolling(SignedIn);

impl<S: Send + Sync> FromRequestParts<S> for Enrolling {
	type Rejection = AuthError;

	fn from_request_parts(
		parts: &mut Parts,
		_state: &S,
	) -> impl Future<Output = Result<Self, Self::Rejection>> + Send {
		std::future::ready(
			parts
				.extensions
				.get::<CurrentSession>()
				.and_then(|current| {
					current
						.signed_in
						.clone()
						.or_else(|| current.enrolling.clone())
				})
				.map(Enrolling)
				.ok_or(AuthError::SignedOut),
		)
	}
}

/// `Option<SignedIn>`: the signed-in person, if there is one.
impl<S: Send + Sync> OptionalFromRequestParts<S> for SignedIn {
	type Rejection = std::convert::Infallible;

	fn from_request_parts(
		parts: &mut Parts,
		_state: &S,
	) -> impl Future<Output = Result<Option<Self>, Self::Rejection>> + Send {
		std::future::ready(Ok(parts
			.extensions
			.get::<CurrentSession>()
			.and_then(|current| current.signed_in.clone())))
	}
}

/// Where the request came from, as the layer saw it.
impl<S: Send + Sync> FromRequestParts<S> for RequestInfo {
	type Rejection = std::convert::Infallible;

	fn from_request_parts(
		parts: &mut Parts,
		_state: &S,
	) -> impl Future<Output = Result<Self, Self::Rejection>> + Send {
		std::future::ready(Ok(parts
			.extensions
			.get::<CurrentSession>()
			.map(|current| current.request.clone())
			.unwrap_or_default()))
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
	let request = RequestInfo {
		transport: body.transport,
		device_name: body.device_name,
		..request
	};
	let step = auth
		.verify_email_code(&body.email, &body.code, &request)
		.await?;
	Ok(match step {
		SignInStep::SignedIn(sign_in) => auth.signed_in_response(*sign_in),
		SignInStep::SecondFactor(challenge) => {
			auth.second_factor_response(challenge, request.transport)
		}
	})
}

async fn second_factor(
	State(auth): State<Auth>,
	request: RequestInfo,
	headers: HeaderMap,
	Json(body): Json<SecondFactorCode>,
) -> Result<Response, AuthError> {
	let token = match body.transport {
		Transport::Bearer => body.token.map(SessionToken::new),
		Transport::Cookie => cookie(&headers, auth.second_factor_cookie_name()),
	}
	.ok_or(AuthError::SignInExpired)?;
	let request = RequestInfo {
		transport: body.transport,
		device_name: body.device_name,
		..request
	};
	let result = auth.complete_sign_in(&token, &body.code, &request).await;
	// A finished or abandoned step's cookie goes; a wrong code keeps it for
	// the next attempt.
	let clear = !matches!(
		result,
		Err(AuthError::InvalidCode | AuthError::RateLimited { .. })
	) && body.transport == Transport::Cookie;
	let mut response = match result {
		Ok(sign_in) => auth.signed_in_response(sign_in),
		Err(error) => error.into_response(),
	};

	if clear {
		response.headers_mut().append(
			header::SET_COOKIE,
			auth.named_cookie(auth.second_factor_cookie_name(), "", 0),
		);
	}

	Ok(response)
}

async fn mfa(
	State(auth): State<Auth>,
	Enrolling(me): Enrolling,
) -> Result<Json<MfaStatus>, AuthError> {
	Ok(Json(auth.mfa_status(me.account).await?))
}

async fn step_up(
	State(auth): State<Auth>,
	Enrolling(me): Enrolling,
	request: RequestInfo,
	Json(body): Json<FactorCode>,
) -> Result<Response, AuthError> {
	let sign_in = auth.step_up(&me, &body.code, &request).await?;
	Ok(auth.signed_in_response(sign_in))
}

async fn begin_totp(
	State(auth): State<Auth>,
	Enrolling(me): Enrolling,
) -> Result<Json<TotpSetup>, AuthError> {
	Ok(Json(auth.begin_totp(&me).await?))
}

async fn confirm_totp(
	State(auth): State<Auth>,
	Enrolling(me): Enrolling,
	request: RequestInfo,
	Json(body): Json<FactorCode>,
) -> Result<Response, AuthError> {
	let TotpConfirmed {
		sign_in,
		recovery_codes,
	} = auth.confirm_totp(&me, &body.code, &request).await?;
	let (session, tokens, cookie) = auth.rotated(sign_in);
	let body = TotpConfirmedResponse {
		session,
		tokens,
		recovery_codes,
	};
	Ok(Auth::rotated_response(body, cookie))
}

async fn disable_totp(
	State(auth): State<Auth>,
	me: SignedIn,
	request: RequestInfo,
) -> Result<StatusCode, AuthError> {
	auth.disable_totp(&me, &request).await?;
	Ok(StatusCode::NO_CONTENT)
}

async fn recovery_codes(
	State(auth): State<Auth>,
	me: SignedIn,
	request: RequestInfo,
) -> Result<Json<RecoveryCodes>, AuthError> {
	Ok(Json(auth.regenerate_recovery_codes(&me, &request).await?))
}

async fn session(
	State(auth): State<Auth>,
	Extension(current): Extension<CurrentSession>,
) -> Result<Json<SessionResponse>, AuthError> {
	let session = match current.signed_in.or(current.enrolling) {
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

async fn refresh(
	State(auth): State<Auth>,
	request: RequestInfo,
	Json(body): Json<RefreshTokens>,
) -> Result<Json<Tokens>, AuthError> {
	let sign_in = auth
		.refresh(&SessionToken::new(body.refresh_token), &request)
		.await?;
	sign_in.tokens().map(Json).ok_or(AuthError::SignedOut)
}

async fn sessions(State(auth): State<Auth>, me: SignedIn) -> Result<Json<Vec<Device>>, AuthError> {
	Ok(Json(auth.devices(&me).await?))
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

async fn revoke_all(
	State(auth): State<Auth>,
	me: SignedIn,
	request: RequestInfo,
) -> Result<Response, AuthError> {
	auth.sign_out_everywhere(me.account, &request).await?;
	let mut response = StatusCode::NO_CONTENT.into_response();
	response
		.headers_mut()
		.insert(header::SET_COOKIE, auth.cleared_cookie());
	Ok(response)
}

async fn dev_sign_in(
	State(auth): State<Auth>,
	request: RequestInfo,
	Json(body): Json<SendCode>,
) -> Result<Response, AuthError> {
	// The development page signs in the browser it's open in.
	let request = RequestInfo {
		transport: Transport::Cookie,
		..request
	};
	let sign_in = auth.dev_sign_in(&body.email, &request).await?;
	Ok(auth.signed_in_response(sign_in))
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

/// Queries and mutations run as the signed-in person: the session the
/// layer resolved for the request, or, when the layer didn't run, the
/// request's own cookie or bearer token. Each session's idempotency keys are
/// its own.
#[cfg(feature = "rpc")]
impl leptoskit_rpc::Identify for Auth {
	fn identify<'a>(
		&'a self,
		request: &'a Parts,
	) -> Pin<
		Box<dyn Future<Output = Result<leptoskit_rpc::Identity, leptoskit_rpc::Error>> + Send + 'a>,
	> {
		Box::pin(async move {
			let signed_in = if let Some(current) = request.extensions.get::<CurrentSession>() {
				current.signed_in.clone()
			} else {
				let peer = request
					.extensions
					.get::<ConnectInfo<SocketAddr>>()
					.map(|ConnectInfo(peer)| *peer);
				self.current_session(&request.headers, peer)
					.await
					.map_err(|error| rpc_error(&error))?
					.signed_in
			};
			let caller = signed_in.as_ref().map(|me| me.session.to_string());
			let identity = leptoskit_rpc::Identity::new(crate::context(signed_in));
			Ok(match caller {
				Some(session) => identity.with_caller(session),
				None => identity,
			})
		})
	}
}

/// What the caller of an operation hears about a failure to identify them.
#[cfg(feature = "rpc")]
fn rpc_error(error: &AuthError) -> leptoskit_rpc::Error {
	let (status, body) = error.describe();
	match status {
		StatusCode::UNAUTHORIZED => leptoskit_rpc::Error::unauthorized(body.message),
		StatusCode::FORBIDDEN => leptoskit_rpc::Error::forbidden(body.message),
		_ => leptoskit_rpc::Error::server(body.message),
	}
}
