//! "Tip a teammate": a few coins, settled by a background job against a
//! **simulated** settlement service.
//!
//! Nothing leaves the server: [`Settlement`] runs in this process, waits as
//! long as it's told to, and declines as often as it's told to. It stands in
//! for the slow, unreliable outside service a real app pays through, so the
//! example can show every state a payment goes through:
//!
//! 1. [`send_tip`] writes the tip as `pending` and enqueues [`SettleTip`] in
//!    the same transaction. The sender's screen shows it at once.
//! 2. The job's `submit` step hands the tip to the service and saves the
//!    receipt; the tip becomes `submitted`. A retried attempt skips the step
//!    and reuses the receipt, so a crash never submits twice.
//! 3. The `confirm` step waits for the service's answer. On success the tip is
//!    `confirmed`, and both people get a notice.
//! 4. A decline is permanent, so the job stops at once and its `on_failure`
//!    mutation, [`tip_gave_up`], marks the tip `failed` and leaves the sender a
//!    notice whose **Retry** button calls [`retry_tip`].

use leptoskit_auth::Account;
use leptoskit_db::Id;
use leptoskit_rpc as rpc;
use schemars::JsonSchema;
use serde::Deserialize;
use serde::Serialize;

/// A tip, as the screen shows it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct TipView {
	pub id: Id<huddle_db::Tip>,
	pub from_id: Id<Account>,
	pub from: String,
	pub to_id: Id<Account>,
	pub to: String,
	pub amount: i64,
	/// `pending`, `submitted`, `confirmed`, or `failed`.
	pub status: String,
	pub reason: Option<String>,
	pub receipt: Option<String>,
}

/// The simulated settlement service: how long it takes, and how often it
/// declines.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Settlement {
	/// How long submitting and confirming take, together.
	pub delay: std::time::Duration,
	/// The share of tips it declines, from 0 (never) to 1 (always).
	pub failure_rate: f64,
}

impl Default for Settlement {
	fn default() -> Self {
		Self {
			delay: std::time::Duration::from_secs(2),
			failure_rate: 0.0,
		}
	}
}

#[cfg(feature = "server")]
impl Settlement {
	/// Accepts a tip and answers with a receipt, keyed so that submitting
	/// the same attempt twice gives the same one.
	async fn submit(&self, key: &str) -> String {
		tokio::time::sleep(self.delay / 2).await;
		format!("sim-{}", fingerprint(key) % 1_000_000_000)
	}

	/// Settles the receipt, or declines it. Whether it declines depends
	/// only on the receipt and the failure rate, so it's the same on every
	/// attempt.
	async fn confirm(&self, receipt: &str) -> rpc::Result<()> {
		tokio::time::sleep(self.delay / 2).await;
		#[expect(
			clippy::cast_precision_loss,
			reason = "a roll between 0 and 1 needs no more precision"
		)]
		let roll = (fingerprint(receipt) % 10_000) as f64 / 10_000.0;

		if roll < self.failure_rate {
			Err(rpc::Error::permanent(
				"The settlement service declined the tip (simulated)",
			))
		} else {
			Ok(())
		}
	}
}

#[cfg(feature = "server")]
fn fingerprint(text: &str) -> u64 {
	use std::hash::Hash;
	use std::hash::Hasher;

	let mut hasher = std::collections::hash_map::DefaultHasher::new();
	text.hash(&mut hasher);
	hasher.finish()
}

/// The tips the signed-in person sent or received, newest first.
#[rpc::query]
pub async fn tips(cx: &rpc::QueryCx) -> rpc::Result<Vec<TipView>> {
	let tips = huddle_db::Tip::query()
		.order_by(|tip| tip.created_at.desc())
		.limit(20)
		.all(cx.db())
		.await?;
	let mut ids: Vec<Id<Account>> = tips
		.iter()
		.flat_map(|tip| [tip.from_id, tip.to_id])
		.collect();
	ids.sort_unstable();
	ids.dedup();
	let names: std::collections::HashMap<Id<Account>, String> = huddle_db::User::query()
		.filter(|user| user.id.in_list(ids))
		.all(cx.db())
		.await?
		.into_iter()
		.map(|user| (user.id, user.name))
		.collect();
	let name = |id: &Id<Account>| {
		names
			.get(id)
			.cloned()
			.unwrap_or_else(|| "Someone".to_owned())
	};
	Ok(tips
		.into_iter()
		.map(|tip| {
			TipView {
				id: tip.id,
				from: name(&tip.from_id),
				from_id: tip.from_id,
				to: name(&tip.to_id),
				to_id: tip.to_id,
				amount: tip.amount,
				status: tip.status,
				reason: tip.reason,
				receipt: tip.receipt,
			}
		})
		.collect())
}

/// Tips `to` some coins. Returns at once, with the tip `pending`.
#[rpc::mutation]
pub async fn send_tip(
	cx: &rpc::MutationCx,
	to: Id<Account>,
	amount: i64,
) -> rpc::Result<Id<huddle_db::Tip>> {
	let me = crate::me(cx.get())?;

	if to == me {
		return Err(rpc::Error::invalid("tip someone else"));
	}

	let tip = huddle_db::NewTip {
		from_id: me,
		to_id: to,
		amount,
		status: "pending".to_owned(),
		receipt: None,
		reason: None,
		attempt: 1,
	}
	.insert(cx.tx())
	.await?;
	// One settlement per attempt, however often this runs.
	cx.enqueue(SettleTip { tip: tip.id })
		.key(format!("tip-{}-1", tip.id))
		.await?;
	Ok(tip.id)
}

/// Settles a tip in two checkpointed steps.
#[rpc::job(
	queue = "settlement",
	retries = 3,
	backoff = exponential(1s, 10s),
	on_failure = tip_gave_up
)]
pub async fn settle_tip(cx: &rpc::JobCx, tip: Id<huddle_db::Tip>) -> rpc::Result<()> {
	let row = huddle_db::Tip::get(cx.db(), tip).await?;
	let settlement = crate::services::get().settlement;
	let key = format!("{}:{}", row.id, row.attempt);
	let receipt: String = cx
		.step("submit", || async { Ok(settlement.submit(&key).await) })
		.await?;
	cx.mutate(SetTipStatus {
		tip,
		status: "submitted".to_owned(),
		receipt: Some(receipt.clone()),
		reason: None,
	})
	.await?;
	cx.progress("Waiting for the settlement service");
	cx.step("confirm", || async { settlement.confirm(&receipt).await })
		.await?;
	cx.mutate(SetTipStatus {
		tip,
		status: "confirmed".to_owned(),
		receipt: Some(receipt),
		reason: None,
	})
	.await?;

	let from = huddle_db::User::get(cx.db(), row.from_id).await?.name;
	let to = huddle_db::User::get(cx.db(), row.to_id).await?.name;
	cx.notify(
		row.from_id,
		rpc::Notice::new(format!("Your tip to {to} went through"))
			.body(format!("{} coins, settled.", row.amount))
			.kind(rpc::NoticeKind::Success),
	)
	.await?;
	cx.notify(
		row.to_id,
		rpc::Notice::new(format!("{from} tipped you {} coins", row.amount))
			.kind(rpc::NoticeKind::Success),
	)
	.await?;
	Ok(())
}

/// Records where a tip is. Only [`SettleTip`] runs it.
#[rpc::mutation]
pub async fn set_tip_status(
	cx: &rpc::MutationCx,
	tip: Id<huddle_db::Tip>,
	status: String,
	receipt: Option<String>,
	reason: Option<String>,
) -> rpc::Result<()> {
	crate::from_job(cx)?;
	let mut row = huddle_db::Tip::get(cx.tx(), tip).await?;
	row.status = status;
	row.receipt = receipt;
	row.reason = reason;
	row.save(cx.tx()).await?;
	Ok(())
}

/// Runs once when [`SettleTip`] gives up: the tip must not stay `pending`.
/// The sender's notice offers **Retry**.
#[rpc::mutation]
pub async fn tip_gave_up(cx: &rpc::MutationCx, job: rpc::Failed<SettleTip>) -> rpc::Result<()> {
	crate::from_job(cx)?;
	let mut row = huddle_db::Tip::get(cx.tx(), job.job.tip).await?;
	"failed".clone_into(&mut row.status);
	row.reason = Some(job.error.clone());
	let from = row.from_id;
	let to = huddle_db::User::get(cx.tx(), row.to_id).await?.name;
	let amount = row.amount;
	row.save(cx.tx()).await?;
	cx.notify(
		from,
		rpc::Notice::new(format!("Your tip to {to} failed"))
			.body(format!("{amount} coins: {}", job.error))
			.action(rpc::NoticeAction::new(
				"Retry",
				RetryTip { tip: job.job.tip },
			))
			.kind(rpc::NoticeKind::Error),
	)
	.await?;
	Ok(())
}

/// Sends a failed tip again, as a new attempt with its own settlement job.
#[rpc::mutation]
pub async fn retry_tip(cx: &rpc::MutationCx, tip: Id<huddle_db::Tip>) -> rpc::Result<()> {
	crate::me(cx.get())?;
	let mut row = huddle_db::Tip::get(cx.tx(), tip).await?;

	if row.status != "failed" {
		return Ok(());
	}

	"pending".clone_into(&mut row.status);
	row.reason = None;
	row.receipt = None;
	row.attempt += 1;
	let attempt = row.attempt;
	row.save(cx.tx()).await?;
	cx.enqueue(SettleTip { tip })
		.key(format!("tip-{tip}-{attempt}"))
		.await?;
	Ok(())
}
