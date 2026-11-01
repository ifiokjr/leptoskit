//! Actions and the inbox, on SQLite and Postgres: an action runs its
//! queries and mutations as the caller, over HTTP and in process, and never
//! takes an idempotency key; notices land in the caller's transaction, each
//! account reads and clears only its own, and every written notice reaches
//! the app's delivery channels.

#![cfg(all(feature = "postgres", feature = "sqlite"))]
#![allow(
	clippy::cognitive_complexity,
	reason = "each scenario reads as one story, step by step"
)]

#[path = "../../leptoskit_db/tests/support/postgres.rs"]
mod postgres;

use std::sync::Mutex;
use std::sync::Once;

use axum::body::Body;
use axum::http::Request;
use axum::http::StatusCode;
use http_body_util::BodyExt;
use leptoskit_db::Actor;
use leptoskit_db::Context;
use leptoskit_db::Db;
use leptoskit_db::Id;
use leptoskit_db::table;
use leptoskit_query::BoxFuture;
use leptoskit_rpc as rpc;
use leptoskit_rpc::Kind;
use leptoskit_rpc::Operation;
use leptoskit_rpc::api;
use leptoskit_rpc::server::Identify;
use leptoskit_rpc::server::Identity;
use leptoskit_rpc::server::Server;
use leptoskit_rpc::server::inbox::Deliver;
use leptoskit_rpc::server::inbox::Delivery;
use postgres::PostgresCluster;
use serde_json::json;
use tower::ServiceExt;

/// A pinned summary per channel.
#[table(name = "summaries")]
pub struct Summary {
	pub id: Id<Self>,
	#[db(max_len = 64)]
	pub channel: String,
	#[db(max_len = 2000)]
	pub text: String,
}

/// The pinned summaries of a channel.
#[rpc::query]
pub async fn summaries(cx: &rpc::QueryCx, channel: String) -> rpc::Result<Vec<String>> {
	Ok(Summary::query()
		.filter(|summary| summary.channel.eq(channel))
		.all(cx.db())
		.await?
		.into_iter()
		.map(|summary| summary.text)
		.collect())
}

/// Pins a summary, for the signed-in person only.
#[rpc::mutation]
pub async fn pin_summary(cx: &rpc::MutationCx, channel: String, text: String) -> rpc::Result<()> {
	cx.require_user::<()>()?;
	NewSummary { channel, text }.insert(cx.tx()).await?;
	Ok(())
}

/// Summarizes a channel with an outside service, then pins the summary.
///
/// # Arguments
///
/// * `channel` - Which channel.
#[api(tag = "chat")]
#[rpc::action]
pub async fn summarize(cx: &rpc::ActionCx, channel: String) -> rpc::Result<String> {
	let me = cx.require_user::<()>()?;
	assert!(cx.signed_in());
	assert_eq!(cx.user_id::<()>(), Some(me));
	assert!(cx.get::<Actor>().is_some());
	assert!(cx.context().actor().is_some());
	let before = cx
		.query(Summaries {
			channel: channel.clone(),
		})
		.await?;
	// The outside call an action exists for.
	let summary = format!("{} said hello ({} before)", channel, before.len());
	cx.mutate(PinSummary {
		channel: channel.clone(),
		text: summary.clone(),
	})
	.await?;

	cx.notify(
		me,
		rpc::Notice::new("Summary ready")
			.body(summary.clone())
			.link(format!("/channels/{channel}"))
			.action(rpc::NoticeAction::new(
				"Summarize again",
				Summarize {
					channel: channel.clone(),
				},
			))
			.kind(rpc::NoticeKind::Success),
	)
	.await?;
	cx.enqueue(Tidy {}).await?;
	assert!(format!("{cx:?}").contains("ActionCx"));
	Ok(summary)
}

/// Housekeeping an action leaves for later.
#[rpc::job]
pub async fn tidy(_cx: &rpc::JobCx) -> rpc::Result<()> {
	Ok(())
}

/// Tells two people something, in one transaction, then fails when asked.
#[rpc::mutation]
pub async fn tell(cx: &rpc::MutationCx, to: Vec<Id<()>>, fail: bool) -> rpc::Result<Vec<String>> {
	let mut ids = Vec::new();

	for account in &to {
		if let Some(id) = cx
			.notify(*account, rpc::Notice::new("Hello").body("from a test"))
			.await?
		{
			ids.push(id);
		}
	}

	// Nobody to tell: nothing written.
	assert!(
		cx.notify(None::<Id<()>>, rpc::Notice::new("Nobody"))
			.await?
			.is_none()
	);

	if fail {
		return Err(rpc::Error::invalid("changed my mind"));
	}

	Ok(ids)
}

/// Writes a notice with no title.
#[rpc::mutation]
pub async fn tell_nothing(cx: &rpc::MutationCx, to: Id<()>) -> rpc::Result<()> {
	cx.notify(to, rpc::Notice::new("  ")).await.map(|_| ())
}

static DELIVERED: Mutex<Vec<Delivery>> = Mutex::new(Vec::new());

/// Records deliveries, as an email channel would send them.
struct Recorder;

impl Deliver for Recorder {
	fn deliver<'a>(&'a self, delivery: &'a Delivery) -> BoxFuture<'a, rpc::Result<()>> {
		Box::pin(async move {
			DELIVERED.lock().unwrap().push(delivery.clone());
			Err(rpc::Error::retry(
				"a channel that fails doesn't take the notice back",
			))
		})
	}
}

fn delivered_to(account: Id<()>) -> Vec<Delivery> {
	DELIVERED
		.lock()
		.unwrap()
		.iter()
		.filter(|delivery| delivery.account == account.to_string())
		.cloned()
		.collect()
}

fn as_account(db: &Db, account: Id<()>) -> Db {
	db.with_context(Context::new().with(Actor::of(account)))
}

/// Trusts an `x-account` header, for the test.
struct Header;

impl Identify for Header {
	fn identify<'a>(
		&'a self,
		request: &'a http::request::Parts,
	) -> BoxFuture<'a, rpc::Result<Identity>> {
		Box::pin(async move {
			let Some(account) = request
				.headers
				.get("x-account")
				.and_then(|value| value.to_str().ok())
				.and_then(|value| value.parse::<Id<()>>().ok())
			else {
				return Ok(Identity::anonymous());
			};
			Ok(Identity::new(Context::new().with(Actor::of(account)))
				.with_caller(account.to_string()))
		})
	}
}

async fn wait_for_delivery(account: Id<()>, count: usize) -> Vec<Delivery> {
	for _ in 0..200 {
		let delivered = delivered_to(account);

		if delivered.len() >= count {
			return delivered;
		}

		tokio::time::sleep(std::time::Duration::from_millis(10)).await;
	}

	delivered_to(account)
}

async fn actions_run_as_the_caller(db: &Db) {
	assert_eq!(Summarize::KIND, Kind::Action);
	let ada_id = Id::new();
	let ada = as_account(db, ada_id);

	// In process.
	let summary = rpc::call(
		&ada,
		Summarize {
			channel: "general".into(),
		},
	)
	.await
	.unwrap();
	assert_eq!(summary, "general said hello (0 before)");
	// Anonymous callers are refused by the action itself.
	assert!(matches!(
		rpc::call(
			&db.with_context(Context::new()),
			Summarize {
				channel: "general".into()
			}
		)
		.await,
		Err(rpc::Error::Unauthorized(_))
	));

	// Over HTTP: `POST /api/summarize`, with an idempotency key that an
	// action ignores, so sending it twice runs it twice.
	let router = Server::new(db.clone()).identify(Header).router::<()>();
	for expected in [
		"general said hello (1 before)",
		"general said hello (2 before)",
	] {
		let response = router
			.clone()
			.oneshot(
				Request::post("/api/summarize")
					.header("content-type", "application/json")
					.header("x-account", ada_id.to_string())
					.header("idempotency-key", "same-key")
					.body(Body::from(json!({ "channel": "general" }).to_string()))
					.unwrap(),
			)
			.await
			.unwrap();
		let status = response.status();
		let body = response.into_body().collect().await.unwrap().to_bytes();
		assert_eq!(status, StatusCode::OK, "{body:?}");
		assert_eq!(serde_json::from_slice::<String>(&body).unwrap(), expected);
	}
	let response = router
		.clone()
		.oneshot(
			Request::post("/api/summarize")
				.header("content-type", "application/json")
				.body(Body::from(json!({ "channel": "general" }).to_string()))
				.unwrap(),
		)
		.await
		.unwrap();
	assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

	// Its notices carry the action to run again.
	let inbox = rpc::call(&ada, rpc::InboxArgs { limit: 2 }).await.unwrap();
	assert_eq!(inbox.unread, 3);
	assert_eq!(inbox.notices.len(), 2);
	let notice = &inbox.notices[0].notice;
	assert_eq!(notice.link.as_deref(), Some("/channels/general"));
	assert_eq!(notice.kind, rpc::NoticeKind::Success);
	assert_eq!(
		notice
			.action
			.as_ref()
			.unwrap()
			.decode::<Summarize>()
			.unwrap()
			.channel,
		"general"
	);
	assert_eq!(wait_for_delivery(ada_id, 3).await.len(), 3);

	// The jobs it enqueued are the caller's.
	let tidy = rpc::server::jobs::JobRow::query()
		.filter(|job| job.name.eq("tidy"))
		.all(&db.system("test"))
		.await
		.unwrap();
	assert_eq!(tidy.len(), 3);
	assert!(tidy.iter().all(|job| job.owner == ada_id.to_string()));
}

async fn each_account_reads_only_its_own_notices(db: &Db) {
	let ada_id = Id::new();
	let grace_id = Id::new();
	let ada = as_account(db, ada_id);
	let grace = as_account(db, grace_id);

	// Written in the mutation's transaction: a failed mutation leaves none,
	// and delivers none.
	assert!(matches!(
		rpc::call(
			&ada,
			Tell {
				to: vec![ada_id, grace_id],
				fail: true
			}
		)
		.await,
		Err(rpc::Error::Invalid(_))
	));
	let ids = rpc::call(
		&ada,
		Tell {
			to: vec![ada_id, grace_id],
			fail: false,
		},
	)
	.await
	.unwrap();
	assert_eq!(ids.len(), 2);
	assert_eq!(wait_for_delivery(grace_id, 1).await.len(), 1);
	assert_eq!(delivered_to(ada_id).len(), 1);
	assert!(matches!(
		rpc::call(&ada, TellNothing { to: ada_id }).await,
		Err(rpc::Error::Invalid(_))
	));

	let ada_inbox = rpc::call(&ada, rpc::InboxArgs { limit: 50 }).await.unwrap();
	assert_eq!(ada_inbox.notices.len(), 1);
	assert_eq!(ada_inbox.notices[0].id, ids[0]);
	assert!(!ada_inbox.notices[0].read);
	assert_eq!(ada_inbox.unread, 1);

	// Grace can't mark Ada's notice read: to her it isn't there.
	assert!(matches!(
		rpc::call(&grace, rpc::MarkRead { id: ids[0].clone() }).await,
		Err(rpc::Error::NotFound(_))
	));
	assert!(matches!(
		rpc::call(&grace, rpc::MarkRead { id: "nope".into() }).await,
		Err(rpc::Error::Invalid(_))
	));
	rpc::call(&ada, rpc::MarkRead { id: ids[0].clone() })
		.await
		.unwrap();
	// Marking it again changes nothing.
	rpc::call(&ada, rpc::MarkRead { id: ids[0].clone() })
		.await
		.unwrap();
	let ada_inbox = rpc::call(&ada, rpc::InboxArgs { limit: 50 }).await.unwrap();
	assert_eq!(ada_inbox.unread, 0);
	assert!(ada_inbox.notices[0].read);

	// Marking all read touches only the caller's.
	assert_eq!(rpc::call(&ada, rpc::MarkAllRead {}).await.unwrap(), 0);
	let grace_inbox = rpc::call(&grace, rpc::InboxArgs { limit: 50 })
		.await
		.unwrap();
	assert_eq!(grace_inbox.unread, 1);
	assert_eq!(rpc::call(&grace, rpc::MarkAllRead {}).await.unwrap(), 1);
	assert_eq!(
		rpc::call(&grace, rpc::InboxArgs { limit: 0 })
			.await
			.unwrap()
			.unread,
		0
	);

	// Nobody signed in has no inbox.
	let nobody = db.with_context(Context::new());
	assert_eq!(
		rpc::call(&nobody, rpc::InboxArgs { limit: 50 })
			.await
			.unwrap(),
		rpc::Inbox::default()
	);
	assert!(matches!(
		rpc::call(&nobody, rpc::MarkAllRead {}).await,
		Err(rpc::Error::Unauthorized(_))
	));
}

fn record_deliveries() {
	static RECORDING: Once = Once::new();
	RECORDING.call_once(|| rpc::server::inbox::deliver_with(Recorder));
}

static SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

#[tokio::test(flavor = "multi_thread")]
async fn sqlite_actions_and_inbox() {
	let _serial = SERIAL.lock().await;
	record_deliveries();
	actions_run_as_the_caller(&leptoskit_db::testing::sqlite().await).await;
	each_account_reads_only_its_own_notices(&leptoskit_db::testing::sqlite().await).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn postgres_actions_and_inbox() {
	let _serial = SERIAL.lock().await;
	record_deliveries();
	let Some(cluster) = PostgresCluster::start() else {
		return;
	};

	for scenario in 0..2 {
		let db = Db::connect(&cluster.database()).await.unwrap();
		leptoskit_db::testing::create_tables(&db).await.unwrap();

		if scenario == 0 {
			actions_run_as_the_caller(&db).await;
		} else {
			each_account_reads_only_its_own_notices(&db).await;
		}
	}
}

#[test]
fn a_published_action_joins_the_spec_and_openapi() {
	let spec = rpc::spec::collect(&rpc::spec::SpecOptions::new("Chat", "1.0.0")).unwrap();
	assert_eq!(
		spec.functions
			.iter()
			.map(|function| function.name.as_str())
			.collect::<Vec<_>>(),
		["summarize"],
		"only what #[api] publishes"
	);
	let summarize = &spec.functions[0];
	assert_eq!(summarize.kind, rpc::spec::ApiKind::Action);
	insta::assert_json_snapshot!("action_spec", summarize);
	let openapi = spec.to_openapi();
	insta::assert_json_snapshot!("action_openapi", openapi["paths"]["/api/summarize"]);
}
