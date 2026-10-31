//! The app every flow test signs in to: notes, with an `idp` that other
//! apps sign in through, served from the real routes and layers in memory.

#![allow(
	dead_code,
	reason = "each test binary uses its own part of the harness"
)]
#![allow(
	clippy::unused_async,
	reason = "`#[server]` functions are async by contract"
)]

#[path = "../../../leptoskit_db/tests/support/postgres.rs"]
pub mod postgres;

use axum::Extension;
use axum::Router;
use axum::body::Body;
use axum::http::Request;
use axum::http::StatusCode;
use axum::http::header;
use axum::response::Response;
use axum::routing::get;
use axum::routing::post;
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use http_body_util::BodyExt;
use leptos::prelude::ServerFnError;
use leptoskit_auth::Account;
use leptoskit_auth::Auth;
use leptoskit_auth::AuthContext;
use leptoskit_auth::CurrentSession;
use leptoskit_auth::Keys;
use leptoskit_auth::Mailbox;
use leptoskit_auth::OnSignUp;
use leptoskit_auth::Profile;
use leptoskit_auth::SignedIn;
use leptoskit_db::Db;
use leptoskit_db::Id;
use leptoskit_db::Tx;
use leptoskit_db::policy;
use leptoskit_db::table;
use leptoskit_idp::ClaimSet;
use leptoskit_idp::Claims;
use leptoskit_idp::ClientAuth;
use leptoskit_idp::ClientCommand;
use leptoskit_idp::ClientCommandOutput;
use leptoskit_idp::ConsentRedirect;
use leptoskit_idp::GrantedScopes;
use leptoskit_idp::Idp;
use leptoskit_idp::IdpBuilder;
use leptoskit_idp::Introspection;
use leptoskit_idp::NewClientSpec;
use leptoskit_idp::OAuthError;
use p256::ecdsa::Signature;
use p256::ecdsa::VerifyingKey;
use p256::ecdsa::signature::Verifier;
use postgres::PostgresCluster;
use serde_json::Value;
use serde_json::json;
use sha2::Digest;
use tower::ServiceExt;
use url::Url;

pub const ORIGIN: &str = "https://notes.example";
pub const CALLBACK: &str = "https://partner.example/callback";
pub const SIGNED_OUT: &str = "https://partner.example/signed-out";
pub const VERIFIER: &str = "a-verifier-of-at-least-forty-three-characters-0123456789";

leptoskit_idp::scopes! {
	/// What other apps may do with someone's notes.
	pub enum Scope {
		/// Read your notes.
		NotesRead = "notes:read",
		/// Write notes for you.
		NotesWrite = "notes:write",
	}
}

/// The app's user table, sharing its key with the account.
#[table(name = "users")]
#[policy(all, |user, cx| cx.signed_in(|me| user.id.eq(me.account)))]
pub struct User {
	#[db(on_delete = cascade)]
	pub id: Id<Account>,
	#[db(max_len = 254)]
	pub name: String,
}

impl OnSignUp for User {
	async fn on_sign_up(
		tx: &Tx,
		account: Id<Account>,
		profile: &Profile,
	) -> leptoskit_auth::Result<()> {
		let name = profile
			.email
			.as_ref()
			.map(|email| email.to_string().replace("@example.com", ""))
			.unwrap_or_default();
		NewUser { id: account, name }.insert(tx).await?;
		Ok(())
	}
}

impl Claims for User {
	async fn claims(
		db: &Db,
		account: Id<Account>,
		scopes: &GrantedScopes,
		out: &mut ClaimSet,
	) -> leptoskit_idp::Result<()> {
		if scopes.has("profile") {
			// `db` acts as the person, so their own policy lets this through.
			if let Some(user) = User::find(db, account).await? {
				out.name(&user.name);
			}
		}
		Ok(())
	}
}

/// Lists the caller's notes: what a partner app calls with its token.
#[leptoskit_rpc::api(scope = Scope::NotesRead)]
#[leptos::server]
pub async fn list_notes() -> Result<Vec<String>, ServerFnError> {
	Ok(Vec::new())
}

/// Needs both scopes.
#[leptoskit_rpc::api(scope = Scope::NotesRead, scope = Scope::NotesWrite)]
#[leptos::server]
pub async fn write_note() -> Result<(), ServerFnError> {
	Ok(())
}

/// Counts notes: another app may call it as itself, with no person.
#[leptoskit_rpc::api(scope = Scope::NotesRead)]
#[leptos::server]
pub async fn count_notes() -> Result<usize, ServerFnError> {
	Ok(0)
}

/// Only the app's own sessions call this.
#[leptoskit_rpc::api]
#[leptos::server]
pub async fn delete_everything() -> Result<(), ServerFnError> {
	Ok(())
}

pub struct App {
	pub router: Router,
	pub mailbox: Mailbox,
	pub idp: Idp,
	pub db: Db,
}

/// What the scoped routes answer: who the call is from, and which users
/// their policies let them see.
pub async fn whoami(me: SignedIn, Extension(db): Extension<Db>) -> String {
	let visible = User::query().count(&db).await.unwrap();
	format!("{} sees {visible}", me.account)
}

/// Who a call that needs no person is from: the app acting, and the person
/// if there is one.
pub async fn caller(Extension(current): Extension<CurrentSession>) -> String {
	let app = current
		.delegation
		.map(|delegation| delegation.client_id)
		.unwrap_or_default();
	match current.signed_in {
		Some(me) => format!("{app} for {}", me.account),
		None => format!("{app} as itself"),
	}
}

pub async fn app(db: Db) -> App {
	app_with(db, |idp| idp).await
}

/// The app, with its provider configured further by `configure`.
pub async fn app_with(db: Db, configure: impl FnOnce(IdpBuilder) -> IdpBuilder) -> App {
	leptoskit_db::testing::create_tables(&db).await.unwrap();
	let mailbox = Mailbox::new();
	let auth = Auth::builder(&db, Keys::single("test", [5; 32]))
		.app_name("Notes")
		.origin(ORIGIN)
		.mailer(mailbox.clone())
		.on_sign_up::<User>()
		.build()
		.unwrap();
	let idp = configure(
		Idp::builder(&db, &auth)
			.scopes::<Scope>()
			.claims::<User>()
			.sign_in_page("/sign-in")
			.consent_page("/consent"),
	)
	.build()
	.unwrap();
	let router = Router::new()
		.route("/api/list_notes", post(whoami))
		.route("/api/write_note", post(whoami))
		.route("/api/count_notes", post(caller))
		.route("/api/delete_everything", post(whoami))
		.route("/me", get(whoami))
		.merge(auth.router())
		.merge(idp.router())
		.layer(auth.layer())
		.layer(idp.layer());

	App {
		router,
		mailbox,
		idp,
		db,
	}
}

/// Both databases, each with every table, including both packs.
pub async fn databases(cluster: Option<&PostgresCluster>) -> Vec<Db> {
	let mut databases = vec![Db::connect("sqlite::memory:").await.unwrap()];
	if let Some(cluster) = cluster {
		databases.push(Db::connect(&cluster.database()).await.unwrap());
	}
	databases
}

pub fn challenge(verifier: &str) -> String {
	URL_SAFE_NO_PAD.encode(sha2::Sha256::digest(verifier.as_bytes()))
}

pub fn basic(client_id: &str, secret: &str) -> String {
	format!("Basic {}", STANDARD.encode(format!("{client_id}:{secret}")))
}

pub fn query(location: &str) -> Vec<(String, String)> {
	Url::parse(ORIGIN)
		.unwrap()
		.join(location)
		.unwrap()
		.query_pairs()
		.into_owned()
		.collect()
}

pub fn param(location: &str, name: &str) -> Option<String> {
	query(location)
		.into_iter()
		.find(|(key, _)| key == name)
		.map(|(_, value)| value)
}

pub fn location(response: &Response) -> String {
	assert_eq!(response.status(), StatusCode::SEE_OTHER, "{response:?}");
	response.headers()[header::LOCATION]
		.to_str()
		.unwrap()
		.to_owned()
}

pub async fn body<T: serde::de::DeserializeOwned>(response: Response) -> T {
	let bytes = response.into_body().collect().await.unwrap().to_bytes();
	serde_json::from_slice(&bytes).unwrap_or_else(|error| {
		panic!("{error}: {}", String::from_utf8_lossy(&bytes));
	})
}

pub async fn text(response: Response) -> String {
	let bytes = response.into_body().collect().await.unwrap().to_bytes();
	String::from_utf8(bytes.to_vec()).unwrap()
}

/// The claims of a JWT, unchecked.
pub fn claims(token: &str) -> Value {
	let payload = token.split('.').nth(1).unwrap();
	serde_json::from_slice(&URL_SAFE_NO_PAD.decode(payload).unwrap()).unwrap()
}

impl App {
	pub async fn send(&self, request: Request<Body>) -> Response {
		self.router.clone().oneshot(request).await.unwrap()
	}

	pub async fn get(&self, path: &str, cookie: Option<&str>) -> Response {
		let mut builder = Request::get(path);
		if let Some(cookie) = cookie {
			builder = builder.header(header::COOKIE, cookie);
		}
		self.send(builder.body(Body::empty()).unwrap()).await
	}

	pub async fn post_json(&self, path: &str, body: Value, cookie: Option<&str>) -> Response {
		let mut builder = Request::post(path)
			.header(header::ORIGIN, ORIGIN)
			.header(header::CONTENT_TYPE, "application/json");
		if let Some(cookie) = cookie {
			builder = builder.header(header::COOKIE, cookie);
		}
		self.send(builder.body(Body::from(body.to_string())).unwrap())
			.await
	}

	/// A form post, as another app's server or browser sends it.
	pub async fn post_form(
		&self,
		path: &str,
		pairs: &[(&str, &str)],
		authorization: Option<&str>,
	) -> Response {
		let form = url::form_urlencoded::Serializer::new(String::new())
			.extend_pairs(pairs)
			.finish();
		let mut builder = Request::post(path)
			.header(header::ORIGIN, "https://partner.example")
			.header(header::CONTENT_TYPE, "application/x-www-form-urlencoded");
		if let Some(authorization) = authorization {
			builder = builder.header(header::AUTHORIZATION, authorization);
		}
		self.send(builder.body(Body::from(form)).unwrap()).await
	}

	pub async fn with_bearer(&self, method: &str, path: &str, token: &str) -> Response {
		self.send(
			Request::builder()
				.method(method)
				.uri(path)
				.header(header::ORIGIN, "https://partner.example")
				.header(header::AUTHORIZATION, format!("Bearer {token}"))
				.body(Body::empty())
				.unwrap(),
		)
		.await
	}

	/// Signs in with an emailed code, returning the cookie to send back.
	pub async fn sign_in(&self, address: &str) -> String {
		let sent = self
			.post_json("/auth/email/send", json!({ "email": address }), None)
			.await;
		assert_eq!(sent.status(), StatusCode::NO_CONTENT);
		let mail = self.mailbox.latest_to(address).unwrap();
		let code = mail.subject.rsplit(' ').next().unwrap().to_owned();
		let verified = self
			.post_json(
				"/auth/email/verify",
				json!({ "email": address, "code": code }),
				None,
			)
			.await;
		assert_eq!(verified.status(), StatusCode::OK);
		let cookie = verified.headers()[header::SET_COOKIE].to_str().unwrap();
		cookie.split(';').next().unwrap().to_owned()
	}

	pub async fn register(
		&self,
		auth_method: ClientAuth,
		scopes: &[&str],
	) -> (String, Option<String>) {
		self.register_spec(spec(auth_method, scopes)).await
	}

	/// Registers the app `spec` describes.
	pub async fn register_spec(&self, spec: NewClientSpec) -> (String, Option<String>) {
		let output = self.idp.run_command(&ClientCommand::Add(spec)).await;
		let ClientCommandOutput::Added { client, secret } = output else {
			panic!("the client should register: {output:?}");
		};
		(client.client_id, secret)
	}

	/// The `/oauth/authorize` address for `client_id`.
	pub fn authorize_url(client_id: &str, scope: &str, extra: &[(&str, &str)]) -> String {
		let challenge = challenge(VERIFIER);
		let mut pairs = vec![
			("response_type", "code"),
			("client_id", client_id),
			("redirect_uri", CALLBACK),
			("scope", scope),
			("state", "af0ifjsldkj"),
			("nonce", "n-0S6_WzA2Mj"),
			("code_challenge", challenge.as_str()),
			("code_challenge_method", "S256"),
		];
		pairs.extend_from_slice(extra);
		let query = url::form_urlencoded::Serializer::new(String::new())
			.extend_pairs(pairs)
			.finish();
		format!("/oauth/authorize?{query}")
	}

	/// Runs the browser half: authorize, consent if asked, and the code.
	pub async fn code(&self, client_id: &str, scope: &str, cookie: &str) -> String {
		let response = self
			.get(&Self::authorize_url(client_id, scope, &[]), Some(cookie))
			.await;
		let mut to = location(&response);

		if to.starts_with("/consent?") {
			let request = param(&to, "request").unwrap();
			let decided = self
				.post_json(
					"/oauth/consent",
					json!({ "request": request, "approve": true }),
					Some(cookie),
				)
				.await;
			assert_eq!(decided.status(), StatusCode::OK);
			to = body::<ConsentRedirect>(decided).await.redirect_to;
		}

		assert!(to.starts_with(CALLBACK), "{to}");
		assert_eq!(param(&to, "state").as_deref(), Some("af0ifjsldkj"));
		assert_eq!(param(&to, "iss").as_deref(), Some(ORIGIN));
		param(&to, "code").unwrap()
	}

	pub async fn exchange(
		&self,
		client_id: &str,
		secret: Option<&str>,
		code: &str,
		verifier: &str,
	) -> Response {
		let authorization = secret.map(|secret| basic(client_id, secret));
		let mut pairs = vec![
			("grant_type", "authorization_code"),
			("code", code),
			("redirect_uri", CALLBACK),
			("code_verifier", verifier),
		];
		if secret.is_none() {
			pairs.push(("client_id", client_id));
		}
		self.post_form("/oauth/token", &pairs, authorization.as_deref())
			.await
	}

	pub async fn refresh(&self, client_id: &str, secret: &str, refresh_token: &str) -> Response {
		self.post_form(
			"/oauth/token",
			&[
				("grant_type", "refresh_token"),
				("refresh_token", refresh_token),
			],
			Some(&basic(client_id, secret)),
		)
		.await
	}

	pub async fn introspect(&self, client_id: &str, secret: &str, token: &str) -> Introspection {
		let response = self
			.post_form(
				"/oauth/introspect",
				&[("token", token)],
				Some(&basic(client_id, secret)),
			)
			.await;
		assert_eq!(response.status(), StatusCode::OK);
		body(response).await
	}

	pub async fn error(response: Response, status: StatusCode) -> String {
		assert_eq!(response.status(), status);
		body::<OAuthError>(response).await.error
	}
}

impl App {
	/// Sets `column` of the rows of the `idp` table `table` that match
	/// `condition` to `at`, as time passing would. The crate alone writes its
	/// tables, so this goes around it in SQL.
	pub async fn set_time(
		&self,
		table: &str,
		column: &str,
		at: leptoskit_db::Timestamp,
		condition: &str,
	) {
		use leptoskit_db::Connection;

		let (table, value) = match self.db.dialect() {
			leptoskit_db::Dialect::Postgres => {
				(
					format!("idp.{table}"),
					format!("'{}'::timestamptz", at.to_rfc3339()),
				)
			}
			leptoskit_db::Dialect::Sqlite => {
				(
					format!("idp_{table}"),
					format!("'{}'", at.format("%Y-%m-%dT%H:%M:%S%.3fZ")),
				)
			}
		};
		self.db
			.run_script(&format!(
				"UPDATE {table} SET {column} = {value} WHERE {condition}"
			))
			.await
			.unwrap();
	}
}

/// The partner app, as the tests register it, before any changes.
pub fn spec(auth_method: ClientAuth, scopes: &[&str]) -> NewClientSpec {
	NewClientSpec {
		name: "Partner".into(),
		redirect_uris: vec![CALLBACK.into()],
		post_logout_redirect_uris: vec![SIGNED_OUT.into()],
		scopes: scopes.iter().map(|scope| (*scope).to_owned()).collect(),
		auth_method,
		grant_types: leptoskit_idp::default_grant_types(),
		jwks: None,
		id_token_alg: leptoskit_idp::SigningAlgorithm::Es256,
		first_party: false,
		introspection: false,
	}
}

/// What the audit trail holds, oldest first: each event's kind and detail.
pub async fn events(app: &App) -> Vec<(leptoskit_auth::EventKind, Option<String>)> {
	leptoskit_auth::Event::query()
		.order_by(|event| event.created_at.asc())
		.all(&app.db.system("test"))
		.await
		.unwrap()
		.into_iter()
		.map(|event| (event.kind, event.detail))
		.collect()
}

/// Verifies a token's signature against the published key set, as a
/// relying party does, and returns the algorithm it was signed with.
pub async fn verify_with_jwks(app: &App, token: &str) -> String {
	let jwks: Value = body(app.get("/oauth/jwks", None).await).await;
	let header: Value = serde_json::from_slice(
		&URL_SAFE_NO_PAD
			.decode(token.split('.').next().unwrap())
			.unwrap(),
	)
	.unwrap();
	let key = jwks["keys"]
		.as_array()
		.unwrap()
		.iter()
		.find(|key| key["kid"] == header["kid"])
		.expect("the token's key is published");
	assert_eq!(key["alg"], header["alg"]);
	let decode = |name: &str| URL_SAFE_NO_PAD.decode(key[name].as_str().unwrap()).unwrap();
	let (input, signature) = token.rsplit_once('.').unwrap();
	let signature = URL_SAFE_NO_PAD.decode(signature).unwrap();

	match header["alg"].as_str().unwrap() {
		"ES256" => {
			let mut point = vec![4];
			point.extend(decode("x"));
			point.extend(decode("y"));
			let key = VerifyingKey::from_sec1_bytes(&point).unwrap();
			let signature = Signature::from_slice(&signature).unwrap();
			key.verify(input.as_bytes(), &signature)
				.expect("the signature verifies");
		}
		"RS256" => {
			ring::signature::RsaPublicKeyComponents {
				n: decode("n"),
				e: decode("e"),
			}
			.verify(
				&ring::signature::RSA_PKCS1_2048_8192_SHA256,
				input.as_bytes(),
				&signature,
			)
			.expect("the signature verifies");
		}
		other => panic!("unexpected algorithm {other}"),
	}

	header["alg"].as_str().unwrap().to_owned()
}

/// A JWT signed with `key`, as an app signs its `private_key_jwt`
/// assertions.
pub fn es256_jwt(key: &p256::ecdsa::SigningKey, kid: Option<&str>, claims: &Value) -> String {
	use p256::ecdsa::signature::Signer;

	let mut header = json!({ "alg": "ES256", "typ": "JWT" });
	if let Some(kid) = kid {
		header["kid"] = json!(kid);
	}
	let input = format!(
		"{}.{}",
		URL_SAFE_NO_PAD.encode(header.to_string()),
		URL_SAFE_NO_PAD.encode(claims.to_string())
	);
	let signature: Signature = key.sign(input.as_bytes());
	format!("{input}.{}", URL_SAFE_NO_PAD.encode(signature.to_bytes()))
}

/// The RSA key the tests give the provider, as PEM.
pub const TEST_RSA_KEY: &str = include_str!("../fixtures/test-only-rsa-key.pem");

/// The test RSA key, for signing as an app would.
pub fn rsa_key() -> ring::signature::RsaKeyPair {
	let body: String = TEST_RSA_KEY
		.lines()
		.filter(|line| !line.starts_with("-----"))
		.collect();
	ring::signature::RsaKeyPair::from_pkcs8(&STANDARD.decode(body).unwrap()).unwrap()
}

/// A JWT signed with the test RSA key.
pub fn rs256_jwt(claims: &Value) -> String {
	let key = rsa_key();
	let input = format!(
		"{}.{}",
		URL_SAFE_NO_PAD.encode(json!({ "alg": "RS256" }).to_string()),
		URL_SAFE_NO_PAD.encode(claims.to_string())
	);
	let mut signature = vec![0; key.public().modulus_len()];
	key.sign(
		&ring::signature::RSA_PKCS1_SHA256,
		&ring::rand::SystemRandom::new(),
		input.as_bytes(),
		&mut signature,
	)
	.unwrap();
	format!("{input}.{}", URL_SAFE_NO_PAD.encode(signature))
}

/// The test RSA key's public half, as a JWK.
pub fn rsa_jwk() -> Value {
	let components = ring::signature::RsaPublicKeyComponents::<Vec<u8>>::from(rsa_key().public());
	json!({
		"kty": "RSA",
		"n": URL_SAFE_NO_PAD.encode(components.n),
		"e": URL_SAFE_NO_PAD.encode(components.e),
	})
}
