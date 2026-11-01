//! "Remind me in 10 minutes": a job enqueued now to run later.
//!
//! The job is a row in `rpc.jobs` with a `run_at` in the future, so it
//! survives a restart, and any server sharing the database may run it.
//! When it runs, it leaves a notice in the inbox, which every device the
//! person has open shows at once.

use huddle_db::Channel;
use leptoskit_db::Id;
use leptoskit_rpc as rpc;

/// Leaves the signed-in person a notice about `channel` in `minutes`.
/// Returns the job's id, for `use_job`.
#[rpc::mutation]
pub async fn remind_me(
	cx: &rpc::MutationCx,
	channel: Id<Channel>,
	minutes: u32,
) -> rpc::Result<String> {
	crate::me(cx.get())?;

	if !(1..=24 * 60).contains(&minutes) {
		return Err(rpc::Error::invalid("pick between a minute and a day"));
	}

	let job = cx
		.enqueue(Remind { channel })
		.after(std::time::Duration::from_secs(u64::from(minutes) * 60))
		.await?;
	Ok(job.to_string())
}

/// Reminds whoever asked about a channel.
#[rpc::job(queue = "reminders")]
pub async fn remind(cx: &rpc::JobCx, channel: Id<Channel>) -> rpc::Result<()> {
	let name = Channel::get(cx.db(), channel).await?.name;
	cx.notify(
		cx.owner(),
		rpc::Notice::new(format!("Reminder: #{name}"))
			.body("You asked to be reminded about this channel.")
			.link(format!("/channels/{channel}")),
	)
	.await?;
	Ok(())
}
