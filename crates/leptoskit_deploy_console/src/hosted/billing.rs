use axum::http::HeaderMap;
use hmac::Mac as _;
use rusqlite::OptionalExtension as _;
use rusqlite::params;

use super::*;

/// Subscription prices are configured Stripe price IDs, never accepted from
/// clients.
#[derive(Clone)]
pub struct Billing {
	client: reqwest::Client,
	api_origin: String,
	key: String,
	webhook: String,
	control_price: String,
	team_price: String,
	serial: Arc<tokio::sync::Mutex<()>>,
}
impl Billing {
	pub fn new(
		key: String,
		webhook: String,
		control_price: String,
		team_price: String,
	) -> Result<Self> {
		ensure!(
			!key.is_empty()
				&& !webhook.is_empty()
				&& control_price.starts_with("price_")
				&& team_price.starts_with("price_")
				&& control_price != team_price,
			"invalid billing configuration"
		);
		Ok(Self {
			client: reqwest::Client::builder()
				.https_only(true)
				.redirect(reqwest::redirect::Policy::none())
				.timeout(std::time::Duration::from_secs(15))
				.build()?,
			api_origin: "https://api.stripe.com".into(),
			key,
			webhook,
			control_price,
			team_price,
			serial: Arc::new(tokio::sync::Mutex::new(())),
		})
	}

	async fn response(response: reqwest::Response) -> Result<serde_json::Value> {
		ensure!(
			response.status().is_success(),
			"billing_provider_unavailable"
		);
		let mut response = response;
		let mut body = Vec::new();
		while let Some(chunk) = response.chunk().await? {
			ensure!(
				body.len() + chunk.len() <= 1024 * 1024,
				"billing response too large"
			);
			body.extend_from_slice(&chunk);
		}
		Ok(serde_json::from_slice(&body)?)
	}

	pub(crate) async fn checkout(
		&self,
		host: &Hosted,
		account: &str,
		plan: &str,
	) -> Result<String> {
		let price = match plan {
			"control" => &self.control_price,
			"team" => &self.team_price,
			_ => anyhow::bail!("unknown plan"),
		};
		let _guard = self.serial.lock().await;
		let customer = host.accounts().customer(account)?;

		let (existing, status): (Option<String>, String) = host.accounts().meta.db()?.query_row(
			"SELECT subscription,status FROM hosted_accounts WHERE id=?1",
			[account],
			|r| Ok((r.get(0)?, r.get(1)?)),
		)?;
		ensure!(
			existing.is_none() || status == "canceled",
			"use billing portal to change an existing subscription"
		);
		let (pending, pending_plan, started, cached, expires): (
			Option<String>,
			Option<String>,
			i64,
			Option<String>,
			i64,
		) = host.accounts().meta.db()?.query_row(
			"SELECT checkout_nonce,checkout_plan,checkout_started,checkout_url,checkout_until \
			 FROM hosted_accounts WHERE id=?1",
			[account],
			|r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
		)?;
		let nonce = if let Some(nonce) = pending {
			ensure!(
				pending_plan.as_deref() == Some(plan) && started + 23 * 3600 > sql_now(),
				"reconcile the existing checkout before creating another"
			);
			if let Some(url) = cached {
				ensure!(
					expires > sql_now(),
					"checkout expired; operator reconciliation required"
				);
				return checked_url(&url, "checkout.stripe.com");
			}
			nonce
		} else {
			let nonce = control::random_key()?;
			host.accounts().meta.db()?.execute(
				"UPDATE hosted_accounts SET \
				 checkout_nonce=?1,checkout_plan=?2,checkout_started=?3 WHERE id=?4",
				params![nonce, plan, sql_now(), account],
			)?;
			nonce
		};
		let fields = [
			("mode", "subscription".to_owned()),
			("client_reference_id", nonce.clone()),
			("customer", customer),
			("line_items[0][price]", price.clone()),
			("line_items[0][quantity]", "1".into()),
			("success_url", format!("{}/console/", host.0.origin)),
			("cancel_url", format!("{}/console/", host.0.origin)),
			(
				"subscription_data[metadata][leptoskit_account]",
				account.into(),
			),
		];
		let response = self
			.client
			.post(format!("{}/v1/checkout/sessions", self.api_origin))
			.bearer_auth(&self.key)
			.header("Stripe-Version", "2026-10-28.endive")
			.header("Idempotency-Key", format!("lk-checkout-{nonce}"))
			.form(&fields)
			.send()
			.await?;
		let value = Self::response(response).await?;

		let url = checked_url(
			value["url"].as_str().context("missing checkout URL")?,
			"checkout.stripe.com",
		)?;
		let until = value["expires_at"]
			.as_i64()
			.context("missing checkout expiry")?;
		host.accounts().meta.db()?.execute(
			"UPDATE hosted_accounts SET checkout_url=?1,checkout_until=?2 WHERE id=?3",
			params![url, until, account],
		)?;
		Ok(url)
	}

	pub(crate) async fn portal(&self, host: &Hosted, account: &str) -> Result<String> {
		let response = self
			.client
			.post(format!("{}/v1/billing_portal/sessions", self.api_origin))
			.bearer_auth(&self.key)
			.header("Stripe-Version", "2026-10-28.endive")
			.form(&[
				("customer", host.accounts().customer(account)?),
				("return_url", format!("{}/console/", host.0.origin)),
			])
			.send()
			.await?;
		let value = Self::response(response).await?;
		checked_url(
			value["url"].as_str().context("missing portal URL")?,
			"billing.stripe.com",
		)
	}

	pub(crate) async fn refresh(&self, host: &Hosted, account: &str, force: bool) -> Result<()> {
		let _guard = self.serial.lock().await;
		let (subscription, synced): (Option<String>, i64) = host.accounts().meta.db()?.query_row(
			"SELECT subscription,synced_at FROM hosted_accounts WHERE id=?1",
			[account],
			|r| Ok((r.get(0)?, r.get(1)?)),
		)?;
		if !force && synced + 300 > sql_now() {
			return Ok(());
		}
		if let Some(subscription) = subscription {
			self.sync_subscription(host, account, &subscription).await?;
		}
		Ok(())
	}

	async fn sync_subscription(
		&self,
		host: &Hosted,
		account: &str,
		subscription: &str,
	) -> Result<()> {
		ensure!(
			control::identifier(subscription) && subscription.starts_with("sub_"),
			"invalid subscription"
		);
		let response = self
			.client
			.get(format!(
				"{}/v1/subscriptions/{subscription}",
				self.api_origin
			))
			.bearer_auth(&self.key)
			.header("Stripe-Version", "2026-10-28.endive")
			.send()
			.await?;
		let value = Self::response(response).await?;
		self.apply_subscription(host, account, &value)
	}

	/// Accept only the account's customer and one configured recurring plan.
	pub fn apply_subscription(
		&self,
		host: &Hosted,
		account: &str,
		value: &serde_json::Value,
	) -> Result<()> {
		ensure!(
			value["customer"].as_str() == Some(host.accounts().customer(account)?.as_str()),
			"billing customer mismatch"
		);
		let subscription = value["id"].as_str().context("missing subscription")?;
		let items = value["items"]["data"]
			.as_array()
			.context("missing subscription items")?;
		ensure!(
			items.len() == 1 && items[0]["quantity"].as_u64() == Some(1),
			"unsupported subscription items"
		);
		let price = items[0]["price"]["id"].as_str().context("missing price")?;
		let plan = if price == self.control_price {
			"control"
		} else if price == self.team_price {
			"team"
		} else {
			anyhow::bail!("unknown price")
		};
		let status = value["status"].as_str().context("missing status")?;
		let until = items[0]["current_period_end"]
			.as_u64()
			.or_else(|| value["current_period_end"].as_u64())
			.unwrap_or(0);
		host.accounts().meta.db()?.execute(
			"UPDATE hosted_accounts SET \
			 subscription=?1,status=?2,plan=?3,valid_until=?4,synced_at=?5,checkout_nonce=CASE \
			 WHEN ?2 IN ('active','trialing') THEN NULL ELSE checkout_nonce END,checkout_url=CASE \
			 WHEN ?2 IN ('active','trialing') THEN NULL ELSE checkout_url END,checkout_plan=CASE \
			 WHEN ?2 IN ('active','trialing') THEN NULL ELSE checkout_plan END WHERE id=?6",
			params![
				subscription,
				status,
				plan,
				i64::try_from(until)?,
				sql_now(),
				account
			],
		)?;
		Ok(())
	}

	pub(crate) fn verify(&self, headers: &HeaderMap, body: &[u8]) -> Result<()> {
		let signature = headers
			.get("stripe-signature")
			.and_then(|s| s.to_str().ok())
			.context("missing signature")?;
		let timestamp = signature
			.split(',')
			.find_map(|part| part.strip_prefix("t="))
			.and_then(|s| s.parse::<u64>().ok())
			.context("invalid timestamp")?;
		ensure!(control::now().abs_diff(timestamp) <= 300, "stale webhook");
		let mut mac = hmac::Hmac::<sha2::Sha256>::new_from_slice(self.webhook.as_bytes())?;
		mac.update(format!("{timestamp}.").as_bytes());
		mac.update(body);
		let valid = signature
			.split(',')
			.filter_map(|p| p.strip_prefix("v1="))
			.any(|hex| {
				if hex.len() != 64 || !hex.is_ascii() {
					return false;
				}
				let decoded = (0..32)
					.map(|i| u8::from_str_radix(&hex[2 * i..2 * i + 2], 16))
					.collect::<std::result::Result<Vec<_>, _>>();
				decoded.is_ok_and(|signature| mac.clone().verify_slice(&signature).is_ok())
			});
		ensure!(valid, "invalid signature");
		Ok(())
	}

	pub(crate) async fn webhook(
		&self,
		host: &Hosted,
		headers: &HeaderMap,
		body: &[u8],
	) -> Result<()> {
		self.verify(headers, body)?;
		let event: serde_json::Value = serde_json::from_slice(body)?;
		let id = event["id"].as_str().context("missing event ID")?;
		let _guard = self.serial.lock().await;
		if host
			.accounts()
			.meta
			.db()?
			.query_row(
				"SELECT 1 FROM hosted_billing_events WHERE id=?1",
				[id],
				|_| Ok(()),
			)
			.optional()?
			.is_some()
		{
			return Ok(());
		}
		let kind = event["type"].as_str().unwrap_or_default();
		if kind.starts_with("customer.subscription.") || kind == "checkout.session.completed" {
			let object = &event["data"]["object"];
			let customer = object["customer"].as_str().context("missing customer")?;
			let account: Option<String> = host
				.accounts()
				.meta
				.db()?
				.query_row(
					"SELECT id FROM hosted_accounts WHERE customer=?1",
					[customer],
					|r| r.get(0),
				)
				.optional()?;
			if let Some(account) = account {
				let subscription = if kind == "checkout.session.completed" {
					object["subscription"].as_str()
				} else {
					object["id"].as_str()
				}
				.context("missing subscription")?;
				let (current, status, pending): (Option<String>, String, Option<String>) =
					host.accounts().meta.db()?.query_row(
						"SELECT subscription,status,checkout_nonce FROM hosted_accounts WHERE \
						 id=?1",
						[&account],
						|r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
					)?;
				let replacement = kind == "checkout.session.completed"
					&& status == "canceled"
					&& pending
						.as_deref()
						.is_some_and(|nonce| object["client_reference_id"].as_str() == Some(nonce));
				if current.as_deref().is_none_or(|s| s == subscription) || replacement {
					// Only the matching new Checkout can replace a canceled
					// subscription. Other old deliveries are acknowledged
					// below. Provider retrieval prevents
					// stale payloads restoring access.
					self.sync_subscription(host, &account, subscription).await?;
				}
			}
		}
		host.accounts().meta.db()?.execute(
			"INSERT INTO hosted_billing_events VALUES(?1,?2)",
			params![id, sql_now()],
		)?;
		Ok(())
	}
}
fn checked_url(value: &str, host: &str) -> Result<String> {
	let url = url::Url::parse(value)?;
	ensure!(
		url.scheme() == "https"
			&& url.host_str() == Some(host)
			&& url.username().is_empty()
			&& url.password().is_none(),
		"invalid billing redirect"
	);
	Ok(value.into())
}

#[cfg(test)]
mod tests {
	use std::sync::atomic::AtomicUsize;
	use std::sync::atomic::Ordering;

	use axum::Json;
	use axum::Router;
	use axum::routing::get;
	use axum::routing::post;

	use super::*;
	fn fixture() -> (tempfile::TempDir, Hosted, Billing) {
		let dir = tempfile::tempdir().unwrap();
		let host = Hosted::open(
			&dir.path().join("cloud"),
			"test-only-hosted-master-key-0123456789".into(),
			"http://localhost:4701",
			None,
		)
		.unwrap();
		host.accounts().create("alice", "cus_alice").unwrap();
		let billing = Billing::new(
			"test-only".into(),
			"webhook-test-secret".into(),
			"price_control".into(),
			"price_team".into(),
		)
		.unwrap();
		(dir, host, billing)
	}
	async fn serve(billing: &mut Billing, app: Router) -> tokio::task::JoinHandle<()> {
		let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
		billing.api_origin = format!("http://{}", listener.local_addr().unwrap());
		billing.client = reqwest::Client::builder()
			.redirect(reqwest::redirect::Policy::none())
			.build()
			.unwrap();
		tokio::spawn(async move {
			axum::serve(listener, app).await.unwrap();
		})
	}
	fn signed(billing: &Billing, event: &serde_json::Value) -> (HeaderMap, Vec<u8>) {
		let bytes = serde_json::to_vec(event).unwrap();
		let timestamp = control::now();
		let mut mac =
			hmac::Hmac::<sha2::Sha256>::new_from_slice(billing.webhook.as_bytes()).unwrap();
		mac.update(format!("{timestamp}.").as_bytes());
		mac.update(&bytes);
		let hex = hex(&mac.finalize().into_bytes());
		let mut headers = HeaderMap::new();
		headers.insert(
			"stripe-signature",
			format!("t={timestamp},v1={hex}").parse().unwrap(),
		);
		(headers, bytes)
	}
	#[tokio::test]
	async fn checkout_reservation_survives_retries_and_refuses_a_second_plan() {
		let (_dir, host, mut billing) = fixture();
		let calls = Arc::new(AtomicUsize::new(0));
		let count = calls.clone();
		let app = Router::new()
			.route(
				"/v1/checkout/sessions",
				post(
					move |axum::Form(form): axum::Form<
						std::collections::HashMap<String, String>,
					>| {
						let count = count.clone();
						async move {
							assert_eq!(form["success_url"], "http://localhost:4701/console/");
							assert_eq!(form["cancel_url"], "http://localhost:4701/console/");
							assert_ne!(form["client_reference_id"], "");
							count.fetch_add(1, Ordering::Relaxed);
							Json(
								serde_json::json!({"url":"https://checkout.stripe.com/c/test","expires_at":sql_now()+3600}),
							)
						}
					},
				),
			)
			.route(
				"/v1/billing_portal/sessions",
				post(
					|axum::Form(form): axum::Form<std::collections::HashMap<String, String>>| {
						async move {
							assert_eq!(form["return_url"], "http://localhost:4701/console/");
							Json(serde_json::json!({"url":"https://billing.stripe.com/p/test"}))
						}
					},
				),
			);
		let server = serve(&mut billing, app).await;
		let first = billing.checkout(&host, "alice", "control").await.unwrap();
		assert_eq!(
			billing.checkout(&host, "alice", "control").await.unwrap(),
			first
		);
		assert!(billing.checkout(&host, "alice", "team").await.is_err());
		assert_eq!(calls.load(Ordering::Relaxed), 1);
		assert_eq!(
			billing.portal(&host, "alice").await.unwrap(),
			"https://billing.stripe.com/p/test"
		);
		server.abort();
	}
	#[tokio::test]
	async fn signed_webhooks_fetch_current_state_deduplicate_and_reject_tampering() {
		let (_dir, host, mut billing) = fixture();
		let calls = Arc::new(AtomicUsize::new(0));
		let count = calls.clone();
		let app=Router::new().route("/v1/subscriptions/sub_alice",get(move||{let count=count.clone();async move{count.fetch_add(1,Ordering::Relaxed);Json(serde_json::json!({"id":"sub_alice","customer":"cus_alice","status":"past_due","items":{"data":[{"quantity":1,"price":{"id":"price_team"},"current_period_end":control::now()+3600}]}}))}}));
		let server = serve(&mut billing, app).await;
		let event = serde_json::json!({"id":"evt_first","type":"customer.subscription.updated","data":{"object":{"id":"sub_alice","customer":"cus_alice","status":"active"}}});
		let (headers, bytes) = signed(&billing, &event);
		billing.webhook(&host, &headers, &bytes).await.unwrap();
		billing.webhook(&host, &headers, &bytes).await.unwrap();
		assert_eq!(host.accounts().view("alice").unwrap().status, "past_due");
		assert!(host.accounts().entitled("alice").is_err());
		assert_eq!(calls.load(Ordering::Relaxed), 1);
		assert!(billing.webhook(&host, &headers, b"{}").await.is_err());
		let mut stale = HeaderMap::new();
		stale.insert("stripe-signature", "t=1,v1=invalid".parse().unwrap());
		assert!(billing.verify(&stale, &bytes).is_err());
		let mut unicode = HeaderMap::new();
		unicode.insert(
			"stripe-signature",
			format!("t={},v1={}", control::now(), "é".repeat(32))
				.parse()
				.unwrap(),
		);
		assert!(billing.verify(&unicode, &bytes).is_err());
		server.abort();
	}

	fn subscription(id: &str, status: &str) -> serde_json::Value {
		serde_json::json!({"id":id,"customer":"cus_alice","status":status,
			"items":{"data":[{"quantity":1,"price":{"id":"price_control"},
				"current_period_end":control::now()+3600}]}})
	}

	async fn deliver(billing: &Billing, host: &Hosted, event: serde_json::Value) {
		let (headers, bytes) = signed(billing, &event);
		billing.webhook(host, &headers, &bytes).await.unwrap();
	}

	#[tokio::test]
	async fn a_new_checkout_can_replace_a_canceled_subscription_despite_delayed_events() {
		let (_dir, host, mut billing) = fixture();
		billing
			.apply_subscription(&host, "alice", &subscription("sub_old", "canceled"))
			.unwrap();
		let app = Router::new()
			.route(
				"/v1/checkout/sessions",
				post(|| {
					async {
						Json(
							serde_json::json!({"url":"https://checkout.stripe.com/c/rejoin",
					"expires_at":sql_now()+3600}),
						)
					}
				}),
			)
			.route(
				"/v1/subscriptions/sub_old",
				get(|| async { Json(subscription("sub_old", "canceled")) }),
			)
			.route(
				"/v1/subscriptions/sub_new",
				get(|| async { Json(subscription("sub_new", "active")) }),
			);
		let server = serve(&mut billing, app).await;
		billing.checkout(&host, "alice", "control").await.unwrap();
		let nonce: String = host
			.accounts()
			.meta
			.db()
			.unwrap()
			.query_row(
				"SELECT checkout_nonce FROM hosted_accounts WHERE id='alice'",
				[],
				|row| row.get(0),
			)
			.unwrap();
		// Stripe may deliver subscription creation before the matching Checkout
		// completion, and old cancellation notifications while it is pending.
		deliver(
			&billing,
			&host,
			serde_json::json!({"id":"evt_new_created",
			"type":"customer.subscription.created","data":{"object":{
				"id":"sub_new","customer":"cus_alice"}}}),
		)
		.await;
		deliver(
			&billing,
			&host,
			serde_json::json!({"id":"evt_old_before",
			"type":"customer.subscription.deleted","data":{"object":{
				"id":"sub_old","customer":"cus_alice"}}}),
		)
		.await;
		assert_eq!(host.accounts().view("alice").unwrap().status, "canceled");
		let pending: Option<String> = host
			.accounts()
			.meta
			.db()
			.unwrap()
			.query_row(
				"SELECT checkout_nonce FROM hosted_accounts WHERE id='alice'",
				[],
				|row| row.get(0),
			)
			.unwrap();
		assert_eq!(pending.as_deref(), Some(nonce.as_str()));
		deliver(
			&billing,
			&host,
			serde_json::json!({"id":"evt_unrelated_checkout",
			"type":"checkout.session.completed","data":{"object":{
				"subscription":"sub_new","customer":"cus_alice","client_reference_id":"other"}}}),
		)
		.await;
		assert!(host.accounts().entitled("alice").is_err());
		let completed = serde_json::json!({"id":"evt_rejoined",
			"type":"checkout.session.completed","data":{"object":{
				"subscription":"sub_new","customer":"cus_alice","client_reference_id":nonce}}});
		deliver(&billing, &host, completed.clone()).await;
		deliver(&billing, &host, completed).await;
		deliver(
			&billing,
			&host,
			serde_json::json!({"id":"evt_old_after",
			"type":"customer.subscription.deleted","data":{"object":{
				"id":"sub_old","customer":"cus_alice"}}}),
		)
		.await;
		assert_eq!(host.accounts().view("alice").unwrap().status, "active");
		assert!(host.accounts().entitled("alice").is_ok());
		server.abort();
	}
}
