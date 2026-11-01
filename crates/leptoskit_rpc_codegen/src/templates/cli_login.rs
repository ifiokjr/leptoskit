//! Signing in for the commands that need it: the device flow (RFC 8628)
//! against the app's own sign-in. `login` shows a code to enter in a
//! browser, and keeps the tokens in a file only you can read; each command
//! then sends the access token, renewing it when it runs out.

use std::path::PathBuf;
use std::time::Duration;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

use serde::Deserialize;
use serde::Serialize;

/// The CLI's name: where its tokens are kept, and what to run.
const APP: &str = "__APP__";
/// The scopes `login` asks for: every one a command needs.
const SCOPES: &str = "__SCOPES__";
/// The environment variable naming the app's client ID, when `--client-id`
/// doesn't.
const CLIENT_ID_ENV: &str = "__CLIENT_ID_ENV__";
/// How long before it expires an access token is renewed.
const EARLY: u64 = 30;

#[derive(Deserialize)]
struct Discovery {
	token_endpoint: String,
	#[serde(default)]
	device_authorization_endpoint: Option<String>,
	#[serde(default)]
	revocation_endpoint: Option<String>,
}

#[derive(Deserialize)]
struct DeviceAuthorization {
	device_code: String,
	user_code: String,
	verification_uri: String,
	#[serde(default)]
	verification_uri_complete: Option<String>,
	expires_in: u64,
	#[serde(default)]
	interval: Option<u64>,
}

#[derive(Deserialize)]
struct Tokens {
	access_token: String,
	expires_in: u64,
	#[serde(default)]
	refresh_token: Option<String>,
}

#[derive(Deserialize)]
struct OAuthError {
	error: String,
	#[serde(default)]
	error_description: Option<String>,
}

/// What `login` keeps between commands. Never printed.
#[derive(Serialize, Deserialize)]
struct Credentials {
	server_url: String,
	client_id: String,
	token_endpoint: String,
	#[serde(default)]
	revocation_endpoint: Option<String>,
	access_token: String,
	#[serde(default)]
	refresh_token: Option<String>,
	expires_at: u64,
}

/// What a token endpoint answered.
enum Answer {
	Tokens(Tokens),
	Refused(OAuthError),
}

fn now() -> u64 {
	SystemTime::now()
		.duration_since(UNIX_EPOCH)
		.map_or(0, |elapsed| elapsed.as_secs())
}

/// The directory the CLI keeps its tokens in.
fn directory() -> Result<PathBuf, String> {
	let base = std::env::var_os("XDG_CONFIG_HOME")
		.filter(|value| !value.is_empty())
		.map(PathBuf::from)
		.or_else(|| {
			if cfg!(windows) {
				std::env::var_os("APPDATA").map(PathBuf::from)
			} else {
				std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config"))
			}
		})
		.ok_or("can't tell where to keep the sign-in: set XDG_CONFIG_HOME or HOME")?;
	Ok(base.join(APP))
}

fn credentials_path() -> Result<PathBuf, String> {
	Ok(directory()?.join("credentials.json"))
}

fn load() -> Result<Option<Credentials>, String> {
	let path = credentials_path()?;

	match std::fs::read(&path) {
		Ok(bytes) => {
			serde_json::from_slice(&bytes)
				.map(Some)
				.map_err(|_| format!("{} is damaged; run `{APP} logout`", path.display()))
		}

		Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
		Err(error) => Err(format!("couldn't read {}: {error}", path.display())),
	}
}

/// Writes the credentials readable only by you, replacing the old file in
/// one step.
fn save(credentials: &Credentials) -> Result<(), String> {
	let directory = directory()?;
	std::fs::create_dir_all(&directory)
		.map_err(|error| format!("couldn't create {}: {error}", directory.display()))?;
	let path = directory.join("credentials.json");
	let staged = directory.join(format!("credentials.{}.json", std::process::id()));
	let json = serde_json::to_vec(credentials).map_err(|error| error.to_string())?;
	let mut options = std::fs::OpenOptions::new();
	options.write(true).create(true).truncate(true);
	#[cfg(unix)]
	{
		use std::os::unix::fs::OpenOptionsExt;
		options.mode(0o600);
	}
	let written = options
		.open(&staged)
		.and_then(|mut file| std::io::Write::write_all(&mut file, &json))
		.and_then(|()| std::fs::rename(&staged, &path));
	written.map_err(|error| {
		let _ = std::fs::remove_file(&staged);
		format!("couldn't write {}: {error}", path.display())
	})
}

fn forget() -> Result<(), String> {
	let path = credentials_path()?;

	match std::fs::remove_file(&path) {
		Ok(()) => Ok(()),

		Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
		Err(error) => Err(format!("couldn't remove {}: {error}", path.display())),
	}
}

/// Holds the renewal lock: two commands renewing at once would each spend
/// the same refresh token, and the second use signs you out.
struct Lock(PathBuf);

impl Lock {
	async fn take() -> Result<Self, String> {
		let directory = directory()?;
		std::fs::create_dir_all(&directory)
			.map_err(|error| format!("couldn't create {}: {error}", directory.display()))?;
		let path = directory.join("credentials.lock");

		for _ in 0..100 {
			match std::fs::OpenOptions::new()
				.write(true)
				.create_new(true)
				.open(&path)
			{
				Ok(_) => return Ok(Self(path)),

				Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
					// A lock left by a command that crashed is stale after 30
					// s.
					let stale = std::fs::metadata(&path)
						.and_then(|metadata| metadata.modified())
						.ok()
						.and_then(|modified| modified.elapsed().ok())
						.is_some_and(|age| age > Duration::from_secs(30));
					if stale {
						let _ = std::fs::remove_file(&path);
					} else {
						tokio::time::sleep(Duration::from_millis(100)).await;
					}
				}

				Err(error) => return Err(format!("couldn't lock {}: {error}", path.display())),
			}
		}

		Err(format!(
			"another `{APP}` command is renewing the sign-in; try again"
		))
	}
}

impl Drop for Lock {
	fn drop(&mut self) {
		let _ = std::fs::remove_file(&self.0);
	}
}

async fn discover(http: &reqwest::Client, server_url: &str) -> Result<Discovery, String> {
	let url = format!("{server_url}/.well-known/openid-configuration");
	let response = http
		.get(&url)
		.send()
		.await
		.map_err(|error| format!("couldn't reach {url}: {error}"))?;

	if !response.status().is_success() {
		return Err(format!(
			"{server_url} doesn't offer sign-in ({})",
			response.status()
		));
	}

	response
		.json()
		.await
		.map_err(|error| format!("{url} isn't a discovery document: {error}"))
}

async fn post(
	http: &reqwest::Client,
	url: &str,
	form: &[(&str, &str)],
) -> Result<reqwest::Response, String> {
	let body = serde_qs::to_string(
		&form
			.iter()
			.copied()
			.collect::<std::collections::BTreeMap<_, _>>(),
	)
	.map_err(|error| error.to_string())?;
	http.post(url)
		.header(
			reqwest::header::CONTENT_TYPE,
			"application/x-www-form-urlencoded",
		)
		.header(reqwest::header::ACCEPT, "application/json")
		.body(body)
		.send()
		.await
		.map_err(|error| format!("couldn't reach {url}: {error}"))
}

async fn token(http: &reqwest::Client, url: &str, form: &[(&str, &str)]) -> Result<Answer, String> {
	let response = post(http, url, form).await?;
	let success = response.status().is_success();
	let bytes = response.bytes().await.map_err(|error| error.to_string())?;

	if success {
		serde_json::from_slice(&bytes)
			.map(Answer::Tokens)
			.map_err(|_| "the server's tokens didn't make sense".to_owned())
	} else {
		serde_json::from_slice(&bytes)
			.map(Answer::Refused)
			.map_err(|_| "the server refused without saying why".to_owned())
	}
}

fn describe(error: &OAuthError) -> String {
	match &error.error_description {
		Some(description) => format!("{}: {description}", error.error),
		None => error.error.clone(),
	}
}

/// `login`: shows a code to enter in a browser, waits for you to approve
/// it, and keeps the tokens.
pub async fn login(server_url: &str, client_id: Option<String>) -> Result<String, String> {
	let client_id = registered_client_id(client_id)?;
	let http = reqwest::Client::new();
	let discovery = discover(&http, server_url).await?;
	let device_endpoint = discovery
		.device_authorization_endpoint
		.as_deref()
		.ok_or_else(|| format!("{server_url} doesn't offer sign-in for devices"))?;
	let device = start_device(&http, device_endpoint, &client_id).await?;

	eprintln!(
		"To sign in, open {} and enter the code {}",
		device.verification_uri, device.user_code
	);

	if let Some(complete) = &device.verification_uri_complete {
		eprintln!("(or open {complete})");
	}

	wait_for_approval(&http, &discovery, &client_id, &device, server_url).await
}

/// The client ID from the flag or the environment, or what to do about it.
fn registered_client_id(client_id: Option<String>) -> Result<String, String> {
	client_id
		.or_else(|| std::env::var(CLIENT_ID_ENV).ok())
		.filter(|id| !id.is_empty())
		.ok_or_else(|| {
			format!(
				"pass --client-id, or set {CLIENT_ID_ENV}: the client ID of this CLI, registered \
				 with `lk auth idp client add \"{APP}\" --public --grant device --scopes {}`",
				SCOPES.replace(' ', ",")
			)
		})
}

/// Asks the server for a device code to show.
async fn start_device(
	http: &reqwest::Client,
	endpoint: &str,
	client_id: &str,
) -> Result<DeviceAuthorization, String> {
	let response = post(
		http,
		endpoint,
		&[("client_id", client_id), ("scope", SCOPES)],
	)
	.await?;

	if !response.status().is_success() {
		let error: OAuthError = response
			.json()
			.await
			.map_err(|_| "the server refused to start signing in".to_owned())?;

		return Err(describe(&error));
	}

	response
		.json()
		.await
		.map_err(|_| "the server's answer didn't make sense".to_owned())
}

/// Waits for the person to approve the code, and keeps the tokens.
async fn wait_for_approval(
	http: &reqwest::Client,
	discovery: &Discovery,
	client_id: &str,
	device: &DeviceAuthorization,
	server_url: &str,
) -> Result<String, String> {
	let mut interval = device.interval.unwrap_or(5).max(1);
	let deadline = now() + device.expires_in;

	while now() < deadline {
		tokio::time::sleep(Duration::from_secs(interval)).await;
		let answer = token(
			http,
			&discovery.token_endpoint,
			&[
				("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
				("device_code", &device.device_code),
				("client_id", client_id),
			],
		)
		.await?;

		match answer {
			Answer::Tokens(tokens) => {
				save(&Credentials {
					server_url: server_url.to_owned(),
					client_id: client_id.to_owned(),
					token_endpoint: discovery.token_endpoint.clone(),
					revocation_endpoint: discovery.revocation_endpoint.clone(),
					access_token: tokens.access_token,
					refresh_token: tokens.refresh_token,
					expires_at: now() + tokens.expires_in,
				})?;
				return Ok(format!("Signed in to {server_url}."));
			}

			Answer::Refused(error) if error.error == "authorization_pending" => {}

			Answer::Refused(error) if error.error == "slow_down" => interval += 5,

			Answer::Refused(error) if error.error == "access_denied" => {
				return Err("the sign-in was refused".to_owned());
			}
			Answer::Refused(error) => return Err(describe(&error)),
		}
	}

	Err(format!("the code expired; run `{APP} login` again"))
}
/// `logout`: revokes the tokens, and forgets them.
pub async fn logout(server_url: &str) -> Result<String, String> {
	let Some(credentials) = load()? else {
		return Ok("You weren't signed in.".to_owned());
	};

	if let (Some(revocation), Some(refresh)) =
		(&credentials.revocation_endpoint, &credentials.refresh_token)
	{
		// Forgotten either way; a server that can't be reached ends the
		// sign-in when its tokens expire.
		let _ = post(
			&reqwest::Client::new(),
			revocation,
			&[("token", refresh), ("client_id", &credentials.client_id)],
		)
		.await;
	}
	forget()?;
	Ok(format!(
		"Signed out of {}.",
		if credentials.server_url.is_empty() {
			server_url
		} else {
			&credentials.server_url
		}
	))
}

/// The `Authorization` header for a command, renewing the access token
/// first when it's about to run out. `None` when nobody signed in to this
/// server, so the command goes without.
pub async fn authorization(server_url: &str) -> Result<Option<String>, String> {
	let Some(credentials) = load()? else {
		return Ok(None);
	};

	if credentials.server_url != server_url {
		return Ok(None);
	}

	if now() + EARLY < credentials.expires_at {
		return Ok(Some(format!("Bearer {}", credentials.access_token)));
	}

	let _lock = Lock::take().await?;
	// Another command may have renewed it while this one waited.
	let Some(credentials) = load()? else {
		return Ok(None);
	};

	if now() + EARLY < credentials.expires_at {
		return Ok(Some(format!("Bearer {}", credentials.access_token)));
	}

	let Some(refresh) = credentials.refresh_token.clone() else {
		forget()?;

		return Err(format!("the sign-in expired; run `{APP} login`"));
	};

	let answer = token(
		&reqwest::Client::new(),
		&credentials.token_endpoint,
		&[
			("grant_type", "refresh_token"),
			("refresh_token", &refresh),
			("client_id", &credentials.client_id),
		],
	)
	.await?;

	match answer {
		Answer::Tokens(tokens) => {
			let header = format!("Bearer {}", tokens.access_token);
			save(&Credentials {
				access_token: tokens.access_token,
				refresh_token: tokens.refresh_token.or(Some(refresh)),
				expires_at: now() + tokens.expires_in,
				..credentials
			})?;
			Ok(Some(header))
		}
		Answer::Refused(error) => {
			forget()?;
			Err(format!(
				"you've been signed out ({}); run `{APP} login`",
				describe(&error)
			))
		}
	}
}
