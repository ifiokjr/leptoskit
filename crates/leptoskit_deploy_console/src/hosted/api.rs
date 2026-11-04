use axum::Json;
use axum::Router;
use axum::body::Body;
use axum::body::Bytes;
use axum::body::to_bytes;
use axum::extract::DefaultBodyLimit;
use axum::extract::Path;
use axum::extract::Query;
use axum::extract::Request;
use axum::extract::State;
use axum::http::HeaderMap;
use axum::http::HeaderValue;
use axum::http::Method;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::response::Response;
use axum::routing::any;
use axum::routing::get;
use axum::routing::post;
use serde_json::Value;
use serde_json::json;
use tower::ServiceExt as _;

use super::*;
use crate::control::Intent;
use crate::control::Phase;
use crate::control::RunResult;

struct Error(StatusCode, &'static str);
type Api<T> = std::result::Result<T, Error>;
impl IntoResponse for Error {
	fn into_response(self) -> Response {
		(self.0, Json(json!({"error":self.1}))).into_response()
	}
}
fn denied(_: anyhow::Error) -> Error {
	Error(StatusCode::FORBIDDEN, "scope_or_subscription_denied")
}
fn bad(_: anyhow::Error) -> Error {
	Error(StatusCode::CONFLICT, "request_rejected")
}
fn bearer(headers: &HeaderMap) -> Api<&str> {
	headers
		.get("authorization")
		.and_then(|h| h.to_str().ok())
		.and_then(|s| s.strip_prefix("Bearer "))
		.ok_or(Error(StatusCode::UNAUTHORIZED, "authentication_required"))
}
fn owner(host: &Hosted, headers: &HeaderMap) -> Api<String> {
	host.authorize(bearer(headers)?)
		.map_err(|_| Error(StatusCode::UNAUTHORIZED, "authentication_required"))
}

pub(super) fn router(host: Hosted) -> Router {
	Router::new()
		.route("/healthz", get(|| async { StatusCode::OK }))
		.route("/api/v1/account", get(account))
		.route("/api/v1/installations", post(install))
		.route("/api/v1/installations/{project}/pair", post(pair))
		.route("/api/v1/installations/{project}/revoke", post(revoke))
		.route("/api/v1/billing/{action}", post(billing))
		.route("/api/v1/webhooks/stripe", post(webhook))
		.route("/api/v1/worker/pair", post(redeem))
		.route("/api/v1/worker/claim", post(claim))
		.route("/api/v1/worker/finish", post(finish))
		.route("/api/v1/state/{project}", any(state))
		.route("/api/v1/{*path}", any(proxy))
		.layer(DefaultBodyLimit::max(8 * 1024 * 1024))
		.layer(axum::middleware::from_fn(
			|request: Request, next: axum::middleware::Next| {
				async move {
					let mut response = next.run(request).await;
					response
						.headers_mut()
						.insert("cache-control", HeaderValue::from_static("no-store"));
					response.headers_mut().insert(
						"x-content-type-options",
						HeaderValue::from_static("nosniff"),
					);
					response
						.headers_mut()
						.insert("referrer-policy", HeaderValue::from_static("no-referrer"));
					response
				}
			},
		))
		.with_state(host)
}
async fn refresh(host: &Hosted, account: &str) -> Api<()> {
	if let Some(b) = &host.0.billing {
		b.refresh(host, account, false)
			.await
			.map_err(|_| Error(StatusCode::SERVICE_UNAVAILABLE, "billing_unavailable"))?;
	}
	Ok(())
}
async fn account(State(host): State<Hosted>, headers: HeaderMap) -> Api<Json<AccountView>> {
	let account = owner(&host, &headers)?;
	refresh(&host, &account).await?;
	Ok(Json(host.accounts().view(&account).map_err(bad)?))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NewInstallation {
	project: String,
	environment: String,
	ownership: Ownership,
}
async fn install(
	State(host): State<Hosted>,
	headers: HeaderMap,
	Json(body): Json<NewInstallation>,
) -> Api<Json<Installation>> {
	let account = owner(&host, &headers)?;
	refresh(&host, &account).await?;
	host.accounts().entitled(&account).map_err(denied)?;
	// Operator provisioned managed installations have fixed approved infra and
	// per-customer credentials.
	if matches!(body.ownership, Ownership::LeptoskitDigitalocean) {
		return Err(Error(
			StatusCode::FORBIDDEN,
			"request_managed_capacity_from_operator",
		));
	}
	Ok(Json(
		host.register(&account, &body.project, &body.environment, body.ownership)
			.map_err(bad)?,
	))
}
async fn pair(
	State(host): State<Hosted>,
	Path(project): Path<String>,
	headers: HeaderMap,
) -> Api<Json<Value>> {
	let account = owner(&host, &headers)?;
	let installation = host
		.accounts()
		.installation(&account, &project)
		.map_err(bad)?;
	if !matches!(installation.ownership, Ownership::Customer) {
		return Err(Error(
			StatusCode::FORBIDDEN,
			"managed_worker_is_operator_owned",
		));
	}
	Ok(Json(
		json!({"code":host.accounts().pairing(&account,&project).map_err(bad)?,"expires_in":600}),
	))
}
async fn revoke(
	State(host): State<Hosted>,
	Path(project): Path<String>,
	headers: HeaderMap,
) -> Api<StatusCode> {
	let account = owner(&host, &headers)?;
	host.accounts()
		.revoke_worker(&account, &project)
		.map_err(bad)?;
	Ok(StatusCode::NO_CONTENT)
}
async fn redeem(State(host): State<Hosted>, Json(body): Json<Value>) -> Api<Json<Value>> {
	let code = body["code"]
		.as_str()
		.ok_or(Error(StatusCode::BAD_REQUEST, "pairing_code_required"))?;
	let (_, installation, key) = host
		.accounts()
		.redeem(code)
		.map_err(|_| Error(StatusCode::UNAUTHORIZED, "pairing_code_invalid"))?;
	Ok(Json(json!({"installation":installation,"key":key})))
}
async fn claim(State(host): State<Hosted>, headers: HeaderMap) -> Api<Json<Option<Job>>> {
	let (account, installation) = host.accounts().worker(bearer(&headers)?).map_err(denied)?;
	refresh(&host, &account).await?;
	host.accounts().seen(&installation.id).map_err(bad)?;
	let (store, service) = host.tenant(&account).map_err(bad)?;
	store.maintain().map_err(bad)?;
	// A worker never resumes an uncertain running operation after process loss.
	let operation = store
		.claim_project(Some(&installation.project))
		.map_err(bad)?;
	if let Some(operation) = operation {
		if host
			.check_job_policy(&account, &installation.project, &operation.intent.action)
			.is_err()
		{
			store
				.finish(
					&operation.id,
					&RunResult {
						succeeded: false,
						message: "Subscription or capacity policy changed before execution".into(),
						details: json!({"code":"policy_changed"}),
					},
				)
				.map_err(bad)?;
			return Ok(Json(None));
		}
		let read = service
			.managed_app_key(&installation.project, false)
			.map_err(bad)?;
		let ingest = service
			.managed_app_key(&installation.project, true)
			.map_err(bad)?;
		Ok(Json(Some(Job {
			operation,
			analytics_read_key: read,
			analytics_ingest_key: format!("{account}.{ingest}"),
		})))
	} else {
		Ok(Json(None))
	}
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Finish {
	id: String,
	succeeded: bool,
	message: String,
	details: Value,
}
async fn finish(
	State(host): State<Hosted>,
	headers: HeaderMap,
	Json(body): Json<Finish>,
) -> Api<StatusCode> {
	let (account, installation) = host.accounts().worker(bearer(&headers)?).map_err(denied)?;
	let (store, _) = host.tenant(&account).map_err(bad)?;
	let operation = store
		.operation(&body.id)
		.map_err(bad)?
		.ok_or(Error(StatusCode::NOT_FOUND, "operation_not_found"))?;
	if operation.project != installation.project {
		return Err(Error(StatusCode::FORBIDDEN, "scope_or_origin_denied"));
	}
	let phase = if body.succeeded {
		Phase::Succeeded
	} else {
		Phase::Failed
	};
	if operation.phase == phase
		&& operation.message == body.message
		&& operation.result == body.details
	{
		return Ok(StatusCode::NO_CONTENT);
	}
	store
		.finish(
			&body.id,
			&RunResult {
				succeeded: body.succeeded,
				message: body.message,
				details: body.details,
			},
		)
		.map_err(bad)?;
	Ok(StatusCode::NO_CONTENT)
}
async fn billing(
	State(host): State<Hosted>,
	Path(action): Path<String>,
	headers: HeaderMap,
	Json(body): Json<Value>,
) -> Api<Json<Value>> {
	let account = owner(&host, &headers)?;
	let b = host.0.billing.as_ref().ok_or(Error(
		StatusCode::SERVICE_UNAVAILABLE,
		"billing_not_configured",
	))?;
	let url = match action.as_str() {
		"checkout" => {
			b.checkout(&host, &account, body["plan"].as_str().unwrap_or_default())
				.await
		}
		"portal" => b.portal(&host, &account).await,
		_ => return Err(Error(StatusCode::NOT_FOUND, "not_found")),
	}
	.map_err(bad)?;
	Ok(Json(json!({"url":url})))
}
async fn webhook(State(host): State<Hosted>, headers: HeaderMap, body: Bytes) -> Api<StatusCode> {
	let b = host.0.billing.as_ref().ok_or(Error(
		StatusCode::SERVICE_UNAVAILABLE,
		"billing_not_configured",
	))?;
	b.webhook(&host, &headers, &body)
		.await
		.map_err(|_| Error(StatusCode::BAD_REQUEST, "webhook_rejected"))?;
	Ok(StatusCode::NO_CONTENT)
}
#[derive(Deserialize)]
struct StateQuery {
	#[serde(rename = "ID")]
	id: Option<String>,
}
async fn state(
	State(host): State<Hosted>,
	Path(project): Path<String>,
	Query(query): Query<StateQuery>,
	headers: HeaderMap,
	method: Method,
	body: Bytes,
) -> Api<Response> {
	use base64::Engine as _;
	let key = if let Ok(key) = bearer(&headers) {
		key.to_owned()
	} else {
		let basic = headers
			.get("authorization")
			.and_then(|h| h.to_str().ok())
			.and_then(|s| s.strip_prefix("Basic "))
			.ok_or(Error(StatusCode::UNAUTHORIZED, "authentication_required"))?;
		let text = String::from_utf8(
			base64::engine::general_purpose::STANDARD
				.decode(basic)
				.map_err(|_| Error(StatusCode::UNAUTHORIZED, "authentication_required"))?,
		)
		.map_err(|_| Error(StatusCode::UNAUTHORIZED, "authentication_required"))?;
		text.split_once(':')
			.map(|(_, p)| p.to_owned())
			.ok_or(Error(StatusCode::UNAUTHORIZED, "authentication_required"))?
	};
	let (account, installation) = if method == Method::GET
		&& let Ok(account) = host.authorize(&key)
	{
		let installation = host
			.accounts()
			.installation(&account, &project)
			.map_err(bad)?;
		(account, installation)
	} else {
		host.accounts().worker(&key).map_err(denied)?
	};
	if installation.project != project {
		return Err(Error(StatusCode::FORBIDDEN, "scope_or_origin_denied"));
	}
	let response = tokio::task::spawn_blocking(move || -> Result<Response> {
		match method.as_str() {
			"GET" => {
				Ok(host.state_read(&account, &project)?.map_or_else(
					|| StatusCode::NOT_FOUND.into_response(),
					|body| ([("content-type", "application/json")], body).into_response(),
				))
			}
			"LOCK" | "UNLOCK" => {
				let lock: Value = serde_json::from_slice(&body)?;
				Ok(host
					.state_lock(&account, &project, &lock, method.as_str() == "UNLOCK")?
					.map_or_else(
						|| StatusCode::OK.into_response(),
						|lock| (StatusCode::LOCKED, Json(lock)).into_response(),
					))
			}
			"POST" => {
				host.state_write(
					&account,
					&project,
					query.id.as_deref().context("lock ID required")?,
					&body,
				)?;
				Ok(StatusCode::OK.into_response())
			}
			_ => Ok(StatusCode::METHOD_NOT_ALLOWED.into_response()),
		}
	})
	.await
	.map_err(|_| Error(StatusCode::SERVICE_UNAVAILABLE, "state_unavailable"))?
	.map_err(bad)?;
	Ok(response)
}
// The standard CLI protocol is reused under each account's isolated journal.
async fn proxy(State(host): State<Hosted>, mut request: Request) -> Api<Response> {
	let key = bearer(request.headers())?.to_owned();
	let (account, forward) = if let Ok(account) = host.authorize(&key) {
		let forward = host.derived(&format!("tenant-owner:{account}"));
		(account, forward)
	} else {
		let (account, token) = key
			.split_once('.')
			.ok_or(Error(StatusCode::UNAUTHORIZED, "authentication_required"))?;
		host.accounts().view(account).map_err(denied)?;
		let (store, _) = host.tenant(account).map_err(bad)?;
		if store.authenticate(token, false).map_err(bad)?.is_none() {
			return Err(Error(StatusCode::UNAUTHORIZED, "authentication_required"));
		}
		(account.to_owned(), token.to_owned())
	};
	let path = request.uri().path().to_owned();
	if path == "/api/v1/login" || path == "/api/v1/logout" {
		return Err(Error(StatusCode::NOT_FOUND, "not_found"));
	}
	let issuing = path == "/api/v1/tokens";
	if request.method() == Method::POST && path.ends_with("/operations") {
		refresh(&host, &account).await?;
		let project = path
			.strip_prefix("/api/v1/projects/")
			.and_then(|s| s.strip_suffix("/operations"))
			.ok_or(Error(StatusCode::BAD_REQUEST, "invalid_project"))?;
		let (parts, body) = request.into_parts();
		let bytes = to_bytes(body, 65536)
			.await
			.map_err(|_| Error(StatusCode::BAD_REQUEST, "invalid_intent"))?;
		let intent: Intent = serde_json::from_slice(&bytes)
			.map_err(|_| Error(StatusCode::BAD_REQUEST, "invalid_intent"))?;
		host.check_job_policy(&account, project, &intent.action)
			.map_err(denied)?;
		request = Request::from_parts(parts, Body::from(bytes));
	}
	if path == "/api/v1/tokens/revoke" {
		let (parts, body) = request.into_parts();
		let bytes = to_bytes(body, 4096)
			.await
			.map_err(|_| Error(StatusCode::BAD_REQUEST, "invalid_key"))?;
		let mut value: Value = serde_json::from_slice(&bytes)
			.map_err(|_| Error(StatusCode::BAD_REQUEST, "invalid_key"))?;
		if let Some(token) = value["key"]
			.as_str()
			.and_then(|s| s.strip_prefix(&format!("{account}.")))
		{
			value["key"] = json!(token);
		}
		request = Request::from_parts(
			parts,
			Body::from(
				serde_json::to_vec(&value)
					.map_err(|_| Error(StatusCode::BAD_REQUEST, "invalid_key"))?,
			),
		);
	}
	request.headers_mut().insert(
		"authorization",
		HeaderValue::from_str(&format!("Bearer {forward}"))
			.map_err(|_| Error(StatusCode::UNAUTHORIZED, "authentication_required"))?,
	);
	let (_, service) = host.tenant(&account).map_err(bad)?;
	request.extensions_mut().clear();
	let response = service
		.router()
		.oneshot(request)
		.await
		.expect("infallible router");
	if issuing && response.status().is_success() {
		let bytes = to_bytes(response.into_body(), 4096)
			.await
			.map_err(|_| Error(StatusCode::SERVICE_UNAVAILABLE, "key_unavailable"))?;
		let mut value: Value = serde_json::from_slice(&bytes)
			.map_err(|_| Error(StatusCode::SERVICE_UNAVAILABLE, "key_unavailable"))?;
		value["key"] = json!(format!(
			"{account}.{}",
			value["key"].as_str().unwrap_or_default()
		));
		Ok(Json(value).into_response())
	} else {
		Ok(response)
	}
}
