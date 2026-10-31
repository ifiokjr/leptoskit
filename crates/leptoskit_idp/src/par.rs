//! Pushed authorization requests (RFC 9126): an app sends its
//! authorization request to `/oauth/par` from its server, authenticated,
//! and sends the browser to `/oauth/authorize` with only a handle. The
//! parameters never pass through the address bar, can't be changed on the
//! way, and are checked before anyone is asked to sign in.

use leptoskit_db::Id;
use leptoskit_db::Json;

use crate::AuthorizeRequest;
use crate::Client;
use crate::ClientCredentials;
use crate::GrantType;
use crate::Idp;
use crate::IdpError;
use crate::NewPushedRequest;
use crate::PushedAuthorization;
use crate::PushedRequest;
use crate::Result;
use crate::clients::redirect_matches;
use crate::secrets;

/// What every `request_uri` from `/oauth/par` starts with.
pub(crate) const REQUEST_URI_PREFIX: &str = "urn:ietf:params:oauth:request_uri:";

impl Idp {
	/// Answers `POST /oauth/par`: checks an authorization request from an
	/// app's server, keeps it, and returns the `request_uri` that stands for
	/// it at `/oauth/authorize`. It works once, and expires as
	/// [`Lifetimes::pushed_request`](crate::Lifetimes::pushed_request) says.
	///
	/// # Errors
	///
	/// `invalid_client` when the app doesn't authenticate;
	/// `unauthorized_client` when it isn't registered for the code flow; any
	/// error `/oauth/authorize` would send back, such as `invalid_scope`; or
	/// when the database fails.
	pub async fn push_request(
		&self,
		credentials: &ClientCredentials,
		request: &AuthorizeRequest,
	) -> Result<PushedAuthorization> {
		let client = self.authenticate(credentials).await?;

		if !client.grant_types.contains(&GrantType::AuthorizationCode) {
			return Err(IdpError::oauth(
				"unauthorized_client",
				"this app isn't registered for the authorization code flow",
			));
		}
		if request.request_uri.is_some() {
			return Err(IdpError::invalid_request(
				"a pushed request can't name another request_uri",
			));
		}
		if request
			.client_id
			.as_deref()
			.is_some_and(|client_id| client_id != client.client_id)
		{
			return Err(IdpError::invalid_request(
				"client_id must be the app that authenticated",
			));
		}

		let redirect_uri = request
			.redirect_uri
			.as_deref()
			.filter(|uri| redirect_matches(&client.redirect_uris, uri))
			.ok_or_else(|| {
				IdpError::invalid_request("redirect_uri must be exactly one the app registered")
			})?
			.to_owned();
		let (id, client_id) = (client.id, client.client_id.clone());
		// Everything `/oauth/authorize` would refuse is refused now, while the
		// app's server can still hear why.
		self.check_request(request, client, redirect_uri)?;

		let now = leptoskit_db::now();
		let lifetime = Self::lifetime(self.inner.lifetimes.pushed_request)?;
		let handle = secrets::token()?;
		let stored = AuthorizeRequest {
			client_id: Some(client_id),
			..request.clone()
		};

		PushedRequest::query()
			.filter(|row| row.expires_at.lt(now))
			.delete(&self.inner.db)
			.await?;
		NewPushedRequest {
			client_id: id,
			handle_hash: secrets::hash(&handle),
			request: Json(
				serde_json::to_value(&stored)
					.map_err(|_| IdpError::invalid_request("the request can't be kept"))?,
			),
			expires_at: now + lifetime,
		}
		.insert(&self.inner.db)
		.await?;

		Ok(PushedAuthorization {
			request_uri: format!("{REQUEST_URI_PREFIX}{handle}"),
			expires_in: u64::try_from(lifetime.num_seconds()).unwrap_or_default(),
		})
	}

	/// The pushed request `uri` names, if it's `client`'s and still live.
	pub(crate) async fn pushed_request(
		&self,
		client: &Client,
		uri: &str,
	) -> Result<Option<(Id<PushedRequest>, AuthorizeRequest)>> {
		let Some(handle) = uri.strip_prefix(REQUEST_URI_PREFIX) else {
			return Ok(None);
		};
		let now = leptoskit_db::now();
		let pushed = PushedRequest::query()
			.filter(|row| {
				row.handle_hash.eq(secrets::hash(handle))
					& row.client_id.eq(client.id)
					& row.expires_at.gt(now)
			})
			.first(&self.inner.db)
			.await?;

		Ok(pushed.and_then(|pushed| {
			let request = serde_json::from_value(pushed.request.0).ok()?;
			Some((pushed.id, request))
		}))
	}

	/// A pushed `prompt=login` comes back from signing in as a five-minute
	/// `max_age`, so it doesn't ask again forever.
	pub(crate) async fn settle_pushed_prompt(&self, pushed: Id<PushedRequest>) -> Result<()> {
		let Some(row) = PushedRequest::find(&self.inner.db, pushed).await? else {
			return Ok(());
		};
		let Ok(mut request) = serde_json::from_value::<AuthorizeRequest>(row.request.0) else {
			return Ok(());
		};
		request.prompt = None;
		request.max_age = Some("300".into());

		if let Ok(request) = serde_json::to_value(&request) {
			PushedRequest::update()
				.filter(|candidate| candidate.id.eq(pushed))
				.set(|candidate| candidate.request, Json(request))
				.run(&self.inner.db)
				.await?;
		}
		Ok(())
	}
}
