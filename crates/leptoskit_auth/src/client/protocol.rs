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
use super::LinkWalletInput;
use super::MergeInput;
use super::PasskeyAdded;
use super::PasskeyMode;
use super::Pending;
use super::ProveWalletInput;
use super::SecondFactorPrompt;
use super::SignInStep;
use super::TotpConfirmation;
use super::WalletApp;
use crate::AddPasskey;
use crate::ContinueProvider;
use crate::CreationChallengeResponse;
use crate::Device;
use crate::ErrorResponse;
use crate::FactorCode;
use crate::FinishProvider;
use crate::Identity;
use crate::IdentityInfo;
use crate::LinkWallet;
use crate::MergeAccounts;
use crate::MergeMode;
use crate::MergeOffer;
use crate::MfaStatus;
use crate::NativeStart;
use crate::Passkey;
use crate::PasskeyAddedResponse;
use crate::PasskeyAssertion;
use crate::PasskeyInfo;
use crate::PasskeyOptions;
use crate::PasskeyStep;
use crate::ProveWallet;
use crate::ProviderFinished;
use crate::ProviderInfo;
use crate::ProviderOutcome;
use crate::ProviderRedirect;
use crate::PublicKeyCredential;
use crate::PublicSession;
use crate::RecoveryCodes;
use crate::RefreshTokens;
use crate::RegisterPublicKeyCredential;
use crate::RemovePasskey;
use crate::RenamePasskey;
use crate::RequestChallengeResponse;
use crate::RevokeSession;
use crate::SecondFactorCode;
use crate::SendCode;
use crate::Session;
use crate::SessionResponse;
use crate::SignedInResponse;
use crate::SiwsInput;
use crate::StartProvider;
use crate::StartProviderRequest;
use crate::Tokens;
use crate::TotpConfirmedResponse;
use crate::TotpSetup;
use crate::Transport;
use crate::UnlinkIdentity;
use crate::UnlinkWallet;
use crate::UpdateWallet;
use crate::VerifyCode;
use crate::VerifyResponse;
use crate::Wallet;
use crate::WalletChallenge;
use crate::WalletInfo;
use crate::WalletLinked;
use crate::WalletProof;
use crate::WalletSettings;
use crate::WalletSignIn;
use crate::WalletSignInStart;

/// `GET` or `POST`, the only methods the routes use.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Method {
	/// A read.
	Get,
	/// A change, with a JSON body or none.
	Post,
}

impl Method {
	/// The method's name, such as `GET`.
	pub fn as_str(self) -> &'static str {
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

/// What came back: the status, and the body as it was sent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HttpResponse {
	/// The HTTP status, such as `200`.
	pub status: u16,
	/// The body.
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

/// A native app's system-browser session that has begun: its ID, and where
/// the browser will come back to the app.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct BrowserSession {
	pub id: u32,
	pub return_to: String,
}

/// What Sign in with Apple on the device answered.
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct AppleId {
	/// The ID token Apple signed for the app.
	pub id_token: String,
	/// The name, as the web flow's `user` JSON, on the first sign-in.
	pub user: Option<String>,
}

impl std::fmt::Debug for AppleId {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("AppleId").finish_non_exhaustive()
	}
}

/// What the device itself does for sign-in: passkeys, going through a
/// provider's page, and asking a wallet to sign. A browser does all three
/// itself; a native app does passkeys and provider pages with its passkeys
/// and web-auth plugins, and says `unsupported` for wallets until a native
/// wallet adapter arrives.
pub(crate) trait Platform: Send + Sync + 'static {
	/// Makes a passkey, as `navigator.credentials.create`.
	fn create(
		&self,
		options: CreationChallengeResponse,
	) -> BoxFuture<'static, Result<RegisterPublicKeyCredential, ErrorResponse>>;

	/// Signs with a passkey, as `navigator.credentials.get`; in the
	/// username field's autofill when `autofill`.
	fn get(
		&self,
		options: RequestChallengeResponse,
		autofill: bool,
	) -> BoxFuture<'static, Result<PublicKeyCredential, ErrorResponse>>;

	/// Stops a passkey request that's waiting, such as the autofill one.
	fn cancel(&self);

	/// Sends the page to `url`, such as a provider's sign-in page.
	fn open(&self, url: &str) -> Result<(), ErrorResponse>;

	/// The page's query string, where a provider's outcome arrives.
	fn query(&self) -> Option<String>;

	/// The Solana wallet apps this device offers. Only a browser has any
	/// yet; elsewhere this answers `unsupported`.
	fn wallet_apps(&self) -> BoxFuture<'static, Result<Vec<WalletApp>, ErrorResponse>> {
		Box::pin(std::future::ready(no_wallets()))
	}

	/// Asks the wallet app named `wallet` to sign the Sign-In With Solana
	/// message `input` describes, with `input.address` when it names one.
	fn sign_with_wallet(
		&self,
		_wallet: String,
		_input: SiwsInput,
	) -> BoxFuture<'static, Result<WalletProof, ErrorResponse>> {
		Box::pin(std::future::ready(no_wallets()))
	}

	/// Whether the device has passkeys at all. Without them, passkey calls
	/// answer `unsupported` before asking the server anything.
	fn passkeys(&self) -> bool {
		true
	}

	/// Whether it offers passkeys in the email field's autofill.
	fn autofill(&self) -> bool {
		true
	}

	/// For a native app: begins a system-browser session.
	fn begin_browser(&self) -> BoxFuture<'static, Result<BrowserSession, ErrorResponse>> {
		Box::pin(std::future::ready(unsupported()))
	}

	/// Opens `url` in the session's browser, and waits for it to come back
	/// to the app: the address it came back to.
	fn browse(
		&self,
		_session: BrowserSession,
		_url: String,
	) -> BoxFuture<'static, Result<String, ErrorResponse>> {
		Box::pin(std::future::ready(unsupported()))
	}

	/// Whether the device may have Sign in with Apple of its own.
	fn has_apple_id(&self) -> bool {
		false
	}

	/// Signs in with Apple on the device, with the server's nonce. Answers
	/// `unsupported` where the device has none after all.
	fn apple_id(&self, _nonce: String) -> BoxFuture<'static, Result<AppleId, ErrorResponse>> {
		Box::pin(std::future::ready(unsupported()))
	}
}

/// A device without passkeys or a page to send anywhere.
pub(crate) struct NoPlatform;

/// What a device without passkeys or a system browser answers.
pub(crate) fn unsupported<T>() -> Result<T, ErrorResponse> {
	Err(error(
		"unsupported",
		"this device can't do that: a native app needs the `passkeys` and `web-auth` plugins, \
		 granted in `[native] plugins`",
	))
}

/// What a native app answers for asking a wallet to sign, until a native
/// wallet adapter exists.
pub(crate) fn no_wallets<T>() -> Result<T, ErrorResponse> {
	Err(error(
		"unsupported",
		"this device can't reach Solana wallets yet: linking a wallet and signing in with one \
		 work in the browser, through the Wallet Standard",
	))
}

impl Platform for NoPlatform {
	fn create(
		&self,
		_options: CreationChallengeResponse,
	) -> BoxFuture<'static, Result<RegisterPublicKeyCredential, ErrorResponse>> {
		Box::pin(std::future::ready(unsupported()))
	}

	fn get(
		&self,
		_options: RequestChallengeResponse,
		_autofill: bool,
	) -> BoxFuture<'static, Result<PublicKeyCredential, ErrorResponse>> {
		Box::pin(std::future::ready(unsupported()))
	}

	fn cancel(&self) {}

	fn open(&self, _url: &str) -> Result<(), ErrorResponse> {
		unsupported()
	}

	fn query(&self) -> Option<String> {
		None
	}

	fn passkeys(&self) -> bool {
		false
	}

	fn autofill(&self) -> bool {
		false
	}
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
	platform: Box<dyn Platform>,
	/// A native app's last provider sign-in: how it ended, and its binding
	/// while a sign-up waits for the person's choice. In memory only.
	provider: std::sync::Mutex<NativeProvider>,
}

/// Where a native app's provider sign-in got to. A browser's arrives in
/// the page's query instead.
#[derive(Default)]
struct NativeProvider {
	outcome: Option<ProviderOutcome>,
	binding: Option<String>,
}

impl<H: Http, C: Credentials> Remote<H, C> {
	pub(crate) fn new(config: ClientConfig, http: H, credentials: C) -> Self {
		Self {
			server: Endpoint::new(config.origin.as_deref(), http),
			credentials,
			device_name: config.device_name,
			platform: Box::new(NoPlatform),
			provider: std::sync::Mutex::new(NativeProvider::default()),
		}
	}

	/// Uses `platform` for passkeys and provider pages.
	pub(crate) fn platform(mut self, platform: impl Platform) -> Self {
		self.platform = Box::new(platform);
		self
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
			self.first_step(&response).await
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

	fn passkey_sign_in(&self, mode: PasskeyMode) -> Pending<'_, PublicSession> {
		Box::pin(Self::passkey_sign_in(self, mode))
	}

	fn cancel_passkey(&self) {
		Self::cancel_passkey(self);
	}

	fn add_passkey(&self, name: Option<String>) -> Pending<'_, PasskeyAdded> {
		Box::pin(Self::add_passkey(self, name))
	}

	fn complete_sign_in_with_passkey(&self) -> Pending<'_, PublicSession> {
		Box::pin(Self::complete_sign_in_with_passkey(self))
	}

	fn step_up_with_passkey(&self) -> Pending<'_, PublicSession> {
		Box::pin(Self::step_up_with_passkey(self))
	}

	fn passkeys(&self) -> Pending<'_, Vec<PasskeyInfo>> {
		Box::pin(Self::passkeys(self))
	}

	fn rename_passkey(&self, input: RenamePasskey) -> Pending<'_, ()> {
		Box::pin(Self::rename_passkey(self, input))
	}

	fn remove_passkey(&self, id: Id<Passkey>) -> Pending<'_, ()> {
		Box::pin(Self::remove_passkey(self, id))
	}

	fn providers(&self) -> Pending<'_, Vec<ProviderInfo>> {
		Box::pin(Self::providers(self))
	}

	fn start_provider(&self, input: StartProvider, link: bool) -> Pending<'_, ()> {
		Box::pin(Self::start_provider(self, input, link))
	}

	fn continue_provider_sign_up(&self) -> Pending<'_, PublicSession> {
		Box::pin(Self::continue_provider_sign_up(self))
	}

	fn provider_outcome(&self) -> Option<ProviderOutcome> {
		Self::provider_outcome(self)
	}

	fn identities(&self) -> Pending<'_, Vec<IdentityInfo>> {
		Box::pin(Self::identities(self))
	}

	fn unlink_identity(&self, id: Id<Identity>) -> Pending<'_, ()> {
		Box::pin(Self::unlink_identity(self, id))
	}

	fn merge_offer(&self) -> Pending<'_, Option<MergeOffer>> {
		Box::pin(Self::merge_offer(self))
	}

	fn merge_accounts(&self, input: MergeInput) -> Pending<'_, ()> {
		Box::pin(Self::merge_accounts(self, input))
	}

	fn cancel_merge(&self) -> Pending<'_, ()> {
		Box::pin(Self::cancel_merge(self))
	}

	fn wallet_settings(&self) -> Pending<'_, WalletSettings> {
		Box::pin(Self::wallet_settings(self))
	}

	fn wallet_apps(&self) -> Pending<'_, Vec<WalletApp>> {
		Box::pin(Self::wallet_apps(self))
	}

	fn wallets(&self) -> Pending<'_, Vec<WalletInfo>> {
		Box::pin(Self::wallets(self))
	}

	fn link_wallet(&self, input: LinkWalletInput) -> Pending<'_, WalletLinked> {
		Box::pin(Self::link_wallet(self, input))
	}

	fn prove_wallet(&self, input: ProveWalletInput) -> Pending<'_, WalletInfo> {
		Box::pin(Self::prove_wallet(self, input))
	}

	fn update_wallet(&self, input: UpdateWallet) -> Pending<'_, WalletInfo> {
		Box::pin(Self::update_wallet(self, input))
	}

	fn unlink_wallet(&self, id: Id<Wallet>) -> Pending<'_, ()> {
		Box::pin(Self::unlink_wallet(self, id))
	}

	fn wallet_sign_in(&self, wallet: String) -> Pending<'_, SignInStep> {
		Box::pin(Self::wallet_sign_in(self, wallet))
	}

	fn send(
		&self,
		method: Method,
		path: String,
		body: Option<String>,
	) -> Pending<'_, HttpResponse> {
		Box::pin(async move { self.call(method, &path, body, unauthorized).await })
	}
}

impl<H: Http, C: Credentials> Remote<H, C> {
	/// Answers `unsupported` before asking the server anything when the
	/// device has no passkeys.
	fn passkeys_here(&self) -> Result<(), ErrorResponse> {
		if self.platform.passkeys() {
			Ok(())
		} else {
			unsupported()
		}
	}

	/// Keeps what a first factor's answer returned: a session, or the second
	/// step's token.
	async fn first_step(&self, response: &HttpResponse) -> Result<SignInStep, ErrorResponse> {
		match decode(response)? {
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
	}

	/// Keeps what a sign-in returned, and answers its session.
	async fn signed_in(&self, response: &HttpResponse) -> Result<PublicSession, ErrorResponse> {
		let signed_in: SignedInResponse = decode(response)?;
		self.credentials.keep(signed_in.tokens).await?;
		Ok(signed_in.session)
	}

	pub(crate) async fn passkey_sign_in(
		&self,
		mode: PasskeyMode,
	) -> Result<PublicSession, ErrorResponse> {
		self.passkeys_here()?;

		if mode == PasskeyMode::Autofill && !self.platform.autofill() {
			return unsupported();
		}

		let body = json(&PasskeyStep {
			token: None,
			transport: C::TRANSPORT,
		});
		let response = self
			.server
			.send(
				Method::Post,
				"/auth/passkey/sign-in/options",
				None,
				Some(body),
			)
			.await?;
		let mut started: PasskeyOptions = decode(&response)?;
		let autofill = mode == PasskeyMode::Autofill;

		if !autofill {
			started.options.mediation = None;
		}

		let credential = self.platform.get(started.options, autofill).await?;
		let body = json(&PasskeyAssertion {
			credential,
			token: started.token,
			transport: C::TRANSPORT,
			device_name: self.device_name(),
		});
		let response = self
			.server
			.send(Method::Post, "/auth/passkey/sign-in", None, Some(body))
			.await?;
		self.signed_in(&response).await
	}

	pub(crate) async fn add_passkey(
		&self,
		name: Option<String>,
	) -> Result<PasskeyAdded, ErrorResponse> {
		self.passkeys_here()?;
		let response = self
			.call(
				Method::Post,
				"/auth/passkey/register/options",
				None,
				unauthorized,
			)
			.await?;
		let options: CreationChallengeResponse = decode(&response)?;
		let credential = self.platform.create(options).await?;
		let body = json(&AddPasskey { credential, name });
		let response = self
			.call(
				Method::Post,
				"/auth/passkey/register",
				Some(body),
				unauthorized,
			)
			.await?;
		let added: PasskeyAddedResponse = decode(&response)?;
		self.rotated(added.tokens).await?;
		Ok(PasskeyAdded {
			passkey: added.passkey,
			session: added.session,
			recovery_codes: added.recovery_codes,
		})
	}

	pub(crate) async fn complete_sign_in_with_passkey(
		&self,
	) -> Result<PublicSession, ErrorResponse> {
		self.passkeys_here()?;
		let token = self.credentials.second_factor();
		let body = json(&PasskeyStep {
			token: token.clone(),
			transport: C::TRANSPORT,
		});
		let response = self
			.server
			.send(
				Method::Post,
				"/auth/passkey/second-factor/options",
				None,
				Some(body),
			)
			.await?;
		let started: PasskeyOptions = decode(&response)?;
		let credential = self.platform.get(started.options, false).await?;
		let body = json(&PasskeyAssertion {
			credential,
			token,
			transport: C::TRANSPORT,
			device_name: self.device_name(),
		});
		let response = self
			.server
			.send(
				Method::Post,
				"/auth/passkey/second-factor",
				None,
				Some(body),
			)
			.await?;
		let session = self.signed_in(&response).await.inspect_err(|refused| {
			if refused.error == "sign_in_expired" {
				self.credentials.hold_second_factor(None);
			}
		})?;
		self.credentials.hold_second_factor(None);
		Ok(session)
	}

	pub(crate) async fn step_up_with_passkey(&self) -> Result<PublicSession, ErrorResponse> {
		self.passkeys_here()?;
		let response = self
			.call(
				Method::Post,
				"/auth/passkey/step-up/options",
				None,
				unauthorized,
			)
			.await?;
		let started: PasskeyOptions = decode(&response)?;
		let credential = self.platform.get(started.options, false).await?;
		let body = json(&PasskeyAssertion {
			credential,
			token: None,
			transport: C::TRANSPORT,
			device_name: None,
		});
		let response = self
			.call(
				Method::Post,
				"/auth/passkey/step-up",
				Some(body),
				unauthorized,
			)
			.await?;
		let signed_in: SignedInResponse = decode(&response)?;
		self.rotated(signed_in.tokens).await?;
		Ok(signed_in.session)
	}

	pub(crate) async fn passkeys(&self) -> Result<Vec<PasskeyInfo>, ErrorResponse> {
		let response = self
			.call(Method::Get, "/auth/passkeys", None, unauthorized)
			.await?;
		decode(&response)
	}

	pub(crate) async fn rename_passkey(&self, input: RenamePasskey) -> Result<(), ErrorResponse> {
		let response = self
			.call(
				Method::Post,
				"/auth/passkeys/rename",
				Some(json(&input)),
				unauthorized,
			)
			.await?;
		succeeded(&response)
	}

	pub(crate) async fn remove_passkey(&self, id: Id<Passkey>) -> Result<(), ErrorResponse> {
		let response = self
			.call(
				Method::Post,
				"/auth/passkeys/remove",
				Some(json(&RemovePasskey { id })),
				unauthorized,
			)
			.await?;
		succeeded(&response)
	}

	pub(crate) async fn providers(&self) -> Result<Vec<ProviderInfo>, ErrorResponse> {
		let response = self
			.server
			.send(Method::Get, "/auth/providers", None, None)
			.await?;
		decode(&response)
	}

	pub(crate) async fn start_provider(
		&self,
		input: StartProvider,
		link: bool,
	) -> Result<(), ErrorResponse> {
		if C::TRANSPORT == Transport::Bearer {
			return self.native_provider(input, link).await;
		}

		let path = if link {
			"/auth/oauth/link"
		} else {
			"/auth/oauth/start"
		};
		let body = StartProviderRequest {
			start: input,
			native: None,
		};
		let response = self
			.call(Method::Post, path, Some(json(&body)), unauthorized)
			.await?;
		let redirect: ProviderRedirect = decode(&response)?;
		self.platform.open(&redirect.url)
	}

	/// A native app's provider sign-in: with Sign in with Apple on the
	/// device when it's Apple and the device has it, and otherwise in the
	/// system browser. Once the provider has answered, how it ended is
	/// [`Remote::provider_outcome`], as a browser's return page would say.
	async fn native_provider(&self, input: StartProvider, link: bool) -> Result<(), ErrorResponse> {
		*self.native_state() = NativeProvider::default();
		let path = if link {
			"/auth/oauth/link"
		} else {
			"/auth/oauth/start"
		};

		if self.platform.has_apple_id() {
			let body = StartProviderRequest {
				start: input.clone(),
				native: Some(NativeStart::AppleId),
			};
			let response = self
				.call(Method::Post, path, Some(json(&body)), unauthorized)
				.await?;

			match decode::<ProviderRedirect>(&response) {
				Ok(started) => {
					if let Some(done) = self.apple_id(&input.provider, started).await? {
						return Ok(done);
					}
				}
				// Not Apple's provider: the system browser instead.
				Err(refused) if refused.error == "unsupported" => {}
				Err(refused) => return Err(refused),
			}
		}

		let session = self.platform.begin_browser().await?;
		let body = StartProviderRequest {
			start: input.clone(),
			native: Some(NativeStart::Browser {
				return_to: session.return_to.clone(),
			}),
		};
		let response = self
			.call(Method::Post, path, Some(json(&body)), unauthorized)
			.await?;
		let started: ProviderRedirect = decode(&response)?;
		let token = started.token.ok_or_else(|| {
			error(
				"server",
				"the server didn't bind the sign-in to this app; is it up to date?",
			)
		})?;
		let sent_state = query_value(&started.url, "state");
		let returned = match self.platform.browse(session, started.url).await {
			Ok(returned) => returned,
			Err(failed) => {
				self.native_state().outcome = Some(ProviderOutcome::Failed(failed.error));
				return Ok(());
			}
		};

		if sent_state.is_none() || query_value(&returned, "state") != sent_state {
			self.native_state().outcome = Some(ProviderOutcome::Failed("sign_in_expired".into()));
			return Ok(());
		}

		let finish = FinishProvider {
			provider: input.provider,
			token,
			state: sent_state,
			code: query_value(&returned, "code"),
			error: query_value(&returned, "error"),
			user: query_value(&returned, "user"),
			id_token: None,
			device_name: self.device_name(),
		};
		self.finish_native(finish).await
	}

	/// Signs in with Apple on the device. `None` when the device turns out
	/// to have no Sign in with Apple, for the system browser to try.
	async fn apple_id(
		&self,
		provider: &str,
		started: ProviderRedirect,
	) -> Result<Option<()>, ErrorResponse> {
		let (Some(token), Some(nonce)) = (started.token, started.nonce) else {
			return Err(error(
				"server",
				"the server didn't send a nonce for Sign in with Apple; is it up to date?",
			));
		};
		let credential = match self.platform.apple_id(nonce).await {
			Ok(credential) => credential,
			Err(failed) if failed.error == "unsupported" => return Ok(None),
			Err(failed) => {
				self.native_state().outcome = Some(ProviderOutcome::Failed(failed.error));
				return Ok(Some(()));
			}
		};
		let finish = FinishProvider {
			provider: provider.to_owned(),
			token,
			user: credential.user,
			id_token: Some(credential.id_token),
			device_name: self.device_name(),
			..FinishProvider::default()
		};
		self.finish_native(finish).await.map(Some)
	}

	/// Finishes a native app's provider flow, and keeps what it ended with.
	async fn finish_native(&self, finish: FinishProvider) -> Result<(), ErrorResponse> {
		let token = finish.token.clone();
		let response = self
			.server
			.send(
				Method::Post,
				"/auth/oauth/finish",
				None,
				Some(json(&finish)),
			)
			.await;
		let finished = match response.and_then(|response| decode::<ProviderFinished>(&response)) {
			Ok(finished) => finished,
			Err(refused) => {
				self.native_state().outcome = Some(ProviderOutcome::Failed(refused.error));
				return Ok(());
			}
		};

		if let Some(signed_in) = finished.signed_in {
			self.credentials.hold_second_factor(None);
			self.credentials.keep(signed_in.tokens).await?;
		}

		if let Some(step) = finished.second_factor {
			self.credentials.hold_second_factor(step.token);
		}

		let mut state = self.native_state();
		state.binding = (finished.outcome == ProviderOutcome::EmailInUse).then_some(token);
		state.outcome = Some(finished.outcome);
		Ok(())
	}

	fn native_state(&self) -> std::sync::MutexGuard<'_, NativeProvider> {
		self.provider
			.lock()
			.unwrap_or_else(std::sync::PoisonError::into_inner)
	}

	pub(crate) async fn continue_provider_sign_up(&self) -> Result<PublicSession, ErrorResponse> {
		let body = match C::TRANSPORT {
			Transport::Cookie => None,
			Transport::Bearer => {
				let token = self.native_state().binding.clone().ok_or_else(|| {
					error("sign_in_expired", "signing in took too long; start again")
				})?;
				Some(json(&ContinueProvider {
					token: Some(token),
					device_name: self.device_name(),
				}))
			}
		};
		let response = self
			.server
			.send(Method::Post, "/auth/oauth/continue", None, body)
			.await?;
		let session = self.signed_in(&response).await?;
		self.native_state().binding = None;
		Ok(session)
	}

	pub(crate) fn provider_outcome(&self) -> Option<ProviderOutcome> {
		if let Some(outcome) = self.native_state().outcome.clone() {
			return Some(outcome);
		}

		ProviderOutcome::from_query(&self.platform.query()?)
	}

	pub(crate) async fn identities(&self) -> Result<Vec<IdentityInfo>, ErrorResponse> {
		let response = self
			.call(Method::Get, "/auth/identities", None, unauthorized)
			.await?;
		decode(&response)
	}

	pub(crate) async fn unlink_identity(&self, id: Id<Identity>) -> Result<(), ErrorResponse> {
		let response = self
			.call(
				Method::Post,
				"/auth/identities/unlink",
				Some(json(&UnlinkIdentity { id })),
				unauthorized,
			)
			.await?;
		succeeded(&response)
	}

	pub(crate) async fn merge_offer(&self) -> Result<Option<MergeOffer>, ErrorResponse> {
		let response = self
			.call(Method::Get, "/auth/merge", None, unauthorized)
			.await?;
		decode(&response)
	}

	pub(crate) async fn merge_accounts(&self, input: MergeInput) -> Result<(), ErrorResponse> {
		let credential = if input.passkey {
			self.passkeys_here()?;
			let response = self
				.call(
					Method::Post,
					"/auth/merge/passkey/options",
					None,
					unauthorized,
				)
				.await?;
			let started: PasskeyOptions = decode(&response)?;
			Some(self.platform.get(started.options, false).await?)
		} else {
			None
		};
		let body = json(&MergeAccounts {
			mode: input.mode.unwrap_or(MergeMode::MoveLogin),
			confirm: input.confirm,
			code: input.code,
			credential,
		});
		let response = self
			.call(Method::Post, "/auth/merge", Some(body), unauthorized)
			.await?;
		succeeded(&response)
	}

	pub(crate) async fn cancel_merge(&self) -> Result<(), ErrorResponse> {
		let response = self
			.call(Method::Post, "/auth/merge/cancel", None, unauthorized)
			.await?;
		succeeded(&response)
	}

	pub(crate) fn cancel_passkey(&self) {
		self.platform.cancel();
	}

	/// A browser asks its wallets; a native app answers `unsupported`
	/// before asking the server anything.
	fn wallets_here() -> Result<(), ErrorResponse> {
		match C::TRANSPORT {
			Transport::Cookie => Ok(()),
			Transport::Bearer => no_wallets(),
		}
	}

	pub(crate) async fn wallet_settings(&self) -> Result<WalletSettings, ErrorResponse> {
		let response = self
			.server
			.send(Method::Get, "/auth/wallets/config", None, None)
			.await?;
		decode(&response)
	}

	pub(crate) async fn wallet_apps(&self) -> Result<Vec<WalletApp>, ErrorResponse> {
		Self::wallets_here()?;
		self.platform.wallet_apps().await
	}

	pub(crate) async fn wallets(&self) -> Result<Vec<WalletInfo>, ErrorResponse> {
		let response = self
			.call(Method::Get, "/auth/wallets", None, unauthorized)
			.await?;
		decode(&response)
	}

	pub(crate) async fn link_wallet(
		&self,
		input: LinkWalletInput,
	) -> Result<WalletLinked, ErrorResponse> {
		Self::wallets_here()?;
		let response = self
			.call(
				Method::Post,
				"/auth/wallets/link/options",
				None,
				unauthorized,
			)
			.await?;
		let challenge: WalletChallenge = decode(&response)?;
		let proof = self
			.platform
			.sign_with_wallet(input.wallet, challenge.input)
			.await?;
		let body = json(&LinkWallet {
			proof,
			label: input.label,
		});
		let response = self
			.call(Method::Post, "/auth/wallets/link", Some(body), unauthorized)
			.await?;
		decode(&response)
	}

	pub(crate) async fn prove_wallet(
		&self,
		input: ProveWalletInput,
	) -> Result<WalletInfo, ErrorResponse> {
		Self::wallets_here()?;
		let body = json(&ProveWallet {
			address: input.address,
		});
		let response = self
			.call(
				Method::Post,
				"/auth/wallets/proof/options",
				Some(body),
				unauthorized,
			)
			.await?;
		let challenge: WalletChallenge = decode(&response)?;
		let proof = self
			.platform
			.sign_with_wallet(input.wallet, challenge.input)
			.await?;
		let response = self
			.call(
				Method::Post,
				"/auth/wallets/proof",
				Some(json(&proof)),
				unauthorized,
			)
			.await?;
		decode(&response)
	}

	pub(crate) async fn update_wallet(
		&self,
		input: UpdateWallet,
	) -> Result<WalletInfo, ErrorResponse> {
		let response = self
			.call(
				Method::Post,
				"/auth/wallets/update",
				Some(json(&input)),
				unauthorized,
			)
			.await?;
		decode(&response)
	}

	pub(crate) async fn unlink_wallet(&self, id: Id<Wallet>) -> Result<(), ErrorResponse> {
		let response = self
			.call(
				Method::Post,
				"/auth/wallets/unlink",
				Some(json(&UnlinkWallet { id })),
				unauthorized,
			)
			.await?;
		succeeded(&response)
	}

	pub(crate) async fn wallet_sign_in(&self, wallet: String) -> Result<SignInStep, ErrorResponse> {
		Self::wallets_here()?;
		let body = json(&WalletSignInStart {
			transport: C::TRANSPORT,
		});
		let response = self
			.server
			.send(
				Method::Post,
				"/auth/wallets/sign-in/options",
				None,
				Some(body),
			)
			.await?;
		let challenge: WalletChallenge = decode(&response)?;
		let proof = self
			.platform
			.sign_with_wallet(wallet, challenge.input)
			.await?;
		let body = json(&WalletSignIn {
			proof,
			token: challenge.token,
			transport: C::TRANSPORT,
			device_name: self.device_name(),
		});
		let response = self
			.server
			.send(Method::Post, "/auth/wallets/sign-in", None, Some(body))
			.await?;
		self.first_step(&response).await
	}
}

/// The value of `name` in `url`'s query, decoded.
fn query_value(url: &str, name: &str) -> Option<String> {
	let (_, query) = url.split_once('?')?;
	let query = query.split('#').next().unwrap_or_default();
	form_pairs(query)
		.into_iter()
		.find(|(key, _)| key == name)
		.map(|(_, value)| value)
}

/// `application/x-www-form-urlencoded` pairs: `+` is a space and `%XX` a
/// byte. A malformed escape is kept as it is.
fn form_pairs(query: &str) -> Vec<(String, String)> {
	let decode = |text: &str| {
		let bytes = text.as_bytes();
		let mut out = Vec::with_capacity(bytes.len());
		let mut index = 0;

		while index < bytes.len() {
			match bytes[index] {
				b'+' => out.push(b' '),
				b'%' => {
					let hex = bytes
						.get(index + 1..index + 3)
						.and_then(|pair| std::str::from_utf8(pair).ok())
						.and_then(|pair| u8::from_str_radix(pair, 16).ok());

					if let Some(byte) = hex {
						out.push(byte);
						index += 2;
					} else {
						out.push(b'%');
					}
				}
				byte => out.push(byte),
			}

			index += 1;
		}

		String::from_utf8_lossy(&out).into_owned()
	};

	query
		.split('&')
		.filter(|pair| !pair.is_empty())
		.map(|pair| {
			let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
			(decode(key), decode(value))
		})
		.collect()
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
