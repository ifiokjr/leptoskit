//! Registered apps: adding, listing and removing them, and checking that a
//! request comes from the app it names.

use std::fmt;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use leptoskit_db::Json;
use url::Url;

use crate::Client;
use crate::ClientAuth;
use crate::ClientInfo;
use crate::ClientSecret;
use crate::Idp;
use crate::IdpError;
use crate::NewClient;
use crate::NewClientSpec;
use crate::Result;
use crate::secrets;

impl Idp {
	/// Registers an app. Returns it, and for a confidential app its secret,
	/// which is shown this once: only its hash is kept.
	///
	/// # Errors
	///
	/// [`IdpError::OAuth`] (`invalid_client_metadata`) when the name is
	/// blank, a redirect address isn't one an app may use, or a scope isn't
	/// offered; or when the database fails.
	pub async fn register_client(
		&self,
		spec: NewClientSpec,
	) -> Result<(ClientInfo, Option<ClientSecret>)> {
		let name = spec.name.trim().to_owned();

		if name.is_empty() || name.chars().count() > 100 {
			return Err(metadata("give the app a name of at most 100 characters"));
		}
		if spec.redirect_uris.is_empty() {
			return Err(metadata("give the app at least one redirect URI"));
		}
		for uri in spec
			.redirect_uris
			.iter()
			.chain(&spec.post_logout_redirect_uris)
		{
			check_redirect_uri(uri)?;
		}
		if spec.scopes.is_empty() {
			return Err(metadata("give the app at least one scope"));
		}
		for scope in &spec.scopes {
			if !self.knows_scope(scope) {
				return Err(metadata(format!("`{scope}` isn't a scope this app offers")));
			}
		}
		if spec.introspection && spec.auth_method == ClientAuth::None {
			return Err(metadata(
				"a public app can't introspect tokens; give it a secret",
			));
		}

		let secret = match spec.auth_method {
			ClientAuth::None => None,
			ClientAuth::SecretBasic | ClientAuth::SecretPost => Some(ClientSecret::generate()?),
		};
		let client = NewClient {
			client_id: secrets::identifier()?,
			name,
			auth_method: spec.auth_method,
			secret_hash: secret.as_ref().map(|secret| secrets::hash(secret.expose())),
			redirect_uris: Json(dedupe(spec.redirect_uris)),
			post_logout_redirect_uris: Json(dedupe(spec.post_logout_redirect_uris)),
			scopes: Json(dedupe(spec.scopes)),
			first_party: spec.first_party,
			introspection: spec.introspection,
			subject_salt: secrets::random::<32>()?.to_vec(),
		}
		.insert(&self.inner.db)
		.await?;

		Ok((ClientInfo::from(client), secret))
	}

	/// Every registered app, oldest first.
	///
	/// # Errors
	///
	/// When the database fails.
	pub async fn clients(&self) -> Result<Vec<ClientInfo>> {
		let clients = Client::query()
			.order_by(|client| client.created_at.asc())
			.all(&self.inner.db)
			.await?;
		Ok(clients.into_iter().map(ClientInfo::from).collect())
	}

	/// Removes an app, and with it every grant, code and token it holds.
	/// Returns whether there was one.
	///
	/// # Errors
	///
	/// When the database fails.
	pub async fn remove_client(&self, client_id: &str) -> Result<bool> {
		let removed = Client::query()
			.filter(|client| client.client_id.eq(client_id.to_owned()))
			.delete(&self.inner.db)
			.await?;
		Ok(removed > 0)
	}

	/// The app with this client ID.
	pub(crate) async fn client(&self, client_id: &str) -> Result<Option<Client>> {
		Ok(Client::query()
			.filter(|client| client.client_id.eq(client_id.to_owned()))
			.first(&self.inner.db)
			.await?)
	}

	/// The app a request authenticates as, checked against how it
	/// registered: its secret over Basic or in the body, or nothing for a
	/// public app.
	pub(crate) async fn authenticate(&self, credentials: &ClientCredentials) -> Result<Client> {
		let (client_id, secret, basic) = match (&credentials.basic, &credentials.client_id) {
			(Some(_), Some(_)) if credentials.client_secret.is_some() => {
				return Err(IdpError::invalid_request(
					"authenticate one way: Basic, or the body, not both",
				));
			}
			(Some((id, secret)), body_id) => {
				if body_id.as_ref().is_some_and(|body_id| body_id != id) {
					return Err(IdpError::invalid_client());
				}
				(id, Some(secret), true)
			}
			(None, Some(id)) => (id, credentials.client_secret.as_ref(), false),
			(None, None) => return Err(IdpError::invalid_client()),
		};
		let client = self
			.client(client_id)
			.await?
			.ok_or_else(IdpError::invalid_client)?;
		let secret_matches = |secret: Option<&String>| {
			match (secret, &client.secret_hash) {
				(Some(secret), Some(hash)) => secrets::matches(secret, hash),
				_ => false,
			}
		};
		let authenticated = match client.auth_method {
			ClientAuth::None => secret.is_none() && !basic,
			ClientAuth::SecretBasic => basic && secret_matches(secret),
			ClientAuth::SecretPost => !basic && secret_matches(secret),
		};

		if authenticated {
			Ok(client)
		} else {
			Err(IdpError::invalid_client())
		}
	}
}

/// How a request to the token, revocation or introspection endpoint says
/// which app it comes from. Its `Debug` leaves out the secret.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct ClientCredentials {
	basic: Option<(String, String)>,
	client_id: Option<String>,
	client_secret: Option<String>,
}

impl ClientCredentials {
	/// Credentials from the `Authorization` header, if any, and the
	/// `client_id` and `client_secret` form fields.
	///
	/// # Errors
	///
	/// `invalid_client` when the header isn't usable Basic credentials.
	pub fn new(
		authorization: Option<&str>,
		client_id: Option<String>,
		client_secret: Option<String>,
	) -> Result<Self> {
		let basic = authorization.map(parse_basic).transpose()?;
		Ok(Self {
			basic,
			client_id: client_id.filter(|id| !id.is_empty()),
			client_secret: client_secret.filter(|secret| !secret.is_empty()),
		})
	}

	/// Whether the app authenticated with HTTP Basic, which an
	/// `invalid_client` answer then challenges.
	pub fn used_basic(&self) -> bool {
		self.basic.is_some()
	}
}

impl fmt::Debug for ClientCredentials {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		let client_id = self
			.basic
			.as_ref()
			.map(|(id, _)| id)
			.or(self.client_id.as_ref());
		f.debug_struct("ClientCredentials")
			.field("client_id", &client_id)
			.field("basic", &self.basic.is_some())
			.finish_non_exhaustive()
	}
}

/// Basic credentials: `client_id:client_secret`, each form-urlencoded, then
/// base64 (RFC 6749 §2.3.1).
fn parse_basic(header: &str) -> Result<(String, String)> {
	let (scheme, encoded) = header
		.split_once(' ')
		.ok_or_else(IdpError::invalid_client)?;

	if !scheme.eq_ignore_ascii_case("basic") {
		return Err(IdpError::invalid_client());
	}

	let decoded = STANDARD
		.decode(encoded.trim())
		.ok()
		.and_then(|bytes| String::from_utf8(bytes).ok())
		.ok_or_else(IdpError::invalid_client)?;
	let (id, secret) = decoded
		.split_once(':')
		.ok_or_else(IdpError::invalid_client)?;
	let unescape = |text: &str| -> Result<String> {
		url::form_urlencoded::parse(format!("v={text}").as_bytes())
			.next()
			.map(|(_, value)| value.into_owned())
			.ok_or_else(IdpError::invalid_client)
	};

	Ok((unescape(id)?, unescape(secret)?))
}

/// An address an app may have people sent back to: an absolute URL with
/// no fragment, over `https`, over `http` only to this device (RFC 8252
/// §7.3), or with an app's own reverse-domain scheme (RFC 8252 §7.1).
fn check_redirect_uri(uri: &str) -> Result<()> {
	let url = Url::parse(uri).map_err(|_| metadata(format!("`{uri}` isn't an absolute URL")))?;
	let loopback = matches!(url.host_str(), Some("127.0.0.1" | "[::1]" | "localhost"));
	let allowed = match url.scheme() {
		"https" => url.host_str().is_some(),
		"http" => loopback,
		"javascript" | "data" | "file" | "about" | "blob" | "vbscript" => false,
		scheme => scheme.contains('.'),
	};

	if url.fragment().is_some() {
		return Err(metadata(format!("`{uri}` can't have a fragment")));
	}
	if !allowed {
		return Err(metadata(format!(
			"`{uri}` must be https, http to this device, or a reverse-domain app scheme such as \
			 `com.example.app:/callback`"
		)));
	}

	Ok(())
}

/// Whether `presented` is one of the app's registered redirect URIs:
/// exactly, except that a loopback address may use any port (RFC 8252
/// §7.3), since native apps can't know theirs in advance.
pub(crate) fn redirect_matches(registered: &[String], presented: &str) -> bool {
	if registered.iter().any(|uri| uri == presented) {
		return true;
	}

	let Ok(presented) = Url::parse(presented) else {
		return false;
	};

	if presented.scheme() != "http" || !matches!(presented.host_str(), Some("127.0.0.1" | "[::1]"))
	{
		return false;
	}

	registered
		.iter()
		.filter_map(|uri| Url::parse(uri).ok())
		.any(|mut uri| {
			uri.scheme() == "http"
				&& uri.host_str() == presented.host_str()
				&& uri.set_port(presented.port()).is_ok()
				&& uri == presented
		})
}

fn metadata(description: impl Into<String>) -> IdpError {
	IdpError::oauth("invalid_client_metadata", description)
}

fn dedupe(values: Vec<String>) -> Vec<String> {
	let mut unique: Vec<String> = Vec::with_capacity(values.len());
	for value in values {
		if !unique.contains(&value) {
			unique.push(value);
		}
	}
	unique
}

impl From<Client> for ClientInfo {
	fn from(client: Client) -> Self {
		Self {
			client_id: client.client_id,
			name: client.name,
			auth_method: client.auth_method,
			redirect_uris: client.redirect_uris.0,
			post_logout_redirect_uris: client.post_logout_redirect_uris.0,
			scopes: client.scopes.0,
			first_party: client.first_party,
			introspection: client.introspection,
			created_at: client.created_at,
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn redirect_uris_are_https_loopback_or_app_schemes() {
		for uri in [
			"https://partner.example/callback",
			"http://127.0.0.1/callback",
			"http://localhost:8080/cb",
			"com.partner.app:/oauth",
		] {
			assert!(check_redirect_uri(uri).is_ok(), "{uri}");
		}
		for uri in [
			"http://partner.example/callback",
			"https://partner.example/cb#frag",
			"javascript:alert(1)",
			"myapp:/callback",
			"/relative",
		] {
			assert!(check_redirect_uri(uri).is_err(), "{uri}");
		}
	}

	#[test]
	fn redirects_match_exactly_but_loopback_ports_vary() {
		let registered = vec![
			"https://partner.example/cb".to_owned(),
			"http://127.0.0.1/cb".to_owned(),
		];

		assert!(redirect_matches(&registered, "https://partner.example/cb"));
		assert!(!redirect_matches(
			&registered,
			"https://partner.example/cb/"
		));
		assert!(!redirect_matches(
			&registered,
			"https://partner.example/cb?x=1"
		));
		assert!(!redirect_matches(
			&registered,
			"https://partner.example:8443/cb"
		));
		assert!(redirect_matches(&registered, "http://127.0.0.1:51234/cb"));
		assert!(!redirect_matches(
			&registered,
			"http://127.0.0.1:51234/other"
		));
		assert!(!redirect_matches(&registered, "http://[::1]:51234/cb"));
		assert!(!redirect_matches(&registered, "not a url"));
	}

	#[test]
	fn basic_credentials_are_unescaped_and_redacted() {
		let header = format!("Basic {}", STANDARD.encode("my%20app:s%3Acret"));
		let credentials = ClientCredentials::new(Some(&header), None, None).unwrap();

		assert_eq!(
			credentials.basic,
			Some(("my app".to_owned(), "s:cret".to_owned()))
		);
		assert!(credentials.used_basic());
		assert!(!format!("{credentials:?}").contains("cret"));

		for header in [
			"Bearer abc",
			"Basic !!!",
			"Basic",
			&format!("Basic {}", STANDARD.encode("nocolon")),
		] {
			assert!(
				ClientCredentials::new(Some(header), None, None).is_err(),
				"{header}"
			);
		}
	}
}
