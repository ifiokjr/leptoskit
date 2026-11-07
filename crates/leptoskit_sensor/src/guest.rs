//! Streaming guest clients shared by every sensor plugin.

use std::collections::VecDeque;

use leptoskit_bridge::Error;
use leptoskit_bridge::guest::Call;
use leptoskit_bridge::guest::Client;

use crate::Batch;
use crate::Interval;
use crate::Kind;
use crate::Sample;
use crate::WATCH;
use crate::WatchRequest;

/// A continuous series of samples from one sensor.
///
/// Await [`Watch::next`] in a loop. The pending native call belongs to the
/// watch, not to the future, so dropping a `next` future (when another branch
/// of a `select!` wins, say) loses nothing: the next call resumes the same
/// request. Dropping the watch cancels that request, and the native side stops
/// the sensor.
#[derive(Debug)]
pub struct Watch<R> {
	client: Client,
	kind: Option<Kind>,
	interval: Interval,
	cursor: u64,
	dropped: u64,
	buffered: VecDeque<R>,
	pending: Option<Call>,
}

impl<R: Sample> Watch<R> {
	/// Watches a single-sensor plugin, such as `accelerometer`.
	#[must_use]
	pub fn new(client: Client, interval: Interval) -> Self {
		Self::with_kind(client, None, interval)
	}

	/// Watches one kind of the combined `sensors` plugin.
	#[must_use]
	pub fn of_kind(client: Client, kind: Kind, interval: Interval) -> Self {
		Self::with_kind(client, Some(kind), interval)
	}

	fn with_kind(client: Client, kind: Option<Kind>, interval: Interval) -> Self {
		Self {
			client,
			kind,
			interval,
			cursor: 0,
			dropped: 0,
			buffered: VecDeque::new(),
			pending: None,
		}
	}

	/// Waits for the next sample, oldest first.
	pub async fn next(&mut self) -> Result<R, Error> {
		loop {
			if let Some(sample) = self.buffered.pop_front() {
				return Ok(sample);
			}

			self.fetch().await?;
		}
	}

	/// Waits for at least one sample, then returns every sample that has
	/// arrived, oldest first. Use it to process a frame's worth at once.
	pub async fn next_batch(&mut self) -> Result<Vec<R>, Error> {
		if self.buffered.is_empty() {
			self.fetch().await?;
		}

		Ok(self.buffered.drain(..).collect())
	}

	/// Samples the native side discarded because they arrived faster than this
	/// watch read them. Read more often, or batch, when this grows.
	#[must_use]
	pub fn dropped(&self) -> u64 {
		self.dropped
	}

	/// The requested sampling interval.
	#[must_use]
	pub fn interval(&self) -> Interval {
		self.interval
	}

	/// Resumes the pending request, or makes one, and buffers its reply.
	async fn fetch(&mut self) -> Result<(), Error> {
		if self.pending.is_none() {
			let request = WatchRequest {
				interval: self.interval,
				cursor: self.cursor,
			}
			.encode();
			let payload = match self.kind {
				Some(kind) => kind.prefix(&request),
				None => request.to_vec(),
			};
			self.pending = Some(self.client.call(WATCH, &payload)?);
		}

		let call = self.pending.as_mut().expect("a request is pending");
		let reply = call.await;
		self.pending = None;
		let batch = Batch::<R>::decode(&reply?, self.cursor)?;
		self.cursor = batch.cursor;
		self.dropped += u64::from(batch.dropped);
		self.buffered.extend(batch.samples);
		Ok(())
	}
}
