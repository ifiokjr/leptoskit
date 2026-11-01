# Background work

Some work is too slow, or too unreliable, to do while someone waits: sending an email, submitting a payment, calling a rate-limited API, importing a spreadsheet. The person should get an answer at once ("Sending…"), then hear how it went, on whichever device they're using, even if the server restarts halfway through.

leptoskit splits this into three pieces, all on the server, all in your app's own database:

- **Jobs** run work later, retried with backoff, in steps that survive a crash.
- **Actions** run work now that may call anything, outside any transaction.
- **The inbox** tells people when slow work finishes or fails.

The work runs on your server. The screens that start it and follow it run everywhere: the web, iOS, Android, macOS, Windows, Linux, the terminal, and HarmonyOS (experimental). They call mutations and actions over HTTP, and follow jobs and the inbox live over the socket from [Real-time screens](24-real-time.md).

## An email, and "Email sent" on the sender's screen

A mutation writes the invite and enqueues the job **in the same transaction**: if the mutation fails, no email goes out, and if it commits, the email is never lost.

```rust
use leptoskit::db::Id;
use leptoskit::rpc;

#[rpc::mutation]
pub async fn invite_member(cx: &rpc::MutationCx, email: String) -> rpc::Result<Id<Invite>> {
	let invite = NewInvite { email, status: InviteStatus::Sending }.insert(cx.tx()).await?;
	cx.enqueue(SendInvite { invite: invite.id }).await?;
	Ok(invite.id)
}

/// Sends the invitation. Runs on a worker; nobody waits for it.
#[rpc::job(queue = "mail", retries = 8)]
pub async fn send_invite(cx: &rpc::JobCx, invite: Id<Invite>) -> rpc::Result<()> {
	let invite = Invite::get(cx.db(), invite).await?;
	cx.step("send", async || mailer::send(&invite.email, &cx.key()).await)
		.await?;
	cx.mutate(MarkInviteSent { invite: invite.id }).await?;
	Ok(())
}
```

`invites` is a [live table](24-real-time.md#mark-the-tables-worth-watching), so `MarkInviteSent` changes the row on every screen showing it: "Sending…" becomes "Email sent" without a refresh.

Start the workers once, when the server boots:

```rust
let jobs = rpc::server::Jobs::from_config(db.clone(), &config.rpc.jobs).start();
// …and when the server shuts down, after it stops taking requests:
jobs.shutdown().await;
```

```toml
# leptoskit.toml
[rpc.jobs]
workers = 4 # jobs each server runs at once
poll_seconds = 5 # how often idle workers look for due jobs
retention_days = 7 # how long finished jobs are kept

[rpc.jobs.queues.mail]
concurrency = 2 # across every server, so the mail provider never sees more
```

Workers start a job within milliseconds of the transaction that enqueued it committing, on any server sharing the database, and look every `poll_seconds` for jobs scheduled to run later. Jobs live in the `rpc` schema pack's `jobs` table, so there's no queue service to run.

The workers also look after the change log that live screens follow: they prune it to `[db] live_retention_minutes` and, on Postgres, take part in [replication capture](22-database.md#capture-changes-from-the-write-ahead-log) when the app uses it. `Jobs::from_configs(db.clone(), &config.rpc.jobs, &config.db)` reads both sections; `.live_retention(None)` keeps the whole log, and `.capture(false)` keeps a server out of capture.

## A payment, shown at once and settled later

A blockchain payment takes seconds to minutes to confirm, and can fail. The job submits it, waits for confirmation, and updates the payment row as it goes:

```rust
#[rpc::mutation]
pub async fn send_payment(cx: &rpc::MutationCx, to: Id<User>, signed: SignedTransaction) -> rpc::Result<Id<Payment>> {
	let payment = NewPayment { to, signature: signed.signature(), status: PaymentStatus::Pending }
		.insert(cx.tx())
		.await?;
	// One settlement job per payment, however often this runs.
	cx.enqueue(SettlePayment { payment: payment.id, signed }).key(payment.id).await?;
	Ok(payment.id)
}

#[rpc::job(
	queue = "chain",
	serialize_by = payment,
	retries = 20,
	backoff = exponential(2s, 5m),
	deadline = 15m,
	on_failure = payment_gave_up,
)]
pub async fn settle_payment(cx: &rpc::JobCx, payment: Id<Payment>, signed: SignedTransaction) -> rpc::Result<()> {
	// Resending the same signed bytes is safe: the signature is the network's idempotency key.
	cx.step("submit", async || chain::submit(&signed).await).await?;
	cx.mutate(SetPaymentStatus { payment, status: PaymentStatus::Submitted }).await?;
	cx.progress("Waiting for confirmation");

	match cx.step("confirm", async || chain::wait_finalized(signed.signature()).await).await? {
		Finality::Finalized => {
			cx.mutate(MarkPaid { payment }).await?;
			cx.notify(cx.owner(), rpc::Notice::new("Payment confirmed").kind(rpc::NoticeKind::Success))
				.await?;
		}
		Finality::Dropped(reason) => {
			cx.mutate(MarkPaymentFailed { payment, reason: reason.clone() }).await?;
			cx.notify(
				cx.owner(),
				rpc::Notice::new("Payment failed")
					.body(reason)
					.link(format!("/payments/{payment}"))
					.action(rpc::NoticeAction::new("Retry", RetryPayment { payment }))
					.kind(rpc::NoticeKind::Error),
			)
			.await?;
		}
	}
	Ok(())
}

/// Runs once, after the last retry or the deadline: the payment never stays "pending" forever.
#[rpc::mutation]
async fn payment_gave_up(cx: &rpc::MutationCx, job: rpc::Failed<SettlePayment>) -> rpc::Result<()> {
	MarkPaymentFailed { payment: job.job.payment, reason: job.error }.run(cx).await
}
```

What each person sees: the payment appears as "pending" the moment the mutation commits; it becomes "submitted" and then "paid" (or "failed", with a reason) on every device showing it; and the inbox gets "Payment confirmed" or "Payment failed · Retry".

Keep what people see in your own tables, as the `payments` row is here. Its status is covered by your policies and queries, and it outlives the job, which is deleted after `retention_days`.

## What a job promises

- **At least once.** A job runs until it succeeds, is cancelled, or runs out of retries or time. A server that crashes mid-job loses its lease, and another server picks the job up.
- **Steps run once.** `cx.step(name, …)` saves its result with the job. A retried job skips the steps it finished and resumes at the first unfinished one. A crash _during_ a step runs that step again, so a step that reaches the outside world sends `cx.key()`: the same on every attempt, and different for each step. Email providers, payment providers, and blockchains (through the signature) accept one.
- **Database changes happen once.** `cx.mutate(…)` runs a mutation with a receipt keyed by the job and the step, so an attempt after a crash finds the result instead of applying it again.
- **Failures say whether to try again.** Return `rpc::Error::retry(…)` (any error does the same) to retry after the backoff, or `rpc::Error::permanent(…)` to stop at once. When a job gives up, its `on_failure` mutation runs once, with the job's arguments and its last error.

Give each step a name that is unique within the job: the name is how the next attempt finds what the last one saved.

## Options

| Option                            | Meaning                                                                                                                         |
| --------------------------------- | ------------------------------------------------------------------------------------------------------------------------------- |
| `queue = "chain"`                 | Which queue. Each queue can have its own concurrency (`[rpc.jobs.queues.chain] concurrency = 2`), across every server           |
| `retries = 20`                    | How many more attempts after a failure; four by default                                                                         |
| `backoff = exponential(2s, 5m)`   | The wait between attempts: doubling from the first to the second, each wait randomly between half and all of it                 |
| `deadline = 15m`                  | No attempt starts later than this after the job was enqueued                                                                    |
| `timeout = 30s`                   | How long one attempt may run before it counts as failed                                                                         |
| `serialize_by = wallet`           | At most one running job per value of that argument, across every server: a wallet's transactions in order, one import at a time |
| `visible_to = signed_in`          | Who may read the job besides whoever enqueued it: `signed_in`, `everyone`, or an argument naming an account                     |
| `on_failure = payment_gave_up`    | A mutation taking `job: rpc::Failed<ThisJob>`, run once when the job gives up                                                   |
| `.after(duration)`, `.at(time)`   | When enqueuing: run later, for reminders, trial expiry, and "undo send" windows                                                 |
| `.key(value)`                     | When enqueuing: at most one job with this key, so enqueuing twice returns the first                                             |
| `.priority(n)`, `.queue("other")` | When enqueuing: higher priorities run first; or put this one in another queue                                                   |

Durations are a whole number and a unit: `ms`, `s`, `m`, `h`, or `d`.

## Jobs on a schedule

A job with no arguments can run on a cron schedule, in UTC:

```rust
#[rpc::cron("*/5 * * * *")]
#[rpc::job]
pub async fn expire_trials(cx: &rpc::JobCx) -> rpc::Result<()> {
	// …
	Ok(())
}
```

Every server running workers enqueues each tick with the tick as its key, so the job runs once per tick however many servers there are. `@hourly`, `@daily`, `@weekly`, `@monthly`, and `@yearly` work too. A schedule that doesn't parse fails to compile.

## Following a job from a screen

`cx.enqueue(…)` returns the job's id, which a mutation can return to the client. `use_job` follows it:

```rust
let report = use_job(move || report_id.get().to_string());
let label = move || match report.data() {
	Some(job) if job.state == "done" => "Ready".to_owned(),
	Some(job) => job.progress.unwrap_or_else(|| "Working…".to_owned()),
	None => String::new(),
};
```

Over the socket, each step and progress update arrives as it's written; without one (a server render, or a build with no socket), `use_job` asks once a second. `cx.progress("…")` and `cx.progress_fraction(0.4)` are written a few times a second at most, so call them as often as is useful.

A job is visible to the person whose request enqueued it, `cx.owner()`, and to nobody else unless `visible_to` says so. `rpc::CancelJob { id }`, a mutation any client can call, cancels a job its caller enqueued: a waiting job never starts, and a running one stops at its next step. Server code calls `rpc::cancel(&db, id)`.

## Actions: slow work someone waits for

A mutation must not call the outside world: it runs in a transaction, and runs again when another transaction gets in its way. When someone does want to wait for an outside call, such as "summarize this channel", use an action:

```rust
#[rpc::action]
pub async fn summarize_channel(cx: &rpc::ActionCx, channel: Id<Channel>) -> rpc::Result<String> {
	let messages = cx.query(Messages { channel, limit: 200 }).await?; // as the caller
	let summary = my_llm::summarize(&messages).await?; // anything goes here
	cx.mutate(PinSummary { channel, summary: summary.clone() }).await?;
	Ok(summary)
}
```

An action isn't a transaction and is never retried. It reaches the database only through queries and mutations, which run as the caller, so policies and receipts still apply. It's served at `POST /api/summarize_channel`, published by `#[api]` like any operation, and called from generated clients as a one-shot call. From a screen:

```rust
let summarize = use_action::<SummarizeChannel>();
summarize.mutate(SummarizeChannel { channel });
let summary = move || summarize.data();
```

A client never sends an action twice by itself, since it can't know whether the first reached the outside world. Work that must survive a restart belongs in a job.

## Telling people: the inbox

When slow work finishes, the person who started it may be on another screen or another device. `cx.notify(account, notice)` puts a notice in their inbox, from a mutation, a job, or an action. From a mutation it's written in the mutation's transaction, so it arrives exactly when the mutation commits.

`use_inbox()` shows the signed-in person's notices on every device they have open, at once:

```rust
let inbox = use_inbox();
let unread = move || inbox.unread();
inbox.mark_read(notice.id.clone());
inbox.mark_all_read();
```

Each account reads and marks only its own notices. A notice's action names a mutation and its arguments; tapping it runs the mutation as the person tapping, under their own policies.

While the app is closed, a notice waits in the inbox. Reaching a closed app needs remote push (APNs, FCM, or Web Push), which leptoskit doesn't send yet. Notices also go to any delivery channel you add with `rpc::server::inbox::deliver_with(…)`, such as email for people who asked for it, after they're written.

## Operating jobs

`leptoskit jobs` reads and changes any deployment's jobs table directly:

```sh
leptoskit jobs list --url "$DATABASE_URL" --state failed
leptoskit jobs retry 0192f3c4-… --url "$DATABASE_URL"
leptoskit jobs cancel 0192f3c4-… --url "$DATABASE_URL"
```

Failed jobs stay until someone retries them; finished and cancelled jobs are deleted after `retention_days`. A retried job starts again from its first step, with no deadline.

## Testing jobs

`rpc::testing::TestJobs` runs jobs in a test on a clock the test moves, one at a time, on the test's own task:

```rust
let db = leptoskit::db::testing::sqlite().await;
let jobs = rpc::testing::TestJobs::new(&db);

rpc::call(&ada, SendPayment { to: grace, signed }).await?;
jobs.kill_in_step("confirm"); // the worker dies after confirming, before saving it
jobs.run_jobs_until_idle().await;
jobs.advance(Duration::from_secs(61)).await; // past the lease: another worker takes over
jobs.run_jobs_until_idle().await; // "submit" is skipped; "confirm" runs again with the same key
```

`advance` also enqueues the cron jobs whose ticks it passed.
