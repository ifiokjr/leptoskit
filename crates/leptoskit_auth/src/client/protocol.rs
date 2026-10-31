//! The client every platform shares: the `/auth` routes, over whatever
//! sends HTTP there, with whatever keeps this device's credentials.

#![allow(
	dead_code,
	reason = "each platform's build uses its own part of this, and the tests all of it"
)]

use leptoskit_db::Id;
use leptoskit_query::BoxFuture;
use serde::Serialize;
use serde::de::DeserializeOwned;

use super::Api;
use super::ClientConfig;
use super::EmailCode;
use super::Pending;
use super::SecondFactorPrompt;
use super::SignInStep;
use super::TotpConfirmation;
use crate::Device;
use crate::ErrorResponse;
use crate::FactorCode;
use crate::MfaStatus;
use crate::PublicSession;
use crate::RecoveryCodes;
use crate::RefreshTokens;
use crate::RevokeSession;
use crate::SecondFactorCode;
use crate::SendCode;
use crate::Session;
use crate::SessionResponse;
use crate::SignedInResponse;
use crate::Tokens;
use crate::TotpConfirmedResponse;
use crate::TotpSetup;
use crate::Transport;
use crate::VerifyCode;
use crate::VerifyResponse;

/// `GET` or `POST`, the only methods the routes use.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Method {
	Get,
	Post,
}

impl Method {
	pub(crate) fn as_str(self) -> &'static str {
		match self {
			Self::Get => "GET",
			Self::Post => "POST",
		}
	}
}

/// A request to one of the routes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct HttpRequest {
	pub method: Method,
	/// The origin and path, or the path alone for the page's own origin.
	pub url: String,
	/// `Bearer …`, for a native app.
	pub authorization: Option<String>,
	/// A JSON body.
	pub body: Option<String>,
}

impl HttpRequest {
	/// The headers every sender puts on the request.
	pub(crate) fn headers(&self) -> Vec<(&'static str, &str)> {
		let mut headers = vec![("accept", "application/json")];

		if self.body.is_some() {
			headers.push(("content-type", "application/json"));
		}

		if let Some(authorization) = &self.authorization {
			headers.push(("authorization", authorization));
		}

		headers
	}
}

/// What came back.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct HttpResponse {
	pub status: u16,
	pub body: Vec<u8>,
}

/// Sends requests: `fetch` in the browser, the HTTP plugin in a native app.
pub(crate) trait Http: Send + Sync + 'static {
	fn send(&self, request: HttpRequest)
	-> BoxFuture<'static, Result<HttpResponse, ErrorResponse>>;
}

/// Keeps this device's credentials: attaches them to each request, keeps
/// what signing in returns, and forgets them on signing out.
pub(crate) trait Credentials: Send + Sync + 'static {
	/// How this device asks the server to keep its session.
	const TRANSPORT: Transport;

	/// The `Authorization` header for the next request, if any. Renews first
	/// when what it holds is about to run out.
	fn authorization<'a, H: Http>(
		&'a self,
		server: &'a Endpoint<H>,
	) -> BoxFuture<'a, Result<Option<String>, ErrorResponse>>;

	/// After the server refused `sent`: renews, and says whether the request
	/// is worth sending again.
	fn refused<'a, H: Http>(
		&'a self,
		server: &'a Endpoint<H>,
		sent: Option<String>,
	) -> BoxFuture<'a, Result<bool, ErrorResponse>>;

	/// Keeps what signing in returned.
	fn keep(&self, tokens: Option<Tokens>) -> BoxFuture<'_, Result<(), ErrorResponse>>;

	/// Forgets everything, on signing out.
	fn forget(&self) -> BoxFuture<'_, ()>;

	/// Keeps the second step of signing in's token, or forgets it with
	/// `None`. A browser's is in a cookie, so it holds nothing.
	fn hold_second_factor(&self, token: Option<String>);

	/// The second step's token, to send back with the code.
	fn second_factor(&self) -> Option<String>;
}

/// The server: its origin, and how to reach it.
pub(crate) struct Endpoint<H> {
	origin: String,
	http: H,
}

impl<H: Http> Endpoint<H> {
	pub(crate) fn new(origin: Option<&str>, http: H) -> Self {
		Self {
			origin: origin.unwrap_or_default().trim_end_matches('/').to_owned(),
			http,
		}
	}

	pub(crate) fn url(&self, path: &str) -> String {
		format!("{}{path}", self.origin)
	}

	pub(crate) async fn send(
		&self,
		method: Method,
		path: &str,
		authorization: Option<String>,
		body: Option<String>,
	) -> Result<HttpResponse, ErrorResponse> {
		self.http
			.send(HttpRequest {
				method,
				url: self.url(path),
				authorization,
				body,
			})
			.await
	}

	/// Trades a refresh token at `POST /auth/refresh`.
	pub(crate) async fn refresh(&self, refresh_token: String) -> Result<Tokens, ErrorResponse> {
		let body = json(&RefreshTokens { refresh_token });
		let response = self
			.send(Method::Post, "/auth/refresh", None, Some(body))
			.await?;
		decode(&response)
	}
}

/// The client over the routes, for one platform's HTTP and credentials.
pub(crate) struct Remote<H, C> {
	server: Endpoint<H>,
	credentials: C,
	device_name: Option<String>,
}

impl<H: Http, C: Credentials> Remote<H, C> {
	pub(crate) fn new(config: ClientConfig, http: H, credentials: C) -> Self {
		Self {
			server: Endpoint::new(config.origin.as_deref(), http),
			credentials,
			device_name: config.device_name,
		}
	}

	#[cfg(test)]
	pub(crate) fn credentials(&self) -> &C {
		&self.credentials
	}

	/// Sends a request with this device's credentials. When the server
	/// refuses them, as `refused` judges its answer, renews them and tries
	/// once more.
	async fn call(
		&self,
		method: Method,
		path: &str,
		body: Option<String>,
		refused: fn(&HttpResponse) -> bool,
	) -> Result<HttpResponse, ErrorResponse> {
		let authorization = self.credentials.authorization(&self.server).await?;
		let response = self
			.server
			.send(method, path, authorization.clone(), body.clone())
			.await?;

		if !refused(&response)
			|| !self
				.credentials
				.refused(&self.server, authorization)
				.await?
		{
			return Ok(response);
		}

		let authorization = self.credentials.authorization(&self.server).await?;
		self.server.send(method, path, authorization, body).await
	}
}

impl<H: Http, C: Credentials> Remote<H, C> {
	/// What a native app calls its device, for sign-ins. Browsers are named
	/// from their user agent.
	fn device_name(&self) -> Option<String> {
		self.device_name
			.clone()
			.filter(|_| C::TRANSPORT == Transport::Bearer)
	}

	/// Keeps the tokens a privilege change replaced the session's with. A
	/// browser's new token came as a cookie, so there's nothing to keep.
	async fn rotated(&self, tokens: Option<Tokens>) -> Result<(), ErrorResponse> {
		match (C::TRANSPORT, tokens) {
			(Transport::Cookie, _) => Ok(()),
			(Transport::Bearer, tokens) => self.credentials.keep(tokens).await,
		}
	}
}

fn unauthorized(response: &HttpResponse) -> bool {
	response.status == 401
}

/// `GET /auth/session` answers an unknown token with no session rather than
/// `401`, so no session after sending one means it needs renewing.
fn no_session(response: &HttpResponse) -> bool {
	unauthorized(response)
		|| decode::<SessionResponse>(response).is_ok_and(|answer| answer.session.is_none())
}

impl<H: Http, C: Credentials> Api for Remote<H, C> {
	fn session(&self) -> Pending<'_, Option<PublicSession>> {
		Box::pin(async move {
			let response = self
				.call(Method::Get, "/auth/session", None, no_session)
				.await?;
			Ok(decode::<SessionResponse>(&response)?.session)
		})
	}

	fn send_code(&self, email: String) -> Pending<'_, ()> {
		Box::pin(async move {
			let body = json(&SendCode { email });
			let response = self
				.server
				.send(Method::Post, "/auth/email/send", None, Some(body))
				.await?;
			succeeded(&response)
		})
	}

	fn verify_code(&self, input: EmailCode) -> Pending<'_, SignInStep> {
		Box::pin(async move {
			let body = json(&VerifyCode {
				email: input.email,
				code: input.code,
				transport: C::TRANSPORT,
				device_name: self.device_name(),
			});
			let response = self
				.server
				.send(Method::Post, "/auth/email/verify", None, Some(body))
				.await?;

			match decode(&response)? {
				VerifyResponse::SignedIn(signed_in) => {
					self.credentials.hold_second_factor(None);
					self.credentials.keep(signed_in.tokens).await?;
					Ok(SignInStep::SignedIn(signed_in.session))
				}
				VerifyResponse::SecondFactor(step) => {
					let step = step.second_factor;
					self.credentials.hold_second_factor(step.token);
					Ok(SignInStep::SecondFactor(SecondFactorPrompt {
						factors: step.factors,
						expires_in: step.expires_in,
					}))
				}
			}
		})
	}

	fn complete_sign_in(&self, code: String) -> Pending<'_, PublicSession> {
		Box::pin(async move {
			let body = json(&SecondFactorCode {
				code,
				token: self.credentials.second_factor(),
				transport: C::TRANSPORT,
				device_name: self.device_name(),
			});
			let response = self
				.server
				.send(Method::Post, "/auth/second-factor", None, Some(body))
				.await?;
			let signed_in = decode::<SignedInResponse>(&response).inspect_err(|refused| {
				// The step is over; only a wrong code leaves it to try again.
				if refused.error == "sign_in_expired" {
					self.credentials.hold_second_factor(None);
				}
			})?;
			self.credentials.hold_second_factor(None);
			self.credentials.keep(signed_in.tokens).await?;
			Ok(signed_in.session)
		})
	}

	fn mfa(&self) -> Pending<'_, MfaStatus> {
		Box::pin(async move {
			let response = self
				.call(Method::Get, "/auth/mfa", None, unauthorized)
				.await?;
			decode(&response)
		})
	}

	fn step_up(&self, code: String) -> Pending<'_, PublicSession> {
		Box::pin(async move {
			let body = json(&FactorCode { code });
			let response = self
				.call(Method::Post, "/auth/step-up", Some(body), unauthorized)
				.await?;
			let signed_in: SignedInResponse = decode(&response)?;
			self.rotated(signed_in.tokens).await?;
			Ok(signed_in.session)
		})
	}

	fn begin_totp(&self) -> Pending<'_, TotpSetup> {
		Box::pin(async move {
			let response = self
				.call(Method::Post, "/auth/totp", None, unauthorized)
				.await?;
			decode(&response)
		})
	}

	fn confirm_totp(&self, code: String) -> Pending<'_, TotpConfirmation> {
		Box::pin(async move {
			let body = json(&FactorCode { code });
			let response = self
				.call(Method::Post, "/auth/totp/confirm", Some(body), unauthorized)
				.await?;
			let confirmed: TotpConfirmedResponse = decode(&response)?;
			self.rotated(confirmed.tokens).await?;
			Ok(TotpConfirmation {
				session: confirmed.session,
				recovery_codes: confirmed.recovery_codes,
			})
		})
	}

	fn disable_totp(&self) -> Pending<'_, ()> {
		Box::pin(async move {
			let response = self
				.call(Method::Post, "/auth/totp/disable", None, unauthorized)
				.await?;
			succeeded(&response)
		})
	}

	fn regenerate_recovery_codes(&self) -> Pending<'_, RecoveryCodes> {
		Box::pin(async move {
			let response = self
				.call(Method::Post, "/auth/recovery-codes", None, unauthorized)
				.await?;
			decode(&response)
		})
	}

	fn sign_out(&self) -> Pending<'_, ()> {
		Box::pin(async move {
			let response = self
				.call(Method::Post, "/auth/sign-out", None, |_| false)
				.await;
			self.credentials.forget().await;
			succeeded(&response?)
		})
	}

	fn devices(&self) -> Pending<'_, Vec<Device>> {
		Box::pin(async move {
			let response = self
				.call(Method::Get, "/auth/sessions", None, unauthorized)
				.await?;
			decode(&response)
		})
	}

	fn revoke(&self, id: Id<Session>) -> Pending<'_, bool> {
		Box::pin(async move {
			let body = json(&RevokeSession { id });
			let response = self
				.call(
					Method::Post,
					"/auth/sessions/revoke",
					Some(body),
					unauthorized,
				)
				.await?;

			if response.status == 404 {
				return Ok(false);
			}

			succeeded(&response).map(|()| true)
		})
	}

	fn revoke_others(&self) -> Pending<'_, ()> {
		Box::pin(async move {
			let response = self
				.call(
					Method::Post,
					"/auth/sessions/revoke-others",
					None,
					unauthorized,
				)
				.await?;
			succeeded(&response)
		})
	}

	fn sign_out_everywhere(&self) -> Pending<'_, ()> {
		Box::pin(async move {
			let response = self
				.call(
					Method::Post,
					"/auth/sessions/revoke-all",
					None,
					unauthorized,
				)
				.await?;
			succeeded(&response)?;
			self.credentials.forget().await;
			Ok(())
		})
	}
}

/// Where there's no way to reach the server.
pub(crate) struct Unreachable;

impl Http for Unreachable {
	fn send(
		&self,
		_request: HttpRequest,
	) -> BoxFuture<'static, Result<HttpResponse, ErrorResponse>> {
		Box::pin(std::future::ready(Err(error(
			"config",
			"this build can't reach the server: turn on leptoskit_auth's `web` feature for the \
			 browser, or `guest` for a native app",
		))))
	}
}

pub(crate) fn error(code: &str, message: &str) -> ErrorResponse {
	ErrorResponse {
		error: code.to_owned(),
		message: message.to_owned(),
		retry_after: None,
	}
}

fn json(body: &impl Serialize) -> String {
	serde_json::to_string(body).expect("request bodies are plain structs, which always serialize")
}

/// A successful answer's body, or the error the server sent instead.
pub(crate) fn decode<T: DeserializeOwned>(response: &HttpResponse) -> Result<T, ErrorResponse> {
	succeeded(response)?;
	serde_json::from_slice(&response.body)
		.map_err(|_| error("server", "the server's answer didn't make sense; try again"))
}

/// The error the server sent, if it didn't succeed.
fn succeeded(response: &HttpResponse) -> Result<(), ErrorResponse> {
	if (200..300).contains(&response.status) {
		return Ok(());
	}

	Err(serde_json::from_slice(&response.body)
		.unwrap_or_else(|_| error("server", "something went wrong; try again")))
}
