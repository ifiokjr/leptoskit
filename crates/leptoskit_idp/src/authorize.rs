//! The authorization code flow, up to the code: checking the request,
//! signing in, asking for consent, and sending the browser back.

use leptoskit_auth::EventKind;
use leptoskit_auth::RequestInfo;
use leptoskit_auth::SignedIn;
use leptoskit_db::Id;
use leptoskit_db::Json;
use leptoskit_db::Tx;
use serde::Deserialize;
use serde::Serialize;
use serde_json::Value;
use url::Url;

use crate::AppInfo;
use crate::Authorization;
use crate::Client;
use crate::ConsentRequest;
use crate::Grant;
use crate::GrantType;
use crate::Idp;
use crate::IdpError;
use crate::NewAuthorization;
use crate::NewGrant;
use crate::PushedRequest;
use crate::Result;
use crate::clients::redirect_matches;
use crate::par::REQUEST_URI_PREFIX;
use crate::secrets;

/// The longest `state` or `nonce` kept.
const MAX_ECHOED: usize = 2048;

/// The longest `claims` parameter read.
const MAX_CLAIMS: usize = 4096;

/// The query of `GET /oauth/authorize`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthorizeRequest {
	/// Must be `code`.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub response_type: Option<String>,
	/// The app asking.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub client_id: Option<String>,
	/// Where to send the answer: exactly one of the app's registered URIs.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub redirect_uri: Option<String>,
	/// The scopes asked for, space-separated.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub scope: Option<String>,
	/// Echoed back to the app unchanged.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub state: Option<String>,
	/// Echoed in the ID token.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub nonce: Option<String>,
	/// The PKCE challenge: required.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub code_challenge: Option<String>,
	/// Must be `S256`.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub code_challenge_method: Option<String>,
	/// `none`, `login`, or `consent`.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub prompt: Option<String>,
	/// The most seconds since the person last signed in.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub max_age: Option<String>,
	/// Only `query` is supported.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub response_mode: Option<String>,
	/// Claims to find in the ID token, as JSON (OpenID Connect Core §5.5),
	/// such as `{"id_token":{"email":null}}`. Only claims the scopes release
	/// appear.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub claims: Option<String>,
	/// Not supported: push the request to `/oauth/par` instead.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub request: Option<String>,
	/// A request pushed to `/oauth/par` (RFC 9126), which stands for every
	/// other parameter but `client_id`.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub request_uri: Option<String>,
}

/// What to do with the browser after `GET /oauth/authorize`.
#[derive(Debug)]
pub enum AuthorizeOutcome {
	/// Send it here: back to the app with a code or an error, or to your
	/// sign-in or consent page.
	Redirect(String),
	/// The request names no registered app, or no address of its: there's
	/// nowhere safe to send the browser, so show the error.
	Invalid(IdpError),
}

/// A checked request: the app, where to answer, and what it asked for.
struct Checked {
	client: Client,
	redirect_uri: String,
	scopes: Vec<String>,
	code_challenge: String,
	prompt: Prompt,
	max_age: Option<i64>,
	claims: RequestedClaims,
	/// The pushed request it came from, which it uses up.
	pushed: Option<Id<PushedRequest>>,
}

/// What the `claims` parameter asked for.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct RequestedClaims {
	/// Claims to put in the ID token.
	pub id_token: Vec<String>,
	/// The only `sub` the app will accept, if it named one.
	pub subject: Option<String>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Prompt {
	Default,
	None,
	Login,
	Consent,
}

impl Idp {
	/// Answers `GET /oauth/authorize` for `signed_in`, or a signed-out
	/// browser.
	///
	/// A good request from someone signed in gets a code straight away if
	/// they already consented to every scope, or else goes to your consent
	/// page. Someone signed out goes to your sign-in page, which sends them
	/// back.
	///
	/// # Errors
	///
	/// When the database fails. Protocol errors are outcomes: sent back to
	/// the app, or [`AuthorizeOutcome::Invalid`].
	pub async fn authorize(
		&self,
		request: &AuthorizeRequest,
		signed_in: Option<&SignedIn>,
		info: &RequestInfo,
	) -> Result<AuthorizeOutcome> {
		let original = request;
		let client_id = request.client_id.as_deref().unwrap_or_default();
		let Some(client) = self
			.client(client_id)
			.await?
			.filter(|client| client.grant_types.contains(&GrantType::AuthorizationCode))
		else {
			return Ok(AuthorizeOutcome::Invalid(IdpError::invalid_request(
				"no app is registered with that client_id",
			)));
		};

		// A pushed request stands for every parameter but the client's.
		let pushed = match request.request_uri.as_deref() {
			Some(uri) if uri.starts_with(REQUEST_URI_PREFIX) => {
				match self.pushed_request(&client, uri).await? {
					Some(pushed) => Some(pushed),
					None => {
						return Ok(AuthorizeOutcome::Invalid(IdpError::oauth(
							"invalid_request_uri",
							"that request_uri is unknown, used, expired, or another app's",
						)));
					}
				}
			}
			_ => None,
		};
		let request = match &pushed {
			Some((_, pushed)) => pushed,
			None => request,
		};

		let Some(redirect_uri) = request
			.redirect_uri
			.as_deref()
			.filter(|uri| redirect_matches(&client.redirect_uris, uri))
			.map(str::to_owned)
		else {
			return Ok(AuthorizeOutcome::Invalid(IdpError::invalid_request(
				"redirect_uri must be exactly one the app registered",
			)));
		};

		// From here on, errors go back to the app.
		let pushed_id = pushed.as_ref().map(|(id, _)| *id);
		let checked = match self.check(request, client, redirect_uri.clone(), pushed_id) {
			Ok(checked) => checked,
			Err(error) => {
				return Ok(AuthorizeOutcome::Redirect(self.error_redirect(
					&redirect_uri,
					&error,
					request.state.as_deref(),
				)));
			}
		};
		let error = |code: &'static str, description: &str| {
			Ok(AuthorizeOutcome::Redirect(self.error_redirect(
				&checked.redirect_uri,
				&IdpError::oauth(code, description),
				request.state.as_deref(),
			)))
		};

		let Some(me) = signed_in else {
			if checked.prompt == Prompt::None {
				return error("login_required", "the person isn't signed in");
			}
			return self.sign_in_redirect(original, &checked).await;
		};

		let elapsed = (leptoskit_db::now() - me.authenticated_at).num_seconds();
		let stale = checked.prompt == Prompt::Login
			|| checked.max_age.is_some_and(|max_age| elapsed > max_age);

		if stale {
			if checked.prompt == Prompt::None {
				return error("login_required", "the person must sign in again");
			}
			return self.sign_in_redirect(original, &checked).await;
		}

		if checked
			.claims
			.subject
			.as_ref()
			.is_some_and(|wanted| *wanted != crate::tokens::subject(&checked.client, me.account))
		{
			return error(
				"login_required",
				"someone else is signed in than the app asked for",
			);
		}

		let granted = Grant::query()
			.filter(|grant| grant.account_id.eq(me.account) & grant.client_id.eq(checked.client.id))
			.first(&self.inner.db)
			.await?;
		let covered = granted.as_ref().is_some_and(|grant| {
			checked
				.scopes
				.iter()
				.all(|scope| grant.scopes.contains(scope))
		});
		let consented =
			checked.client.first_party || (covered && checked.prompt != Prompt::Consent);

		if !consented && checked.prompt == Prompt::None {
			return error(
				"consent_required",
				"the person hasn't let this app do all of that",
			);
		}

		let tx = self.inner.db.begin().await?;
		let result = self
			.start(&tx, &checked, request, me, consented, info)
			.await;
		let outcome = crate::tokens::finish(tx, result).await?;

		Ok(AuthorizeOutcome::Redirect(outcome))
	}

	/// Records the request: as a code, when consent is already given, or
	/// for the consent page.
	async fn start(
		&self,
		tx: &Tx,
		checked: &Checked,
		request: &AuthorizeRequest,
		me: &SignedIn,
		consented: bool,
		info: &RequestInfo,
	) -> Result<String> {
		let now = leptoskit_db::now();
		self.forget_expired(tx).await?;

		if let Some(pushed) = checked.pushed {
			// A pushed request works once.
			PushedRequest::query()
				.filter(|row| row.id.eq(pushed))
				.delete(tx)
				.await?;
		}

		if consented {
			self.grant(tx, me, &checked.client, &checked.scopes, info)
				.await?;
			let code = secrets::token()?;
			NewAuthorization {
				client_id: checked.client.id,
				account_id: me.account,
				redirect_uri: checked.redirect_uri.clone(),
				scopes: Json(checked.scopes.clone()),
				state: request.state.clone(),
				nonce: request.nonce.clone(),
				id_token_claims: Json(checked.claims.id_token.clone()),
				code_challenge: checked.code_challenge.clone(),
				auth_time: me.authenticated_at,
				consent_hash: None,
				code_hash: Some(secrets::hash(&code)),
				expires_at: now + Self::lifetime(self.inner.lifetimes.code)?,
				used_at: None,
				family_id: None,
			}
			.insert(tx)
			.await?;
			return Ok(self.code_redirect(&checked.redirect_uri, &code, request.state.as_deref()));
		}

		let handle = secrets::token()?;
		NewAuthorization {
			client_id: checked.client.id,
			account_id: me.account,
			redirect_uri: checked.redirect_uri.clone(),
			scopes: Json(checked.scopes.clone()),
			state: request.state.clone(),
			nonce: request.nonce.clone(),
			id_token_claims: Json(checked.claims.id_token.clone()),
			code_challenge: checked.code_challenge.clone(),
			auth_time: me.authenticated_at,
			consent_hash: Some(secrets::hash(&handle)),
			code_hash: None,
			expires_at: now + Self::lifetime(self.inner.lifetimes.consent)?,
			used_at: None,
			family_id: None,
		}
		.insert(tx)
		.await?;

		Ok(with_query(
			&self.inner.consent_page,
			&[("request", handle.as_str())],
		))
	}

	/// What the consent page shows for `request`.
	///
	/// # Errors
	///
	/// [`IdpError::NotFound`] when the request is unknown, answered, expired,
	/// or someone else's; or when the database fails.
	pub async fn consent_request(&self, me: &SignedIn, request: &str) -> Result<ConsentRequest> {
		let (authorization, client) = self.pending(me, request).await?;
		let redirect_host = Url::parse(&authorization.redirect_uri)
			.ok()
			.and_then(|url| url.host_str().map(str::to_owned))
			.unwrap_or_else(|| authorization.redirect_uri.clone());

		Ok(ConsentRequest {
			app: AppInfo {
				client_id: client.client_id,
				name: client.name,
			},
			scopes: self.describe(&authorization.scopes),
			redirect_host,
		})
	}

	/// Answers a consent request, and returns where to send the browser:
	/// back to the app with a code, or with `access_denied`.
	///
	/// # Errors
	///
	/// As [`Idp::consent_request`].
	pub async fn decide(
		&self,
		me: &SignedIn,
		request: &str,
		approve: bool,
		info: &RequestInfo,
	) -> Result<String> {
		let (authorization, client) = self.pending(me, request).await?;
		let tx = self.inner.db.begin().await?;
		let result = async {
			let now = leptoskit_db::now();
			let code = secrets::token()?;
			let mut update = Authorization::update()
				// Only if it's still waiting: a second answer finds nothing.
				.filter(|row| row.id.eq(authorization.id) & row.consent_hash.is_not_null())
				.set(|row| row.consent_hash, None);

			update = if approve {
				update
					.set(|row| row.code_hash, Some(secrets::hash(&code)))
					.set(
						|row| row.expires_at,
						now + Self::lifetime(self.inner.lifetimes.code)?,
					)
			} else {
				update.set(|row| row.used_at, Some(now))
			};

			if update.run(&tx).await? == 0 {
				return Err(IdpError::NotFound);
			}

			if !approve {
				return Ok(self.error_redirect(
					&authorization.redirect_uri,
					&IdpError::oauth("access_denied", "the person said no"),
					authorization.state.as_deref(),
				));
			}

			self.grant(&tx, me, &client, &authorization.scopes, info)
				.await?;
			Ok(self.code_redirect(
				&authorization.redirect_uri,
				&code,
				authorization.state.as_deref(),
			))
		}
		.await;

		crate::tokens::finish(tx, result).await
	}

	/// The waiting request `handle` names, if it's `me`'s and still live.
	async fn pending(&self, me: &SignedIn, handle: &str) -> Result<(Authorization, Client)> {
		let now = leptoskit_db::now();
		let authorization = Authorization::query()
			.filter(|row| {
				row.consent_hash.eq(Some(secrets::hash(handle)))
					& row.account_id.eq(me.account)
					& row.used_at.is_null()
					& row.expires_at.gt(now)
			})
			.first(&self.inner.db)
			.await?
			.ok_or(IdpError::NotFound)?;
		let client = Client::find(&self.inner.db, authorization.client_id)
			.await?
			.ok_or(IdpError::NotFound)?;
		Ok((authorization, client))
	}

	/// Records that `me` let `client` have `scopes`, on top of whatever they
	/// let it have before, and says so in their audit trail when that's
	/// more than before.
	pub(crate) async fn grant(
		&self,
		tx: &Tx,
		me: &SignedIn,
		client: &Client,
		scopes: &[String],
		info: &RequestInfo,
	) -> Result<()> {
		let now = leptoskit_db::now();
		let existing = Grant::query()
			.filter(|grant| grant.account_id.eq(me.account) & grant.client_id.eq(client.id))
			.first(tx)
			.await?;

		match existing {
			Some(grant) => {
				let mut all = grant.scopes.0.clone();
				for scope in scopes {
					if !all.contains(scope) {
						all.push(scope.clone());
					}
				}
				if all == grant.scopes.0 {
					return Ok(());
				}
				Grant::update()
					.filter(|row| row.id.eq(grant.id))
					.set(|row| row.scopes, Json(all))
					.set(|row| row.updated_at, now)
					.run(tx)
					.await?;
			}
			None => {
				NewGrant {
					account_id: me.account,
					client_id: client.id,
					scopes: Json(scopes.to_vec()),
					updated_at: now,
					last_used_at: None,
				}
				.insert(tx)
				.await?;
			}
		}

		self.inner
			.auth
			.record(
				tx,
				Some(me.account),
				EventKind::AppConnected,
				Some(&client.name),
				info,
			)
			.await?;
		Ok(())
	}

	/// Clears requests and codes that expired a day ago: a used code is kept
	/// that long, so replaying it still ends the tokens it started.
	async fn forget_expired(&self, tx: &Tx) -> Result<()> {
		let cutoff = leptoskit_db::now() - chrono::TimeDelta::days(1);
		Authorization::query()
			.filter(|row| row.expires_at.lt(cutoff))
			.delete(tx)
			.await?;
		Ok(())
	}

	/// Checks a request as `/oauth/authorize` would, before anyone signs in.
	pub(crate) fn check_request(
		&self,
		request: &AuthorizeRequest,
		client: Client,
		redirect_uri: String,
	) -> Result<()> {
		self.check(request, client, redirect_uri, None).map(|_| ())
	}

	/// Checks everything but who's signed in.
	fn check(
		&self,
		request: &AuthorizeRequest,
		client: Client,
		redirect_uri: String,
		pushed: Option<Id<PushedRequest>>,
	) -> Result<Checked> {
		if request.request.is_some() {
			return Err(IdpError::oauth(
				"request_not_supported",
				"request objects aren't supported; push the request to /oauth/par",
			));
		}
		if pushed.is_none() && request.request_uri.is_some() {
			return Err(IdpError::oauth(
				"request_uri_not_supported",
				"only request_uri values from /oauth/par are supported",
			));
		}
		if request.response_type.as_deref() != Some("code") {
			return Err(IdpError::oauth(
				"unsupported_response_type",
				"only the authorization code flow is supported: response_type=code",
			));
		}
		if request
			.response_mode
			.as_deref()
			.is_some_and(|mode| mode != "query")
		{
			return Err(IdpError::invalid_request(
				"only response_mode=query is supported",
			));
		}

		let code_challenge = request
			.code_challenge
			.clone()
			.filter(|challenge| secrets::is_pkce_value(challenge))
			.ok_or_else(|| IdpError::invalid_request("PKCE is required: send a code_challenge"))?;
		if request.code_challenge_method.as_deref() != Some("S256") {
			return Err(IdpError::invalid_request(
				"code_challenge_method must be S256",
			));
		}

		if [&request.state, &request.nonce]
			.into_iter()
			.flatten()
			.any(|value| value.len() > MAX_ECHOED)
		{
			return Err(IdpError::invalid_request("state and nonce are too long"));
		}

		let mut scopes: Vec<String> = Vec::new();
		for scope in request.scope.as_deref().unwrap_or_default().split(' ') {
			if !scope.is_empty() && !scopes.iter().any(|known| known == scope) {
				scopes.push(scope.to_owned());
			}
		}
		if scopes.is_empty() {
			return Err(IdpError::invalid_scope("ask for at least one scope"));
		}
		if let Some(scope) = scopes
			.iter()
			.find(|scope| !client.scopes.contains(*scope) || !self.knows_scope(scope))
		{
			return Err(IdpError::invalid_scope(format!(
				"this app may not ask for `{scope}`"
			)));
		}

		let prompts: Vec<&str> = request
			.prompt
			.as_deref()
			.unwrap_or_default()
			.split(' ')
			.filter(|prompt| !prompt.is_empty())
			.collect();
		let prompt = match prompts.as_slice() {
			[] => Prompt::Default,
			["none"] => Prompt::None,
			prompts if prompts.contains(&"none") => {
				return Err(IdpError::invalid_request(
					"prompt=none can't be combined with another prompt",
				));
			}
			prompts if prompts.contains(&"login") => Prompt::Login,
			prompts if prompts.contains(&"consent") => Prompt::Consent,
			_ => Prompt::Default,
		};
		let max_age = match request.max_age.as_deref() {
			None => None,
			Some(value) => {
				Some(
					value
						.parse::<i64>()
						.ok()
						.filter(|seconds| *seconds >= 0)
						.ok_or_else(|| {
							IdpError::invalid_request("max_age must be a number of seconds")
						})?,
				)
			}
		};

		let claims = requested_claims(request.claims.as_deref())?;

		Ok(Checked {
			client,
			redirect_uri,
			scopes,
			code_challenge,
			prompt,
			max_age,
			claims,
			pushed,
		})
	}

	/// Your sign-in page, returning to this same request once signed in.
	/// `prompt=login` becomes a five-minute `max_age`, so the request comes
	/// back satisfied rather than asking again forever. A pushed request
	/// keeps travelling by its `request_uri`, changed the same way.
	async fn sign_in_redirect(
		&self,
		request: &AuthorizeRequest,
		checked: &Checked,
	) -> Result<AuthorizeOutcome> {
		let login = checked.prompt == Prompt::Login;
		let mut retry = request.clone();

		if let Some(pushed) = checked.pushed {
			if login {
				self.settle_pushed_prompt(pushed).await?;
			}
			retry = AuthorizeRequest {
				client_id: request.client_id.clone(),
				request_uri: request.request_uri.clone(),
				..AuthorizeRequest::default()
			};
		} else if login {
			retry.prompt = None;
			retry.max_age = Some("300".into());
		}

		Ok(AuthorizeOutcome::Redirect(self.sign_in_url(&retry)))
	}

	fn sign_in_url(&self, retry: &AuthorizeRequest) -> String {
		let query = url::form_urlencoded::Serializer::new(String::new())
			.extend_pairs(
				serde_json::to_value(retry)
					.ok()
					.and_then(|value| value.as_object().cloned())
					.unwrap_or_default()
					.iter()
					.filter_map(|(key, value)| {
						value.as_str().map(|value| (key.clone(), value.to_owned()))
					}),
			)
			.finish();
		let return_to = format!("/oauth/authorize?{query}");

		with_query(
			&self.inner.sign_in_page,
			&[("return_to", return_to.as_str())],
		)
	}

	/// The app's redirect URI with a code, `state`, and `iss` (RFC 9207).
	fn code_redirect(&self, redirect_uri: &str, code: &str, state: Option<&str>) -> String {
		let mut pairs = vec![("code", code)];
		if let Some(state) = state {
			pairs.push(("state", state));
		}
		pairs.push(("iss", self.inner.issuer.as_str()));
		with_query(redirect_uri, &pairs)
	}

	/// The app's redirect URI with an error, `state`, and `iss`.
	pub(crate) fn error_redirect(
		&self,
		redirect_uri: &str,
		error: &IdpError,
		state: Option<&str>,
	) -> String {
		let description = error.description();
		let mut pairs = vec![
			("error", error.code()),
			("error_description", description.as_str()),
		];
		if let Some(state) = state {
			pairs.push(("state", state));
		}
		pairs.push(("iss", self.inner.issuer.as_str()));
		with_query(redirect_uri, &pairs)
	}
}

/// Reads the `claims` parameter: a JSON object whose `id_token` and
/// `userinfo` members name claims, each `null` or an object with
/// `essential`, `value` or `values` (OpenID Connect Core §5.5).
///
/// `userinfo` always returns every claim the scopes release, so only the
/// `id_token` names are kept, and a requested `sub` value.
pub(crate) fn requested_claims(claims: Option<&str>) -> Result<RequestedClaims> {
	let Some(claims) = claims.filter(|claims| !claims.is_empty()) else {
		return Ok(RequestedClaims::default());
	};
	let malformed = || IdpError::invalid_request("claims must be a JSON object of claim requests");

	if claims.len() > MAX_CLAIMS {
		return Err(IdpError::invalid_request("claims is too long"));
	}

	let value: Value = serde_json::from_str(claims).map_err(|_| malformed())?;
	let object = value.as_object().ok_or_else(malformed)?;
	let mut requested = RequestedClaims::default();

	for (target, members) in object {
		let members = members.as_object().ok_or_else(malformed)?;
		for (name, request) in members {
			let valid = match request {
				Value::Null => true,
				Value::Object(request) => {
					request.get("essential").is_none_or(Value::is_boolean)
						&& request.get("values").is_none_or(Value::is_array)
				}
				_ => false,
			};
			if !valid {
				return Err(malformed());
			}
			if target == "id_token" {
				if name == "sub" {
					requested.subject = request
						.get("value")
						.and_then(Value::as_str)
						.map(str::to_owned);
				}
				if !requested.id_token.contains(name) {
					requested.id_token.push(name.clone());
				}
			}
		}
	}

	Ok(requested)
}

/// `base` with `pairs` added to its query, keeping any it has.
pub(crate) fn with_query(base: &str, pairs: &[(&str, &str)]) -> String {
	let query = url::form_urlencoded::Serializer::new(String::new())
		.extend_pairs(pairs)
		.finish();
	let separator = if base.contains('?') { '&' } else { '?' };
	format!("{base}{separator}{query}")
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn the_claims_parameter_names_id_token_claims() {
		assert_eq!(requested_claims(None).unwrap(), RequestedClaims::default());
		let claims = requested_claims(Some(
			r#"{"id_token":{"email":{"essential":true},"name":null,"sub":{"value":"abc"}},"userinfo":{"picture":null}}"#,
		))
		.unwrap();
		assert_eq!(claims.id_token, ["email", "name", "sub"]);
		assert_eq!(claims.subject.as_deref(), Some("abc"));

		for malformed in [
			"[]",
			"not json",
			r#"{"id_token":[]}"#,
			r#"{"id_token":{"email":true}}"#,
			r#"{"id_token":{"email":{"essential":"yes"}}}"#,
			r#"{"userinfo":{"email":{"values":"a"}}}"#,
		] {
			assert!(requested_claims(Some(malformed)).is_err(), "{malformed}");
		}
		assert!(requested_claims(Some(&"x".repeat(MAX_CLAIMS + 1))).is_err());
	}

	#[test]
	fn queries_are_added_to_what_is_there() {
		assert_eq!(
			with_query("https://a.example/cb", &[("code", "x y"), ("state", "s&t")]),
			"https://a.example/cb?code=x+y&state=s%26t"
		);
		assert_eq!(
			with_query("https://a.example/cb?app=1", &[("code", "c")]),
			"https://a.example/cb?app=1&code=c"
		);
	}
}
